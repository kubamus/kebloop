//! Prompt URLs that prefill a prompt in a provider's web interface.
//!
//! Siri (macOS only) is not available on Linux; a stored `local` (Siri) provider falls back
//! to ChatGPT through [`AiProvider::from_stored`].

use std::fmt::Write as _;

/// Providers that can receive a prompt through a URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AiProvider {
    /// <https://chatgpt.com>
    #[default]
    ChatGpt,
    /// <https://claude.ai>
    Claude,
}

impl AiProvider {
    /// Parses a stored provider identifier; unknown values and the macOS-only `local` (Siri)
    /// fall back to ChatGPT.
    pub fn from_stored(identifier: &str) -> Self {
        match identifier.trim().to_ascii_lowercase().as_str() {
            "claude" => Self::Claude,
            _ => Self::ChatGpt,
        }
    }

    /// The identifier used in stored configuration.
    pub fn identifier(self) -> &'static str {
        match self {
            Self::ChatGpt => "openai",
            Self::Claude => "claude",
        }
    }

    /// The URL that opens this provider with `prompt` prefilled, or `None` for a blank prompt.
    pub fn url(self, prompt: &str) -> Option<String> {
        match self {
            Self::ChatGpt => AiPromptUrl::chatgpt(prompt),
            Self::Claude => AiPromptUrl::claude(prompt),
        }
    }
}

/// Builds provider URLs that prefill a prompt in the provider's web interface.
#[derive(Debug, Clone, Copy)]
pub struct AiPromptUrl;

impl AiPromptUrl {
    /// `https://chatgpt.com/?q=<prompt>`; `None` when the prompt is blank.
    pub fn chatgpt(prompt: &str) -> Option<String> {
        make_url("chatgpt.com", "/", prompt)
    }

    /// `https://claude.ai/new?q=<prompt>`; `None` when the prompt is blank.
    pub fn claude(prompt: &str) -> Option<String> {
        make_url("claude.ai", "/new", prompt)
    }
}

fn make_url(host: &str, path: &str, prompt: &str) -> Option<String> {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(format!(
        "https://{host}{path}?q={}",
        percent_encode(trimmed)
    ))
}

/// Encodes everything except RFC 3986 unreserved characters, so `+`, `&`, `#` and
/// newlines are data and never read as form separators or spaces.
fn percent_encode(text: &str) -> String {
    let mut output = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            output.push(char::from(byte));
        } else {
            let _ = write!(output, "%{byte:02X}");
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_reserved_characters() {
        assert_eq!(percent_encode("a b+c&d#é\n"), "a%20b%2Bc%26d%23%C3%A9%0A");
    }

    #[test]
    fn stored_providers_fall_back_to_chatgpt() {
        assert_eq!(AiProvider::from_stored("local"), AiProvider::ChatGpt);
        assert_eq!(AiProvider::from_stored("Claude"), AiProvider::Claude);
        assert_eq!(AiProvider::from_stored(""), AiProvider::ChatGpt);
    }
}
