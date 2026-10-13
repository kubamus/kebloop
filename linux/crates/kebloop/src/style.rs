//! Application-wide styling: the static sheet, the settings-dependent sheet and the colour scheme.

use gtk::gdk;
use kebloop_services::personalization::PersonalizationSettings;

const STATIC_CSS: &str = include_str!("style.css");

/// Geometry derived from the personalization settings. Pure, so it can be tested.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// Text size of result titles in px.
    pub title_px: f64,
    /// Text size of subtitles in px.
    pub subtitle_px: f64,
    /// Row height in px.
    pub row_height: i32,
    /// Panel width in px.
    pub width: i32,
    /// Icon size in px.
    pub icon_px: i32,
}

impl Metrics {
    /// Compute from settings (mirrors `rowHeight` in the macOS controller).
    pub fn from_settings(settings: &PersonalizationSettings) -> Self {
        let text = settings.text_size.clamp(11.0, 24.0);
        let base: f64 = if settings.compact { 40.0 } else { 48.0 };
        let row: f64 = (text * 2.0 + 16.0).max(base);
        Self {
            title_px: text,
            subtitle_px: (text - 3.0).max(10.0),
            row_height: row.round() as i32,
            width: settings.panel_width.clamp(420.0, 1000.0).round() as i32,
            icon_px: if settings.compact { 28 } else { 32 },
        }
    }
}

/// CSS for the settings-dependent values.
pub fn dynamic_css(metrics: &Metrics) -> String {
    format!(
        "row.kb-row {{ min-height: {row}px; }}\n\
         row.kb-row .kb-title {{ font-size: {title}px; }}\n\
         row.kb-row .kb-subtitle {{ font-size: {subtitle}px; }}\n\
         .kb-input {{ font-size: {input}px; }}\n\
         .kb-header {{ min-height: 56px; }}\n",
        row = metrics.row_height,
        title = metrics.title_px,
        subtitle = metrics.subtitle_px,
        input = (metrics.title_px + 5.0).round(),
    )
}

/// Installs and updates the style sheets for the default display.
pub struct Styling {
    dynamic: gtk::CssProvider,
}

impl Styling {
    /// Register the static sheet and an empty dynamic one. Needs an initialised display.
    pub fn install() -> Self {
        let dynamic = gtk::CssProvider::new();
        if let Some(display) = gdk::Display::default() {
            let fixed = gtk::CssProvider::new();
            fixed.load_from_string(STATIC_CSS);
            gtk::style_context_add_provider_for_display(
                &display,
                &fixed,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            gtk::style_context_add_provider_for_display(
                &display,
                &dynamic,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
            );
        }
        Self { dynamic }
    }

    /// Apply settings: dynamic sizes and the colour scheme.
    pub fn apply(&self, settings: &PersonalizationSettings) {
        self.dynamic
            .load_from_string(&dynamic_css(&Metrics::from_settings(settings)));
        apply_color_scheme(&settings.appearance);
    }
}

/// `system` follows the desktop; `light` and `dark` force a scheme for every Kebloop window.
pub fn apply_color_scheme(appearance: &str) {
    let scheme = match appearance {
        "light" => adw::ColorScheme::ForceLight,
        "dark" => adw::ColorScheme::ForceDark,
        _ => adw::ColorScheme::Default,
    };
    adw::StyleManager::default().set_color_scheme(scheme);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_height_follows_density_and_text_size() {
        let mut settings = PersonalizationSettings::default();
        assert_eq!(Metrics::from_settings(&settings).row_height, 48);
        settings.compact = true;
        assert_eq!(Metrics::from_settings(&settings).row_height, 44);
        settings.text_size = 24.0;
        assert_eq!(Metrics::from_settings(&settings).row_height, 64);
    }

    #[test]
    fn dynamic_css_mentions_every_metric() {
        let css = dynamic_css(&Metrics::from_settings(&PersonalizationSettings::default()));
        assert!(css.contains("min-height: 48px"));
        assert!(css.contains("font-size: 14px"));
    }
}
