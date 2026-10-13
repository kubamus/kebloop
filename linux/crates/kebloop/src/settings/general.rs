//! General: launcher shortcut, launch at login, AI provider, language, application refresh.

use std::rc::Rc;

use adw::prelude::*;
use kebloop_platform::autostart::Autostart;
use kebloop_services::ai::AiProvider;
use kebloop_services::shortcuts::{Shortcut, assign_launcher_shortcut, choose_launcher_shortcut};

use super::logic::{self, PageId, accelerator_label};
use super::recorder::record_shortcut;
use super::widgets::{
    ComboControl, Env, Page, SwitchControl, button_row, copy_button, group, mono_label, note,
    preferences_page, set_error,
};
use crate::context::AppContext;
use crate::i18n::tr;

/// The XDG autostart entry for this process, pointing at the running binary (desktop-entry
/// quoted) unless it is installed in a standard system prefix.
pub fn autostart_entry(ctx: &AppContext) -> Autostart {
    let exec = kebloop_platform::autostart::exec_line(
        std::env::current_exe().ok().as_deref(),
        &["--background"],
    );
    Autostart::for_dirs(&ctx.xdg).with_exec(exec)
}

/// Labels of the launcher shortcut choices and the index of the selected one.
fn launcher_choices(ctx: &AppContext) -> (Vec<String>, Vec<String>, u32) {
    ctx.with_settings(|settings| {
        let mut ids: Vec<String> = Vec::new();
        let mut labels: Vec<String> = Vec::new();
        for choice in Shortcut::choices() {
            ids.push(choice.id.clone());
            labels.push(choice.label.clone());
        }
        if let Some(custom) = &settings.launcher_custom_shortcut {
            ids.push("custom".into());
            let label = custom
                .resolved()
                .map_or_else(|| custom.label.clone(), |a| accelerator_label(&a));
            labels.push(format!("{} · {label}", tr("Custom")));
        }
        let selected = ids
            .iter()
            .position(|id| *id == settings.launcher_shortcut_id)
            .unwrap_or(0);
        (ids, labels, u32::try_from(selected).unwrap_or(0))
    })
}

/// Build the page.
pub fn build(env: &Rc<Env>) -> Page {
    let ctx = &env.ctx;
    let page = preferences_page();
    page.set_title(&PageId::General.title());
    page.set_description(&PageId::General.subtitle());

    // Launcher shortcut.
    let launcher = group(&tr("Launcher"), "");
    let (ids, labels, selected) = launcher_choices(ctx);
    let ids = Rc::new(std::cell::RefCell::new(ids));
    let combo = {
        let ids = ids.clone();
        let ctx = ctx.clone();
        Rc::new(ComboControl::new(
            env,
            &tr("Open launcher"),
            "",
            &labels,
            selected,
            move |index| {
                let Some(id) = ids.borrow().get(index as usize).cloned() else {
                    return Ok(());
                };
                ctx.update_settings(|s| choose_launcher_shortcut(s, &id))
                    .map_err(|e| tr(&e.to_string()))
            },
        ))
    };
    launcher.add(&combo.row);
    let record_error = super::widgets::error_label();
    {
        let env = env.clone();
        let error = record_error.clone();
        launcher.add(&button_row(
            &tr("Record Custom Shortcut…"),
            "input-keyboard-symbolic",
            move || {
                set_error(&error, None);
                let ctx = env.ctx.clone();
                record_shortcut(
                    &env,
                    &tr("Press a shortcut for the launcher…"),
                    move |accel, label| {
                        ctx.update_settings(|s| assign_launcher_shortcut(s, accel, label))
                            .map_err(|e| tr(&e.to_string()))
                    },
                );
            },
        ));
    }
    launcher.add(&record_error);
    let portal_note = note("");
    launcher.add(&portal_note);
    let command_row = super::widgets::action_row(
        &tr("Compositor command"),
        &tr("Bind this command to a key in your compositor to toggle the launcher."),
    );
    let program = logic::kebloop_program(
        &kebloop_platform::paths::Env::from_process().path_dirs(),
        std::env::current_exe().ok().as_deref(),
    );
    let toggle = logic::toggle_command(&program);
    let command_label = mono_label(&toggle);
    command_label.set_valign(gtk::Align::Center);
    command_row.add_suffix(&command_label);
    command_row.add_suffix(&copy_button(env, {
        let toggle = toggle.clone();
        move || toggle.clone()
    }));
    launcher.add(&command_row);
    page.add(&launcher);

    // Launch at login.
    let startup = group(&tr("Startup"), "");
    let login = {
        let ctx = ctx.clone();
        Rc::new(SwitchControl::new(
            env,
            &tr("Launch at login"),
            &tr("Starts Kebloop in the background when you log in."),
            autostart_entry(&ctx).is_enabled(),
            {
                let ctx = ctx.clone();
                move |on| {
                    let entry = autostart_entry(&ctx);
                    let result = if on { entry.enable() } else { entry.disable() };
                    result.map_err(|error| {
                        format!("{} {error}", tr("Could not change launch at login."))
                    })
                }
            },
        ))
    };
    startup.add(&login.row);
    page.add(&startup);

    // Direct paste.
    let paste_group = group(&tr("Direct Paste"), "");
    let paste_status = super::widgets::action_row(&tr("Paste tool"), &paste_status_text(ctx));
    paste_status.add_prefix(&gtk::Image::from_icon_name("insert-text-symbolic"));
    paste_status.set_subtitle_lines(0);
    paste_group.add(&paste_status);
    let direct_paste = {
        let ctx = ctx.clone();
        Rc::new(SwitchControl::new(
            env,
            &tr("Paste directly into the previous window"),
            &tr(
                "After copying from Clipboard History, Emoji, or Snippets, send the paste keys to the window that was active before Kebloop opened. The copy always stays on the clipboard.",
            ),
            direct_paste_on(&ctx),
            {
                let ctx = ctx.clone();
                move |on| ctx.edit_settings(|s| crate::direct_paste::set_enabled(&mut s.extra, on))
            },
        ))
    };
    paste_group.add(&direct_paste.row);
    page.add(&paste_group);

    // AI provider.
    let ai = group(&tr("AI handoff"), "");
    let providers: Vec<String> = AiProvider::ALL
        .iter()
        .map(|p| p.settings_title().to_owned())
        .collect();
    let current = ctx.config.borrow().global().ai_provider;
    let ai_note = note(&tr(current.explanation_key()));
    let ai_combo = {
        let ctx = ctx.clone();
        let ai_note = ai_note.clone();
        Rc::new(ComboControl::new(
            env,
            &tr("AI provider"),
            "",
            &providers,
            AiProvider::ALL
                .iter()
                .position(|p| *p == current)
                .and_then(|i| u32::try_from(i).ok())
                .unwrap_or(0),
            move |index| {
                let Some(provider) = AiProvider::ALL.get(index as usize).copied() else {
                    return Ok(());
                };
                ctx.config
                    .borrow_mut()
                    .set_ai_provider(provider)
                    .map_err(|e| tr(&e.to_string()))?;
                ai_note.set_label(&tr(provider.explanation_key()));
                ctx.emit(crate::context::Change::Settings);
                Ok(())
            },
        ))
    };
    ai.add(&ai_combo.row);
    ai.add(&ai_note);
    ai.add(&note(&tr(
        "Type a query in the launcher, then press Tab. The provider opens in your browser with the query filled in.",
    )));
    page.add(&ai);

    // Language.
    let language = group(&tr("Language"), "");
    let code = ctx.l10n.language().code();
    let language_row = super::widgets::action_row(
        &tr("Language"),
        &tr(
            "Kebloop follows your system language (LANGUAGE, LC_ALL, LANG). English and Polish are available.",
        ),
    );
    let language_value = gtk::Label::builder()
        .label(if code == "pl" { "Polski" } else { "English" })
        .css_classes(["dim-label"])
        .valign(gtk::Align::Center)
        .build();
    language_row.add_suffix(&language_value);
    language.add(&language_row);
    page.add(&language);

    // Applications.
    let apps = group(&tr("Applications"), "");
    {
        let ctx = ctx.clone();
        let env = env.clone();
        apps.add(&button_row(
            &tr("Refresh Applications"),
            "view-refresh-symbolic",
            move || {
                crate::services::apps::refresh(&ctx);
                env.toast(&tr("Refreshing the application list…"));
            },
        ));
    }
    {
        let ctx = ctx.clone();
        apps.add(&button_row(
            &tr("Welcome and Help"),
            "starred-symbolic",
            move || crate::onboarding::open_onboarding(&ctx.app, &ctx),
        ));
    }
    page.add(&apps);

    let refresh = {
        let ctx = ctx.clone();
        let ids = ids.clone();
        let combo = combo.clone();
        let login = login.clone();
        let portal_note = portal_note.clone();
        let ai_combo = ai_combo.clone();
        let ai_note = ai_note.clone();
        let direct_paste = direct_paste.clone();
        let paste_status = paste_status.clone();
        move || {
            let (new_ids, labels, selected) = launcher_choices(&ctx);
            if *ids.borrow() != new_ids {
                *ids.borrow_mut() = new_ids;
                combo.sync(&labels, selected);
            } else {
                combo.select(selected);
            }
            login.sync(autostart_entry(&ctx).is_enabled());
            direct_paste.sync(direct_paste_on(&ctx));
            paste_status.set_subtitle(&gtk::glib::markup_escape_text(&paste_status_text(&ctx)));
            portal_note.set_label(&portal_text(&ctx));
            let provider = ctx.config.borrow().global().ai_provider;
            if let Some(index) = AiProvider::ALL.iter().position(|p| *p == provider) {
                ai_combo.select(u32::try_from(index).unwrap_or(0));
            }
            ai_note.set_label(&tr(provider.explanation_key()));
        }
    };
    refresh();
    Page::new(&page, refresh)
}

fn portal_text(ctx: &AppContext) -> String {
    use crate::context::PortalState;
    match &*ctx.portal.borrow() {
        PortalState::Active(_) => {
            tr("The launcher shortcut is registered through the desktop's global shortcuts portal.")
        }
        PortalState::Checking => tr("Checking for global shortcut support…"),
        PortalState::Unavailable(_) => crate::i18n::trf(
            "Your desktop (%@) has no global shortcuts portal. Bind the command below in your compositor; see Shortcuts.",
            &[kebloop_services::l10n::FormatArg::Str(
                ctx.session.compositor.name().to_owned(),
            )],
        ),
    }
}

/// Whether direct paste is on (an explicit choice, else on when a paste tool is installed).
fn direct_paste_on(ctx: &AppContext) -> bool {
    ctx.with_settings(|s| crate::direct_paste::enabled(&s.extra, ctx.paste_tool.is_some()))
}

fn paste_status_text(ctx: &AppContext) -> String {
    match ctx.paste_tool {
        Some(tool) => crate::i18n::trf(
            "Detected: %@",
            &[kebloop_services::l10n::FormatArg::Str(
                tool.program().to_owned(),
            )],
        ),
        None => tr("Install wtype or ydotool to paste directly."),
    }
}
