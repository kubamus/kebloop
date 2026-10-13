//! Window management: which window commands this compositor offers, a shortcut per command
//! (recorded here, saved in the configuration, also listed under Shortcuts and Help), the
//! suggested Ctrl+Alt set, and, without a global-shortcuts portal, compositor key bindings that
//! run `kebloop command <id>`.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use kebloop_platform::shortcuts::SnippetTarget;
use kebloop_services::commands::LauncherCommand;
use kebloop_services::shortcuts::{assign_command_shortcut, remove_command_shortcut};
use kebloop_services::window_management::apply_suggested_shortcuts;

use super::logic::{self, PageId, accelerator_label};
use super::recorder::record_shortcut;
use super::shortcuts::{default_target, window_lines};
use super::widgets::{
    ComboControl, DynGroup, Env, Page, action_row, button_row, copy_button, group, icon_button,
    mono_label, note, preferences_page, row_button,
};
use crate::context::{AppContext, PortalState};
use crate::i18n::{tr, trf};
use crate::launcher::model::window_note_text;
use kebloop_services::l10n::FormatArg;

/// The groups of the page, in the order of the macOS settings.
const GROUPS: [(&str, &[LauncherCommand]); 5] = [
    (
        "Halves",
        &[
            LauncherCommand::LeftHalf,
            LauncherCommand::RightHalf,
            LauncherCommand::TopHalf,
            LauncherCommand::BottomHalf,
        ],
    ),
    (
        "Quarters",
        &[
            LauncherCommand::TopLeftQuarter,
            LauncherCommand::TopRightQuarter,
            LauncherCommand::BottomLeftQuarter,
            LauncherCommand::BottomRightQuarter,
        ],
    ),
    (
        "Thirds",
        &[
            LauncherCommand::LeftThird,
            LauncherCommand::CenterThird,
            LauncherCommand::RightThird,
            LauncherCommand::LeftTwoThirds,
            LauncherCommand::RightTwoThirds,
        ],
    ),
    (
        "Size & Position",
        &[
            LauncherCommand::MaximizeWindow,
            LauncherCommand::CenterWindow,
            LauncherCommand::RestoreWindow,
        ],
    ),
    (
        "Displays",
        &[
            LauncherCommand::PreviousDisplay,
            LauncherCommand::NextDisplay,
        ],
    ),
];

/// The status row text: the compositor and what it can do, or why window commands are missing.
fn status_text(ctx: &AppContext) -> (String, String) {
    match (&ctx.compositor, &ctx.compositor_error) {
        (Some(compositor), _) => {
            let kind = compositor.kind();
            let table = kebloop_platform::window_management::capability_table(kind);
            let approximate = table
                .iter()
                .filter(|(_, support)| {
                    !matches!(
                        support,
                        kebloop_platform::window_management::ActionSupport::Full
                    )
                })
                .count();
            let detail = if approximate == 0 {
                tr("Every window command works as on macOS.")
            } else {
                trf(
                    "%d commands work approximately; they are marked below.",
                    &[FormatArg::Int(i64::try_from(approximate).unwrap_or(0))],
                )
            };
            (
                trf(
                    "Window commands use %@.",
                    &[FormatArg::Str(kind.name().to_owned())],
                ),
                detail,
            )
        }
        (None, reason) => (
            tr("Needs niri, Hyprland, Sway or i3."),
            reason.clone().unwrap_or_default(),
        ),
    }
}

/// Build the page.
pub fn build(env: &Rc<Env>) -> Page {
    let ctx = env.ctx.clone();
    let page = preferences_page();
    page.set_title(&PageId::WindowManagement.title());
    page.set_description(&PageId::WindowManagement.subtitle());
    let program = logic::kebloop_program(
        &kebloop_platform::paths::Env::from_process().path_dirs(),
        std::env::current_exe().ok().as_deref(),
    );

    // Status and the suggested shortcuts.
    let intro = group("", "");
    let status = action_row(&tr("Compositor"), "");
    status.add_prefix(&gtk::Image::from_icon_name("view-dual-symbolic"));
    status.set_title_lines(0);
    status.set_subtitle_lines(0);
    intro.add(&status);
    intro.add(&note(&tr(
        "Arrange the window that had focus before Kebloop opened with a global shortcut or a launcher command. Windows fit within the output’s usable area where the compositor reports it.",
    )));
    let feedback = note("");
    feedback.set_visible(false);
    {
        let ctx = ctx.clone();
        let env = env.clone();
        let feedback = feedback.clone();
        intro.add(&button_row(
            &tr("Use Suggested Shortcuts"),
            "object-select-symbolic",
            move || {
                let outcome = ctx.update_settings(|settings| {
                    Ok::<_, std::convert::Infallible>(apply_suggested_shortcuts(settings, &|_| {
                        true
                    }))
                });
                match outcome {
                    Ok(outcome) => {
                        let mut text = trf(
                            "Added %d shortcuts.",
                            &[FormatArg::Int(i64::try_from(outcome.added).unwrap_or(0))],
                        );
                        if !outcome.conflicts.is_empty() {
                            let titles: Vec<String> = outcome
                                .conflicts
                                .iter()
                                .map(|id| {
                                    LauncherCommand::from_id(id).map_or_else(
                                        || id.clone(),
                                        |command| command.title(&ctx.l10n),
                                    )
                                })
                                .collect();
                            text = format!(
                                "{text} {} {}",
                                tr("Conflicts skipped:"),
                                titles.join(", ")
                            );
                        }
                        feedback.set_label(&text);
                        feedback.set_visible(true);
                    }
                    Err(error) => env.toast(&tr(&error.to_string())),
                }
            },
        ));
    }
    intro.add(&note(&tr(
        "Uses Ctrl+Alt with arrow keys for halves, U/I/J/K for quarters, D/F/G for thirds, E/T for two thirds, Return to maximize, C to center, and Backspace to restore. Add Shift to Left/Right for displays. Existing shortcuts are kept; conflicts are skipped.",
    )));
    intro.add(&feedback);
    page.add(&intro);

    // One group per kind of arrangement.
    let groups: Vec<(&'static [LauncherCommand], Rc<DynGroup>)> = GROUPS
        .iter()
        .map(|(title, commands)| {
            let dynamic = Rc::new(DynGroup::new(group(&tr(title), "")));
            page.add(&dynamic.group);
            (*commands, dynamic)
        })
        .collect();

    // Compositor key bindings, for desktops without a global-shortcuts portal.
    let bindings = group(
        &tr("Compositor key bindings"),
        &tr(
            "Paste these lines into your compositor configuration to bind the window shortcuts. Unassigned commands use the suggested shortcut.",
        ),
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
            let offered = ctx.availability.borrow().clone();
            let commands: Vec<LauncherCommand> = GROUPS
                .iter()
                .flat_map(|(_, commands)| commands.iter().copied())
                .filter(|command| offered.offers(*command))
                .collect();
            let lines = ctx.with_settings(|s| window_lines(target, s, &commands, &program));
            snippet_label.set_label(
                &lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            hint_label.set_label(&trf(
                "Add to %@",
                &[FormatArg::Str(target.config_hint().to_owned())],
            ));
        })
    };
    let combo = {
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
    bindings.add(&combo.row);
    bindings.add(&hint_label);
    let frame = gtk::Box::builder()
        .css_classes(["card"])
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    snippet_label.set_margin_top(10);
    snippet_label.set_margin_bottom(10);
    snippet_label.set_margin_start(12);
    snippet_label.set_hexpand(true);
    frame.append(&snippet_label);
    let copy_all = copy_button(env, {
        let snippet_label = snippet_label.clone();
        move || snippet_label.label().to_string()
    });
    copy_all.set_valign(gtk::Align::Start);
    copy_all.set_margin_top(4);
    copy_all.set_margin_end(4);
    frame.append(&copy_all);
    bindings.add(&frame);
    page.add(&bindings);

    let notes = group("", "");
    notes.add(&note(&tr(
        "Restore returns to the frame saved before the first arrangement. Full-screen and hidden windows cannot be arranged. Display actions require more than one display.",
    )));
    page.add(&notes);

    let refresh: Rc<dyn Fn()> = {
        let env = env.clone();
        Rc::new(move || {
            let ctx = &env.ctx;
            let (title, subtitle) = status_text(ctx);
            status.set_title(&gtk::glib::markup_escape_text(&title));
            status.set_subtitle(&gtk::glib::markup_escape_text(&subtitle));
            bindings.set_visible(
                matches!(*ctx.portal.borrow(), PortalState::Unavailable(_))
                    && ctx.compositor.is_some(),
            );
            render();
            let offered = ctx.availability.borrow().clone();
            let recorded = ctx.with_settings(|s| s.command_shortcuts.clone());
            for (commands, dynamic) in &groups {
                let shown: Vec<LauncherCommand> = commands
                    .iter()
                    .copied()
                    .filter(|command| offered.offers(*command))
                    .collect();
                dynamic.group.set_visible(!shown.is_empty());
                let signature = shown
                    .iter()
                    .map(|command| {
                        let label = recorded
                            .iter()
                            .find(|b| b.id == command.id())
                            .map_or(String::new(), |b| b.label.clone());
                        format!("{}={label}", command.id())
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                dynamic.update(&signature, || {
                    shown
                        .iter()
                        .map(|command| command_row(&env, *command, &offered, &recorded).upcast())
                        .collect()
                });
            }
        })
    };
    refresh();
    Page::new(&page, move || refresh())
}

fn command_row(
    env: &Rc<Env>,
    command: LauncherCommand,
    offered: &crate::platform::CommandAvailability,
    recorded: &[kebloop_services::shortcuts::CommandShortcutBinding],
) -> adw::ActionRow {
    let ctx = &env.ctx;
    let title = command.title(&ctx.l10n);
    let subtitle = offered
        .window_note(command)
        .map(|note| window_note_text(note, &ctx.l10n))
        .unwrap_or_default();
    let row = action_row(&title, &subtitle);
    row.add_prefix(&gtk::Image::from_icon_name(command.icon_name()));
    let binding = recorded.iter().find(|b| b.id == command.id());
    let current = binding.map(|b| {
        b.resolved()
            .map_or_else(|| b.label.clone(), |a| accelerator_label(&a))
    });
    let button = row_button(&current.clone().unwrap_or_else(|| tr("Record Shortcut")));
    button.update_property(&[gtk::accessible::Property::Label(&format!(
        "{title}: {}",
        current.clone().unwrap_or_else(|| tr("Record Shortcut"))
    ))]);
    {
        let env = env.clone();
        let id = command.id().to_owned();
        let prompt = trf("Press a shortcut for %@…", &[FormatArg::Str(title.clone())]);
        button.connect_clicked(move |_| {
            let ctx = env.ctx.clone();
            let id = id.clone();
            record_shortcut(&env, &prompt, move |accelerator, label| {
                ctx.update_settings(|s| {
                    assign_command_shortcut(s, &id, accelerator, label, &|_| true)
                })
                .map_err(|error| tr(&error.to_string()))
            });
        });
    }
    row.add_suffix(&button);
    if current.is_some() {
        let remove = icon_button(
            "edit-clear-symbolic",
            &trf("Remove shortcut for %@", &[FormatArg::Str(title)]),
        );
        let env = env.clone();
        let id = command.id().to_owned();
        remove.connect_clicked(move |_| {
            if let Err(message) = env.ctx.edit_settings(|s| {
                remove_command_shortcut(s, &id);
            }) {
                env.toast(&message);
            }
        });
        row.add_suffix(&remove);
    }
    row
}
