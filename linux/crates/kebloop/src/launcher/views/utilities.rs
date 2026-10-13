//! Keyboard-accessible inline dictionary, dictation and local calendar pages.
use crate::{context::AppContext, i18n::tr};
use gtk::{gdk, gio, prelude::*};
use kebloop_services::{
    automation::AutomationRunner,
    calendar::{self, CalendarEvent, CalendarSettings},
    voice_dictionary::{self, DictationControl, DictationTools},
};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UtilityKind {
    Dictionary,
    Dictation,
    Calendar,
    NextMeeting,
}
#[derive(Clone)]
pub struct UtilityPage(Rc<Inner>);
struct Inner {
    ctx: Weak<AppContext>,
    root: gtk::Box,
    output: gtk::Label,
    actions: gtk::Box,
    status: gtk::Label,
    kind: Cell<UtilityKind>,
    generation: Cell<u64>,
    runner: RefCell<Option<AutomationRunner>>,
    dictation: RefCell<Option<DictationControl>>,
    dialog: RefCell<Option<gio::Cancellable>>,
    model: RefCell<Option<std::path::PathBuf>>,
    duration: gtk::SpinButton,
    events: RefCell<Vec<CalendarEvent>>,
}
impl UtilityPage {
    pub fn new(ctx: &Rc<AppContext>) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        for set in [
            gtk::Widget::set_margin_start,
            gtk::Widget::set_margin_end,
            gtk::Widget::set_margin_top,
            gtk::Widget::set_margin_bottom,
        ] {
            set(root.upcast_ref(), 16);
        }
        let status = gtk::Label::builder().xalign(0.0).wrap(true).build();
        let output = gtk::Label::builder()
            .xalign(0.0)
            .yalign(0.0)
            .wrap(true)
            .selectable(true)
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .child(&output)
            .min_content_height(120)
            .max_content_height(280)
            .propagate_natural_height(true)
            .build();
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let duration = gtk::SpinButton::with_range(1.0, 120.0, 1.0);
        duration.set_value(10.0);
        root.append(&status);
        root.append(&scroll);
        root.append(&actions);
        Self(Rc::new(Inner {
            ctx: Rc::downgrade(ctx),
            root,
            output,
            actions,
            status,
            kind: Cell::new(UtilityKind::Dictionary),
            generation: Cell::new(0),
            runner: RefCell::new(None),
            dictation: RefCell::new(None),
            dialog: RefCell::new(None),
            model: RefCell::new(None),
            duration,
            events: RefCell::new(Vec::new()),
        }))
    }
    pub fn widget(&self) -> &gtk::Box {
        &self.0.root
    }
    pub fn cancel(&self) {
        self.0.generation.set(self.0.generation.get() + 1);
        if let Some(runner) = self.0.runner.borrow_mut().take() {
            runner.cancel();
        }
        if let Some(control) = self.0.dictation.borrow_mut().take() {
            control.cancel();
        }
        let dialog = self.0.dialog.borrow_mut().take();
        if let Some(dialog) = dialog {
            dialog.cancel();
            self.set_confirming(false);
        }
    }
    pub fn begin(&self, kind: UtilityKind) {
        self.cancel();
        self.0.kind.set(kind);
        self.0.output.set_text("");
        while let Some(child) = self.0.actions.first_child() {
            self.0.actions.remove(&child);
        }
        match kind {
            UtilityKind::Dictionary => {
                self.0.status.set_text(&tr("Enter a word above and press Return. Definitions use local StarDict dictionaries (sdcv)."));
                self.button("Copy", |p| p.copy());
            }
            UtilityKind::Dictation => {
                self.0.status.set_text(&tr("Choose a local whisper.cpp model, then press Ctrl+Return to record or stop and transcribe. Audio stays on this device. Esc cancels."));
                self.button("Choose Model…", |p| p.choose_model());
                self.0.actions.append(&self.0.duration);
                self.0
                    .duration
                    .set_tooltip_text(Some(&tr("Recording duration in seconds")));
                self.button("Record / Stop", |p| p.record());
                self.button("Cancel", |p| {
                    p.cancel();
                    p.0.status.set_text(&tr("Recording cancelled"));
                });
                self.button("Copy", |p| p.copy());
                self.button("Insert", |p| p.insert());
                if self.0.model.borrow().is_none()
                    && let Some(ctx) = self.0.ctx.upgrade()
                {
                    let path = ctx.xdg.app_data_dir().join("dictation-model.txt");
                    let weak = Rc::downgrade(&self.0);
                    let generation = self.0.generation.get();
                    crate::work::spawn(
                        "load-dictation-model",
                        move || voice_dictionary::load_model_path(&path),
                        move |result| {
                            if let Some(inner) = weak.upgrade() {
                                if inner.generation.get() != generation
                                    || inner.model.borrow().is_some()
                                {
                                    return;
                                }
                                match result {
                                    Ok(path) => {
                                        inner.model.replace(path);
                                    }
                                    Err(error) => inner.status.set_text(&error),
                                }
                            }
                        },
                    );
                }
            }
            UtilityKind::Calendar | UtilityKind::NextMeeting => {
                self.button("Choose Calendars…", |p| p.choose_calendars());
                self.button("Refresh", |p| p.load_calendar());
                self.button("Join Next Meeting", |p| p.join());
                self.load_calendar();
            }
        }
    }
    fn button(&self, title: &str, callback: impl Fn(&Self) + 'static) {
        let b = gtk::Button::with_label(&tr(title));
        let weak = Rc::downgrade(&self.0);
        b.connect_clicked(move |_| {
            if let Some(inner) = weak.upgrade() {
                callback(&Self(inner));
            }
        });
        self.0.actions.append(&b);
    }
    pub fn activate(&self, query: &str) {
        match self.0.kind.get() {
            UtilityKind::Dictionary => self.lookup(query),
            UtilityKind::Dictation => self.record(),
            _ => self.join(),
        }
    }
    fn copy(&self) {
        let Some(ctx) = self.0.ctx.upgrade() else {
            return;
        };
        let text = self.0.output.text();
        if text.is_empty() {
            return;
        }
        if crate::services::clipboard::copy_text(&ctx, &text, false) {
            self.0.status.set_text(&tr("Copied"));
        } else {
            self.0
                .status
                .set_text(&tr("Could not write to the clipboard."));
        }
    }
    fn insert(&self) {
        let Some(ctx) = self.0.ctx.upgrade() else {
            return;
        };
        let text = self.0.output.text();
        if text.is_empty() {
            return;
        }
        if crate::services::clipboard::copy_text(&ctx, &text, false) {
            if let Some(launcher) = ctx.launcher.borrow().clone() {
                launcher.paste_into_previous();
            }
        } else {
            self.0
                .status
                .set_text(&tr("Could not write to the clipboard."));
        }
    }

    fn set_confirming(&self, confirming: bool) {
        if let Some(ctx) = self.0.ctx.upgrade()
            && let Some(launcher) = ctx.launcher.borrow().as_ref()
        {
            launcher.set_confirming(confirming);
        }
    }
    fn begin_dialog(&self) -> Option<gio::Cancellable> {
        if self.0.dialog.borrow().is_some() {
            return None;
        }
        let dialog = gio::Cancellable::new();
        self.0.dialog.replace(Some(dialog.clone()));
        self.set_confirming(true);
        Some(dialog)
    }
    fn finish_dialog(&self, dialog: &gio::Cancellable) -> bool {
        if self.0.dialog.borrow().as_ref() != Some(dialog) {
            return false;
        }
        self.0.dialog.borrow_mut().take();
        self.set_confirming(false);
        true
    }
    fn lookup(&self, query: &str) {
        self.cancel();
        let Some(ctx) = self.0.ctx.upgrade() else {
            return;
        };
        let generation = self.0.generation.get();
        let runner = AutomationRunner::new();
        let token = ctx.register_run(runner.cancel_handle());
        self.0.runner.replace(Some(runner.clone()));
        let query = query.to_owned();
        let weak = Rc::downgrade(&self.0);
        let weak_ctx = Rc::downgrade(&ctx);
        self.0.output.set_text("");
        self.0.status.set_text(&tr("Looking up…"));
        let started = crate::work::spawn(
            "dictionary",
            move || voice_dictionary::lookup(&query, &runner),
            move |result| {
                if let Some(ctx) = weak_ctx.upgrade() {
                    ctx.finish_run(token);
                }
                if let Some(inner) = weak.upgrade() {
                    if inner.generation.get() != generation {
                        return;
                    }
                    inner.runner.borrow_mut().take();
                    match result {
                        Ok(text) => {
                            inner.output.set_text(&text);
                            inner.status.set_text(&tr("Return looks up another word · Ctrl+C copies selected definition text"));
                        }
                        Err(error) => inner.status.set_text(&error),
                    }
                }
            },
        );
        if !started {
            ctx.finish_run(token);
            self.0.runner.borrow_mut().take();
            self.0
                .status
                .set_text(&tr("Could not start the background operation."));
        }
    }
    fn choose_model(&self) {
        let Some(cancellable) = self.begin_dialog() else {
            return;
        };
        let dialog = gtk::FileDialog::builder()
            .title(tr("Choose whisper.cpp model"))
            .build();
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("whisper.cpp models (*.bin)"));
        filter.add_pattern("*.bin");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        dialog.set_filters(Some(&filters));
        let parent = self
            .0
            .root
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok());
        let weak = Rc::downgrade(&self.0);
        let generation = self.0.generation.get();
        let completed = cancellable.clone();
        dialog.open(parent.as_ref(), Some(&cancellable), move |result| {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let page = Self(inner.clone());
            if !page.finish_dialog(&completed) || inner.generation.get() != generation {
                return;
            }
            let Ok(file) = result else {
                return;
            };
            let Some(path) = file.path() else {
                inner.status.set_text(&tr("Choose a local file."));
                return;
            };
            let Some(ctx) = inner.ctx.upgrade() else {
                return;
            };
            let settings_path = ctx.xdg.app_data_dir().join("dictation-model.txt");
            // Mark the chosen path immediately so an older asynchronous settings read
            // cannot overwrite this selection while persistence finishes.
            inner.model.replace(Some(path.clone()));
            let weak = Rc::downgrade(&inner);
            let started = crate::work::spawn(
                "save-dictation-model",
                move || {
                    if !path.is_file() {
                        return Err("Choose an existing local model file".to_owned());
                    }
                    kebloop_services::util::atomic_write(
                        &settings_path,
                        path.to_string_lossy().as_bytes(),
                        0o600,
                    )
                    .map_err(|e| e.to_string())?;
                    Ok(path)
                },
                move |result| {
                    if let Some(inner) = weak.upgrade() {
                        if inner.generation.get() != generation {
                            return;
                        }
                        match result {
                            Ok(path) => {
                                inner.model.replace(Some(path));
                                inner
                                    .status
                                    .set_text(&tr("Model selected. Ctrl+Return starts recording."));
                            }
                            Err(error) => inner.status.set_text(&error),
                        }
                    }
                },
            );
            if !started {
                inner
                    .status
                    .set_text(&tr("Could not start the background operation."));
            }
        });
    }
    fn record(&self) {
        if let Some(control) = self.0.dictation.borrow().as_ref() {
            if control.stop_recording() {
                self.0.status.set_text(&tr("Transcribing on this device…"));
            } else {
                self.0
                    .status
                    .set_text(&tr("Transcribing on this device… Use Cancel to discard."));
            }
            return;
        }
        let Some(ctx) = self.0.ctx.upgrade() else {
            return;
        };
        let Some(model) = self.0.model.borrow().clone() else {
            self.choose_model();
            return;
        };
        let tools = match DictationTools::discover(&model) {
            Ok(tools) => tools,
            Err(error) => {
                self.0.status.set_text(&error);
                return;
            }
        };
        let generation = self.0.generation.get();
        let control = DictationControl::new();
        let tokens = control
            .cancel_handles()
            .map(|handle| ctx.register_run(handle));
        self.0.dictation.replace(Some(control.clone()));
        let directory = ctx.xdg.app_cache_dir().join("dictation");
        let seconds = self.0.duration.value_as_int() as u32;
        let weak = Rc::downgrade(&self.0);
        let weak_ctx = Rc::downgrade(&ctx);
        self.0.output.set_text("");
        self.0.status.set_text(&tr(
            "Recording… Ctrl+Return stops and transcribes. Esc cancels.",
        ));
        let phase_control = control.clone();
        let phase_weak = Rc::downgrade(&self.0);
        gtk::glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            let Some(inner) = phase_weak.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            if inner.generation.get() != generation || inner.dictation.borrow().is_none() {
                return gtk::glib::ControlFlow::Break;
            }
            if !phase_control.is_recording() {
                inner.status.set_text(&tr("Transcribing on this device…"));
                return gtk::glib::ControlFlow::Break;
            }
            gtk::glib::ControlFlow::Continue
        });
        let started = crate::work::spawn(
            "dictation",
            move || tools.transcribe_controlled(&control, &directory, seconds),
            move |result| {
                if let Some(ctx) = weak_ctx.upgrade() {
                    for token in tokens {
                        ctx.finish_run(token);
                    }
                }
                if let Some(inner) = weak.upgrade() {
                    if inner.generation.get() != generation {
                        return;
                    }
                    inner.dictation.borrow_mut().take();
                    match result {
                        Ok(text) => {
                            inner.output.set_text(&text);
                            inner.status.set_text(&tr(
                                "Transcript ready. Select text to copy, or use Copy.",
                            ));
                        }
                        Err(error) => inner.status.set_text(&error),
                    }
                }
            },
        );
        if !started {
            for token in tokens {
                ctx.finish_run(token);
            }
            self.0.dictation.borrow_mut().take();
            self.0
                .status
                .set_text(&tr("Could not start the background operation."));
        }
    }
    fn choose_calendars(&self) {
        let Some(cancellable) = self.begin_dialog() else {
            return;
        };
        let dialog = gtk::FileDialog::builder()
            .title(tr("Choose local .ics calendars"))
            .build();
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("iCalendar files (*.ics)"));
        filter.add_pattern("*.ics");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        dialog.set_filters(Some(&filters));
        let parent = self
            .0
            .root
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok());
        let weak = Rc::downgrade(&self.0);
        let generation = self.0.generation.get();
        let completed = cancellable.clone();
        dialog.open_multiple(parent.as_ref(), Some(&cancellable), move |result| {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let page = Self(inner.clone());
            if !page.finish_dialog(&completed) || inner.generation.get() != generation {
                return;
            }
            let Ok(files) = result else {
                return;
            };
            let Some(ctx) = inner.ctx.upgrade() else {
                return;
            };
            if files.n_items() > 32 {
                inner
                    .status
                    .set_text(&tr("Choose at most 32 calendar files."));
                return;
            }
            let files = (0..files.n_items())
                .filter_map(|i| files.item(i).and_downcast::<gio::File>())
                .filter_map(|file| file.path())
                .map(|path| path.to_string_lossy().into_owned())
                .collect();
            let settings = CalendarSettings { files };
            let path = ctx.xdg.app_data_dir().join("calendars.json");
            let weak = Rc::downgrade(&inner);
            let started = crate::work::spawn(
                "save-calendars",
                move || settings.save(&path),
                move |result| {
                    if let Some(inner) = weak.upgrade() {
                        if inner.generation.get() != generation {
                            return;
                        }
                        match result {
                            Ok(()) => Self(inner).load_calendar(),
                            Err(error) => inner.status.set_text(&error.to_string()),
                        }
                    }
                },
            );
            if !started {
                inner
                    .status
                    .set_text(&tr("Could not start the background operation."));
            }
        });
    }
    fn load_calendar(&self) {
        self.cancel();
        self.0.events.borrow_mut().clear();
        self.0.output.set_text("");
        let Some(ctx) = self.0.ctx.upgrade() else {
            return;
        };
        let path = ctx.xdg.app_data_dir().join("calendars.json");
        let zone = ctx.time_zone.clone();
        let generation = self.0.generation.get();
        let weak = Rc::downgrade(&self.0);
        self.0.status.set_text(&tr("Loading selected calendars…"));
        let started = crate::work::spawn(
            "calendar",
            move || {
                let settings = CalendarSettings::load(&path).map_err(|e| e.to_string())?;
                if settings.files.is_empty() {
                    return Err(
                        "Choose local .ics calendars to enable events and meeting commands.".into(),
                    );
                }
                calendar::read_events(&settings, jiff::Timestamp::now(), &zone)
            },
            move |result| {
                if let Some(inner) = weak.upgrade() {
                    if inner.generation.get() != generation {
                        return;
                    }
                    match result {
                        Ok(events) => {
                            inner.output.set_text(
                                &events
                                    .iter()
                                    .map(|e| {
                                        format!(
                                            "{}\n{}{}",
                                            e.title,
                                            e.start,
                                            if e.meeting_url.is_some() {
                                                " · Join available"
                                            } else {
                                                ""
                                            }
                                        )
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n\n"),
                            );
                            inner.events.replace(events);
                            inner.status.set_text(&tr(
                                "Upcoming events · Return joins the next available meeting",
                            ));
                        }
                        Err(e) => {
                            inner.events.borrow_mut().clear();
                            inner.status.set_text(&e);
                        }
                    }
                }
            },
        );
        if !started {
            self.0
                .status
                .set_text(&tr("Could not start the background operation."));
        }
    }
    fn join(&self) {
        let now = jiff::Timestamp::now();
        let url = self
            .0
            .events
            .borrow()
            .iter()
            .find(|e| e.end > now && e.meeting_url.is_some())
            .and_then(|e| e.meeting_url.clone());
        match url {
            Some(url) => {
                if let Err(e) =
                    gio::AppInfo::launch_default_for_uri(&url, None::<&gio::AppLaunchContext>)
                {
                    self.0.status.set_text(&e.to_string());
                }
            }
            None => self.0.status.set_text(&tr("No upcoming meeting link")),
        }
    }
    pub fn key(&self, key: gdk::Key, state: gdk::ModifierType, query: &str) -> bool {
        if self.0.kind.get() == UtilityKind::Dictation
            && state.contains(gdk::ModifierType::CONTROL_MASK)
            && state.contains(gdk::ModifierType::SHIFT_MASK)
            && matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
        {
            self.insert();
            return true;
        }
        if state.contains(gdk::ModifierType::CONTROL_MASK)
            && matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
        {
            self.activate(query);
            return true;
        }
        false
    }
}
