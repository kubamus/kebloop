//! Local dictionary and explicit, cancellable on-device dictation through installed tools.
use crate::automation::{AutomationError, AutomationRunner, CancelHandle, ExecuteRequest};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, Ordering},
};
use std::time::Duration;

pub fn find_tool(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .filter_map(|path| path.canonicalize().ok())
            .find(|path| {
                std::fs::metadata(path).is_ok_and(|metadata| {
                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                })
            })
    })
}
pub fn lookup(word: &str, runner: &AutomationRunner) -> Result<String, String> {
    let word = word.trim();
    if word.is_empty() || word.len() > 256 || word.contains(['\n', '\r', '\0']) {
        return Err("Enter a word (up to 256 bytes).".into());
    }
    let tool = find_tool("sdcv")
        .ok_or("Install sdcv and a StarDict dictionary for offline definitions.")?;
    let output = runner
        .execute(
            &ExecuteRequest::new(
                tool.to_string_lossy(),
                vec!["--non-interactive".into(), "--".into(), word.into()],
            )
            .with_output_limit(64 * 1024),
        )
        .map_err(|e| e.to_string())?;
    if output.stdout.trim().is_empty() {
        Err("No definition found.".into())
    } else {
        Ok(output.stdout)
    }
}
/// Read only a bounded regular settings file; a malformed cache cannot block GTK.
pub fn load_model_path(settings: &Path) -> Result<Option<PathBuf>, String> {
    use std::io::Read;
    let metadata = match std::fs::metadata(settings) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.is_file() || metadata.len() > 4096 {
        return Err("Invalid dictation model setting".into());
    }
    let mut value = String::new();
    std::fs::File::open(settings)
        .map_err(|e| e.to_string())?
        .take(4097)
        .read_to_string(&mut value)
        .map_err(|e| e.to_string())?;
    if value.len() > 4096 || value.contains(['\n', '\r', '\0']) || value.is_empty() {
        return Err("Invalid dictation model setting".into());
    }
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err("Choose an absolute local model path".into());
    }
    Ok(Some(path))
}
/// Stopping finalizes the recording and continues transcription; cancelling discards it.
#[derive(Debug, Clone)]
pub struct DictationControl {
    recorder: AutomationRunner,
    transcriber: AutomationRunner,
    stop: Arc<AtomicBool>,
    phase: Arc<AtomicU8>,
}
impl DictationControl {
    pub fn new() -> Self {
        Self {
            recorder: AutomationRunner::new(),
            transcriber: AutomationRunner::new(),
            stop: Arc::new(AtomicBool::new(false)),
            phase: Arc::new(AtomicU8::new(0)),
        }
    }
    pub fn cancel_handles(&self) -> [CancelHandle; 2] {
        [
            self.recorder.cancel_handle(),
            self.transcriber.cancel_handle(),
        ]
    }
    pub fn stop_recording(&self) -> bool {
        if self.phase.load(Ordering::Acquire) != 0 {
            return false;
        }
        self.stop.store(true, Ordering::Release);
        self.recorder.cancel();
        true
    }
    pub fn cancel(&self) {
        self.recorder.cancel();
        self.transcriber.cancel();
    }
    pub fn is_recording(&self) -> bool {
        self.phase.load(Ordering::Acquire) == 0
    }
}
impl Default for DictationControl {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct DictationTools {
    pub recorder: PathBuf,
    pub whisper: PathBuf,
    pub model: PathBuf,
}
impl DictationTools {
    pub fn discover(model: &Path) -> Result<Self, String> {
        if !model.is_file() {
            return Err("Choose a local whisper.cpp model (.bin) before recording.".into());
        }
        Ok(Self {
            recorder: find_tool("arecord")
                .ok_or("Install alsa-utils (arecord) to record audio.")?,
            whisper: find_tool("whisper-cli")
                .ok_or("Install whisper.cpp (whisper-cli) for on-device transcription.")?,
            model: model.into(),
        })
    }
    pub fn transcribe(
        &self,
        runner: &AutomationRunner,
        directory: &Path,
        seconds: u32,
    ) -> Result<String, String> {
        self.transcribe_impl(runner, runner, None, directory, seconds)
    }
    pub fn transcribe_controlled(
        &self,
        control: &DictationControl,
        directory: &Path,
        seconds: u32,
    ) -> Result<String, String> {
        let result = self.transcribe_impl(
            &control.recorder,
            &control.transcriber,
            Some(control),
            directory,
            seconds,
        );
        control.phase.store(2, Ordering::Release);
        result
    }
    fn transcribe_impl(
        &self,
        recorder: &AutomationRunner,
        transcriber: &AutomationRunner,
        control: Option<&DictationControl>,
        directory: &Path,
        seconds: u32,
    ) -> Result<String, String> {
        if !(1..=120).contains(&seconds) {
            return Err("Recording duration must be 1–120 seconds.".into());
        }
        if transcriber.cancel_handle().is_cancelled() {
            return Err("Recording cancelled".into());
        }
        let audio = PrivateAudio::new(directory)?;
        let recording = recorder.execute(
            &ExecuteRequest::new(
                self.recorder.to_string_lossy(),
                vec![
                    "-q".into(),
                    "-f".into(),
                    "S16_LE".into(),
                    "-r".into(),
                    "16000".into(),
                    "-c".into(),
                    "1".into(),
                    "-d".into(),
                    seconds.to_string(),
                    audio.path.to_string_lossy().into_owned(),
                ],
            )
            .with_timeout(Duration::from_secs(u64::from(seconds) + 5)),
        );
        if transcriber.cancel_handle().is_cancelled() {
            return Err("Recording cancelled".into());
        }
        match recording {
            Ok(_) => {}
            Err(AutomationError::Cancelled)
                if control.is_some_and(|control| control.stop.load(Ordering::Acquire)) => {}
            Err(error) => return Err(error.to_string()),
        }
        // arecord finalizes the WAV header on SIGTERM. An immediate stop can produce
        // no samples; never invoke transcription on missing or empty audio.
        let metadata = std::fs::symlink_metadata(&audio.path)
            .map_err(|_| "No audio recorded. Record briefly before stopping.".to_owned())?;
        if !metadata.is_file() || metadata.len() <= 44 || metadata.len() > 4 * 1024 * 1024 {
            return Err("No usable audio recorded".into());
        }
        if let Some(control) = control {
            control.phase.store(1, Ordering::Release);
        }
        let output = transcriber
            .execute(
                &ExecuteRequest::new(
                    self.whisper.to_string_lossy(),
                    vec![
                        "-m".into(),
                        self.model.to_string_lossy().into_owned(),
                        "-f".into(),
                        audio.path.to_string_lossy().into_owned(),
                        "--no-timestamps".into(),
                        "-l".into(),
                        "auto".into(),
                    ],
                )
                .with_timeout(Duration::from_secs(180))
                .with_output_limit(64 * 1024),
            )
            .map_err(|e| e.to_string())?;
        let text = output.stdout.trim().to_owned();
        if text.is_empty() {
            Err("No speech recognized.".into())
        } else {
            Ok(text)
        }
    }
}
struct PrivateAudio {
    directory: PathBuf,
    path: PathBuf,
}
impl PrivateAudio {
    fn new(base: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(base).map_err(|e| e.to_string())?;
        let metadata = std::fs::symlink_metadata(base).map_err(|e| e.to_string())?;
        if !metadata.is_dir() {
            return Err("Dictation cache must be a local directory, not a symbolic link".into());
        }
        let directory = base.join(format!("recording-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(|e| e.to_string())?;
        let path = directory.join("audio.wav");
        Ok(Self { directory, path })
    }
}
impl Drop for PrivateAudio {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_inputs_never_start_tools() {
        let runner = AutomationRunner::new();
        assert!(lookup("", &runner).is_err());
        assert!(lookup("word\ncommand", &runner).is_err());
        let tools = DictationTools {
            recorder: "/does-not-exist".into(),
            whisper: "/does-not-exist".into(),
            model: "/does-not-exist".into(),
        };
        assert!(
            tools
                .transcribe(&runner, Path::new("/does-not-exist"), 121)
                .is_err()
        );
        assert!(DictationTools::discover(Path::new("/does-not-exist")).is_err());
    }
    #[test]
    fn audio_directory_is_private_and_removed_on_drop() {
        let base = tempfile::tempdir().unwrap();
        let audio = PrivateAudio::new(base.path()).unwrap();
        let directory = audio.directory.clone();
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        std::fs::write(&audio.path, "private audio").unwrap();
        drop(audio);
        assert!(!directory.exists());
    }
    #[test]
    fn stopping_does_not_cancel_transcription() {
        let control = DictationControl::new();
        assert!(control.stop_recording());
        let handles = control.cancel_handles();
        assert!(handles[0].is_cancelled());
        assert!(!handles[1].is_cancelled());
        control.cancel();
        assert!(handles[1].is_cancelled());
    }
    #[test]
    fn cancelled_session_does_not_start_tools_or_create_audio() {
        let root = tempfile::tempdir().unwrap();
        let audio = root.path().join("never-created");
        let control = DictationControl::new();
        control.cancel();
        let tools = DictationTools {
            recorder: "/missing".into(),
            whisper: "/missing".into(),
            model: "/missing".into(),
        };
        assert!(
            tools
                .transcribe_controlled(&control, &audio, 10)
                .unwrap_err()
                .contains("cancelled")
        );
        assert!(!audio.exists());
    }
    #[test]
    fn model_settings_are_bounded_and_regular_files() {
        let root = tempfile::tempdir().unwrap();
        let settings = root.path().join("model.txt");
        assert_eq!(load_model_path(&settings).unwrap(), None);
        std::fs::write(&settings, "/models/whisper.bin").unwrap();
        assert_eq!(
            load_model_path(&settings).unwrap(),
            Some(PathBuf::from("/models/whisper.bin"))
        );
        std::fs::write(&settings, "x".repeat(4097)).unwrap();
        assert!(load_model_path(&settings).is_err());
        assert!(load_model_path(root.path()).is_err());
    }
    #[test]
    fn stopped_recording_transcribes_and_cleans_private_audio() {
        let root = tempfile::tempdir().unwrap();
        let recorder = root.path().join("record.sh");
        let whisper = root.path().join("whisper.sh");
        let cache = root.path().join("audio");
        std::fs::write(&recorder,"#!/bin/sh\nfor arg do audio=$arg; done\nprintf 'RIFF0000WAVE000000000000000000000000000000000000000000000000000000' > \"$audio\"\ntrap 'exit 0' TERM\nsleep 10\n").unwrap();
        std::fs::write(&whisper, "#!/bin/sh\nprintf 'local transcript'\n").unwrap();
        for path in [&recorder, &whisper] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let tools = DictationTools {
            recorder,
            whisper,
            model: root.path().join("model.bin"),
        };
        let control = DictationControl::new();
        let worker_control = control.clone();
        let worker_cache = cache.clone();
        let worker = std::thread::spawn(move || {
            tools.transcribe_controlled(&worker_control, &worker_cache, 10)
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let recorded = std::fs::read_dir(&cache)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .any(|entry| {
                    std::fs::metadata(entry.path().join("audio.wav"))
                        .is_ok_and(|meta| meta.len() > 44)
                });
            if recorded {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "recording fixture did not start"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(control.stop_recording());
        assert_eq!(worker.join().unwrap().unwrap(), "local transcript");
        assert!(!control.is_recording());
        assert_eq!(std::fs::read_dir(cache).unwrap().count(), 0);
    }
}
