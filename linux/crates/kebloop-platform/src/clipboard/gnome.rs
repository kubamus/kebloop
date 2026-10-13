//! GNOME Shell bridge: poll cached snapshots, never block on clipboard owners.
use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use zbus::blocking::Proxy;

fn error(e: impl std::fmt::Display) -> ClipboardError {
    ClipboardError::Unavailable(format!("GNOME clipboard bridge: {e}"))
}
fn proxy() -> Result<Proxy<'static>, ClipboardError> {
    let conn = zbus::blocking::connection::Builder::session()
        .map_err(error)?
        .method_timeout(Duration::from_secs(2))
        .build()
        .map_err(error)?;
    Proxy::new(
        &conn,
        "org.kebloop.Gnome",
        "/org/kebloop/Gnome",
        "org.kebloop.Gnome1",
    )
    .map_err(error)
}
pub(super) fn probe() -> ClipboardBackend {
    match proxy().and_then(|p| p.call::<_, _, u32>("Ping", &()).map_err(error)) {
        Ok(1) => ClipboardBackend::Gnome,
        _ => ClipboardBackend::Unavailable(
            "enable the optional Kebloop GNOME Shell extension for clipboard history".into(),
        ),
    }
}
#[derive(Deserialize)]
struct Snapshot {
    revision: u64,
    mimes: Vec<String>,
    data: BTreeMap<String, String>,
    own_write: bool,
    sensitive: bool,
    ready: bool,
}
pub(super) fn start(
    cfg: ClipboardConfig,
) -> Result<(ClipboardWatcher, async_channel::Receiver<ClipboardEvent>), ClipboardError> {
    let p = proxy()?;
    let (events, receiver) = async_channel::bounded(64);
    let (commands, rx) = mpsc::channel();
    let (wake, wake_rx) = UnixStream::pair().map_err(error)?;
    wake.set_nonblocking(true).map_err(error)?;
    let running = Arc::new(AtomicBool::new(true));
    let last_error = Arc::new(Mutex::new(None));
    let watcher = ClipboardWatcher {
        backend: ClipboardBackend::Gnome,
        commands,
        wake,
        running: running.clone(),
        last_error: last_error.clone(),
        recording: cfg.recording.clone(),
    };
    std::thread::Builder::new()
        .name("kebloop-gnome-clipboard".into())
        .spawn(move || {
            let _wake = wake_rx;
            let result = run(&p, &cfg, &rx, &events);
            if let Err(e) = result
                && let Ok(mut slot) = last_error.lock()
            {
                *slot = Some(e.to_string());
            }
            running.store(false, Ordering::Relaxed);
        })
        .map_err(error)?;
    Ok((watcher, receiver))
}
fn run(
    p: &Proxy<'_>,
    cfg: &ClipboardConfig,
    rx: &mpsc::Receiver<Command>,
    events: &async_channel::Sender<ClipboardEvent>,
) -> Result<(), ClipboardError> {
    let mut revision = None;
    let mut delivered = None;
    loop {
        while let Ok(command) = rx.try_recv() {
            match command {
                Command::Stop => {
                    let _ = p.call::<_, _, String>("GetClipboard", &(false,));
                    return Ok(());
                }
                Command::Clear { reply } => {
                    reply(p.call::<_, _, ()>("ClearClipboard", &()).map_err(error))
                }
                Command::Set {
                    data,
                    transient,
                    reply,
                } => {
                    // Mutter's memory source advertises one format. Prefer plain text for
                    // mixed text payloads so ordinary paste remains broadly compatible.
                    let item = data
                        .iter()
                        .find(|(mime, _)| text_rank(mime).is_some())
                        .or_else(|| data.iter().next());
                    let result = match item {
                        Some((mime, bytes)) => p
                            .call::<_, _, ()>(
                                "SetClipboard",
                                &(mime.as_str(), STANDARD.encode(bytes), transient),
                            )
                            .map_err(error),
                        None => Err(error("empty clipboard payload")),
                    };
                    reply(result);
                }
            }
        }
        let recording = cfg.recording.load(Ordering::Relaxed);
        let json: String = p.call("GetClipboard", &(recording,)).map_err(error)?;
        if json.len() > 72 * 1024 * 1024 {
            return Err(error("clipboard snapshot exceeds limit"));
        }
        let mut snap: Snapshot = serde_json::from_str(&json).map_err(error)?;
        if revision != Some(snap.revision) {
            if !snap.own_write {
                cfg.revision.fetch_add(1, Ordering::SeqCst);
            }
            revision = Some(snap.revision);
        }
        if snap.ready && delivered != Some((snap.revision, recording)) {
            if recording && !snap.sensitive {
                let data_json: String = p.call("GetClipboardData", &()).map_err(error)?;
                if data_json.len() > 72 * 1024 * 1024 {
                    return Err(error("clipboard snapshot exceeds limit"));
                }
                let payload: Snapshot = serde_json::from_str(&data_json).map_err(error)?;
                if payload.revision != snap.revision {
                    continue;
                }
                snap.data = payload.data;
            }
            let plan = plan_reads(&snap.mimes, cfg, recording);
            let mut event = ClipboardEvent {
                mimes: snap.mimes,
                sensitive: snap.sensitive || plan.sensitive,
                own_write: snap.own_write,
                ..ClipboardEvent::default()
            };
            if !event.sensitive && recording {
                for item in plan.items {
                    if let Some(encoded) = snap.data.get(&item.offered)
                        && encoded.len() <= item.max.saturating_add(2) / 3 * 4
                        && let Ok(bytes) = STANDARD.decode(encoded)
                        && bytes.len() <= item.max
                    {
                        event.data.insert(item.key, bytes);
                        continue;
                    }
                    event.skipped.push(item.key);
                }
            }
            let _ = events.try_send(event);
            delivered = Some((snap.revision, recording));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
