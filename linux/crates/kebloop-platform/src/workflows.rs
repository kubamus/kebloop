//! Desktop workflows through bounded compositor IPC and AT-SPI (no synthetic keys).
use crate::compositor::{Compositor, LayoutCommand, Rect, Window, WindowId};
use kebloop_services::window_layouts::{
    LayoutFrame, SavedWindow, SavedWindowLayout, adjusted_frame,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedObjectPath;

pub struct DesktopWorkflows {
    compositor: Arc<dyn Compositor>,
}
#[derive(Debug, Clone, Default)]
pub struct LayoutRestoreReport {
    pub restored: usize,
    pub missing: usize,
    pub failed: Vec<String>,
}
impl DesktopWorkflows {
    pub fn new(compositor: Arc<dyn Compositor>) -> Self {
        Self { compositor }
    }
    pub fn search_windows(&self, query: &str) -> Result<Vec<Window>, String> {
        let query = crate::text::fold(query);
        let mut windows = self.compositor.windows().map_err(display)?;
        windows.retain(|w| {
            w.pid != Some(std::process::id())
                && crate::text::fold(&format!(
                    "{} {}",
                    w.app_id.as_deref().unwrap_or(""),
                    w.title
                ))
                .contains(&query)
        });
        windows.sort_by(|a, b| {
            b.focused
                .cmp(&a.focused)
                .then(b.focus_rank.cmp(&a.focus_rank))
                .then(a.title.cmp(&b.title))
        });
        windows.truncate(100);
        Ok(windows)
    }
    pub fn focus_window(&self, id: &WindowId) -> Result<(), String> {
        self.compositor.show_window(id).map_err(display)
    }
    pub fn capture_layout(&self) -> Result<Vec<SavedWindow>, String> {
        if !self.compositor.capabilities().exact_geometry {
            return Err(
                "This compositor does not expose exact window geometry for saved layouts".into(),
            );
        }
        let outputs = self.compositor.outputs().map_err(display)?;
        let mut saved = Vec::new();
        for window in self.compositor.windows().map_err(display)? {
            if saved.len() >= 100 {
                break;
            }
            if window.pid == Some(std::process::id()) || window.hidden || window.fullscreen {
                continue;
            }
            let (Some(frame), Some(app_id)) = (window.frame, window.app_id) else {
                continue;
            };
            let output = outputs
                .iter()
                .find(|output| Some(&output.name) == window.output.as_ref())
                .or_else(|| {
                    outputs
                        .iter()
                        .max_by_key(|output| frame.intersection_area(output.usable))
                });
            let Some(output) = output else {
                continue;
            };
            saved.push(SavedWindow {
                app_id,
                title: window.title,
                output: Some(output.name.clone()),
                frame: portable(frame),
                output_frame: portable(output.usable),
                floating: window.floating,
            });
        }
        if saved.is_empty() {
            return Err("No supported windows to save".into());
        }
        Ok(saved)
    }
    pub fn restore_layout(
        &self,
        layout: &SavedWindowLayout,
    ) -> Result<LayoutRestoreReport, String> {
        if !self.compositor.capabilities().exact_geometry {
            return Err("This compositor does not support saved window geometry".into());
        }
        let windows = self.compositor.windows().map_err(display)?;
        let outputs = self.compositor.outputs().map_err(display)?;
        let mut used = HashSet::new();
        let mut report = LayoutRestoreReport::default();
        // Exact titles first, then only an unambiguous remaining window from that application.
        let deadline = Instant::now() + Duration::from_secs(10);
        for saved in layout.windows.iter().take(100) {
            if Instant::now() >= deadline {
                report.failed.push("Restoration time limit reached".into());
                break;
            }
            let candidates: Vec<_> = windows
                .iter()
                .filter(|w| {
                    !used.contains(&w.id)
                        && w.pid != Some(std::process::id())
                        && w.app_id.as_deref() == Some(&saved.app_id)
                        && !w.hidden
                        && !w.fullscreen
                })
                .collect();
            let exact: Vec<_> = candidates
                .iter()
                .copied()
                .filter(|w| w.title == saved.title)
                .collect();
            let window = if exact.len() == 1 {
                exact.first().copied()
            } else if exact.is_empty() && candidates.len() == 1 {
                candidates.first().copied()
            } else {
                None
            };
            let Some(window) = window else {
                report.missing += 1;
                continue;
            };
            used.insert(window.id.clone());
            let output = outputs
                .iter()
                .find(|o| Some(&o.name) == saved.output.as_ref())
                .or_else(|| outputs.iter().find(|o| o.focused))
                .or_else(|| outputs.first());
            let Some(output) = output else {
                report
                    .failed
                    .push(format!("{}: no display available", saved.title));
                continue;
            };
            let frame = adjusted_frame(saved, portable(output.usable));
            match self.compositor.layout(&LayoutCommand::SetFrame {
                id: window.id.clone(),
                frame: Rect::new(frame.x, frame.y, frame.width, frame.height),
            }) {
                Ok(()) => report.restored += 1,
                Err(error) => report.failed.push(format!("{}: {error}", saved.title)),
            }
        }
        Ok(report)
    }
}
fn portable(frame: Rect) -> LayoutFrame {
    LayoutFrame {
        x: frame.x,
        y: frame.y,
        width: frame.w,
        height: frame.h,
    }
}
fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccessibleTarget {
    pub bus_name: String,
    pub path: OwnedObjectPath,
}
#[derive(Debug, Clone)]
pub struct MenuItem {
    pub target: AccessibleTarget,
    pub pid: u32,
    pub title: String,
    pub path: String,
    pub action_index: i32,
}
#[derive(Debug, Clone)]
pub struct TextSelection {
    pub target: AccessibleTarget,
    pub pid: u32,
    pub text: String,
    pub start: i32,
    pub end: i32,
    pub editable: bool,
}

/// Connect on a worker thread. Every D-Bus method times out after 300 ms; tree walking
/// has a five-second budget and a 256-object cap, so inaccessible applications cannot hang GTK.
pub struct AccessibilityWorkflows {
    connection: Connection,
}
impl AccessibilityWorkflows {
    pub fn connect() -> Result<Self, String> {
        let session = zbus::blocking::connection::Builder::session()
            .map_err(display)?
            .method_timeout(Duration::from_millis(300))
            .build()
            .map_err(display)?;
        let bus = Proxy::new(&session, "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus")
            .map_err(display)?;
        let address: String = bus.call("GetAddress", &()).map_err(display)?;
        let connection = zbus::blocking::connection::Builder::address(address.as_str())
            .map_err(display)?
            .method_timeout(Duration::from_millis(300))
            .build()
            .map_err(display)?;
        Ok(Self { connection })
    }
    fn proxy<'a>(
        &'a self,
        target: &'a AccessibleTarget,
        interface: &'a str,
    ) -> Result<Proxy<'a>, String> {
        // Critical focus/name/count verification must query the application, not a
        // cached property value: some toolkits omit PropertiesChanged emissions.
        zbus::blocking::proxy::Builder::<Proxy<'a>>::new(&self.connection)
            .destination(target.bus_name.as_str())
            .map_err(display)?
            .path(target.path.as_str())
            .map_err(display)?
            .interface(interface)
            .map_err(display)?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .map_err(display)
    }
    fn pid(&self, target: &AccessibleTarget) -> Result<u32, String> {
        let bus = Proxy::new(
            &self.connection,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        )
        .map_err(display)?;
        bus.call("GetConnectionUnixProcessID", &(target.bus_name.as_str(),))
            .map_err(display)
    }
    fn walk(
        &self,
        pid: u32,
        mut visit: impl FnMut(&AccessibleTarget, &str, &str, &Proxy<'_>) -> Result<bool, String>,
    ) -> Result<(), String> {
        if pid == std::process::id() {
            return Err("Choose an application outside Kebloop".into());
        }
        let root = AccessibleTarget {
            bus_name: "org.a11y.atspi.Registry".into(),
            path: OwnedObjectPath::try_from("/org/a11y/atspi/accessible/root").map_err(display)?,
        };
        let accessible = self.proxy(&root, "org.a11y.atspi.Accessible")?;
        let applications: Vec<(String, OwnedObjectPath)> =
            accessible.call("GetChildren", &()).map_err(display)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut queue = VecDeque::new();
        for (bus_name, path) in applications.into_iter().take(100) {
            if Instant::now() >= deadline {
                return Err("Accessibility search timed out".into());
            }
            let target = AccessibleTarget { bus_name, path };
            if self.pid(&target).ok() == Some(pid) {
                queue.push_back((target, String::new()));
            }
        }
        if queue.is_empty() {
            return Err("This application does not expose accessible menus or text".into());
        }
        let mut seen = HashSet::new();
        while let Some((target, parent_path)) = queue.pop_front() {
            if seen.len() >= 256 || Instant::now() >= deadline {
                break;
            }
            if !seen.insert(target.clone()) {
                continue;
            }
            let Ok(accessible) = self.proxy(&target, "org.a11y.atspi.Accessible") else {
                continue;
            };
            let name: String = accessible.get_property("Name").unwrap_or_default();
            let role: String = accessible.call("GetRoleName", &()).unwrap_or_default();
            if visit(&target, &name, &parent_path, &accessible)? {
                return Ok(());
            }
            let branch = if role.contains("menu") && !name.is_empty() {
                if parent_path.is_empty() {
                    name
                } else {
                    format!("{parent_path} → {name}")
                }
            } else {
                parent_path
            };
            let children: Vec<(String, OwnedObjectPath)> =
                accessible.call("GetChildren", &()).unwrap_or_default();
            for (bus_name, path) in children
                .into_iter()
                .take(256usize.saturating_sub(seen.len() + queue.len()))
            {
                queue.push_back((AccessibleTarget { bus_name, path }, branch.clone()));
            }
        }
        Ok(())
    }
    pub fn menu_items(&self, pid: u32) -> Result<Vec<MenuItem>, String> {
        let mut items = Vec::new();
        self.walk(pid, |target, name, path, accessible| {
            let role: String = accessible.call("GetRoleName", &()).unwrap_or_default();
            if name.is_empty() || !role.contains("menu item") || !enabled(accessible)? {
                return Ok(false);
            }
            let action = self.proxy(target, "org.a11y.atspi.Action")?;
            let count: i32 = action.get_property("NActions").unwrap_or(0);
            if count > 0 {
                items.push(MenuItem {
                    target: target.clone(),
                    pid,
                    title: name.into(),
                    path: path.into(),
                    action_index: 0,
                });
            }
            Ok(false)
        })?;
        Ok(items)
    }
    pub fn activate_menu(&self, item: &MenuItem) -> Result<(), String> {
        if self.pid(&item.target)? != item.pid {
            return Err("The application changed".into());
        }
        let accessible = self.proxy(&item.target, "org.a11y.atspi.Accessible")?;
        let name: String = accessible.get_property("Name").map_err(display)?;
        if name != item.title || !enabled(&accessible)? {
            return Err("The menu item changed or is disabled".into());
        }
        let action = self.proxy(&item.target, "org.a11y.atspi.Action")?;
        let performed: bool = action
            .call("DoAction", &(item.action_index,))
            .map_err(display)?;
        if performed {
            Ok(())
        } else {
            Err("The application refused this menu action".into())
        }
    }
    pub fn capture_selection(&self, pid: u32) -> Result<TextSelection, String> {
        let mut selection = None;
        self.walk(pid, |target, _, _, accessible| {
            if !safe_text(accessible, true)? || self.pid(target).ok() != Some(pid) {
                return Ok(false);
            }
            let Ok(text) = self.proxy(target, "org.a11y.atspi.Text") else {
                return Ok(false);
            };
            let count: i32 = text.call("GetNSelections", &()).unwrap_or(0);
            if count != 1 {
                return Ok(false);
            }
            let (start, end): (i32, i32) = text.call("GetSelection", &(0i32,)).map_err(display)?;
            if start < 0 || end <= start || end.saturating_sub(start) > 32_768 {
                return Ok(false);
            }
            let contents: String = text.call("GetText", &(start, end)).map_err(display)?;
            if contents.is_empty() || contents.len() > 131_072 {
                return Ok(false);
            }
            let states: Vec<u32> = accessible.call("GetState", &()).map_err(display)?;
            selection = Some(TextSelection {
                target: target.clone(),
                pid,
                text: contents,
                start,
                end,
                editable: state(&states, 7),
            });
            Ok(true)
        })?;
        selection.ok_or_else(|| {
            "Select text in a supported application first. Password fields are excluded.".into()
        })
    }
    /// Only edits the captured object, range and content after focus has returned to it.
    /// Copy the result instead when this returns an error; no clipboard or synthetic paste is used.
    pub fn replace_selection(
        &self,
        selection: &TextSelection,
        replacement: &str,
    ) -> Result<(), String> {
        if !selection.editable || replacement.len() > 131_072 {
            return Err("Replacement is unavailable for this selection".into());
        }
        if self.pid(&selection.target)? != selection.pid {
            return Err("The application changed".into());
        }
        let accessible = self.proxy(&selection.target, "org.a11y.atspi.Accessible")?;
        if !safe_text(&accessible, true)? {
            return Err("The original text field is no longer focused or is secure".into());
        }
        let states: Vec<u32> = accessible.call("GetState", &()).map_err(display)?;
        if !state(&states, 7) {
            return Err("The text field is no longer editable".into());
        }
        let text = self.proxy(&selection.target, "org.a11y.atspi.Text")?;
        let count: i32 = text.call("GetNSelections", &()).map_err(display)?;
        let range: (i32, i32) = text.call("GetSelection", &(0i32,)).map_err(display)?;
        let contents: String = text
            .call("GetText", &(selection.start, selection.end))
            .map_err(display)?;
        if !selection_matches(selection, count, range, &contents) {
            return Err("The selected text changed; copy the result instead".into());
        }
        let character_count: i32 = text.get_property("CharacterCount").map_err(display)?;
        if selection.start < 0 || selection.end > character_count {
            return Err("The selected text changed".into());
        }
        let before_start = selection.start.saturating_sub(32).max(0);
        let before: String = text
            .call("GetText", &(before_start, selection.start))
            .map_err(display)?;
        let tail_end = selection.end.saturating_add(32).min(character_count);
        let tail: String = text
            .call("GetText", &(selection.end, tail_end))
            .map_err(display)?;
        let editable = self.proxy(&selection.target, "org.a11y.atspi.EditableText")?;
        // Read-only checks complete before the final exact selection revalidation.
        let current_range: (i32, i32) = text.call("GetSelection", &(0i32,)).map_err(display)?;
        let current: String = text
            .call("GetText", &(selection.start, selection.end))
            .map_err(display)?;
        if current_range != range || current != contents || !safe_text(&accessible, true)? {
            return Err("The selected text changed; copy the result instead".into());
        }
        let deleted: Result<bool, zbus::Error> =
            editable.call("DeleteText", &(selection.start, selection.end));
        if matches!(deleted, Ok(false)) {
            return Err("The application refused replacement".into());
        }
        let inserted = if matches!(deleted, Ok(true)) {
            editable.call::<_, _, bool>(
                "InsertText",
                &(selection.start, replacement, replacement.len() as i32),
            )
        } else {
            Err(deleted.unwrap_err())
        };
        if matches!(inserted, Ok(true)) {
            return Ok(());
        }
        // A timeout can mean the insert succeeded but its reply was lost. Restore only
        // when count and surrounding text prove the field is still in the deleted state.
        let expected_count = character_count - (selection.end - selection.start);
        let current_count: Result<i32, zbus::Error> = text.get_property("CharacterCount");
        let current_before: Result<String, zbus::Error> =
            text.call("GetText", &(before_start, selection.start));
        let current_tail: Result<String, zbus::Error> = text.call(
            "GetText",
            &(
                selection.start,
                selection.start + (tail_end - selection.end),
            ),
        );
        if current_count.ok() != Some(expected_count)
            || current_before.ok().as_deref() != Some(before.as_str())
            || current_tail.ok().as_deref() != Some(tail.as_str())
            || !safe_text(&accessible, true)?
        {
            return Err(
                "The replacement outcome could not be verified; check the original text field"
                    .into(),
            );
        }
        let restored: Result<bool, zbus::Error> = editable.call(
            "InsertText",
            &(
                selection.start,
                selection.text.as_str(),
                selection.text.len() as i32,
            ),
        );
        Err(if matches!(restored, Ok(true)) {
            "Replacement failed; original text restored"
        } else {
            "Replacement and restoration failed"
        }
        .into())
    }
}
fn selection_matches(selection: &TextSelection, count: i32, range: (i32, i32), text: &str) -> bool {
    count == 1 && range == (selection.start, selection.end) && text == selection.text
}
fn state(states: &[u32], bit: usize) -> bool {
    states
        .get(bit / 32)
        .is_some_and(|value| value & (1 << (bit % 32)) != 0)
}
fn enabled(accessible: &Proxy<'_>) -> Result<bool, String> {
    let states: Vec<u32> = accessible.call("GetState", &()).map_err(display)?;
    Ok(state(&states, 8) && !state(&states, 6))
}
fn safe_text(accessible: &Proxy<'_>, focused: bool) -> Result<bool, String> {
    let states: Vec<u32> = accessible.call("GetState", &()).map_err(display)?;
    let role: u32 = accessible.call("GetRole", &()).map_err(display)?;
    let attrs: HashMap<String, String> = accessible.call("GetAttributes", &()).map_err(display)?;
    Ok((!focused || state(&states, 12))
        && !state(&states, 6)
        && role != 40
        && !attrs.iter().any(|(key, value)| {
            matches!(key.as_str(), "text-input-type" | "input-type" | "type")
                && value.eq_ignore_ascii_case("password")
        }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_selection_gate() {
        let selection = TextSelection {
            target: AccessibleTarget {
                bus_name: ":1.2".into(),
                path: OwnedObjectPath::try_from("/field").unwrap(),
            },
            pid: 1,
            text: "🦀żółć".into(),
            start: 2,
            end: 7,
            editable: true,
        };
        assert!(selection_matches(&selection, 1, (2, 7), "🦀żółć"));
        assert!(!selection_matches(&selection, 1, (3, 8), "🦀żółć"));
        assert!(!selection_matches(&selection, 2, (2, 7), "🦀żółć"));
        assert!(!selection_matches(&selection, 1, (2, 7), "changed"));
    }
    #[test]
    fn state_bits_cross_words() {
        assert!(state(&[0, 4], 34));
        assert!(!state(&[], 12));
    }

    struct LayoutFixture {
        windows: Vec<Window>,
        applied: Arc<std::sync::Mutex<Vec<WindowId>>>,
    }
    impl Compositor for LayoutFixture {
        fn kind(&self) -> crate::compositor::CompositorKind {
            crate::compositor::CompositorKind::X11
        }
        fn capabilities(&self) -> crate::compositor::CompositorCapabilities {
            crate::compositor::capabilities_for(self.kind())
        }
        fn windows(&self) -> Result<Vec<Window>, crate::compositor::CompositorError> {
            Ok(self.windows.clone())
        }
        fn outputs(
            &self,
        ) -> Result<Vec<crate::compositor::Output>, crate::compositor::CompositorError> {
            Ok(vec![crate::compositor::Output {
                name: "new-display".into(),
                frame: Rect::new(0, 0, 800, 600),
                usable: Rect::new(0, 0, 800, 600),
                focused: true,
            }])
        }
        fn focus_window(&self, _: &WindowId) -> Result<(), crate::compositor::CompositorError> {
            Ok(())
        }
        fn close_window(&self, _: &WindowId) -> Result<(), crate::compositor::CompositorError> {
            Ok(())
        }
        fn hide_window(&self, _: &WindowId) -> Result<(), crate::compositor::CompositorError> {
            Ok(())
        }
        fn quit_session(&self) -> Result<(), crate::compositor::CompositorError> {
            Ok(())
        }
        fn layout(
            &self,
            command: &LayoutCommand,
        ) -> Result<(), crate::compositor::CompositorError> {
            if let LayoutCommand::SetFrame { id, frame } = command {
                assert!(frame.right() <= 800 && frame.bottom() <= 600);
                self.applied.lock().unwrap().push(id.clone());
                if id.0 == "failure" {
                    return Err(crate::compositor::CompositorError::Rejected(
                        "closed during restore".into(),
                    ));
                }
            }
            Ok(())
        }
    }
    fn layout_window(id: &str, title: &str) -> Window {
        Window {
            id: WindowId(id.into()),
            app_id: Some("editor".into()),
            title: title.into(),
            pid: Some(1),
            focused: false,
            floating: true,
            fullscreen: false,
            hidden: false,
            output: None,
            workspace: None,
            frame: Some(Rect::new(0, 0, 500, 400)),
            size: None,
            focus_rank: None,
        }
    }
    fn saved_window(title: &str) -> SavedWindow {
        SavedWindow {
            app_id: "editor".into(),
            title: title.into(),
            output: Some("removed-display".into()),
            frame: LayoutFrame {
                x: 500,
                y: 0,
                width: 500,
                height: 800,
            },
            output_frame: LayoutFrame {
                x: 0,
                y: 0,
                width: 1000,
                height: 800,
            },
            floating: true,
        }
    }
    #[test]
    fn layout_restore_is_partial_and_never_reuses_windows() {
        let applied = Arc::new(std::sync::Mutex::new(Vec::new()));
        let workflows = DesktopWorkflows::new(Arc::new(LayoutFixture {
            windows: vec![layout_window("ok", "one"), layout_window("failure", "two")],
            applied: applied.clone(),
        }));
        let report = workflows
            .restore_layout(&SavedWindowLayout {
                id: "test".into(),
                name: "Work".into(),
                windows: vec![
                    saved_window("one"),
                    saved_window("two"),
                    saved_window("one"),
                ],
            })
            .unwrap();
        assert_eq!(report.restored, 1);
        assert_eq!(report.missing, 1);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(applied.lock().unwrap().len(), 2);
    }
    #[test]
    fn ambiguous_application_windows_are_left_untouched() {
        let applied = Arc::new(std::sync::Mutex::new(Vec::new()));
        let workflows = DesktopWorkflows::new(Arc::new(LayoutFixture {
            windows: vec![layout_window("a", "one"), layout_window("b", "two")],
            applied: applied.clone(),
        }));
        let report = workflows
            .restore_layout(&SavedWindowLayout {
                id: "test".into(),
                name: "Work".into(),
                windows: vec![saved_window("unknown")],
            })
            .unwrap();
        assert_eq!(report.missing, 1);
        assert!(applied.lock().unwrap().is_empty());
    }
    #[derive(Default)]
    struct Field {
        text: String,
        focused: bool,
        password: bool,
        start: i32,
        end: i32,
        selections: i32,
        fail_insert: bool,
        mutations: usize,
    }
    struct Accessible(Arc<std::sync::Mutex<Field>>);
    #[zbus::interface(name = "org.a11y.atspi.Accessible")]
    impl Accessible {
        fn get_state(&self) -> Vec<u32> {
            vec![
                (1 << 7)
                    | if self.0.lock().unwrap().focused {
                        1 << 12
                    } else {
                        0
                    },
                0,
            ]
        }
        fn get_role(&self) -> u32 {
            if self.0.lock().unwrap().password {
                40
            } else {
                61
            }
        }
        fn get_attributes(&self) -> HashMap<String, String> {
            HashMap::new()
        }
    }
    struct Text(Arc<std::sync::Mutex<Field>>);
    #[zbus::interface(name = "org.a11y.atspi.Text")]
    impl Text {
        #[zbus(property)]
        fn character_count(&self) -> i32 {
            self.0.lock().unwrap().text.chars().count() as i32
        }
        fn get_n_selections(&self) -> i32 {
            self.0.lock().unwrap().selections
        }
        fn get_selection(&self, _index: i32) -> (i32, i32) {
            let field = self.0.lock().unwrap();
            (field.start, field.end)
        }
        fn get_text(&self, start: i32, end: i32) -> String {
            self.0
                .lock()
                .unwrap()
                .text
                .chars()
                .skip(start as usize)
                .take((end - start) as usize)
                .collect()
        }
    }
    struct Editable(Arc<std::sync::Mutex<Field>>);
    #[zbus::interface(name = "org.a11y.atspi.EditableText")]
    impl Editable {
        fn delete_text(&self, start: i32, end: i32) -> bool {
            let mut field = self.0.lock().unwrap();
            field.text = field
                .text
                .chars()
                .enumerate()
                .filter_map(|(i, c)| (i < start as usize || i >= end as usize).then_some(c))
                .collect();
            field.mutations += 1;
            true
        }
        fn insert_text(&self, position: i32, text: &str, length: i32) -> bool {
            assert_eq!(length as usize, text.len());
            let mut field = self.0.lock().unwrap();
            if field.fail_insert {
                field.fail_insert = false;
                return false;
            }
            let before: String = field.text.chars().take(position as usize).collect();
            let after: String = field.text.chars().skip(position as usize).collect();
            field.text = format!("{before}{text}{after}");
            field.mutations += 1;
            true
        }
    }
    struct PrivateBus(std::process::Child);
    impl Drop for PrivateBus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    #[test]
    #[ignore = "requires dbus-daemon; creates a private accessibility fixture bus"]
    fn private_selected_text_exact_guard_unicode_and_rollback() {
        use std::io::BufRead;
        let mut bus = PrivateBus(
            std::process::Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address=1"])
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let mut address = String::new();
        std::io::BufReader::new(bus.0.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let field = Arc::new(std::sync::Mutex::new(Field::default()));
        let server = zbus::blocking::connection::Builder::address(address.trim())
            .unwrap()
            .serve_at("/field", Accessible(field.clone()))
            .unwrap()
            .serve_at("/field", Text(field.clone()))
            .unwrap()
            .serve_at("/field", Editable(field.clone()))
            .unwrap()
            .build()
            .unwrap();
        let client = zbus::blocking::connection::Builder::address(address.trim())
            .unwrap()
            .method_timeout(Duration::from_secs(1))
            .build()
            .unwrap();
        let accessibility = AccessibilityWorkflows { connection: client };
        let selection = TextSelection {
            target: AccessibleTarget {
                bus_name: server.unique_name().unwrap().to_string(),
                path: OwnedObjectPath::try_from("/field").unwrap(),
            },
            pid: std::process::id(),
            text: "🦀żółć".into(),
            start: 5,
            end: 10,
            editable: true,
        };
        let reset = || {
            *field.lock().unwrap() = Field {
                text: "hello🦀żółćtail".into(),
                focused: true,
                start: 5,
                end: 10,
                selections: 1,
                ..Field::default()
            };
        };
        reset();
        accessibility
            .replace_selection(&selection, "Zażółć 🦀")
            .unwrap();
        assert_eq!(field.lock().unwrap().text, "helloZażółć 🦀tail");
        for gate in 0..5 {
            reset();
            {
                let mut field = field.lock().unwrap();
                match gate {
                    0 => field.focused = false,
                    1 => field.password = true,
                    2 => field.start = 6,
                    3 => field.selections = 2,
                    _ => field.text = "helloXXXXXtail".into(),
                }
            }
            assert!(
                accessibility
                    .replace_selection(&selection, "changed")
                    .is_err()
            );
            assert_eq!(field.lock().unwrap().mutations, 0);
        }
        reset();
        field.lock().unwrap().fail_insert = true;
        let error = accessibility
            .replace_selection(&selection, "changed")
            .unwrap_err();
        assert!(error.contains("restored"), "{error}");
        assert_eq!(field.lock().unwrap().text, "hello🦀żółćtail");
        reset();
        let mut wrong_pid = selection.clone();
        wrong_pid.pid += 1;
        assert!(
            accessibility
                .replace_selection(&wrong_pid, "changed")
                .is_err()
        );
        assert_eq!(field.lock().unwrap().mutations, 0);
    }
}
