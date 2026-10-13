//! The launcher panel: header with the search field, a stack of views (result list, emoji grid,
//! snippet form, extension browser) and the action menu. One instance is built at startup and
//! reused for every opening.

pub mod actions;
mod control;
mod exec;
pub mod exists;
mod flows;
pub mod keys;
pub mod model;
pub mod results;
pub mod views;
mod window;
pub(crate) use window::layer_shell_supported;

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gdk, glib};
use kebloop_core::{CalculationOutcome, CalculatorEngine, Locale};
use kebloop_platform::compositor::Window;
use kebloop_platform::files::FileMatch;
use kebloop_services::automation::{AutomationRunner, ExtensionManifest, ExtensionSession};
use kebloop_services::file_search::{
    DEBOUNCE_MS, FileRanker, FileSearchCache, FileSearchResult, MAX_RESULTS,
};

use self::actions::{ActionContext, ActionMenu, ResultAction, actions_for};
use self::keys::{Key, KeyAction, KeyState, Mods, ViewKind};
use self::model::{ResultItem, RootInputs, Target};
use self::results::{ResultList, RowStyle};
use self::views::clipboard as clipboard_view;
use self::views::emoji::EmojiView;
use self::views::extension::ExtensionPage;
use self::views::snippets::SnippetPage;
use self::window::PanelWindow;
use crate::context::{AppContext, Change};
use crate::i18n::tr;
use crate::style::Metrics;

/// What the panel is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Root search.
    Root,
    /// Clipboard history.
    Clipboard,
    /// Quicklinks.
    Quicklinks,
    /// Snippet chooser.
    Snippets,
    /// Web search.
    WebSearch,
    /// Profile switcher.
    Profiles,
    /// Browse commands.
    Browse,
    /// Accepted calculator results.
    CalculationHistory,
    /// Script commands.
    Scripts,
    /// Extension chooser.
    Extensions,
    /// Emoji grid.
    Emoji,
    /// Snippet form/preview.
    Snippet,
    /// One extension's browser (manifest id).
    Extension(String),
}

impl Mode {
    fn view_kind(&self) -> ViewKind {
        match self {
            Mode::Root => ViewKind::Root,
            Mode::Clipboard => ViewKind::Clipboard,
            Mode::Emoji => ViewKind::Emoji,
            Mode::Snippet => ViewKind::Form,
            _ => ViewKind::List,
        }
    }

    fn title(&self) -> String {
        match self {
            Mode::Root => tr("Results"),
            Mode::Clipboard => tr("Clipboard History"),
            Mode::Quicklinks => tr("Quicklinks"),
            Mode::Snippets | Mode::Snippet => tr("Snippets"),
            Mode::WebSearch => tr("Web Search"),
            Mode::Profiles => tr("Profiles"),
            Mode::Browse => tr("Browse Commands"),
            Mode::CalculationHistory => tr("Calculator History"),
            Mode::Scripts => tr("Script Commands"),
            Mode::Extensions | Mode::Extension(_) => tr("Extensions"),
            Mode::Emoji => tr("Emoji & Symbols"),
        }
    }

    fn placeholder(&self) -> String {
        match self {
            Mode::Root => tr("Search apps, links, or commands…"),
            Mode::Clipboard => tr("Search clipboard history…"),
            Mode::Quicklinks => tr("Search quicklinks…"),
            Mode::Snippets | Mode::Snippet => tr("Search snippets…"),
            Mode::WebSearch => tr("Search the web…"),
            Mode::Profiles => tr("Search profiles…"),
            Mode::Browse => tr("Search commands and help"),
            Mode::CalculationHistory => tr("Search calculation history…"),
            Mode::Scripts => tr("Search script commands…"),
            Mode::Extensions => tr("Search extensions…"),
            Mode::Extension(_) => tr("Search extension"),
            Mode::Emoji => tr("Search emoji"),
        }
    }

    fn page(&self) -> &'static str {
        match self {
            Mode::Emoji => "emoji",
            Mode::Snippet => "snippet",
            Mode::Extension(_) => "extension",
            _ => "list",
        }
    }
}

/// Kind of the line under the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    /// An error.
    Error,
    /// Information.
    Info,
}

struct State {
    mode: Mode,
    items: Vec<ResultItem>,
    notice: Option<(String, NoticeKind)>,
}

struct FileState {
    results: Vec<FileSearchResult>,
    directories: HashSet<String>,
    cache: FileSearchCache,
    ranker: FileRanker,
    generation: u64,
    timer: Option<glib::SourceId>,
    sender: async_channel::Sender<(u64, Vec<FileMatch>)>,
}

#[derive(Default)]
struct ExtensionRun {
    manifest: Option<ExtensionManifest>,
    session: ExtensionSession,
    runner: Option<AutomationRunner>,
    token: Option<u64>,
    generation: u64,
    timer: Option<glib::SourceId>,
}

type ShowCallback = Box<dyn FnOnce(&Launcher)>;

struct Inner {
    ctx: Weak<AppContext>,
    panel: PanelWindow,
    panel_box: gtk::Box,
    input: gtk::Text,
    back: gtk::Button,
    search_icon: gtk::Image,
    category_title: gtk::Label,
    clear_button: gtk::Button,
    ai_button: gtk::Button,
    actions_button: gtk::Button,
    stack: gtk::Stack,
    list: ResultList,
    emoji: EmojiView,
    snippet: SnippetPage,
    extension: ExtensionPage,
    message: gtk::Label,
    actions: ActionMenu,
    state: RefCell<State>,
    calculator: CalculatorEngine,
    files: RefCell<FileState>,
    extension_run: RefCell<ExtensionRun>,
    thumbnails_requested: RefCell<HashSet<String>>,
    /// Existence of pinned files, answered from memory (see [`exists`]).
    exists: Arc<exists::ExistsCache>,
    exists_kick: async_channel::Sender<()>,
    suppress_changed: Cell<u32>,
    /// The window that had focus before the launcher opened (the target of paste and of the
    /// window commands); never a Kebloop window.
    previous: RefCell<Option<Window>>,
    /// Counts the openings of the panel, so late answers for an earlier one are ignored.
    revision: Cell<u64>,
    /// An opening is waiting for the compositor snapshot before taking focus.
    opening: Cell<bool>,
    /// Commands that need the panel to have finished opening.
    after_show: RefCell<Vec<ShowCallback>>,
    /// Cancels a paste that is still waiting for focus when the launcher opens again.
    paste_cancel: RefCell<Arc<AtomicBool>>,
    /// The input method is composing text (non-empty preedit string).
    preedit: Cell<bool>,
    confirming: Cell<bool>,
    /// The user moved the selection since the last full refresh.
    selection_touched: Cell<bool>,
    was_active: Cell<bool>,
    metrics: Cell<Metrics>,
}

/// Handle to the launcher panel (cheap to clone).
#[derive(Clone)]
pub struct Launcher {
    inner: Rc<Inner>,
}

struct WeakLauncher(Weak<Inner>);

impl WeakLauncher {
    fn upgrade(&self) -> Option<Launcher> {
        self.0.upgrade().map(|inner| Launcher { inner })
    }
}

impl Launcher {
    fn downgrade(&self) -> WeakLauncher {
        WeakLauncher(Rc::downgrade(&self.inner))
    }

    fn ctx(&self) -> Option<Rc<AppContext>> {
        self.inner.ctx.upgrade()
    }

    /// Build the panel (hidden). GTK must be initialised.
    pub fn new(ctx: &Rc<AppContext>) -> Self {
        let settings = ctx.settings();
        let metrics = Metrics::from_settings(&settings.personalization);
        let fixed_output = fixed_output(&settings.personalization);
        let panel = PanelWindow::new(&ctx.app, fixed_output);

        let search_icon = gtk::Image::builder()
            .icon_name("system-search-symbolic")
            .pixel_size(16)
            .css_classes(["kb-search-icon"])
            .build();
        let back = gtk::Button::builder()
            .icon_name("go-previous-symbolic")
            .css_classes(["flat", "kb-back"])
            .tooltip_text(tr("Back to launcher"))
            .focusable(false)
            .visible(false)
            .build();
        back.update_property(&[gtk::accessible::Property::Label(&tr("Back to launcher"))]);
        let input = gtk::Text::builder()
            .hexpand(true)
            .css_classes(["kb-input"])
            .valign(gtk::Align::Center)
            .build();
        input.update_property(&[gtk::accessible::Property::Label(&tr("Search"))]);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        header.add_css_class("kb-header");
        header.append(&back);
        header.append(&search_icon);
        header.append(&input);

        let separator = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        separator.add_css_class("kb-separator");

        let category_title = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["kb-category-title"])
            .build();
        let clear_button = flat_button(&tr("Clear…"));
        clear_button.set_visible(false);
        clear_button.update_property(&[gtk::accessible::Property::Label(&tr(
            "Clear clipboard history",
        ))]);
        let ai_button = flat_button("");
        ai_button.set_visible(false);
        let actions_button = flat_button(&tr("Actions"));
        let category = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        category.add_css_class("kb-category");
        category.append(&category_title);
        category.append(&clear_button);
        category.append(&ai_button);
        category.append(&actions_button);

        let list = ResultList::new();
        let emoji = EmojiView::new();
        let snippet = SnippetPage::new();
        let extension = ExtensionPage::new();
        let stack = gtk::Stack::builder()
            .hhomogeneous(true)
            .vhomogeneous(false)
            .transition_type(gtk::StackTransitionType::None)
            .build();
        stack.add_named(list.widget(), Some("list"));
        stack.add_named(emoji.widget(), Some("emoji"));
        stack.add_named(snippet.widget(), Some("snippet"));
        stack.add_named(extension.widget(), Some("extension"));

        let message = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .css_classes(["kb-message"])
            .visible(false)
            .selectable(false)
            .build();

        let panel_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        panel_box.add_css_class("kb-panel");
        panel_box.set_overflow(gtk::Overflow::Hidden);
        panel_box.append(&header);
        panel_box.append(&separator);
        panel_box.append(&category);
        panel_box.append(&stack);
        panel_box.append(&message);
        panel.window().set_child(Some(&panel_box));
        gtk::prelude::GtkWindowExt::set_focus(panel.window(), Some(&input));

        let actions = ActionMenu::new();
        actions.attach(&actions_button);

        let (file_sender, file_receiver) = async_channel::unbounded();
        let exists = Arc::new(exists::ExistsCache::new(exists::EXISTS_TTL));
        let (exists_kick, kick_receiver) = async_channel::bounded(1);
        let (exists_changed, exists_changes) = async_channel::bounded(1);
        if let Err(error) =
            exists::spawn_checker(exists.clone(), kick_receiver, exists_changed, |path| {
                path.exists()
            })
        {
            eprintln!("kebloop: could not start the file checker: {error}");
        }
        let locale = Locale::from_tag(ctx.l10n.language().code());
        let inner = Rc::new(Inner {
            ctx: Rc::downgrade(ctx),
            panel,
            panel_box,
            input,
            back,
            search_icon,
            category_title,
            clear_button,
            ai_button,
            actions_button,
            stack,
            list,
            emoji,
            snippet,
            extension,
            message,
            actions,
            state: RefCell::new(State {
                mode: Mode::Root,
                items: Vec::new(),
                notice: None,
            }),
            calculator: CalculatorEngine::new(locale, ctx.time_zone.clone()),
            files: RefCell::new(FileState {
                results: Vec::new(),
                directories: HashSet::new(),
                cache: FileSearchCache::new(),
                ranker: FileRanker::new(&ctx.home),
                generation: 0,
                timer: None,
                sender: file_sender,
            }),
            extension_run: RefCell::new(ExtensionRun::default()),
            thumbnails_requested: RefCell::new(HashSet::new()),
            exists,
            exists_kick,
            suppress_changed: Cell::new(0),
            previous: RefCell::new(None),
            revision: Cell::new(0),
            opening: Cell::new(false),
            after_show: RefCell::new(Vec::new()),
            paste_cancel: RefCell::new(Arc::new(AtomicBool::new(false))),
            preedit: Cell::new(false),
            confirming: Cell::new(false),
            selection_touched: Cell::new(false),
            was_active: Cell::new(false),
            metrics: Cell::new(metrics),
        });
        let launcher = Self { inner };
        launcher.wire(file_receiver);
        let weak = launcher.downgrade();
        glib::spawn_future_local(async move {
            while exists_changes.recv().await.is_ok() {
                let Some(launcher) = weak.upgrade() else {
                    break;
                };
                if launcher.is_visible() && launcher.mode() == Mode::Root {
                    launcher.refresh(true);
                }
            }
        });
        launcher.apply_settings();
        launcher.update_chrome();
        launcher
    }

    // ----- construction helpers -------------------------------------------------------------

    fn wire(&self, file_results: async_channel::Receiver<(u64, Vec<FileMatch>)>) {
        let inner = &self.inner;

        let weak = self.downgrade();
        inner.input.connect_changed(move |_| {
            if let Some(launcher) = weak.upgrade()
                && launcher.inner.suppress_changed.get() == 0
            {
                launcher.on_query_changed();
            }
        });

        let weak = self.downgrade();
        inner.input.connect_preedit_changed(move |_, preedit| {
            if let Some(launcher) = weak.upgrade() {
                launcher.inner.preedit.set(!preedit.is_empty());
            }
        });

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = self.downgrade();
        keys.connect_key_pressed(move |_, keyval, _, state| {
            weak.upgrade()
                .map_or(glib::Propagation::Proceed, |l| l.on_key(keyval, state))
        });
        inner.panel.window().add_controller(keys);

        let weak = self.downgrade();
        inner
            .panel
            .window()
            .connect_is_active_notify(move |window| {
                let Some(launcher) = weak.upgrade() else {
                    return;
                };
                if window.is_active() {
                    launcher.inner.was_active.set(true);
                    return;
                }
                // Focus can blip while our own popover maps or the keyboard mode changes; only
                // hide when focus is still gone a moment later.
                let weak = launcher.downgrade();
                glib::timeout_add_local_once(Duration::from_millis(150), move || {
                    let Some(launcher) = weak.upgrade() else {
                        return;
                    };
                    let inner = &launcher.inner;
                    let window = inner.panel.window();
                    if inner.was_active.get()
                        && !inner.confirming.get()
                        && !inner.actions.is_open()
                        && window.is_visible()
                        && !window.is_active()
                    {
                        launcher.hide();
                    }
                });
            });

        let weak = self.downgrade();
        inner.panel.window().connect_close_request(move |_| {
            if let Some(launcher) = weak.upgrade() {
                launcher.hide();
            }
            glib::Propagation::Stop
        });

        let weak = self.downgrade();
        inner.back.connect_clicked(move |_| {
            if let Some(launcher) = weak.upgrade() {
                launcher.set_mode(Mode::Root);
            }
        });
        let weak = self.downgrade();
        inner.actions_button.connect_clicked(move |_| {
            if let Some(launcher) = weak.upgrade() {
                launcher.toggle_actions();
            }
        });
        let weak = self.downgrade();
        inner.ai_button.connect_clicked(move |_| {
            if let Some(launcher) = weak.upgrade() {
                launcher.ask_ai();
            }
        });
        let weak = self.downgrade();
        inner.clear_button.connect_clicked(move |_| {
            if let Some(launcher) = weak.upgrade() {
                launcher.confirm_clear_clipboard();
            }
        });

        // List interaction.
        let weak = self.downgrade();
        inner.list.connect_activate(move |index| {
            if let Some(launcher) = weak.upgrade() {
                launcher.inner.list.select(index, false);
                launcher.perform_selected();
            }
        });
        let weak = self.downgrade();
        inner.list.connect_context(move |index| {
            if let Some(launcher) = weak.upgrade() {
                launcher.inner.list.select(index, false);
                launcher.show_actions();
            }
        });
        let weak = self.downgrade();
        inner.list.connect_remove(move |index| {
            if let Some(launcher) = weak.upgrade() {
                launcher.inner.list.select(index, false);
                launcher.remove_selected_clipboard_item();
            }
        });
        let weak = self.downgrade();
        inner.extension.list().connect_activate(move |index| {
            if let Some(launcher) = weak.upgrade() {
                launcher.inner.extension.select(index);
                launcher.perform_selected();
            }
        });
        let weak = self.downgrade();
        inner.extension.list().connect_context(move |index| {
            if let Some(launcher) = weak.upgrade() {
                launcher.inner.extension.select(index);
                launcher.show_actions();
            }
        });
        let weak = self.downgrade();
        inner.extension.connect_action(move |result_id, action_id| {
            if let Some(launcher) = weak.upgrade() {
                launcher.run_extension_action(&result_id, &action_id);
            }
        });

        // Emoji.
        let weak = self.downgrade();
        inner.emoji.connect_pick(move |entry, paste| {
            if let Some(launcher) = weak.upgrade() {
                launcher.pick_emoji(entry, paste);
            }
        });

        // Snippet flow.
        let (on_copy, on_insert, on_cancel) =
            (self.downgrade(), self.downgrade(), self.downgrade());
        inner.snippet.connect(
            move |text| {
                if let Some(launcher) = on_copy.upgrade() {
                    launcher.finish_snippet(&text, false);
                }
            },
            move |text| {
                if let Some(launcher) = on_insert.upgrade() {
                    launcher.finish_snippet(&text, true);
                }
            },
            move || {
                if let Some(launcher) = on_cancel.upgrade() {
                    launcher.set_mode(Mode::Root);
                }
            },
        );

        // The layer surface holds the keyboard exclusively, except while the action popover (or
        // one of our dialogs) is open.
        let weak = self.downgrade();
        inner.actions.connect_open_changed(move |_| {
            if let Some(launcher) = weak.upgrade() {
                launcher.update_keyboard_mode();
            }
        });

        // Outside clicks in the panel close the action menu.
        let outside = gtk::GestureClick::new();
        outside.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = self.downgrade();
        outside.connect_pressed(move |_, _, _, _| {
            if let Some(launcher) = weak.upgrade()
                && launcher.inner.actions.is_open()
            {
                launcher.inner.actions.close();
            }
        });
        inner.stack.add_controller(outside);

        // File search results arrive from worker threads.
        let weak = self.downgrade();
        glib::spawn_future_local(async move {
            while let Ok((generation, matches)) = file_results.recv().await {
                let Some(launcher) = weak.upgrade() else {
                    break;
                };
                launcher.accept_file_results(generation, matches);
            }
        });

        // Application-wide changes.
        if let Some(ctx) = self.ctx() {
            let weak = self.downgrade();
            ctx.subscribe(Rc::new(move |change| {
                let Some(launcher) = weak.upgrade() else {
                    return;
                };
                launcher.on_change(change);
            }));
        }
    }

    fn on_change(&self, change: Change) {
        match change {
            Change::Settings | Change::Profile => {
                self.apply_settings();
                if self.is_visible() {
                    self.refresh(true);
                    self.update_chrome();
                }
            }
            Change::Apps | Change::Files | Change::Platform => {
                if self.is_visible() && matches!(self.mode(), Mode::Root | Mode::Browse) {
                    self.refresh(true);
                }
            }
            Change::Rates => {
                if self.is_visible() && self.mode() == Mode::Root {
                    // The conversion that failed for lack of a rate may succeed now.
                    self.inner.state.borrow_mut().notice = None;
                    self.refresh(true);
                }
            }
            Change::Clipboard => {
                if self.is_visible() && self.mode() == Mode::Clipboard {
                    self.refresh(true);
                }
            }
            Change::Usage | Change::Status => {}
        }
    }

    /// Apply appearance settings: width, density, translucency.
    pub fn apply_settings(&self) {
        let Some(ctx) = self.ctx() else { return };
        let (metrics, transparency, show_hints) = ctx.with_settings(|s| {
            (
                Metrics::from_settings(&s.personalization),
                s.personalization.transparency,
                s.personalization.show_shortcut_hints,
            )
        });
        let inner = &self.inner;
        inner.metrics.set(metrics);
        if !self.is_visible() {
            let output = ctx.with_settings(|s| fixed_output(&s.personalization));
            inner.panel.set_output(output.as_ref());
        }
        inner.panel_box.set_width_request(metrics.width);
        inner.panel.set_width(metrics.width);
        inner.list.set_style(RowStyle {
            show_hint: show_hints,
            icon_px: metrics.icon_px,
        });
        inner.extension.list().set_style(RowStyle {
            show_hint: show_hints,
            icon_px: metrics.icon_px,
        });
        let visible_rows = 7;
        inner
            .list
            .set_max_height((metrics.row_height + 2) * visible_rows + 8);
        let opaque = !transparency || adw::StyleManager::default().is_high_contrast();
        if opaque {
            inner.panel_box.add_css_class("kb-opaque");
        } else {
            inner.panel_box.remove_css_class("kb-opaque");
        }
        if adw::StyleManager::default().is_high_contrast() {
            inner.panel_box.add_css_class("kb-contrast");
        } else {
            inner.panel_box.remove_css_class("kb-contrast");
        }
    }

    // ----- state accessors ------------------------------------------------------------------

    /// The current mode.
    pub fn mode(&self) -> Mode {
        self.inner.state.borrow().mode.clone()
    }

    /// Whether the panel is showing.
    pub fn is_visible(&self) -> bool {
        self.inner.panel.is_visible()
    }

    /// The search text.
    pub fn query(&self) -> String {
        self.inner.input.text().to_string()
    }

    fn set_query_silently(&self, text: &str) {
        let inner = &self.inner;
        if inner.input.text() == text {
            return;
        }
        inner.suppress_changed.set(inner.suppress_changed.get() + 1);
        inner.input.set_text(text);
        inner.input.set_position(-1);
        inner.suppress_changed.set(inner.suppress_changed.get() - 1);
    }

    /// The top-level window (dialog parent).
    pub fn window(&self) -> &gtk::Window {
        self.inner.panel.window()
    }

    // ----- showing and hiding ---------------------------------------------------------------

    /// Show the launcher with an empty query (or just focus it when it is already open).
    pub fn show(&self) {
        let Some(ctx) = self.ctx() else { return };
        if self.is_visible() {
            self.inner.panel.present();
            self.focus_input();
            return;
        }
        let inner = &self.inner;
        if inner.opening.get() {
            return;
        }
        inner.opening.set(true);
        inner.state.borrow_mut().notice = None;
        inner.state.borrow_mut().mode = Mode::Root;
        inner.revision.set(inner.revision.get().wrapping_add(1));
        self.cancel_paste();
        inner.was_active.set(false);
        self.set_confirming(false);
        self.set_query_silently("");
        {
            let mut files = inner.files.borrow_mut();
            files.results.clear();
            files.directories.clear();
            files.generation += 1;
        }
        crate::services::apps::refresh_if_stale(&ctx, Duration::from_secs(10));
        // Look the configured output up again: the monitor object from startup is stale after a
        // hot-plug, and a disconnected output falls back to the compositor's focused one.
        let output = ctx.with_settings(|s| fixed_output(&s.personalization));
        inner.panel.set_output(output.as_ref());
        self.show_page();
        self.refresh(false);
        self.update_chrome();
        // Wait for the compositor snapshot before presenting and taking focus.
        self.begin_session();
    }

    /// Present after capturing the previous window, unless this opening was cancelled.
    fn finish_show(&self) {
        if !self.inner.opening.replace(false) {
            return;
        }
        self.inner.panel.present();
        self.focus_input();
        let callbacks = self.inner.after_show.take();
        for callback in callbacks {
            callback(self);
        }
    }

    /// Defer commands that require a visible panel until its focus snapshot completes.
    fn when_shown(&self, then: impl FnOnce(&Launcher) + 'static) {
        if self.inner.opening.get() {
            self.inner.after_show.borrow_mut().push(Box::new(then));
        } else {
            then(self);
        }
    }

    /// Hide the launcher.
    pub fn hide(&self) {
        let inner = &self.inner;
        inner.opening.set(false);
        inner.after_show.borrow_mut().clear();
        inner.actions.close();
        self.cancel_file_timer();
        self.cancel_extension();
        self.set_confirming(false);
        inner.was_active.set(false);
        inner.preedit.set(false);
        inner.panel.hide();
    }

    /// Show when hidden, hide when shown.
    pub fn toggle(&self) {
        if self.is_visible() || self.inner.opening.get() {
            self.hide();
        } else {
            self.show();
        }
    }

    /// Mark that one of our own dialogs is open: the panel does not hide when it loses focus.
    fn set_confirming(&self, confirming: bool) {
        self.inner.confirming.set(confirming);
        self.update_keyboard_mode();
    }

    /// Exclusive keyboard while the panel is shown and no confirmation dialog of ours needs the
    /// keyboard; on-demand otherwise. Focus loss still hides the panel
    /// (see the `is_active` handler), which keeps working when the compositor grants only
    /// on-demand focus.
    fn update_keyboard_mode(&self) {
        let inner = &self.inner;
        // The action popover never takes keyboard focus (keys stay with the panel), so it
        // doesn't need on-demand mode.
        let exclusive = !inner.confirming.get();
        inner.panel.set_keyboard_exclusive(exclusive);
    }

    fn focus_input(&self) {
        let inner = &self.inner;
        if inner.input.is_visible() {
            inner.input.grab_focus();
            inner.input.set_position(-1);
        }
    }

    // ----- modes and chrome -----------------------------------------------------------------

    fn show_page(&self) {
        let mode = self.mode();
        self.inner.stack.set_visible_child_name(mode.page());
    }

    /// Switch to `mode` with an empty query.
    pub fn set_mode(&self, mode: Mode) {
        let inner = &self.inner;
        if !matches!(mode, Mode::Extension(_)) {
            self.cancel_extension();
        }
        inner.actions.close();
        self.cancel_file_timer();
        {
            let mut state = inner.state.borrow_mut();
            state.mode = mode.clone();
            state.notice = None;
        }
        self.set_query_silently("");
        self.show_page();
        match &mode {
            Mode::Emoji => {
                let recents = self
                    .ctx()
                    .map(|c| c.config.borrow().global().usage.emoji_recents.clone())
                    .unwrap_or_default();
                inner.emoji.reset(&recents);
            }
            Mode::Extension(id) => self.begin_extension(id),
            Mode::Snippet => {}
            _ => self.refresh(false),
        }
        self.update_chrome();
        if mode == Mode::Root {
            self.inner.files.borrow_mut().results.clear();
            self.refresh(false);
        }
        self.inner.panel.fit_to_content();
        self.focus_input();
    }

    fn update_chrome(&self) {
        let Some(ctx) = self.ctx() else { return };
        let inner = &self.inner;
        let mode = self.mode();
        let profile = ctx.config.borrow().active_profile_name().to_owned();
        let title = if mode == Mode::Root || profile.is_empty() {
            mode.title()
        } else {
            format!("{} · {profile}", mode.title())
        };
        inner.category_title.set_text(&title);
        inner.input.set_placeholder_text(Some(&mode.placeholder()));
        let is_root = mode == Mode::Root;
        inner.back.set_visible(!is_root);
        inner.search_icon.set_visible(is_root);
        inner.input.set_visible(mode != Mode::Snippet);
        inner.clear_button.set_visible(mode == Mode::Clipboard);
        inner
            .clear_button
            .set_sensitive(!ctx.clipboard.borrow().items().is_empty());
        let hints = ctx.with_settings(|s| s.personalization.show_shortcut_hints);
        let query_blank = self.query().trim().is_empty();
        inner.ai_button.set_visible(is_root && !query_blank);
        let provider = kebloop_services::ai::AiProvider::from_stored(Some(
            ctx.config.borrow().global().ai_provider.raw_value(),
        ));
        let ask = tr_ask(provider.title());
        inner
            .ai_button
            .set_label(&if hints { format!("{ask}  Tab") } else { ask });
        inner.actions_button.set_label(&if hints {
            format!("{}  Ctrl+K", tr("Actions"))
        } else {
            tr("Actions")
        });
        inner
            .actions_button
            .set_visible(mode != Mode::Snippet && mode != Mode::Emoji);
        inner
            .actions_button
            .set_sensitive(self.has_actionable_selection());
        self.render_notice();
    }

    fn render_notice(&self) {
        let inner = &self.inner;
        let notice = inner.state.borrow().notice.clone();
        match notice {
            Some((text, kind)) => {
                inner.message.set_text(&text);
                inner.message.set_visible(true);
                if kind == NoticeKind::Error {
                    inner.message.remove_css_class("kb-note");
                } else {
                    inner.message.add_css_class("kb-note");
                }
            }
            None => inner.message.set_visible(false),
        }
    }

    /// Show a line under the list.
    pub fn set_notice(&self, text: Option<String>, kind: NoticeKind) {
        self.inner.state.borrow_mut().notice = text.map(|t| (t, kind));
        self.render_notice();
    }

    // ----- results --------------------------------------------------------------------------

    fn selected_item(&self) -> Option<ResultItem> {
        let state = self.inner.state.borrow();
        state.items.get(self.inner.list.selected()).cloned()
    }

    fn has_actionable_selection(&self) -> bool {
        match self.mode() {
            Mode::Extension(_) => self.inner.extension.selected_result().is_some(),
            Mode::Emoji | Mode::Snippet => false,
            _ => self.selected_item().is_some(),
        }
    }

    fn files_for_query(&self) -> (Vec<FileSearchResult>, HashSet<String>) {
        let files = self.inner.files.borrow();
        (files.results.clone(), files.directories.clone())
    }

    /// Rebuild the list for the current mode and query.
    pub fn refresh(&self, preserve_selection: bool) {
        let Some(ctx) = self.ctx() else { return };
        let inner = &self.inner;
        let mode = self.mode();
        if matches!(mode, Mode::Emoji | Mode::Snippet | Mode::Extension(_)) {
            return;
        }
        let query = self.query();
        if !preserve_selection {
            inner.selection_touched.set(false);
        }
        let keep_selection = preserve_selection && inner.selection_touched.get();
        let previous_id = if keep_selection {
            self.selected_item().map(|item| item.id)
        } else {
            None
        };
        let previous_index = inner.list.selected();
        let l10n = &ctx.l10n;
        let usage = ctx.usage_map();
        let availability = ctx.availability.borrow().clone();
        let running = ctx.running_apps.borrow().clone();
        let mut notice: Option<(String, NoticeKind)> = None;
        let mut empty_message = tr("No results");
        let items: Vec<ResultItem> = match &mode {
            Mode::Root => {
                let (files, directories) = self.files_for_query();
                let calculation = self.evaluate(&ctx, &query);
                let recents = ctx.recents();
                let apps = ctx.apps.borrow();
                let assembled = ctx.with_settings(|settings| {
                    model::assemble_root(&RootInputs {
                        query: &query,
                        settings,
                        l10n,
                        apps: &apps,
                        usage: &usage,
                        recents: &recents,
                        files: &files,
                        directories: &directories,
                        calculation: &calculation,
                        availability: &availability,
                        running: &running,
                        home: &ctx.home,
                        file_exists: &|path| {
                            let (exists, queued) = inner.exists.lookup(path, Instant::now());
                            if queued {
                                let _ = inner.exists_kick.try_send(());
                            }
                            exists
                        },
                    })
                });
                if let Some(message) = assembled.message {
                    notice = Some((message, NoticeKind::Error));
                }
                if apps.is_empty() {
                    empty_message = tr("Finding your apps…");
                }
                assembled.items
            }
            Mode::Browse => {
                let apps: Vec<kebloop_platform::apps::DesktopApp> = Vec::new();
                let directories = HashSet::new();
                let labels = shortcut_labels(&ctx);
                ctx.with_settings(|settings| {
                    model::assemble_browse(
                        &RootInputs {
                            query: &query,
                            settings,
                            l10n,
                            apps: &apps,
                            usage: &usage,
                            recents: &[],
                            files: &[],
                            directories: &directories,
                            calculation: &CalculationOutcome::None,
                            availability: &availability,
                            running: &running,
                            home: &ctx.home,
                            file_exists: &|_| false,
                        },
                        &labels,
                    )
                })
            }
            Mode::CalculationHistory => {
                empty_message = tr("Copy a calculation to start your history");
                model::assemble_calculation_history(
                    &query,
                    ctx.calculation_history.borrow().entries(),
                    l10n,
                    &ctx.time_zone,
                )
            }
            Mode::Snippets => {
                empty_message = tr("Add snippets in Settings");
                ctx.with_settings(|s| model::assemble_snippets(&query, &s.snippets, &usage, l10n))
            }
            Mode::Quicklinks => {
                empty_message = if ctx.with_settings(|s| s.quicklinks.is_empty()) {
                    tr("Add quicklinks in Settings")
                } else {
                    tr("No quicklinks found")
                };
                ctx.with_settings(|s| {
                    model::assemble_quicklinks(&query, &s.quicklinks, &usage, l10n, &ctx.home)
                })
            }
            Mode::WebSearch => {
                empty_message = if ctx.with_settings(|s| s.web_searches.is_empty()) {
                    tr("Add a web search template in Settings")
                } else {
                    tr("Type a query, then press Return to search")
                };
                ctx.with_settings(|s| model::assemble_web_search(&query, s, l10n, &ctx.home))
            }
            Mode::Profiles => {
                let config = ctx.config.borrow();
                model::assemble_profiles(
                    &query,
                    config.profiles(),
                    config.active_profile_id(),
                    l10n,
                )
            }
            Mode::Scripts => {
                empty_message = tr("Add script commands in Settings");
                ctx.with_settings(|s| model::assemble_scripts(&query, &s.scripts, l10n))
            }
            Mode::Extensions => {
                empty_message = tr("Add extensions in Settings");
                ctx.with_settings(|s| model::assemble_extensions(&query, &s.extensions, l10n))
            }
            Mode::Clipboard => {
                let items = model::assemble_clipboard(
                    &query,
                    ctx.clipboard.borrow().items(),
                    l10n,
                    &ctx.time_zone,
                );
                if let Some(note) = clipboard_view::current_note(&ctx, items.len()) {
                    let text = clipboard_view::note_text(&note);
                    if items.is_empty() {
                        empty_message = text;
                    } else {
                        notice = Some((text, NoticeKind::Info));
                    }
                }
                items
            }
            Mode::Emoji | Mode::Snippet | Mode::Extension(_) => Vec::new(),
        };

        let selected = previous_id
            .and_then(|id| items.iter().position(|item| item.id == id))
            .unwrap_or(if keep_selection { previous_index } else { 0 });
        let with_remove = mode == Mode::Clipboard;
        let missing = |entry_id: &str| self.request_thumbnail(entry_id);
        let pinned: HashSet<String> =
            ctx.with_settings(|s| s.pins.iter().map(|pin| pin.id.clone()).collect());
        inner.list.set_items(
            &items,
            selected,
            &empty_message,
            with_remove,
            &pinned,
            &missing,
        );
        inner.panel.fit_to_content();
        inner.state.borrow_mut().items = items;
        if let Some(notice) = notice {
            inner.state.borrow_mut().notice = Some(notice);
        } else if !preserve_selection
            || inner
                .state
                .borrow()
                .notice
                .as_ref()
                .is_some_and(|(_, kind)| *kind == NoticeKind::Info)
        {
            inner.state.borrow_mut().notice = None;
        }
        self.update_chrome();
    }

    fn request_thumbnail(&self, entry_id: &str) {
        let Some(ctx) = self.ctx() else { return };
        if !self
            .inner
            .thumbnails_requested
            .borrow_mut()
            .insert(entry_id.to_owned())
        {
            return;
        }
        let weak = self.downgrade();
        let size = self.inner.metrics.get().icon_px;
        clipboard_view::load_thumbnail(&ctx, entry_id, size, move |id, texture| {
            if let Some(launcher) = weak.upgrade() {
                launcher.inner.list.set_thumbnail(id, &texture);
            }
        });
    }

    // ----- typing ---------------------------------------------------------------------------

    fn on_query_changed(&self) {
        let mode = self.mode();
        self.inner.actions.close();
        self.inner.state.borrow_mut().notice = None;
        match mode {
            Mode::Emoji => self.inner.emoji.set_query(&self.query()),
            Mode::Extension(_) => {
                self.schedule_extension_search();
                self.update_chrome();
            }
            Mode::Snippet => {}
            Mode::Root => {
                if let Some(ctx) = self.ctx() {
                    crate::services::rates::request(&ctx, &self.query());
                }
                self.schedule_file_search();
                self.refresh(false);
            }
            _ => self.refresh(false),
        }
    }

    /// The calculator verdict for `query`, converting currencies with the saved rates.
    fn evaluate(&self, ctx: &AppContext, query: &str) -> CalculationOutcome {
        if query.trim().is_empty() {
            return CalculationOutcome::None;
        }
        self.inner
            .calculator
            .evaluate_with_rates(query, &ctx.currency.snapshots())
    }

    // ----- file search ----------------------------------------------------------------------

    fn cancel_file_timer(&self) {
        let mut files = self.inner.files.borrow_mut();
        files.generation += 1;
        if let Some(timer) = files.timer.take() {
            timer.remove();
        }
    }

    fn schedule_file_search(&self) {
        let Some(ctx) = self.ctx() else { return };
        let query = self.query();
        let trimmed = query.trim().to_owned();
        let enabled = ctx.with_settings(|s| s.personalization.files_enabled);
        let calculation_wins = matches!(self.evaluate(&ctx, &query), CalculationOutcome::Result(_));
        self.cancel_file_timer();
        let mut files = self.inner.files.borrow_mut();
        if !enabled || trimmed.is_empty() || calculation_wins {
            files.results.clear();
            return;
        }
        // Instant, provisional answer from earlier snapshots; the real search replaces it.
        let provisional = files.cache.matching(&trimmed, &files.ranker);
        files.results = provisional;
        let generation = files.generation;
        let weak = self.downgrade();
        let timer = glib::timeout_add_local_once(Duration::from_millis(DEBOUNCE_MS), move || {
            if let Some(launcher) = weak.upgrade() {
                launcher.inner.files.borrow_mut().timer = None;
                launcher.start_file_query(generation, trimmed.clone());
            }
        });
        files.timer = Some(timer);
    }

    fn start_file_query(&self, generation: u64, query: String) {
        let Some(ctx) = self.ctx() else { return };
        let snapshot = ctx.file_indexer.snapshot();
        let sender = self.inner.files.borrow().sender.clone();
        let spawned = std::thread::Builder::new()
            .name("kebloop-file-query".into())
            .spawn(move || {
                let matches = snapshot.query(&query, MAX_RESULTS);
                let _ = sender.send_blocking((generation, matches));
            });
        if let Err(error) = spawned {
            eprintln!("kebloop: could not start the file search: {error}");
        }
    }

    fn accept_file_results(&self, generation: u64, matches: Vec<FileMatch>) {
        {
            let mut files = self.inner.files.borrow_mut();
            if files.generation != generation || self.mode() != Mode::Root {
                return;
            }
            let found: Vec<FileSearchResult> = matches
                .iter()
                .map(|m| FileSearchResult::new(m.name.clone(), m.path.clone()))
                .collect();
            for hit in &matches {
                if hit.is_dir {
                    files.directories.insert(hit.path.clone());
                }
            }
            let query = self.inner.input.text().to_string();
            files.cache.store(&found, query.trim());
            files.results = found;
        }
        if self.is_visible() {
            self.refresh(true);
        }
    }

    // ----- keyboard -------------------------------------------------------------------------

    fn key_state(&self) -> KeyState {
        let mode = self.mode();
        KeyState {
            view: mode.view_kind(),
            actions_open: self.inner.actions.is_open(),
            query_empty: self.query().trim().is_empty(),
            has_results: self.has_actionable_selection(),
            preedit: self.inner.preedit.get(),
        }
    }

    fn on_key(&self, keyval: gdk::Key, state: gdk::ModifierType) -> glib::Propagation {
        let Some((key, mods)) = translate_key(keyval, state) else {
            return glib::Propagation::Proceed;
        };
        match keys::decide(key, mods, self.key_state()) {
            KeyAction::Propagate => glib::Propagation::Proceed,
            action => {
                self.run_key_action(action);
                glib::Propagation::Stop
            }
        }
    }

    fn run_key_action(&self, action: KeyAction) {
        match action {
            KeyAction::Propagate | KeyAction::Consume => {}
            KeyAction::Hide => self.hide(),
            KeyAction::Back => self.set_mode(Mode::Root),
            KeyAction::HideUi => self.hide_all_windows(),
            KeyAction::OpenSettings => {
                self.hide();
                if let Some(ctx) = self.ctx() {
                    crate::settings::open_settings(&ctx.app, &ctx, None);
                }
            }
            KeyAction::Move(delta) => self.move_selection(delta),
            KeyAction::Page(pages) => self.move_selection(pages * 6),
            KeyAction::First => self.select_index(0),
            KeyAction::Last => self.select_index(usize::MAX),
            KeyAction::Activate => self.perform_selected(),
            KeyAction::ToggleActions => self.toggle_actions(),
            KeyAction::AskAi => self.ask_ai(),
            KeyAction::RemoveClipboardItem => self.remove_selected_clipboard_item(),
            KeyAction::ClearClipboard => self.confirm_clear_clipboard(),
            KeyAction::CopySelected => match self.mode() {
                Mode::Emoji => self.inner.emoji.copy_selected(),
                _ => {
                    if let Some(item) = self.selected_item() {
                        self.perform_action(&ResultAction::Copy, &item);
                    }
                }
            },
            KeyAction::PastePlainSelected => {
                if let Some(item) = self.selected_item() {
                    self.perform_action(&ResultAction::PastePlainText, &item);
                }
            }
            KeyAction::QuickSelect(index) => {
                if index < self.inner.list.len() {
                    self.select_index(index);
                    self.perform_selected();
                }
            }
            KeyAction::ActionsMove(delta) => self.inner.actions.move_selection(delta),
            KeyAction::ActionsActivate => self.inner.actions.activate_selected(),
            KeyAction::ActionsClose => self.inner.actions.close(),
            KeyAction::GridMove(dx, dy) => self.inner.emoji.move_by(dx, dy),
        }
        if !matches!(
            action,
            KeyAction::HideUi | KeyAction::Hide | KeyAction::OpenSettings
        ) {
            self.inner
                .actions_button
                .set_sensitive(self.has_actionable_selection());
        }
        if matches!(action, KeyAction::Page(_)) && self.mode() == Mode::Emoji {
            self.inner.emoji.page(match action {
                KeyAction::Page(pages) => pages,
                _ => 0,
            });
        }
    }

    fn hide_all_windows(&self) {
        self.hide();
        if let Some(ctx) = self.ctx() {
            for slot in [
                &ctx.settings_window,
                &ctx.help_window,
                &ctx.onboarding_window,
            ] {
                if let Some(window) = slot.borrow().as_ref() {
                    window.set_visible(false);
                }
            }
        }
    }

    fn move_selection(&self, delta: isize) {
        match self.mode() {
            Mode::Extension(_) => self.inner.extension.move_selection(delta),
            Mode::Emoji | Mode::Snippet => {}
            _ => {
                let current = self.inner.list.selected();
                self.select_index(current.saturating_add_signed(delta));
            }
        }
    }

    fn select_index(&self, index: usize) {
        self.inner.selection_touched.set(true);
        match self.mode() {
            Mode::Extension(_) => self.inner.extension.select(index),
            Mode::Emoji | Mode::Snippet => {}
            _ => self.inner.list.select(index, false),
        }
    }

    /// Run the selected result.
    pub fn perform_selected(&self) {
        match self.mode() {
            Mode::Emoji => self.inner.emoji.activate_selected(),
            Mode::Snippet => {}
            Mode::Extension(_) => {
                if let Some(result) = self.inner.extension.selected_result()
                    && let Some(action) = result.actions.first()
                {
                    self.run_extension_action(&result.id, &action.id);
                }
            }
            _ => {
                if let Some(item) = self.selected_item() {
                    self.perform_item(&item);
                }
            }
        }
    }

    // ----- action menu ----------------------------------------------------------------------

    fn toggle_actions(&self) {
        if self.inner.actions.is_open() {
            self.inner.actions.close();
        } else {
            self.show_actions();
        }
    }

    fn show_actions(&self) {
        let Some(ctx) = self.ctx() else { return };
        let inner = &self.inner;
        if let Mode::Extension(_) = self.mode() {
            let Some(result) = inner.extension.selected_result() else {
                return;
            };
            let entries: Vec<actions::ActionEntry> = result
                .actions
                .iter()
                .map(|a| actions::ActionEntry {
                    action: ResultAction::ExtensionAction(a.id.clone()),
                    title: a.title.clone(),
                    icon: "go-next-symbolic",
                })
                .collect();
            if entries.is_empty() {
                return;
            }
            let weak = self.downgrade();
            let result_id = result.id.clone();
            inner.actions.show(&result.title, entries, move |entry| {
                if let (Some(launcher), ResultAction::ExtensionAction(id)) =
                    (weak.upgrade(), entry.action)
                {
                    launcher.run_extension_action(&result_id, &id);
                }
            });
            return;
        }
        let Some(item) = self.selected_item() else {
            return;
        };
        let context = self.action_context(&ctx, &item);
        let entries = actions_for(&item, &context, &ctx.l10n);
        if entries.is_empty() {
            return;
        }
        let weak = self.downgrade();
        let chosen_item = item.clone();
        inner.actions.show(&item.title, entries, move |entry| {
            if let Some(launcher) = weak.upgrade() {
                launcher.perform_action(&entry.action, &chosen_item);
            }
        });
    }

    fn action_context(&self, ctx: &AppContext, item: &ResultItem) -> ActionContext {
        let pinned = ctx.with_settings(|s| s.pins.iter().any(|p| p.id == item.id));
        let editable_quicklink = match &item.target {
            Target::Quicklink { link, .. } => {
                ctx.with_settings(|s| s.quicklinks.iter().any(|l| l.id == link.id))
            }
            _ => false,
        };
        let desktop_actions = match &item.target {
            Target::Application { desktop_id, .. } => ctx
                .apps
                .borrow()
                .iter()
                .find(|a| a.id == *desktop_id)
                .map(ActionContext::desktop_actions_of)
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let running = matches!(&item.target, Target::Application { desktop_id, .. }
            if ctx.running_apps.borrow().contains(desktop_id));
        let has_plain_text = match &item.target {
            Target::Clipboard { entry_id, .. } => ctx
                .clipboard
                .borrow()
                .items()
                .iter()
                .any(|entry| entry.id == *entry_id && !entry.text.is_empty()),
            _ => false,
        };
        ActionContext {
            pinned,
            editable_quicklink,
            desktop_actions,
            running,
            controls: ctx.app_controls(),
            has_plain_text,
        }
    }
}

fn flat_button(label: &str) -> gtk::Button {
    gtk::Button::builder()
        .label(label)
        .css_classes(["flat"])
        .focusable(false)
        .build()
}

fn tr_ask(provider: &str) -> String {
    crate::i18n::trf(
        "Ask %@",
        &[kebloop_services::l10n::FormatArg::Str(provider.to_owned())],
    )
}

/// Labels of the shortcuts assigned to commands, keyed by command id.
fn shortcut_labels(ctx: &AppContext) -> std::collections::HashMap<String, String> {
    ctx.with_settings(|s| {
        s.command_shortcuts
            .iter()
            .map(|b| {
                let label = b
                    .resolved()
                    .map_or_else(|| b.label.clone(), |accelerator| accelerator.to_string());
                (b.id.clone(), label)
            })
            .collect()
    })
}

/// The monitor chosen in settings (`placement == "display"`), if connected.
fn fixed_output(
    settings: &kebloop_services::personalization::PersonalizationSettings,
) -> Option<gdk::Monitor> {
    if settings.placement != "display" || settings.display_id.is_empty() {
        return None;
    }
    let display = gdk::Display::default()?;
    let monitors = display.monitors();
    (0..monitors.n_items())
        .filter_map(|i| {
            monitors
                .item(i)
                .and_then(|o| o.downcast::<gdk::Monitor>().ok())
        })
        .find(|m| m.connector().as_deref() == Some(settings.display_id.as_str()))
}

/// GDK key event → launcher key and modifiers. `None` for keys the launcher ignores.
fn translate_key(keyval: gdk::Key, state: gdk::ModifierType) -> Option<(Key, Mods)> {
    let mods = Mods {
        ctrl: state.contains(gdk::ModifierType::CONTROL_MASK),
        shift: state.contains(gdk::ModifierType::SHIFT_MASK),
        alt: state.contains(gdk::ModifierType::ALT_MASK),
        logo: state.contains(gdk::ModifierType::SUPER_MASK),
    };
    let key = match keyval {
        gdk::Key::Escape => Key::Escape,
        gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter => Key::Return,
        gdk::Key::Tab | gdk::Key::ISO_Left_Tab => Key::Tab,
        gdk::Key::Up | gdk::Key::KP_Up => Key::Up,
        gdk::Key::Down | gdk::Key::KP_Down => Key::Down,
        gdk::Key::Left | gdk::Key::KP_Left => Key::Left,
        gdk::Key::Right | gdk::Key::KP_Right => Key::Right,
        gdk::Key::Page_Up | gdk::Key::KP_Page_Up => Key::PageUp,
        gdk::Key::Page_Down | gdk::Key::KP_Page_Down => Key::PageDown,
        gdk::Key::Home | gdk::Key::KP_Home => Key::Home,
        gdk::Key::End | gdk::Key::KP_End => Key::End,
        gdk::Key::BackSpace => Key::BackSpace,
        other => Key::Char(other.to_lower().to_unicode()?),
    };
    Some((key, mods))
}
