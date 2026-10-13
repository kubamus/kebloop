//! Shortcuts: the launcher shortcut, application and command shortcuts with a key recorder,
//! and, when the desktop has no GlobalShortcuts portal, ready-to-paste compositor bindings.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use kebloop_platform::session::Compositor;
use kebloop_platform::shortcuts::{Snippet, SnippetTarget, snippet_for};
use kebloop_services::config::PortableSettings;
use kebloop_services::shortcuts::{
    assign_app_shortcut, assign_command_shortcut, assign_launcher_shortcut, remove_app_shortcut,
    remove_command_shortcut,
};

use super::logic::{self, PageId, accelerator_label, command_targets, filter_apps};
use super::recorder::record_shortcut;
use super::widgets::{
    ComboControl, DynGroup, Env, Page, action_row, copy_button, group, icon_button, mono_label,
    note, placeholder_row, preferences_page, row_button,
};
use crate::context::{AppContext, PortalState};
use crate::i18n::{tr, trf};
use kebloop_services::l10n::FormatArg;

/// How many applications are listed at once.
const APP_ROWS: usize = 40;

/// Compositor config lines for every shortcut that has an accelerator: one `bind` per
/// shortcut running the matching `kebloop` command.
pub fn compositor_lines(
    target: SnippetTarget,
    settings: &PortableSettings,
    program: &str,
) -> Vec<Snippet> {
    let mut lines = Vec::new();
    let mut push = |accelerator: String, command: String| {
        if let Ok(snippet) = snippet_for(target, &accelerator, &command) {
            lines.push(snippet);
        }
    };
    push(
        settings.launcher_accelerator().to_string(),
        logic::toggle_command(program),
    );
    for binding in &settings.app_shortcuts {
        if let Some(accelerator) = binding.resolved() {
            push(
                accelerator.to_string(),
                logic::launch_command(program, &binding.app_path),
            );
        }
    }
    for binding in &settings.command_shortcuts {
        if let Some(accelerator) = binding.resolved() {
            push(
                accelerator.to_string(),
                logic::command_command(program, &binding.id),
            );
        }
    }
    lines
}

/// Compositor config lines that run the window commands in `commands`: the recorded shortcut of
/// each, or the suggested one when none is recorded, so the lines can be pasted as they are.
pub fn window_lines(
    target: SnippetTarget,
    settings: &PortableSettings,
    commands: &[kebloop_services::commands::LauncherCommand],
    program: &str,
) -> Vec<Snippet> {
    let suggestions = kebloop_services::window_management::suggested_shortcuts();
    commands
        .iter()
        .filter_map(|command| {
            let accelerator = settings
                .command_shortcuts
                .iter()
                .find(|binding| binding.id == command.id())
                .and_then(|binding| binding.resolved())
                .map(|accelerator| accelerator.to_string())
                .or_else(|| {
                    suggestions
                        .iter()
                        .find(|suggestion| suggestion.command_id == command.id())
                        .map(|suggestion| suggestion.accelerator.clone())
                })?;
            snippet_for(
                target,
                &accelerator,
                &logic::command_command(program, command.id()),
            )
            .ok()
        })
        .collect()
}

/// The compositor to preselect for `compositor`.
pub fn default_target(compositor: &Compositor) -> usize {
    match compositor {
        Compositor::Niri => 0,
        Compositor::Hyprland => 1,
        Compositor::Sway => 2,
        Compositor::Other(name) if name.eq_ignore_ascii_case("i3") => 3,
        _ => 0,
    }
}

fn portal_unavailable(ctx: &AppContext) -> bool {
    matches!(*ctx.portal.borrow(), PortalState::Unavailable(_))
}

fn status_text(ctx: &AppContext) -> (String, String) {
    match &*ctx.portal.borrow() {
        PortalState::Checking => (tr("Checking for global shortcut support…"), String::new()),
        PortalState::Active(bound) => {
            let list = bound
                .iter()
                .map(|(id, trigger)| format!("{id}: {trigger}"))
                .collect::<Vec<_>>()
                .join(", ");
            (
                tr("Registered through the desktop portal."),
                if list.is_empty() {
                    tr("Your desktop may ask you to confirm or change the keys.")
                } else {
                    list
                },
            )
        }
        PortalState::Unavailable(reason) => (
            trf(
                "No global shortcuts portal on %@. Bind the commands below in your compositor.",
                &[FormatArg::Str(ctx.session.compositor.name().to_owned())],
            ),
            reason.clone(),
        ),
    }
}

/// Build the page.
pub fn build(env: &Rc<Env>) -> Page {
    let ctx = env.ctx.clone();
    let page = preferences_page();
    page.set_title(&PageId::Shortcuts.title());
    page.set_description(&PageId::Shortcuts.subtitle());
    let program = logic::kebloop_program(
        &kebloop_platform::paths::Env::from_process().path_dirs(),
        std::env::current_exe().ok().as_deref(),
    );

    // Status.
    let status_group = group(&tr("Global shortcuts"), "");
    let status_row = action_row("", "");
    status_row.add_prefix(&gtk::Image::from_icon_name("input-keyboard-symbolic"));
    status_row.set_title_lines(0);
    status_row.set_subtitle_lines(0);
    status_group.add(&status_row);
    page.add(&status_group);

    // Launcher.
    let launcher_group = group(&tr("Launcher"), "");
    let launcher_row = action_row(&tr("Open launcher"), "");
    let launcher_value = gtk::Label::builder()
        .css_classes(["dim-label"])
        .valign(gtk::Align::Center)
        .build();
    launcher_row.add_suffix(&launcher_value);
    let record = row_button(&tr("Record Custom Shortcut…"));
    {
        let env = env.clone();
        record.connect_clicked(move |_| {
            let ctx = env.ctx.clone();
            record_shortcut(
                &env,
                &tr("Press a shortcut for the launcher…"),
                move |accel, label| {
                    ctx.update_settings(|s| assign_launcher_shortcut(s, accel, label))
                        .map_err(|e| tr(&e.to_string()))
                },
            );
        });
    }
    launcher_row.add_suffix(&record);
    launcher_group.add(&launcher_row);
    launcher_group.add(&note(&tr(
        "Super+Space may conflict with a shortcut of your desktop.",
    )));
    page.add(&launcher_group);

    // Application shortcuts.
    let apps_group = group(
        &tr("App Shortcuts"),
        &tr("Open an app from anywhere with Control, Alt, or Super plus a key."),
    );
    let app_search = gtk::SearchEntry::builder()
        .placeholder_text(tr("Search applications"))
        .margin_bottom(6)
        .build();
    app_search.update_property(&[gtk::accessible::Property::Label(&tr(
        "Search applications for shortcuts",
    ))]);
    apps_group.add(&app_search);
    let apps = Rc::new(DynGroup::new(apps_group));
    page.add(&apps.group);

    // Command shortcuts.
    let commands = Rc::new(DynGroup::new(group(
        &tr("Command Shortcuts"),
        &tr("Assign global shortcuts to commands and saved actions."),
    )));
    page.add(&commands.group);

    // Compositor bindings.
    let compositor_group = group(
        &tr("Compositor key bindings"),
        &tr("Paste these lines into your compositor configuration to bind the shortcuts above."),
    );
    let targets: Vec<String> = SnippetTarget::ALL
        .iter()
        .map(|t| t.name().to_owned())
        .collect();
    let selected = Rc::new(Cell::new(default_target(&ctx.session.compositor)));
    let snippet_label = mono_label("");
    let hint_label = note("");
    let render: Rc<dyn Fn()> = {
        let ctx = ctx.clone();
        let selected = selected.clone();
        let snippet_label = snippet_label.clone();
        let hint_label = hint_label.clone();
        let program = program.clone();
        Rc::new(move || {
            let target = SnippetTarget::ALL[selected.get().min(SnippetTarget::ALL.len() - 1)];
            let lines = ctx.with_settings(|s| compositor_lines(target, s, &program));
            snippet_label.set_label(
                &lines
                    .iter()
                    .map(|l| l.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            hint_label.set_label(&trf(
                "Add to %@",
                &[FormatArg::Str(target.config_hint().to_owned())],
            ));
        })
    };
    let target_combo = {
        let selected = selected.clone();
        let render = render.clone();
        Rc::new(ComboControl::new(
            env,
            &tr("Compositor"),
            "",
            &targets,
            u32::try_from(selected.get()).unwrap_or(0),
            move |index| {
                selected.set(index as usize);
                render();
                Ok(())
            },
        ))
    };
    compositor_group.add(&target_combo.row);
    compositor_group.add(&hint_label);
    let snippet_frame = gtk::Box::builder()
        .css_classes(["card"])
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    snippet_label.set_margin_top(10);
    snippet_label.set_margin_bottom(10);
    snippet_label.set_margin_start(12);
    snippet_label.set_hexpand(true);
    snippet_frame.append(&snippet_label);
    let copy_all = copy_button(env, {
        let snippet_label = snippet_label.clone();
        move || snippet_label.label().to_string()
    });
    copy_all.set_valign(gtk::Align::Start);
    copy_all.set_margin_top(4);
    copy_all.set_margin_end(4);
    snippet_frame.append(&copy_all);
    compositor_group.add(&snippet_frame);
    page.add(&compositor_group);

    // Commands reference (always useful: custom setups, other compositors).
    let reference = group(
        &tr("Commands"),
        &tr("These commands work from any key binding or script."),
    );
    for (title, command) in [
        (tr("Toggle the launcher"), logic::toggle_command(&program)),
        (
            tr("Run a command"),
            logic::command_command(&program, "<id>"),
        ),
        (
            tr("Launch an application"),
            logic::launch_command(&program, "<desktop-id>"),
        ),
    ] {
        let row = action_row(&title, "");
        let label = mono_label(&command);
        label.set_valign(gtk::Align::Center);
        row.add_suffix(&label);
        row.add_suffix(&copy_button(env, move || command.clone()));
        reference.add(&row);
    }
    page.add(&reference);

    let refresh: Rc<dyn Fn()> = {
        let env = env.clone();
        let apps = apps.clone();
        let commands = commands.clone();
        let app_search = app_search.clone();
        let status_row = status_row.clone();
        let launcher_value = launcher_value.clone();
        let compositor_group = compositor_group.clone();
        let render = render.clone();
        Rc::new(move || {
            let ctx = &env.ctx;
            let (title, subtitle) = status_text(ctx);
            status_row.set_title(&gtk::glib::markup_escape_text(&title));
            status_row.set_subtitle(&gtk::glib::markup_escape_text(&subtitle));
            status_row.set_visible(true);
            compositor_group.set_visible(portal_unavailable(ctx));
            launcher_value
                .set_label(&ctx.with_settings(|s| accelerator_label(&s.launcher_accelerator())));
            render();
            refresh_apps(&env, &apps, &app_search.text());
            refresh_commands(&env, &commands);
        })
    };
    {
        let refresh = refresh.clone();
        app_search.connect_search_changed(move |_| refresh());
    }
    refresh();
    Page::new(&page, move || refresh())
}

fn refresh_apps(env: &Rc<Env>, apps: &DynGroup, query: &str) {
    let ctx = &env.ctx;
    let installed = ctx.apps.borrow();
    let (bindings, mut unbound): (Vec<_>, Vec<_>) = {
        let settings = ctx.settings();
        let wanted = kebloop_core::search::normalize(query);
        let bound: Vec<_> = settings
            .app_shortcuts
            .iter()
            .filter(|b| {
                wanted.is_empty() || kebloop_core::search::normalize(&b.app_name).contains(&wanted)
            })
            .cloned()
            .collect();
        let (shown, _) = filter_apps(&installed, query, installed.len());
        let rest: Vec<_> = shown
            .into_iter()
            .filter(|app| !settings.app_shortcuts.iter().any(|b| b.app_path == app.id))
            .cloned()
            .collect();
        (bound, rest)
    };
    let hidden = unbound.len().saturating_sub(APP_ROWS);
    unbound.truncate(APP_ROWS);
    let signature = format!(
        "{query}|{}|{}|{hidden}",
        bindings
            .iter()
            .map(|b| format!("{}={}", b.app_path, b.label))
            .collect::<Vec<_>>()
            .join(","),
        unbound
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>()
            .join(",")
    );
    apps.update(&signature, || {
        let mut rows: Vec<gtk::Widget> = Vec::new();
        for binding in &bindings {
            let app = installed.iter().find(|a| a.id == binding.app_path);
            let row = app_row(
                env,
                &binding.app_path,
                &binding.app_name,
                app.and_then(|a| a.icon.clone()),
                Some(
                    binding
                        .resolved()
                        .map_or_else(|| binding.label.clone(), |a| accelerator_label(&a)),
                ),
                binding.resolved().is_none(),
            );
            rows.push(row.upcast());
        }
        for app in &unbound {
            rows.push(app_row(env, &app.id, &app.name, app.icon.clone(), None, false).upcast());
        }
        if rows.is_empty() {
            rows.push(placeholder_row(
                &tr("No matching applications"),
                &tr("Try a different search or refresh the application index in General settings."),
            ));
        }
        if hidden > 0 {
            rows.push(placeholder_row(
                &trf(
                    "%d more applications. Refine the search to see them.",
                    &[FormatArg::Int(i64::try_from(hidden).unwrap_or(0))],
                ),
                "",
            ));
        }
        rows
    });
}

fn app_row(
    env: &Rc<Env>,
    id: &str,
    name: &str,
    icon: Option<String>,
    label: Option<String>,
    unavailable: bool,
) -> adw::ActionRow {
    let row = action_row(name, "");
    let image = gtk::Image::new();
    crate::icons::apply(&image, &crate::launcher::model::IconSpec::App(icon), 24);
    row.add_prefix(&image);
    if unavailable {
        row.set_subtitle(&tr(
            "This shortcut has no Linux equivalent. Record a new one.",
        ));
    }
    let button = row_button(&label.clone().unwrap_or_else(|| tr("Record Shortcut")));
    {
        let env = env.clone();
        let (id, name) = (id.to_owned(), name.to_owned());
        button.connect_clicked(move |_| {
            let ctx = env.ctx.clone();
            let (id, name) = (id.clone(), name.clone());
            record_shortcut(
                &env,
                &trf("Press a shortcut for %@…", &[FormatArg::Str(name.clone())]),
                move |accel, label| {
                    ctx.update_settings(|s| {
                        assign_app_shortcut(s, &id, &name, accel, label, &|_| true)
                    })
                    .map_err(|e| tr(&e.to_string()))
                },
            );
        });
    }
    row.add_suffix(&button);
    if label.is_some() {
        let remove = icon_button(
            "edit-clear-symbolic",
            &trf("Remove shortcut for %@", &[FormatArg::Str(name.to_owned())]),
        );
        let env = env.clone();
        let id = id.to_owned();
        remove.connect_clicked(move |_| {
            if let Err(message) = env.ctx.edit_settings(|s| {
                remove_app_shortcut(s, &id);
            }) {
                env.toast(&message);
            }
        });
        row.add_suffix(&remove);
    }
    row
}

fn refresh_commands(env: &Rc<Env>, commands: &DynGroup) {
    let ctx = &env.ctx;
    let offered = ctx.availability.borrow().clone();
    let (targets, bindings) = ctx.with_settings(|s| {
        (
            command_targets(s, &ctx.l10n, &offered),
            s.command_shortcuts.clone(),
        )
    });
    let signature = format!(
        "{}|{}",
        targets
            .iter()
            .map(|t| format!("{}={}", t.id, t.title))
            .collect::<Vec<_>>()
            .join(","),
        bindings
            .iter()
            .map(|b| format!("{}={}", b.id, b.label))
            .collect::<Vec<_>>()
            .join(",")
    );
    commands.update(&signature, || {
        targets
            .iter()
            .map(|target| {
                let binding = bindings.iter().find(|b| b.id == target.id);
                let row = action_row(&target.title, "");
                row.add_prefix(&gtk::Image::from_icon_name(target.icon));
                let current = binding.map(|b| {
                    b.resolved()
                        .map_or_else(|| b.label.clone(), |a| accelerator_label(&a))
                });
                let button = row_button(&current.clone().unwrap_or_else(|| tr("Record Shortcut")));
                {
                    let env = env.clone();
                    let (id, title) = (target.id.clone(), target.title.clone());
                    button.connect_clicked(move |_| {
                        let ctx = env.ctx.clone();
                        let id = id.clone();
                        record_shortcut(
                            &env,
                            &trf("Press a shortcut for %@…", &[FormatArg::Str(title.clone())]),
                            move |accel, label| {
                                ctx.update_settings(|s| {
                                    assign_command_shortcut(s, &id, accel, label, &|_| true)
                                })
                                .map_err(|e| tr(&e.to_string()))
                            },
                        );
                    });
                }
                row.add_suffix(&button);
                if current.is_some() {
                    let remove = icon_button(
                        "edit-clear-symbolic",
                        &trf(
                            "Remove shortcut for %@",
                            &[FormatArg::Str(target.title.clone())],
                        ),
                    );
                    let env = env.clone();
                    let id = target.id.clone();
                    remove.connect_clicked(move |_| {
                        if let Err(message) = env.ctx.edit_settings(|s| {
                            remove_command_shortcut(s, &id);
                        }) {
                            env.toast(&message);
                        }
                    });
                    row.add_suffix(&remove);
                }
                row.upcast()
            })
            .collect()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebloop_platform::session::Compositor;

    #[test]
    fn compositor_lines_cover_launcher_apps_and_commands() {
        let mut settings = PortableSettings::default();
        assign_app_shortcut(
            &mut settings,
            "org.mozilla.firefox.desktop",
            "Firefox",
            "<Super>f",
            "Super+F",
            &|_| true,
        )
        .unwrap();
        assign_command_shortcut(&mut settings, "emoji", "<Super>e", "Super+E", &|_| true).unwrap();
        let lines = compositor_lines(SnippetTarget::Niri, &settings, "kebloop");
        let text: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(text.len(), 3);
        assert!(text[0].starts_with("Alt+Space") && text[0].contains("\"toggle\""));
        assert!(text[1].contains("\"launch\"") && text[1].contains("org.mozilla.firefox.desktop"));
        assert!(text[2].contains("\"command\"") && text[2].contains("\"emoji\""));
        let sway = compositor_lines(SnippetTarget::Sway, &settings, "kebloop");
        assert_eq!(sway[0].text, "bindsym Mod1+space exec kebloop toggle");
    }

    #[test]
    fn window_lines_use_the_recorded_shortcut_or_the_suggested_one() {
        use kebloop_services::commands::LauncherCommand as C;
        let mut settings = PortableSettings::default();
        assign_command_shortcut(&mut settings, "leftHalf", "<Super>h", "Super+H", &|_| true)
            .unwrap();
        let lines = window_lines(
            SnippetTarget::Sway,
            &settings,
            &[C::LeftHalf, C::MaximizeWindow, C::Emoji],
            "kebloop",
        );
        let text: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(text.len(), 2, "commands without any shortcut are left out");
        assert_eq!(text[0], "bindsym Mod4+h exec kebloop command leftHalf");
        assert!(text[1].ends_with("exec kebloop command maximizeWindow"));
        assert!(text[1].starts_with("bindsym ") && text[1].contains("Return"));
    }

    #[test]
    fn detected_compositor_selects_its_snippet() {
        assert_eq!(default_target(&Compositor::Niri), 0);
        assert_eq!(default_target(&Compositor::Hyprland), 1);
        assert_eq!(default_target(&Compositor::Sway), 2);
        assert_eq!(default_target(&Compositor::Other("i3".into())), 3);
        assert_eq!(default_target(&Compositor::Gnome), 0);
    }
}
