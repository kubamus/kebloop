//! Case- and diacritic-insensitive text folding shared by the index modules.

/// Folds `s` for comparison the same way the rest of Kebloop does (and as Foundation's
/// case- and diacritic-insensitive comparison does on macOS).
pub use kebloop_services::util::fold;

#[cfg(test)]
mod tests {
    use super::fold;

    #[test]
    fn folds_case_and_decomposable_diacritics_only() {
        assert_eq!(fold("Zażółć GĘŚLĄ jaźń"), "zazołc gesla jazn");
        assert_eq!(fold("Éclair"), "eclair");
        assert_eq!(fold("Straße"), "straße");
        assert_eq!(fold("ABC"), "abc");
    }
}
