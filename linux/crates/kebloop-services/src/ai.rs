//! AI handoff provider setting. The macOS "local (Siri)" provider does not exist on Linux: a
//! stored `local` / `siri` value falls back to ChatGPT. Prompt URLs live in `kebloop-core`.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// Cloud provider a prompt is handed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AiProvider {
    /// ChatGPT (default).
    #[default]
    OpenAi,
    /// Claude.
    Claude,
}

impl AiProvider {
    /// Providers offered on Linux.
    pub const ALL: [AiProvider; 2] = [Self::OpenAi, Self::Claude];

    /// Persisted value (identical to the macOS raw values).
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Claude => "claude",
        }
    }

    /// Parse a stored value; anything unknown (including `local` / `siri`) becomes ChatGPT.
    pub fn from_stored(value: Option<&str>) -> Self {
        match value {
            Some("claude") => Self::Claude,
            _ => Self::OpenAi,
        }
    }

    /// Short display name.
    pub fn title(self) -> &'static str {
        match self {
            Self::OpenAi => "ChatGPT",
            Self::Claude => "Claude",
        }
    }

    /// Settings display name.
    pub fn settings_title(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI (ChatGPT)",
            Self::Claude => "Claude",
        }
    }

    /// Provider home page.
    pub fn website_url(self) -> &'static str {
        match self {
            Self::OpenAi => "https://chatgpt.com/",
            Self::Claude => "https://claude.ai/new",
        }
    }

    /// Localization key (English text) of the settings explanation.
    pub fn explanation_key(self) -> &'static str {
        match self {
            Self::OpenAi => "Opens ChatGPT with your query already filled in.",
            Self::Claude => {
                "Opens Claude with your query in the link. You may need to sign in first."
            }
        }
    }
}

impl fmt::Display for AiProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.title())
    }
}

impl Serialize for AiProvider {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.raw_value())
    }
}

impl<'de> Deserialize<'de> for AiProvider {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Ok(Self::from_stored(value.as_str()))
    }
}

/// Trim a prompt and reject empty ones (the message matches the macOS app).
pub fn normalize_prompt(text: &str) -> Result<String, &'static str> {
    let prompt = text.trim();
    if prompt.is_empty() {
        Err("Enter a prompt first.")
    } else {
        Ok(prompt.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_values_fall_back_to_openai() {
        assert_eq!(AiProvider::from_stored(None), AiProvider::OpenAi);
        assert_eq!(
            AiProvider::from_stored(Some("unknown-provider")),
            AiProvider::OpenAi
        );
        assert_eq!(AiProvider::from_stored(Some("local")), AiProvider::OpenAi);
        assert_eq!(AiProvider::from_stored(Some("siri")), AiProvider::OpenAi);
        assert_eq!(AiProvider::from_stored(Some("claude")), AiProvider::Claude);
    }

    #[test]
    fn provider_metadata() {
        assert_eq!(AiProvider::ALL, [AiProvider::OpenAi, AiProvider::Claude]);
        assert_eq!(AiProvider::OpenAi.website_url(), "https://chatgpt.com/");
        assert_eq!(AiProvider::Claude.website_url(), "https://claude.ai/new");
        assert_eq!(AiProvider::OpenAi.settings_title(), "OpenAI (ChatGPT)");
        assert!(AiProvider::OpenAi.explanation_key().contains("filled in"));
        assert!(
            AiProvider::Claude
                .explanation_key()
                .contains("query in the link")
        );
    }

    #[test]
    fn serde_is_lenient() {
        let p: AiProvider = serde_json::from_str("\"siri\"").unwrap();
        assert_eq!(p, AiProvider::OpenAi);
        let p: AiProvider = serde_json::from_str("42").unwrap();
        assert_eq!(p, AiProvider::OpenAi);
        assert_eq!(
            serde_json::to_string(&AiProvider::Claude).unwrap(),
            "\"claude\""
        );
    }

    #[test]
    fn prompts_are_trimmed_and_must_not_be_empty() {
        assert_eq!(
            normalize_prompt("\n  Héllo 🌍\nsecond line  \t\n").unwrap(),
            "Héllo 🌍\nsecond line"
        );
        assert_eq!(normalize_prompt(" \n\t "), Err("Enter a prompt first."));
    }
}
