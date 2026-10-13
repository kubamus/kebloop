//! Quicklinks and web-search templates: target validation, `{query}` templating and keyword rules.

use std::path::PathBuf;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::util::{expand_tilde, new_id, normalize_path};

/// Everything except ASCII alphanumerics and `-._~` is percent-encoded in `{query}` values.
const QUERY_ENCODE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

const QUERY_PLACEHOLDER: &str = "{query}";
const HOST_MARKER: &str = "KEBLOOPQUERY";

/// What a quicklink resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// An `http(s)` URL.
    Url(Url),
    /// A local file or folder.
    File(PathBuf),
}

impl Destination {
    /// `true` for local files.
    pub fn is_file(&self) -> bool {
        matches!(self, Self::File(_))
    }
}

/// Resolve a quicklink target. `argument` replaces `{query}` (percent-encoded); `home` expands `~/`.
/// Returns `None` for unsafe or malformed targets.
pub fn destination(target: &str, argument: &str, home: &str) -> Option<Destination> {
    let mut value = target.trim().to_string();
    if value.contains(QUERY_PLACEHOLDER) {
        if value.starts_with('/') || value.starts_with("~/") {
            return None;
        }
        // Placeholders must sit in a path, query or fragment, never in the authority.
        let probe = Url::parse(&value.replace(QUERY_PLACEHOLDER, HOST_MARKER)).ok()?;
        let marker = HOST_MARKER.to_ascii_lowercase();
        let in_authority = probe
            .host_str()
            .is_some_and(|h| h.to_ascii_lowercase().contains(&marker))
            || probe.username().contains(HOST_MARKER)
            || probe.password().is_some_and(|p| p.contains(HOST_MARKER));
        if in_authority {
            return None;
        }
        let encoded = utf8_percent_encode(argument, QUERY_ENCODE).to_string();
        value = value.replace(QUERY_PLACEHOLDER, &encoded);
    }
    if value.starts_with('/') || value.starts_with("~/") {
        let expanded = expand_tilde(&value, home);
        return Some(Destination::File(PathBuf::from(normalize_path(&expanded))));
    }
    if value.contains('{') || value.contains('}') {
        return None;
    }
    let url = Url::parse(&value).ok()?;
    let scheme = url.scheme().to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    match url.host_str() {
        Some(host) if !host.is_empty() => Some(Destination::Url(url)),
        _ => None,
    }
}

/// A named link or file shortcut, optionally triggered by a keyword.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Quicklink {
    /// Stable identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// URL, template or path.
    pub target: String,
    /// Optional single-word trigger (stored lower-case).
    #[serde(default)]
    pub keyword: String,
}

impl Quicklink {
    /// Resolve with an argument for `{query}` templates.
    pub fn resolved(&self, argument: &str, home: &str) -> Option<Destination> {
        destination(&self.target, argument, home)
    }
}

/// A web search template, e.g. `https://duckduckgo.com/?q={query}` with keyword `web`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WebSearchTemplate {
    /// Stable identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Trigger keyword.
    pub keyword: String,
    /// Template containing `{query}`.
    pub target: String,
}

impl WebSearchTemplate {
    /// Resolve with a search argument.
    pub fn resolved(&self, argument: &str, home: &str) -> Option<Destination> {
        destination(&self.target, argument, home)
    }

    /// The template installed on first launch.
    pub fn default_search() -> Self {
        Self {
            id: "duckduckgo".into(),
            name: "DuckDuckGo".into(),
            keyword: "web".into(),
            target: "https://duckduckgo.com/?q={query}".into(),
        }
    }
}

/// Validation failures; messages match the macOS app.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QuicklinkError {
    /// Name missing.
    #[error("Enter a name for this quicklink.")]
    NameRequired,
    /// Target is not a safe http(s) URL or absolute/`~/` path.
    #[error(
        "Use an https:// or http:// URL, or an absolute / or ~/ file path. Templates use {{query}}."
    )]
    InvalidTarget,
    /// Keyword contains whitespace.
    #[error("Keywords must be a single word.")]
    KeywordNotSingleWord,
    /// Keyword already used by another link or search.
    #[error("This keyword is already assigned.")]
    KeywordTaken,
    /// Search name missing.
    #[error("Enter a search name.")]
    SearchNameRequired,
    /// Search keyword missing.
    #[error("Enter a search keyword.")]
    SearchKeywordRequired,
    /// Search template invalid.
    #[error("Use an http:// or https:// template containing {{query}}.")]
    InvalidTemplate,
}

fn keyword_error(
    links: &[Quicklink],
    searches: &[WebSearchTemplate],
    keyword: &str,
    excluding: Option<&str>,
) -> Option<QuicklinkError> {
    if keyword.is_empty() {
        return None;
    }
    if keyword.chars().any(char::is_whitespace) {
        return Some(QuicklinkError::KeywordNotSingleWord);
    }
    let taken = links
        .iter()
        .any(|l| Some(l.id.as_str()) != excluding && l.keyword.to_lowercase() == keyword)
        || searches
            .iter()
            .any(|s| Some(s.id.as_str()) != excluding && s.keyword.to_lowercase() == keyword);
    taken.then_some(QuicklinkError::KeywordTaken)
}

/// Create or update a quicklink. Returns its id.
pub fn save_quicklink(
    links: &mut Vec<Quicklink>,
    searches: &[WebSearchTemplate],
    id: Option<&str>,
    name: &str,
    target: &str,
    keyword: &str,
    home: &str,
) -> Result<String, QuicklinkError> {
    let name = name.trim();
    let target = target.trim();
    let keyword = keyword.trim().to_lowercase();
    if name.is_empty() {
        return Err(QuicklinkError::NameRequired);
    }
    if destination(target, "", home).is_none() {
        return Err(QuicklinkError::InvalidTarget);
    }
    if let Some(error) = keyword_error(links, searches, &keyword, id) {
        return Err(error);
    }
    let link = Quicklink {
        id: id.map(str::to_string).unwrap_or_else(new_id),
        name: name.to_string(),
        target: target.to_string(),
        keyword,
    };
    let id = link.id.clone();
    match links.iter_mut().find(|l| l.id == link.id) {
        Some(existing) => *existing = link,
        None => links.push(link),
    }
    Ok(id)
}

/// Create or update a web-search template. Returns its id.
pub fn save_web_search(
    links: &[Quicklink],
    searches: &mut Vec<WebSearchTemplate>,
    id: Option<&str>,
    name: &str,
    keyword: &str,
    target: &str,
    home: &str,
) -> Result<String, QuicklinkError> {
    let keyword = keyword.trim().to_lowercase();
    if name.trim().is_empty() {
        return Err(QuicklinkError::SearchNameRequired);
    }
    if keyword.is_empty() {
        return Err(QuicklinkError::SearchKeywordRequired);
    }
    let valid = target.contains(QUERY_PLACEHOLDER)
        && destination(target, "", home).is_some_and(|d| !d.is_file());
    if !valid {
        return Err(QuicklinkError::InvalidTemplate);
    }
    if let Some(error) = keyword_error(links, searches, &keyword, id) {
        return Err(error);
    }
    let search = WebSearchTemplate {
        id: id.map(str::to_string).unwrap_or_else(new_id),
        name: name.trim().to_string(),
        keyword,
        target: target.trim().to_string(),
    };
    let id = search.id.clone();
    match searches.iter_mut().find(|s| s.id == search.id) {
        Some(existing) => *existing = search,
        None => searches.push(search),
    }
    Ok(id)
}

/// Remove a quicklink by id.
pub fn remove_quicklink(links: &mut Vec<Quicklink>, id: &str) {
    links.retain(|l| l.id != id);
}

/// Remove a web search by id.
pub fn remove_web_search(searches: &mut Vec<WebSearchTemplate>, id: &str) {
    searches.retain(|s| s.id != id);
}

/// Drop links whose target no longer resolves (the macOS store filters these on load).
pub fn retain_resolvable(links: &mut Vec<Quicklink>, home: &str) {
    links.retain(|l| l.resolved("", home).is_some());
}

/// A keyword match: the link (or synthesized web-search link) and the remaining argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeywordMatch {
    /// Matched link; web searches appear as `web:<id>` links.
    pub link: Quicklink,
    /// Text after the keyword.
    pub argument: String,
}

/// Resolve `"keyword some argument"` against links first, then web searches.
pub fn resolve_query(
    links: &[Quicklink],
    searches: &[WebSearchTemplate],
    query: &str,
) -> Option<KeywordMatch> {
    let trimmed = query.trim();
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let keyword = parts.next().filter(|k| !k.is_empty())?;
    let argument = parts.next().unwrap_or("").trim_start().to_string();
    let lowered = keyword.to_lowercase();
    if let Some(link) = links
        .iter()
        .find(|l| !l.keyword.is_empty() && l.keyword.to_lowercase() == lowered)
    {
        return Some(KeywordMatch {
            link: link.clone(),
            argument,
        });
    }
    searches
        .iter()
        .find(|s| s.keyword.to_lowercase() == lowered)
        .map(|s| KeywordMatch {
            link: Quicklink {
                id: format!("web:{}", s.id),
                name: s.name.clone(),
                target: s.target.clone(),
                keyword: s.keyword.clone(),
            },
            argument,
        })
}

/// Fallback web search for text that matched nothing else (uses the first template).
pub fn fallback_url(
    searches: &[WebSearchTemplate],
    query: &str,
    home: &str,
) -> Option<Destination> {
    searches.first()?.resolved(query, home)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/test";

    fn url_of(d: Option<Destination>) -> Url {
        match d {
            Some(Destination::Url(u)) => u,
            other => panic!("expected URL, got {other:?}"),
        }
    }

    #[test]
    fn web_and_local_destinations() {
        let url = url_of(destination(
            " https://example.com/search?q=swift ",
            "",
            HOME,
        ));
        assert_eq!(url.host_str(), Some("example.com"));
        assert_eq!(
            url_of(destination("http://localhost:3000", "", HOME)).scheme(),
            "http"
        );
        assert_eq!(
            destination("/usr/share", "", HOME),
            Some(Destination::File("/usr/share".into()))
        );
        assert_eq!(
            destination("~/Documents", "", HOME),
            Some(Destination::File("/home/test/Documents".into()))
        );
    }

    #[test]
    fn unsafe_and_invalid_targets_are_rejected() {
        for target in [
            "javascript:alert(1)",
            "data:text/plain,Hello",
            "file:///etc/passwd",
            "mailto:test@example.com",
            "https://",
            "example.com",
            "./Documents",
            "",
            "open /Applications",
        ] {
            assert_eq!(destination(target, "", HOME), None, "{target}");
        }
    }

    #[test]
    fn legacy_decode_and_template_encoding() {
        let link: Quicklink =
            serde_json::from_str(r#"{"id":"old","name":"Old","target":"https://example.com"}"#)
                .unwrap();
        assert_eq!(link.keyword, "");
        let url = url_of(destination(
            "https://example.com/?q={query}",
            "Łódź & cats/+?#",
            HOME,
        ));
        let value = url.query_pairs().next().unwrap().1.into_owned();
        assert_eq!(value, "Łódź & cats/+?#");
        assert_eq!(destination("https://{query}.example.com", "x", HOME), None);
        assert_eq!(destination("https://example.com/{unknown}", "", HOME), None);
        assert_eq!(destination("/tmp/{query}", "", HOME), None);
    }

    #[test]
    fn quicklinks_validate_save_edit_remove() {
        let mut links = Vec::new();
        let searches = vec![WebSearchTemplate::default_search()];
        assert!(
            save_quicklink(
                &mut links,
                &searches,
                None,
                "  ",
                "https://example.com",
                "",
                HOME
            )
            .is_err()
        );
        assert!(
            save_quicklink(
                &mut links,
                &searches,
                None,
                "Bad",
                "javascript:alert(1)",
                "",
                HOME
            )
            .is_err()
        );
        assert!(links.is_empty());
        let id = save_quicklink(
            &mut links,
            &searches,
            None,
            " Docs ",
            " https://example.com/docs ",
            "",
            HOME,
        )
        .unwrap();
        assert_eq!(links[0].name, "Docs");
        assert_eq!(links[0].target, "https://example.com/docs");
        save_quicklink(
            &mut links,
            &searches,
            Some(&id),
            "Documents",
            "~/Documents",
            "",
            HOME,
        )
        .unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "~/Documents");
        remove_quicklink(&mut links, &id);
        assert!(links.is_empty());
    }

    #[test]
    fn keyword_conflicts_and_resolution() {
        let mut links = Vec::new();
        let mut searches = vec![WebSearchTemplate::default_search()];
        save_quicklink(
            &mut links,
            &searches,
            None,
            "GitHub",
            "https://github.com/search?q={query}",
            "gh",
            HOME,
        )
        .unwrap();
        assert_eq!(
            save_quicklink(
                &mut links,
                &searches,
                None,
                "Conflict",
                "https://example.com",
                "GH",
                HOME
            ),
            Err(QuicklinkError::KeywordTaken)
        );
        assert_eq!(
            save_web_search(
                &links,
                &mut searches,
                None,
                "Conflict",
                "gh",
                "https://example.com/?q={query}",
                HOME
            ),
            Err(QuicklinkError::KeywordTaken)
        );
        assert_eq!(
            save_web_search(
                &links,
                &mut searches,
                None,
                "Invalid",
                "bad",
                "https://example.com",
                HOME
            ),
            Err(QuicklinkError::InvalidTemplate)
        );
        assert_eq!(
            save_quicklink(
                &mut links,
                &searches,
                None,
                "Two words",
                "https://example.com",
                "a b",
                HOME
            ),
            Err(QuicklinkError::KeywordNotSingleWord)
        );
        let resolved = resolve_query(&links, &searches, "gh hello world").unwrap();
        assert_eq!(resolved.argument, "hello world");
        assert_eq!(resolved.link.keyword, "gh");
        assert!(resolve_query(&links, &searches, "unknown query").is_none());
        let web = resolve_query(&links, &searches, "web rust").unwrap();
        assert_eq!(web.link.id, "web:duckduckgo");
        assert!(fallback_url(&searches, "unknown query", HOME).is_some());
    }
}
