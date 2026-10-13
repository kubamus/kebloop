//! Performing results and actions: launching applications, opening files and links, copying,
//! pinning and the built-in commands.

use std::path::Path;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use kebloop_core::AiProvider;
use kebloop_core::CalculationResult;
use kebloop_services::application_control::ApplicationAction;
use kebloop_services::commands::{LauncherCommand, is_user_command_id};
use kebloop_services::personalization::{PinnedResult, toggle_pin};
use kebloop_services::quicklinks::{self, Destination};

use super::actions::ResultAction;
use super::model::{ResultItem, Target, command_result_id};
use super::views::{clipboard as clipboard_view, script};
use super::{Launcher, Mode, NoticeKind};
use crate::context::AppContext;
use crate::i18n::{tr, trf};
use crate::platform::{system_action_of, window_action_of};
use kebloop_services::l10n::FormatArg;

impl Launcher {
    // ----- helpers --------------------------------------------------------------------------

    fn launch_context(&self) -> Option<gdk::AppLaunchContext> {
        gdk::Display::default().map(|display| display.app_launch_context())
    }

    /// Hide, then count the use (the write happens after the window is gone).
    fn accept(&self, id: &str, count_launch: bool) {
        self.hide();
        self.count_use_with(id, count_launch);
    }

    /// Remember the use of `id` as recent, once the panel is gone.
    pub(super) fn count_use(&self, id: &str) {
        self.count_use_with(id, false);
    }

    fn count_use_with(&self, id: &str, count_launch: bool) {
        if let Some(ctx) = self.ctx() {
            let id = id.to_owned();
            glib::idle_add_local_once(move || ctx.record_use(&id, count_launch));
        }
    }

    /// Add an answer the user copied to the calculator history (typing alone never records).
    fn record_calculation(&self, result: &CalculationResult) {
        let Some(ctx) = self.ctx() else { return };
        let result = result.clone();
        glib::idle_add_local_once(move || {
            if !ctx
                .calculation_history
                .borrow_mut()
                .record_accepted(&result)
            {
                eprintln!("kebloop: the calculation could not be added to the history");
            }
            if let Some(error) = ctx.calculation_history.borrow().last_error() {
                eprintln!("kebloop: could not save the calculator history: {error}");
            }
        });
    }

    pub(super) fn fail(&self, message: String) {
        self.set_notice(Some(message), NoticeKind::Error);
    }

    /// Open a URI with the default handler. Synchronous: spawning the handler is quick and the
    /// activation token must be requested while the panel still has focus.
    fn open_uri(&self, uri: &str) -> Result<(), String> {
        let context = self.launch_context();
        gio::AppInfo::launch_default_for_uri(uri, context.as_ref()).map_err(|e| e.to_string())
    }

    fn open_path(&self, path: &str) -> Result<(), String> {
        if !Path::new(path).exists() {
            return Err(tr("This item no longer exists."));
        }
        self.open_uri(&gio::File::for_path(path).uri())
    }

    /// Launch the desktop file at `path`, optionally one of its `[Desktop Action]`s.
    pub(super) fn launch_desktop_file(
        &self,
        path: &str,
        action: Option<&str>,
    ) -> Result<(), String> {
        let info = gio_unix::DesktopAppInfo::from_filename(path)
            .ok_or_else(|| tr("This result is no longer available."))?;
        let context = self.launch_context();
        match action {
            Some(action) => {
                info.launch_action(action, context.as_ref());
                Ok(())
            }
            None => info
                .launch(&[], context.as_ref())
                .map_err(|e| e.to_string()),
        }
    }

    fn copy_text(&self, text: &str) -> bool {
        match self.ctx() {
            Some(ctx) => crate::services::clipboard::copy_text(&ctx, text, false),
            None => false,
        }
    }

    // ----- results --------------------------------------------------------------------------

    /// Run a result: the "Open" action.
    pub(super) fn perform_item(&self, item: &ResultItem) {
        let Some(ctx) = self.ctx() else { return };
        match &item.target {
            Target::Application { path, desktop_id } => {
                match self.launch_desktop_file(path, None) {
                    Ok(()) => self.accept(&item.id, true),
                    Err(error) => self.fail(trf(
                        "Couldn’t open %@: %@",
                        &[
                            FormatArg::Str(app_name(&ctx, desktop_id, &item.title)),
                            FormatArg::Str(error),
                        ],
                    )),
                }
            }
            Target::Command(command) => self.handle_command(*command, &item.id),
            Target::Quicklink { link, argument } => match link.resolved(argument, &ctx.home) {
                Some(Destination::Url(url)) => self.finish_open(url.as_str(), &item.id, true),
                Some(Destination::File(path)) => {
                    let path = path.to_string_lossy().into_owned();
                    match self.open_path(&path) {
                        Ok(()) => self.accept(&item.id, true),
                        Err(error) => self.fail(error),
                    }
                }
                None => self.fail(tr("Couldn’t open this quicklink.")),
            },
            Target::File { path, .. } => match self.open_path(path) {
                Ok(()) => self.accept(&item.id, false),
                Err(error) => self.fail(error),
            },
            Target::Calculation { result } => {
                if self.copy_text(&result.clipboard) {
                    self.accept(&item.id, false);
                    self.record_calculation(result);
                } else {
                    self.fail(tr("Could not write to the clipboard."));
                }
            }
            Target::HistoricalCalculation { result } => {
                if self.copy_text(&result.clipboard) {
                    self.hide();
                } else {
                    self.fail(tr("Could not write to the clipboard."));
                }
            }
            Target::WebSearch { query } => {
                let url = ctx
                    .with_settings(|s| quicklinks::fallback_url(&s.web_searches, query, &ctx.home));
                match url {
                    Some(Destination::Url(url)) => self.finish_open(url.as_str(), &item.id, false),
                    _ => self.fail(tr("Couldn’t open web search.")),
                }
            }
            Target::Snippet(snippet) => self.begin_snippet(snippet),
            Target::Script(script_command) => {
                self.hide();
                script::run(&ctx, script_command, Vec::new());
                let id = item.id.clone();
                glib::idle_add_local_once(move || ctx.record_use(&id, true));
            }
            Target::Extension(manifest) => self.set_mode(Mode::Extension(manifest.id.clone())),
            Target::Clipboard { entry_id, .. } => {
                match clipboard_view::copy_entry(&ctx, entry_id) {
                    Ok(()) => {
                        self.count_use(&item.id);
                        self.paste_into_previous();
                    }
                    Err(error) => self.fail(error),
                }
            }
            Target::Profile { id, active } => {
                if !*active {
                    self.switch_profile(id);
                }
            }
            Target::ExtensionResult { .. } => {}
        }
    }

    fn finish_open(&self, uri: &str, id: &str, count_launch: bool) {
        match self.open_uri(uri) {
            Ok(()) => self.accept(id, count_launch),
            Err(error) => self.fail(error),
        }
    }

    /// Run one entry of the action menu on `item`.
    pub(super) fn perform_action(&self, action: &ResultAction, item: &ResultItem) {
        let Some(ctx) = self.ctx() else { return };
        match action {
            ResultAction::Open => self.perform_item(item),
            ResultAction::DesktopAction(id) => {
                if let Target::Application { path, .. } = &item.target {
                    match self.launch_desktop_file(path, Some(id)) {
                        Ok(()) => self.accept(&item.id, true),
                        Err(error) => self.fail(error),
                    }
                }
            }
            ResultAction::Reveal => {
                if let Some(path) = target_path(&item.target) {
                    self.reveal(&path, &item.id);
                }
            }
            ResultAction::CopyPath => {
                if let Some(path) = target_path(&item.target) {
                    if self.copy_text(&path) {
                        self.accept(&item.id, false);
                    } else {
                        self.fail(tr("Could not write to the clipboard."));
                    }
                }
            }
            ResultAction::OpenWith => {
                if let Some(path) = target_path(&item.target) {
                    self.open_with(&path);
                }
            }
            ResultAction::EditQuicklink => {
                self.hide();
                crate::settings::open_settings(&ctx.app, &ctx, Some("quicklinks"));
            }
            ResultAction::Pin | ResultAction::Unpin => self.toggle_pin(item),
            ResultAction::RunWithInput => {
                if let Target::Script(script_command) = &item.target {
                    self.ask_script_arguments(script_command);
                }
            }
            ResultAction::RemoveClipboardItem => {
                if let Target::Clipboard { entry_id, .. } = &item.target {
                    clipboard_view::remove_entry(&ctx, entry_id);
                }
            }
            ResultAction::HideApp => {
                if let Target::Application { desktop_id, .. } = &item.target {
                    let id = desktop_id.clone();
                    let outcome = ctx.update_settings(|settings| {
                        if !settings.personalization.hidden_apps.contains(&id) {
                            settings.personalization.hidden_apps.push(id.clone());
                        }
                        Ok::<(), std::convert::Infallible>(())
                    });
                    if outcome.is_err() {
                        self.fail(tr("Could not save the setting."));
                    }
                }
            }
            ResultAction::Copy => {
                if let Target::Clipboard { entry_id, .. } = &item.target {
                    match clipboard_view::copy_entry(&ctx, entry_id) {
                        Ok(()) => self.accept(&item.id, false),
                        Err(error) => self.fail(error),
                    }
                }
            }
            ResultAction::PastePlainText => {
                if let Target::Clipboard { entry_id, .. } = &item.target {
                    let text = ctx
                        .clipboard
                        .borrow()
                        .items()
                        .iter()
                        .find(|entry| entry.id == *entry_id)
                        .map(|entry| entry.text.clone())
                        .filter(|text| !text.is_empty());
                    match text {
                        Some(text) if self.copy_text(&text) => {
                            self.count_use(&item.id);
                            self.paste_into_previous();
                        }
                        Some(_) => self.fail(tr("Could not write to the clipboard.")),
                        None => {}
                    }
                }
            }
            ResultAction::ClearCalculationHistory => self.confirm_clear_calculation_history(),
            ResultAction::HideApplication => self.run_app_control(ApplicationAction::Hide, item),
            ResultAction::QuitApplication => self.run_app_control(ApplicationAction::Quit, item),
            ResultAction::ForceQuitApplication => {
                self.run_app_control(ApplicationAction::ForceQuit, item);
            }
            ResultAction::RestartApplication => {
                self.run_app_control(ApplicationAction::Restart, item);
            }
            ResultAction::ExtensionAction(_) => {}
        }
    }

    fn toggle_pin(&self, item: &ResultItem) {
        let Some(ctx) = self.ctx() else { return };
        if let Target::Clipboard { entry_id, pinned } = &item.target {
            if !clipboard_view::set_pinned(&ctx, entry_id, !*pinned) {
                self.set_notice(Some(tr("Clipboard pin limit reached")), NoticeKind::Error);
            }
            return;
        }
        let Some(pin) = pin_for(item) else { return };
        let outcome = ctx.update_settings(|settings| {
            toggle_pin(&mut settings.pins, pin.clone());
            Ok::<(), std::convert::Infallible>(())
        });
        if outcome.is_err() {
            self.fail(tr("Could not save the setting."));
        }
    }

    fn reveal(&self, path: &str, id: &str) {
        if !Path::new(path).exists() {
            self.fail(tr("This item no longer exists."));
            return;
        }
        let uri = gio::File::for_path(path).uri();
        let parent = Path::new(path)
            .parent()
            .map(|p| gio::File::for_path(p).uri().to_string());
        self.accept(id, false);
        let startup_id = String::new();
        glib::spawn_future_local(async move {
            let shown = async {
                let connection = gio::bus_get_future(gio::BusType::Session).await?;
                connection
                    .call_future(
                        Some("org.freedesktop.FileManager1"),
                        "/org/freedesktop/FileManager1",
                        "org.freedesktop.FileManager1",
                        "ShowItems",
                        Some(&(vec![uri.to_string()], startup_id).to_variant()),
                        None,
                        gio::DBusCallFlags::NONE,
                        3000,
                    )
                    .await
                    .map(|_| ())
            }
            .await;
            if shown.is_err()
                && let Some(parent) = parent
                && let Err(error) =
                    gio::AppInfo::launch_default_for_uri(&parent, None::<&gio::AppLaunchContext>)
            {
                eprintln!("kebloop: could not open the containing folder: {error}");
            }
        });
    }

    fn open_with(&self, path: &str) {
        if !Path::new(path).exists() {
            self.fail(tr("This item no longer exists."));
            return;
        }
        let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
        launcher.set_always_ask(true);
        self.set_confirming(true);
        let weak = self.downgrade();
        launcher.launch(
            Some(self.window()),
            None::<&gio::Cancellable>,
            move |result| {
                let Some(launcher) = weak.upgrade() else {
                    return;
                };
                launcher.set_confirming(false);
                match result {
                    Ok(()) => launcher.hide(),
                    Err(error) if error.matches(gtk::DialogError::Dismissed) => {
                        launcher.focus_input();
                    }
                    Err(error) => launcher.fail(error.to_string()),
                }
            },
        );
    }

    // ----- AI handoff -----------------------------------------------------------------------

    /// Open the AI provider with the query (Tab).
    pub(super) fn ask_ai(&self) {
        let Some(ctx) = self.ctx() else { return };
        if self.mode() != Mode::Root {
            return;
        }
        let query = self.query();
        if query.trim().is_empty() {
            return;
        }
        let provider =
            AiProvider::from_stored(ctx.config.borrow().global().ai_provider.raw_value());
        match provider.url(&query) {
            Some(url) => match self.open_uri(&url) {
                Ok(()) => self.hide(),
                Err(error) => self.fail(error),
            },
            None => self.fail(tr("Enter a prompt first.")),
        }
    }

    // ----- commands -------------------------------------------------------------------------

    /// Perform a built-in command from the list.
    pub(super) fn handle_command(&self, command: LauncherCommand, result_id: &str) {
        let Some(ctx) = self.ctx() else { return };
        match command {
            LauncherCommand::Snippets => self.set_mode(Mode::Snippets),
            LauncherCommand::Scripts => self.set_mode(Mode::Scripts),
            LauncherCommand::Extensions => self.set_mode(Mode::Extensions),
            LauncherCommand::Clipboard => self.set_mode(Mode::Clipboard),
            LauncherCommand::Quicklinks => self.set_mode(Mode::Quicklinks),
            LauncherCommand::Emoji => self.set_mode(Mode::Emoji),
            LauncherCommand::Profiles => self.set_mode(Mode::Profiles),
            LauncherCommand::Browse => self.set_mode(Mode::Browse),
            LauncherCommand::WebSearch => self.set_mode(Mode::WebSearch),
            LauncherCommand::Settings => {
                self.accept(result_id, true);
                crate::settings::open_settings(&ctx.app, &ctx, None);
            }
            LauncherCommand::Help => {
                self.accept(result_id, true);
                crate::help::open_help(&ctx.app, &ctx);
            }
            LauncherCommand::Onboarding => {
                self.accept(result_id, true);
                crate::onboarding::open_onboarding(&ctx.app, &ctx);
            }
            LauncherCommand::Shutdown => self.confirm_shutdown(),
            LauncherCommand::AppleShortcuts => {
                self.fail(tr("Apple Shortcuts are not available on Linux."));
            }
            LauncherCommand::CalculationHistory => self.set_mode(Mode::CalculationHistory),
            other => {
                if let Some(action) = system_action_of(other) {
                    self.run_system_command(other, action);
                } else if let Some(action) = window_action_of(other) {
                    self.run_window_command(other, action);
                } else {
                    self.fail(tr("This command is not available on Linux."));
                }
            }
        }
    }

    /// Run a command by id (CLI `command`, global shortcuts, help window). Built-ins open the
    /// launcher in the right mode; user commands run directly.
    pub fn execute_command(&self, id: &str) {
        let Some(ctx) = self.ctx() else { return };
        if !ctx.with_settings(|s| s.personalization.commands_enabled) {
            return;
        }
        let settings = ctx.settings();
        if let Some(script_command) = settings
            .scripts
            .iter()
            .find(|s| s.command_id() == id && s.enabled)
        {
            self.hide();
            script::run(&ctx, script_command, Vec::new());
            return;
        }
        if let Some(manifest) = settings
            .extensions
            .iter()
            .find(|m| m.command_id() == id && m.enabled)
        {
            self.show();
            self.set_mode(Mode::Extension(manifest.id.clone()));
            return;
        }
        if let Some(snippet) = id
            .strip_prefix("snippet:")
            .and_then(|sid| settings.snippets.iter().find(|s| s.id == sid))
        {
            self.show();
            self.begin_snippet(snippet);
            return;
        }
        if let Some(link) = id
            .strip_prefix("link:")
            .and_then(|lid| settings.quicklinks.iter().find(|l| l.id == lid))
        {
            let item = super::model::quicklink_item(link, "", &ctx.l10n, &ctx.home);
            self.show();
            self.when_shown(move |launcher| launcher.perform_item(&item));
            return;
        }
        if is_user_command_id(id) {
            return;
        }
        let Some(command) = LauncherCommand::from_id(match id.trim_start_matches("command:") {
            "welcome" => "onboarding",
            other => other,
        }) else {
            return;
        };
        if !ctx.availability.borrow().offers(command) {
            return;
        }
        // Window arrangement and the system commands without a confirmation act without
        // showing the panel; a failure brings it up.
        let silent = window_action_of(command).is_some()
            || system_action_of(command).is_some_and(|action| !action.needs_confirmation());
        if !silent
            && !matches!(
                command,
                LauncherCommand::Settings | LauncherCommand::Help | LauncherCommand::Onboarding
            )
        {
            self.show();
        }
        self.when_shown(move |launcher| {
            launcher.handle_command(command, &command_result_id(command));
        });
    }

    /// An application shortcut (CLI `launch`, portal): launch the application, bring it to the
    /// front, or hide it when it already has focus.
    pub fn launch_application_id(&self, desktop_id: &str) {
        self.toggle_application(desktop_id);
    }

    /// Launch an application by desktop-file id.
    pub(super) fn start_application(&self, desktop_id: &str) {
        let Some(ctx) = self.ctx() else { return };
        let path = ctx
            .apps
            .borrow()
            .iter()
            .find(|a| a.id == desktop_id)
            .map(|a| a.path.to_string_lossy().into_owned());
        let info = match &path {
            Some(path) => gio_unix::DesktopAppInfo::from_filename(path),
            None if desktop_id.starts_with('/') => {
                gio_unix::DesktopAppInfo::from_filename(desktop_id)
            }
            None => gio_unix::DesktopAppInfo::new(desktop_id),
        };
        let Some(info) = info else {
            eprintln!("kebloop: no such application: {desktop_id}");
            return;
        };
        let context = self.launch_context();
        match info.launch(&[], context.as_ref()) {
            Ok(()) => {
                self.hide();
                let id = desktop_id.to_owned();
                glib::idle_add_local_once(move || ctx.record_use(&id, true));
            }
            Err(error) => eprintln!("kebloop: could not launch {desktop_id}: {error}"),
        }
    }
}

fn app_name(ctx: &AppContext, desktop_id: &str, fallback: &str) -> String {
    ctx.apps
        .borrow()
        .iter()
        .find(|a| a.id == desktop_id)
        .map_or_else(|| fallback.to_owned(), |a| a.name.clone())
}

/// The filesystem path behind a result, for reveal/copy/open-with.
pub(super) fn target_path(target: &Target) -> Option<String> {
    match target {
        Target::File { path, .. } | Target::Application { path, .. } => Some(path.clone()),
        _ => None,
    }
}

/// The favorites entry for a result, if it can be pinned.
pub(super) fn pin_for(item: &ResultItem) -> Option<PinnedResult> {
    let (path, kind) = match &item.target {
        Target::Application { path, .. } => (path.clone(), "application"),
        Target::File { path, .. } => (path.clone(), "file"),
        Target::Quicklink { link, .. } => (link.target.clone(), "quicklink"),
        Target::Command(_) => (String::new(), "command"),
        _ => return None,
    };
    Some(PinnedResult {
        id: item.id.clone(),
        name: item.title.clone(),
        path,
        kind: kind.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::model::IconSpec;
    use kebloop_services::commands::LauncherCommand;

    fn item(target: Target, id: &str) -> ResultItem {
        ResultItem {
            id: id.into(),
            target,
            title: "Name".into(),
            subtitle: String::new(),
            icon: IconSpec::Symbolic("x"),
            action: String::new(),
        }
    }

    #[test]
    fn pins_exist_for_apps_files_links_and_commands_only() {
        let app = item(
            Target::Application {
                desktop_id: "a.desktop".into(),
                path: "/usr/share/applications/a.desktop".into(),
            },
            "a.desktop",
        );
        let pin = pin_for(&app).expect("applications can be pinned");
        assert_eq!(pin.kind, "application");
        assert_eq!(pin.id, "a.desktop");
        let command = item(Target::Command(LauncherCommand::Emoji), "command:emoji");
        assert_eq!(
            pin_for(&command).map(|p| p.kind),
            Some("command".to_owned())
        );
        let calculation = item(
            Target::Calculation {
                result: CalculationResult {
                    expression: "2+2".into(),
                    display: "4".into(),
                    clipboard: "4".into(),
                },
            },
            "calculator",
        );
        assert!(pin_for(&calculation).is_none());
    }

    #[test]
    fn paths_come_from_files_and_applications() {
        let file = Target::File {
            path: "/tmp/x".into(),
            is_dir: false,
        };
        assert_eq!(target_path(&file).as_deref(), Some("/tmp/x"));
        assert_eq!(target_path(&Target::WebSearch { query: "q".into() }), None);
    }
}
