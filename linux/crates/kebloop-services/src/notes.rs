//! Local Markdown note storage with crash recovery.
//!
//! Notes are individual Markdown files under `$XDG_DATA_HOME/kebloop/notes`. Drafts are written
//! atomically to a private Recovery directory while the user edits, then removed after a
//! successful save. The path is injected so the store can be tested without using the real home.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;
use crate::util::{atomic_write, new_id};

/// Maximum UTF-8 size of one note.
pub const MAXIMUM_NOTE_BYTES: usize = 2 * 1024 * 1024;
/// Maximum size of the encoded recovery record.
pub const MAXIMUM_RECOVERY_BYTES: usize = MAXIMUM_NOTE_BYTES * 6 + 4_096;
/// Maximum number of notes loaded at once.
pub const MAXIMUM_NOTE_COUNT: usize = 500;
/// Maximum combined size of loaded note contents.
pub const MAXIMUM_LOADED_BYTES: usize = 25 * 1024 * 1024;

/// One local Markdown note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownNote {
    /// Stable identifier, used as the Markdown filename.
    pub id: String,
    /// Markdown source.
    pub contents: String,
    /// File modification time, or last draft update time while editing.
    pub modified_at: SystemTime,
}

impl MarkdownNote {
    /// A short title derived from the first non-empty line.
    pub fn title(&self) -> String {
        self.contents
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(|line| {
                let heading = line.trim_start_matches('#').trim();
                let title = if heading.is_empty() { line } else { heading };
                title.chars().take(80).collect()
            })
            .unwrap_or_else(|| "Untitled Note".to_owned())
    }

    /// Case- and diacritic-insensitive search over title and body.
    pub fn matches(&self, query: &str) -> bool {
        if query.trim().is_empty() {
            return true;
        }
        let needle = crate::util::fold(query.trim());
        crate::util::fold(&self.title()).contains(&needle)
            || crate::util::fold(&self.contents).contains(&needle)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryDraft {
    id: String,
    contents: String,
    saved_at: u64,
}

/// Store for local Markdown notes.
pub struct MarkdownNotesStore {
    directory: PathBuf,
    recovery_directory: PathBuf,
    notes: Vec<MarkdownNote>,
    recovery_candidates: HashSet<String>,
    last_error: Option<String>,
}

impl std::fmt::Debug for MarkdownNotesStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MarkdownNotesStore")
            .field("directory", &self.directory)
            .field("notes", &self.notes.len())
            .field("recovery_candidates", &self.recovery_candidates.len())
            .finish_non_exhaustive()
    }
}

impl MarkdownNotesStore {
    /// Open notes inside the Kebloop XDG data directory (`<data_dir>/notes`).
    pub fn open(data_dir: &Path) -> Result<Self, ConfigError> {
        Self::open_notes_dir(&data_dir.join("notes"))
    }

    /// Open notes from an explicit notes directory.
    pub fn open_notes_dir(directory: &Path) -> Result<Self, ConfigError> {
        let recovery_directory = directory.join("Recovery");
        fs::create_dir_all(&recovery_directory)
            .map_err(|error| ConfigError::io(&recovery_directory, error))?;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .map_err(|error| ConfigError::io(directory, error))?;
        fs::set_permissions(&recovery_directory, fs::Permissions::from_mode(0o700))
            .map_err(|error| ConfigError::io(&recovery_directory, error))?;

        let mut store = Self {
            directory: directory.to_owned(),
            recovery_directory,
            notes: Vec::new(),
            recovery_candidates: HashSet::new(),
            last_error: None,
        };
        store.load();
        Ok(store)
    }

    /// Notes ordered from most recently modified to oldest.
    pub fn notes(&self) -> &[MarkdownNote] {
        &self.notes
    }

    /// The note with `id`, if it is loaded.
    pub fn note(&self, id: &str) -> Option<&MarkdownNote> {
        self.notes.iter().find(|note| note.id == id)
    }

    /// IDs with an unsaved draft that can be restored or discarded.
    pub fn recovery_candidates(&self) -> &HashSet<String> {
        &self.recovery_candidates
    }

    /// The most recent storage error, if an operation failed.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Add an empty or pre-filled note and save it immediately.
    pub fn create(&mut self, contents: impl Into<String>) -> Result<MarkdownNote, ConfigError> {
        let contents = contents.into();
        self.validate_contents(&contents, None)?;
        if self.notes.len() >= MAXIMUM_NOTE_COUNT {
            return Err(ConfigError::invalid(
                "Kebloop's notes folder has reached its local storage limit.",
            ));
        }
        let note = MarkdownNote {
            id: new_id(),
            contents,
            modified_at: SystemTime::now(),
        };
        let path = self.note_path(&note.id)?;
        atomic_write(&path, note.contents.as_bytes(), 0o600)?;
        self.notes.push(note.clone());
        self.sort_notes();
        self.last_error = None;
        Ok(note)
    }

    /// Atomically save the current editor contents as a recovery draft.
    ///
    /// Call this after a short UI debounce; `flush` commits the draft to the Markdown file.
    pub fn write_recovery(&mut self, id: &str, contents: &str) -> Result<(), ConfigError> {
        let index = self
            .notes
            .iter()
            .position(|note| note.id == id)
            .ok_or_else(|| ConfigError::invalid("The note is no longer available."))?;
        self.validate_contents(contents, Some(id))?;
        let draft = RecoveryDraft {
            id: id.to_owned(),
            contents: contents.to_owned(),
            saved_at: epoch_seconds(SystemTime::now()),
        };
        let encoded = serde_json::to_vec(&draft)?;
        if encoded.len() > MAXIMUM_RECOVERY_BYTES {
            return Err(note_too_large());
        }
        let path = self.recovery_path(id)?;
        atomic_write(&path, &encoded, 0o600)?;
        self.notes[index].contents = contents.to_owned();
        self.notes[index].modified_at = SystemTime::now();
        self.recovery_candidates.insert(id.to_owned());
        self.sort_notes();
        self.last_error = None;
        Ok(())
    }

    /// Save current contents to the Markdown file, then remove its recovery draft.
    pub fn flush(&mut self, id: &str, contents: &str) -> Result<(), ConfigError> {
        let index = self
            .notes
            .iter()
            .position(|note| note.id == id)
            .ok_or_else(|| ConfigError::invalid("The note is no longer available."))?;
        self.validate_contents(contents, Some(id))?;
        let path = self.note_path(id)?;
        atomic_write(&path, contents.as_bytes(), 0o600)?;
        self.notes[index].contents = contents.to_owned();
        self.notes[index].modified_at = fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or_else(|_| SystemTime::now());
        let recovery = self.recovery_path(id)?;
        match fs::remove_file(&recovery) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(ConfigError::io(recovery, error)),
        }
        self.recovery_candidates.remove(id);
        self.sort_notes();
        self.last_error = None;
        Ok(())
    }

    /// Restore a valid recovery draft and commit it to the Markdown file.
    pub fn restore_recovery(&mut self, id: &str) -> Result<(), ConfigError> {
        if !self.recovery_candidates.contains(id) {
            return Err(ConfigError::invalid(
                "There is no recovery draft for this note.",
            ));
        }
        let path = self.recovery_path(id)?;
        let bytes = fs::read(&path).map_err(|error| ConfigError::io(&path, error))?;
        if bytes.len() > MAXIMUM_RECOVERY_BYTES {
            return Err(note_too_large());
        }
        let draft: RecoveryDraft = serde_json::from_slice(&bytes)?;
        if draft.id != id || !is_safe_id(&draft.id) {
            return Err(ConfigError::invalid("The recovery draft is invalid."));
        }
        if self.note(id).is_none() {
            return Err(ConfigError::invalid("The note is no longer available."));
        }
        self.validate_contents(&draft.contents, Some(id))?;
        self.flush(id, &draft.contents)?;
        if let Some(note) = self.notes.iter_mut().find(|note| note.id == id) {
            note.modified_at = UNIX_EPOCH + std::time::Duration::from_secs(draft.saved_at);
        }
        self.sort_notes();
        Ok(())
    }

    /// Discard a recovery draft without changing the saved note.
    pub fn discard_recovery(&mut self, id: &str) -> Result<(), ConfigError> {
        let path = self.recovery_path(id)?;
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(ConfigError::io(path, error)),
        }
        let saved_path = self.note_path(id)?;
        if let Some(index) = self.notes.iter().position(|note| note.id == id)
            && let Ok(contents) = fs::read_to_string(&saved_path)
        {
            self.notes[index].contents = contents;
            self.notes[index].modified_at = fs::metadata(&saved_path)
                .and_then(|metadata| metadata.modified())
                .unwrap_or(UNIX_EPOCH);
            self.sort_notes();
        }
        self.recovery_candidates.remove(id);
        self.last_error = None;
        Ok(())
    }

    /// Remove a note and its recovery draft from disk.
    pub fn remove(&mut self, id: &str) -> Result<(), ConfigError> {
        // Clear recovery first: if deleting the Markdown file fails, its saved contents remain
        // intact. Removing the Markdown file first could leave a recovery-only phantom note.
        self.discard_recovery(id)?;
        let note_path = self.note_path(id)?;
        match fs::remove_file(&note_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(ConfigError::io(&note_path, error)),
        }
        self.notes.retain(|note| note.id != id);
        self.last_error = None;
        Ok(())
    }

    /// Export a note as plain UTF-8 Markdown to an explicitly selected destination.
    pub fn export(&self, id: &str, destination: &Path) -> Result<(), ConfigError> {
        let note = self
            .note(id)
            .ok_or_else(|| ConfigError::invalid("The note is no longer available."))?;
        atomic_write(destination, note.contents.as_bytes(), 0o600)
    }

    fn load(&mut self) {
        let Ok(entries) = fs::read_dir(&self.directory) else {
            self.last_error = Some("Kebloop's notes folder could not be read.".to_owned());
            return;
        };
        let mut loaded_bytes = 0usize;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "md") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if !is_safe_id(id) || self.notes.len() >= MAXIMUM_NOTE_COUNT {
                continue;
            }
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if !metadata.file_type().is_file() {
                continue;
            }
            let Ok(byte_count) = usize::try_from(metadata.len()) else {
                continue;
            };
            if byte_count > MAXIMUM_NOTE_BYTES
                || loaded_bytes.saturating_add(byte_count) > MAXIMUM_LOADED_BYTES
            {
                continue;
            }
            let Ok(contents) = fs::read_to_string(&path) else {
                continue;
            };
            self.notes.push(MarkdownNote {
                id: id.to_owned(),
                contents,
                modified_at: metadata.modified().unwrap_or(UNIX_EPOCH),
            });
            loaded_bytes = loaded_bytes.saturating_add(byte_count);
        }
        self.load_recovery(loaded_bytes);
        self.sort_notes();
    }

    fn load_recovery(&mut self, loaded_bytes: usize) {
        let Ok(entries) = fs::read_dir(&self.recovery_directory) else {
            return;
        };
        let mut total_bytes = loaded_bytes;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if !metadata.file_type().is_file()
                || usize::try_from(metadata.len()).is_ok_and(|size| size > MAXIMUM_RECOVERY_BYTES)
            {
                continue;
            }
            let Ok(bytes) = fs::read(&path) else { continue };
            let Ok(draft) = serde_json::from_slice::<RecoveryDraft>(&bytes) else {
                continue;
            };
            if !is_safe_id(&draft.id)
                || path.file_stem().and_then(|stem| stem.to_str()) != Some(draft.id.as_str())
                || draft.contents.len() > MAXIMUM_NOTE_BYTES
            {
                continue;
            }
            let existing = self.notes.iter().position(|note| note.id == draft.id);
            let previous_size = existing.map_or(0, |index| self.notes[index].contents.len());
            let projected = total_bytes
                .saturating_sub(previous_size)
                .saturating_add(draft.contents.len());
            if (existing.is_none() && self.notes.len() >= MAXIMUM_NOTE_COUNT)
                || projected > MAXIMUM_LOADED_BYTES
            {
                continue;
            }
            total_bytes = projected;
            if let Some(index) = existing {
                if self.notes[index].contents == draft.contents {
                    let _ = fs::remove_file(&path);
                    continue;
                }
            } else {
                self.notes.push(MarkdownNote {
                    id: draft.id.clone(),
                    contents: String::new(),
                    modified_at: UNIX_EPOCH + std::time::Duration::from_secs(draft.saved_at),
                });
            }
            self.recovery_candidates.insert(draft.id);
        }
    }

    fn validate_contents(
        &self,
        contents: &str,
        replacing_id: Option<&str>,
    ) -> Result<(), ConfigError> {
        if contents.len() > MAXIMUM_NOTE_BYTES {
            return Err(note_too_large());
        }
        let existing_bytes = self
            .notes
            .iter()
            .filter(|note| Some(note.id.as_str()) != replacing_id)
            .map(|note| note.contents.len())
            .sum::<usize>();
        if existing_bytes.saturating_add(contents.len()) > MAXIMUM_LOADED_BYTES {
            return Err(ConfigError::invalid(
                "Kebloop's notes folder has reached its local storage limit.",
            ));
        }
        Ok(())
    }

    fn note_path(&self, id: &str) -> Result<PathBuf, ConfigError> {
        if !is_safe_id(id) {
            return Err(ConfigError::invalid("The note identifier is invalid."));
        }
        Ok(self.directory.join(format!("{id}.md")))
    }

    fn recovery_path(&self, id: &str) -> Result<PathBuf, ConfigError> {
        if !is_safe_id(id) {
            return Err(ConfigError::invalid("The note identifier is invalid."));
        }
        Ok(self.recovery_directory.join(format!("{id}.json")))
    }

    fn sort_notes(&mut self) {
        self.notes
            .sort_by_key(|note| std::cmp::Reverse(note.modified_at));
    }
}

fn note_too_large() -> ConfigError {
    ConfigError::invalid("This note would exceed Kebloop's local note storage limit.")
}

fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn epoch_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn creates_saves_and_reopens_markdown_notes_with_private_permissions() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = MarkdownNotesStore::open(directory.path()).unwrap();
        let note = store
            .create("# Planning\n\nWrite the launch plan.")
            .unwrap();
        assert_eq!(note.title(), "Planning");
        let path = directory
            .path()
            .join("notes")
            .join(format!("{}.md", note.id));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "# Planning\n\nWrite the launch plan."
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(directory.path().join("notes"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        let reopened = MarkdownNotesStore::open(directory.path()).unwrap();
        assert_eq!(
            reopened.note(&note.id).map(|value| value.contents.as_str()),
            Some("# Planning\n\nWrite the launch plan.")
        );
    }

    #[test]
    fn recovery_round_trips_and_save_removes_the_recovery_record() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = MarkdownNotesStore::open(directory.path()).unwrap();
        let note = store.create("").unwrap();
        store
            .write_recovery(&note.id, "draft with **formatting**")
            .unwrap();
        assert!(store.recovery_candidates().contains(&note.id));

        let mut reopened = MarkdownNotesStore::open(directory.path()).unwrap();
        assert!(reopened.recovery_candidates().contains(&note.id));
        assert_eq!(
            reopened.note(&note.id).map(|value| value.contents.as_str()),
            Some("")
        );
        reopened.restore_recovery(&note.id).unwrap();
        assert_eq!(
            reopened.note(&note.id).map(|value| value.contents.as_str()),
            Some("draft with **formatting**")
        );
        assert!(!reopened.recovery_candidates().contains(&note.id));
        assert!(
            !directory
                .path()
                .join("notes/Recovery")
                .join(format!("{}.json", note.id))
                .exists()
        );
    }

    #[test]
    fn recovery_can_be_discarded_without_changing_saved_note_and_rejects_oversized_drafts() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = MarkdownNotesStore::open(directory.path()).unwrap();
        let note = store.create("saved").unwrap();
        store.write_recovery(&note.id, "draft").unwrap();
        store.discard_recovery(&note.id).unwrap();
        assert_eq!(
            store.note(&note.id).map(|value| value.contents.as_str()),
            Some("saved")
        );
        assert!(!store.recovery_candidates().contains(&note.id));

        let oversized = "x".repeat(MAXIMUM_NOTE_BYTES + 1);
        assert!(store.write_recovery(&note.id, &oversized).is_err());
        assert!(store.flush(&note.id, &oversized).is_err());
        assert_eq!(
            fs::read_to_string(
                directory
                    .path()
                    .join("notes")
                    .join(format!("{}.md", note.id))
            )
            .unwrap(),
            "saved"
        );
    }

    #[test]
    fn exports_and_removes_a_note() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = MarkdownNotesStore::open(directory.path()).unwrap();
        let note = store.create("# Share this\n").unwrap();
        let destination = directory.path().join("export.md");
        store.export(&note.id, &destination).unwrap();
        assert_eq!(fs::read_to_string(&destination).unwrap(), "# Share this\n");
        store.write_recovery(&note.id, "unsaved").unwrap();
        store.remove(&note.id).unwrap();
        assert!(store.note(&note.id).is_none());
        assert!(
            !destination
                .parent()
                .unwrap()
                .join("notes")
                .join(format!("{}.md", note.id))
                .exists()
        );
        assert!(!store.recovery_candidates().contains(&note.id));
    }

    #[test]
    fn search_folds_case_and_diacritics() {
        let note = MarkdownNote {
            id: "note-id".to_owned(),
            contents: "# Résumé\nPlan the meeting".to_owned(),
            modified_at: SystemTime::now(),
        };
        assert!(note.matches("resume"));
        assert!(note.matches("MEETING"));
        assert!(!note.matches("tomorrow"));
    }
}
