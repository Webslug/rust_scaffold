//! search.json is the reference library for the menu search box: each entry
//! names a place in the app (a page, optionally an element on it) and the
//! words that should find it. It is read on every request, so edits show up
//! the next time the app starts or the library is reloaded, without a
//! rebuild. Bad entries are skipped one by one with a reason.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use tracing::{info, warn};
use crate::files::{self, short_text, FileStatus};

const SEARCH_FILE: &str = "search.json";
/// The default file written when search.json is missing.
const DEFAULT_SEARCH: &str = include_str!("../search.default.json");

pub const MAX_SEARCH_BYTES: u64 = 1024 * 1024;
const MAX_ENTRIES: usize = 500;
const MAX_KEYWORDS: usize = 20;
const MAX_LABEL_LEN: usize = 80;
const MAX_KEYWORD_LEN: usize = 40;
const MAX_ID_LEN: usize = 60;

/// One place the search can jump to. Unknown fields (for example "_note")
/// are ignored so the file can carry comments.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SearchEntry {
    /// Shown in the drop-down.
    pub label: String,
    /// The page: "dashboard" selects the tab #tab-dashboard and the pane
    /// #page-dashboard.
    pub page: String,
    /// Optional id of an element on that page to scroll to and focus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Extra words that should find this entry.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Optional Bootstrap Icons name without the "bi-" prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct SearchReply {
    pub status: FileStatus,
    pub path: String,
    pub entries: Vec<SearchEntry>,
    /// One line per skipped entry, saying which entry and why.
    pub skipped: Vec<String>,
}

/// Lowercase letters, digits and hyphens, at most `max` long.
fn is_slug(text: &str, max: usize) -> bool {
    !text.is_empty()
        && text.len() <= max
        && text.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// An HTML id: starts with a letter, then letters, digits, hyphens or
/// underscores. Keeps the value safe to use in a CSS selector.
fn is_element_id(text: &str) -> bool {
    let mut chars = text.chars();
    text.len() <= MAX_ID_LEN
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn check(entry: &SearchEntry) -> Result<(), String> {
    if !short_text(&entry.label, MAX_LABEL_LEN) {
        return Err("label must be 1 to 80 characters with no control characters".into());
    }
    if !is_slug(&entry.page, MAX_ID_LEN) {
        return Err("page must be lowercase letters, digits and hyphens".into());
    }
    if entry.target.as_deref().is_some_and(|t| !is_element_id(t)) {
        return Err("target must be an element id (letters, digits, hyphens, underscores)".into());
    }
    if entry.keywords.len() > MAX_KEYWORDS {
        return Err(format!("at most {MAX_KEYWORDS} keywords"));
    }
    if !entry.keywords.iter().all(|k| short_text(k, MAX_KEYWORD_LEN)) {
        return Err("each keyword must be 1 to 40 characters".into());
    }
    if entry.icon.as_deref().is_some_and(|i| !is_slug(i, MAX_KEYWORD_LEN)) {
        return Err("icon must be a Bootstrap Icons name such as \"palette\"".into());
    }
    Ok(())
}

/// Validates the parsed file and returns the usable entries and skip reasons.
fn parse(text: &str) -> Result<(Vec<SearchEntry>, Vec<String>), String> {
    let root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let items = root
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("the file must be an object with an \"entries\" list")?;
    if items.len() > MAX_ENTRIES {
        return Err(format!("more than {MAX_ENTRIES} entries"));
    }
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let name: String = item
            .get("label")
            .and_then(Value::as_str)
            .map(|s| s.chars().take(40).collect())
            .unwrap_or_default();
        let result = serde_json::from_value::<SearchEntry>(item.clone())
            .map_err(|e| e.to_string())
            .and_then(|entry| check(&entry).map(|()| entry));
        match result {
            Ok(entry) => entries.push(entry),
            Err(reason) => skipped.push(format!("entry {} {name:?}: {reason}", index + 1)),
        }
    }
    Ok((entries, skipped))
}

/// search.json sits next to config.json in the settings folder.
pub fn search_path() -> PathBuf {
    crate::settings_dir().join(SEARCH_FILE)
}

pub fn load(path: PathBuf) -> SearchReply {
    let mut reply = SearchReply {
        status: FileStatus::Ok,
        path: path.display().to_string(),
        entries: Vec::new(),
        skipped: Vec::new(),
    };
    let (status, text) = files::read_or_create(&path, DEFAULT_SEARCH, MAX_SEARCH_BYTES, "search");
    reply.status = status;
    let Some(text) = text else { return reply };
    match parse(&text) {
        Ok((entries, skipped)) => {
            if skipped.is_empty() {
                info!("search: {} loaded", entries.len());
            } else {
                warn!("search: {} loaded, {} skipped", entries.len(), skipped.len());
            }
            reply.entries = entries;
            reply.skipped = skipped;
        }
        Err(e) => {
            warn!("search: invalid file: {e}");
            reply.status = FileStatus::Invalid;
            reply.skipped = vec![e];
        }
    }
    reply
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rust_scaffold_search_{}_{name}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir.join(SEARCH_FILE)
    }

    #[test]
    fn default_file_is_valid() {
        let (entries, skipped) = parse(DEFAULT_SEARCH).unwrap();
        assert!(skipped.is_empty(), "{skipped:?}");
        assert!(!entries.is_empty());
    }

    #[test]
    fn missing_file_is_created_and_loaded() {
        let path = temp_path("missing");
        let _ = fs::remove_file(&path);
        let reply = load(path.clone());
        assert_eq!(reply.status, FileStatus::Created);
        assert_eq!(fs::read_to_string(path).unwrap(), DEFAULT_SEARCH);
        assert!(!reply.entries.is_empty());
    }

    #[test]
    fn oversized_and_invalid_files_are_not_loaded_or_changed() {
        let path = temp_path("big");
        fs::write(&path, vec![b' '; MAX_SEARCH_BYTES as usize + 1]).unwrap();
        assert_eq!(load(path).status, FileStatus::TooLarge);

        let path = temp_path("bad");
        fs::write(&path, "{nope").unwrap();
        let reply = load(path.clone());
        assert_eq!(reply.status, FileStatus::Invalid);
        assert_eq!(fs::read_to_string(path).unwrap(), "{nope");
    }

    #[test]
    fn bad_entries_are_skipped_good_ones_kept() {
        let (entries, skipped) = parse(
            r#"{"entries":[
                {"label":"Theme","page":"options","target":"opt-theme","keywords":["colour"],"icon":"palette"},
                {"label":"Bad page","page":"../etc"},
                {"label":"Bad target","page":"options","target":"x\"]"},
                {"label":"","page":"options"},
                {"label":"Bad icon","page":"options","icon":"bi bi-x"},
                {"label":"No page"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(skipped.len(), 5, "{skipped:?}");
    }
}
