//! Sway and i3: i3-ipc over `$SWAYSOCK` / `$I3SOCK` (`GET_TREE`, `GET_OUTPUTS`,
//! `GET_WORKSPACES`, `RUN_COMMAND`). Both speak the same protocol; the tree of
//! a native Wayland Sway window carries `app_id`, an X11 window the class in
//! `window_properties`.

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

const RUN_COMMAND: u32 = 0;
const GET_WORKSPACES: u32 = 1;
const GET_OUTPUTS: u32 = 3;
const GET_TREE: u32 = 4;

/// Workspace that holds scratchpad (hidden) windows.
const SCRATCH_WORKSPACE: &str = "__i3_scratch";
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone)]
pub struct Sway {
    kind: CompositorKind,
    socket: PathBuf,
    timeout: Duration,
}

impl Sway {
    /// `kind` must be [`CompositorKind::Sway`] or [`CompositorKind::I3`].
    pub fn new(kind: CompositorKind, socket: PathBuf) -> Self {
        Self {
            kind,
            socket,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn query(&self, message_type: u32) -> Result<Value, CompositorError> {
        let body = ipc::i3_exchange(&self.socket, message_type, b"", self.timeout)?;
        parse_json(&String::from_utf8_lossy(&body))
    }

    /// Runs a command; every result entry must report success.
    fn run(&self, command: &str) -> Result<(), CompositorError> {
        let body = ipc::i3_exchange(&self.socket, RUN_COMMAND, command.as_bytes(), self.timeout)?;
        let reply = parse_json(&String::from_utf8_lossy(&body))?;
        let results = reply
            .as_array()
            .ok_or_else(|| CompositorError::Protocol("RUN_COMMAND reply is not an array".into()))?;
        for result in results {
            if result.get("success").and_then(Value::as_bool) != Some(true) {
                let message = json_str(result, "error")
                    .or_else(|| json_str(result, "parse_error"))
                    .unwrap_or_else(|| "command failed".into());
                return Err(CompositorError::Rejected(message));
            }
        }
        Ok(())
    }

    fn on_window(&self, id: &WindowId, command: &str) -> Result<(), CompositorError> {
        let con = container_id(id)?;
        self.run(&format!("[con_id={con}] {command}"))
    }
}

fn container_id(id: &WindowId) -> Result<u64, CompositorError> {
    id.0.parse()
        .map_err(|_| CompositorError::Protocol(format!("not a container id: {id}")))
}

pub(super) fn parse_tree(tree: &Value) -> Vec<Window> {
    let mut windows = Vec::new();
    collect(tree, None, None, 0, &mut windows);
    windows
}

fn children(node: &Value) -> impl Iterator<Item = &Value> {
    ["nodes", "floating_nodes"]
        .into_iter()
        .filter_map(|key| node.get(key).and_then(Value::as_array))
        .flatten()
}

fn collect(
    node: &Value,
    output: Option<&str>,
    workspace: Option<&str>,
    depth: usize,
    windows: &mut Vec<Window>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let kind = node.get("type").and_then(Value::as_str).unwrap_or("");
    let name = node.get("name").and_then(Value::as_str);
    match kind {
        "output" => {
            for child in children(node) {
                collect(child, name, None, depth + 1, windows);
            }
        }
        "workspace" => {
            for child in children(node) {
                collect(child, output, name, depth + 1, windows);
            }
        }
        "con" | "floating_con" => {
            if children(node).next().is_some() {
                for child in children(node) {
                    collect(child, output, workspace, depth + 1, windows);
                }
            } else if let Some(window) = leaf_window(node, kind, output, workspace) {
                windows.push(window);
            }
        }
        _ => {
            for child in children(node) {
                collect(child, output, workspace, depth + 1, windows);
            }
        }
    }
}

fn leaf_window(
    node: &Value,
    kind: &str,
    output: Option<&str>,
    workspace: Option<&str>,
) -> Option<Window> {
    let id = node.get("id")?.as_u64()?;
    let app_id = json_str(node, "app_id").or_else(|| {
        node.get("window_properties")
            .and_then(|p| json_str(p, "class"))
    });
    // A leaf without any application identity is not a client window.
    if app_id.is_none() && node.get("window_properties").is_none() && json_pid(node).is_none() {
        return None;
    }
    let rect = node.get("rect").and_then(|r| {
        Some(Rect::new(
            json_i32(r.get("x")?)?,
            json_i32(r.get("y")?)?,
            json_i32(r.get("width")?)?,
            json_i32(r.get("height")?)?,
        ))
    });
    let fullscreen = node
        .get("fullscreen_mode")
        .and_then(Value::as_i64)
        .is_some_and(|m| m > 0);
    Some(Window {
        id: WindowId(id.to_string()),
        app_id,
        title: json_str(node, "name").unwrap_or_default(),
        pid: json_pid(node),
        focused: node
            .get("focused")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        // i3 wraps floating clients in a `floating_con`, but the client itself is a `con`.
        floating: kind == "floating_con"
            || matches!(
                node.get("floating").and_then(Value::as_str),
                Some("auto_on" | "user_on")
            ),
        fullscreen,
        hidden: workspace == Some(SCRATCH_WORKSPACE),
        output: output.filter(|o| *o != "__i3").map(str::to_owned),
        workspace: workspace.map(str::to_owned),
        size: rect.map(|r| (r.w, r.h)),
        frame: rect,
        focus_rank: None,
    })
}

pub(super) fn parse_outputs(outputs: &Value, workspaces: &Value) -> Vec<Output> {
    let rect_of = |value: &Value| -> Option<Rect> {
        let r = value.get("rect")?;
        Some(Rect::new(
            json_i32(r.get("x")?)?,
            json_i32(r.get("y")?)?,
            json_i32(r.get("width")?)?,
            json_i32(r.get("height")?)?,
        ))
    };
    let mut result: Vec<Output> = outputs
        .as_array()
        .map(|list| {
            list.iter()
                .filter(|o| o.get("active").and_then(Value::as_bool) != Some(false))
                .filter_map(|o| {
                    let name = json_str(o, "name")?;
                    let frame = rect_of(o)?;
                    // The visible workspace's rect already excludes bars and panels.
                    let usable = workspaces
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|w| {
                            w.get("visible").and_then(Value::as_bool) == Some(true)
                                && json_str(w, "output").as_deref() == Some(name.as_str())
                        })
                        .find_map(rect_of)
                        .unwrap_or(frame);
                    Some(Output {
                        name,
                        frame,
                        usable,
                        focused: o.get("focused").and_then(Value::as_bool).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    result.sort_by(|a, b| (a.frame.x, a.frame.y, &a.name).cmp(&(b.frame.x, b.frame.y, &b.name)));
    result
}

impl Compositor for Sway {
    fn kind(&self) -> CompositorKind {
        self.kind
    }

    fn capabilities(&self) -> CompositorCapabilities {
        CAPABILITIES
    }

    fn windows(&self) -> Result<Vec<Window>, CompositorError> {
        Ok(parse_tree(&self.query(GET_TREE)?))
    }

    fn outputs(&self) -> Result<Vec<Output>, CompositorError> {
        let outputs = self.query(GET_OUTPUTS)?;
        let workspaces = self.query(GET_WORKSPACES).unwrap_or(Value::Null);
        Ok(parse_outputs(&outputs, &workspaces))
    }

    fn focus_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        self.on_window(id, "focus")
    }

    fn close_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        self.on_window(id, "kill")
    }

    fn hide_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        self.on_window(id, "move scratchpad")
    }

    fn show_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        let hidden = self
            .windows()?
            .into_iter()
            .find(|w| &w.id == id)
            .is_some_and(|w| w.hidden);
        // `scratchpad show` toggles, so it is only used for parked windows.
        self.on_window(id, if hidden { "scratchpad show" } else { "focus" })
    }

    fn quit_session(&self) -> Result<(), CompositorError> {
        self.run("exit")
    }

    fn layout(&self, command: &LayoutCommand) -> Result<(), CompositorError> {
        match command {
            LayoutCommand::SetFrame { id, frame } => self.on_window(
                id,
                &format!(
                    "floating enable, resize set width {} px height {} px, \
                     move absolute position {} px {} px",
                    frame.w, frame.h, frame.x, frame.y
                ),
            ),
            LayoutCommand::SetTiled { id } => self.on_window(id, "floating disable"),
            _ => Err(CompositorError::Unsupported {
                backend: self.kind.name(),
                what: "column-style layout commands are niri-only",
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ipc::fake;
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn tree() -> Value {
        json!({
          "id": 1, "type": "root", "name": "root", "nodes": [
            {"id": 2, "type": "output", "name": "__i3", "nodes": [
              {"id": 3, "type": "workspace", "name": "__i3_scratch", "nodes": [], "floating_nodes": [
                {"id": 40, "type": "floating_con", "name": "hidden", "app_id": "foot", "pid": 400,
                 "focused": false, "fullscreen_mode": 0, "nodes": [], "floating_nodes": [],
                 "rect": {"x": 0, "y": 0, "width": 300, "height": 200}}]}]},
            {"id": 4, "type": "output", "name": "DP-1", "nodes": [
              {"id": 5, "type": "workspace", "name": "1", "nodes": [
                {"id": 6, "type": "con", "name": null, "nodes": [
                  {"id": 10, "type": "con", "name": "Firefox", "app_id": "firefox", "pid": 100,
                   "focused": true, "fullscreen_mode": 0, "nodes": [], "floating_nodes": [],
                   "rect": {"x": 0, "y": 30, "width": 960, "height": 1050}},
                  {"id": 11, "type": "con", "name": "xterm", "app_id": null, "pid": 110,
                   "window_properties": {"class": "XTerm"}, "focused": false, "fullscreen_mode": 1,
                   "nodes": [], "floating_nodes": [],
                   "rect": {"x": 960, "y": 30, "width": 960, "height": 1050}}],
                 "floating_nodes": []}],
               "floating_nodes": [
                {"id": 1200, "type": "floating_con", "nodes": [
                {"id": 12, "type": "con", "floating": "user_on", "name": "calc", "app_id": "calculator", "pid": 120,
                 "focused": false, "fullscreen_mode": 0, "nodes": [], "floating_nodes": [],
                 "rect": {"x": 100, "y": 100, "width": 300, "height": 400}}]}]}],
             "floating_nodes": []}],
          "floating_nodes": []
        })
    }

    #[test]
    fn parses_the_tree() {
        let windows = parse_tree(&tree());
        assert_eq!(windows.len(), 4);
        let by_id = |id: &str| windows.iter().find(|w| w.id.0 == id).unwrap();
        let ff = by_id("10");
        assert_eq!(ff.app_id.as_deref(), Some("firefox"));
        assert_eq!(ff.output.as_deref(), Some("DP-1"));
        assert_eq!(ff.workspace.as_deref(), Some("1"));
        assert!(ff.focused && !ff.floating && !ff.hidden && !ff.fullscreen);
        assert_eq!(ff.frame, Some(Rect::new(0, 30, 960, 1050)));
        let xterm = by_id("11");
        assert_eq!(xterm.app_id.as_deref(), Some("XTerm"));
        assert!(xterm.fullscreen);
        assert!(by_id("12").floating);
        let hidden = by_id("40");
        assert!(hidden.hidden);
        assert_eq!(hidden.output, None);
    }

    #[test]
    fn parses_i3_floating_clients_inside_wrappers() {
        for state in ["auto_on", "user_on", "auto_off", "user_off"] {
            let tree = json!({
                "type": "root", "nodes": [{
                    "type": if state.ends_with("_on") { "floating_con" } else { "con" },
                    "id": 20, "nodes": [{
                        "type": "con", "id": 21, "floating": state,
                        "window_properties": {"class": "XTerm"},
                        "rect": {"x": 100, "y": 100, "width": 300, "height": 200}
                    }]
                }]
            });
            let windows = parse_tree(&tree);
            assert_eq!(windows.len(), 1);
            assert_eq!(windows[0].id.0, "21", "use the client, not its wrapper");
            assert_eq!(windows[0].floating, state.ends_with("_on"), "{state}");
        }
    }

    #[test]
    fn parses_outputs_with_workspace_usable_area() {
        let outputs = json!([
          {"name": "DP-1", "active": true, "focused": true,
           "rect": {"x": 0, "y": 0, "width": 1920, "height": 1080}},
          {"name": "HDMI-A-1", "active": true, "focused": false,
           "rect": {"x": 1920, "y": 0, "width": 1280, "height": 720}},
          {"name": "off", "active": false, "rect": {"x": 0, "y": 0, "width": 1, "height": 1}}
        ]);
        let workspaces = json!([
          {"name": "1", "visible": true, "output": "DP-1",
           "rect": {"x": 0, "y": 30, "width": 1920, "height": 1050}},
          {"name": "2", "visible": false, "output": "HDMI-A-1",
           "rect": {"x": 1920, "y": 99, "width": 1, "height": 1}}
        ]);
        let result = parse_outputs(&outputs, &workspaces);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].usable, Rect::new(0, 30, 1920, 1050));
        assert_eq!(result[1].usable, result[1].frame);
        assert!(result[0].focused);
    }

    fn server(dir: &TempDir) -> (Sway, fake::Log) {
        let path = fake::socket_path(dir.path());
        let log = fake::i3_server(&path, |kind, payload| {
            match kind {
            GET_TREE => tree().to_string(),
            GET_OUTPUTS => {
                r#"[{"name":"DP-1","active":true,"focused":true,"rect":{"x":0,"y":0,"width":1920,"height":1080}}]"#.into()
            }
            GET_WORKSPACES => {
                r#"[{"name":"1","visible":true,"output":"DP-1","rect":{"x":0,"y":30,"width":1920,"height":1050}}]"#.into()
            }
            RUN_COMMAND if payload.contains("bad") => {
                r#"[{"success":true},{"success":false,"error":"Cannot do that"}]"#.into()
            }
            RUN_COMMAND => r#"[{"success":true}]"#.into(),
            _ => "null".into(),
        }
        });
        (Sway::new(CompositorKind::Sway, path), log)
    }

    fn commands(log: &fake::Log) -> Vec<String> {
        fake::log_of(log)
            .into_iter()
            .filter_map(|l| l.strip_prefix("0 ").map(str::to_owned))
            .collect()
    }

    #[test]
    fn restoring_an_i3_floating_client_keeps_it_floating() {
        use crate::window_management::{WindowAction, WindowManager};
        use std::sync::Arc;

        let dir = TempDir::new().unwrap();
        let (sway, log) = server(&dir);
        let manager = WindowManager::new(Arc::new(sway));
        let id = WindowId("12".into());
        manager.perform(WindowAction::LeftHalf, Some(&id)).unwrap();
        manager.perform(WindowAction::Restore, Some(&id)).unwrap();
        let commands = commands(&log);
        assert_eq!(commands.len(), 2);
        assert!(
            commands
                .iter()
                .all(|command| command.contains("floating enable"))
        );
        assert!(commands[1].contains("width 300 px height 400 px"));
        assert!(commands[1].contains("position 100 px 100 px"));
    }

    #[test]
    fn queries_over_the_socket() {
        let dir = TempDir::new().unwrap();
        let (sway, _) = server(&dir);
        assert_eq!(sway.windows().unwrap().len(), 4);
        assert_eq!(sway.focused_window().unwrap().unwrap().id.0, "10");
        let outputs = sway.outputs().unwrap();
        assert_eq!(outputs[0].usable, Rect::new(0, 30, 1920, 1050));
        assert_eq!(sway.kind(), CompositorKind::Sway);
    }

    #[test]
    fn command_wire_format() {
        let dir = TempDir::new().unwrap();
        let (sway, log) = server(&dir);
        let id = WindowId("10".into());
        sway.focus_window(&id).unwrap();
        sway.close_window(&id).unwrap();
        sway.hide_window(&id).unwrap();
        sway.layout(&LayoutCommand::SetFrame {
            id: id.clone(),
            frame: Rect::new(960, 30, 960, 1050),
        })
        .unwrap();
        sway.layout(&LayoutCommand::SetTiled { id: id.clone() })
            .unwrap();
        sway.quit_session().unwrap();
        assert_eq!(
            commands(&log),
            vec![
                "[con_id=10] focus",
                "[con_id=10] kill",
                "[con_id=10] move scratchpad",
                "[con_id=10] floating enable, resize set width 960 px height 1050 px, \
                 move absolute position 960 px 30 px",
                "[con_id=10] floating disable",
                "exit",
            ]
        );
    }

    #[test]
    fn show_uses_scratchpad_only_for_parked_windows() {
        let dir = TempDir::new().unwrap();
        let (sway, log) = server(&dir);
        sway.show_window(&WindowId("40".into())).unwrap();
        sway.show_window(&WindowId("10".into())).unwrap();
        assert_eq!(
            commands(&log),
            vec!["[con_id=40] scratchpad show", "[con_id=10] focus"]
        );
    }

    #[test]
    fn failed_commands_are_reported() {
        let dir = TempDir::new().unwrap();
        let (sway, _) = server(&dir);
        assert!(matches!(
            sway.run("bad"),
            Err(CompositorError::Rejected(m)) if m == "Cannot do that"
        ));
        assert!(matches!(
            sway.focus_window(&WindowId("abc".into())),
            Err(CompositorError::Protocol(_))
        ));
        assert!(matches!(
            sway.layout(&LayoutCommand::CenterColumn {
                id: WindowId("1".into())
            }),
            Err(CompositorError::Unsupported {
                backend: "Sway",
                ..
            })
        ));
    }

    #[test]
    fn i3_reports_its_own_name() {
        let sway = Sway::new(CompositorKind::I3, PathBuf::from("/nonexistent"));
        assert_eq!(sway.kind(), CompositorKind::I3);
        assert!(matches!(sway.windows(), Err(CompositorError::Io(_))));
    }
}
