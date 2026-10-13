//! Optional keyword expansion through AT-SPI EditableText. Applications that do
//! not expose accessible editable text are left untouched, including terminals
//! and password controls. No global keystrokes or privileged devices are read.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kebloop_services::snippets::{Snippet, SnippetKeywordMatcher};
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

pub type RenderSnippet = Arc<dyn Fn(&Snippet) -> Result<String, String> + Send + Sync>;
#[derive(Clone)]
pub struct ExpansionConfig {
    pub snippets: Arc<Mutex<Vec<Snippet>>>,
    /// Supplies the user's locale, timezone, clock and current clipboard value.
    pub render: RenderSnippet,
}

struct Lifecycle {
    running: AtomicBool,
    mutation: Mutex<()>,
}
impl Lifecycle {
    fn new() -> Self {
        Self {
            running: AtomicBool::new(true),
            mutation: Mutex::new(()),
        }
    }
}

pub struct SnippetExpansion {
    connection: Connection,
    lifecycle: Arc<Lifecycle>,
    error: Arc<Mutex<Option<String>>>,
}
impl SnippetExpansion {
    pub fn start(config: ExpansionConfig) -> Result<Self, String> {
        let session = zbus::blocking::connection::Builder::session()
            .map_err(display)?
            .method_timeout(Duration::from_millis(500))
            .build()
            .map_err(display)?;
        let bus = Proxy::new(&session, "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus")
            .map_err(display)?;
        let address: String = bus.call("GetAddress", &()).map_err(display)?;
        let connection = zbus::blocking::connection::Builder::address(address.as_str())
            .map_err(display)?
            .method_timeout(Duration::from_millis(500))
            .build()
            .map_err(display)?;
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.a11y.atspi.Event.Object")
            .map_err(display)?
            .member("TextChanged")
            .map_err(display)?
            .build();
        let iterator =
            MessageIterator::for_match_rule(rule, &connection, Some(32)).map_err(display)?;
        let registry = Proxy::new(
            &connection,
            "org.a11y.atspi.Registry",
            "/org/a11y/atspi/registry",
            "org.a11y.atspi.Registry",
        )
        .map_err(display)?;
        registry
            .call::<_, _, ()>(
                "RegisterEvent",
                &("object:text-changed:insert", Vec::<String>::new(), ""),
            )
            .map_err(display)?;
        let lifecycle = Arc::new(Lifecycle::new());
        let error = Arc::new(Mutex::new(None));
        let worker_connection = connection.clone();
        let worker_lifecycle = lifecycle.clone();
        let worker_error = error.clone();
        std::thread::Builder::new()
            .name("kebloop-snippet-expansion".into())
            .spawn(move || {
                let mut suppression: Option<(String, String, Instant)> = None;
                for message in iterator {
                    if !worker_lifecycle.running.load(Ordering::Acquire) {
                        break;
                    }
                    let message = match message {
                        Ok(message) => message,
                        Err(e) => {
                            set_error(&worker_error, display(e));
                            break;
                        }
                    };
                    let header = message.header();
                    let (Some(sender), Some(path)) = (header.sender(), header.path()) else {
                        continue;
                    };
                    let sender = sender.as_str();
                    let path = path.as_str();
                    if suppression.as_ref().is_some_and(|(s, p, until)| {
                        s == sender && p == path && Instant::now() < *until
                    }) {
                        continue;
                    }
                    let Some((start, _inserted)) = inserted_space(&message) else {
                        continue;
                    };
                    match expand(
                        &worker_connection,
                        sender,
                        path,
                        start,
                        &config,
                        &worker_lifecycle,
                    ) {
                        Ok(true) => {
                            suppression = Some((
                                sender.into(),
                                path.into(),
                                Instant::now() + Duration::from_millis(300),
                            ))
                        }
                        Ok(false) => {}
                        Err(e) => set_error(&worker_error, e),
                    }
                }
                worker_lifecycle.running.store(false, Ordering::Release);
            })
            .map_err(display)?;
        Ok(Self {
            connection,
            lifecycle,
            error,
        })
    }
    pub fn is_running(&self) -> bool {
        self.lifecycle.running.load(Ordering::Acquire)
    }
    pub fn last_error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|e| e.clone())
    }
}
impl Drop for SnippetExpansion {
    fn drop(&mut self) {
        self.lifecycle.running.store(false, Ordering::Release);
        let lifecycle = self.lifecycle.clone();
        let connection = self.connection.clone();
        // Keep GTK responsive and let an already-started edit/rollback finish
        // before disconnecting the application that owns the text field.
        let _ = std::thread::Builder::new()
            .name("kebloop-snippet-stop".into())
            .spawn(move || {
                let _mutation = lifecycle.mutation.lock().unwrap_or_else(|e| e.into_inner());
                let _ = connection.close();
            });
    }
}
fn display(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn set_error(slot: &Mutex<Option<String>>, value: String) {
    if let Ok(mut slot) = slot.lock() {
        *slot = Some(value);
    }
}

fn inserted_space(message: &zbus::Message) -> Option<(i32, String)> {
    // New AT-SPI uses property dictionaries; older toolkits append a source ref.
    let body = message.body();
    let decoded = body
        .deserialize::<(String, i32, i32, OwnedValue, HashMap<String, OwnedValue>)>()
        .map(|(detail, start, length, text, _)| (detail, start, length, text))
        .or_else(|_| {
            body.deserialize::<(String, i32, i32, OwnedValue, (String, OwnedObjectPath))>()
                .map(|(detail, start, length, text, _)| (detail, start, length, text))
        })
        .ok()?;
    let (detail, start, length, text) = decoded;
    let text = <&str>::try_from(&text).ok()?;
    (detail == "insert" && start >= 0 && length == 1 && text == " ").then(|| (start, text.into()))
}
fn permitted(states: &[u32], role: u32, attrs: &HashMap<String, String>) -> bool {
    let bits = states.first().copied().unwrap_or(0);
    bits & (1 << 7) != 0
        && bits & (1 << 12) != 0
        && bits & (1 << 6) == 0
        && role != 40
        && !attrs.iter().any(|(key, value)| {
            matches!(key.as_str(), "text-input-type" | "input-type" | "type")
                && value.eq_ignore_ascii_case("password")
        })
}
fn safe_accessible(accessible: &Proxy<'_>) -> Result<bool, String> {
    let states: Vec<u32> = accessible.call("GetState", &()).map_err(display)?;
    let role: u32 = accessible.call("GetRole", &()).map_err(display)?;
    let attrs: HashMap<String, String> = accessible.call("GetAttributes", &()).map_err(display)?;
    Ok(permitted(&states, role, &attrs))
}
fn plan<'a>(tail: &str, caret: i32, snippets: &'a [Snippet]) -> Option<(&'a Snippet, i32, String)> {
    if !tail.ends_with(' ') || tail.chars().count() > 128 {
        return None;
    }
    let mut matcher = SnippetKeywordMatcher::default();
    let mut found = None;
    for character in tail.chars() {
        found = matcher.record(character, snippets);
    }
    let snippet = found?;
    let length = snippet.keyword.as_ref()?.chars().count() + 1;
    let start = caret.checked_sub(i32::try_from(length).ok()?)?;
    if start < 0 {
        return None;
    }
    let original: String = tail
        .chars()
        .skip(tail.chars().count().checked_sub(length)?)
        .collect();
    Some((snippet, start, original))
}
fn expand(
    connection: &Connection,
    sender: &str,
    path: &str,
    insertion: i32,
    config: &ExpansionConfig,
    lifecycle: &Lifecycle,
) -> Result<bool, String> {
    let dbus = Proxy::new(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .map_err(display)?;
    let pid: u32 = dbus
        .call("GetConnectionUnixProcessID", &(sender,))
        .map_err(display)?;
    if pid == std::process::id() {
        return Ok(false);
    }
    expand_target(connection, sender, path, insertion, config, lifecycle)
}

fn expand_target(
    connection: &Connection,
    sender: &str,
    path: &str,
    insertion: i32,
    config: &ExpansionConfig,
    lifecycle: &Lifecycle,
) -> Result<bool, String> {
    let accessible =
        Proxy::new(connection, sender, path, "org.a11y.atspi.Accessible").map_err(display)?;
    if !safe_accessible(&accessible)? {
        return Ok(false);
    }
    let text = Proxy::new(connection, sender, path, "org.a11y.atspi.Text").map_err(display)?;
    let caret: i32 = text.get_property("CaretOffset").map_err(display)?;
    let selections: i32 = text.call("GetNSelections", &()).map_err(display)?;
    if caret != insertion.saturating_add(1) || selections != 0 {
        return Ok(false);
    }
    let tail: String = text
        .call("GetText", &(caret.saturating_sub(128).max(0), caret))
        .map_err(display)?;
    let (snippet, start, original) = {
        let snippets = config.snippets.lock().map_err(display)?;
        let Some((snippet, start, original)) = plan(&tail, caret, &snippets) else {
            return Ok(false);
        };
        (snippet.clone(), start, original)
    };
    let rendered = (config.render)(&snippet)?;
    if rendered.chars().count() > 4096 || rendered.len() > i32::MAX as usize {
        return Err("Snippet expansion exceeds 4096 characters".into());
    }
    // Rendering can involve clipboard I/O; recheck the target before mutation.
    if !safe_accessible(&accessible)? {
        return Ok(false);
    }
    let current_caret: i32 = text.get_property("CaretOffset").map_err(display)?;
    let selections: i32 = text.call("GetNSelections", &()).map_err(display)?;
    let current: String = text.call("GetText", &(start, caret)).map_err(display)?;
    if current_caret != caret || selections != 0 || current != original {
        return Ok(false);
    }
    let editable =
        Proxy::new(connection, sender, path, "org.a11y.atspi.EditableText").map_err(display)?;
    let _mutation = lifecycle.mutation.lock().map_err(display)?;
    let current_snippet = config.snippets.lock().map_err(display)?.contains(&snippet);
    if !lifecycle.running.load(Ordering::Acquire) || !current_snippet {
        return Ok(false);
    }
    // This is the acceptance point for the transaction. Configuration updates
    // apply to the next event; keep only the mutation lock through network I/O
    // so GTK can update snippets without waiting for an application reply.
    // Cancellation after deletion must not leave the original trigger erased.
    let deleted: bool = editable
        .call("DeleteText", &(start, caret))
        .map_err(display)?;
    if !deleted {
        return Err("The focused application refused snippet deletion".into());
    }
    // InsertText's length is UTF-8 bytes; position and deletion offsets are characters.
    let inserted: Result<bool, zbus::Error> = if rendered.is_empty() {
        Ok(true)
    } else {
        editable.call(
            "InsertText",
            &(start, rendered.as_str(), rendered.len() as i32),
        )
    };
    if matches!(inserted, Ok(true)) {
        let moved: bool = text
            .call(
                "SetCaretOffset",
                &(start + rendered.chars().count() as i32,),
            )
            .map_err(display)?;
        if !moved {
            return Err("Snippet expanded, but the application refused to move the caret".into());
        }
        return Ok(true);
    }
    if let Err(e) = inserted {
        return Err(format!("Snippet insertion status unknown: {e}"));
    }
    let restored: bool = editable
        .call(
            "InsertText",
            &(start, original.as_str(), original.len() as i32),
        )
        .map_err(display)?;
    if restored {
        let _: bool = text.call("SetCaretOffset", &(caret,)).map_err(display)?;
    }
    Err(if restored {
        "The focused application refused snippet insertion; original text restored"
    } else {
        "Snippet insertion and restoration failed"
    }
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snippet(keyword: &str) -> Snippet {
        Snippet {
            id: "1".into(),
            name: "test".into(),
            template: "hello".into(),
            keyword: Some(keyword.into()),
        }
    }
    #[test]
    fn unicode_offsets_are_characters() {
        let snippets = [snippet("żółć")];
        let (_, start, original) = plan("🦀 żółć ", 7, &snippets).unwrap();
        assert_eq!(start, 2);
        assert_eq!(original, "żółć ");
        assert!(plan("xżółć ", 6, &snippets).is_none());
    }
    #[test]
    fn refuses_password_or_unfocused_or_readonly_controls() {
        let focused_editable = (1 << 7) | (1 << 12);
        assert!(permitted(&[focused_editable, 0], 61, &HashMap::new()));
        assert!(!permitted(&[focused_editable], 40, &HashMap::new()));
        assert!(!permitted(&[1 << 7], 61, &HashMap::new()));
        assert!(!permitted(&[1 << 12], 61, &HashMap::new()));
        assert!(!permitted(
            &[focused_editable],
            61,
            &HashMap::from([("text-input-type".into(), "password".into())])
        ));
    }
    #[test]
    fn refuses_long_tail_and_prompted_templates() {
        let mut snippet = snippet("test");
        snippet.template = "{name}".into();
        assert!(plan("test ", 5, &[snippet]).is_none());
        assert!(
            plan(
                &format!("{} test ", "x".repeat(129)),
                135,
                &[self::snippet("test")]
            )
            .is_none()
        );
    }
}

#[cfg(test)]
mod dbus_tests {
    use super::*;
    use std::io::BufRead;
    use std::process::{Child, Command, Stdio};

    struct PrivateBus(Child);
    impl Drop for PrivateBus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    #[derive(Default)]
    struct Field {
        text: String,
        caret: i32,
        role: u32,
        focused: bool,
        selections: i32,
        password: bool,
        reject_insert: bool,
        mutations: usize,
    }
    #[derive(Clone)]
    struct Accessible(Arc<Mutex<Field>>);
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
            self.0.lock().unwrap().role
        }
        fn get_attributes(&self) -> HashMap<String, String> {
            if self.0.lock().unwrap().password {
                HashMap::from([("text-input-type".into(), "password".into())])
            } else {
                HashMap::new()
            }
        }
    }
    struct Text(Arc<Mutex<Field>>);
    #[zbus::interface(name = "org.a11y.atspi.Text")]
    impl Text {
        #[zbus(property)]
        fn caret_offset(&self) -> i32 {
            self.0.lock().unwrap().caret
        }
        fn get_n_selections(&self) -> i32 {
            self.0.lock().unwrap().selections
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
        fn set_caret_offset(&self, offset: i32) -> bool {
            self.0.lock().unwrap().caret = offset;
            true
        }
    }
    struct Editable(Arc<Mutex<Field>>);
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
            field.caret = start;
            field.mutations += 1;
            true
        }
        fn insert_text(&self, position: i32, text: &str, length: i32) -> bool {
            assert_eq!(
                length as usize,
                text.len(),
                "AT-SPI insertion length must be UTF-8 bytes"
            );
            let mut field = self.0.lock().unwrap();
            if field.reject_insert {
                field.reject_insert = false;
                return false;
            }
            let before: String = field.text.chars().take(position as usize).collect();
            let after: String = field.text.chars().skip(position as usize).collect();
            field.text = format!("{before}{text}{after}");
            field.mutations += 1;
            true
        }
    }
    fn reset(field: &Mutex<Field>) {
        *field.lock().unwrap() = Field {
            text: "🦀 żółć ".into(),
            caret: 7,
            role: 61,
            focused: true,
            ..Field::default()
        };
    }
    #[test]
    #[ignore = "requires dbus-daemon; creates a private accessibility fixture bus"]
    fn private_atspi_unicode_guards_revalidation_and_rollback() {
        let child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut bus = PrivateBus(child);
        let mut address = String::new();
        std::io::BufReader::new(bus.0.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let address = address.trim();
        let field = Arc::new(Mutex::new(Field::default()));
        reset(&field);
        let _server = zbus::blocking::connection::Builder::address(address)
            .unwrap()
            .name("org.kebloop.SnippetFixture")
            .unwrap()
            .serve_at("/field", Accessible(field.clone()))
            .unwrap()
            .serve_at("/field", Text(field.clone()))
            .unwrap()
            .serve_at("/field", Editable(field.clone()))
            .unwrap()
            .build()
            .unwrap();
        let client = zbus::blocking::connection::Builder::address(address)
            .unwrap()
            .method_timeout(Duration::from_secs(1))
            .build()
            .unwrap();
        let config = ExpansionConfig {
            snippets: Arc::new(Mutex::new(vec![Snippet {
                id: "1".into(),
                name: "test".into(),
                keyword: Some("żółć".into()),
                template: "Zażółć 🦀".into(),
            }])),
            render: Arc::new(|snippet| Ok(snippet.template.clone())),
        };
        let lifecycle = Arc::new(Lifecycle::new());
        let invoke = |config: &ExpansionConfig| {
            expand_target(
                &client,
                "org.kebloop.SnippetFixture",
                "/field",
                6,
                config,
                &lifecycle,
            )
        };
        assert!(invoke(&config).unwrap());
        {
            let field = field.lock().unwrap();
            assert_eq!(field.text, "🦀 Zażółć 🦀");
            assert_eq!(field.caret, 10);
        }
        // Production entry rejects this process's own accessible fields.
        reset(&field);
        assert!(
            !expand(
                &client,
                "org.kebloop.SnippetFixture",
                "/field",
                6,
                &config,
                &lifecycle
            )
            .unwrap()
        );
        for guard in 0..4 {
            reset(&field);
            {
                let mut field = field.lock().unwrap();
                match guard {
                    0 => field.role = 40,
                    1 => field.password = true,
                    2 => field.focused = false,
                    _ => field.selections = 1,
                }
            }
            assert!(!invoke(&config).unwrap());
            assert_eq!(field.lock().unwrap().mutations, 0);
        }
        reset(&field);
        let during_render = field.clone();
        let moving_focus = ExpansionConfig {
            render: Arc::new(move |_| {
                during_render.lock().unwrap().focused = false;
                Ok("hello".into())
            }),
            ..config.clone()
        };
        assert!(!invoke(&moving_focus).unwrap());
        assert_eq!(field.lock().unwrap().mutations, 0);
        reset(&field);
        let saved = config.snippets.lock().unwrap().clone();
        let removed_snippets = config.snippets.clone();
        let removing_snippet = ExpansionConfig {
            render: Arc::new(move |_| {
                removed_snippets.lock().unwrap().clear();
                Ok("hello".into())
            }),
            ..config.clone()
        };
        assert!(!invoke(&removing_snippet).unwrap());
        assert_eq!(field.lock().unwrap().mutations, 0);
        *config.snippets.lock().unwrap() = saved;
        reset(&field);
        let cancelling = lifecycle.clone();
        let cancelling_service = ExpansionConfig {
            render: Arc::new(move |_| {
                cancelling.running.store(false, Ordering::Release);
                Ok("hello".into())
            }),
            ..config.clone()
        };
        assert!(!invoke(&cancelling_service).unwrap());
        assert_eq!(field.lock().unwrap().mutations, 0);
        lifecycle.running.store(true, Ordering::Release);
        reset(&field);
        field.lock().unwrap().reject_insert = true;
        assert!(
            invoke(&config)
                .unwrap_err()
                .contains("original text restored")
        );
        assert_eq!(field.lock().unwrap().text, "🦀 żółć ");
        assert_eq!(field.lock().unwrap().caret, 7);
    }
    #[test]
    fn modern_and_legacy_insert_events_only_accept_single_spaces() {
        let space = OwnedValue::from(zbus::zvariant::Str::from(" "));
        let message = zbus::Message::signal("/field", "org.a11y.atspi.Event.Object", "TextChanged")
            .unwrap()
            .build(&(
                "insert",
                6i32,
                1i32,
                space,
                HashMap::<String, OwnedValue>::new(),
            ))
            .unwrap();
        assert_eq!(inserted_space(&message), Some((6, " ".into())));
        let text = OwnedValue::from(zbus::zvariant::Str::from("keyword "));
        let message = zbus::Message::signal("/field", "org.a11y.atspi.Event.Object", "TextChanged")
            .unwrap()
            .build(&(
                "insert",
                0i32,
                8i32,
                text,
                HashMap::<String, OwnedValue>::new(),
            ))
            .unwrap();
        assert_eq!(inserted_space(&message), None);
        let space = OwnedValue::from(zbus::zvariant::Str::from(" "));
        let message = zbus::Message::signal("/field", "org.a11y.atspi.Event.Object", "TextChanged")
            .unwrap()
            .build(&(
                "insert",
                6i32,
                1i32,
                space,
                (
                    "org.kebloop.Fixture",
                    OwnedObjectPath::try_from("/field").unwrap(),
                ),
            ))
            .unwrap();
        assert_eq!(inserted_space(&message), Some((6, " ".into())));
    }
}
