//! KWin 6 native scripting API, with authenticated bounded D-Bus replies.
use super::*;
use std::{
    io::Write,
    sync::{Mutex, mpsc},
    time::Duration,
};
use zbus::{
    blocking::{Proxy, connection::Builder, fdo::DBusProxy},
    message::Header,
};
static REQUEST_LOCK: Mutex<()> = Mutex::new(());
const TIMEOUT: Duration = Duration::from_secs(2);
const SCRIPT: &str = include_str!("../../../../integrations/kwin/request.js");
pub const CAPABILITIES: CompositorCapabilities = CompositorCapabilities {
    list_windows: true,
    focus: true,
    close: true,
    hide: true,
    exact_geometry: true,
    column_layout: false,
    quit_session: false,
};
#[derive(Debug, Clone, Copy)]
pub struct KWin;
struct Reply {
    sender: String,
    result: mpsc::SyncSender<String>,
}
#[zbus::interface(name = "org.kebloop.KWinReply1")]
impl Reply {
    fn complete(&self, payload: &str, #[zbus(header)] header: Header<'_>) -> zbus::fdo::Result<()> {
        if header.sender().map(|s| s.as_str()) != Some(self.sender.as_str()) {
            return Err(zbus::fdo::Error::AccessDenied(
                "only KWin may return a result".into(),
            ));
        }
        if payload.len() > 4 * 1024 * 1024 {
            return Err(zbus::fdo::Error::InvalidArgs("result too large".into()));
        }
        self.result
            .try_send(payload.to_owned())
            .map_err(|_| zbus::fdo::Error::Failed("request already completed".into()))
    }
}
fn error(e: impl std::fmt::Display) -> CompositorError {
    CompositorError::Io(format!("KWin scripting: {e}"))
}
fn script(request: &Value, callback: &str) -> String {
    let mut request = request.clone();
    request["callback"] = Value::String(callback.into());
    format!("const request = {request};\n{SCRIPT}")
}
fn result(json: &str) -> Result<Value, CompositorError> {
    let reply = parse_json(json)?;
    if reply["ok"].as_bool() != Some(true) {
        return Err(CompositorError::Rejected(
            json_str(&reply, "error").unwrap_or_else(|| "invalid KWin result".into()),
        ));
    }
    Ok(reply["value"].clone())
}
fn rect(v: &Value) -> Result<Rect, CompositorError> {
    let get = |key| {
        v.get(key)
            .and_then(json_i32)
            .ok_or_else(|| CompositorError::Protocol("invalid KWin rectangle".into()))
    };
    Ok(Rect::new(get("x")?, get("y")?, get("w")?, get("h")?))
}
impl KWin {
    pub fn new() -> Result<Self, CompositorError> {
        if Self::request(serde_json::json!({"action":"ping"}))?.as_u64() != Some(1) {
            return Err(CompositorError::Protocol(
                "unsupported KWin scripting API".into(),
            ));
        }
        Ok(Self)
    }
    fn request(request: Value) -> Result<Value, CompositorError> {
        // KWin assigns script object paths from its loaded script count. Keep
        // requests serial so unload/load cannot race within this process.
        let _request = REQUEST_LOCK.lock().map_err(error)?;
        let connection = Builder::session()
            .map_err(error)?
            .method_timeout(TIMEOUT)
            .build()
            .map_err(error)?;
        let owner = DBusProxy::new(&connection)
            .map_err(error)?
            .get_name_owner("org.kde.KWin".try_into().map_err(error)?)
            .map_err(error)?;
        let (sender, receiver) = mpsc::sync_channel(1);
        connection
            .object_server()
            .at(
                "/org/kebloop/KWinReply",
                Reply {
                    sender: owner.to_string(),
                    result: sender,
                },
            )
            .map_err(error)?;
        let callback = connection
            .unique_name()
            .ok_or_else(|| error("session bus has no unique name"))?
            .as_str();
        let mut file = tempfile::Builder::new()
            .prefix("kebloop-kwin-")
            .suffix(".js")
            .tempfile()
            .map_err(error)?;
        file.write_all(script(&request, callback).as_bytes())
            .map_err(error)?;
        file.flush().map_err(error)?;
        let name = file
            .path()
            .file_name()
            .ok_or_else(|| error("missing temporary script name"))?
            .to_string_lossy()
            .into_owned();
        // Address the original unique owner, so a replacement service cannot consume this request.
        let proxy = Proxy::new(
            &connection,
            owner.as_str(),
            "/Scripting",
            "org.kde.kwin.Scripting",
        )
        .map_err(error)?;
        let path = file
            .path()
            .to_str()
            .ok_or_else(|| error("temporary path is not UTF-8"))?;
        let id: i32 = proxy
            .call("loadScript", &(path, name.as_str()))
            .map_err(error)?;
        if id < 0 {
            return Err(CompositorError::Rejected(
                "KWin refused the temporary script".into(),
            ));
        }
        let response = (|| {
            let path = format!("/Scripting/Script{id}");
            let runner = Proxy::new(
                &connection,
                owner.as_str(),
                path.as_str(),
                "org.kde.kwin.Script",
            )
            .map_err(error)?;
            runner.call::<_, _, ()>("run", &()).map_err(error)?;
            let json = receiver.recv_timeout(TIMEOUT).map_err(error)?;
            result(&json)
        })();
        // Always remove one-shot scripts, including rejected commands/timeouts.
        let cleanup = proxy
            .call::<_, _, bool>("unloadScript", &(name.as_str(),))
            .map_err(error);
        match response {
            Ok(value) => {
                if !cleanup? {
                    return Err(error("could not unload temporary script"));
                }
                Ok(value)
            }
            Err(e) => Err(e),
        }
    }
    fn command(&self, action: &str, id: &WindowId) -> Result<(), CompositorError> {
        Self::request(serde_json::json!({"action":action,"id":id.0})).map(|_| ())
    }
}
impl Compositor for KWin {
    fn kind(&self) -> CompositorKind {
        CompositorKind::KWin
    }
    fn capabilities(&self) -> CompositorCapabilities {
        CAPABILITIES
    }
    fn windows(&self) -> Result<Vec<Window>, CompositorError> {
        let values = Self::request(serde_json::json!({"action":"windows"}))?;
        values
            .as_array()
            .ok_or_else(|| CompositorError::Protocol("KWin windows is not an array".into()))?
            .iter()
            .map(|v| {
                let frame = rect(&v["frame"])?;
                Ok(Window {
                    id: WindowId(json_str(v, "id").ok_or_else(|| {
                        CompositorError::Protocol("missing KWin window id".into())
                    })?),
                    app_id: json_str(v, "app_id"),
                    title: json_str(v, "title").unwrap_or_default(),
                    pid: json_pid(v),
                    focused: v["focused"].as_bool().unwrap_or(false),
                    floating: true,
                    fullscreen: v["fullscreen"].as_bool().unwrap_or(false),
                    hidden: v["hidden"].as_bool().unwrap_or(false),
                    output: json_str(v, "output"),
                    workspace: json_str(v, "workspace"),
                    frame: Some(frame),
                    size: Some((frame.w, frame.h)),
                    focus_rank: None,
                })
            })
            .collect()
    }
    fn outputs(&self) -> Result<Vec<Output>, CompositorError> {
        let values = Self::request(serde_json::json!({"action":"outputs"}))?;
        values
            .as_array()
            .ok_or_else(|| CompositorError::Protocol("KWin outputs is not an array".into()))?
            .iter()
            .map(|v| {
                Ok(Output {
                    name: json_str(v, "name").ok_or_else(|| {
                        CompositorError::Protocol("missing KWin output name".into())
                    })?,
                    frame: rect(&v["frame"])?,
                    usable: rect(&v["usable"])?,
                    focused: v["focused"].as_bool().unwrap_or(false),
                })
            })
            .collect()
    }
    fn focus_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        self.command("focus", id)
    }
    fn close_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        self.command("close", id)
    }
    fn hide_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        self.command("hide", id)
    }
    fn show_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        self.command("show", id)
    }
    fn quit_session(&self) -> Result<(), CompositorError> {
        Err(CompositorError::Unsupported {
            backend: "KWin",
            what: "ending the session",
        })
    }
    fn layout(&self, command: &LayoutCommand) -> Result<(), CompositorError> {
        match command {
        LayoutCommand::SetFrame {id,frame}=>Self::request(serde_json::json!({"action":"frame","id":id.0,"frame":{"x":frame.x,"y":frame.y,"w":frame.w,"h":frame.h}})).map(|_|()),
        _=>Err(CompositorError::Unsupported {backend:"KWin",what:"tiling/column commands"})
    }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_ids_are_json_data() {
        let text = script(
            &serde_json::json!({"action":"focus","id":"\";throw new Error('injected'); //"}),
            ":1.2",
        );
        let line = text
            .lines()
            .next()
            .unwrap()
            .strip_prefix("const request = ")
            .unwrap()
            .strip_suffix(';')
            .unwrap();
        let request: Value = serde_json::from_str(line).unwrap();
        assert_eq!(request["callback"], ":1.2");
        assert_eq!(request["id"], "\";throw new Error('injected'); //");
    }
    #[test]
    fn reports_script_rejections() {
        assert!(matches!(
            result(r#"{"ok":false,"error":"window no longer exists"}"#),
            Err(CompositorError::Rejected(_))
        ));
        assert_eq!(
            result(r#"{"ok":true,"value":1}"#).unwrap(),
            serde_json::json!(1)
        );
    }
    #[test]
    fn rejects_bad_rectangles() {
        assert!(rect(&serde_json::json!({"x":0,"y":0,"w":"bad","h":20})).is_err());
    }
    #[test]
    #[ignore = "requires KWin 6 with XWayland; creates only its own test window"]
    fn live_controls() {
        use x11rb::{
            connection::Connection,
            protocol::{
                Event,
                xproto::{self, AtomEnum, ConnectionExt as _},
            },
            wrapper::ConnectionExt as _,
        };
        let backend = KWin::new().unwrap();
        let (client, screen) = x11rb::connect(None).unwrap();
        let root = client.setup().roots[screen].root;
        let xid = client.generate_id().unwrap();
        client
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                xid,
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
        let atom = |name: &str| {
            client
                .intern_atom(false, name.as_bytes())
                .unwrap()
                .reply()
                .unwrap()
                .atom
        };
        let delete = atom("WM_DELETE_WINDOW");
        client
            .change_property32(
                xproto::PropMode::REPLACE,
                xid,
                atom("WM_PROTOCOLS"),
                AtomEnum::ATOM,
                &[delete],
            )
            .unwrap()
            .check()
            .unwrap();
        client
            .change_property8(
                xproto::PropMode::REPLACE,
                xid,
                AtomEnum::WM_NAME,
                AtomEnum::STRING,
                b"Kebloop isolated KWin test",
            )
            .unwrap()
            .check()
            .unwrap();
        client
            .change_property8(
                xproto::PropMode::REPLACE,
                xid,
                AtomEnum::WM_CLASS,
                AtomEnum::STRING,
                b"kebloop-kwin-test\0KebloopKWinTest\0",
            )
            .unwrap()
            .check()
            .unwrap();
        client.map_window(xid).unwrap().check().unwrap();
        client.flush().unwrap();
        let wait = |predicate: &dyn Fn(&Window) -> bool| {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(window) = backend
                    .windows()
                    .unwrap()
                    .into_iter()
                    .find(|w| w.title.starts_with("Kebloop isolated KWin test") && predicate(w))
                {
                    break window;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "window state did not update"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        let window = wait(&|_| true);
        backend.focus_window(&window.id).unwrap();
        wait(&|w| w.focused);
        backend.hide_window(&window.id).unwrap();
        wait(&|w| w.hidden);
        backend.show_window(&window.id).unwrap();
        wait(&|w| !w.hidden && w.focused);
        let frame = Rect::new(50, 60, 600, 450);
        backend
            .layout(&LayoutCommand::SetFrame {
                id: window.id.clone(),
                frame,
            })
            .unwrap();
        wait(&|w| w.frame == Some(frame));
        let outputs = backend.outputs().unwrap();
        if let Some(other) = outputs
            .iter()
            .find(|output| window.output.as_deref() != Some(output.name.as_str()))
        {
            let frame = Rect::new(other.usable.x + 20, other.usable.y + 20, 400, 300);
            backend
                .layout(&LayoutCommand::SetFrame {
                    id: window.id.clone(),
                    frame,
                })
                .unwrap();
            wait(&|w| w.frame == Some(frame) && w.output.as_deref() == Some(other.name.as_str()));
        }
        backend.close_window(&window.id).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(Event::ClientMessage(event)) = client.poll_for_event().unwrap()
                && event.window == xid
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
        client.destroy_window(xid).unwrap().check().unwrap();
    }
    #[test]
    #[ignore = "requires a KWin 6 graphical session"]
    fn live_read_only() {
        let backend = KWin::new().unwrap();
        assert!(!backend.outputs().unwrap().is_empty());
        backend.windows().unwrap();
    }
}
