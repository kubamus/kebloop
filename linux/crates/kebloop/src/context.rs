//! The shared application context: stores, indexes and services, owned by the GTK main thread.
//! Window code receives an `Rc<AppContext>`; worker threads never touch it (they get snapshots
//! and send results back over channels).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};

use kebloop_platform::apps::DesktopApp;
use kebloop_platform::clipboard::ClipboardWatcher;
use kebloop_platform::compositor::Compositor;
use kebloop_platform::files::FileIndexer;
use kebloop_platform::paste::PasteTool;
use kebloop_platform::paths::XdgDirs;
use kebloop_platform::session::SessionInfo;
use kebloop_platform::system_actions::Availability;
use kebloop_platform::window_management::WindowManager;
use kebloop_services::calculation_history::CalculationHistoryStore;
use kebloop_services::clipboard::ClipboardStore;
use kebloop_services::config::{ConfigStore, PortableSettings, UpdateError};
use kebloop_services::currency::CurrencyRateStore;
use kebloop_services::file_search::FileSearchRules;
use kebloop_services::l10n::Localizer;
use kebloop_services::persist::DebouncedWriter;

use crate::launcher::Launcher;
use crate::platform::{AppControls, CommandAvailability};
use crate::style::Styling;

/// What changed; listeners refresh whatever depends on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Profile settings, aliases, quicklinks, snippets, pins, ... (any `update_settings`).
    Settings,
    /// A different profile became active.
    Profile,
    /// The application list was rescanned.
    Apps,
    /// The file index was rebuilt.
    Files,
    /// Clipboard history or its recording state changed.
    Clipboard,
    /// Launch counts or recents changed.
    Usage,
    /// A backend state changed (global-shortcut portal, clipboard backend, paste tool).
    Status,
    /// Which platform commands are offered (power-off, system actions) was probed.
    Platform,
    /// New exchange rates were saved.
    Rates,
}

/// State of the XDG GlobalShortcuts portal registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortalState {
    /// Registration has not finished yet.
    Checking,
    /// The portal accepted the shortcuts; the entries are `(id, trigger description)`.
    Active(Vec<(String, String)>),
    /// The portal is missing or refused; the text explains why. Compositor key bindings that run
    /// `kebloop toggle` / `kebloop command` / `kebloop launch` are the way to bind keys then.
    Unavailable(String),
}

/// State of the Wayland clipboard backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardStatus {
    /// Still connecting.
    Starting,
    /// Recording clipboard changes.
    Active,
    /// Not available; the text explains why (shown to the user).
    Unavailable(String),
    /// The history store could not be opened, so history is off for this run; the text says why.
    HistoryDisabled(String),
}

/// Identifies a change listener for [`AppContext::unsubscribe`].
pub type ListenerId = u64;
type Listener = Rc<dyn Fn(Change)>;

/// Shared state. Fields are public for the settings, help and onboarding windows.
pub struct AppContext {
    /// The application.
    pub app: adw::Application,
    /// XDG base directories.
    pub xdg: XdgDirs,
    /// Home directory as a string.
    pub home: String,
    /// Translations.
    pub l10n: Localizer,
    /// System time zone (for clipboard timestamps and the calculator).
    pub time_zone: jiff::tz::TimeZone,
    /// Persistent configuration (profiles, global usage).
    pub config: RefCell<ConfigStore>,
    /// Clipboard history.
    pub clipboard: RefCell<ClipboardStore>,
    /// Installed applications, sorted by name.
    pub apps: RefCell<Vec<DesktopApp>>,
    /// Background file index.
    pub file_indexer: FileIndexer,
    /// File exclusion policy.
    pub file_rules: FileSearchRules,
    /// Clipboard watcher/setter, once started.
    pub clipboard_watcher: RefCell<Option<Arc<ClipboardWatcher>>>,
    /// Backend state.
    pub clipboard_status: RefCell<ClipboardStatus>,
    /// Shared with the clipboard watcher: whether history is recording (enabled and not paused).
    /// While `false` the watcher never reads clipboard content.
    pub clipboard_recording: Arc<AtomicBool>,
    /// Background writer for the clipboard history and usage counters (flushed on quit).
    pub persistence: Arc<DebouncedWriter>,
    /// Problems found at startup (recovered or unreadable files, disabled history) that the user
    /// should see; listed in Help and sent as a desktop notification.
    pub notices: RefCell<Vec<String>>,
    /// Which built-in commands this machine can run (power-off, system actions, windows).
    pub availability: RefCell<CommandAvailability>,
    /// The result of the last system-action probe, with the reasons for unavailable ones.
    pub system_probe: RefCell<Vec<(kebloop_platform::system_actions::SystemAction, Availability)>>,
    /// Compositor IPC (window list, focus, close, layout), when a supported one is running.
    pub compositor: Option<Arc<dyn Compositor>>,
    /// Why there is no compositor IPC.
    pub compositor_error: Option<String>,
    /// Window arrangement on the compositor (remembers frames for Restore).
    pub window_manager: Option<Arc<WindowManager>>,
    /// Desktop ids of applications that have a window, refreshed whenever the launcher opens.
    pub running_apps: RefCell<HashSet<String>>,
    /// Saved exchange rates (downloads happen on a worker thread).
    pub currency: Arc<CurrencyRateStore>,
    /// Accepted calculator results.
    pub calculation_history: RefCell<CalculationHistoryStore>,
    /// Queries for the exchange-rate worker thread.
    pub rate_requests: RefCell<Option<async_channel::Sender<String>>>,
    /// Counts clipboard changes made by other programs (a paste is cancelled if it moves).
    pub clipboard_revision: Arc<AtomicU64>,
    /// `wtype`/`ydotool`, when installed.
    pub paste_tool: Option<PasteTool>,
    /// Status of the opt-in accessibility snippet service, shown in Settings.
    pub snippet_expansion_status: RefCell<String>,
    /// Display server and compositor.
    pub session: SessionInfo,
    /// Global-shortcut portal registration state.
    pub portal: RefCell<PortalState>,
    /// Style sheets.
    pub styling: Styling,
    /// The launcher panel (created at startup).
    pub launcher: RefCell<Option<Launcher>>,
    /// Cancel handles of running scripts and extensions (cancelled on quit).
    running: RefCell<Vec<(u64, kebloop_services::automation::CancelHandle)>>,
    next_run: Cell<u64>,
    /// The settings window, if open.
    pub settings_window: RefCell<Option<gtk::Window>>,
    /// The help window, if open.
    pub help_window: RefCell<Option<gtk::Window>>,
    /// The onboarding window, if open.
    pub onboarding_window: RefCell<Option<gtk::Window>>,
    /// Application scan bookkeeping.
    pub(crate) scan: RefCell<crate::services::apps::ScanState>,
    keep_alive: RefCell<Vec<Box<dyn std::any::Any>>>,
    listeners: RefCell<Vec<(ListenerId, Listener)>>,
    next_listener: Cell<ListenerId>,
}

impl AppContext {
    /// Assemble a context from already opened parts.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        app: adw::Application,
        xdg: XdgDirs,
        l10n: Localizer,
        mut config: ConfigStore,
        mut clipboard: ClipboardStore,
        file_indexer: FileIndexer,
        styling: Styling,
    ) -> Self {
        let persistence = Arc::new(DebouncedWriter::new());
        config.set_deferred_writer(persistence.clone());
        clipboard.set_deferred_writer(persistence.clone());
        let home = xdg.home.to_string_lossy().into_owned();
        let temp = std::env::temp_dir().to_string_lossy().into_owned();
        let (compositor, compositor_error) = match kebloop_platform::compositor::connect(
            &kebloop_platform::paths::Env::from_process(),
        ) {
            Ok(connected) => (Some(Arc::<dyn Compositor>::from(connected)), None),
            Err(error) => (None, Some(error.to_string())),
        };
        let window_manager = compositor
            .clone()
            .map(|compositor| Arc::new(WindowManager::new(compositor)));
        let availability = CommandAvailability::new(
            false,
            [],
            compositor.as_ref().map(|compositor| compositor.kind()),
        );
        let currency = Arc::new(CurrencyRateStore::with_system_defaults(
            &xdg.app_cache_dir(),
        ));
        let calculation_history = CalculationHistoryStore::with_system_clock(&xdg.app_data_dir());
        Self {
            app,
            file_rules: FileSearchRules::new(&home, &temp),
            home,
            xdg,
            l10n,
            time_zone: jiff::tz::TimeZone::system(),
            config: RefCell::new(config),
            clipboard: RefCell::new(clipboard),
            apps: RefCell::new(Vec::new()),
            file_indexer,
            clipboard_watcher: RefCell::new(None),
            clipboard_status: RefCell::new(ClipboardStatus::Starting),
            clipboard_recording: Arc::new(AtomicBool::new(false)),
            persistence,
            notices: RefCell::new(Vec::new()),
            availability: RefCell::new(availability),
            system_probe: RefCell::new(Vec::new()),
            compositor,
            compositor_error,
            window_manager,
            running_apps: RefCell::new(HashSet::new()),
            currency,
            calculation_history: RefCell::new(calculation_history),
            rate_requests: RefCell::new(None),
            clipboard_revision: Arc::new(AtomicU64::new(0)),
            paste_tool: kebloop_platform::paste::detect_paste_tool_in_path(),
            snippet_expansion_status: RefCell::new(String::new()),
            session: SessionInfo::detect(),
            portal: RefCell::new(PortalState::Checking),
            styling,
            launcher: RefCell::new(None),
            running: RefCell::new(Vec::new()),
            next_run: Cell::new(1),
            settings_window: RefCell::new(None),
            help_window: RefCell::new(None),
            onboarding_window: RefCell::new(None),
            scan: RefCell::new(crate::services::apps::ScanState::default()),
            keep_alive: RefCell::new(Vec::new()),
            listeners: RefCell::new(Vec::new()),
            next_listener: Cell::new(1),
        }
    }

    /// Run `f` on the active profile's settings without cloning them.
    pub fn with_settings<R>(&self, f: impl FnOnce(&PortableSettings) -> R) -> R {
        f(self.config.borrow().settings())
    }

    /// A copy of the active profile's settings.
    pub fn settings(&self) -> PortableSettings {
        self.with_settings(Clone::clone)
    }

    /// Change the active profile's settings (validated and saved atomically), then tell the
    /// listeners. Errors leave the settings unchanged.
    pub fn update_settings<R, E: std::error::Error + 'static>(
        &self,
        change: impl FnOnce(&mut PortableSettings) -> Result<R, E>,
    ) -> Result<R, UpdateError<E>> {
        let result = self.config.borrow_mut().update(change);
        if result.is_ok() {
            self.emit(Change::Settings);
        }
        result
    }

    /// Apply an infallible edit to the active profile's settings. The error text is ready to show
    /// to the user (validation failures and write errors never panic).
    pub fn edit_settings(&self, change: impl FnOnce(&mut PortableSettings)) -> Result<(), String> {
        self.update_settings(|settings| {
            change(settings);
            Ok::<(), std::convert::Infallible>(())
        })
        .map_err(|error| crate::i18n::tr(&error.to_string()))
    }

    /// Launch counts as the ranker wants them.
    pub fn usage_map(&self) -> HashMap<String, i64> {
        self.config
            .borrow()
            .global()
            .usage
            .usage
            .iter()
            .map(|(id, count)| (id.clone(), i64::try_from(*count).unwrap_or(i64::MAX)))
            .collect()
    }

    /// Result ids used recently, newest first.
    pub fn recents(&self) -> Vec<String> {
        self.config.borrow().global().usage.recent_results.clone()
    }

    /// Count a launch of `id` and remember it as recent.
    pub fn record_use(&self, id: &str, count_launch: bool) {
        let outcome = self.config.borrow_mut().record_use(id, count_launch);
        match outcome {
            Ok(()) => self.emit(Change::Usage),
            Err(error) => eprintln!("kebloop: could not save usage: {error}"),
        }
    }

    /// Register a change listener.
    pub fn subscribe(&self, listener: Rc<dyn Fn(Change)>) -> ListenerId {
        let id = self.next_listener.get();
        self.next_listener.set(id + 1);
        self.listeners.borrow_mut().push((id, listener));
        id
    }

    /// Remove a listener registered with [`AppContext::subscribe`].
    pub fn unsubscribe(&self, id: ListenerId) {
        self.listeners
            .borrow_mut()
            .retain(|(listener, _)| *listener != id);
    }

    /// Notify listeners. Safe to call from inside a listener.
    pub fn emit(&self, change: Change) {
        let snapshot: Vec<Rc<dyn Fn(Change)>> = self
            .listeners
            .borrow()
            .iter()
            .map(|(_, f)| f.clone())
            .collect();
        for listener in snapshot {
            listener(change);
        }
    }

    /// Remember a running script/extension so quitting can cancel it; returns its token.
    pub fn register_run(&self, handle: kebloop_services::automation::CancelHandle) -> u64 {
        let token = self.next_run.get();
        self.next_run.set(token + 1);
        self.running.borrow_mut().push((token, handle));
        token
    }

    /// Forget a finished run.
    pub fn finish_run(&self, token: u64) {
        self.running.borrow_mut().retain(|(t, _)| *t != token);
    }

    /// Write pending background saves (config usage, clipboard history). Called on quit.
    pub fn flush_persistence(&self) {
        if let Err(error) = self.persistence.flush() {
            eprintln!("kebloop: could not save: {error}");
        }
    }

    /// Remember a startup problem for the Help window.
    pub fn add_notice(&self, text: String) {
        eprintln!("kebloop: {text}");
        self.notices.borrow_mut().push(text);
    }

    /// Cancel everything still running.
    pub fn cancel_all_runs(&self) {
        for (_, handle) in self.running.borrow().iter() {
            handle.cancel();
        }
    }

    /// Keep a service handle (watcher, tray, portal session) alive for the whole run.
    pub fn keep_alive(&self, handle: impl std::any::Any) {
        self.keep_alive.borrow_mut().push(Box::new(handle));
    }

    /// What the compositor lets Kebloop do to running applications.
    pub fn app_controls(&self) -> AppControls {
        AppControls::of(self.compositor.as_ref().map(|c| c.capabilities()))
    }

    /// Record the result of the power-off probe; tells the listeners when it changed.
    pub fn set_shutdown_available(&self, allowed: bool) {
        let changed = {
            let mut availability = self.availability.borrow_mut();
            let next = CommandAvailability::new(
                allowed,
                availability_system(&self.system_probe.borrow()),
                availability.window_kind(),
            );
            let changed = *availability != next;
            *availability = next;
            changed
        };
        if changed {
            self.emit(Change::Platform);
        }
    }

    /// Record the result of the system-action probe; tells the listeners when it changed.
    pub fn set_system_probe(
        &self,
        probe: Vec<(kebloop_platform::system_actions::SystemAction, Availability)>,
    ) {
        let changed = {
            let mut availability = self.availability.borrow_mut();
            let next = CommandAvailability::new(
                availability.shutdown(),
                availability_system(&probe),
                availability.window_kind(),
            );
            let changed = *availability != next;
            *availability = next;
            changed
        };
        *self.system_probe.borrow_mut() = probe;
        if changed {
            self.emit(Change::Platform);
        }
    }

    /// The launcher panel.
    pub fn launcher(&self) -> Option<Launcher> {
        self.launcher.borrow().clone()
    }
}

fn availability_system(
    probe: &[(kebloop_platform::system_actions::SystemAction, Availability)],
) -> Vec<kebloop_platform::system_actions::SystemAction> {
    probe
        .iter()
        .filter(|(_, availability)| availability.is_available())
        .map(|(action, _)| *action)
        .collect()
}
