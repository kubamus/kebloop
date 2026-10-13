//! Multi-step flows: snippets, extensions, clipboard confirmations, profile switching, emoji
//! picking and the shutdown confirmation.

use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use kebloop_core::Locale;
use kebloop_services::automation::{AutomationError, AutomationRunner, ExtensionSession};
use kebloop_services::snippets::Snippet;

use super::views::clipboard as clipboard_view;
use super::views::script;
use super::views::snippets::RenderEnv;
use super::{Launcher, Mode, NoticeKind};
use crate::context::Change;
use crate::i18n::tr;

/// How long a snippet waits for the clipboard text before it is treated as empty.
const CLIPBOARD_READ_TIMEOUT: Duration = Duration::from_millis(800);
/// Pause after the last keystroke before an extension search starts.
const EXTENSION_DEBOUNCE: Duration = Duration::from_millis(220);

impl Launcher {
    // ----- emoji ----------------------------------------------------------------------------

    /// Copy `entry`, then paste it into the previous window (or only copy it).
    pub(super) fn pick_emoji(&self, entry: &'static kebloop_core::EmojiEntry, paste: bool) {
        let Some(ctx) = self.ctx() else { return };
        if !crate::services::clipboard::copy_text(&ctx, entry.symbol, false) {
            self.set_notice(
                Some(tr("Could not write to the clipboard.")),
                NoticeKind::Error,
            );
            return;
        }
        if let Err(error) = ctx.config.borrow_mut().record_emoji(entry.symbol) {
            eprintln!("kebloop: could not save emoji recents: {error}");
        }
        if paste {
            self.paste_into_previous();
        } else {
            self.hide();
        }
    }

    // ----- snippets -------------------------------------------------------------------------

    pub(super) fn begin_snippet(&self, snippet: &Snippet) {
        let Some(ctx) = self.ctx() else { return };
        self.set_mode(Mode::Snippet);
        let id = format!("snippet:{}", snippet.id);
        glib::idle_add_local_once(move || ctx.record_use(&id, true));
        let Some(ctx) = self.ctx() else { return };
        let env = RenderEnv {
            locale: Locale::from_tag(ctx.l10n.language().code()),
            time_zone: ctx.time_zone.clone(),
            clipboard: String::new(),
        };
        if !snippet.template.contains("{clipboard}") {
            self.inner.snippet.begin(snippet, env);
            return;
        }
        // `{clipboard}` is the text on the clipboard right now; reading it is asynchronous.
        let snippet = snippet.clone();
        let weak = self.downgrade();
        glib::spawn_future_local(async move {
            let clipboard = read_clipboard_text().await;
            let Some(launcher) = weak.upgrade() else {
                return;
            };
            if launcher.mode() == Mode::Snippet {
                launcher
                    .inner
                    .snippet
                    .begin(&snippet, RenderEnv { clipboard, ..env });
            }
        });
    }

    pub(super) fn finish_snippet(&self, text: &str, insert: bool) {
        let Some(ctx) = self.ctx() else { return };
        if !crate::services::clipboard::copy_text(&ctx, text, false) {
            self.set_notice(
                Some(tr("Could not write to the clipboard.")),
                NoticeKind::Error,
            );
            return;
        }
        if insert {
            self.paste_into_previous();
        } else {
            self.hide();
        }
    }

    // ----- scripts --------------------------------------------------------------------------

    pub(super) fn ask_script_arguments(
        &self,
        script_command: &kebloop_services::automation::ScriptCommand,
    ) {
        let Some(ctx) = self.ctx() else { return };
        let dialog = adw::AlertDialog::new(
            Some(&script_command.title),
            Some(&tr("Additional literal arguments, one per line")),
        );
        let text = gtk::TextView::builder()
            .monospace(true)
            .wrap_mode(gtk::WrapMode::None)
            .build();
        text.update_property(&[gtk::accessible::Property::Label(&tr(
            "Script arguments, one per line",
        ))]);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&text)
            .min_content_height(90)
            .min_content_width(320)
            .build();
        scroll.add_css_class("card");
        dialog.set_extra_child(Some(&scroll));
        dialog.add_responses(&[("cancel", &tr("Cancel")), ("run", &tr("Run"))]);
        dialog.set_response_appearance("run", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        self.set_confirming(true);
        let weak = self.downgrade();
        let script_command = script_command.clone();
        dialog.choose(
            Some(self.window()),
            None::<&gtk::gio::Cancellable>,
            move |response| {
                let Some(launcher) = weak.upgrade() else {
                    return;
                };
                launcher.set_confirming(false);
                if response == "run" {
                    let buffer = text.buffer();
                    let content = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
                    let arguments: Vec<String> = content
                        .lines()
                        .filter(|line| !line.is_empty())
                        .map(str::to_owned)
                        .collect();
                    launcher.hide();
                    script::run(&ctx, &script_command, arguments);
                } else {
                    launcher.focus_input();
                }
            },
        );
    }

    // ----- clipboard confirmations ----------------------------------------------------------

    pub(super) fn remove_selected_clipboard_item(&self) {
        if self.mode() != Mode::Clipboard {
            return;
        }
        let Some(ctx) = self.ctx() else { return };
        if let Some(super::model::ResultItem {
            target: super::model::Target::Clipboard { entry_id, .. },
            ..
        }) = self.selected_item()
        {
            clipboard_view::remove_entry(&ctx, &entry_id);
        }
    }

    pub(super) fn confirm_clear_clipboard(&self) {
        let Some(ctx) = self.ctx() else { return };
        if self.mode() != Mode::Clipboard || ctx.clipboard.borrow().items().is_empty() {
            return;
        }
        let dialog = adw::AlertDialog::new(
            Some(&tr("Clear clipboard history?")),
            Some(&tr(
                "All saved clipboard items, including pinned items and attachments, will be removed.",
            )),
        );
        dialog.add_responses(&[("cancel", &tr("Cancel")), ("clear", &tr("Clear History"))]);
        dialog.set_response_appearance("clear", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        self.set_confirming(true);
        let weak = self.downgrade();
        dialog.choose(
            Some(self.window()),
            None::<&gtk::gio::Cancellable>,
            move |response| {
                let Some(launcher) = weak.upgrade() else {
                    return;
                };
                launcher.set_confirming(false);
                if response == "clear" {
                    clipboard_view::clear_all(&ctx);
                    launcher.inner.list.clear_thumbnails();
                    launcher.inner.thumbnails_requested.borrow_mut().clear();
                }
                launcher.focus_input();
            },
        );
    }

    // ----- profiles -------------------------------------------------------------------------

    pub(super) fn switch_profile(&self, id: &str) {
        let Some(ctx) = self.ctx() else { return };
        let outcome = ctx
            .config
            .borrow_mut()
            .switch_profile_with(id, &mut |_| Ok(()));
        match outcome {
            Ok(()) => {
                ctx.emit(Change::Profile);
                self.refresh(true);
            }
            Err(error) => self.set_notice(Some(error.to_string()), NoticeKind::Error),
        }
    }

    // ----- shutdown -------------------------------------------------------------------------

    pub(super) fn confirm_shutdown(&self) {
        self.confirm(
            &tr("Shut down this computer?"),
            &tr("Unsaved work in other applications may be lost. Your system may ask you to confirm."),
            &tr("Shut Down"),
            |launcher| {
                launcher.hide();
                let weak = launcher.downgrade();
                crate::work::spawn(
                    "kebloop-power-off",
                    crate::services::system::power_off,
                    move |result| {
                        if let (Some(launcher), Err(error)) = (weak.upgrade(), result) {
                            launcher.show();
                            launcher.set_notice(
                                Some(crate::i18n::trf(
                                    "Couldn’t shut down: %@",
                                    &[kebloop_services::l10n::FormatArg::Str(error)],
                                )),
                                NoticeKind::Error,
                            );
                        }
                    },
                );
            },
        );
    }

    // ----- calculator history ---------------------------------------------------------------

    pub(super) fn confirm_clear_calculation_history(&self) {
        let Some(ctx) = self.ctx() else { return };
        self.confirm(
            &tr("Clear calculator history?"),
            &tr("All accepted calculations will be removed."),
            &tr("Clear History"),
            move |launcher| {
                ctx.calculation_history.borrow_mut().clear();
                let error = ctx
                    .calculation_history
                    .borrow()
                    .last_error()
                    .map(str::to_owned);
                launcher.refresh(false);
                if let Some(error) = error {
                    launcher.set_notice(Some(error), NoticeKind::Error);
                }
            },
        );
    }

    // ----- extensions -----------------------------------------------------------------------

    pub(super) fn cancel_extension(&self) {
        let mut run = self.inner.extension_run.borrow_mut();
        run.generation += 1;
        if let Some(timer) = run.timer.take() {
            timer.remove();
        }
        if let Some(runner) = run.runner.take() {
            runner.cancel();
        }
        if let (Some(token), Some(ctx)) = (run.token.take(), self.ctx()) {
            ctx.finish_run(token);
        }
    }

    pub(super) fn begin_extension(&self, manifest_id: &str) {
        let Some(ctx) = self.ctx() else { return };
        let manifest = ctx.with_settings(|s| {
            s.extensions
                .iter()
                .find(|m| m.id == manifest_id && m.enabled)
                .cloned()
        });
        let Some(manifest) = manifest else {
            self.set_mode(Mode::Root);
            self.set_notice(
                Some(tr("Review and enable this extension in Settings first.")),
                NoticeKind::Error,
            );
            return;
        };
        {
            let mut run = self.inner.extension_run.borrow_mut();
            run.manifest = Some(manifest);
            run.session = ExtensionSession::new();
        }
        self.inner.extension.clear();
        self.start_extension_search(String::new());
    }

    pub(super) fn schedule_extension_search(&self) {
        {
            let mut run = self.inner.extension_run.borrow_mut();
            if let Some(timer) = run.timer.take() {
                timer.remove();
            }
        }
        let weak = self.downgrade();
        let timer = glib::timeout_add_local_once(EXTENSION_DEBOUNCE, move || {
            let Some(launcher) = weak.upgrade() else {
                return;
            };
            launcher.inner.extension_run.borrow_mut().timer = None;
            let query = launcher.query();
            launcher.start_extension_search(query);
        });
        self.inner.extension_run.borrow_mut().timer = Some(timer);
    }

    fn start_extension_search(&self, query: String) {
        self.run_extension(move |session, manifest, preferences, runner| {
            session.search(manifest, &query, preferences, runner)
        });
    }

    pub(super) fn run_extension_action(&self, result_id: &str, action_id: &str) {
        let (result_id, action_id) = (result_id.to_owned(), action_id.to_owned());
        self.run_extension(move |session, manifest, preferences, runner| {
            session.perform(manifest, &result_id, &action_id, preferences, runner)
        });
    }

    /// Run an extension operation on a worker thread with a copy of the session; the outcome is
    /// applied only if no newer operation started meanwhile.
    fn run_extension<F>(&self, operation: F)
    where
        F: FnOnce(
                &mut ExtensionSession,
                &kebloop_services::automation::ExtensionManifest,
                &std::collections::BTreeMap<String, String>,
                &AutomationRunner,
            ) -> Result<(), AutomationError>
            + Send
            + 'static,
    {
        let Some(ctx) = self.ctx() else { return };
        let (manifest, mut session, generation) = {
            let mut run = self.inner.extension_run.borrow_mut();
            let Some(manifest) = run.manifest.clone() else {
                return;
            };
            if let Some(runner) = run.runner.take() {
                runner.cancel();
            }
            if let Some(token) = run.token.take() {
                ctx.finish_run(token);
            }
            run.generation += 1;
            let runner = AutomationRunner::new();
            run.token = Some(ctx.register_run(runner.cancel_handle()));
            run.runner = Some(runner);
            (manifest, run.session.clone(), run.generation)
        };
        let runner = self
            .inner
            .extension_run
            .borrow()
            .runner
            .clone()
            .unwrap_or_default();
        let preferences = ctx
            .config
            .borrow()
            .global()
            .extension_preferences
            .get(&manifest.id)
            .cloned()
            .unwrap_or_default();
        self.inner
            .extension
            .show_status(Some(&tr("Running…")), true);
        let (sender, receiver) =
            async_channel::bounded::<(ExtensionSession, Result<(), AutomationError>)>(1);
        let spawned = std::thread::Builder::new()
            .name("kebloop-extension".into())
            .spawn(move || {
                let outcome = operation(&mut session, &manifest, &preferences, &runner);
                let _ = sender.send_blocking((session, outcome));
            });
        if spawned.is_err() {
            self.inner
                .extension
                .show_status(Some(&tr("Could not start the extension.")), false);
            return;
        }
        let weak = self.downgrade();
        glib::spawn_future_local(async move {
            let Ok((session, outcome)) = receiver.recv().await else {
                return;
            };
            let Some(launcher) = weak.upgrade() else {
                return;
            };
            let ctx = launcher.ctx();
            {
                let mut run = launcher.inner.extension_run.borrow_mut();
                if run.generation != generation {
                    return;
                }
                run.session = session.clone();
                run.runner = None;
                if let (Some(token), Some(ctx)) = (run.token.take(), ctx.as_ref()) {
                    ctx.finish_run(token);
                }
            }
            let extension = &launcher.inner.extension;
            let previous = extension.selected_result().map(|r| r.id);
            match outcome {
                Ok(()) => {
                    extension.set_results(session.results(), previous.as_deref());
                    extension.show_status(session.message(), false);
                }
                Err(AutomationError::Cancelled) => {}
                Err(error) => {
                    extension.set_results(&[], None);
                    extension.show_status(Some(&error.to_string()), false);
                }
            }
            launcher.update_chrome();
        });
    }
}

/// The text on the clipboard, or an empty string when there is none or it does not answer in
/// time.
async fn read_clipboard_text() -> String {
    let Some(display) = gtk::gdk::Display::default() else {
        return String::new();
    };
    let read = async {
        display
            .clipboard()
            .read_text_future()
            .await
            .ok()
            .flatten()
            .map(|text| text.to_string())
            .unwrap_or_default()
    };
    let timeout = async {
        glib::timeout_future(CLIPBOARD_READ_TIMEOUT).await;
        String::new()
    };
    futures_lite::future::or(read, timeout).await
}
