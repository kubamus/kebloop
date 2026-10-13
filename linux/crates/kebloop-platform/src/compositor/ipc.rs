//! Blocking Unix-socket exchanges with a bounded size and timeout.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use super::CompositorError;

/// Replies larger than this are refused (a window tree is a few hundred KB at most).
pub(crate) const MAX_REPLY: u64 = 16 * 1024 * 1024;

fn io_err(error: &std::io::Error) -> CompositorError {
    CompositorError::Io(error.to_string())
}

fn connect(path: &Path, timeout: Duration) -> Result<UnixStream, CompositorError> {
    let stream = UnixStream::connect(path)
        .map_err(|e| CompositorError::Io(format!("{}: {e}", path.display())))?;
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|()| stream.set_write_timeout(Some(timeout)))
        .map_err(|e| io_err(&e))?;
    Ok(stream)
}

/// Sends one line and reads one line (niri).
pub(crate) fn line_exchange(
    path: &Path,
    request: &str,
    timeout: Duration,
) -> Result<String, CompositorError> {
    let mut stream = connect(path, timeout)?;
    stream
        .write_all(request.as_bytes())
        .and_then(|()| stream.write_all(b"\n"))
        .and_then(|()| stream.flush())
        .map_err(|e| io_err(&e))?;
    let mut reader = BufReader::new(stream.take(MAX_REPLY));
    let mut line = String::new();
    let read = reader.read_line(&mut line).map_err(|e| io_err(&e))?;
    if read == 0 {
        return Err(CompositorError::Protocol(
            "connection closed without a reply".into(),
        ));
    }
    Ok(line)
}

/// Sends the request and reads until the peer closes the connection (Hyprland).
pub(crate) fn eof_exchange(
    path: &Path,
    request: &str,
    timeout: Duration,
) -> Result<String, CompositorError> {
    let mut stream = connect(path, timeout)?;
    stream
        .write_all(request.as_bytes())
        .and_then(|()| stream.flush())
        .map_err(|e| io_err(&e))?;
    let mut bytes = Vec::new();
    stream
        .take(MAX_REPLY)
        .read_to_end(&mut bytes)
        .map_err(|e| io_err(&e))?;
    String::from_utf8(bytes).map_err(|e| CompositorError::Protocol(e.to_string()))
}

const I3_MAGIC: &[u8; 6] = b"i3-ipc";

/// One i3-ipc request/reply (sway, i3). Header: magic, length, type (native endian).
pub(crate) fn i3_exchange(
    path: &Path,
    message_type: u32,
    payload: &[u8],
    timeout: Duration,
) -> Result<Vec<u8>, CompositorError> {
    let mut stream = connect(path, timeout)?;
    let length = u32::try_from(payload.len())
        .map_err(|_| CompositorError::Protocol("request too large".into()))?;
    let mut message = Vec::with_capacity(14 + payload.len());
    message.extend_from_slice(I3_MAGIC);
    message.extend_from_slice(&length.to_ne_bytes());
    message.extend_from_slice(&message_type.to_ne_bytes());
    message.extend_from_slice(payload);
    stream
        .write_all(&message)
        .and_then(|()| stream.flush())
        .map_err(|e| io_err(&e))?;
    let mut header = [0u8; 14];
    stream.read_exact(&mut header).map_err(|e| io_err(&e))?;
    if &header[..6] != I3_MAGIC {
        return Err(CompositorError::Protocol("bad i3-ipc magic".into()));
    }
    let reply_length = u32::from_ne_bytes([header[6], header[7], header[8], header[9]]);
    if u64::from(reply_length) > MAX_REPLY {
        return Err(CompositorError::Protocol("reply too large".into()));
    }
    let mut body = vec![0u8; reply_length as usize];
    stream.read_exact(&mut body).map_err(|e| io_err(&e))?;
    Ok(body)
}

/// Test helpers: tiny single-purpose socket servers.
#[cfg(test)]
pub(crate) mod fake {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::net::UnixListener;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::thread;

    /// Requests received by a fake server, in order.
    pub(crate) type Log = Arc<Mutex<Vec<String>>>;

    pub(crate) fn log_of(log: &Log) -> Vec<String> {
        log.lock().map(|l| l.clone()).unwrap_or_default()
    }

    pub(crate) fn socket_path(dir: &Path) -> PathBuf {
        dir.join("ipc.sock")
    }

    /// Serves forever (the thread is detached; the test's temp dir goes away).
    /// `reply` maps a request line to a reply line.
    pub(crate) fn line_server(path: &Path, reply: impl Fn(&str) -> String + Send + 'static) -> Log {
        let listener = UnixListener::bind(path).expect("bind");
        let log: Log = Arc::default();
        let shared = Arc::clone(&log);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut line = String::new();
                let mut reader = BufReader::new(&stream);
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    continue;
                }
                let line = line.trim_end().to_owned();
                let answer = reply(&line);
                if let Ok(mut l) = shared.lock() {
                    l.push(line);
                }
                let mut writer = &stream;
                let _ = writer.write_all(answer.as_bytes());
                let _ = writer.write_all(b"\n");
            }
        });
        log
    }

    /// Hyprland style: reads the whole request (until the client stops sending
    /// for a moment), replies, closes.
    pub(crate) fn eof_server(path: &Path, reply: impl Fn(&str) -> String + Send + 'static) -> Log {
        let listener = UnixListener::bind(path).expect("bind");
        let log: Log = Arc::default();
        let shared = Arc::clone(&log);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut buffer = [0u8; 4096];
                let Ok(count) = (&stream).read(&mut buffer) else {
                    continue;
                };
                let request = String::from_utf8_lossy(&buffer[..count]).into_owned();
                let answer = reply(&request);
                if let Ok(mut l) = shared.lock() {
                    l.push(request);
                }
                let mut writer = &stream;
                let _ = writer.write_all(answer.as_bytes());
            }
        });
        log
    }

    /// i3-ipc style: log entries are `"<type> <payload>"`.
    pub(crate) fn i3_server(
        path: &Path,
        reply: impl Fn(u32, &str) -> String + Send + 'static,
    ) -> Log {
        let listener = UnixListener::bind(path).expect("bind");
        let log: Log = Arc::default();
        let shared = Arc::clone(&log);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut header = [0u8; 14];
                if (&stream).read_exact(&mut header).is_err() {
                    continue;
                }
                let length = u32::from_ne_bytes([header[6], header[7], header[8], header[9]]);
                let kind = u32::from_ne_bytes([header[10], header[11], header[12], header[13]]);
                let mut payload = vec![0u8; length as usize];
                if (&stream).read_exact(&mut payload).is_err() {
                    continue;
                }
                let payload = String::from_utf8_lossy(&payload).into_owned();
                let answer = reply(kind, &payload);
                if let Ok(mut l) = shared.lock() {
                    l.push(format!("{kind} {payload}"));
                }
                let body = answer.as_bytes();
                let mut out = Vec::new();
                out.extend_from_slice(b"i3-ipc");
                out.extend_from_slice(&(body.len() as u32).to_ne_bytes());
                out.extend_from_slice(&kind.to_ne_bytes());
                out.extend_from_slice(body);
                let mut writer = &stream;
                let _ = writer.write_all(&out);
            }
        });
        log
    }
}
