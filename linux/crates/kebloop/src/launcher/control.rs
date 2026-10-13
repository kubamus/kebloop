//! The launcher's dealings with the rest of the desktop: the window that had focus before the
//! panel opened, running applications and their controls, direct paste, system commands and
//! window management. Everything that talks to the compositor, D-Bus or a subprocess runs on a
//! worker thread; results come back to the main loop.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use adw::prelude::*;
use gtk::gio;
use kebloop_platform::app_control::{
    AppAction, AppController, AppIdentity, AppOutcome, SystemProcesses, running_desktop_ids,
    windows_of,
};
use kebloop_platform::paste::{LivePasteEnvironment, PasteRequest, paste_into};
use kebloop_platform::system_actions::SystemAction;
use kebloop_platform::window_management::WindowAction;
use kebloop_services::application_control::{
    ApplicationAction, ToggleFocusStep, plan_toggle_focus,
};
use kebloop_services::commands::LauncherCommand;

use super::model::{ResultItem, Target, command_result_id};
use super::{Launcher, Mode, NoticeKind};
use crate::i18n::{tr, trf};
use crate::platform::identity_of;
use crate::{direct_paste, work};
use kebloop_services::l10n::FormatArg;

/// What the launcher showed when an action took it away; a late failure restores that.
pub(super) struct ExternalContext {
    query: String,
    mode: Mode,
    result_id: String,
    revision: u64,
}

impl Launcher {
    // ----- the session ----------------------------------------------------------------------

    /// Ask the compositor (off the main thread) which window has focus and which applications
    /// have windows. The panel is presented only after the snapshot finishes, so the previous
    /// window still holds focus when the worker queries the compositor.
    pub(super) fn begin_session(&self) {
        let Some(ctx) = self.ctx() else { return };
        let inner = &self.inner;
        inner.previous.borrow_mut().take();
        let Some(compositor) = ctx.compositor.clone() else {
            ctx.running_apps.borrow_mut().clear();
            self.finish_show();
            return;
        };
        let identities: Vec<AppIdentity> = ctx.apps.borrow().iter().map(identity_of).collect();
        let revision = inner.revision.get();
        let own_pid = std::process::id();
        let own_app_id = crate::cli::app_id().to_lowercase();
        let weak = self.downgrade();
        let spawned = work::spawn(
            "kebloop-windows",
            move || {
                compositor.windows().map(|windows| {
                    let running = running_desktop_ids(&identities, &windows);
                    let focused = windows.into_iter().find(|window| {
                        window.focused
                            && !window.hidden
                            && window.pid != Some(own_pid)
                            && window.app_id.as_deref().map(str::to_lowercase).as_deref()
                                != Some(own_app_id.as_str())
                    });
                    (running, focused)
                })
            },
            move |result| {
                let Some(launcher) = weak.upgrade() else {
                    return;
                };
                if launcher.inner.revision.get() != revision || !launcher.inner.opening.get() {
                    return;
                }
                let Some(ctx) = launcher.ctx() else { return };
                match result {
                    Ok((running, focused)) => {
                        *launcher.inner.previous.borrow_mut() = focused;
                        let changed = *ctx.running_apps.borrow() != running;
                        *ctx.running_apps.borrow_mut() = running;
                        if changed && launcher.mode() == Mode::Root {
                            // Callers may have restored a query, selection or error while the
                            // snapshot was pending. Updating running badges must retain those.
                            let selected = launcher.selected_item().map(|item| item.id);
                            let notice = launcher.inner.state.borrow().notice.clone();
                            launcher.refresh(true);
                            if let Some(id) = selected {
                                let position = launcher
                                    .inner
                                    .state
                                    .borrow()
                                    .items
                                    .iter()
                                    .position(|item| item.id == id);
                                if let Some(position) = position {
                                    launcher.inner.list.select(position, false);
                                }
                            }
                            if let Some((text, kind)) = notice {
                                launcher.set_notice(Some(text), kind);
                            }
                        }
                    }
                    Err(error) => {
                        eprintln!("kebloop: could not list the windows: {error}");
                        ctx.running_apps.borrow_mut().clear();
                    }
                }
                launcher.finish_show();
            },
        );
        if !spawned {
            ctx.running_apps.borrow_mut().clear();
            self.finish_show();
        }
    }

    /// Stop a paste that is still waiting for the previous window to regain focus.
    pub(super) fn cancel_paste(&self) {
        self.inner
            .paste_cancel
            .borrow()
            .store(true, Ordering::Relaxed);
    }

    // ----- actions that leave the launcher --------------------------------------------------

    /// Hide the panel for an action that finishes elsewhere, remembering how to come back if
    /// it fails.
    pub(super) fn begin_external_action(&self, result_id: &str) -> ExternalContext {
        let context = ExternalContext {
            query: self.query(),
            mode: self.mode(),
            result_id: result_id.to_owned(),
            revision: self.inner.revision.get(),
        };
        self.hide();
        context
    }

    /// Bring the launcher back with `message`, unless the user opened it again meanwhile.
    pub(super) fn show_external_error(&self, message: &str, context: &ExternalContext) {
        if self.is_visible() || self.inner.revision.get() != context.revision {
            return;
        }
        self.show();
        if !matches!(
            context.mode,
            Mode::Root | Mode::Emoji | Mode::Snippet | Mode::Extension(_)
        ) {
            self.set_mode(context.mode.clone());
        }
        self.set_query_silently(&context.query);
        self.refresh(false);
        let position = self
            .inner
            .state
            .borrow()
            .items
            .iter()
            .position(|item| item.id == context.result_id);
        if let Some(position) = position {
            self.inner.list.select(position, false);
        }
        self.set_notice(Some(message.to_owned()), NoticeKind::Error);
    }

    /// Show the launcher listing `command` with `message` (an action started without the panel
    /// failed).
    fn show_command_error(&self, command: LauncherCommand, message: &str) {
        let Some(ctx) = self.ctx() else { return };
        if self.is_visible() {
            return;
        }
        self.show();
        self.set_query_silently(&command.title(&ctx.l10n));
        self.refresh(false);
        self.set_notice(Some(message.to_owned()), NoticeKind::Error);
    }

    /// A confirmation of ours; the panel stays open meanwhile.
    pub(super) fn confirm(
        &self,
        title: &str,
        detail: &str,
        label: &str,
        then: impl FnOnce(&Launcher) + 'static,
    ) {
        if self.inner.opening.get() {
            let title = title.to_owned();
            let detail = detail.to_owned();
            let label = label.to_owned();
            self.when_shown(move |launcher| launcher.confirm(&title, &detail, &label, then));
            return;
        }
        let dialog = adw::AlertDialog::new(Some(title), Some(detail));
        dialog.add_responses(&[("cancel", &tr("Cancel")), ("confirm", label)]);
        dialog.set_response_appearance("confirm", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        self.set_confirming(true);
        let weak = self.downgrade();
        dialog.choose(
            Some(self.window()),
            None::<&gio::Cancellable>,
            move |response| {
                let Some(launcher) = weak.upgrade() else {
                    return;
                };
                launcher.set_confirming(false);
                if response == "confirm" {
                    then(&launcher);
                }
                if launcher.is_visible() {
                    launcher.focus_input();
                }
            },
        );
    }

    // ----- direct paste ---------------------------------------------------------------------

    /// Hide the launcher and paste what was just copied into the window that had focus before
    /// it opened. The copy stays on the clipboard whatever happens; failures are reported.
    pub(super) fn paste_into_previous(&self) {
        let Some(ctx) = self.ctx() else { return };
        let previous = self.inner.previous.borrow().clone();
        self.hide();
        let enabled =
            ctx.with_settings(|s| direct_paste::enabled(&s.extra, ctx.paste_tool.is_some()));
        let plan = match direct_paste::plan(
            enabled,
            ctx.paste_tool,
            ctx.compositor.is_some(),
            previous.as_ref(),
        ) {
            Ok(plan) => plan,
            Err(refusal) => {
                show_paste_alert(&direct_paste::refusal_text(&refusal));
                return;
            }
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.inner
            .paste_cancel
            .replace(cancel.clone())
            .store(true, Ordering::Relaxed);
        let compositor = ctx.compositor.clone();
        let counter = ctx.clipboard_revision.clone();
        let snapshot = counter.load(Ordering::Relaxed);
        work::spawn(
            "kebloop-paste",
            move || {
                let read = || counter.load(Ordering::Relaxed);
                let environment = LivePasteEnvironment {
                    compositor: compositor.as_deref(),
                    cancel: Some(&cancel),
                    clipboard_revision: Some(&read),
                };
                let mut request = PasteRequest::new(plan.tool, plan.chord, plan.target);
                request.clipboard_revision = Some(snapshot);
                paste_into(&environment, &request)
            },
            |result| {
                if let Some(text) = result
                    .err()
                    .and_then(|error| direct_paste::failure_text(&error))
                {
                    show_paste_alert(&text);
                }
            },
        );
    }

    // ----- application controls -------------------------------------------------------------

    /// Hide, quit, force quit or restart the application behind `item`.
    pub(super) fn run_app_control(&self, action: ApplicationAction, item: &ResultItem) {
        let Some(ctx) = self.ctx() else { return };
        let Target::Application { desktop_id, .. } = &item.target else {
            return;
        };
        let Some(app) = ctx
            .apps
            .borrow()
            .iter()
            .find(|app| app.id == *desktop_id)
            .cloned()
        else {
            return;
        };
        if ctx.compositor.is_none() {
            self.fail(tr("This session has no supported compositor."));
            return;
        }
        let item = item.clone();
        if action.needs_confirmation() {
            self.confirm(
                &tr("Force Quit Application…"),
                &format!("{}\n{}", app.name, tr("Unsaved changes may be lost.")),
                &tr("Force Quit"),
                move |launcher| launcher.start_app_control(action, &item, &app),
            );
        } else {
            self.start_app_control(action, &item, &app);
        }
    }

    fn start_app_control(
        &self,
        action: ApplicationAction,
        item: &ResultItem,
        app: &kebloop_platform::apps::DesktopApp,
    ) {
        let Some(ctx) = self.ctx() else { return };
        let Some(compositor) = ctx.compositor.clone() else {
            return;
        };
        let context = self.begin_external_action(&item.id);
        let identity = identity_of(app);
        let path = app.path.to_string_lossy().into_owned();
        let name = app.name.clone();
        let operation = match action {
            ApplicationAction::Quit => AppAction::Quit,
            ApplicationAction::ForceQuit => AppAction::ForceQuit,
            ApplicationAction::Restart => AppAction::Restart,
            ApplicationAction::ToggleFocus => AppAction::ToggleFocus,
            ApplicationAction::Hide => AppAction::Hide,
        };
        let weak = self.downgrade();
        work::spawn(
            "kebloop-app-control",
            move || {
                AppController::new(compositor.as_ref(), &SystemProcesses)
                    .perform(operation, &identity, true)
                    .map_err(|error| tr(&error.to_string()))
            },
            move |result| {
                let Some(launcher) = weak.upgrade() else {
                    return;
                };
                match result {
                    Ok(AppOutcome::Relaunch) => {
                        // The old instance is gone: start it again.
                        if let Err(error) = launcher.launch_desktop_file(&path, None) {
                            let message = trf(
                                "Couldn’t open %@: %@",
                                &[FormatArg::Str(name), FormatArg::Str(error)],
                            );
                            launcher.show_external_error(&message, &context);
                        }
                    }
                    Ok(_) => {}
                    Err(message) => launcher.show_external_error(&message, &context),
                }
            },
        );
    }

    /// An application shortcut: launch the application, bring it to the front, or hide it when
    /// it already has focus (and the compositor can hide windows).
    pub(super) fn toggle_application(&self, desktop_id: &str) {
        let Some(ctx) = self.ctx() else { return };
        let identity = ctx
            .apps
            .borrow()
            .iter()
            .find(|app| app.id == desktop_id)
            .map_or_else(
                || AppIdentity::from_entry(desktop_id, None, ""),
                identity_of,
            );
        if self.is_visible() || self.inner.opening.get() {
            self.hide();
        }
        let Some(compositor) = ctx.compositor.clone() else {
            self.start_application(desktop_id);
            return;
        };
        let id = desktop_id.to_owned();
        let weak = self.downgrade();
        work::spawn(
            "kebloop-app-toggle",
            move || -> Result<ToggleFocusStep, String> {
                let windows = match compositor.windows() {
                    Ok(windows) => windows,
                    Err(error) => {
                        eprintln!("kebloop: could not list the windows: {error}");
                        return Ok(ToggleFocusStep::Launch);
                    }
                };
                let mine = windows_of(&identity, &windows);
                let frontmost = mine.iter().any(|w| w.focused && !w.hidden);
                let step = plan_toggle_focus(!mine.is_empty(), frontmost);
                let controller = AppController::new(compositor.as_ref(), &SystemProcesses);
                let outcome = match step {
                    ToggleFocusStep::Launch => return Ok(step),
                    // Where windows cannot be hidden the focused application stays as it is.
                    ToggleFocusStep::Hide if !controller.hide_supported() => return Ok(step),
                    ToggleFocusStep::Hide => controller.perform(AppAction::Hide, &identity, false),
                    ToggleFocusStep::Activate => {
                        controller.perform(AppAction::Focus, &identity, false)
                    }
                };
                outcome.map(|_| step).map_err(|error| error.to_string())
            },
            move |result| {
                let Some(launcher) = weak.upgrade() else {
                    return;
                };
                match result {
                    Ok(ToggleFocusStep::Launch) => launcher.start_application(&id),
                    Ok(_) => {}
                    Err(message) => launcher.notify_failure(&message),
                }
            },
        );
    }

    /// Tell the user about a failure that happened while the launcher was closed.
    fn notify_failure(&self, message: &str) {
        eprintln!("kebloop: {message}");
        if let Some(ctx) = self.ctx() {
            let notification = gio::Notification::new(&tr("Kebloop"));
            notification.set_body(Some(message));
            ctx.app
                .send_notification(Some("action-failed"), &notification);
        }
    }

    // ----- system commands ------------------------------------------------------------------

    /// Lock, sleep, restart, log out, toggle mute or toggle appearance. Restart and log out ask
    /// first. Failures are reported; success is never assumed beyond what the backend reported.
    pub(super) fn run_system_command(&self, command: LauncherCommand, action: SystemAction) {
        if action.needs_confirmation() {
            let (title, label) = match action {
                SystemAction::Restart => (tr("Restart this computer?"), tr("Restart")),
                _ => (tr("Log out of this session?"), tr("Log Out")),
            };
            self.confirm(
                &title,
                &tr("Unsaved work in other applications may be lost. Your system may ask you to confirm."),
                &label,
                move |launcher| launcher.start_system_action(command, action),
            );
        } else {
            self.start_system_action(command, action);
        }
    }

    fn start_system_action(&self, command: LauncherCommand, action: SystemAction) {
        let Some(ctx) = self.ctx() else { return };
        let context = self
            .is_visible()
            .then(|| self.begin_external_action(&command_result_id(command)));
        let compositor = ctx.compositor.clone();
        let weak = self.downgrade();
        work::spawn(
            "kebloop-system-action",
            move || {
                crate::services::system::perform(action, compositor.as_deref())
                    .map_err(|error| tr(&error))
            },
            move |result| {
                let (Some(launcher), Err(message)) = (weak.upgrade(), result) else {
                    return;
                };
                match &context {
                    Some(context) => launcher.show_external_error(&message, context),
                    None => launcher.show_command_error(command, &message),
                }
            },
        );
    }

    // ----- window management ----------------------------------------------------------------

    /// Arrange the window that had focus before the launcher opened; from a shortcut or the
    /// command line (launcher closed), the window that has focus now.
    pub(super) fn run_window_command(&self, command: LauncherCommand, action: WindowAction) {
        let Some(ctx) = self.ctx() else { return };
        let Some(manager) = ctx.window_manager.clone() else {
            self.fail(tr("This session has no supported compositor."));
            return;
        };
        let visible = self.is_visible();
        let target = if visible {
            self.inner.previous.borrow().clone()
        } else {
            None
        };
        let context = visible.then(|| self.begin_external_action(&command_result_id(command)));
        let compositor = ctx.compositor.clone();
        let weak = self.downgrade();
        work::spawn(
            "kebloop-window-action",
            move || {
                let outcome = manager
                    .perform(action, target.as_ref().map(|window| &window.id))
                    .map_err(|error| tr(&error.to_string()));
                if outcome.is_ok()
                    && let (Some(window), Some(compositor)) = (&target, &compositor)
                {
                    // Give the window the focus the panel had taken.
                    let _ = compositor.focus_window(&window.id);
                }
                outcome
            },
            move |result| {
                let (Some(launcher), Err(message)) = (weak.upgrade(), result) else {
                    return;
                };
                match &context {
                    Some(context) => launcher.show_external_error(&message, context),
                    None => launcher.show_command_error(command, &message),
                }
            },
        );
    }
}

/// The "Direct Paste" alert (Swift: an `NSAlert`): a plain window, because the launcher is gone.
fn show_paste_alert(text: &str) {
    let dialog = gtk::AlertDialog::builder()
        .modal(false)
        .message(tr("Direct Paste"))
        .detail(text)
        .buttons([tr("OK")])
        .build();
    dialog.choose(None::<&gtk::Window>, None::<&gio::Cancellable>, |_| {});
}
