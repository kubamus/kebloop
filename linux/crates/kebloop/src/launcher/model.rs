//! GTK-free result model: what the launcher lists for a query, in which order, with which
//! titles. Everything here is a pure function of its inputs so it can be unit tested.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use kebloop_core::{AppSearch, CalculationOutcome, CalculationResult, Searchable};
use kebloop_platform::apps::DesktopApp;
use kebloop_services::automation::{ExtensionManifest, ScriptCommand};
use kebloop_services::calculation_history::CalculationHistoryEntry;
use kebloop_services::clipboard::ClipboardEntry;
use kebloop_services::commands::LauncherCommand;
use kebloop_services::config::{ConfigurationProfile, PortableSettings};
use kebloop_services::file_search::{FileSearchResult, ROOT_RESULTS};
use kebloop_services::l10n::Localizer;
use kebloop_services::personalization::PinnedResult;
use kebloop_services::quicklinks::{self, Destination, Quicklink};
use kebloop_services::snippets::Snippet;
use kebloop_services::util::fold;

use crate::platform::{CommandAvailability, WindowNote};

/// Maximum ranked (non-file) results in root search.
pub const ROOT_RANKED_LIMIT: usize = 12;
/// Applications shown in the "frequent" empty state.
pub const FREQUENT_APPS: usize = 3;

/// How a row's icon is found. Resolved lazily by the GTK layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconSpec {
    /// A symbolic icon name from the icon theme.
    Symbolic(&'static str),
    /// A themed icon name or an absolute image path from a desktop entry.
    App(Option<String>),
    /// A file or folder; the icon follows its content type.
    File {
        /// Absolute path.
        path: String,
        /// Whether it is a directory.
        is_dir: bool,
    },
    /// A clipboard image thumbnail, loaded on demand by entry id.
    ClipboardImage(String),
}

/// What happens when a result is chosen.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// Launch a desktop application.
    Application {
        /// Desktop-file id.
        desktop_id: String,
        /// Path of the `.desktop` file.
        path: String,
    },
    /// Copy a calculator answer (and remember it in the calculator history).
    Calculation {
        /// The answer; `clipboard` is the text to copy.
        result: CalculationResult,
    },
    /// Copy an answer from the calculator history.
    HistoricalCalculation {
        /// The saved answer.
        result: CalculationResult,
    },
    /// A built-in command.
    Command(LauncherCommand),
    /// Open a quicklink, with the keyword argument (possibly empty).
    Quicklink {
        /// The link (a synthetic one for web-search keywords).
        link: Quicklink,
        /// Text after the keyword.
        argument: String,
    },
    /// A snippet.
    Snippet(Snippet),
    /// An enabled script command.
    Script(ScriptCommand),
    /// An enabled extension.
    Extension(ExtensionManifest),
    /// A file or folder.
    File {
        /// Absolute path.
        path: String,
        /// Whether it is a directory.
        is_dir: bool,
    },
    /// Search the web for the query with the first template.
    WebSearch {
        /// Query text.
        query: String,
    },
    /// A clipboard history entry.
    Clipboard {
        /// Entry id.
        entry_id: String,
        /// Whether it is pinned.
        pinned: bool,
    },
    /// A result row of the extension browser.
    ExtensionResult {
        /// Result id within the extension's response.
        result_id: String,
    },
    /// A profile.
    Profile {
        /// Profile id.
        id: String,
        /// Whether it is the active profile.
        active: bool,
    },
}

/// One row of the result list.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultItem {
    /// Stable identifier (usage, pins and selection preservation).
    pub id: String,
    /// What choosing the row does.
    pub target: Target,
    /// Primary text.
    pub title: String,
    /// Secondary text.
    pub subtitle: String,
    /// Icon.
    pub icon: IconSpec,
    /// Name of the primary action ("Open", "Copy answer", ...).
    pub action: String,
}

/// Result of assembling a list.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Assembled {
    /// Rows in display order.
    pub items: Vec<ResultItem>,
    /// A line shown with the list (calculator error).
    pub message: Option<String>,
}

/// A rankable candidate that borrows its data.
struct Candidate<'a> {
    id: Cow<'a, str>,
    name: Cow<'a, str>,
    aliases: Cow<'a, [String]>,
    keywords: Cow<'a, [String]>,
    index: usize,
}

impl Searchable for Candidate<'_> {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn aliases(&self) -> &[String] {
        &self.aliases
    }
    fn keywords(&self) -> &[String] {
        &self.keywords
    }
}

/// Secondary search keywords of an application: its desktop-entry `Keywords=` plus its
/// `GenericName=` ("Web Browser"), so "browser" finds Firefox and "terminal" finds a terminal.
fn app_keywords(app: &DesktopApp) -> Cow<'_, [String]> {
    match &app.generic_name {
        Some(generic) if !generic.is_empty() => {
            let mut keywords = Vec::with_capacity(app.keywords.len() + 1);
            keywords.push(generic.clone());
            keywords.extend(app.keywords.iter().cloned());
            Cow::Owned(keywords)
        }
        _ => Cow::Borrowed(app.keywords.as_slice()),
    }
}

/// Everything root search needs.
pub struct RootInputs<'a> {
    /// Raw query as typed.
    pub query: &'a str,
    /// Active profile settings.
    pub settings: &'a PortableSettings,
    /// Translations.
    pub l10n: &'a Localizer,
    /// Installed applications.
    pub apps: &'a [DesktopApp],
    /// Launch counts by result id.
    pub usage: &'a HashMap<String, i64>,
    /// Recently used result ids, newest first.
    pub recents: &'a [String],
    /// File matches (already limited to searchable folders).
    pub files: &'a [FileSearchResult],
    /// Paths among `files` that are directories.
    pub directories: &'a std::collections::HashSet<String>,
    /// Calculator verdict for the query.
    pub calculation: &'a CalculationOutcome,
    /// Which platform commands (Shut Down, system actions, window management) are offered.
    pub availability: &'a CommandAvailability,
    /// Desktop ids of applications that have a window (shown as "Running").
    pub running: &'a HashSet<String>,
    /// Home directory, for quicklink targets.
    pub home: &'a str,
    /// Whether the pinned file still exists (injected for tests).
    pub file_exists: &'a dyn Fn(&str) -> bool,
}

/// Result id of an application.
pub fn app_result_id(app: &DesktopApp) -> String {
    app.id.clone()
}

/// Result id of a built-in command.
pub fn command_result_id(command: LauncherCommand) -> String {
    format!("command:{}", command.id())
}

/// Result id of a file.
pub fn file_result_id(path: &str) -> String {
    format!("file:{path}")
}

/// `true` when the application is hidden by the user (by desktop id or desktop-file path).
pub fn is_hidden(settings: &PortableSettings, app: &DesktopApp) -> bool {
    let path = app.path.to_string_lossy();
    settings
        .personalization
        .hidden_apps
        .iter()
        .any(|hidden| *hidden == app.id || *hidden == path)
}

fn destination_text(destination: &Destination) -> String {
    match destination {
        Destination::Url(url) => url.to_string(),
        Destination::File(path) => path.display().to_string(),
    }
}

fn quicklink_subtitle(link: &Quicklink, argument: &str, home: &str) -> String {
    if link.keyword.is_empty() {
        return link.target.clone();
    }
    let resolved = link
        .resolved(argument, home)
        .map_or_else(|| link.target.clone(), |d| destination_text(&d));
    format!("{} · {resolved}", link.keyword)
}

fn abbreviate_home(path: &str, home: &str) -> String {
    let home = home.trim_end_matches('/');
    if !home.is_empty() && path == home {
        return "~".into();
    }
    match path.strip_prefix(home) {
        Some(rest) if !home.is_empty() && rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_owned(),
    }
}

/// A result row for an application.
pub fn application_item(
    app: &DesktopApp,
    all_apps: &[DesktopApp],
    l10n: &Localizer,
    home: &str,
    running: bool,
) -> ResultItem {
    let duplicate = all_apps
        .iter()
        .any(|other| other.id != app.id && other.name == app.name);
    let subtitle = if let Some(subtitle) = app.subtitle.as_ref().filter(|_| duplicate) {
        format!(
            "{subtitle} · {}",
            abbreviate_home(&app.path.to_string_lossy(), home)
        )
    } else {
        app.generic_name
            .clone()
            .filter(|g| !g.is_empty() && *g != app.name)
            .unwrap_or_else(|| l10n.tr("Application"))
    };
    let subtitle = if running {
        format!("{subtitle} · {}", l10n.tr("Running"))
    } else {
        subtitle
    };
    ResultItem {
        id: app_result_id(app),
        target: Target::Application {
            desktop_id: app.id.clone(),
            path: app.path.to_string_lossy().into_owned(),
        },
        title: app.name.clone(),
        subtitle,
        icon: IconSpec::App(app.icon.clone()),
        action: l10n.tr("Open"),
    }
}

/// A result row for a built-in command.
pub fn command_item(
    command: LauncherCommand,
    l10n: &Localizer,
    note: Option<WindowNote>,
) -> ResultItem {
    let subtitle = match command {
        LauncherCommand::Settings => l10n.tr("Preferences and shortcuts"),
        LauncherCommand::Shutdown => l10n.tr("Powers off this computer after confirmation"),
        _ => match note {
            Some(note) => format!("{} · {}", l10n.tr("Command"), window_note_text(note, l10n)),
            None => l10n.tr("Command"),
        },
    };
    ResultItem {
        id: command_result_id(command),
        target: Target::Command(command),
        title: command.title(l10n),
        subtitle,
        icon: IconSpec::Symbolic(command.icon_name()),
        action: l10n.tr("Open"),
    }
}

/// How a window command differs from macOS on this compositor, for the row subtitle.
pub fn window_note_text(note: WindowNote, l10n: &Localizer) -> String {
    match note {
        WindowNote::Approximate(text) => l10n.format("Approximate: %@", &[l10n.tr(text).into()]),
        WindowNote::FloatingOnly => l10n.tr("Needs a floating window"),
    }
}

/// A result row for a quicklink.
pub fn quicklink_item(
    link: &Quicklink,
    argument: &str,
    l10n: &Localizer,
    home: &str,
) -> ResultItem {
    ResultItem {
        id: format!("link:{}", link.id),
        target: Target::Quicklink {
            link: link.clone(),
            argument: argument.to_owned(),
        },
        title: link.name.clone(),
        subtitle: quicklink_subtitle(link, argument, home),
        icon: IconSpec::Symbolic("insert-link-symbolic"),
        action: l10n.tr("Open"),
    }
}

/// A result row for a snippet.
pub fn snippet_item(snippet: &Snippet, l10n: &Localizer) -> ResultItem {
    let preview: String = snippet
        .template
        .chars()
        .take(120)
        .map(|c| if c == '\n' { ' ' } else { c })
        .collect();
    ResultItem {
        id: format!("snippet:{}", snippet.id),
        target: Target::Snippet(snippet.clone()),
        title: snippet.name.clone(),
        subtitle: preview,
        icon: IconSpec::Symbolic("insert-text-symbolic"),
        action: l10n.tr("Preview"),
    }
}

/// A result row for a script command.
pub fn script_item(script: &ScriptCommand, l10n: &Localizer) -> ResultItem {
    ResultItem {
        id: script.command_id(),
        target: Target::Script(script.clone()),
        title: script.title.clone(),
        subtitle: script.description.clone(),
        icon: IconSpec::Symbolic("utilities-terminal-symbolic"),
        action: l10n.tr("Run"),
    }
}

/// A result row for an extension.
pub fn extension_item(manifest: &ExtensionManifest, l10n: &Localizer) -> ResultItem {
    ResultItem {
        id: manifest.command_id(),
        target: Target::Extension(manifest.clone()),
        title: manifest.name.clone(),
        subtitle: manifest.description.clone(),
        icon: IconSpec::Symbolic("application-x-addon-symbolic"),
        action: l10n.tr("Open"),
    }
}

/// A result row for a file.
pub fn file_item(
    file: &FileSearchResult,
    is_dir: bool,
    l10n: &Localizer,
    home: &str,
) -> ResultItem {
    ResultItem {
        id: file_result_id(&file.path),
        target: Target::File {
            path: file.path.clone(),
            is_dir,
        },
        title: file.name.clone(),
        subtitle: abbreviate_home(&file.path, home),
        icon: IconSpec::File {
            path: file.path.clone(),
            is_dir,
        },
        action: l10n.tr("Open"),
    }
}

fn web_search_item(query: &str, l10n: &Localizer) -> ResultItem {
    ResultItem {
        id: "web-search".into(),
        target: Target::WebSearch {
            query: query.to_owned(),
        },
        title: l10n.format("Search the web for %@", &[query.into()]),
        subtitle: l10n.tr("Opens your configured search website"),
        icon: IconSpec::Symbolic("system-search-symbolic"),
        action: l10n.tr("Search"),
    }
}

fn calculation_item(result: &CalculationResult, l10n: &Localizer) -> ResultItem {
    ResultItem {
        id: "calculator".into(),
        target: Target::Calculation {
            result: result.clone(),
        },
        title: result.display.clone(),
        subtitle: result.expression.clone(),
        icon: IconSpec::Symbolic("accessories-calculator-symbolic"),
        action: l10n.tr("Copy Answer"),
    }
}

/// A result row for an entry of the calculator history.
pub fn history_item(
    entry: &CalculationHistoryEntry,
    l10n: &Localizer,
    time_zone: &jiff::tz::TimeZone,
) -> ResultItem {
    ResultItem {
        id: entry.result_id(),
        target: Target::HistoricalCalculation {
            result: entry.result(),
        },
        title: entry.display.clone(),
        subtitle: format!(
            "{} · {}",
            entry.expression,
            format_time(entry.accepted_at.into(), time_zone)
        ),
        icon: IconSpec::Symbolic("document-open-recent-symbolic"),
        action: l10n.tr("Copy Answer"),
    }
}

/// Built-in commands offered right now.
pub fn available_commands(
    settings: &PortableSettings,
    availability: &CommandAvailability,
) -> Vec<LauncherCommand> {
    if !settings.personalization.commands_enabled {
        return Vec::new();
    }
    availability.commands().collect()
}

/// Rank `candidates` against `query`; returns the candidates' `index` values best first.
fn rank(query: &str, candidates: &[Candidate<'_>], usage: &HashMap<String, i64>) -> Vec<usize> {
    AppSearch::new()
        .ranked(query, candidates, usage)
        .into_iter()
        .map(|c| c.index)
        .collect()
}

/// Where a pooled candidate's row comes from. Application rows are built lazily because there
/// can be thousands of them and only a handful are ever shown.
enum Source<'a> {
    App(&'a DesktopApp),
    Item(Box<ResultItem>),
}

/// A flat list of everything root search can return.
struct Pool<'a> {
    sources: Vec<Source<'a>>,
    candidates: Vec<Candidate<'a>>,
}

impl<'a> Pool<'a> {
    fn push_item(&mut self, item: ResultItem, aliases: Cow<'a, [String]>) {
        let index = self.sources.len();
        self.candidates.push(Candidate {
            id: Cow::Owned(item.id.clone()),
            name: Cow::Owned(item.title.clone()),
            aliases,
            keywords: Cow::Borrowed(&[][..]),
            index,
        });
        self.sources.push(Source::Item(Box::new(item)));
    }

    fn item(&self, index: usize, inputs: &RootInputs<'_>) -> ResultItem {
        match &self.sources[index] {
            Source::App(app) => application_item(
                app,
                inputs.apps,
                inputs.l10n,
                inputs.home,
                inputs.running.contains(&app.id),
            ),
            Source::Item(item) => (**item).clone(),
        }
    }

    fn is_command(&self, index: usize) -> bool {
        matches!(&self.sources[index], Source::Item(item) if matches!(item.target, Target::Command(_)))
    }

    fn is_app(&self, index: usize) -> bool {
        matches!(self.sources[index], Source::App(_))
    }
}

fn build_pool<'a>(inputs: &'a RootInputs<'a>) -> Pool<'a> {
    let settings = inputs.settings;
    let personalization = &settings.personalization;
    let l10n = inputs.l10n;
    let mut pool = Pool {
        sources: Vec::with_capacity(inputs.apps.len() + 32),
        candidates: Vec::with_capacity(inputs.apps.len() + 32),
    };
    if personalization.apps_enabled {
        for app in inputs.apps.iter().filter(|a| !is_hidden(settings, a)) {
            let aliases = settings
                .aliases
                .get(&app.id)
                .map_or(Cow::Borrowed(&[][..]), |alias| {
                    Cow::Borrowed(std::slice::from_ref(alias))
                });
            let index = pool.sources.len();
            pool.candidates.push(Candidate {
                id: Cow::Borrowed(&app.id),
                name: Cow::Borrowed(&app.name),
                aliases,
                keywords: app_keywords(app),
                index,
            });
            pool.sources.push(Source::App(app));
        }
    }
    if personalization.links_enabled {
        for link in &settings.quicklinks {
            pool.push_item(
                quicklink_item(link, "", l10n, inputs.home),
                Cow::Borrowed(&[][..]),
            );
        }
    }
    if personalization.commands_enabled {
        for command in available_commands(settings, inputs.availability) {
            let aliases: Vec<String> = command.aliases().iter().map(|a| (*a).to_owned()).collect();
            pool.push_item(
                command_item(command, l10n, inputs.availability.window_note(command)),
                Cow::Owned(aliases),
            );
        }
        for snippet in &settings.snippets {
            pool.push_item(snippet_item(snippet, l10n), Cow::Borrowed(&[][..]));
        }
        for script in settings.scripts.iter().filter(|s| s.enabled) {
            pool.push_item(script_item(script, l10n), Cow::Borrowed(&[][..]));
        }
        for manifest in settings.extensions.iter().filter(|m| m.enabled) {
            pool.push_item(extension_item(manifest, l10n), Cow::Borrowed(&[][..]));
        }
    }
    pool
}

fn pinned_items(
    pins: &[PinnedResult],
    pool: &Pool<'_>,
    inputs: &RootInputs<'_>,
) -> Vec<ResultItem> {
    let personalization = &inputs.settings.personalization;
    pins.iter()
        .filter_map(|pin| {
            if let Some(candidate) = pool.candidates.iter().find(|c| c.id == pin.id) {
                return Some(pool.item(candidate.index, inputs));
            }
            if pin.kind == "file"
                && personalization.files_enabled
                && personalization.allows_file(&pin.path, inputs.home)
                && (inputs.file_exists)(&pin.path)
            {
                let file = FileSearchResult::new(pin.name.clone(), pin.path.clone());
                return Some(file_item(&file, false, inputs.l10n, inputs.home));
            }
            None
        })
        .collect()
}

fn dedupe(items: Vec<ResultItem>) -> Vec<ResultItem> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.id.clone()))
        .collect()
}

/// The empty-query list: favorites plus recent or frequent applications, then commands.
fn assemble_empty(inputs: &RootInputs<'_>, pool: &Pool<'_>) -> Vec<ResultItem> {
    let settings = inputs.settings;
    let favorites = pinned_items(&settings.pins, pool, inputs);
    let favorite_ids: Vec<&str> = favorites.iter().map(|i| i.id.as_str()).collect();
    let commands: Vec<ResultItem> = pool
        .candidates
        .iter()
        .filter(|c| pool.is_command(c.index) && !favorite_ids.contains(&c.id.as_ref()))
        .map(|c| pool.item(c.index, inputs))
        .collect();
    let items = match settings.personalization.empty_mode.as_str() {
        "favorites" => {
            let mut items = favorites.clone();
            items.extend(commands);
            items
        }
        "recent" => {
            let mut items: Vec<ResultItem> = inputs
                .recents
                .iter()
                .filter_map(|id| {
                    pool.candidates
                        .iter()
                        .find(|c| c.id == *id && !pool.is_command(c.index))
                })
                .map(|c| pool.item(c.index, inputs))
                .collect();
            items.extend(commands);
            items
        }
        _ => {
            let mut frequent: Vec<(&Candidate<'_>, i64)> = pool
                .candidates
                .iter()
                .filter(|c| pool.is_app(c.index) && !favorite_ids.contains(&c.id.as_ref()))
                .map(|c| (c, inputs.usage.get(c.id.as_ref()).copied().unwrap_or(0)))
                .collect();
            // Most used first, ties by name.
            frequent.sort_by(|a, b| {
                b.1.cmp(&a.1)
                    .then_with(|| fold(a.0.name.as_ref()).cmp(&fold(b.0.name.as_ref())))
            });
            let mut items = favorites.clone();
            items.extend(
                frequent
                    .into_iter()
                    .take(FREQUENT_APPS)
                    .map(|(c, _)| pool.item(c.index, inputs)),
            );
            items.extend(commands);
            items
        }
    };
    dedupe(items)
}

/// Root search results for `inputs.query`.
pub fn assemble_root(inputs: &RootInputs<'_>) -> Assembled {
    let settings = inputs.settings;
    let personalization = &settings.personalization;
    let pool = build_pool(inputs);
    let trimmed = inputs.query.trim();

    if trimmed.is_empty() {
        return Assembled {
            items: assemble_empty(inputs, &pool),
            message: None,
        };
    }

    let ranked = rank(trimmed, &pool.candidates, inputs.usage);
    let mut items: Vec<ResultItem> = ranked
        .into_iter()
        .take(ROOT_RANKED_LIMIT)
        .map(|index| pool.item(index, inputs))
        .collect();

    if personalization.files_enabled {
        items.extend(
            inputs
                .files
                .iter()
                .filter(|f| personalization.allows_file(&f.path, inputs.home))
                .take(ROOT_RESULTS)
                .map(|f| {
                    file_item(
                        f,
                        inputs.directories.contains(&f.path),
                        inputs.l10n,
                        inputs.home,
                    )
                }),
        );
    }
    if personalization.links_enabled {
        if let Some(matched) =
            quicklinks::resolve_query(&settings.quicklinks, &settings.web_searches, inputs.query)
        {
            let item = quicklink_item(&matched.link, &matched.argument, inputs.l10n, inputs.home);
            items.insert(0, item);
            items = dedupe(items);
        }
        if items.is_empty()
            && quicklinks::fallback_url(&settings.web_searches, trimmed, inputs.home).is_some()
        {
            items.push(web_search_item(trimmed, inputs.l10n));
        }
    }
    let mut message = None;
    match inputs.calculation {
        CalculationOutcome::Result(result) => {
            items.insert(0, calculation_item(result, inputs.l10n));
        }
        CalculationOutcome::Failure(error) => message = Some(inputs.l10n.tr(error)),
        CalculationOutcome::None | CalculationOutcome::Incomplete => {}
    }
    Assembled { items, message }
}

/// Snippet list (`Snippets` view).
pub fn assemble_snippets(
    query: &str,
    snippets: &[Snippet],
    usage: &HashMap<String, i64>,
    l10n: &Localizer,
) -> Vec<ResultItem> {
    let candidates: Vec<Candidate<'_>> = snippets
        .iter()
        .enumerate()
        .map(|(index, s)| Candidate {
            id: Cow::Owned(format!("snippet:{}", s.id)),
            name: Cow::Borrowed(&s.name),
            aliases: Cow::Borrowed(&[][..]),
            keywords: Cow::Borrowed(&[][..]),
            index,
        })
        .collect();
    rank(query.trim(), &candidates, usage)
        .into_iter()
        .map(|i| snippet_item(&snippets[i], l10n))
        .collect()
}

/// Quicklink list (`Quicklinks` view).
pub fn assemble_quicklinks(
    query: &str,
    links: &[Quicklink],
    usage: &HashMap<String, i64>,
    l10n: &Localizer,
    home: &str,
) -> Vec<ResultItem> {
    let candidates: Vec<Candidate<'_>> = links
        .iter()
        .enumerate()
        .map(|(index, l)| Candidate {
            id: Cow::Owned(format!("link:{}", l.id)),
            name: Cow::Borrowed(&l.name),
            aliases: Cow::Borrowed(&[][..]),
            keywords: Cow::Borrowed(&[][..]),
            index,
        })
        .collect();
    rank(query.trim(), &candidates, usage)
        .into_iter()
        .map(|i| quicklink_item(&links[i], "", l10n, home))
        .collect()
}

/// Script list (`Script Commands` view): enabled scripts only.
pub fn assemble_scripts(
    query: &str,
    scripts: &[ScriptCommand],
    l10n: &Localizer,
) -> Vec<ResultItem> {
    let enabled: Vec<&ScriptCommand> = scripts.iter().filter(|s| s.enabled).collect();
    let candidates: Vec<Candidate<'_>> = enabled
        .iter()
        .enumerate()
        .map(|(index, s)| Candidate {
            id: Cow::Owned(s.command_id()),
            name: Cow::Borrowed(&s.title),
            aliases: Cow::Borrowed(&[][..]),
            keywords: Cow::Borrowed(&[][..]),
            index,
        })
        .collect();
    rank(query.trim(), &candidates, &HashMap::new())
        .into_iter()
        .map(|i| script_item(enabled[i], l10n))
        .collect()
}

/// Extension list (`Extensions` view): enabled extensions only.
pub fn assemble_extensions(
    query: &str,
    manifests: &[ExtensionManifest],
    l10n: &Localizer,
) -> Vec<ResultItem> {
    let enabled: Vec<&ExtensionManifest> = manifests.iter().filter(|m| m.enabled).collect();
    let candidates: Vec<Candidate<'_>> = enabled
        .iter()
        .enumerate()
        .map(|(index, m)| Candidate {
            id: Cow::Owned(m.command_id()),
            name: Cow::Borrowed(&m.name),
            aliases: Cow::Borrowed(&[][..]),
            keywords: Cow::Borrowed(&[][..]),
            index,
        })
        .collect();
    rank(query.trim(), &candidates, &HashMap::new())
        .into_iter()
        .map(|i| extension_item(enabled[i], l10n))
        .collect()
}

/// Web search view: one row once there is a query and a template.
pub fn assemble_web_search(
    query: &str,
    settings: &PortableSettings,
    l10n: &Localizer,
    home: &str,
) -> Vec<ResultItem> {
    let trimmed = query.trim();
    if trimmed.is_empty()
        || quicklinks::fallback_url(&settings.web_searches, trimmed, home).is_none()
    {
        return Vec::new();
    }
    vec![web_search_item(trimmed, l10n)]
}

/// Calculator history view: newest first, filtered by `query` (expression or answer).
pub fn assemble_calculation_history(
    query: &str,
    entries: &[CalculationHistoryEntry],
    l10n: &Localizer,
    time_zone: &jiff::tz::TimeZone,
) -> Vec<ResultItem> {
    let query = query.trim();
    entries
        .iter()
        .filter(|entry| entry.matches(query))
        .map(|entry| history_item(entry, l10n, time_zone))
        .collect()
}

/// Profile list.
pub fn assemble_profiles(
    query: &str,
    profiles: &[ConfigurationProfile],
    active_id: &str,
    l10n: &Localizer,
) -> Vec<ResultItem> {
    let candidates: Vec<Candidate<'_>> = profiles
        .iter()
        .enumerate()
        .map(|(index, p)| Candidate {
            id: Cow::Owned(format!("profile:{}", p.id)),
            name: Cow::Borrowed(&p.name),
            aliases: Cow::Borrowed(&[][..]),
            keywords: Cow::Borrowed(&[][..]),
            index,
        })
        .collect();
    let order = if query.trim().is_empty() {
        (0..profiles.len()).collect()
    } else {
        rank(query.trim(), &candidates, &HashMap::new())
    };
    order
        .into_iter()
        .map(|i| {
            let profile = &profiles[i];
            let active = profile.id == active_id;
            ResultItem {
                id: format!("profile:{}", profile.id),
                target: Target::Profile {
                    id: profile.id.clone(),
                    active,
                },
                title: profile.name.clone(),
                subtitle: if active {
                    l10n.tr("Active")
                } else {
                    l10n.tr("Local profile")
                },
                icon: IconSpec::Symbolic(if active {
                    "object-select-symbolic"
                } else {
                    "avatar-default-symbolic"
                }),
                action: l10n.tr("Switch"),
            }
        })
        .collect()
}

/// "Browse Commands": built-in commands, user commands and quicklinks, optionally filtered.
/// `shortcuts` maps command ids to their shortcut label.
pub fn assemble_browse(
    inputs: &RootInputs<'_>,
    shortcuts: &HashMap<String, String>,
) -> Vec<ResultItem> {
    let mut settings = inputs.settings.clone();
    settings.personalization.apps_enabled = false;
    settings.personalization.commands_enabled = true;
    settings.personalization.links_enabled = true;
    let local = RootInputs {
        settings: &settings,
        query: inputs.query,
        l10n: inputs.l10n,
        apps: &[],
        usage: inputs.usage,
        recents: &[],
        files: &[],
        directories: inputs.directories,
        calculation: &CalculationOutcome::None,
        availability: inputs.availability,
        running: inputs.running,
        home: inputs.home,
        file_exists: inputs.file_exists,
    };
    let pool = build_pool(&local);
    let query = inputs.query.trim();
    let order: Vec<usize> = if query.is_empty() {
        (0..pool.sources.len()).collect()
    } else {
        rank(query, &pool.candidates, inputs.usage)
    };
    order
        .into_iter()
        .map(|index| {
            let mut item = pool.item(index, &local);
            let key = match &item.target {
                Target::Command(c) => Some(c.id().to_owned()),
                Target::Quicklink { link, .. } => Some(format!("link:{}", link.id)),
                Target::Snippet(s) => Some(format!("snippet:{}", s.id)),
                Target::Script(s) => Some(s.command_id()),
                Target::Extension(m) => Some(m.command_id()),
                _ => None,
            };
            if let Some(label) = key.and_then(|k| shortcuts.get(&k)) {
                item.subtitle = format!("{} · {label}", item.subtitle);
            }
            item
        })
        .collect()
}

/// Folded match text of clipboard entries, computed once per entry and reused across keystrokes.
/// Keyed by entry id and validated by the text length: an id keeps its text for its whole life
/// (re-copying the same text reuses the entry).
#[derive(Debug, Default)]
pub struct ClipboardFoldCache {
    entries: HashMap<String, (usize, String)>,
    placeholder: String,
}

impl ClipboardFoldCache {
    /// Folded text of the entry (the localized placeholder for entries without text, which is
    /// never cached because it depends on the language). Entries can be 100 KB each, so the
    /// folding is done once per entry instead of on every keystroke.
    fn folded(&mut self, entry: &ClipboardEntry, l10n: &Localizer) -> &str {
        if entry.text.is_empty() {
            return self.placeholder(entry, l10n);
        }
        let stale = self
            .entries
            .get(&entry.id)
            .is_none_or(|(length, _)| *length != entry.text.len());
        if stale {
            self.entries
                .insert(entry.id.clone(), (entry.text.len(), fold(&entry.text)));
        }
        self.entries.get(&entry.id).map_or("", |(_, folded)| folded)
    }

    fn placeholder(&mut self, entry: &ClipboardEntry, l10n: &Localizer) -> &str {
        self.placeholder = fold(&entry.searchable_text(l10n));
        &self.placeholder
    }

    /// Forget entries that no longer exist.
    fn retain_present(&mut self, entries: &[ClipboardEntry]) {
        if self.entries.len() > entries.len() {
            let present: std::collections::HashSet<&str> =
                entries.iter().map(|e| e.id.as_str()).collect();
            self.entries.retain(|id, _| present.contains(id.as_str()));
        }
    }
}

thread_local! {
    static CLIPBOARD_FOLDS: std::cell::RefCell<ClipboardFoldCache> =
        std::cell::RefCell::new(ClipboardFoldCache::default());
}

/// Clipboard history view: pinned first, newest first, filtered by `query` (matched against the
/// whole text of each entry).
pub fn assemble_clipboard(
    query: &str,
    entries: &[ClipboardEntry],
    l10n: &Localizer,
    time_zone: &jiff::tz::TimeZone,
) -> Vec<ResultItem> {
    CLIPBOARD_FOLDS.with(|cache| {
        assemble_clipboard_cached(&mut cache.borrow_mut(), query, entries, l10n, time_zone)
    })
}

/// [`assemble_clipboard`] with an explicit cache.
pub fn assemble_clipboard_cached(
    cache: &mut ClipboardFoldCache,
    query: &str,
    entries: &[ClipboardEntry],
    l10n: &Localizer,
    time_zone: &jiff::tz::TimeZone,
) -> Vec<ResultItem> {
    let needle = fold(query.trim());
    cache.retain_present(entries);
    let mut matching: Vec<&ClipboardEntry> = entries
        .iter()
        .filter(|e| needle.is_empty() || cache.folded(e, l10n).contains(&needle))
        .collect();
    // Stable: keeps the store's newest-first order inside each group.
    matching.sort_by_key(|e| !e.is_pinned);
    matching
        .into_iter()
        .map(|entry| clipboard_item(entry, l10n, time_zone))
        .collect()
}

/// One clipboard row.
pub fn clipboard_item(
    entry: &ClipboardEntry,
    l10n: &Localizer,
    time_zone: &jiff::tz::TimeZone,
) -> ResultItem {
    let preview = entry.preview(l10n);
    ResultItem {
        id: format!("clipboard:{}", entry.id),
        target: Target::Clipboard {
            entry_id: entry.id.clone(),
            pinned: entry.is_pinned,
        },
        title: if entry.is_pinned {
            format!("📌 {preview}")
        } else {
            preview
        },
        subtitle: format_time(entry.time(), time_zone),
        icon: if let Some(key) = thumbnail_key(entry) {
            IconSpec::ClipboardImage(key)
        } else {
            IconSpec::Symbolic("edit-paste-symbolic")
        },
        action: l10n.tr("Paste"),
    }
}

/// Identity of an entry's thumbnail: entry id plus the attachment file name (a fresh UUID for
/// every saved attachment). An id that is reused for a new image (the same text copied again
/// with a different picture) therefore never shows the old cached thumbnail.
pub fn thumbnail_key(entry: &ClipboardEntry) -> Option<String> {
    let attachment = entry.image_attachment.as_ref()?;
    Some(format!("{}/{}", entry.id, attachment.file_name))
}

/// Entry id and attachment file name of a [`thumbnail_key`].
pub fn split_thumbnail_key(key: &str) -> Option<(&str, &str)> {
    key.split_once('/')
}

/// `2026-10-13 14:05` in `time_zone`.
pub fn format_time(time: SystemTime, time_zone: &jiff::tz::TimeZone) -> String {
    jiff::Timestamp::try_from(time)
        .map(|timestamp| {
            timestamp
                .to_zoned(time_zone.clone())
                .strftime("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
