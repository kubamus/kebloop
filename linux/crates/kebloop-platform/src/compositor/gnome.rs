//! Optional GNOME Shell extension bridge over the authenticated session bus.
use super::*;
use zbus::blocking::Proxy;

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
pub struct Gnome;
impl Gnome {
    pub fn new() -> Result<Self, CompositorError> {
        let version: u32 = Self::proxy()?.call("Ping", &()).map_err(error)?;
        if version != 1 {
            return Err(CompositorError::Protocol(
                "unsupported GNOME bridge version".into(),
            ));
        }
        Ok(Self)
    }
    fn proxy() -> Result<Proxy<'static>, CompositorError> {
        let conn = zbus::blocking::connection::Builder::session()
            .map_err(error)?
            .method_timeout(DEFAULT_TIMEOUT)
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
    fn command(&self, value: Value) -> Result<(), CompositorError> {
        Self::proxy()?
            .call::<_, _, ()>("Command", &(value.to_string(),))
            .map_err(error)
    }
}
fn error(e: zbus::Error) -> CompositorError {
    CompositorError::Io(format!(
        "GNOME bridge: {e}; enable the Kebloop Shell extension"
    ))
}
fn rect(v: &Value) -> Result<Rect, CompositorError> {
    let get = |key| {
        v.get(key)
            .and_then(json_i32)
            .ok_or_else(|| CompositorError::Protocol("invalid GNOME frame".into()))
    };
    Ok(Rect::new(get("x")?, get("y")?, get("w")?, get("h")?))
}
impl Compositor for Gnome {
    fn kind(&self) -> CompositorKind {
        CompositorKind::Gnome
    }
    fn capabilities(&self) -> CompositorCapabilities {
        CAPABILITIES
    }
    fn windows(&self) -> Result<Vec<Window>, CompositorError> {
        let json: String = Self::proxy()?.call("Windows", &()).map_err(error)?;
        let values: Vec<Value> =
            serde_json::from_str(&json).map_err(|e| CompositorError::Protocol(e.to_string()))?;
        values
            .into_iter()
            .map(|v| {
                let frame = rect(&v["frame"])?;
                Ok(Window {
                    id: WindowId(
                        json_str(&v, "id")
                            .ok_or_else(|| CompositorError::Protocol("missing GNOME id".into()))?,
                    ),
                    app_id: json_str(&v, "app_id"),
                    title: json_str(&v, "title").unwrap_or_default(),
                    pid: json_pid(&v),
                    focused: v["focused"].as_bool().unwrap_or(false),
                    floating: true,
                    fullscreen: v["fullscreen"].as_bool().unwrap_or(false),
                    hidden: v["hidden"].as_bool().unwrap_or(false),
                    output: json_str(&v, "output"),
                    workspace: json_str(&v, "workspace"),
                    frame: Some(frame),
                    size: Some((frame.w, frame.h)),
                    focus_rank: v["focus_rank"].as_u64(),
                })
            })
            .collect()
    }
    fn outputs(&self) -> Result<Vec<Output>, CompositorError> {
        let json: String = Self::proxy()?.call("Outputs", &()).map_err(error)?;
        let values: Vec<Value> =
            serde_json::from_str(&json).map_err(|e| CompositorError::Protocol(e.to_string()))?;
        values
            .into_iter()
            .map(|v| {
                Ok(Output {
                    name: json_str(&v, "name")
                        .ok_or_else(|| CompositorError::Protocol("missing GNOME monitor".into()))?,
                    frame: rect(&v["frame"])?,
                    usable: rect(&v["usable"])?,
                    focused: v["focused"].as_bool().unwrap_or(false),
                })
            })
            .collect()
    }
    fn focus_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        self.command(serde_json::json!({"action":"focus","id":id.0}))
    }
    fn close_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        self.command(serde_json::json!({"action":"close","id":id.0}))
    }
    fn hide_window(&self, id: &WindowId) -> Result<(), CompositorError> {
        self.command(serde_json::json!({"action":"hide","id":id.0}))
    }
    fn quit_session(&self) -> Result<(), CompositorError> {
        Err(CompositorError::Unsupported {
            backend: "GNOME",
            what: "quit session through window bridge",
        })
    }
    fn layout(&self, command: &LayoutCommand) -> Result<(), CompositorError> {
        match command {
        LayoutCommand::SetFrame{id,frame}=>self.command(serde_json::json!({"action":"frame","id":id.0,"frame":{"x":frame.x,"y":frame.y,"w":frame.w,"h":frame.h}})),
        _=>Err(CompositorError::Unsupported{backend:"GNOME",what:"tiling commands"})}
    }
}

/// Injects the ordinary paste shortcut using Mutter's virtual keyboard.
pub fn simulate_paste(shift: bool) -> Result<(), String> {
    Gnome::proxy()
        .map_err(|e| e.to_string())?
        .call::<_, _, ()>("Paste", &(shift,))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bridge_frames_preserve_negative_monitor_coordinates() {
        let frame = rect(&serde_json::json!({"x":-1920,"y":0,"w":1920,"h":1080})).unwrap();
        assert_eq!(frame, Rect::new(-1920, 0, 1920, 1080));
    }
    #[test]
    fn malformed_bridge_geometry_is_rejected() {
        assert!(rect(&serde_json::json!({"x":0,"y":0,"w":"wide","h":1080})).is_err());
        assert!(rect(&serde_json::json!({"x":0,"y":0,"w":1920})).is_err());
    }
}
