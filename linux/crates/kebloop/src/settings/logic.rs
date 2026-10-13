//! GTK-free settings logic: the page registry, search matching, shortcut presentation and the
//! small text conversions the pages share. Everything here is unit-tested.

use std::path::{Path, PathBuf};

use kebloop_core::search::normalize;
use kebloop_platform::apps::DesktopApp;
use kebloop_services::accelerator::{ALT, Accelerator, CONTROL, SHIFT, SUPER};
use kebloop_services::config::PortableSettings;

use crate::i18n::tr;

/// Sidebar section a page belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// General, shortcuts, search and appearance.
    Preferences,
    /// Quicklinks, aliases, snippets, scripts, extensions, profiles.
    Personalization,
    /// Clipboard, configuration, help, about.
    Data,
}

impl Group {
    /// Localization key of the section heading.
    pub fn title_key(self) -> &'static str {
        match self {
            Self::Preferences => "Preferences",
            Self::Personalization => "Personalization",
            Self::Data => "Data & Help",
        }
    }
}

/// A settings page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PageId {
    /// Launcher shortcut, AI provider, launch at login.
    General,
    /// Launcher, application and command shortcuts.
    Shortcuts,
    /// Search categories, folders, exclusions, hidden apps, favorites.
    Search,
    /// Color scheme, sizes, density, transparency, placement.
    Appearance,
    /// Window arrangement commands and their shortcuts.
    WindowManagement,
    /// Quicklinks and web searches.
    Quicklinks,
    /// Application aliases.
    Aliases,
    /// Snippets.
    Snippets,
    /// Script commands.
    Scripts,
    /// Extensions.
    Extensions,
    /// Profiles.
    Profiles,
    /// Clipboard history.
    Clipboard,
    /// Export and import.
    Configuration,
    /// Commands and availability.
    Help,
    /// Version and legal information.
    About,
}

impl PageId {
    /// Pages in sidebar order.
    pub const ALL: [PageId; 15] = [
        Self::General,
        Self::Shortcuts,
        Self::Search,
        Self::Appearance,
        Self::WindowManagement,
        Self::Quicklinks,
        Self::Aliases,
        Self::Snippets,
        Self::Scripts,
        Self::Extensions,
        Self::Profiles,
        Self::Clipboard,
        Self::Configuration,
        Self::Help,
        Self::About,
    ];

    /// The name used on the command line (`kebloop settings <slug>`).
    pub fn slug(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Shortcuts => "shortcuts",
            Self::Search => "search",
            Self::Appearance => "appearance",
            Self::WindowManagement => "window-management",
            Self::Quicklinks => "quicklinks",
            Self::Aliases => "aliases",
            Self::Snippets => "snippets",
            Self::Scripts => "scripts",
            Self::Extensions => "extensions",
            Self::Profiles => "profiles",
            Self::Clipboard => "clipboard",
            Self::Configuration => "configuration",
            Self::Help => "help",
            Self::About => "about",
        }
    }

    /// Look a page up by its command-line name. Accepts the macOS section names too.
    pub fn from_slug(slug: &str) -> Option<Self> {
        let slug = slug.trim().to_ascii_lowercase();
        let page = match slug.as_str() {
            "appshortcuts" | "app-shortcuts" | "commandshortcuts" | "command-shortcuts" => {
                Self::Shortcuts
            }
            "websearch" | "web-search" | "links" => Self::Quicklinks,
            "config" | "import" | "export" => Self::Configuration,
            "script" | "scriptcommands" => Self::Scripts,
            "extension" => Self::Extensions,
            "snippet" => Self::Snippets,
            "windowmanagement" | "window-management" | "windows" => Self::WindowManagement,
            "profile" => Self::Profiles,
            other => return Self::ALL.into_iter().find(|p| p.slug() == other),
        };
        Some(page)
    }

    /// Section of the sidebar.
    pub fn group(self) -> Group {
        match self {
            Self::General
            | Self::Shortcuts
            | Self::Search
            | Self::Appearance
            | Self::WindowManagement => Group::Preferences,
            Self::Quicklinks
            | Self::Aliases
            | Self::Snippets
            | Self::Scripts
            | Self::Extensions
            | Self::Profiles => Group::Personalization,
            Self::Clipboard | Self::Configuration | Self::Help | Self::About => Group::Data,
        }
    }

    /// Localization key of the title.
    pub fn title_key(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Shortcuts => "Shortcuts",
            Self::Search => "Search",
            Self::Appearance => "Appearance",
            Self::WindowManagement => "Window Management",
            Self::Quicklinks => "Quicklinks",
            Self::Aliases => "Aliases",
            Self::Snippets => "Snippets",
            Self::Scripts => "Script Commands",
            Self::Extensions => "Extensions",
            Self::Profiles => "Profiles",
            Self::Clipboard => "Clipboard",
            Self::Configuration => "Configuration",
            Self::Help => "Help",
            Self::About => "About",
        }
    }

    /// Localization key of the one-line description.
    pub fn subtitle_key(self) -> &'static str {
        match self {
            Self::General => "Set your launcher shortcut, startup behavior, and AI handoff.",
            Self::Shortcuts => "Assign global shortcuts to the launcher, apps, and commands.",
            Self::Search => "Choose what Kebloop searches and manage indexed locations.",
            Self::Appearance => "Adjust the launcher’s look, size, and display placement.",
            Self::WindowManagement => "Configure window layouts and their keyboard shortcuts.",
            Self::Quicklinks => "Manage saved links and keyword-based web searches.",
            Self::Aliases => "Give applications additional names for search.",
            Self::Snippets => "Create reusable text with explicit placeholders.",
            Self::Scripts => "Manage reviewed local script commands.",
            Self::Extensions => "Manage local extensions and their preferences.",
            Self::Profiles => "Keep separate sets of launcher preferences and saved items.",
            Self::Clipboard => "Control local clipboard history and saved items.",
            Self::Configuration => "Preview, import, and export Kebloop settings.",
            Self::Help => "Browse commands, examples, and your current shortcuts.",
            Self::About => "Version, links, and license information.",
        }
    }

    /// Symbolic icon of the sidebar row.
    pub fn icon(self) -> &'static str {
        match self {
            Self::General => "preferences-system-symbolic",
            Self::Shortcuts => "input-keyboard-symbolic",
            Self::Search => "system-search-symbolic",
            Self::Appearance => "applications-graphics-symbolic",
            Self::WindowManagement => "view-dual-symbolic",
            Self::Quicklinks => "insert-link-symbolic",
            Self::Aliases => "edit-find-replace-symbolic",
            Self::Snippets => "insert-text-symbolic",
            Self::Scripts => "utilities-terminal-symbolic",
            Self::Extensions => "application-x-addon-symbolic",
            Self::Profiles => "avatar-default-symbolic",
            Self::Clipboard => "edit-paste-symbolic",
            Self::Configuration => "document-save-symbolic",
            Self::Help => "help-browser-symbolic",
            Self::About => "help-about-symbolic",
        }
    }

    /// Extra English search terms for the sidebar search (titles are matched in the user's
    /// language as well).
    pub fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::General => &[
                "paste",
                "direct paste",
                "wtype",
                "ydotool",
                "launch at login",
                "autostart",
                "startup",
                "ai provider",
                "chatgpt",
                "claude",
                "language",
            ],
            Self::Shortcuts => &[
                "hotkey",
                "keybinding",
                "keyboard",
                "global shortcut",
                "niri",
                "hyprland",
                "sway",
                "app shortcuts",
                "command shortcuts",
            ],
            Self::Search => &[
                "folders",
                "files",
                "exclusions",
                "hidden apps",
                "favorites",
                "pinned",
                "empty launcher",
            ],
            Self::Appearance => &[
                "theme",
                "dark",
                "light",
                "text size",
                "panel width",
                "compact",
                "transparency",
                "display",
                "monitor",
            ],
            Self::WindowManagement => &[
                "windows", "tile", "snap", "halves", "quarters", "thirds", "maximize", "center",
                "restore", "display",
            ],
            Self::Quicklinks => &["links", "web search", "keyword", "bookmarks", "url"],
            Self::Aliases => &["application names", "apps"],
            Self::Snippets => &["text", "placeholders", "keyword", "date", "clipboard"],
            Self::Scripts => &["executable", "arguments", "timeout"],
            Self::Extensions => &["manifest", "plugins"],
            Self::Profiles => &["switch", "duplicate"],
            Self::Clipboard => &["history", "retention", "pause", "privacy"],
            Self::Configuration => &["export", "import", "backup", "json", "merge", "replace"],
            Self::Help => &["commands", "examples", "welcome", "onboarding"],
            Self::About => &["version", "license", "credits"],
        }
    }

    /// Localized title.
    pub fn title(self) -> String {
        tr(self.title_key())
    }

    /// Localized description.
    pub fn subtitle(self) -> String {
        tr(self.subtitle_key())
    }

    /// Whether the sidebar search `query` selects this page: every word of the query must occur
    /// in the title, description or keywords (case- and diacritic-insensitive).
    pub fn matches(self, query: &str) -> bool {
        let wanted = normalize(query);
        if wanted.is_empty() {
            return true;
        }
        let haystack = normalize(&format!(
            "{} {} {} {}",
            self.title(),
            self.title_key(),
            self.subtitle(),
            self.keywords().join(" ")
        ));
        wanted.split(' ').all(|word| haystack.contains(word))
    }
}

/// Human-readable form of an accelerator: `<Control><Alt>t` becomes `Ctrl+Alt+T`.
pub fn accelerator_label(accelerator: &Accelerator) -> String {
    let mut parts: Vec<String> = Vec::new();
    let modifiers = accelerator.modifiers();
    for (bit, name) in [
        (CONTROL, "Ctrl"),
        (ALT, "Alt"),
        (SHIFT, "Shift"),
        (SUPER, "Super"),
    ] {
        if modifiers & bit != 0 {
            parts.push(name.to_owned());
        }
    }
    let key = accelerator.key();
    let mut chars = key.chars();
    let pretty = match (chars.next(), chars.next()) {
        (Some(c), None) => c.to_uppercase().collect::<String>(),
        (Some(first), Some(_)) => {
            let mut word: String = first.to_uppercase().collect();
            word.push_str(&key[first.len_utf8()..]);
            word.replace('_', " ")
        }
        _ => key.to_owned(),
    };
    parts.push(pretty);
    parts.join("+")
}

/// What a key press means to the shortcut recorder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recorded {
    /// Only modifiers are down so far; show them as feedback.
    Pending(u8),
    /// A complete chord.
    Chord(Accelerator),
    /// The key cannot be used (message is ready to show).
    Rejected(String),
}

/// Turn a key press (`key_name` is the GDK key name of the unshifted key, `None` for a pure
/// modifier press) plus the held modifiers into a recorder outcome.
pub fn record_key(modifiers: u8, key_name: Option<&str>) -> Recorded {
    let Some(name) = key_name else {
        return Recorded::Pending(modifiers);
    };
    match Accelerator::new(modifiers, name) {
        Ok(accelerator) if accelerator.has_command_modifier() => Recorded::Chord(accelerator),
        Ok(_) => Recorded::Rejected(tr("Include Control, Alt, or Super.")),
        Err(error) => Recorded::Rejected(tr(&error.to_string())),
    }
}

/// Label for the held modifiers while recording (`Ctrl+Alt+…`).
pub fn modifiers_label(modifiers: u8) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for (bit, name) in [
        (CONTROL, "Ctrl"),
        (ALT, "Alt"),
        (SHIFT, "Shift"),
        (SUPER, "Super"),
    ] {
        if modifiers & bit != 0 {
            parts.push(name);
        }
    }
    if parts.is_empty() {
        "…".to_owned()
    } else {
        format!("{}+…", parts.join("+"))
    }
}

/// Installed applications that match `query` (name, desktop id), at most `cap`, plus the number
/// of matches that did not fit.
pub fn filter_apps<'a>(
    apps: &'a [DesktopApp],
    query: &str,
    cap: usize,
) -> (Vec<&'a DesktopApp>, usize) {
    let wanted = normalize(query);
    let mut matching = apps.iter().filter(|app| {
        wanted.is_empty() || normalize(&format!("{} {}", app.name, app.id)).contains(&wanted)
    });
    let shown: Vec<&DesktopApp> = matching.by_ref().take(cap).collect();
    let hidden = matching.count();
    (shown, hidden)
}

/// One entry of the command-shortcut list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandTarget {
    /// Command id as stored in bindings (`emoji`, `link:<id>`, `snippet:<id>`, ...).
    pub id: String,
    /// Display title.
    pub title: String,
    /// Symbolic icon name.
    pub icon: &'static str,
}

/// Everything a shortcut can be assigned to: enabled scripts and extensions, the commands that
/// exist on Linux, quicklinks and snippets.
pub fn command_targets(
    settings: &PortableSettings,
    localizer: &kebloop_services::l10n::Localizer,
    commands: &crate::platform::CommandAvailability,
) -> Vec<CommandTarget> {
    let mut targets = Vec::new();
    for script in settings.scripts.iter().filter(|s| s.enabled) {
        targets.push(CommandTarget {
            id: script.command_id(),
            title: script.title.clone(),
            icon: "utilities-terminal-symbolic",
        });
    }
    for extension in settings.extensions.iter().filter(|e| e.enabled) {
        targets.push(CommandTarget {
            id: extension.command_id(),
            title: extension.name.clone(),
            icon: "application-x-addon-symbolic",
        });
    }
    for command in commands.commands() {
        targets.push(CommandTarget {
            id: command.id().to_owned(),
            title: command.title(localizer),
            icon: command.icon_name(),
        });
    }
    for link in &settings.quicklinks {
        targets.push(CommandTarget {
            id: format!("link:{}", link.id),
            title: link.name.clone(),
            icon: "insert-link-symbolic",
        });
    }
    for snippet in &settings.snippets {
        targets.push(CommandTarget {
            id: format!("snippet:{}", snippet.id),
            title: snippet.name.clone(),
            icon: "insert-text-symbolic",
        });
    }
    targets
}

/// Quote `text` for a POSIX shell only when needed.
pub fn shell_quote(text: &str) -> String {
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.:/@+=,".contains(c));
    if plain {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

/// The program name to put into compositor bindings: `kebloop` when it is on `PATH`, otherwise
/// the absolute path of the running executable.
pub fn kebloop_program(path_dirs: &[PathBuf], current_exe: Option<&Path>) -> String {
    if path_dirs
        .iter()
        .any(|dir| is_executable(&dir.join("kebloop")))
    {
        return "kebloop".to_owned();
    }
    current_exe.map_or_else(
        || "kebloop".to_owned(),
        |exe| shell_quote(&exe.to_string_lossy()),
    )
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// Command line that toggles the launcher.
pub fn toggle_command(program: &str) -> String {
    format!("{program} toggle")
}

/// Command line that runs the command `id`.
pub fn command_command(program: &str, id: &str) -> String {
    format!("{program} command {}", shell_quote(id))
}

/// Command line that launches the application `desktop_id`.
pub fn launch_command(program: &str, desktop_id: &str) -> String {
    format!("{program} launch {}", shell_quote(desktop_id))
}

/// Script/extension arguments from the multi-line editor: one literal argument per line, no
/// arguments for blank text. A single trailing newline is ignored.
pub fn arguments_from_text(text: &str) -> Vec<String> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    if text.trim().is_empty() {
        return Vec::new();
    }
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_owned())
        .collect()
}

/// Inverse of [`arguments_from_text`].
pub fn arguments_to_text(arguments: &[String]) -> String {
    arguments.join("\n")
}

/// Why a typed path was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathProblem {
    /// Not absolute and not `~/…`.
    NotAbsolute,
    /// Does not exist on this machine.
    Missing,
    /// Already in the list.
    Duplicate,
}

impl PathProblem {
    /// Localized explanation.
    pub fn message(self) -> String {
        match self {
            Self::NotAbsolute => tr("Enter an absolute path or a path beginning with ~/."),
            Self::Missing => tr("This path is unavailable. Choose an existing folder or app."),
            Self::Duplicate => tr("This path is already listed."),
        }
    }
}

/// Validate a typed folder path against the existing list. `exists` is injected for tests.
pub fn check_path(
    input: &str,
    existing: &[String],
    home: &str,
    exists: &dyn Fn(&Path) -> bool,
) -> Result<String, PathProblem> {
    let path = input.trim();
    if !(path.starts_with('/') || path.starts_with("~/")) {
        return Err(PathProblem::NotAbsolute);
    }
    let expanded = kebloop_services::util::expand_tilde(path, home);
    if !exists(Path::new(&expanded)) {
        return Err(PathProblem::Missing);
    }
    if existing.iter().any(|p| p == path) {
        return Err(PathProblem::Duplicate);
    }
    Ok(path.to_owned())
}

/// A path as shown in lists: the home directory becomes `~`.
pub fn display_path(path: &str, home: &str) -> String {
    match path.strip_prefix(home) {
        Some("") => "~".to_owned(),
        Some(rest) if rest.starts_with('/') && !home.is_empty() && home != "/" => {
            format!("~{rest}")
        }
        _ => path.to_owned(),
    }
}

/// Add `additions` to `list`, keeping it sorted and free of duplicates. Returns how many were new.
pub fn add_paths(list: &mut Vec<String>, additions: &[String]) -> usize {
    let before = list.len();
    for path in additions {
        if !path.is_empty() && !list.contains(path) {
            list.push(path.clone());
        }
    }
    list.sort();
    list.len() - before
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebloop_services::l10n::{Language, catalog_keys};

    #[test]
    fn every_page_has_translated_texts_and_a_unique_slug() {
        let english = catalog_keys(Language::English);
        let polish = catalog_keys(Language::Polish);
        let mut slugs = std::collections::HashSet::new();
        for page in PageId::ALL {
            for key in [
                page.title_key(),
                page.subtitle_key(),
                page.group().title_key(),
            ] {
                assert!(english.contains(key), "no English entry for {key:?}");
                assert!(polish.contains(key), "no Polish entry for {key:?}");
            }
            assert!(slugs.insert(page.slug()));
            assert_eq!(PageId::from_slug(page.slug()), Some(page));
        }
    }

    #[test]
    fn slugs_accept_macos_names_and_reject_unknown_pages() {
        assert_eq!(PageId::from_slug("appShortcuts"), Some(PageId::Shortcuts));
        assert_eq!(PageId::from_slug("webSearch"), Some(PageId::Quicklinks));
        assert_eq!(PageId::from_slug(" Clipboard "), Some(PageId::Clipboard));
        assert_eq!(PageId::from_slug("nope"), None);
    }

    #[test]
    fn sidebar_search_matches_titles_keywords_and_descriptions() {
        assert!(PageId::General.matches("login"));
        assert!(PageId::Shortcuts.matches("niri"));
        assert!(PageId::Configuration.matches("EXPORT json"));
        assert!(PageId::Clipboard.matches(""));
        assert!(!PageId::Appearance.matches("clipboard"));
    }

    #[test]
    fn accelerators_read_naturally() {
        let label = |text: &str| accelerator_label(&Accelerator::parse(text).unwrap());
        assert_eq!(label("<Alt>space"), "Alt+Space");
        assert_eq!(label("<Control><Alt>t"), "Ctrl+Alt+T");
        assert_eq!(label("<Shift><Super>F5"), "Shift+Super+F5");
        assert_eq!(label("<Super>Page_Up"), "Super+Page Up");
    }

    #[test]
    fn recorder_requires_a_command_modifier_and_a_valid_key() {
        assert_eq!(
            record_key(CONTROL | ALT, None),
            Recorded::Pending(CONTROL | ALT)
        );
        match record_key(CONTROL | SHIFT, Some("t")) {
            Recorded::Chord(chord) => assert_eq!(chord.to_string(), "<Shift><Control>t"),
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            record_key(SHIFT, Some("t")),
            Recorded::Rejected(_)
        ));
        assert!(matches!(record_key(0, Some("F5")), Recorded::Rejected(_)));
        assert!(matches!(
            record_key(CONTROL, Some("Not A Key")),
            Recorded::Rejected(_)
        ));
        assert_eq!(modifiers_label(CONTROL | SUPER), "Ctrl+Super+…");
        assert_eq!(modifiers_label(0), "…");
    }

    fn app(id: &str, name: &str) -> DesktopApp {
        DesktopApp {
            id: id.into(),
            path: PathBuf::from(id),
            name: name.into(),
            generic_name: None,
            keywords: Vec::new(),
            comment: None,
            icon: None,
            terminal: false,
            exec: String::new(),
            categories: Vec::new(),
            startup_wm_class: None,
            actions: Vec::new(),
            origin: kebloop_platform::apps::AppOrigin::System,
            subtitle: None,
        }
    }

    #[test]
    fn app_filter_caps_results_and_counts_the_rest() {
        let apps: Vec<DesktopApp> = (0..10)
            .map(|i| app(&format!("app{i}.desktop"), &format!("Édit {i}")))
            .chain([app("firefox.desktop", "Firefox")])
            .collect();
        let (shown, more) = filter_apps(&apps, "edit", 4);
        assert_eq!((shown.len(), more), (4, 6));
        let (shown, more) = filter_apps(&apps, "fire", 4);
        assert_eq!((shown.len(), more), (1, 0));
        let (shown, more) = filter_apps(&apps, "", 3);
        assert_eq!((shown.len(), more), (3, 8));
    }

    #[test]
    fn shell_quoting_and_commands() {
        assert_eq!(shell_quote("snippet:abc-1"), "snippet:abc-1");
        assert_eq!(shell_quote("my app.desktop"), "'my app.desktop'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(toggle_command("kebloop"), "kebloop toggle");
        assert_eq!(
            command_command("kebloop", "link:1"),
            "kebloop command link:1"
        );
        assert_eq!(
            launch_command("kebloop", "org.mozilla.firefox.desktop"),
            "kebloop launch org.mozilla.firefox.desktop"
        );
    }

    #[test]
    fn program_name_prefers_path_lookup() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("kebloop");
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let other = Path::new("/opt/my apps/kebloop");
        assert_eq!(
            kebloop_program(&[dir.path().to_path_buf()], Some(other)),
            "kebloop"
        );
        assert_eq!(kebloop_program(&[], Some(other)), "'/opt/my apps/kebloop'");
        assert_eq!(kebloop_program(&[], None), "kebloop");
    }

    #[test]
    fn arguments_round_trip_one_per_line() {
        assert!(arguments_from_text("").is_empty());
        assert!(arguments_from_text("  \n").is_empty());
        assert_eq!(
            arguments_from_text("-c\necho hi\n"),
            vec!["-c".to_owned(), "echo hi".to_owned()]
        );
        assert_eq!(arguments_from_text("a\r\n\nb"), ["a", "", "b"]);
        let args = vec!["x y".to_owned(), "z".to_owned()];
        assert_eq!(arguments_from_text(&arguments_to_text(&args)), args);
    }

    #[test]
    fn typed_paths_are_validated() {
        let exists = |p: &Path| p.starts_with("/home/u/Docs") || p == Path::new("/srv");
        let existing = vec!["/srv".to_owned()];
        assert_eq!(
            check_path("Docs", &[], "/home/u", &exists),
            Err(PathProblem::NotAbsolute)
        );
        assert_eq!(
            check_path("~/Docs", &[], "/home/u", &exists),
            Ok("~/Docs".to_owned())
        );
        assert_eq!(
            check_path("/nope", &[], "/home/u", &exists),
            Err(PathProblem::Missing)
        );
        assert_eq!(
            check_path(" /srv ", &existing, "/home/u", &exists),
            Err(PathProblem::Duplicate)
        );
        assert_eq!(display_path("/home/u/Docs", "/home/u"), "~/Docs");
        assert_eq!(display_path("/home/user2/x", "/home/u"), "/home/user2/x");
        let mut list = vec!["/b".to_owned()];
        assert_eq!(
            add_paths(&mut list, &["/a".into(), "/b".into(), String::new()]),
            1
        );
        assert_eq!(list, ["/a", "/b"]);
    }

    #[test]
    fn command_targets_cover_enabled_automation_links_and_snippets() {
        use kebloop_services::automation::ScriptCommand;
        use kebloop_services::quicklinks::Quicklink;
        use kebloop_services::snippets::Snippet;
        let mut settings = PortableSettings::default();
        let mut enabled = ScriptCommand::new("s1", "Backup", "/bin/true", Vec::new());
        enabled.enabled = true;
        settings.scripts = vec![
            enabled,
            ScriptCommand::new("s2", "Off", "/bin/true", Vec::new()),
        ];
        settings.quicklinks = vec![Quicklink {
            id: "q".into(),
            name: "Docs".into(),
            target: "https://example.com".into(),
            keyword: String::new(),
        }];
        settings.snippets = vec![Snippet::new("Sig", "bye")];
        let ids: Vec<String> = command_targets(
            &settings,
            &kebloop_services::l10n::Localizer::english(),
            &crate::platform::CommandAvailability::new(
                false,
                [kebloop_platform::system_actions::SystemAction::Lock],
                Some(kebloop_platform::compositor::CompositorKind::Sway),
            ),
        )
        .into_iter()
        .map(|t| t.id)
        .collect();
        assert_eq!(ids[0], "script:s1");
        assert!(!ids.contains(&"script:s2".to_owned()));
        assert!(ids.contains(&"emoji".to_owned()));
        assert!(!ids.contains(&"appleShortcuts".to_owned()));
        assert!(ids.contains(&"link:q".to_owned()));
        assert!(ids.iter().any(|id| id.starts_with("snippet:")));
    }
}
