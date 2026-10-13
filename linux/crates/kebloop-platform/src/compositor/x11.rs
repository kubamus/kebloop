//! EWMH integration for X11 window managers (including Mutter and KWin).
use super::*;
use std::sync::Mutex;
use x11rb::{
    connection::Connection,
    protocol::{
        randr::ConnectionExt as _,
        xproto::{self, AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask},
    },
    rust_connection::RustConnection,
};

pub const CAPABILITIES: CompositorCapabilities = CompositorCapabilities {
    list_windows: true,
    focus: true,
    close: true,
    hide: true,
    exact_geometry: true,
    column_layout: false,
    quit_session: false,
};
pub struct X11 {
    connection: Mutex<RustConnection>,
    root: u32,
}
fn error(e: impl std::fmt::Display) -> CompositorError {
    CompositorError::Io(e.to_string())
}
impl X11 {
    pub fn new(display: &str) -> Result<Self, CompositorError> {
        let (connection, screen) = x11rb::connect(Some(display)).map_err(error)?;
        let root = connection.setup().roots[screen].root;
        Ok(Self {
            connection: Mutex::new(connection),
            root,
        })
    }
    fn atom(c: &RustConnection, name: &str) -> Result<u32, CompositorError> {
        Ok(c.intern_atom(false, name.as_bytes())
            .map_err(error)?
            .reply()
            .map_err(error)?
            .atom)
    }
    fn property(
        c: &RustConnection,
        window: u32,
        name: &str,
    ) -> Result<xproto::GetPropertyReply, CompositorError> {
        c.get_property(false, window, Self::atom(c, name)?, AtomEnum::ANY, 0, 4096)
            .map_err(error)?
            .reply()
            .map_err(error)
    }
    fn numbers(c: &RustConnection, window: u32, name: &str) -> Result<Vec<u32>, CompositorError> {
        Ok(Self::property(c, window, name)?
            .value32()
            .map(|v| v.collect())
            .unwrap_or_default())
    }
    fn message(
        c: &RustConnection,
        root: u32,
        window: u32,
        name: &str,
        data: [u32; 5],
    ) -> Result<(), CompositorError> {
        let event = ClientMessageEvent::new(32, window, Self::atom(c, name)?, data);
        c.send_event(
            false,
            root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        )
        .map_err(error)?
        .check()
        .map_err(error)?;
        c.flush().map_err(error)
    }
    fn id(id: &WindowId) -> Result<u32, CompositorError> {
        id.0.parse()
            .map_err(|_| CompositorError::Protocol("invalid X11 window id".into()))
    }
    fn request(&self, id: &WindowId, name: &str, data: [u32; 5]) -> Result<(), CompositorError> {
        let c = self.connection.lock().map_err(error)?;
        Self::message(&c, self.root, Self::id(id)?, name, data)
    }
}
fn class_name(bytes: &[u8]) -> Option<String> {
    bytes
        .split(|b| *b == 0)
        .rfind(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
}
fn moveresize_data(frame: Rect, ext: &[u32]) -> [u32; 5] {
    let width = (frame.w
        - ext.first().copied().unwrap_or(0) as i32
        - ext.get(1).copied().unwrap_or(0) as i32)
        .max(1) as u32;
    let height = (frame.h
        - ext.get(2).copied().unwrap_or(0) as i32
        - ext.get(3).copied().unwrap_or(0) as i32)
        .max(1) as u32;
    // NorthWest gravity positions the frame; EWMH width/height specify the client.
    [
        1 | (15 << 8) | (2 << 12),
        frame.x as u32,
        frame.y as u32,
        width,
        height,
    ]
}
fn intersect(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    Rect::new(
        x,
        y,
        (a.right().min(b.right()) - x).max(0),
        (a.bottom().min(b.bottom()) - y).max(0),
    )
}
impl Compositor for X11 {
    fn kind(&self) -> CompositorKind {
        CompositorKind::X11
    }
    fn capabilities(&self) -> CompositorCapabilities {
        CAPABILITIES
    }
    fn windows(&self) -> Result<Vec<Window>, CompositorError> {
        let c = self.connection.lock().map_err(error)?;
        let focused = Self::numbers(&c, self.root, "_NET_ACTIVE_WINDOW")?
            .first()
            .copied();
        let hidden = Self::atom(&c, "_NET_WM_STATE_HIDDEN")?;
        let fullscreen = Self::atom(&c, "_NET_WM_STATE_FULLSCREEN")?;
        let dock = Self::atom(&c, "_NET_WM_WINDOW_TYPE_DOCK")?;
        let desktop = Self::atom(&c, "_NET_WM_WINDOW_TYPE_DESKTOP")?;
        let mut result = Vec::new();
        let mut clients = Self::numbers(&c, self.root, "_NET_CLIENT_LIST_STACKING")?;
        if clients.is_empty() {
            clients = Self::numbers(&c, self.root, "_NET_CLIENT_LIST")?;
        }
        for id in clients {
            // Clients may disappear between listing and reading their properties.
            let types = Self::numbers(&c, id, "_NET_WM_WINDOW_TYPE").unwrap_or_default();
            if types.contains(&dock) || types.contains(&desktop) {
                continue;
            }
            let read = || -> Result<Window, CompositorError> {
                let geometry = c.get_geometry(id).map_err(error)?.reply().map_err(error)?;
                let position = c
                    .translate_coordinates(id, self.root, 0, 0)
                    .map_err(error)?
                    .reply()
                    .map_err(error)?;
                let ext = Self::numbers(&c, id, "_NET_FRAME_EXTENTS")?;
                let left = ext.first().copied().unwrap_or(0) as i32;
                let right = ext.get(1).copied().unwrap_or(0) as i32;
                let top = ext.get(2).copied().unwrap_or(0) as i32;
                let bottom = ext.get(3).copied().unwrap_or(0) as i32;
                let frame = Rect::new(
                    i32::from(position.dst_x) - left,
                    i32::from(position.dst_y) - top,
                    i32::from(geometry.width) + left + right,
                    i32::from(geometry.height) + top + bottom,
                );
                let states = Self::numbers(&c, id, "_NET_WM_STATE")?;
                let mut title = Self::property(&c, id, "_NET_WM_NAME")?.value;
                if title.is_empty() {
                    title = Self::property(&c, id, "WM_NAME")?.value;
                }
                Ok(Window {
                    id: WindowId(id.to_string()),
                    app_id: class_name(&Self::property(&c, id, "WM_CLASS")?.value),
                    title: String::from_utf8_lossy(&title).into_owned(),
                    pid: Self::numbers(&c, id, "_NET_WM_PID")?
                        .first()
                        .copied()
                        .filter(|p| *p > 0),
                    focused: focused == Some(id),
                    floating: true,
                    fullscreen: states.contains(&fullscreen),
                    hidden: states.contains(&hidden),
                    output: None,
                    workspace: Self::numbers(&c, id, "_NET_WM_DESKTOP")?
                        .first()
                        .map(u32::to_string),
                    frame: Some(frame),
                    size: Some((frame.w, frame.h)),
                    focus_rank: None,
                })
            };
            if let Ok(window) = read() {
                result.push(window);
            }
        }
        Ok(result)
    }
    fn outputs(&self) -> Result<Vec<Output>, CompositorError> {
        let c = self.connection.lock().map_err(error)?;
        let desktop = Self::numbers(&c, self.root, "_NET_CURRENT_DESKTOP")?
            .first()
            .copied()
            .unwrap_or(0) as usize;
        let areas = Self::numbers(&c, self.root, "_NET_WORKAREA")?;
        let usable = areas
            .get(desktop * 4..desktop * 4 + 4)
            .map(|v| Rect::new(v[0] as i32, v[1] as i32, v[2] as i32, v[3] as i32));
        let mut outputs = Vec::new();
        if let Ok(cookie) = c.randr_get_monitors(self.root, true)
            && let Ok(reply) = cookie.reply()
        {
            for monitor in reply.monitors {
                let name = c
                    .get_atom_name(monitor.name)
                    .map_err(error)?
                    .reply()
                    .map_err(error)?
                    .name;
                let frame = Rect::new(
                    i32::from(monitor.x),
                    i32::from(monitor.y),
                    i32::from(monitor.width),
                    i32::from(monitor.height),
                );
                outputs.push(Output {
                    name: String::from_utf8_lossy(&name).into_owned(),
                    frame,
                    usable: usable.map(|a| intersect(frame, a)).unwrap_or(frame),
                    focused: monitor.primary,
                });
            }
        }
        if outputs.is_empty() {
            let geometry = c
                .get_geometry(self.root)
                .map_err(error)?
                .reply()
                .map_err(error)?;
            let frame = Rect::new(0, 0, i32::from(geometry.width), i32::from(geometry.height));
            outputs.push(Output {
                name: "X11".into(),
                frame,
                usable: usable.unwrap_or(frame),
                focused: true,
            });
        }
        Ok(outputs)
    }
    fn focus_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        self.request(id, "_NET_ACTIVE_WINDOW", [2, 0, 0, 0, 0])
    }
    fn close_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        self.request(id, "_NET_CLOSE_WINDOW", [0, 2, 0, 0, 0])
    }
    fn hide_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        self.request(id, "WM_CHANGE_STATE", [3, 0, 0, 0, 0])
    }
    fn show_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        let c = self.connection.lock().map_err(error)?;
        c.map_window(Self::id(id)?)
            .map_err(error)?
            .check()
            .map_err(error)?;
        Self::message(
            &c,
            self.root,
            Self::id(id)?,
            "_NET_ACTIVE_WINDOW",
            [2, 0, 0, 0, 0],
        )
    }
    fn quit_session(&self) -> Result<(), CompositorError> {
        Err(CompositorError::Unsupported {
            backend: "X11",
            what: "ending the session",
        })
    }
    fn layout(&self, command: &LayoutCommand) -> Result<(), CompositorError> {
        match command {
            LayoutCommand::SetFrame { id, frame } => {
                let c = self.connection.lock().map_err(error)?;
                let id = Self::id(id)?;
                let max_h = Self::atom(&c, "_NET_WM_STATE_MAXIMIZED_HORZ")?;
                let max_v = Self::atom(&c, "_NET_WM_STATE_MAXIMIZED_VERT")?;
                Self::message(&c, self.root, id, "_NET_WM_STATE", [0, max_h, max_v, 2, 0])?;
                Self::message(
                    &c,
                    self.root,
                    id,
                    "_NET_WM_STATE",
                    [0, Self::atom(&c, "_NET_WM_STATE_FULLSCREEN")?, 0, 2, 0],
                )?;
                let ext = Self::numbers(&c, id, "_NET_FRAME_EXTENTS")?;
                Self::message(
                    &c,
                    self.root,
                    id,
                    "_NET_MOVERESIZE_WINDOW",
                    moveresize_data(*frame, &ext),
                )
            }
            _ => Err(CompositorError::Unsupported {
                backend: "X11",
                what: "tiling/column commands",
            }),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_geometry_accounts_for_decorations_and_negative_monitors() {
        assert_eq!(
            moveresize_data(Rect::new(-1920, 30, 960, 1050), &[5, 5, 30, 5]),
            [1 | (15 << 8) | (2 << 12), (-1920_i32) as u32, 30, 950, 1015]
        );
        assert_eq!(
            moveresize_data(Rect::new(0, 0, 5, 5), &[10, 10, 20, 10])[3..],
            [1, 1]
        );
    }
    #[test]
    fn reads_class_not_instance() {
        assert_eq!(
            class_name(b"terminal\0Org.Terminal\0"),
            Some("Org.Terminal".into())
        );
        assert_eq!(class_name(b""), None);
    }
    #[test]
    fn clips_workarea_per_monitor() {
        assert_eq!(
            intersect(Rect::new(1920, 0, 1920, 1080), Rect::new(0, 30, 3840, 1050)),
            Rect::new(1920, 30, 1920, 1050)
        );
    }
    #[test]
    #[ignore = "requires an X11 session with an EWMH window manager"]
    fn live_read_only() {
        let backend = X11::new(&std::env::var("DISPLAY").expect("DISPLAY")).unwrap();
        assert!(!backend.outputs().unwrap().is_empty());
        backend.windows().unwrap();
    }
    #[test]
    #[ignore = "requires an X11 EWMH window manager; creates only its own test window"]
    fn live_controls() {
        use x11rb::protocol::Event;
        use x11rb::wrapper::ConnectionExt as _;
        let display = std::env::var("DISPLAY").expect("DISPLAY");
        let backend = X11::new(&display).unwrap();
        let (client, screen) = x11rb::connect(Some(&display)).unwrap();
        let root = client.setup().roots[screen].root;
        let id = client.generate_id().unwrap();
        client
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                id,
                root,
                100,
                100,
                400,
                300,
                0,
                xproto::WindowClass::INPUT_OUTPUT,
                0,
                &xproto::CreateWindowAux::new().background_pixel(0xffffff),
            )
            .unwrap()
            .check()
            .unwrap();
        let delete = X11::atom(&client, "WM_DELETE_WINDOW").unwrap();
        client
            .change_property32(
                xproto::PropMode::REPLACE,
                id,
                X11::atom(&client, "WM_PROTOCOLS").unwrap(),
                AtomEnum::ATOM,
                &[delete],
            )
            .unwrap()
            .check()
            .unwrap();
        client
            .change_property8(
                xproto::PropMode::REPLACE,
                id,
                AtomEnum::WM_NAME,
                AtomEnum::STRING,
                b"Kebloop isolated compositor test",
            )
            .unwrap()
            .check()
            .unwrap();
        client
            .change_property8(
                xproto::PropMode::REPLACE,
                id,
                AtomEnum::WM_CLASS,
                AtomEnum::STRING,
                b"kebloop-test\0KebloopTest\0",
            )
            .unwrap()
            .check()
            .unwrap();
        client.map_window(id).unwrap().check().unwrap();
        client.flush().unwrap();
        let window_id = WindowId(id.to_string());
        let wait = |predicate: &dyn Fn(&Window) -> bool| {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            loop {
                if backend
                    .windows()
                    .unwrap()
                    .iter()
                    .any(|w| w.id == window_id && predicate(w))
                {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "window state did not update"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        wait(&|w| w.app_id.as_deref() == Some("KebloopTest"));
        backend.focus_window(&window_id).unwrap();
        wait(&|w| w.focused);
        backend.hide_window(&window_id).unwrap();
        wait(&|w| w.hidden);
        backend.show_window(&window_id).unwrap();
        wait(&|w| !w.hidden && w.focused);
        let frame = Rect::new(50, 60, 600, 450);
        backend
            .layout(&LayoutCommand::SetFrame {
                id: window_id.clone(),
                frame,
            })
            .unwrap();
        wait(&|w| w.frame == Some(frame));
        backend.close_window(&window_id).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(Event::ClientMessage(event)) = client.poll_for_event().unwrap()
                && event.window == id
                && event.data.as_data32()[0] == delete
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "WM_DELETE_WINDOW was not sent"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        client.destroy_window(id).unwrap().check().unwrap();
    }
    #[test]
    fn rejects_invalid_ids() {
        assert!(X11::id(&WindowId("invalid".into())).is_err());
    }
}
