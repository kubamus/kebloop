//! The launcher's top-level window: a layer-shell overlay where the compositor supports it,
//! otherwise an undecorated fixed-size window.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

/// Fraction of the output height between the top edge and the panel.
pub const TOP_MARGIN_FRACTION: f64 = 0.20;
/// Layer-shell namespace (matched by compositor layer rules for blur).
pub const NAMESPACE: &str = "kebloop";

/// The layer-shell library must not be queried with an X11 display.
pub(crate) fn layer_shell_supported() -> bool {
    gdk::Display::default().is_some_and(|display| display.type_().name() == "GdkWaylandDisplay")
        && gtk4_layer_shell::is_supported()
}

/// Top margin for an output of `output_height` logical pixels, never negative.
pub fn top_margin(output_height: i32) -> i32 {
    (f64::from(output_height.max(0)) * TOP_MARGIN_FRACTION).round() as i32
}

/// Height of the output the margin is computed from. Priority: the output chosen in settings,
/// the output the surface actually entered, the last height used, the first output, a default.
pub fn pick_output_height(
    fixed: Option<i32>,
    entered: Option<i32>,
    last_used: Option<i32>,
    first: Option<i32>,
) -> i32 {
    fixed.or(entered).or(last_used).or(first).unwrap_or(1080)
}

/// Keyboard mode of the layer surface. Exclusive keeps the keyboard with the panel while it is
/// shown; while one of our own dialogs or the action popover is open the surface must not hold
/// the keyboard exclusively (the compositor would steal keys from them), so it drops to on-demand.
pub fn keyboard_mode(exclusive: bool) -> KeyboardMode {
    if exclusive {
        KeyboardMode::Exclusive
    } else {
        KeyboardMode::OnDemand
    }
}

/// Wrapper that owns the window and knows how it is positioned.
#[derive(Clone)]
pub struct PanelWindow {
    window: gtk::Window,
    layer_shell: bool,
    margin: Rc<Cell<i32>>,
    /// Output height the current margin was computed from.
    height: Rc<Cell<Option<i32>>>,
    /// Output chosen in settings for the next show.
    fixed: Rc<RefCell<Option<gdk::Monitor>>>,
    /// Output the surface reported entering.
    entered: Rc<RefCell<Option<gdk::Monitor>>>,
    width: Rc<Cell<i32>>,
}

fn apply_margin(window: &gtk::Window, current: &Cell<i32>, output_height: i32) {
    let margin = top_margin(output_height);
    if margin != current.get() {
        current.set(margin);
        window.set_margin(Edge::Top, margin);
    }
}

impl PanelWindow {
    /// Build the (hidden) window. Must run after GTK is initialised.
    pub fn new(app: &adw::Application, fixed_output: Option<gdk::Monitor>) -> Self {
        let window = gtk::Window::builder()
            .application(app)
            .decorated(false)
            .resizable(false)
            .hide_on_close(true)
            .css_classes(["kb-window"])
            .title("Kebloop")
            .build();
        let layer_shell = layer_shell_supported();
        let margin = Rc::new(Cell::new(0));
        if layer_shell {
            window.init_layer_shell();
            window.set_layer(Layer::Overlay);
            window.set_namespace(Some(NAMESPACE));
            window.set_anchor(Edge::Top, true);
            window.set_anchor(Edge::Left, false);
            window.set_anchor(Edge::Right, false);
            window.set_anchor(Edge::Bottom, false);
            window.set_exclusive_zone(0);
            window.set_keyboard_mode(keyboard_mode(true));
            if let Some(monitor) = &fixed_output {
                window.set_monitor(Some(monitor));
            }
        }
        let panel = Self {
            window,
            layer_shell,
            margin,
            height: Rc::new(Cell::new(None)),
            fixed: Rc::new(RefCell::new(fixed_output)),
            entered: Rc::new(RefCell::new(None)),
            width: Rc::new(Cell::new(-1)),
        };
        if layer_shell {
            panel.follow_entered_output();
        }
        panel.update_margin_from_display();
        panel
    }

    /// The margin depends on the output the compositor really put the panel on, which is only
    /// known once the surface reports entering it: recompute then.
    fn follow_entered_output(&self) {
        let (margin, height, entered) = (
            self.margin.clone(),
            self.height.clone(),
            self.entered.clone(),
        );
        let fixed = self.fixed.clone();
        self.window.connect_realize(move |window| {
            let Some(surface) = window.surface() else {
                return;
            };
            let (window, margin, height) = (window.downgrade(), margin.clone(), height.clone());
            let (entered, fixed) = (entered.clone(), fixed.clone());
            let left = entered.clone();
            surface.connect_enter_monitor(move |_, monitor| {
                let Some(window) = window.upgrade() else {
                    return;
                };
                *entered.borrow_mut() = Some(monitor.clone());
                let used = pick_output_height(
                    fixed.borrow().as_ref().map(|m| m.geometry().height()),
                    Some(monitor.geometry().height()),
                    height.get(),
                    None,
                );
                height.set(Some(used));
                apply_margin(&window, &margin, used);
            });
            surface.connect_leave_monitor(move |_, _| {
                *left.borrow_mut() = None;
            });
        });
    }

    /// Switch between exclusive keyboard grab (panel shown, nothing else open) and on-demand
    /// (a dialog or popover of ours needs the keyboard).
    pub fn set_keyboard_exclusive(&self, exclusive: bool) {
        if self.layer_shell {
            self.window.set_keyboard_mode(keyboard_mode(exclusive));
        }
    }

    /// The GTK window.
    pub fn window(&self) -> &gtk::Window {
        &self.window
    }

    /// Show the panel on `output` (`None`: the compositor's focused output). Takes effect the
    /// next time the panel is shown.
    pub fn set_output(&self, output: Option<&gdk::Monitor>) {
        if self.layer_shell {
            self.window.set_monitor(output);
            *self.fixed.borrow_mut() = output.cloned();
            if output.is_none() {
                *self.entered.borrow_mut() = None;
            }
            self.update_margin_from_display();
        }
    }

    /// Set the content width.
    pub fn set_width(&self, width: i32) {
        self.width.set(width);
        self.window.set_default_size(width, -1);
    }

    /// Ask the window to take the natural height of its content again. GTK keeps the size a
    /// window grew to, so after a long list is replaced by a single result the panel would
    /// otherwise keep an empty area below it.
    pub fn fit_to_content(&self) {
        if self.window.is_visible() {
            self.window.set_default_size(self.width.get(), -1);
            self.window.queue_resize();
        }
    }

    /// Pick the margin from the output chosen in settings, else the output the surface entered;
    /// before either is known the last used height (or the first output) stands in.
    pub fn update_margin_from_display(&self) {
        if !self.layer_shell {
            return;
        }
        let height_of = |monitor: &gdk::Monitor| monitor.geometry().height();
        let fixed = self.fixed.borrow().as_ref().map(height_of);
        let entered = self.entered.borrow().as_ref().map(height_of);
        let first = || {
            gdk::Display::default()
                .and_then(|display| display.monitors().item(0))
                .and_then(|object| object.downcast::<gdk::Monitor>().ok())
                .map(|monitor| height_of(&monitor))
        };
        let used = pick_output_height(
            fixed,
            entered,
            self.height.get(),
            if fixed.is_none() && entered.is_none() && self.height.get().is_none() {
                first()
            } else {
                None
            },
        );
        self.height.set(Some(used));
        apply_margin(&self.window, &self.margin, used);
    }

    /// Show the window.
    pub fn present(&self) {
        self.update_margin_from_display();
        self.window.present();
    }

    /// Hide the window.
    pub fn hide(&self) {
        self.window.set_visible(false);
    }

    /// Whether the window is currently shown.
    pub fn is_visible(&self) -> bool {
        self.window.is_visible()
    }
}

#[cfg(test)]
mod tests {
    use super::{pick_output_height, top_margin};
    use gtk4_layer_shell::KeyboardMode;

    #[test]
    fn margin_is_a_fifth_of_the_output_height() {
        assert_eq!(top_margin(1200), 240);
        assert_eq!(top_margin(1080), 216);
        assert_eq!(top_margin(-5), 0);
    }

    #[test]
    fn margin_follows_the_output_the_panel_is_really_on() {
        // The configured output wins, then the one the surface entered, then what was used last.
        assert_eq!(
            pick_output_height(Some(1440), Some(1080), Some(900), Some(720)),
            1440
        );
        assert_eq!(
            pick_output_height(None, Some(1080), Some(900), Some(720)),
            1080
        );
        assert_eq!(pick_output_height(None, None, Some(900), Some(720)), 900);
        assert_eq!(pick_output_height(None, None, None, Some(720)), 720);
        assert_eq!(pick_output_height(None, None, None, None), 1080);
    }

    #[test]
    fn keyboard_is_exclusive_unless_a_dialog_or_popover_is_open() {
        assert_eq!(super::keyboard_mode(true), KeyboardMode::Exclusive);
        assert_eq!(super::keyboard_mode(false), KeyboardMode::OnDemand);
    }
}
