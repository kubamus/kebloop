//! Subprocess runner: own process group, clean environment (plus session plumbing), bounded input/output, timeout and
//! cancellation (`killpg` SIGTERM, then SIGKILL).

use std::io::{ErrorKind, Read, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::sys::signal::{Signal, killpg};
use nix::sys::wait::{Id, WaitPidFlag, WaitStatus, waitid};
use nix::unistd::Pid;

use super::AutomationError;

/// `PATH` given to every child.
pub const CLEAN_PATH: &str = "/usr/local/bin:/usr/bin:/bin";
/// `LANG` given to every child.
pub const CLEAN_LANG: &str = "C.UTF-8";
/// Non-secret session plumbing forwarded from the parent so tools such as `notify-send`,
/// `wl-copy` or `playerctl` can reach the session. Every other variable is dropped.
pub const SESSION_ENV: [&str; 6] = [
    "HOME",
    "XDG_RUNTIME_DIR",
    "WAYLAND_DISPLAY",
    "DISPLAY",
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_CURRENT_DESKTOP",
];
/// Default timeout for script commands.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
/// Largest timeout the runner accepts.
pub const MAX_TIMEOUT: Duration = Duration::from_secs(300);
/// Default combined stdout+stderr limit (256 KiB).
pub const DEFAULT_OUTPUT_LIMIT: usize = 262_144;
/// Largest output limit the runner accepts.
pub const MAX_OUTPUT_LIMIT: usize = 1_048_576;
/// Largest stdin payload (256 KiB).
pub const MAX_INPUT_BYTES: usize = 262_144;

const POLL_INTERVAL: Duration = Duration::from_millis(5);
const TERM_GRACE: Duration = Duration::from_millis(250);

/// Process groups of the invocations that are running right now. An entry exists exactly while
/// its (possibly exited) leader has not been reaped, so the group id cannot have been recycled.
static LIVE_GROUPS: Mutex<Vec<Pid>> = Mutex::new(Vec::new());

fn live_groups() -> std::sync::MutexGuard<'static, Vec<Pid>> {
    LIVE_GROUPS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Number of subprocess groups currently running.
pub fn running_group_count() -> usize {
    live_groups().len()
}

/// Terminate every running subprocess group synchronously: `SIGTERM`, wait up to `grace` for the
/// group leaders to exit, then `SIGKILL`. For application shutdown, where the supervising threads
/// would otherwise be killed before they can clean up and leave the processes orphaned.
pub fn terminate_all_groups(grace: Duration) {
    for pgid in live_groups().iter() {
        let _ = killpg(*pgid, Signal::SIGTERM);
    }
    let end = Instant::now() + grace;
    loop {
        let alive = live_groups()
            .iter()
            .any(|pgid| !leader_exited(*pgid).unwrap_or(true));
        if !alive || Instant::now() >= end {
            break;
        }
        thread::sleep(POLL_INTERVAL);
    }
    for pgid in live_groups().iter() {
        let _ = killpg(*pgid, Signal::SIGKILL);
    }
}

/// Captured output of a successful run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationOutput {
    /// Standard output (lossy UTF-8).
    pub stdout: String,
    /// Standard error (lossy UTF-8).
    pub stderr: String,
}

/// One subprocess invocation. Arguments are passed literally (never through a shell).
#[derive(Debug, Clone)]
pub struct ExecuteRequest {
    /// Absolute path of the executable.
    pub executable: String,
    /// Literal arguments.
    pub arguments: Vec<String>,
    /// Bytes written to stdin.
    pub input: Vec<u8>,
    /// Wall-clock limit.
    pub timeout: Duration,
    /// Limit for stdout + stderr combined.
    pub output_limit: usize,
}

impl ExecuteRequest {
    /// Request with the default 10 s timeout, 256 KiB output limit and empty stdin.
    pub fn new(executable: impl Into<String>, arguments: Vec<String>) -> Self {
        Self {
            executable: executable.into(),
            arguments,
            input: Vec::new(),
            timeout: DEFAULT_TIMEOUT,
            output_limit: DEFAULT_OUTPUT_LIMIT,
        }
    }

    /// Set stdin bytes.
    pub fn with_input(mut self, input: Vec<u8>) -> Self {
        self.input = input;
        self
    }

    /// Set the timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the combined output limit.
    pub fn with_output_limit(mut self, limit: usize) -> Self {
        self.output_limit = limit;
        self
    }
}

/// Cheap, clonable handle that cancels a run from any thread.
#[derive(Debug, Clone, Default)]
pub struct CancelHandle {
    cancelled: Arc<AtomicBool>,
}

impl CancelHandle {
    /// Request cancellation. The running call returns [`AutomationError::Cancelled`] shortly
    /// after and its whole process group is terminated. Cancelling before the run starts makes
    /// the run fail without launching anything.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Executes one request at a time on the calling thread (run it on a worker thread and keep a
/// [`CancelHandle`]). A runner is single-use for cancellation: once cancelled it stays cancelled.
#[derive(Debug, Clone, Default)]
pub struct AutomationRunner {
    cancel: CancelHandle,
}

impl AutomationRunner {
    /// Fresh, uncancelled runner.
    pub fn new() -> Self {
        Self::default()
    }

    /// Handle that cancels this runner.
    pub fn cancel_handle(&self) -> CancelHandle {
        self.cancel.clone()
    }

    /// Cancel (convenience for `cancel_handle().cancel()`).
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Run the request to completion. Always leaves no process of its group behind.
    pub fn execute(&self, request: &ExecuteRequest) -> Result<AutomationOutput, AutomationError> {
        validate_executable(&request.executable)?;
        if request.timeout.is_zero()
            || request.timeout > MAX_TIMEOUT
            || request.output_limit == 0
            || request.output_limit > MAX_OUTPUT_LIMIT
        {
            return Err(AutomationError::Invalid("Invalid execution limits.".into()));
        }
        if request.input.len() > MAX_INPUT_BYTES {
            return Err(AutomationError::Invalid(
                "Input exceeds 256 KB. Reduce the query or preferences.".into(),
            ));
        }
        if self.cancel.is_cancelled() {
            return Err(AutomationError::Cancelled);
        }

        let mut child = Command::new(&request.executable)
            .args(&request.arguments)
            .env_clear()
            .env("PATH", CLEAN_PATH)
            .env("LANG", CLEAN_LANG)
            .envs(session_env(|name| std::env::var(name).ok()))
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| match e.kind() {
                ErrorKind::NotFound | ErrorKind::PermissionDenied => {
                    AutomationError::MissingExecutable
                }
                _ => AutomationError::Spawn(e.to_string()),
            })?;
        let pgid = Pid::from_raw(i32::try_from(child.id()).unwrap_or(i32::MAX));
        live_groups().push(pgid);

        let result = self.supervise(&mut child, pgid, request);
        // Whatever happened, nothing of the group may outlive the call. The leader is still a
        // zombie here (peeked, not reaped), so its process group id cannot be recycled.
        if result.is_err() {
            let _ = killpg(pgid, Signal::SIGTERM);
            let grace_end = Instant::now() + TERM_GRACE;
            while Instant::now() < grace_end && !leader_exited(pgid).unwrap_or(true) {
                thread::sleep(POLL_INTERVAL);
            }
        }
        {
            // Deregister under the lock that `terminate_all_groups` signals under; the leader is
            // still unreaped, so the id is valid until `wait` below.
            let mut live = live_groups();
            let _ = killpg(pgid, Signal::SIGKILL);
            live.retain(|p| *p != pgid);
        }
        let status = child.wait();

        let (stdout, stderr) = result?;
        let status = status.map_err(|e| AutomationError::Spawn(e.to_string()))?;
        let code = match status.code() {
            Some(code) => code,
            None => 128 + status.signal().unwrap_or(0),
        };
        if code != 0 {
            return Err(AutomationError::Failed(code));
        }
        Ok(AutomationOutput {
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        })
    }

    #[allow(clippy::type_complexity)]
    fn supervise(
        &self,
        child: &mut Child,
        pgid: Pid,
        request: &ExecuteRequest,
    ) -> Result<(Vec<u8>, Vec<u8>), AutomationError> {
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| AutomationError::Spawn("no stdout pipe".into()))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| AutomationError::Spawn("no stderr pipe".into()))?;
        fcntl(&stdout, FcntlArg::F_SETFL(OFlag::O_NONBLOCK))
            .map_err(|e| AutomationError::Spawn(e.to_string()))?;
        fcntl(&stderr, FcntlArg::F_SETFL(OFlag::O_NONBLOCK))
            .map_err(|e| AutomationError::Spawn(e.to_string()))?;

        if let Some(mut stdin) = child.stdin.take()
            && !request.input.is_empty()
        {
            let input = request.input.clone();
            // Detached: it ends when the child reads everything or the pipe breaks.
            thread::spawn(move || {
                let _ = stdin.write_all(&input);
            });
        }

        let deadline = Instant::now() + request.timeout;
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut total = 0usize;
        loop {
            if self.cancel.is_cancelled() {
                return Err(AutomationError::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(AutomationError::Timeout);
            }
            drain(&mut stdout, &mut out, &mut total, request.output_limit)?;
            drain(&mut stderr, &mut err, &mut total, request.output_limit)?;
            match leader_exited(pgid) {
                Ok(true) => break,
                Ok(false) => {}
                Err(e) => return Err(AutomationError::Spawn(e.to_string())),
            }
            thread::sleep(POLL_INTERVAL);
        }
        drain(&mut stdout, &mut out, &mut total, request.output_limit)?;
        drain(&mut stderr, &mut err, &mut total, request.output_limit)?;
        if self.cancel.is_cancelled() {
            return Err(AutomationError::Cancelled);
        }
        Ok((out, err))
    }
}

fn validate_executable(path: &str) -> Result<(), AutomationError> {
    use std::os::unix::fs::PermissionsExt;
    if !path.starts_with('/') || path.contains('\0') {
        return Err(AutomationError::MissingExecutable);
    }
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() && meta.permissions().mode() & 0o111 != 0 => Ok(()),
        _ => Err(AutomationError::MissingExecutable),
    }
}

/// The allow-listed session variables that `lookup` knows (empty values are dropped).
pub fn session_env(lookup: impl Fn(&str) -> Option<String>) -> Vec<(&'static str, String)> {
    SESSION_ENV
        .iter()
        .filter_map(|&name| lookup(name).filter(|v| !v.is_empty()).map(|v| (name, v)))
        .collect()
}

/// Peek whether the process-group leader has exited without reaping it.
fn leader_exited(pgid: Pid) -> Result<bool, nix::Error> {
    let status = waitid(
        Id::Pid(pgid),
        WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG | WaitPidFlag::WNOWAIT,
    )?;
    Ok(!matches!(status, WaitStatus::StillAlive))
}

fn drain(
    reader: &mut impl Read,
    into: &mut Vec<u8>,
    total: &mut usize,
    limit: usize,
) -> Result<(), AutomationError> {
    let mut buffer = [0u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => {
                if *total + count > limit {
                    return Err(AutomationError::OutputLimit);
                }
                *total += count;
                into.extend_from_slice(&buffer[..count]);
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) => return Ok(()),
        }
    }
}
