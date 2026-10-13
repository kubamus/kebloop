//! XFixes selection monitoring. All transfers advance through the event loop so
//! slow selection owners cannot delay revision updates or clipboard commands.
use super::*;
use std::sync::Arc;
use x11rb::connection::Connection as _;
use x11rb::protocol::Event;
use x11rb::protocol::res::{ClientIdMask, ClientIdSpec, ConnectionExt as _};
use x11rb::protocol::xfixes::{ConnectionExt as _, SelectionEventMask};
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{COPY_DEPTH_FROM_PARENT, CURRENT_TIME, NONE};

fn error(e: impl std::fmt::Display) -> ClipboardError {
    ClipboardError::Unavailable(format!("X11: {e}"))
}
fn atom(c: &RustConnection, name: &str) -> Result<Atom, ClipboardError> {
    Ok(c.intern_atom(false, name.as_bytes())
        .map_err(error)?
        .reply()
        .map_err(error)?
        .atom)
}
pub(super) fn probe() -> ClipboardBackend {
    match x11rb::connect(None).map_err(error).and_then(|(c, _)| {
        c.xfixes_query_version(5, 0)
            .map_err(error)?
            .reply()
            .map_err(error)
    }) {
        Ok(_) => ClipboardBackend::X11,
        Err(e) => ClipboardBackend::Unavailable(format!("X11 XFixes unavailable: {e}")),
    }
}
struct Read {
    event: ClipboardEvent,
    items: std::collections::VecDeque<ReadItem>,
    current: Option<ReadItem>,
    bytes: Vec<u8>,
    incremental: bool,
    deadline: Instant,
    total_deadline: Instant,
    targets: bool,
    timestamp: Timestamp,
    target: Atom,
}
struct WriteTransfer {
    window: Window,
    property: Atom,
    target: Atom,
    bytes: Vec<u8>,
    offset: usize,
    deadline: Instant,
}
struct Runtime {
    c: RustConnection,
    window: Window,
    clipboard: Atom,
    targets: Atom,
    incr: Atom,
    property: Atom,
    payload: BTreeMap<Atom, Vec<u8>>,
    read: Option<Read>,
    writes: Vec<WriteTransfer>,
}
impl Runtime {
    fn new() -> Result<Self, ClipboardError> {
        let (c, screen) = x11rb::connect(None).map_err(error)?;
        c.xfixes_query_version(5, 0)
            .map_err(error)?
            .reply()
            .map_err(error)?;
        let window = c.generate_id().map_err(error)?;
        c.create_window(
            COPY_DEPTH_FROM_PARENT,
            window,
            c.setup().roots[screen].root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .map_err(error)?
        .check()
        .map_err(error)?;
        let clipboard = atom(&c, "CLIPBOARD")?;
        c.xfixes_select_selection_input(
            window,
            clipboard,
            SelectionEventMask::SET_SELECTION_OWNER
                | SelectionEventMask::SELECTION_WINDOW_DESTROY
                | SelectionEventMask::SELECTION_CLIENT_CLOSE,
        )
        .map_err(error)?
        .check()
        .map_err(error)?;
        let targets = atom(&c, "TARGETS")?;
        let incr = atom(&c, "INCR")?;
        let property = atom(&c, "KEBLOOP_SELECTION")?;
        c.flush().map_err(error)?;
        Ok(Self {
            c,
            window,
            clipboard,
            targets,
            incr,
            property,
            payload: BTreeMap::new(),
            read: None,
            writes: Vec::new(),
        })
    }
    fn request(&self, target: Atom, timestamp: Timestamp) -> Result<(), ClipboardError> {
        self.c
            .delete_property(self.window, self.property)
            .map_err(error)?;
        self.c
            .convert_selection(
                self.window,
                self.clipboard,
                target,
                self.property,
                timestamp,
            )
            .map_err(error)?;
        self.c.flush().map_err(error)
    }
    fn next(
        &mut self,
        cfg: &ClipboardConfig,
        tx: &async_channel::Sender<ClipboardEvent>,
    ) -> Result<(), ClipboardError> {
        let Some(mut read) = self.read.take() else {
            return Ok(());
        };
        if Instant::now() >= read.total_deadline {
            read.items.clear();
        }
        if let Some(item) = read.items.pop_front() {
            read.deadline = (Instant::now() + cfg.read_timeout).min(read.total_deadline);
            read.bytes.clear();
            read.incremental = false;
            let target = atom(&self.c, &item.offered)?;
            read.target = target;
            read.current = Some(item);
            self.request(target, read.timestamp)?;
            self.read = Some(read);
        } else {
            let _ = tx.try_send(read.event);
        }
        Ok(())
    }
    fn finish_type(
        &mut self,
        success: bool,
        cfg: &ClipboardConfig,
        tx: &async_channel::Sender<ClipboardEvent>,
    ) -> Result<(), ClipboardError> {
        if let Some(read) = self.read.as_mut()
            && let Some(item) = read.current.take()
        {
            if success {
                let bytes = if item.latin1 {
                    latin1_to_utf8(&read.bytes)
                } else {
                    std::mem::take(&mut read.bytes)
                };
                read.event.data.insert(item.key, bytes);
            } else {
                read.event.skipped.push(item.key);
            }
        }
        self.next(cfg, tx)
    }
    fn property(
        &mut self,
        cfg: &ClipboardConfig,
        tx: &async_channel::Sender<ClipboardEvent>,
    ) -> Result<(), ClipboardError> {
        let Some(read) = self.read.as_ref() else {
            return Ok(());
        };
        let max = if read.targets {
            65536
        } else {
            read.current.as_ref().map_or(0, |i| i.max)
        };
        let reply = self
            .c
            .get_property(
                true,
                self.window,
                self.property,
                AtomEnum::ANY,
                0,
                (max / 4 + 1) as u32,
            )
            .map_err(error)?
            .reply()
            .map_err(error)?;
        if reply.type_ == self.incr {
            if read.targets {
                self.read = None;
                self.c
                    .delete_property(self.window, self.property)
                    .map_err(error)?;
                return Ok(());
            }
            if let Some(read) = self.read.as_mut() {
                read.incremental = true;
            }
            self.c
                .delete_property(self.window, self.property)
                .map_err(error)?;
            return Ok(());
        }
        if read.targets {
            if reply.type_ != u32::from(AtomEnum::ATOM) || reply.bytes_after > 0 {
                self.read = None;
                return Ok(());
            }
            let mut mimes = Vec::new();
            let mut name_bytes = 0usize;
            for a in reply.value32().into_iter().flatten().take(4096) {
                let name = self
                    .c
                    .get_atom_name(a)
                    .map_err(error)?
                    .reply()
                    .map_err(error)?;
                if name.name.len() > 1024 {
                    continue;
                }
                name_bytes += name.name.len();
                if name_bytes > 65536 {
                    break;
                }
                if let Ok(name) = String::from_utf8(name.name) {
                    mimes.push(name);
                }
            }
            let plan = plan_reads(&mimes, cfg, cfg.recording.load(Ordering::Relaxed));
            if let Some(read) = self.read.as_mut() {
                read.targets = false;
                read.event.mimes = mimes;
                read.event.sensitive = plan.sensitive;
                read.items = plan.items.into();
            }
            return self.next(cfg, tx);
        }
        let read = self.read.as_mut().unwrap();
        if reply.bytes_after > 0 || read.bytes.len().saturating_add(reply.value.len()) > max {
            return self.finish_type(false, cfg, tx);
        }
        if read.incremental && reply.value.is_empty() {
            return self.finish_type(true, cfg, tx);
        }
        read.bytes.extend(reply.value);
        if !read.incremental {
            self.finish_type(true, cfg, tx)?;
        }
        Ok(())
    }
    fn serve(&mut self, event: SelectionRequestEvent) -> Result<(), ClipboardError> {
        let property = if event.property == NONE {
            event.target
        } else {
            event.property
        };
        let mut accepted = false;
        if event.selection == self.clipboard {
            if event.target == self.targets {
                let mut targets: Vec<_> = self.payload.keys().copied().collect();
                targets.push(self.targets);
                self.c
                    .change_property32(
                        PropMode::REPLACE,
                        event.requestor,
                        property,
                        AtomEnum::ATOM,
                        &targets,
                    )
                    .map_err(error)?;
                accepted = true;
            } else if let Some(bytes) = self.payload.get(&event.target) {
                if bytes.len() <= 64 * 1024 {
                    self.c
                        .change_property8(
                            PropMode::REPLACE,
                            event.requestor,
                            property,
                            event.target,
                            bytes,
                        )
                        .map_err(error)?;
                } else {
                    self.c
                        .change_window_attributes(
                            event.requestor,
                            &ChangeWindowAttributesAux::new()
                                .event_mask(EventMask::PROPERTY_CHANGE),
                        )
                        .map_err(error)?;
                    self.c
                        .change_property32(
                            PropMode::REPLACE,
                            event.requestor,
                            property,
                            self.incr,
                            &[bytes.len() as u32],
                        )
                        .map_err(error)?;
                    self.writes.push(WriteTransfer {
                        window: event.requestor,
                        property,
                        target: event.target,
                        bytes: bytes.clone(),
                        offset: 0,
                        deadline: Instant::now() + Duration::from_secs(10),
                    });
                }
                accepted = true;
            }
        }
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: event.time,
            requestor: event.requestor,
            selection: event.selection,
            target: event.target,
            property: if accepted { property } else { NONE },
        };
        self.c
            .send_event(false, event.requestor, EventMask::NO_EVENT, notify)
            .map_err(error)?;
        Ok(())
    }
    fn chunk(&mut self, event: PropertyNotifyEvent) -> Result<(), ClipboardError> {
        if event.state != Property::DELETE {
            return Ok(());
        }
        if let Some(index) = self
            .writes
            .iter()
            .position(|w| w.window == event.window && w.property == event.atom)
        {
            let w = &mut self.writes[index];
            let end = (w.offset + 64 * 1024).min(w.bytes.len());
            self.c
                .change_property8(
                    PropMode::REPLACE,
                    w.window,
                    w.property,
                    w.target,
                    &w.bytes[w.offset..end],
                )
                .map_err(error)?;
            if w.offset == w.bytes.len() {
                self.writes.swap_remove(index);
            } else {
                w.offset = end;
            }
        }
        Ok(())
    }
}
pub(super) fn start(
    cfg: ClipboardConfig,
) -> Result<(ClipboardWatcher, async_channel::Receiver<ClipboardEvent>), ClipboardError> {
    let mut rt = Runtime::new()?;
    let (events, receiver) = async_channel::bounded(64);
    let (commands, rx) = mpsc::channel();
    let (wake, wake_rx) = UnixStream::pair().map_err(error)?;
    wake.set_nonblocking(true).map_err(error)?;
    let running = Arc::new(AtomicBool::new(true));
    let last_error = Arc::new(Mutex::new(None));
    let watcher = ClipboardWatcher {
        backend: ClipboardBackend::X11,
        commands,
        wake,
        running: running.clone(),
        last_error: last_error.clone(),
        recording: cfg.recording.clone(),
    };
    std::thread::Builder::new()
        .name("kebloop-x11-clipboard".into())
        .spawn(move || {
            let result = run(&mut rt, &cfg, &rx, &events, wake_rx);
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
    rt: &mut Runtime,
    cfg: &ClipboardConfig,
    rx: &mpsc::Receiver<Command>,
    events: &async_channel::Sender<ClipboardEvent>,
    _wake: UnixStream,
) -> Result<(), ClipboardError> {
    loop {
        while let Ok(command) = rx.try_recv() {
            match command {
                Command::Stop => return Ok(()),
                Command::Clear { reply } => {
                    let result =
                        rt.c.set_selection_owner(NONE, rt.clipboard, CURRENT_TIME)
                            .map_err(error)
                            .and_then(|c| c.check().map_err(error));
                    rt.payload.clear();
                    reply(result);
                }
                Command::Set {
                    mut data,
                    transient,
                    reply,
                } => {
                    if transient {
                        data.insert(TRANSIENT_MIME.into(), Vec::new());
                    }
                    let result = (|| {
                        let mut payload = BTreeMap::new();
                        for (name, bytes) in with_text_aliases(data) {
                            payload.insert(atom(&rt.c, &name)?, bytes);
                        }
                        rt.payload = payload;
                        rt.c.set_selection_owner(rt.window, rt.clipboard, CURRENT_TIME)
                            .map_err(error)?
                            .check()
                            .map_err(error)?;
                        rt.c.flush().map_err(error)?;
                        Ok(())
                    })();
                    reply(result);
                }
            }
        }
        for _ in 0..256 {
            let Some(event) = rt.c.poll_for_event().map_err(error)? else {
                break;
            };
            match event {
                Event::XfixesSelectionNotify(event) if event.selection == rt.clipboard => {
                    rt.read = None;
                    if event.owner != rt.window {
                        cfg.revision.fetch_add(1, Ordering::SeqCst);
                        if event.owner != NONE {
                            let now = Instant::now();
                            rt.read = Some(Read {
                                event: ClipboardEvent {
                                    source_app: source_app(&rt.c, event.owner),
                                    ..ClipboardEvent::default()
                                },
                                items: Default::default(),
                                current: None,
                                bytes: Vec::new(),
                                incremental: false,
                                deadline: now + cfg.read_timeout,
                                total_deadline: now + cfg.total_timeout,
                                targets: true,
                                timestamp: event.selection_timestamp,
                                target: rt.targets,
                            });
                            rt.request(rt.targets, event.selection_timestamp)?;
                        }
                    }
                }
                Event::SelectionNotify(event) if event.requestor == rt.window => {
                    if rt
                        .read
                        .as_ref()
                        .is_some_and(|r| event.time == r.timestamp && event.target == r.target)
                    {
                        if event.property == NONE {
                            if rt.read.as_ref().is_some_and(|r| r.targets) {
                                rt.read = None;
                            } else {
                                rt.finish_type(false, cfg, events)?;
                            }
                        } else {
                            rt.property(cfg, events)?;
                        }
                    }
                }
                Event::PropertyNotify(event) => {
                    if event.window == rt.window
                        && event.atom == rt.property
                        && event.state == Property::NEW_VALUE
                        && rt.read.as_ref().is_some_and(|r| r.incremental)
                    {
                        rt.property(cfg, events)?;
                    } else {
                        rt.chunk(event)?;
                    }
                }
                Event::SelectionRequest(event) => rt.serve(event)?,
                _ => {}
            }
        }
        if rt
            .read
            .as_ref()
            .is_some_and(|r| Instant::now() >= r.deadline)
        {
            if rt.read.as_ref().is_some_and(|r| r.targets) {
                rt.read = None;
            } else {
                rt.finish_type(false, cfg, events)?;
            }
        }
        rt.writes.retain(|w| Instant::now() < w.deadline);
        rt.c.flush().map_err(error)?;
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn source_app(c: &RustConnection, owner: Window) -> Option<String> {
    // WM_CLASS belongs to the actual selection owner, never the focused window.
    let class = c
        .get_property(false, owner, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024)
        .ok()?
        .reply()
        .ok()?;
    if let Some(value) = class.value.split(|b| *b == 0).rfind(|s| !s.is_empty()) {
        return String::from_utf8(value.to_vec()).ok();
    }
    // Toolkit clipboard proxy windows lack WM_CLASS. XRes identifies their local
    // client process without trusting a window-supplied PID property.
    let reply = c
        .res_query_client_ids(&[ClientIdSpec {
            client: owner,
            mask: ClientIdMask::LOCAL_CLIENT_PID,
        }])
        .ok()?
        .reply()
        .ok()?;
    let pid = reply.ids.iter().find_map(|id| id.value.first().copied())?;
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()?
        .file_name()?
        .to_str()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn receive(rx: &async_channel::Receiver<ClipboardEvent>) -> ClipboardEvent {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(event) = rx.try_recv() {
                return event;
            }
            assert!(Instant::now() < deadline, "X11 clipboard event timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    #[ignore = "requires a private X11 display (Xvfb); overwrites clipboard"]
    fn private_x11_text_rich_and_incr_roundtrip() {
        let cfg = ClipboardConfig::default();
        let revision = cfg.revision.clone();
        let (_reader, rx) = start(cfg).unwrap();
        let (writer, _) = start(ClipboardConfig::default()).unwrap();
        for size in [16, 300_000] {
            let text = vec![b'x'; size];
            writer
                .set_clipboard(
                    BTreeMap::from([
                        (TEXT_KEY.into(), text.clone()),
                        ("text/html".into(), b"<b>hello</b>".to_vec()),
                        ("image/png".into(), vec![137, 80, 78, 71]),
                    ]),
                    false,
                )
                .unwrap();
            let event = receive(&rx);
            assert_eq!(event.data.get(TEXT_KEY), Some(&text));
            assert_eq!(event.data.get("text/html"), Some(&b"<b>hello</b>".to_vec()));
            assert_eq!(event.data.get("image/png"), Some(&vec![137, 80, 78, 71]));
            assert!(!event.own_write);
        }
        let previous = revision.load(Ordering::SeqCst);
        writer.clear_clipboard().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while revision.load(Ordering::SeqCst) == previous {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    #[ignore = "requires a private X11 display (Xvfb); overwrites clipboard"]
    fn private_x11_sensitive_and_size_limits() {
        let cfg = ClipboardConfig {
            max_text_bytes: 8,
            ..ClipboardConfig::default()
        };
        let (_reader, rx) = start(cfg).unwrap();
        let (writer, _) = start(ClipboardConfig::default()).unwrap();
        writer
            .set_clipboard(
                BTreeMap::from([(TEXT_KEY.into(), b"secret".to_vec())]),
                true,
            )
            .unwrap();
        let event = receive(&rx);
        assert!(event.sensitive);
        assert!(!event.data.contains_key(TEXT_KEY));
        writer
            .set_clipboard(
                BTreeMap::from([(TEXT_KEY.into(), vec![b'x'; 300_000])]),
                false,
            )
            .unwrap();
        let event = receive(&rx);
        assert!(!event.data.contains_key(TEXT_KEY));
        assert!(event.skipped.iter().any(|key| key == TEXT_KEY));
    }
}
