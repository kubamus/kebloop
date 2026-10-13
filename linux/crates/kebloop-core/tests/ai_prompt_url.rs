//! Port of `Tests/KebloopCoreTests/AIPromptURLTests.swift`.

use kebloop_core::{AiPromptUrl, AiProvider};

/// Splits `https://host/path?q=value` and returns (host, path, raw query value).
fn split(url: &str) -> (&str, &str, &str) {
    let rest = url.strip_prefix("https://").expect("https scheme");
    let (authority_and_path, query) = rest.split_once('?').expect("query");
    let slash = authority_and_path.find('/').expect("path");
    let (host, path) = authority_and_path.split_at(slash);
    (host, path, query.strip_prefix("q=").expect("q parameter"))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).expect("hex digits");
            decoded.push(u8::from_str_radix(hex, 16).expect("valid hex"));
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).expect("utf-8")
}

#[test]
fn chatgpt_uses_fixed_origin_and_encodes_prompt_as_query() {
    let prompt = "  Héllo 🌍 + 2\nWhat about a & b? #tag  ";
    let url = AiPromptUrl::chatgpt(prompt).expect("url");
    let (host, path, query) = split(&url);

    assert_eq!(host, "chatgpt.com");
    assert_eq!(path, "/");
    assert_eq!(
        percent_decode(query),
        "Héllo 🌍 + 2\nWhat about a & b? #tag"
    );
    assert!(!query.contains('+'));
    assert!(!query.contains(['&', '#', ' ', '\n']));
}

#[test]
fn claude_uses_new_chat_path_and_preserves_prompt_exactly_after_outer_trim() {
    let url = AiPromptUrl::claude("\n  summarize this\nsecond line\t ").expect("url");
    let (host, path, query) = split(&url);

    assert_eq!(host, "claude.ai");
    assert_eq!(path, "/new");
    assert_eq!(percent_decode(query), "summarize this\nsecond line");
}

#[test]
fn empty_or_whitespace_prompt_has_no_url() {
    assert!(AiPromptUrl::chatgpt(" \n\t ").is_none());
    assert!(AiPromptUrl::claude("").is_none());
    assert!(AiProvider::Claude.url("  ").is_none());
}

#[test]
fn providers_dispatch_to_their_origin() {
    assert!(
        AiProvider::ChatGpt
            .url("hi")
            .is_some_and(|url| url.starts_with("https://chatgpt.com/?q="))
    );
    assert!(
        AiProvider::Claude
            .url("hi")
            .is_some_and(|url| url.starts_with("https://claude.ai/new?q="))
    );
}
