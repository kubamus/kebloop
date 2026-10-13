//! Status-notifier tray icon (optional; the app works without one).

use std::rc::Rc;

use gtk::glib;
use kebloop_platform::tray::{TrayConfig, TrayEvent, TrayLabels, start_tray};

use crate::cli::Request;
use crate::context::AppContext;
use crate::i18n::tr;

/// Start the tray on a worker thread. A missing StatusNotifierWatcher is not an error.
pub fn init(ctx: &Rc<AppContext>) {
    let labels = TrayLabels {
        title: "Kebloop".into(),
        open_launcher: tr("Open Kebloop"),
        settings: tr("Settings…"),
        refresh_apps: tr("Refresh Applications"),
        quit: tr("Quit Kebloop"),
    };
    let mut config = TrayConfig {
        labels,
        ..TrayConfig::default()
    };
    config.id = crate::cli::app_id();
    let (sender, receiver) = async_channel::bounded(1);
    let spawned = std::thread::Builder::new()
        .name("kebloop-tray".into())
        .spawn(move || match start_tray(config) {
            Ok((handle, events)) => {
                let _ = sender.send_blocking(Some(events));
                // The tray lives as long as this thread keeps the handle.
                loop {
                    std::thread::park();
                    if handle.is_closed() {
                        break;
                    }
                }
            }
            Err(error) => {
                eprintln!("kebloop: no tray icon: {error}");
                let _ = sender.send_blocking(None);
            }
        });
    if spawned.is_err() {
        return;
    }
    let ctx = ctx.clone();
    glib::spawn_future_local(async move {
        let Ok(Some(events)) = receiver.recv().await else {
            return;
        };
        while let Ok(event) = events.recv().await {
            let request = match event {
                TrayEvent::OpenLauncher => Request::Show,
                TrayEvent::Settings => Request::Settings(None),
                TrayEvent::RefreshApps => {
                    super::apps::refresh(&ctx);
                    continue;
                }
                TrayEvent::Quit => Request::Quit,
            };
            crate::app::dispatch(&ctx, request);
        }
    });
}
