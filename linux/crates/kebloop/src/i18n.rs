//! Translation lookup for user-visible strings. Keys are the English texts, as in the macOS
//! catalogs; unknown keys render as themselves.

use std::sync::OnceLock;

use kebloop_services::l10n::{FormatArg, Localizer};

static LOCALIZER: OnceLock<Localizer> = OnceLock::new();

/// Install the process-wide localizer (first call wins).
pub fn init(localizer: Localizer) -> &'static Localizer {
    LOCALIZER.get_or_init(|| localizer)
}

/// The process-wide localizer (English until [`init`] ran).
pub fn localizer() -> &'static Localizer {
    LOCALIZER.get_or_init(Localizer::english)
}

/// Translate `key`.
pub fn tr(key: &str) -> String {
    localizer().tr(key)
}

/// Translate `key` and substitute `%@`/`%d` style arguments.
pub fn trf(key: &str, args: &[FormatArg]) -> String {
    localizer().format(key, args)
}

#[cfg(test)]
mod tests {
    use kebloop_services::l10n::{Catalog, Language, catalog_keys, linux_catalog, shared_catalog};

    /// Every literal passed to `tr("…")` / `trf("…", …)` / `.tr("…")` in the sources must exist
    /// in the English and the Polish catalogs, so no user-visible string silently stays English.
    #[test]
    fn every_translated_key_has_english_and_polish_text() {
        let sources: &[(&str, &str)] = &[
            ("app", include_str!("app.rs")),
            ("context", include_str!("context.rs")),
            ("direct_paste", include_str!("direct_paste.rs")),
            ("platform", include_str!("platform.rs")),
            ("help", include_str!("help.rs")),
            ("onboarding", include_str!("onboarding.rs")),
            ("settings/mod", include_str!("settings/mod.rs")),
            ("settings/about", include_str!("settings/about.rs")),
            ("settings/aliases", include_str!("settings/aliases.rs")),
            (
                "settings/appearance",
                include_str!("settings/appearance.rs"),
            ),
            ("settings/clipboard", include_str!("settings/clipboard.rs")),
            (
                "settings/configuration",
                include_str!("settings/configuration.rs"),
            ),
            (
                "settings/extensions",
                include_str!("settings/extensions.rs"),
            ),
            ("settings/general", include_str!("settings/general.rs")),
            ("settings/logic", include_str!("settings/logic.rs")),
            ("settings/profiles", include_str!("settings/profiles.rs")),
            (
                "settings/quicklinks",
                include_str!("settings/quicklinks.rs"),
            ),
            ("settings/recorder", include_str!("settings/recorder.rs")),
            ("settings/scripts", include_str!("settings/scripts.rs")),
            ("settings/search", include_str!("settings/search.rs")),
            ("settings/shortcuts", include_str!("settings/shortcuts.rs")),
            ("settings/snippets", include_str!("settings/snippets.rs")),
            ("settings/widgets", include_str!("settings/widgets.rs")),
            (
                "settings/window_management",
                include_str!("settings/window_management.rs"),
            ),
            ("icons", include_str!("icons.rs")),
            ("services/mod", include_str!("services/mod.rs")),
            ("services/tray", include_str!("services/tray.rs")),
            ("launcher/mod", include_str!("launcher/mod.rs")),
            ("launcher/model", include_str!("launcher/model.rs")),
            ("launcher/window", include_str!("launcher/window.rs")),
            ("launcher/results", include_str!("launcher/results.rs")),
            ("launcher/actions", include_str!("launcher/actions.rs")),
            ("launcher/control", include_str!("launcher/control.rs")),
            ("launcher/exec", include_str!("launcher/exec.rs")),
            ("launcher/flows", include_str!("launcher/flows.rs")),
            ("services/shortcuts", include_str!("services/shortcuts.rs")),
            ("launcher/views/mod", include_str!("launcher/views/mod.rs")),
            (
                "launcher/views/clipboard",
                include_str!("launcher/views/clipboard.rs"),
            ),
            (
                "launcher/views/emoji",
                include_str!("launcher/views/emoji.rs"),
            ),
            (
                "launcher/views/snippets",
                include_str!("launcher/views/snippets.rs"),
            ),
            (
                "launcher/views/script",
                include_str!("launcher/views/script.rs"),
            ),
            (
                "launcher/views/extension",
                include_str!("launcher/views/extension.rs"),
            ),
        ];
        let english = catalog_keys(Language::English);
        let polish = catalog_keys(Language::Polish);
        let mut missing = Vec::new();
        for (name, text) in sources {
            for key in translated_literals(text) {
                if !english.contains(&key) {
                    missing.push(format!("{name}: no English entry for {key:?}"));
                }
                if !polish.contains(&key) {
                    missing.push(format!("{name}: no Polish entry for {key:?}"));
                }
            }
        }
        assert!(missing.is_empty(), "{}", missing.join("\n"));
    }

    /// Polish must not leave a Linux-only key identical to English when it is a sentence.
    #[test]
    fn linux_polish_overlay_covers_the_english_overlay() {
        let english: &Catalog = linux_catalog(Language::English);
        let polish: &Catalog = linux_catalog(Language::Polish);
        for key in english.keys() {
            assert!(
                polish.contains_key(key) || shared_catalog(Language::Polish).contains_key(key),
                "missing Polish translation for {key:?}"
            );
        }
    }

    fn translated_literals(source: &str) -> Vec<String> {
        let mut found = Vec::new();
        for needle in ["tr(", "trf(", "format("] {
            let mut rest = source;
            while let Some(position) = rest.find(needle) {
                let tail = &rest[position + needle.len()..];
                let previous = rest[..position].chars().last();
                let trimmed = tail.trim_start();
                if previous.is_some_and(|c| c.is_alphanumeric() || c == '_')
                    || !trimmed.starts_with('"')
                {
                    rest = tail;
                    continue;
                }
                let after = &trimmed[1..];
                let mut literal = String::new();
                let mut chars = after.chars();
                let mut closed = false;
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => match chars.next() {
                            Some('n') => literal.push('\n'),
                            Some('"') => literal.push('"'),
                            Some('\\') => literal.push('\\'),
                            Some('u') => {
                                // \u{2026}
                                let mut digits = String::new();
                                for d in chars.by_ref() {
                                    if d == '}' {
                                        break;
                                    }
                                    if d != '{' {
                                        digits.push(d);
                                    }
                                }
                                if let Some(ch) = u32::from_str_radix(&digits, 16)
                                    .ok()
                                    .and_then(char::from_u32)
                                {
                                    literal.push(ch);
                                }
                            }
                            Some(other) => literal.push(other),
                            None => break,
                        },
                        '"' => {
                            closed = true;
                            break;
                        }
                        c => literal.push(c),
                    }
                }
                if closed {
                    found.push(literal);
                }
                rest = after;
            }
        }
        found
    }
}
