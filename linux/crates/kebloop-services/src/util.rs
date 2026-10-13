//! Small helpers shared by several modules: text folding, path handling, atomic file writes.

use std::cmp::Ordering;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::error::ConfigError;

/// Case- and diacritic-insensitive folding (the Linux stand-in for Foundation's `folding`).
pub fn fold(text: &str) -> String {
    text.nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Generate a fresh upper-case UUID string used for persisted identifiers.
pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string().to_uppercase()
}

/// `true` when `value` is a canonical hyphenated UUID.
pub fn is_uuid(value: &str) -> bool {
    value.len() == 36 && uuid::Uuid::parse_str(value).is_ok()
}

/// Expand a leading `~` / `~/` using an injected home directory.
pub fn expand_tilde(path: &str, home: &str) -> String {
    if path == "~" {
        home.to_string()
    } else if let Some(rest) = path.strip_prefix("~/") {
        format!("{}/{}", home.trim_end_matches('/'), rest)
    } else {
        path.to_string()
    }
}

/// Lexically normalise a path: collapse `//`, `.` and `..` (no symlink resolution).
pub fn normalize_path(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if matches!(parts.last(), Some(last) if *last != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

/// Natural ("localized standard") comparison: folded, numbers compared by value.
pub fn natural_compare(a: &str, b: &str) -> Ordering {
    fn tokens(text: &str) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        for c in text.chars() {
            let digit = c.is_ascii_digit();
            match out.last_mut() {
                Some((is_digit, run)) if *is_digit == digit => run.push(c),
                _ => out.push((digit, c.to_string())),
            }
        }
        out
    }
    let (left, right) = (tokens(&fold(a)), tokens(&fold(b)));
    for (l, r) in left.iter().zip(right.iter()) {
        let order = if l.0 && r.0 {
            let (ln, rn) = (l.1.trim_start_matches('0'), r.1.trim_start_matches('0'));
            ln.len().cmp(&rn.len()).then_with(|| ln.cmp(rn))
        } else {
            l.1.cmp(&r.1)
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    left.len().cmp(&right.len())
}

/// First free `<name>.corrupt[-N]` sibling, used to set unreadable files aside.
pub fn corrupt_destination(path: &Path) -> std::path::PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut candidate = path.with_file_name(format!("{name}.corrupt"));
    let mut counter = 1;
    while candidate.exists() {
        candidate = path.with_file_name(format!("{name}.corrupt-{counter}"));
        counter += 1;
    }
    candidate
}

/// Create a directory (and parents) with the given mode for newly created directories.
pub fn create_dir_all_mode(dir: &Path, mode: u32) -> Result<(), ConfigError> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(mode)
        .create(dir)
        .map_err(|e| ConfigError::io(dir, e))
}

/// Atomically replace `path` with `bytes`: write a temporary sibling, fsync it, rename over the
/// target and fsync the directory. `file_mode` is applied to the new file.
///
/// A symlinked `path` (for example a dotfiles-managed config) is followed: the real file is
/// replaced, not the link.
pub fn atomic_write(path: &Path, bytes: &[u8], file_mode: u32) -> Result<(), ConfigError> {
    let resolved = fs::canonicalize(path).ok();
    let path = resolved.as_deref().unwrap_or(path);
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(dir) = dir {
        create_dir_all_mode(dir, 0o700)?;
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let temporary = path.with_file_name(format!(".{name}.{}.tmp", new_id()));
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(file_mode)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::set_permissions(&temporary, fs::Permissions::from_mode(file_mode))?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if let Err(source) = result {
        let _ = fs::remove_file(&temporary);
        return Err(ConfigError::io(path, source));
    }
    if let Some(handle) = dir.and_then(|d| fs::File::open(d).ok()) {
        let _ = handle.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folding_ignores_case_and_diacritics() {
        assert_eq!(fold("Résumé"), "resume");
        assert_eq!(fold("Łódź"), "łodz");
    }

    #[test]
    fn normalizes_paths_lexically() {
        assert_eq!(normalize_path("/a/b/../c//./d"), "/a/c/d");
        assert_eq!(normalize_path("/.."), "/");
        assert_eq!(normalize_path("a/../../b"), "../b");
    }

    #[test]
    fn natural_order_compares_numbers_by_value() {
        assert_eq!(natural_compare("file2", "file10"), Ordering::Less);
        assert_eq!(natural_compare("File", "file"), Ordering::Equal);
    }

    #[test]
    fn atomic_write_follows_symlinked_target() {
        let dir = tempfile::tempdir().unwrap();
        let real_dir = dir.path().join("dotfiles");
        fs::create_dir_all(&real_dir).unwrap();
        let real = real_dir.join("config.json");
        fs::write(&real, "old").unwrap();
        let link = dir.path().join("config.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        atomic_write(&link, b"new", 0o600).unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&real).unwrap(), b"new");
        // Temporary files are created next to the real file, not the link.
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
        assert_eq!(fs::read_dir(&real_dir).unwrap().count(), 1);
    }

    #[test]
    fn atomic_write_replaces_and_sets_mode() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("sub/file.json");
        atomic_write(&target, b"one", 0o600).unwrap();
        atomic_write(&target, b"two", 0o600).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"two");
        let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(fs::read_dir(target.parent().unwrap()).unwrap().count(), 1);
    }
}
