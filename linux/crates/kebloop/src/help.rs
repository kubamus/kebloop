//! Help: searchable command list, current shortcuts, what is available in this session, the
//! shortcut-hints switch and a button to replay the welcome walkthrough. Used as a page of the
//! settings window and as a window of its own (the "Help" command).

use std::rc::Rc;

use adw::prelude::*;
use kebloop_core::search::normalize;
use kebloop_platform::compositor::CompositorKind;
use kebloop_platform::paste::PasteTool;
use kebloop_platform::system_actions::{Availability as SystemAvailability, SystemAction};
use kebloop_services::commands::LauncherCommand;
use kebloop_services::config::PortableSettings;
use kebloop_services::l10n::{FormatArg, Localizer};

use crate::context::{AppContext, ClipboardStatus, ListenerId, PortalState};
use crate::i18n::{tr, trf};
use crate::platform::{CommandAvailability, system_action_of, window_action_of};
use crate::settings::logic::accelerator_label;
use crate::settings::widgets::{
    DynGroup, SwitchControl, action_row, button_row, group, note, placeholder_row, row_button,
};

/// One command in the help list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpEntry {
    /// Command id (`emoji`, `link:<id>`, ...), what "Open" runs.
    pub id: String,
    /// Title.
    pub title: String,
    /// What it does.
    pub description: String,
    /// What to type.
    pub example: String,
    /// Extra state ("3 saved quicklinks.").
    pub availability: Option<String>,
    /// Assigned shortcut label.
    pub shortcut: Option<String>,
    /// Symbolic icon.
    pub icon: &'static str,
    /// Words the search also matches.
    pub aliases: String,
}

impl HelpEntry {
    fn haystack(&self) -> String {
        normalize(&format!(
            "{} {} {} {} {} {}",
            self.title,
            self.description,
            self.example,
            self.id,
            self.availability.as_deref().unwrap_or(""),
            self.aliases
        ))
    }
}

fn description(command: LauncherCommand) -> String {
    use LauncherCommand as C;
    match command {
        C::Snippets => tr("Fill placeholders, preview, and copy reusable text."),
        C::Scripts => tr("Run reviewed local scripts with bounded output and cancellation."),
        C::Extensions => tr("Browse results and actions from enabled local extensions."),
        C::Clipboard => tr(
            "Browse and paste saved clipboard items. Ctrl+Return copies; Ctrl+Shift+Return pastes plain text.",
        ),
        C::Quicklinks => tr(
            "Open saved websites, files, and folders. Use a keyword with text to pass it as an argument.",
        ),
        C::Emoji => tr("Search emoji by name or category. Return pastes; Ctrl+Return copies."),
        C::Settings => {
            tr("Open Kebloop settings, including search, appearance, shortcuts, and profiles.")
        }
        C::Shutdown => tr("Ask for confirmation, then power off the computer."),
        C::Browse => tr("Search the available built-in commands and quicklinks."),
        C::Help => tr(
            "Read command examples and see which features and shortcuts are currently available.",
        ),
        C::Onboarding => tr(
            "Review the first-run choices for login startup, clipboard history, and the launcher shortcut.",
        ),
        C::Profiles => tr("Switch between saved work and personal settings profiles."),
        C::WebSearch => {
            tr("Send your query to a configured web search template after you choose the result.")
        }
        C::AppleShortcuts => tr("Browse the Apple Shortcuts you selected in Settings."),
        C::CalculationHistory => tr("Browse, copy, and clear calculations you accepted."),
        other if system_action_of(other).is_some() => {
            tr("Control your computer. Restart and log out ask for confirmation first.")
        }
        other if window_action_of(other).is_some() => {
            tr("Arrange the window that had focus before Kebloop opened.")
        }
        _ => String::new(),
    }
}

fn example(command: LauncherCommand) -> String {
    use LauncherCommand as C;
    match command {
        C::Snippets => tr("snippets"),
        C::Scripts => tr("scripts"),
        C::Extensions => tr("extensions"),
        C::Clipboard => "clipboard".into(),
        C::Quicklinks => tr("docs or gh kebloop"),
        C::Emoji => tr("emoji smile"),
        C::Settings => "settings".into(),
        C::Shutdown => "shutdown".into(),
        C::Browse => "commands".into(),
        C::Help => "help".into(),
        C::Onboarding => "welcome".into(),
        C::Profiles => "profiles".into(),
        C::WebSearch => tr("web Warsaw cafés"),
        C::AppleShortcuts => "shortcuts".into(),
        C::CalculationHistory => "calculator history".into(),
        other => other.aliases().first().copied().unwrap_or_default().into(),
    }
}

/// Every command, saved quicklink, snippet-independent entry, in display order.
pub fn entries(
    settings: &PortableSettings,
    clipboard_enabled: bool,
    localizer: &Localizer,
    commands: &CommandAvailability,
) -> Vec<HelpEntry> {
    let shortcut_of = |id: &str| {
        settings
            .command_shortcuts
            .iter()
            .find(|b| b.id == id)
            .map(|b| {
                b.resolved()
                    .map_or_else(|| b.label.clone(), |a| accelerator_label(&a))
            })
    };
    let count = |n: usize| FormatArg::Int(i64::try_from(n).unwrap_or(0));
    let mut out: Vec<HelpEntry> = commands
        .commands()
        .map(|command| {
            let availability = match command {
                LauncherCommand::Clipboard => Some(tr(if clipboard_enabled {
                    "Clipboard recording is on; saved text stays on this computer."
                } else {
                    "Clipboard recording is off."
                })),
                LauncherCommand::Quicklinks => Some(trf(
                    "%d saved quicklinks.",
                    &[count(settings.quicklinks.len())],
                )),
                LauncherCommand::WebSearch => Some(trf(
                    "%d web search templates configured.",
                    &[count(settings.web_searches.len())],
                )),
                LauncherCommand::Snippets => {
                    Some(trf("%d snippets saved.", &[count(settings.snippets.len())]))
                }
                _ => None,
            };
            HelpEntry {
                id: command.id().to_owned(),
                title: command.title(localizer),
                description: description(command),
                example: example(command),
                availability,
                shortcut: shortcut_of(command.id()),
                icon: command.icon_name(),
                aliases: command.aliases().join(" "),
            }
        })
        .collect();
    for link in &settings.quicklinks {
        let id = format!("link:{}", link.id);
        out.push(HelpEntry {
            shortcut: shortcut_of(&id),
            id,
            title: link.name.clone(),
            description: if link.keyword.is_empty() {
                tr("Open the saved link by its name.")
            } else {
                tr("Type its keyword and an optional search argument.")
            },
            example: if link.keyword.is_empty() {
                link.name.clone()
            } else {
                format!("{} kebloop", link.keyword)
            },
            availability: (!link.keyword.is_empty())
                .then(|| trf("Keyword: %@", &[FormatArg::Str(link.keyword.clone())])),
            icon: "insert-link-symbolic",
            aliases: link.target.clone(),
        });
    }
    out
}

/// Entries matching `query` (every word must occur in the title, text, example, id or shortcut).
pub fn filter_entries<'a>(entries: &'a [HelpEntry], query: &str) -> Vec<&'a HelpEntry> {
    let wanted = normalize(query);
    entries
        .iter()
        .filter(|entry| {
            wanted.is_empty() || {
                let haystack = format!(
                    "{} {}",
                    entry.haystack(),
                    normalize(entry.shortcut.as_deref().unwrap_or(""))
                );
                wanted.split(' ').all(|word| haystack.contains(word))
            }
        })
        .collect()
}

/// One line of the "this session" panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Availability {
    /// Feature name.
    pub feature: String,
    /// State or explanation.
    pub state: String,
    /// Whether the feature works.
    pub ok: bool,
}

/// What the "This session" panel is built from.
pub struct SessionFacts<'a> {
    /// State of the clipboard backend.
    pub clipboard: &'a ClipboardStatus,
    /// State of the global-shortcut portal.
    pub portal: &'a PortalState,
    /// The launcher is a layer-shell panel.
    pub layer_shell: bool,
    /// The installed paste tool.
    pub paste_tool: Option<PasteTool>,
    /// Direct paste is switched on.
    pub direct_paste: bool,
    /// logind lets this session power off.
    pub shutdown: bool,
    /// The compositor window commands work with.
    pub window_kind: Option<CompositorKind>,
    /// The system-action probe.
    pub system: &'a [(SystemAction, SystemAvailability)],
}

/// What works in this session.
pub fn availability(facts: &SessionFacts<'_>) -> Vec<Availability> {
    let line = |feature: &str, state: String, ok: bool| Availability {
        feature: tr(feature),
        state,
        ok,
    };
    let mut lines = vec![
        match facts.clipboard {
            ClipboardStatus::Active => line(
                "Clipboard history",
                tr("Recording clipboard changes."),
                true,
            ),
            ClipboardStatus::Starting => line("Clipboard history", tr("Starting…"), false),
            ClipboardStatus::HistoryDisabled(_) => line(
                "Clipboard history",
                tr("Clipboard history is turned off in this session."),
                false,
            ),
            ClipboardStatus::Unavailable(_) => line(
                "Clipboard history",
                tr("Clipboard history is unavailable on this compositor."),
                false,
            ),
        },
        match facts.portal {
            PortalState::Active(_) => line(
                "Global shortcuts",
                tr("Registered through the desktop portal."),
                true,
            ),
            PortalState::Checking => line(
                "Global shortcuts",
                tr("Checking for global shortcut support…"),
                false,
            ),
            PortalState::Unavailable(_) => line(
                "Global shortcuts",
                tr(
                    "No portal: bind key combinations in your compositor (see Settings → Shortcuts).",
                ),
                false,
            ),
        },
        line(
            "Launcher panel",
            if facts.layer_shell {
                tr("Floating layer-shell panel.")
            } else {
                tr("Regular window (this desktop has no layer-shell).")
            },
            facts.layer_shell,
        ),
        match (facts.paste_tool, facts.direct_paste) {
            (Some(tool), true) => line(
                "Direct paste",
                trf("Detected: %@", &[FormatArg::Str(tool.program().to_owned())]),
                true,
            ),
            (Some(tool), false) => line(
                "Direct paste",
                trf(
                    "Detected: %@, turned off in Settings.",
                    &[FormatArg::Str(tool.program().to_owned())],
                ),
                false,
            ),
            (None, _) => line(
                "Direct paste",
                tr("Install wtype or ydotool to paste directly."),
                false,
            ),
        },
        match facts.window_kind {
            Some(kind) => line(
                "Window management",
                trf(
                    "Window commands use %@.",
                    &[FormatArg::Str(kind.name().to_owned())],
                ),
                true,
            ),
            None => line(
                "Window management",
                tr("Needs niri, Hyprland, Sway or i3."),
                false,
            ),
        },
    ];
    let unavailable: Vec<_> = facts
        .system
        .iter()
        .filter_map(|(action, availability)| match availability {
            SystemAvailability::Unavailable(reason) => Some((*action, reason.clone())),
            SystemAvailability::Available => None,
        })
        .collect();
    if facts.system.is_empty() {
        lines.push(line("System commands", tr("Checking…"), false));
    } else if unavailable.is_empty() {
        lines.push(line("System commands", tr("Available."), true));
    } else {
        for (action, reason) in unavailable {
            let title = LauncherCommand::from_id(action.id())
                .map_or_else(|| action.id().to_owned(), |c| c.title_key().to_owned());
            lines.push(line(&title, reason, false));
        }
    }
    lines.push(line(
        "Shut Down…",
        if facts.shutdown {
            tr("Available.")
        } else {
            tr("Not permitted for this session.")
        },
        facts.shutdown,
    ));
    lines
}

fn settings_direct_paste(settings: &PortableSettings, tool_installed: bool) -> bool {
    crate::direct_paste::enabled(&settings.extra, tool_installed)
}

/// The help content widget. Follows application changes until it is destroyed.
#[derive(Clone)]
pub struct HelpView {
    root: gtk::Widget,
}

impl HelpView {
    /// Build the view.
    pub fn new(ctx: Rc<AppContext>) -> Self {
        let page = adw::PreferencesPage::new();
        page.set_title(&tr("Help"));

        let top = group("", "");
        let search = gtk::SearchEntry::builder()
            .placeholder_text(tr("Search commands and help"))
            .build();
        search.update_property(&[gtk::accessible::Property::Label(&tr(
            "Search commands and help",
        ))]);
        top.add(&search);
        let intro = note("");
        top.add(&intro);
        let hints = Rc::new({
            let ctx = ctx.clone();
            let env_ctx = ctx.clone();
            SwitchControl::with_ctx(
                &tr("Show shortcut hints in the launcher"),
                env_ctx.with_settings(|s| s.personalization.show_shortcut_hints),
                move |on| ctx.edit_settings(|s| s.personalization.show_shortcut_hints = on),
            )
        });
        top.add(&hints.row);
        page.add(&top);

        let notices = Rc::new(DynGroup::new(group(&tr("Startup notices"), "")));
        page.add(&notices.group);
        let session = Rc::new(DynGroup::new(group(&tr("This session"), "")));
        page.add(&session.group);
        let commands = Rc::new(DynGroup::new(group(&tr("Built-in commands"), "")));
        page.add(&commands.group);
        let shortcuts = Rc::new(DynGroup::new(group(&tr("Current shortcuts"), "")));
        page.add(&shortcuts.group);
        let replay = group("", "");
        {
            let ctx = ctx.clone();
            replay.add(&button_row(
                &tr("Replay Welcome"),
                "starred-symbolic",
                move || crate::onboarding::open_onboarding(&ctx.app, &ctx),
            ));
        }
        replay.add(&note(&tr(
            "Help works offline and updates as you add links and shortcuts.",
        )));
        page.add(&replay);

        let refresh: Rc<dyn Fn()> = {
            let ctx = ctx.clone();
            let search = search.clone();
            Rc::new(move || {
                let settings = ctx.settings();
                hints.sync(settings.personalization.show_shortcut_hints);
                intro.set_label(&trf(
                    "Open the launcher with %@, then type a command or search for an app, file, or link.",
                    &[FormatArg::Str(accelerator_label(&settings.launcher_accelerator()))],
                ));
                let startup: Vec<String> = ctx.notices.borrow().clone();
                notices.group.set_visible(!startup.is_empty());
                notices.update(&format!("{startup:?}"), || {
                    startup
                        .iter()
                        .map(|text| {
                            let row = action_row(text, "");
                            row.set_title_lines(0);
                            row.add_prefix(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
                            row.upcast()
                        })
                        .collect()
                });
                let offered = ctx.availability.borrow().clone();
                let facts = availability(&SessionFacts {
                    clipboard: &ctx.clipboard_status.borrow(),
                    portal: &ctx.portal.borrow(),
                    layer_shell: crate::launcher::layer_shell_supported(),
                    paste_tool: ctx.paste_tool,
                    direct_paste: settings_direct_paste(&settings, ctx.paste_tool.is_some()),
                    shutdown: offered.shutdown(),
                    window_kind: offered.window_kind(),
                    system: &ctx.system_probe.borrow(),
                });
                let signature = format!("{facts:?}|{}", ctx.session.compositor.name());
                session.update(&signature, || {
                    let mut rows: Vec<gtk::Widget> = facts
                        .iter()
                        .map(|fact| {
                            let row = action_row(&fact.feature, &fact.state);
                            row.set_subtitle_lines(0);
                            row.add_prefix(&gtk::Image::from_icon_name(if fact.ok {
                                "object-select-symbolic"
                            } else {
                                "dialog-warning-symbolic"
                            }));
                            row.upcast()
                        })
                        .collect();
                    rows.push(action_row(&tr("Desktop"), ctx.session.compositor.name()).upcast());
                    rows
                });
                let all = entries(
                    &settings,
                    ctx.clipboard.borrow().is_enabled(),
                    &ctx.l10n,
                    &offered,
                );
                let query = search.text().to_string();
                let shown = filter_entries(&all, &query);
                let signature = format!("{query}|{shown:?}");
                commands.update(&signature, || {
                    if shown.is_empty() {
                        return vec![placeholder_row(&tr("No results"), "")];
                    }
                    shown.iter().map(|entry| entry_row(&ctx, entry)).collect()
                });
                let mut lines: Vec<(String, String)> = vec![(
                    tr("Open launcher"),
                    accelerator_label(&settings.launcher_accelerator()),
                )];
                for binding in &settings.app_shortcuts {
                    lines.push((
                        binding.app_name.clone(),
                        binding
                            .resolved()
                            .map_or_else(|| binding.label.clone(), |a| accelerator_label(&a)),
                    ));
                }
                for binding in &settings.command_shortcuts {
                    let title = LauncherCommand::from_id(&binding.id)
                        .map_or_else(|| binding.id.clone(), |c| c.title(&ctx.l10n));
                    lines.push((
                        title,
                        binding
                            .resolved()
                            .map_or_else(|| binding.label.clone(), |a| accelerator_label(&a)),
                    ));
                }
                let wanted = normalize(&query);
                lines.retain(|(name, key)| {
                    wanted.is_empty() || normalize(&format!("{name} {key}")).contains(&wanted)
                });
                shortcuts.update(&format!("{lines:?}"), || {
                    lines
                        .iter()
                        .map(|(name, key)| {
                            let row = action_row(name, "");
                            let label = gtk::Label::builder()
                                .label(key)
                                .css_classes(["monospace", "dim-label"])
                                .build();
                            row.add_suffix(&label);
                            row.upcast()
                        })
                        .collect()
                });
            })
        };
        {
            let refresh = refresh.clone();
            search.connect_search_changed(move |_| refresh());
        }
        refresh();
        let listener: ListenerId = {
            let refresh = refresh.clone();
            ctx.subscribe(Rc::new(move |_| refresh()))
        };
        let unsubscribe_from = Rc::downgrade(&ctx);
        page.connect_destroy(move |_| {
            if let Some(ctx) = unsubscribe_from.upgrade() {
                ctx.unsubscribe(listener);
            }
        });
        Self {
            root: page.upcast(),
        }
    }

    /// The widget.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone()
    }
}

fn entry_row(ctx: &Rc<AppContext>, entry: &HelpEntry) -> gtk::Widget {
    let mut detail = format!("{}\n{} {}", entry.description, tr("Try:"), entry.example);
    if let Some(availability) = &entry.availability {
        detail.push('\n');
        detail.push_str(availability);
    }
    let row = action_row(&entry.title, &detail);
    row.set_subtitle_lines(0);
    row.add_prefix(&gtk::Image::from_icon_name(entry.icon));
    if let Some(shortcut) = &entry.shortcut {
        row.add_suffix(
            &gtk::Label::builder()
                .label(shortcut)
                .css_classes(["monospace", "dim-label"])
                .build(),
        );
    }
    let open = row_button(&tr("Open"));
    open.update_property(&[gtk::accessible::Property::Label(&trf(
        "Open command %@",
        &[FormatArg::Str(entry.title.clone())],
    ))]);
    {
        let (ctx, id) = (ctx.clone(), entry.id.clone());
        open.connect_clicked(move |_| {
            if let Some(launcher) = ctx.launcher() {
                launcher.execute_command(&id);
            }
        });
    }
    row.add_suffix(&open);
    row.upcast()
}

/// Show the help window.
pub fn open_help(app: &adw::Application, ctx: &Rc<AppContext>) {
    if let Some(existing) = ctx.help_window.borrow().as_ref() {
        existing.present();
        return;
    }
    if let Some(launcher) = ctx.launcher() {
        launcher.hide();
    }
    let view = HelpView::new(ctx.clone());
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&view.widget()));
    let window = adw::Window::builder()
        .application(app)
        .title(tr("Help"))
        .default_width(720)
        .default_height(680)
        .content(&toolbar)
        .build();
    *ctx.help_window.borrow_mut() = Some(window.clone().upcast());
    let closing = ctx.clone();
    window.connect_close_request(move |_| {
        *closing.help_window.borrow_mut() = None;
        gtk::glib::Propagation::Proceed
    });
    window.present();
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebloop_services::quicklinks::Quicklink;

    fn settings_with_link() -> PortableSettings {
        let mut settings = PortableSettings::default();
        settings.quicklinks.push(Quicklink {
            id: "q1".into(),
            name: "Docs".into(),
            target: "https://example.com/?q={query}".into(),
            keyword: "docs".into(),
        });
        settings
    }

    #[test]
    fn entries_list_available_commands_and_quicklinks() {
        let list = entries(
            &settings_with_link(),
            true,
            &Localizer::english(),
            &CommandAvailability::new(true, [SystemAction::Lock], Some(CompositorKind::Niri)),
        );
        let ids: Vec<&str> = list.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"emoji") && ids.contains(&"link:q1"));
        assert!(!ids.contains(&"appleShortcuts"));
        assert!(ids.contains(&"calculationHistory") && ids.contains(&"lock"));
        assert!(ids.contains(&"leftHalf") && !ids.contains(&"sleep"));
        let lock = list.iter().find(|e| e.id == "lock").unwrap();
        assert!(!lock.description.is_empty());
        let docs = list.iter().find(|e| e.id == "link:q1").unwrap();
        assert_eq!(docs.example, "docs kebloop");
        assert_eq!(docs.availability.as_deref(), Some("Keyword: docs"));
    }

    #[test]
    fn search_matches_titles_aliases_and_shortcuts() {
        let mut settings = settings_with_link();
        kebloop_services::shortcuts::assign_command_shortcut(
            &mut settings,
            "emoji",
            "<Control><Alt>e",
            "Ctrl+Alt+E",
            &|_| true,
        )
        .unwrap();
        let list = entries(
            &settings,
            false,
            &Localizer::english(),
            &CommandAvailability::default(),
        );
        let found = |q: &str| -> Vec<&str> {
            filter_entries(&list, q)
                .into_iter()
                .map(|e| e.id.as_str())
                .collect()
        };
        assert_eq!(found("schowek"), ["clipboard"]);
        assert!(found("ctrl alt e").contains(&"emoji"));
        assert_eq!(found("zzzz"), Vec::<&str>::new());
        assert_eq!(found("").len(), list.len());
    }

    #[test]
    fn availability_reports_each_backend() {
        let system = [
            (SystemAction::Lock, SystemAvailability::Available),
            (
                SystemAction::ToggleMute,
                SystemAvailability::Unavailable("no wpctl".into()),
            ),
        ];
        let facts = availability(&SessionFacts {
            clipboard: &ClipboardStatus::Unavailable("no data-control".into()),
            portal: &PortalState::Unavailable("none".into()),
            layer_shell: true,
            paste_tool: Some(PasteTool::Wtype),
            direct_paste: true,
            shutdown: false,
            window_kind: Some(CompositorKind::Niri),
            system: &system,
        });
        let ok: Vec<bool> = facts.iter().map(|f| f.ok).collect();
        // clipboard, portal, panel, direct paste, windows, one unavailable system command, power
        assert_eq!(ok, [false, false, true, true, true, false, false]);
        assert!(facts[3].state.contains("wtype"));
        assert_eq!(facts[5].state, "no wpctl");
        let off = availability(&SessionFacts {
            direct_paste: false,
            system: &[],
            window_kind: None,
            ..SessionFacts {
                clipboard: &ClipboardStatus::Active,
                portal: &PortalState::Checking,
                layer_shell: false,
                paste_tool: Some(PasteTool::Ydotool),
                direct_paste: true,
                shutdown: true,
                window_kind: None,
                system: &[],
            }
        });
        assert!(!off[3].ok && off[3].state.contains("turned off"));
        assert!(!off[4].ok);
    }
}
