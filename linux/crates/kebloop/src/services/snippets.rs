//! Opt-in snippet expansion through focused, editable accessibility text objects.
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use gtk::{glib, prelude::*};
use kebloop_core::Locale;
use kebloop_platform::snippet_expansion::{ExpansionConfig, SnippetExpansion};
use kebloop_services::snippets::{RenderContext, Snippet};

use crate::context::{AppContext, Change};
use crate::i18n::tr;

type RenderRequest = (u64, Snippet, mpsc::SyncSender<Result<String, String>>);

pub fn enabled(ctx: &AppContext) -> bool {
    ctx.config
        .borrow()
        .global()
        .snippet_keyword_expansion_enabled
}

pub fn init(ctx: &Rc<AppContext>) {
    let handle = Rc::new(RefCell::new(None::<SnippetExpansion>));
    let polling_handle = handle.clone();
    let snippets = Arc::new(Mutex::new(ctx.with_settings(|s| s.snippets.clone())));
    let generation = Rc::new(Cell::new(0_u64));
    let starting = Rc::new(Cell::new(false));
    let (requests, renderer) = async_channel::bounded::<RenderRequest>(4);
    let rendering_generation = generation.clone();
    let weak = Rc::downgrade(ctx);
    glib::spawn_future_local(async move {
        while let Ok((version, snippet, reply)) = renderer.recv().await {
            let Some(ctx) = weak.upgrade() else { break };
            if rendering_generation.get() != version
                || !enabled(&ctx)
                || !ctx.with_settings(|s| s.snippets.contains(&snippet))
            {
                let _ = reply.try_send(Err(tr("Snippet expansion is turned off.")));
                continue;
            }
            let clipboard = if snippet.template.contains("{clipboard}") {
                match read_clipboard().await {
                    Ok(text) => text,
                    Err(error) => {
                        let _ = reply.try_send(Err(error));
                        continue;
                    }
                }
            } else {
                String::new()
            };
            if rendering_generation.get() != version
                || !enabled(&ctx)
                || !ctx.with_settings(|s| s.snippets.contains(&snippet))
            {
                let _ = reply.try_send(Err(tr("Snippet changed before expansion.")));
                continue;
            }
            let read = || clipboard.clone();
            let result = snippet
                .render_with(
                    &BTreeMap::new(),
                    &RenderContext {
                        now: jiff::Timestamp::now(),
                        locale: Locale::from_tag(ctx.l10n.language().code()),
                        time_zone: ctx.time_zone.clone(),
                        clipboard: &read,
                    },
                )
                .map_err(|e| e.to_string());
            let _ = reply.try_send(result);
        }
    });

    let weak = Rc::downgrade(ctx);
    let apply: Rc<dyn Fn()> = Rc::new(move || {
        let Some(ctx) = weak.upgrade() else { return };
        if let Ok(mut current) = snippets.lock() {
            *current = ctx.with_settings(|s| s.snippets.clone());
        }
        if !enabled(&ctx) {
            generation.set(generation.get().wrapping_add(1));
            starting.set(false);
            handle.borrow_mut().take();
            *ctx.snippet_expansion_status.borrow_mut() = tr("Off");
            ctx.emit(Change::Status);
            return;
        }
        if handle.borrow().is_some() || starting.replace(true) {
            return;
        }
        generation.set(generation.get().wrapping_add(1));
        let version = generation.get();
        *ctx.snippet_expansion_status.borrow_mut() = tr("Starting…");
        let requests = requests.clone();
        let config = ExpansionConfig {
            snippets: snippets.clone(),
            render: Arc::new(move |snippet| {
                let (reply, result) = mpsc::sync_channel(1);
                requests
                    .try_send((version, snippet.clone(), reply))
                    .map_err(|e| e.to_string())?;
                result
                    .recv_timeout(Duration::from_secs(2))
                    .map_err(|e| e.to_string())?
            }),
        };
        let (handle, generation, starting, ctx) = (
            handle.clone(),
            generation.clone(),
            starting.clone(),
            ctx.clone(),
        );
        let failed_ctx = ctx.clone();
        let failed_starting = starting.clone();
        let spawned = crate::work::spawn(
            "kebloop-snippet-start",
            move || SnippetExpansion::start(config),
            move |result| {
                if generation.get() != version {
                    return;
                }
                starting.set(false);
                match result {
                    Ok(service) => {
                        *handle.borrow_mut() = Some(service);
                        *ctx.snippet_expansion_status.borrow_mut() =
                            tr("Active in accessible text fields");
                    }
                    Err(error) => {
                        *ctx.snippet_expansion_status.borrow_mut() = error;
                    }
                }
                ctx.emit(Change::Status);
            },
        );
        if !spawned {
            failed_starting.set(false);
            *failed_ctx.snippet_expansion_status.borrow_mut() =
                tr("Could not start snippet expansion.");
            failed_ctx.emit(Change::Status);
        }
    });
    let update = apply.clone();
    ctx.subscribe(Rc::new(move |change| {
        if matches!(change, Change::Settings | Change::Profile) {
            update();
        }
    }));
    apply();
    let weak = Rc::downgrade(ctx);
    glib::timeout_add_seconds_local(5, move || {
        let Some(ctx) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let status = polling_handle.borrow().as_ref().and_then(|service| {
            service
                .last_error()
                .or_else(|| (!service.is_running()).then(|| tr("Accessibility service stopped.")))
        });
        if let Some(status) = status {
            if *ctx.snippet_expansion_status.borrow() != status {
                *ctx.snippet_expansion_status.borrow_mut() = status;
                ctx.emit(Change::Status);
            }
            if polling_handle
                .borrow()
                .as_ref()
                .is_some_and(|service| !service.is_running())
            {
                polling_handle.borrow_mut().take();
            }
        }
        glib::ControlFlow::Continue
    });
}

async fn read_clipboard() -> Result<String, String> {
    let display = gtk::gdk::Display::default().ok_or_else(|| tr("Clipboard is unavailable."))?;
    let read = async {
        display
            .clipboard()
            .read_text_future()
            .await
            .map(|value| value.map(|s| s.to_string()).unwrap_or_default())
            .map_err(|e| e.to_string())
    };
    let timeout = async {
        glib::timeout_future(Duration::from_millis(800)).await;
        Err(tr("Clipboard read timed out."))
    };
    futures_lite::future::or(read, timeout).await
}
