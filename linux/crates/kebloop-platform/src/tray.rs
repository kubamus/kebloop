//! StatusNotifierItem tray icon (via `ksni`). Optional: the app works without it.

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{MenuItem, StandardItem};

/// Actions emitted by the tray.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    OpenLauncher,
    Settings,
    RefreshApps,
    Quit,
}

/// Localisable strings (injected by the GUI).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayLabels {
    pub title: String,
    pub open_launcher: String,
    pub settings: String,
    pub refresh_apps: String,
    pub quit: String,
}

impl Default for TrayLabels {
    fn default() -> Self {
        Self {
            title: "Kebloop".into(),
            open_launcher: "Open Launcher".into(),
            settings: "Settings".into(),
            refresh_apps: "Refresh Apps".into(),
            quit: "Quit".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TrayConfig {
    pub id: String,
    pub icon_name: String,
    pub labels: TrayLabels,
    /// Set inside sandboxes (Flatpak) where owning a bus name is not allowed.
    pub disable_dbus_name: bool,
}

impl Default for TrayConfig {
    fn default() -> Self {
        Self {
            id: "io.github.kubamus.Kebloop".into(),
            icon_name: "io.github.kubamus.Kebloop".into(),
            labels: TrayLabels::default(),
            disable_dbus_name: false,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TrayError {
    /// No StatusNotifierWatcher/host: run without a tray.
    #[error("no system tray available: {0}")]
    Unavailable(String),
    #[error("tray error: {0}")]
    Other(String),
}

struct KebloopTray {
    config: TrayConfig,
    events: async_channel::Sender<TrayEvent>,
}

impl KebloopTray {
    fn item(&self, label: &str, icon: &str, event: TrayEvent) -> MenuItem<Self> {
        StandardItem {
            label: label.to_owned(),
            icon_name: icon.to_owned(),
            activate: Box::new(move |this: &mut Self| {
                let _ = this.events.try_send(event);
            }),
            ..Default::default()
        }
        .into()
    }
}

impl ksni::Tray for KebloopTray {
    fn id(&self) -> String {
        self.config.id.clone()
    }
    fn title(&self) -> String {
        self.config.labels.title.clone()
    }
    fn icon_name(&self) -> String {
        self.config.icon_name.clone()
    }
    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.events.try_send(TrayEvent::OpenLauncher);
    }
    /// The watcher vanished (shell restart) or has not appeared yet: keep the service alive so
    /// the icon registers as soon as a watcher shows up.
    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        true
    }
    fn menu(&self) -> Vec<MenuItem<Self>> {
        let l = &self.config.labels;
        vec![
            self.item(&l.open_launcher, "system-search", TrayEvent::OpenLauncher),
            self.item(&l.settings, "preferences-system", TrayEvent::Settings),
            self.item(&l.refresh_apps, "view-refresh", TrayEvent::RefreshApps),
            MenuItem::Separator,
            self.item(&l.quit, "application-exit", TrayEvent::Quit),
        ]
    }
}

/// Handle to the running tray. Dropping it removes the icon.
pub struct TrayHandle {
    handle: Handle<KebloopTray>,
}

impl TrayHandle {
    /// Replaces the localised labels (language change).
    pub fn set_labels(&self, labels: TrayLabels) {
        let _ = self.handle.update(|t| t.config.labels = labels);
    }

    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    /// Removes the icon and waits for the service to stop.
    pub fn shutdown(self) {
        self.handle.shutdown().wait();
    }
}

impl Drop for TrayHandle {
    fn drop(&mut self) {
        let _ = self.handle.shutdown();
    }
}

/// Registers the tray. Starting before the panel/watcher exists is fine: the service assumes a
/// StatusNotifier implementation will appear (`assume_sni_available`), stays idle while there is
/// none and registers the icon when a watcher comes online. Fails only when the D-Bus session
/// itself is unusable; the caller should continue without a tray then.
pub fn start_tray(
    config: TrayConfig,
) -> Result<(TrayHandle, async_channel::Receiver<TrayEvent>), TrayError> {
    let (tx, rx) = async_channel::unbounded();
    let disable_name = config.disable_dbus_name;
    let tray = KebloopTray { config, events: tx };
    match tray
        .disable_dbus_name(disable_name)
        .assume_sni_available(true)
        .spawn()
    {
        Ok(handle) => Ok((TrayHandle { handle }, rx)),
        Err(e @ (ksni::Error::Watcher(_) | ksni::Error::WontShow)) => {
            Err(TrayError::Unavailable(e.to_string()))
        }
        Err(e) => Err(TrayError::Other(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ksni::Tray;

    #[test]
    fn menu_has_four_actions_and_a_separator() {
        let (tx, rx) = async_channel::unbounded();
        let tray = KebloopTray {
            config: TrayConfig::default(),
            events: tx,
        };
        let menu = tray.menu();
        assert_eq!(menu.len(), 5);
        assert!(matches!(menu[3], MenuItem::Separator));
        let labels: Vec<String> = menu
            .iter()
            .filter_map(|m| match m {
                MenuItem::Standard(s) => Some(s.label.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            labels,
            ["Open Launcher", "Settings", "Refresh Apps", "Quit"]
        );

        // Activating each item sends the matching event.
        let mut tray = tray;
        let mut seen = Vec::new();
        for item in tray.menu() {
            if let MenuItem::Standard(s) = item {
                (s.activate)(&mut tray);
                seen.push(rx.try_recv().unwrap());
            }
        }
        assert_eq!(
            seen,
            [
                TrayEvent::OpenLauncher,
                TrayEvent::Settings,
                TrayEvent::RefreshApps,
                TrayEvent::Quit
            ]
        );
        tray.activate(0, 0);
        assert_eq!(rx.try_recv().unwrap(), TrayEvent::OpenLauncher);
    }

    #[test]
    fn tray_stays_alive_while_the_watcher_is_offline() {
        let (tx, _rx) = async_channel::unbounded();
        let tray = KebloopTray {
            config: TrayConfig::default(),
            events: tx,
        };
        assert!(tray.watcher_offline(ksni::OfflineReason::No));
        assert!(tray.watcher_offline(ksni::OfflineReason::Error(ksni::Error::WontShow)));
    }

    /// Must run in a session bus without a StatusNotifierWatcher, e.g.
    /// `dbus-run-session -- cargo test -- --ignored tray_starts_without_watcher`.
    #[test]
    #[ignore = "needs a private session bus without a watcher"]
    fn tray_starts_without_watcher() {
        match start_tray(TrayConfig::default()) {
            Ok((handle, _events)) => assert!(!handle.is_closed()),
            Err(other) => panic!("the tray must wait for a watcher, got: {other}"),
        }
    }
}
