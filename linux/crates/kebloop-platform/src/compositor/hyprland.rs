//! Hyprland: requests on `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock`
//! (`j/clients` for JSON, `/dispatch ...` for dispatchers; the reply is `ok`
//! or an error text, and the server closes the connection).

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

use super::{
    Compositor, CompositorCapabilities, CompositorError, CompositorKind, DEFAULT_TIMEOUT,
    LayoutCommand, Output, Rect, Window, WindowId, ipc, json_i32, json_pid, json_str, parse_json,
};

pub(super) const CAPABILITIES: CompositorCapabilities = CompositorCapabilities {
    list_windows: true,
    focus: true,
    close: true,
    hide: true,
    exact_geometry: true,
    column_layout: false,
    quit_session: true,
};

const BACKEND: &str = "Hyprland";

#[derive(Debug, Clone)]
pub struct Hyprland {
    socket: PathBuf,
    timeout: Duration,
}

impl Hyprland {
    pub fn new(socket: PathBuf) -> Self {
        Self {
            socket,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn json(&self, command: &str) -> Result<Value, CompositorError> {
        let text = ipc::eof_exchange(&self.socket, &format!("j/{command}"), self.timeout)?;
        parse_json(&text)
    }

    fn dispatch(&self, dispatcher: &str) -> Result<(), CompositorError> {
        let reply = ipc::eof_exchange(
            &self.socket,
            &format!("/dispatch {dispatcher}"),
            self.timeout,
        )?;
        if reply.trim() == "ok" {
            Ok(())
        } else {
            Err(CompositorError::Rejected(reply.trim().to_owned()))
        }
    }

    fn clients(&self) -> Result<Vec<Window>, CompositorError> {
        let clients = self.json("clients")?;
        let list = clients
            .as_array()
            .ok_or_else(|| CompositorError::Protocol("clients is not an array".into()))?;
        let monitors = self.monitor_names();
        let active = self.active_address();
        Ok(list
            .iter()
            .filter_map(|c| parse_client(c, &monitors, active.as_deref()))
            .collect())
    }

    fn monitor_names(&self) -> Vec<(i64, String)> {
        self.json("monitors")
            .ok()
            .and_then(|m| {
                m.as_array().map(|list| {
                    list.iter()
                        .filter_map(|m| Some((m.get("id")?.as_i64()?, json_str(m, "name")?)))
                        .collect()
                })
            })
            .unwrap_or_default()
    }

    fn active_address(&self) -> Option<String> {
        self.json("activewindow")
            .ok()
            .and_then(|w| json_str(&w, "address"))
    }

    fn active_workspace_id(&self) -> Result<i64, CompositorError> {
        let monitors = self.json("monitors")?;
        let list = monitors
            .as_array()
            .ok_or_else(|| CompositorError::Protocol("monitors is not an array".into()))?;
        list.iter()
            .find(|m| m.get("focused").and_then(Value::as_bool) == Some(true))
            .or_else(|| list.first())
            .and_then(|m| m.pointer("/activeWorkspace/id")?.as_i64())
            .ok_or_else(|| CompositorError::Protocol("no active workspace".into()))
    }
}

fn address(id: &WindowId) -> Result<String, CompositorError> {
    let valid =
        id.0.strip_prefix("0x")
            .is_some_and(|hex| !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit()));
    if valid {
        Ok(format!("address:{}", id.0))
    } else {
        Err(CompositorError::Protocol(format!(
            "not a Hyprland window address: {id}"
        )))
    }
}

pub(super) fn parse_client(
    value: &Value,
    monitors: &[(i64, String)],
    active_address: Option<&str>,
) -> Option<Window> {
    let address = json_str(value, "address")?;
    if value.get("mapped").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let pair = |key: &str| -> Option<(i32, i32)> {
        let a = value.get(key)?.as_array()?;
        Some((json_i32(a.first()?)?, json_i32(a.get(1)?)?))
    };
    let at = pair("at");
    let size = pair("size");
    let frame = at.zip(size).map(|((x, y), (w, h))| Rect::new(x, y, w, h));
    let workspace = value.get("workspace").and_then(|w| json_str(w, "name"));
    let monitor = value.get("monitor").and_then(Value::as_i64);
    let fullscreen = match value.get("fullscreen") {
        Some(Value::Bool(b)) => *b,
        Some(v) => v.as_i64().is_some_and(|n| n > 0),
        None => false,
    };
    let class = json_str(value, "class").or_else(|| json_str(value, "initialClass"));
    let history = value.get("focusHistoryID").and_then(Value::as_u64);
    Some(Window {
        focused: active_address == Some(address.as_str()),
        id: WindowId(address),
        app_id: class,
        title: json_str(value, "title").unwrap_or_default(),
        pid: json_pid(value),
        floating: value
            .get("floating")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        fullscreen,
        hidden: workspace
            .as_deref()
            .is_some_and(|name| name.starts_with("special")),
        output: monitor.and_then(|m| {
            monitors
                .iter()
                .find(|(id, _)| *id == m)
                .map(|(_, name)| name.clone())
        }),
        workspace,
        frame,
        size,
        focus_rank: history.map(|h| u64::MAX - h),
    })
}

pub(super) fn parse_monitor(value: &Value) -> Option<Output> {
    if value.get("disabled").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let name = json_str(value, "name")?;
    let x = json_i32(value.get("x")?)?;
    let y = json_i32(value.get("y")?)?;
    let scale = value
        .get("scale")
        .and_then(Value::as_f64)
        .filter(|s| *s > 0.0)
        .unwrap_or(1.0);
    let (mut w, mut h) = (
        value.get("width")?.as_f64()?,
        value.get("height")?.as_f64()?,
    );
    // Odd transforms are the 90/270 degree rotations.
    if value
        .get("transform")
        .and_then(Value::as_i64)
        .is_some_and(|t| t % 2 != 0)
    {
        std::mem::swap(&mut w, &mut h);
    }
    let width = json_i32(&Value::from((w / scale).round()))?;
    let height = json_i32(&Value::from((h / scale).round()))?;
    let frame = Rect::new(x, y, width, height);
    let reserved = value
        .get("reserved")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(json_i32).collect::<Vec<_>>())
        .filter(|a| a.len() == 4)
        .unwrap_or_else(|| vec![0; 4]);
    let usable = Rect::new(
        frame.x + reserved[0],
        frame.y + reserved[1],
        (frame.w - reserved[0] - reserved[2]).max(1),
        (frame.h - reserved[1] - reserved[3]).max(1),
    );
    Some(Output {
        name,
        frame,
        usable,
        focused: value
            .get("focused")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

impl Compositor for Hyprland {
    fn kind(&self) -> CompositorKind {
        CompositorKind::Hyprland
    }

    fn capabilities(&self) -> CompositorCapabilities {
        CAPABILITIES
    }

    fn windows(&self) -> Result<Vec<Window>, CompositorError> {
        self.clients()
    }

    fn focused_window(&self) -> Result<Option<Window>, CompositorError> {
        let Some(active) = self.active_address() else {
            return Ok(None);
        };
        Ok(self.clients()?.into_iter().find(|w| w.id.0 == active))
    }

    fn outputs(&self) -> Result<Vec<Output>, CompositorError> {
        let monitors = self.json("monitors")?;
        let list = monitors
            .as_array()
            .ok_or_else(|| CompositorError::Protocol("monitors is not an array".into()))?;
        let mut outputs: Vec<Output> = list.iter().filter_map(parse_monitor).collect();
        outputs
            .sort_by(|a, b| (a.frame.x, a.frame.y, &a.name).cmp(&(b.frame.x, b.frame.y, &b.name)));
        Ok(outputs)
    }

    fn focus_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        self.dispatch(&format!("focuswindow {}", address(id)?))
    }

    fn close_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        self.dispatch(&format!("closewindow {}", address(id)?))
    }

    fn hide_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        self.dispatch(&format!("movetoworkspacesilent special,{}", address(id)?))
    }

    fn show_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        let target = address(id)?;
        let hidden = self
            .clients()?
            .into_iter()
            .find(|w| &w.id == id)
            .is_some_and(|w| w.hidden);
        if hidden {
            // Bring it to the workspace the user is looking at; this also focuses it.
            let workspace = self.active_workspace_id()?;
            self.dispatch(&format!("movetoworkspace {workspace},{target}"))
        } else {
            self.dispatch(&format!("focuswindow {target}"))
        }
    }

    fn quit_session(&self) -> Result<(), CompositorError> {
        self.dispatch("exit")
    }

    fn layout(&self, command: &LayoutCommand) -> Result<(), CompositorError> {
        match command {
            LayoutCommand::SetFrame { id, frame } => {
                let target = address(id)?;
                self.dispatch(&format!("setfloating {target}"))?;
                self.dispatch(&format!(
                    "resizewindowpixel exact {} {},{target}",
                    frame.w, frame.h
                ))?;
                self.dispatch(&format!(
                    "movewindowpixel exact {} {},{target}",
                    frame.x, frame.y
                ))
            }
            LayoutCommand::SetTiled { id } => self.dispatch(&format!("settiled {}", address(id)?)),
            _ => Err(CompositorError::Unsupported {
                backend: BACKEND,
                what: "column-style layout commands are niri-only",
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ipc::fake;
    use super::*;
    use tempfile::TempDir;

    const CLIENTS: &str = r#"[
      {"address":"0xaaa","mapped":true,"hidden":false,"at":[10,40],"size":[800,600],
       "workspace":{"id":1,"name":"1"},"floating":false,"monitor":0,"class":"firefox",
       "title":"Web","pid":111,"fullscreen":0,"focusHistoryID":0},
      {"address":"0xbbb","mapped":true,"hidden":false,"at":[0,0],"size":[100,100],
       "workspace":{"id":-99,"name":"special:special"},"floating":true,"monitor":0,
       "class":"kitty","title":"t","pid":222,"fullscreen":2,"focusHistoryID":3},
      {"address":"0xccc","mapped":false,"at":[0,0],"size":[1,1],"class":"ghost"}
    ]"#;
    const MONITORS: &str = r#"[
      {"id":0,"name":"DP-1","width":3840,"height":2160,"x":0,"y":0,"scale":2.0,"transform":0,
       "reserved":[0,30,0,0],"focused":true,"activeWorkspace":{"id":1,"name":"1"}},
      {"id":1,"name":"HDMI-A-1","width":1080,"height":1920,"x":1920,"y":0,"scale":1.0,"transform":1,
       "reserved":[0,0,0,0],"focused":false,"activeWorkspace":{"id":2,"name":"2"}}
    ]"#;

    fn server(dir: &TempDir) -> (Hyprland, fake::Log) {
        let path = fake::socket_path(dir.path());
        let log = fake::eof_server(&path, |request| match request {
            "j/clients" => CLIENTS.into(),
            "j/monitors" => MONITORS.into(),
            "j/activewindow" => r#"{"address":"0xaaa","class":"firefox"}"#.into(),
            r if r.starts_with("/dispatch fail") => "Invalid dispatcher".into(),
            r if r.starts_with("/dispatch ") => "ok".into(),
            _ => "unknown request".into(),
        });
        (Hyprland::new(path), log)
    }

    fn dispatches(log: &fake::Log) -> Vec<String> {
        fake::log_of(log)
            .into_iter()
            .filter(|l| l.starts_with("/dispatch"))
            .collect()
    }

    #[test]
    fn parses_clients_and_monitors() {
        let dir = TempDir::new().unwrap();
        let (hypr, _) = server(&dir);
        let windows = hypr.windows().unwrap();
        assert_eq!(windows.len(), 2, "unmapped clients are dropped");
        let a = &windows[0];
        assert_eq!(a.id, WindowId("0xaaa".into()));
        assert_eq!(a.app_id.as_deref(), Some("firefox"));
        assert_eq!(a.frame, Some(Rect::new(10, 40, 800, 600)));
        assert_eq!(a.output.as_deref(), Some("DP-1"));
        assert!(a.focused && !a.hidden && !a.fullscreen);
        let b = &windows[1];
        assert!(b.hidden && b.fullscreen && b.floating && !b.focused);
        assert!(a.focus_rank > b.focus_rank);
        assert_eq!(hypr.focused_window().unwrap().unwrap().id, a.id);

        let outputs = hypr.outputs().unwrap();
        assert_eq!(outputs.len(), 2);
        // 3840x2160 at scale 2 -> 1920x1080 logical, 30 px reserved on top.
        assert_eq!(outputs[0].frame, Rect::new(0, 0, 1920, 1080));
        assert_eq!(outputs[0].usable, Rect::new(0, 30, 1920, 1050));
        // Rotated monitor: width and height swap.
        assert_eq!(outputs[1].frame, Rect::new(1920, 0, 1920, 1080));
        assert!(outputs[0].focused);
    }

    #[test]
    fn empty_active_window_means_none() {
        let dir = TempDir::new().unwrap();
        let path = fake::socket_path(dir.path());
        fake::eof_server(&path, |request| {
            if request == "j/activewindow" {
                "{}".into()
            } else {
                "[]".into()
            }
        });
        assert_eq!(Hyprland::new(path).focused_window().unwrap(), None);
    }

    #[test]
    fn dispatcher_wire_format() {
        let dir = TempDir::new().unwrap();
        let (hypr, log) = server(&dir);
        let id = WindowId("0xaaa".into());
        hypr.focus_window(&id).unwrap();
        hypr.close_window(&id).unwrap();
        hypr.hide_window(&id).unwrap();
        hypr.layout(&LayoutCommand::SetFrame {
            id: id.clone(),
            frame: Rect::new(0, 30, 960, 1050),
        })
        .unwrap();
        hypr.layout(&LayoutCommand::SetTiled { id: id.clone() })
            .unwrap();
        hypr.quit_session().unwrap();
        assert_eq!(
            dispatches(&log),
            vec![
                "/dispatch focuswindow address:0xaaa",
                "/dispatch closewindow address:0xaaa",
                "/dispatch movetoworkspacesilent special,address:0xaaa",
                "/dispatch setfloating address:0xaaa",
                "/dispatch resizewindowpixel exact 960 1050,address:0xaaa",
                "/dispatch movewindowpixel exact 0 30,address:0xaaa",
                "/dispatch settiled address:0xaaa",
                "/dispatch exit",
            ]
        );
    }

    #[test]
    fn show_moves_hidden_windows_to_the_active_workspace() {
        let dir = TempDir::new().unwrap();
        let (hypr, log) = server(&dir);
        hypr.show_window(&WindowId("0xbbb".into())).unwrap();
        hypr.show_window(&WindowId("0xaaa".into())).unwrap();
        assert_eq!(
            dispatches(&log),
            vec![
                "/dispatch movetoworkspace 1,address:0xbbb",
                "/dispatch focuswindow address:0xaaa",
            ]
        );
    }

    #[test]
    fn rejections_and_bad_ids() {
        let dir = TempDir::new().unwrap();
        let (hypr, log) = server(&dir);
        assert!(matches!(
            hypr.focus_window(&WindowId("12".into())),
            Err(CompositorError::Protocol(_))
        ));
        assert!(matches!(
            hypr.focus_window(&WindowId("0xzz".into())),
            Err(CompositorError::Protocol(_))
        ));
        assert!(dispatches(&log).is_empty());
        assert!(matches!(
            hypr.dispatch("fail now"),
            Err(CompositorError::Rejected(m)) if m == "Invalid dispatcher"
        ));
        assert!(matches!(
            hypr.layout(&LayoutCommand::CenterColumn {
                id: WindowId("0xaaa".into())
            }),
            Err(CompositorError::Unsupported { .. })
        ));
    }
}
