//! Application aliases: one alias per application, unique after normalisation.

use std::collections::{BTreeMap, BTreeSet};

use crate::util::fold;

/// Maximum alias length in characters.
pub const MAX_ALIAS_LENGTH: usize = 100;

/// Alias validation failures (messages match the macOS app).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AliasError {
    /// Alias exceeds [`MAX_ALIAS_LENGTH`].
    #[error("Aliases can be up to {MAX_ALIAS_LENGTH} characters.")]
    TooLong,
    /// Alias has no letter or number.
    #[error("Use at least one letter or number in the alias.")]
    NoAlphanumeric,
    /// Alias is used by another application.
    #[error("That alias is already assigned to another app.")]
    Duplicate,
}

/// Fold case and diacritics and reduce to space-separated alphanumeric words.
pub fn normalized(value: &str) -> String {
    fold(value)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Save an alias for `app_key` (desktop id or path). An empty alias removes the current one.
pub fn save_alias(
    aliases: &mut BTreeMap<String, String>,
    app_key: &str,
    raw_alias: &str,
) -> Result<(), AliasError> {
    let alias = raw_alias.trim();
    if alias.is_empty() {
        aliases.remove(app_key);
        return Ok(());
    }
    if alias.chars().count() > MAX_ALIAS_LENGTH {
        return Err(AliasError::TooLong);
    }
    let wanted = normalized(alias);
    if wanted.is_empty() {
        return Err(AliasError::NoAlphanumeric);
    }
    if aliases
        .iter()
        .any(|(key, value)| key != app_key && normalized(value) == wanted)
    {
        return Err(AliasError::Duplicate);
    }
    aliases.insert(app_key.to_string(), alias.to_string());
    Ok(())
}

/// Remove an application's alias.
pub fn remove_alias(aliases: &mut BTreeMap<String, String>, app_key: &str) {
    aliases.remove(app_key);
}

/// Sanitise stored aliases on load: trim, drop empty/overlong/non-alphanumeric ones and keep the
/// first (by key order) of any normalised duplicates.
pub fn clean(stored: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut cleaned = BTreeMap::new();
    let mut used = BTreeSet::new();
    for (key, value) in stored {
        let alias = value.trim();
        let norm = normalized(alias);
        if alias.is_empty()
            || alias.chars().count() > MAX_ALIAS_LENGTH
            || norm.is_empty()
            || !used.insert(norm)
        {
            continue;
        }
        cleaned.insert(key.clone(), alias.to_string());
    }
    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_trim_and_can_be_removed() {
        let mut aliases = BTreeMap::new();
        save_alias(&mut aliases, "ghostty.desktop", "  terminal  ").unwrap();
        assert_eq!(aliases["ghostty.desktop"], "terminal");
        save_alias(&mut aliases, "ghostty.desktop", "   ").unwrap();
        assert!(aliases.is_empty());
    }

    #[test]
    fn aliases_reject_normalized_duplicates_and_overlong_values() {
        let mut aliases = BTreeMap::new();
        save_alias(&mut aliases, "ghostty", "Résumé").unwrap();
        assert_eq!(
            save_alias(&mut aliases, "terminal", "resume"),
            Err(AliasError::Duplicate)
        );
        assert!(!aliases.contains_key("terminal"));
        assert_eq!(
            save_alias(&mut aliases, "ghostty", &"a".repeat(101)),
            Err(AliasError::TooLong)
        );
        assert_eq!(aliases["ghostty"], "Résumé");
        assert_eq!(
            save_alias(&mut aliases, "x", "!!!"),
            Err(AliasError::NoAlphanumeric)
        );
        // Re-saving the same app with a normalised-equal alias is allowed.
        save_alias(&mut aliases, "ghostty", "RESUME").unwrap();
    }

    #[test]
    fn clean_drops_invalid_and_duplicate_entries() {
        let stored: BTreeMap<String, String> =
            [("a", " Term "), ("b", "term"), ("c", ""), ("d", "???")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
        let cleaned = clean(&stored);
        assert_eq!(cleaned.len(), 1);
        assert_eq!(cleaned["a"], "Term");
    }
}
