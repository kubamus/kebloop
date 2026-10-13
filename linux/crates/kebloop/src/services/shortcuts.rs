//! Global shortcuts through the XDG GlobalShortcuts portal, when the desktop offers it.
//! Without the portal nothing is registered; `kebloop toggle` / `command` / `launch` stay
//! available for compositor key bindings (the settings window shows ready-made snippets).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use kebloop_platform::shortcuts::{
    PortalShortcuts, PortalSupport, ShortcutEventKind, ShortcutSpec, detect_portal_support,
    start_portal_shortcuts,
};

use crate::cli::Request;
use crate::context::{AppContext, Change, PortalState};

const LAUNCHER_ID: &str = "launcher";
const COMMAND_PREFIX: &str = "command:";
const APP_PREFIX: &str = "app:";

/// Numbers the portal registrations. Setting up a registration can take long (the portal may
/// wait for a confirmation dialog), so an older one can finish after a newer one was started;
/// its result must then be ignored.
#[derive(Debug, Default)]
struct Generations(Cell<u64>);

impl Generations {
    /// Start a new registration; every earlier one becomes stale.
    fn begin(&self) -> u64 {
        self.0.set(self.0.get() + 1);
        self.0.get()
    }

    /// Whether `generation` is still the latest registration.
    fn is_current(&self, generation: u64) -> bool {
        self.0.get() == generation
    }
}

/// What the portal should know about; compared to avoid re-binding for unrelated changes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Wanted {
    specs: Vec<(String, String, Option<String>)>,
}

/// The shortcuts the active profile asks for.
fn wanted(ctx: &AppContext) -> Wanted {
    ctx.with_settings(|settings| {
        let mut specs = vec![(
            LAUNCHER_ID.to_owned(),
            crate::i18n::tr("Open launcher"),
            Some(settings.launcher_accelerator().to_string()),
        )];
        for binding in &settings.command_shortcuts {
            specs.push((
                format!("{COMMAND_PREFIX}{}", binding.id),
                binding.label.clone(),
                binding.resolved().map(|a| a.to_string()),
            ));
        }
        for binding in &settings.app_shortcuts {
            specs.push((
                format!("{APP_PREFIX}{}", binding.app_path),
                binding.app_name.clone(),
                binding.resolved().map(|a| a.to_string()),
            ));
        }
        Wanted { specs }
    })
}

/// Register the portal shortcuts now and whenever the bindings change.
pub fn init(ctx: &Rc<AppContext>) {
    set_state(ctx, PortalState::Checking);
    let state: Rc<RefCell<Option<Wanted>>> = Rc::new(RefCell::new(None));
    let handle: Rc<RefCell<Option<PortalShortcuts>>> = Rc::new(RefCell::new(None));
    let generations = Rc::new(Generations::default());
    register(ctx, &state, &handle, &generations);
    let listening = ctx.clone();
    ctx.subscribe(Rc::new(move |change| {
        if matches!(change, Change::Settings | Change::Profile) {
            register(&listening, &state, &handle, &generations);
        }
    }));
}

fn set_state(ctx: &AppContext, state: PortalState) {
    if *ctx.portal.borrow() != state {
        *ctx.portal.borrow_mut() = state;
        ctx.emit(Change::Status);
    }
}

fn register(
    ctx: &Rc<AppContext>,
    state: &Rc<RefCell<Option<Wanted>>>,
    handle: &Rc<RefCell<Option<PortalShortcuts>>>,
    generations: &Rc<Generations>,
) {
    let wanted = wanted(ctx);
    if state.borrow().as_ref() == Some(&wanted) {
        return;
    }
    *state.borrow_mut() = Some(wanted.clone());
    handle.borrow_mut().take();
    let generation = generations.begin();

    let specs: Vec<ShortcutSpec> = wanted
        .specs
        .iter()
        .filter_map(|(id, description, accelerator)| {
            let spec = ShortcutSpec::new(id.clone(), description.clone());
            match accelerator {
                Some(accelerator) => spec
                    .with_accelerator(accelerator)
                    .ok()
                    .or(Some(ShortcutSpec::new(id.clone(), description.clone()))),
                None => Some(spec),
            }
        })
        .collect();
    let (sender, receiver) = async_channel::bounded(1);
    let spawned = std::thread::Builder::new()
        .name("kebloop-portal-setup".into())
        .spawn(move || {
            if let PortalSupport::Unavailable(reason) = detect_portal_support() {
                let _ = sender.send_blocking(Err(reason));
                return;
            }
            // May wait for the user to confirm the binding dialog.
            let started = start_portal_shortcuts(specs).map_err(|error| error.to_string());
            let _ = sender.send_blocking(started);
        });
    if spawned.is_err() {
        return;
    }
    let ctx = ctx.clone();
    let handle = handle.clone();
    let generations = generations.clone();
    glib::spawn_future_local(async move {
        let result = receiver.recv().await;
        if !generations.is_current(generation) {
            // A newer registration was started meanwhile; dropping `result` closes this session.
            return;
        }
        match result {
            Ok(Ok((session, events))) => {
                let bound = session
                    .bound
                    .iter()
                    .map(|b| (b.id.clone(), b.trigger_description.clone()))
                    .collect();
                *handle.borrow_mut() = Some(session);
                set_state(&ctx, PortalState::Active(bound));
                while let Ok(event) = events.recv().await {
                    if !generations.is_current(generation) {
                        break;
                    }
                    if event.kind != ShortcutEventKind::Activated {
                        continue;
                    }
                    let request = if event.id == LAUNCHER_ID {
                        Request::Toggle
                    } else if let Some(id) = event.id.strip_prefix(COMMAND_PREFIX) {
                        Request::Command(id.to_owned())
                    } else if let Some(id) = event.id.strip_prefix(APP_PREFIX) {
                        Request::Launch(id.to_owned())
                    } else {
                        continue;
                    };
                    crate::app::dispatch(&ctx, request);
                }
            }
            Ok(Err(reason)) => {
                eprintln!("kebloop: global shortcuts unavailable: {reason}");
                set_state(&ctx, PortalState::Unavailable(reason));
            }
            Err(_) => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::Generations;

    #[test]
    fn only_the_latest_registration_is_current() {
        let generations = Generations::default();
        let first = generations.begin();
        assert!(generations.is_current(first));
        let second = generations.begin();
        assert!(
            !generations.is_current(first),
            "a stale result must be ignored"
        );
        assert!(generations.is_current(second));
        let third = generations.begin();
        assert!(!generations.is_current(second) && generations.is_current(third));
    }
}
