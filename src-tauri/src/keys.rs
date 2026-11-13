//! keys.json holds the shortcut keys: each binding joins a key such as "/"
//! or "Alt+3" to an action such as "focus-search" or "page:jobs". Rust
//! checks every binding and writes the key in one standard form, so the
//! page only has to compare strings. Bad or duplicate bindings are skipped
//! one by one with a reason, like search.json.

use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use tracing::{info, warn};
use crate::files::{self, FileStatus};

const KEYS_FILE: &str = "keys.json";
/// The default file written when keys.json is missing.
const DEFAULT_KEYS: &str = include_str!("../keys.default.json");

pub const MAX_KEYS_BYTES: u64 = 64 * 1024;
const MAX_BINDINGS: usize = 100;
const MAX_ACTION_LEN: usize = 60;

/// Keys with a name rather than a character, as the browser reports them.
const NAMED_KEYS: [&str; 26] = [
    "Escape", "Enter", "Space", "Home", "End", "PageUp", "PageDown", "Insert", "Delete", "Backspace",
    "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight",
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
];

/// Ctrl with these letters is kept for editing text.
const EDITING_LETTERS: [char; 6] = ['A', 'C', 'V', 'X', 'Y', 'Z'];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Binding {
    pub key: String,
    pub action: String,
}

#[derive(Serialize, Debug)]
pub struct KeysReply {
    pub status: FileStatus,
    pub path: String,
    /// Usable bindings, keys in standard form ("Ctrl+Alt+Shift+K").
    pub bindings: Vec<Binding>,
    /// One line per skipped binding, saying which and why.
    pub skipped: Vec<String>,
}

/// Splits "Ctrl+Alt+K" into its modifiers and main key. "+" itself can be
/// the main key: "+" or "Ctrl++".
fn split_key(text: &str) -> (Vec<&str>, &str) {
    let (prefix, main) = if text == "+" {
        ("", "+")
    } else if let Some(prefix) = text.strip_suffix("++") {
        (prefix, "+")
    } else {
        match text.rfind('+') {
            Some(i) => (&text[..i], &text[i + 1..]),
            None => ("", text),
        }
    };
    let mods = if prefix.is_empty() { Vec::new() } else { prefix.split('+').collect() };
    (mods, main)
}

/// Returns the key in standard form, or why it cannot be used.
pub fn normalise_key(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("the key is empty".into());
    }
    let (mods, main) = split_key(text);
    let (mut ctrl, mut alt, mut shift) = (false, false, false);
    for m in mods {
        let flag = match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => &mut ctrl,
            "alt" => &mut alt,
            "shift" => &mut shift,
            "" => return Err("a \"+\" is in the wrong place".into()),
            _ => return Err(format!("{m:?} is not Ctrl, Alt or Shift")),
        };
        if *flag {
            return Err(format!("{m} is given twice"));
        }
        *flag = true;
    }

    let mut chars = main.chars();
    let main = match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => c.to_ascii_uppercase().to_string(),
        (Some(c), None) if c.is_control() || c.is_whitespace() => {
            return Err("use \"Space\" for the space bar; other invisible characters cannot be keys".into())
        }
        (Some(c), None) => {
            if shift {
                return Err(format!("write the shifted character itself instead of Shift+{c}"));
            }
            c.to_string()
        }
        _ => match NAMED_KEYS.iter().find(|n| n.eq_ignore_ascii_case(main)) {
            Some(name) => name.to_string(),
            None => return Err(format!("{main:?} is not a key name this app knows")),
        },
    };

    if ctrl && !alt && main.len() == 1 && main.chars().all(|c| EDITING_LETTERS.contains(&c)) {
        return Err(format!("Ctrl+{main} is kept for editing text"));
    }

    let mut out = String::new();
    for (on, name) in [(ctrl, "Ctrl+"), (alt, "Alt+"), (shift, "Shift+")] {
        if on {
            out.push_str(name);
        }
    }
    out.push_str(&main);
    Ok(out)
}

/// Lowercase letters, digits and hyphens.
fn is_slug(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_ACTION_LEN
        && text.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn check_action(action: &str) -> Result<(), String> {
    match action {
        "focus-search" | "leave-field" => Ok(()),
        _ => match action.strip_prefix("page:") {
            Some(page) if is_slug(page) => Ok(()),
            Some(_) => Err("the page name must be lowercase letters, digits and hyphens".into()),
            None => Err(format!("{action:?} is not an action (focus-search, leave-field or page:NAME)")),
        },
    }
}

/// Validates the parsed file and returns the usable bindings and skip reasons.
fn parse(text: &str) -> Result<(Vec<Binding>, Vec<String>), String> {
    let root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let items = root
        .get("bindings")
        .and_then(Value::as_array)
        .ok_or("the file must be an object with a \"bindings\" list")?;
    if items.len() > MAX_BINDINGS {
        return Err(format!("more than {MAX_BINDINGS} bindings"));
    }
    let mut bindings = Vec::new();
    let mut skipped = Vec::new();
    let mut seen = HashSet::new();
    for (index, item) in items.iter().enumerate() {
        let name: String = item
            .get("key")
            .and_then(Value::as_str)
            .map(|s| s.chars().take(20).collect())
            .unwrap_or_default();
        let result = serde_json::from_value::<Binding>(item.clone())
            .map_err(|e| e.to_string())
            .and_then(|b| {
                check_action(&b.action)?;
                let key = normalise_key(&b.key)?;
                if !seen.insert(key.clone()) {
                    return Err(format!("{key} is already used by an earlier binding"));
                }
                Ok(Binding { key, action: b.action })
            });
        match result {
            Ok(binding) => bindings.push(binding),
            Err(reason) => skipped.push(format!("binding {} {name:?}: {reason}", index + 1)),
        }
    }
    Ok((bindings, skipped))
}

/// keys.json sits next to config.json in the settings folder.
pub fn keys_path() -> PathBuf {
    crate::settings_dir().join(KEYS_FILE)
}

pub fn load(path: PathBuf) -> KeysReply {
    let mut reply = KeysReply {
        status: FileStatus::Ok,
        path: path.display().to_string(),
        bindings: Vec::new(),
        skipped: Vec::new(),
    };
    let (status, text) = files::read_or_create(&path, DEFAULT_KEYS, MAX_KEYS_BYTES, "keys");
    reply.status = status;
    let Some(text) = text else { return reply };
    match parse(&text) {
        Ok((bindings, skipped)) => {
            if skipped.is_empty() {
                info!("keys: {} loaded", bindings.len());
            } else {
                warn!("keys: {} loaded, {} skipped: {}", bindings.len(), skipped.len(), skipped.join("; "));
            }
            reply.bindings = bindings;
            reply.skipped = skipped;
        }
        Err(e) => {
            warn!("keys: invalid file: {e}");
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
        let dir = std::env::temp_dir().join(format!("rust_scaffold_keys_{}_{name}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir.join(KEYS_FILE)
    }

    #[test]
    fn default_file_is_valid() {
        let (bindings, skipped) = parse(DEFAULT_KEYS).unwrap();
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(bindings.len(), 12);
        assert!(bindings.iter().any(|b| b.key == "/" && b.action == "focus-search"));
        assert!(!bindings.iter().any(|b| b.key == "F1"), "Kim asked for no F1 by default");
    }

    #[test]
    fn keys_are_written_in_one_standard_form() {
        assert_eq!(normalise_key("/").unwrap(), "/");
        assert_eq!(normalise_key("?").unwrap(), "?");
        assert_eq!(normalise_key("ctrl+k").unwrap(), "Ctrl+K");
        assert_eq!(normalise_key("shift+alt+control+p").unwrap(), "Ctrl+Alt+Shift+P");
        assert_eq!(normalise_key("Alt+1").unwrap(), "Alt+1");
        assert_eq!(normalise_key("escape").unwrap(), "Escape");
        assert_eq!(normalise_key("Shift+f5").unwrap(), "Shift+F5");
        assert_eq!(normalise_key("+").unwrap(), "+");
        assert_eq!(normalise_key("Ctrl++").unwrap(), "Ctrl++");
        assert_eq!(normalise_key(" Alt+Space ").unwrap(), "Alt+Space");
    }

    #[test]
    fn bad_keys_are_refused() {
        for bad in ["", "Ctrl+", "Hyper+K", "Ctrl+Ctrl+K", "Shift+/", "Ctrl+C", "Ctrl+Shift+Z", "Tab", "F13", "ab", " ", "Ctrl++K"] {
            assert!(normalise_key(bad).is_err(), "{bad:?} should be refused");
        }
        // Ctrl+Alt+C is not an editing key.
        assert_eq!(normalise_key("Ctrl+Alt+C").unwrap(), "Ctrl+Alt+C");
    }

    #[test]
    fn bad_and_duplicate_bindings_are_skipped_good_ones_kept() {
        let (bindings, skipped) = parse(
            r#"{"bindings":[
                {"key":"/","action":"focus-search"},
                {"key":"Alt+3","action":"page:tables"},
                {"key":"alt+3","action":"page:cards"},
                {"key":"Alt+4","action":"page:../etc"},
                {"key":"Alt+5","action":"run-anything"},
                {"key":"Ctrl+V","action":"focus-search"},
                {"action":"leave-field"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(bindings.len(), 2);
        assert_eq!(skipped.len(), 5, "{skipped:?}");
        assert!(skipped[0].contains("already used"), "{skipped:?}");
    }

    #[test]
    fn missing_file_is_created_oversized_and_invalid_ones_left_alone() {
        let path = temp_path("missing");
        let _ = fs::remove_file(&path);
        let reply = load(path.clone());
        assert_eq!(reply.status, FileStatus::Created);
        assert_eq!(fs::read_to_string(&path).unwrap(), DEFAULT_KEYS);
        assert_eq!(reply.bindings.len(), 12);

        let path = temp_path("big");
        fs::write(&path, vec![b' '; MAX_KEYS_BYTES as usize + 1]).unwrap();
        assert_eq!(load(path).status, FileStatus::TooLarge);

        let path = temp_path("bad");
        fs::write(&path, "{nope").unwrap();
        let reply = load(path.clone());
        assert_eq!(reply.status, FileStatus::Invalid);
        assert_eq!(fs::read_to_string(path).unwrap(), "{nope");
    }
}
