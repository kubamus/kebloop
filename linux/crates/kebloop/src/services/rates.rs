//! Exchange rates: the launcher asks for the pair in a currency query; one worker thread
//! downloads it (never on the GTK thread) and tells the main loop when new rates were saved.

use std::rc::Rc;

use gtk::glib;
use kebloop_services::currency::RefreshOutcome;

use crate::context::{AppContext, Change};

/// Start the worker thread.
pub fn init(ctx: &Rc<AppContext>) {
    let (requests, queue) = async_channel::unbounded::<String>();
    let (updated, updates) = async_channel::unbounded::<()>();
    let store = ctx.currency.clone();
    let spawned = std::thread::Builder::new()
        .name("kebloop-rates".into())
        .spawn(move || {
            while let Ok(mut query) = queue.recv_blocking() {
                // Only the newest query matters; keystrokes that queued up meanwhile are dropped.
                while let Ok(newer) = queue.try_recv() {
                    query = newer;
                }
                if store.refresh_if_needed(&query) == RefreshOutcome::Updated
                    && updated.send_blocking(()).is_err()
                {
                    break;
                }
            }
        });
    if let Err(error) = spawned {
        eprintln!("kebloop: could not start the exchange-rate worker: {error}");
        return;
    }
    *ctx.rate_requests.borrow_mut() = Some(requests);
    let weak = Rc::downgrade(ctx);
    glib::spawn_future_local(async move {
        while updates.recv().await.is_ok() {
            let Some(ctx) = weak.upgrade() else { break };
            ctx.emit(Change::Rates);
        }
    });
}

/// Download the rate `query` needs, unless a fresh one is saved. Cheap for ordinary queries.
pub fn request(ctx: &AppContext, query: &str) {
    if kebloop_core::CalculatorEngine::currency_pair(query).is_none() {
        return;
    }
    if let Some(requests) = ctx.rate_requests.borrow().as_ref() {
        let _ = requests.try_send(query.to_owned());
    }
}
