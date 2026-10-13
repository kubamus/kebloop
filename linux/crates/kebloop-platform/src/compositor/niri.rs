//! niri: JSON requests, one line each, over `$NIRI_SOCKET`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};

use super::{
    Compositor, CompositorCapabilities, CompositorError, CompositorKind, Cycle, DEFAULT_TIMEOUT,
    LayoutCommand, Output, PositionChange, Rect, SizeChange, Window, WindowId, ipc, json_i32,
    json_pid, json_str, parse_json,
};

pub(super) const CAPABILITIES: CompositorCapabilities = CompositorCapabilities {
    list_windows: true,
    focus: true,
    close: true,
    // niri has no hidden/minimized state.
    hide: false,
    exact_geometry: false,
    column_layout: true,
    quit_session: true,
};

const BACKEND: &str = "niri";

#[derive(Debug, Clone)]
pub struct Niri {
    socket: PathBuf,
    timeout: Duration,
}

impl Niri {
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

    fn request(&self, request: &Value) -> Result<Value, CompositorError> {
        let line = ipc::line_exchange(&self.socket, &request.to_string(), self.timeout)?;
        let reply = parse_json(&line)?;
        if let Some(error) = reply.get("Err") {
            return Err(CompositorError::Rejected(
                error
                    .as_str()
                    .map_or_else(|| error.to_string(), str::to_owned),
            ));
        }
        reply
            .get("Ok")
            .cloned()
            .ok_or_else(|| CompositorError::Protocol("reply has neither Ok nor Err".into()))
    }

    fn action(&self, action: &Value) -> Result<(), CompositorError> {
        self.request(&json!({ "Action": action })).map(|_| ())
    }

    fn focus_numeric(&self, id: &WindowId) -> Result<u64, CompositorError> {
        parse_id(id)
    }

    /// Actions without an `id` operate on the focused window: focus the target first.
    fn focus_first(&self, id: &WindowId) -> Result<(), CompositorError> {
        let numeric = self.focus_numeric(id)?;
        self.action(&json!({ "FocusWindow": { "id": numeric } }))
    }

    fn workspace_outputs(&self) -> HashMap<u64, String> {
        let Ok(reply) = self.request(&json!("Workspaces")) else {
            return HashMap::new();
        };
        reply
            .get("Workspaces")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|w| Some((w.get("id")?.as_u64()?, json_str(w, "output")?)))
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn parse_id(id: &WindowId) -> Result<u64, CompositorError> {
    id.0.parse()
        .map_err(|_| CompositorError::Protocol(format!("not a niri window id: {id}")))
}

fn size_change(change: SizeChange) -> Value {
    match change {
        SizeChange::Fixed(px) => json!({ "SetFixed": px }),
        SizeChange::Percent(p) => json!({ "SetProportion": p }),
    }
}

fn position_change(change: PositionChange) -> Value {
    match change {
        PositionChange::Fixed(px) => json!({ "SetFixed": px }),
        PositionChange::Percent(p) => json!({ "SetProportion": p }),
    }
}

pub(super) fn parse_window(value: &Value, outputs: &HashMap<u64, String>) -> Option<Window> {
    let id = value.get("id")?.as_u64()?;
    let workspace_id = value.get("workspace_id").and_then(Value::as_u64);
    let size = value
        .pointer("/layout/window_size")
        .and_then(Value::as_array)
        .and_then(|a| Some((json_i32(a.first()?)?, json_i32(a.get(1)?)?)));
    let focus_rank = value
        .get("focus_timestamp")
        .filter(|t| !t.is_null())
        .and_then(|t| {
            let secs = t.get("secs")?.as_u64()?;
            let nanos = t.get("nanos")?.as_u64()?;
            secs.checked_mul(1_000_000_000)?.checked_add(nanos)
        });
    Some(Window {
        id: WindowId(id.to_string()),
        app_id: json_str(value, "app_id"),
        title: json_str(value, "title").unwrap_or_default(),
        pid: json_pid(value),
        focused: value
            .get("is_focused")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        floating: value
            .get("is_floating")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        fullscreen: false,
        hidden: false,
        output: workspace_id.and_then(|w| outputs.get(&w).cloned()),
        workspace: workspace_id.map(|w| w.to_string()),
        frame: None,
        size,
        focus_rank,
    })
}

pub(super) fn parse_output(value: &Value, focused_name: Option<&str>) -> Option<Output> {
    let name = json_str(value, "name")?;
    let logical = value.get("logical").filter(|l| !l.is_null())?;
    let frame = Rect::new(
        json_i32(logical.get("x")?)?,
        json_i32(logical.get("y")?)?,
        json_i32(logical.get("width")?)?,
        json_i32(logical.get("height")?)?,
    );
    Some(Output {
        focused: focused_name == Some(name.as_str()),
        name,
        frame,
        usable: frame,
    })
}

impl Compositor for Niri {
    fn kind(&self) -> CompositorKind {
        CompositorKind::Niri
    }

    fn capabilities(&self) -> CompositorCapabilities {
        CAPABILITIES
    }

    fn windows(&self) -> Result<Vec<Window>, CompositorError> {
        let reply = self.request(&json!("Windows"))?;
        let list = reply
            .get("Windows")
            .and_then(Value::as_array)
            .ok_or_else(|| CompositorError::Protocol("missing Windows".into()))?;
        let outputs = self.workspace_outputs();
        Ok(list
            .iter()
            .filter_map(|w| parse_window(w, &outputs))
            .collect())
    }

    fn focused_window(&self) -> Result<Option<Window>, CompositorError> {
        let reply = self.request(&json!("FocusedWindow"))?;
        let value = reply
            .get("FocusedWindow")
            .ok_or_else(|| CompositorError::Protocol("missing FocusedWindow".into()))?;
        if value.is_null() {
            return Ok(None);
        }
        let outputs = self.workspace_outputs();
        Ok(parse_window(value, &outputs).map(|mut w| {
            w.focused = true;
            w
        }))
    }

    fn outputs(&self) -> Result<Vec<Output>, CompositorError> {
        let reply = self.request(&json!("Outputs"))?;
        let map = reply
            .get("Outputs")
            .and_then(Value::as_object)
            .ok_or_else(|| CompositorError::Protocol("missing Outputs".into()))?;
        let focused = self
            .request(&json!("FocusedOutput"))
            .ok()
            .and_then(|r| r.get("FocusedOutput").and_then(|o| json_str(o, "name")));
        let mut outputs: Vec<Output> = map
            .values()
            .filter_map(|o| parse_output(o, focused.as_deref()))
            .collect();
        // Left to right, then top to bottom: a stable "next/previous" order.
        outputs
            .sort_by(|a, b| (a.frame.x, a.frame.y, &a.name).cmp(&(b.frame.x, b.frame.y, &b.name)));
        Ok(outputs)
    }

    fn focus_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        self.focus_first(id)
    }

    fn close_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        self.action(&json!({ "CloseWindow": { "id": parse_id(id)? } }))
    }

    fn hide_window(&self, _id: &WindowId) -> Result<(), CompositorError> {
        Err(CompositorError::Unsupported {
            backend: BACKEND,
            what: "niri has no hidden or minimized window state",
        })
    }

    fn quit_session(&self) -> Result<(), CompositorError> {
        self.action(&json!({ "Quit": { "skip_confirmation": true } }))
    }

    fn layout(&self, command: &LayoutCommand) -> Result<(), CompositorError> {
        match command {
            LayoutCommand::SetColumnWidth { id, change } => {
                self.focus_first(id)?;
                self.action(&json!({ "SetColumnWidth": { "change": size_change(*change) } }))
            }
            LayoutCommand::SetWindowSize { id, width, height } => {
                let numeric = parse_id(id)?;
                if let Some(change) = width {
                    self.action(&json!({ "SetWindowWidth": {
                        "id": numeric, "change": size_change(*change) } }))?;
                }
                if let Some(change) = height {
                    self.action(&json!({ "SetWindowHeight": {
                        "id": numeric, "change": size_change(*change) } }))?;
                }
                Ok(())
            }
            LayoutCommand::MoveFloating { id, x, y } => {
                self.action(&json!({ "MoveFloatingWindow": {
                    "id": parse_id(id)?,
                    "x": position_change(*x),
                    "y": position_change(*y) } }))
            }
            LayoutCommand::CenterColumn { id } => {
                self.focus_first(id)?;
                self.action(&json!({ "CenterColumn": {} }))
            }
            LayoutCommand::CenterWindow { id } => {
                self.action(&json!({ "CenterWindow": { "id": parse_id(id)? } }))
            }
            LayoutCommand::MoveToOutput { id, cycle } => {
                self.focus_first(id)?;
                let name = match cycle {
                    Cycle::Next => "MoveWindowToMonitorNext",
                    Cycle::Previous => "MoveWindowToMonitorPrevious",
                };
                self.action(&json!({ (name): {} }))
            }
            LayoutCommand::SetFrame { .. } | LayoutCommand::SetTiled { .. } => {
                Err(CompositorError::Unsupported {
                    backend: BACKEND,
                    what: "niri places tiled windows itself; exact frames are not available",
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ipc::fake;
    use super::*;
    use tempfile::TempDir;

    const WINDOW: &str = r#"{"id":22,"title":"term","app_id":"com.mitchellh.ghostty","pid":13756,"workspace_id":1,"is_focused":true,"is_floating":false,"is_urgent":false,"layout":{"pos_in_scrolling_layout":[1,1],"tile_size":[948.0,1184.0],"window_size":[948,1184],"tile_pos_in_workspace_view":null,"window_offset_in_tile":[0.0,0.0]},"focus_timestamp":{"secs":41423,"nanos":5466251}}"#;
    const OUTPUT: &str = r#"{"name":"eDP-1","make":"S","model":"M","serial":null,"physical_size":[300,190],"modes":[],"current_mode":0,"is_custom_mode":false,"vrr_supported":true,"vrr_enabled":true,"logical":{"x":0,"y":0,"width":1920,"height":1200,"scale":1.5,"transform":"Normal"}}"#;

    fn server(dir: &TempDir) -> (Niri, fake::Log) {
        let path = fake::socket_path(dir.path());
        let log = fake::line_server(&path, |request| {
            match serde_json::from_str::<Value>(request).unwrap_or(Value::Null) {
                Value::String(s) if s == "Windows" => {
                    format!(r#"{{"Ok":{{"Windows":[{WINDOW}]}}}}"#)
                }
                Value::String(s) if s == "FocusedWindow" => {
                    format!(r#"{{"Ok":{{"FocusedWindow":{WINDOW}}}}}"#)
                }
                Value::String(s) if s == "Workspaces" => {
                    r#"{"Ok":{"Workspaces":[{"id":1,"idx":1,"name":"1","output":"eDP-1"}]}}"#.into()
                }
                Value::String(s) if s == "Outputs" => {
                    format!(r#"{{"Ok":{{"Outputs":{{"eDP-1":{OUTPUT}}}}}}}"#)
                }
                Value::String(s) if s == "FocusedOutput" => {
                    format!(r#"{{"Ok":{{"FocusedOutput":{OUTPUT}}}}}"#)
                }
                v if v.get("Action").is_some_and(|a| a.get("Fail").is_some()) => {
                    r#"{"Err":"nope"}"#.into()
                }
                v if v.get("Action").is_some() => r#"{"Ok":"Handled"}"#.into(),
                _ => r#"{"Err":"error parsing request"}"#.into(),
            }
        });
        (Niri::new(path), log)
    }

    fn sent(log: &fake::Log) -> Vec<Value> {
        fake::log_of(log)
            .iter()
            .filter_map(|l| serde_json::from_str(l).ok())
            .filter(|v: &Value| v.get("Action").is_some())
            .collect()
    }

    #[test]
    fn lists_windows_outputs_and_focus() {
        let dir = TempDir::new().unwrap();
        let (niri, _) = server(&dir);
        let windows = niri.windows().unwrap();
        assert_eq!(windows.len(), 1);
        let w = &windows[0];
        assert_eq!(w.id, WindowId("22".into()));
        assert_eq!(w.app_id.as_deref(), Some("com.mitchellh.ghostty"));
        assert_eq!(w.pid, Some(13756));
        assert_eq!(w.output.as_deref(), Some("eDP-1"));
        assert_eq!(w.size, Some((948, 1184)));
        assert!(w.focused && !w.floating && !w.hidden);
        assert_eq!(w.focus_rank, Some(41_423_005_466_251));
        assert_eq!(niri.focused_window().unwrap().unwrap().id, w.id);
        let outputs = niri.outputs().unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].frame, Rect::new(0, 0, 1920, 1200));
        assert!(outputs[0].focused);
    }

    #[test]
    fn null_focused_window_and_error_replies() {
        let dir = TempDir::new().unwrap();
        let path = fake::socket_path(dir.path());
        fake::line_server(&path, |request| {
            if request.contains("FocusedWindow") {
                r#"{"Ok":{"FocusedWindow":null}}"#.into()
            } else {
                r#"{"Err":"boom"}"#.into()
            }
        });
        let niri = Niri::new(path);
        assert_eq!(niri.focused_window().unwrap(), None);
        assert!(matches!(
            niri.windows(),
            Err(CompositorError::Rejected(m)) if m == "boom"
        ));
    }

    #[test]
    fn unreachable_socket_is_an_io_error() {
        let dir = TempDir::new().unwrap();
        let niri = Niri::new(dir.path().join("missing.sock"));
        assert!(matches!(niri.windows(), Err(CompositorError::Io(_))));
    }

    #[test]
    fn wire_format_of_actions() {
        let dir = TempDir::new().unwrap();
        let (niri, log) = server(&dir);
        let id = WindowId("5".into());
        niri.focus_window(&id).unwrap();
        niri.close_window(&id).unwrap();
        niri.layout(&LayoutCommand::SetColumnWidth {
            id: id.clone(),
            change: SizeChange::Percent(50.0),
        })
        .unwrap();
        niri.layout(&LayoutCommand::CenterColumn { id: id.clone() })
            .unwrap();
        niri.layout(&LayoutCommand::CenterWindow { id: id.clone() })
            .unwrap();
        niri.layout(&LayoutCommand::MoveToOutput {
            id: id.clone(),
            cycle: Cycle::Next,
        })
        .unwrap();
        niri.layout(&LayoutCommand::SetWindowSize {
            id: id.clone(),
            width: Some(SizeChange::Fixed(800)),
            height: Some(SizeChange::Percent(100.0)),
        })
        .unwrap();
        niri.layout(&LayoutCommand::MoveFloating {
            id: id.clone(),
            x: PositionChange::Percent(50.0),
            y: PositionChange::Fixed(20.0),
        })
        .unwrap();
        niri.quit_session().unwrap();
        // Serializations verified against the real `niri msg action` client.
        let expected = vec![
            json!({"Action":{"FocusWindow":{"id":5}}}),
            json!({"Action":{"CloseWindow":{"id":5}}}),
            json!({"Action":{"FocusWindow":{"id":5}}}),
            json!({"Action":{"SetColumnWidth":{"change":{"SetProportion":50.0}}}}),
            json!({"Action":{"FocusWindow":{"id":5}}}),
            json!({"Action":{"CenterColumn":{}}}),
            json!({"Action":{"CenterWindow":{"id":5}}}),
            json!({"Action":{"FocusWindow":{"id":5}}}),
            json!({"Action":{"MoveWindowToMonitorNext":{}}}),
            json!({"Action":{"SetWindowWidth":{"id":5,"change":{"SetFixed":800}}}}),
            json!({"Action":{"SetWindowHeight":{"id":5,"change":{"SetProportion":100.0}}}}),
            json!({"Action":{"MoveFloatingWindow":{"id":5,
                "x":{"SetProportion":50.0},"y":{"SetFixed":20.0}}}}),
            json!({"Action":{"Quit":{"skip_confirmation":true}}}),
        ];
        assert_eq!(sent(&log), expected);
    }

    #[test]
    fn unsupported_operations_do_not_touch_the_socket() {
        let dir = TempDir::new().unwrap();
        let niri = Niri::new(dir.path().join("missing.sock"));
        let id = WindowId("5".into());
        assert!(matches!(
            niri.hide_window(&id),
            Err(CompositorError::Unsupported { .. })
        ));
        assert!(matches!(
            niri.layout(&LayoutCommand::SetTiled { id: id.clone() }),
            Err(CompositorError::Unsupported { .. })
        ));
        assert!(matches!(
            niri.close_window(&WindowId("x".into())),
            Err(CompositorError::Protocol(_))
        ));
    }

    /// Read-only: lists windows and the focused window of the running niri.
    #[test]
    #[ignore = "requires a running niri session"]
    fn live_read_only() {
        let socket = std::env::var("NIRI_SOCKET").expect("NIRI_SOCKET");
        let niri = Niri::new(PathBuf::from(socket));
        let windows = niri.windows().expect("Windows");
        eprintln!("{} windows", windows.len());
        for w in &windows {
            eprintln!(
                "  {} {:?} pid={:?} out={:?}",
                w.id, w.app_id, w.pid, w.output
            );
        }
        eprintln!(
            "focused: {:?}",
            niri.focused_window().expect("FocusedWindow").map(|w| w.id)
        );
        let outputs = niri.outputs().expect("Outputs");
        assert!(!outputs.is_empty());
        eprintln!("outputs: {outputs:?}");
    }
}
