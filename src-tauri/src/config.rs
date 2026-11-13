use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::debug_log;

/// Files larger than this are never read.
pub const MAX_CONFIG_BYTES: u64 = 5 * 1024 * 1024;
const CONFIG_FILE: &str = "config.json";

const MIN_SIZE: u32 = 200;
const MAX_SIZE: u32 = 10_000;
const MAX_THEME_LEN: usize = 40;
const MAX_ID_LEN: usize = 40;
const MAX_LISTS: usize = 20;
const MAX_ITEMS_PER_LIST: usize = 50;
const MAX_COLLAPSED: usize = 100;
const MAX_TRAY_ITEMS: usize = 10;
const MAX_TRAY_LABEL_LEN: usize = 40;
const MAX_EXTRAS: usize = 100;
const MAX_EXTRA_KEY_LEN: usize = 40;
const MAX_EXTRA_TEXT_LEN: usize = 4000;
/// What a custom tray menu item may do. A fixed set on purpose: config.json
/// must never be able to run arbitrary commands.
pub const TRAY_ACTIONS: [&str; 4] = ["show", "hide", "toggle", "quit"];

/// One custom entry in the tray context menu.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TrayItem {
    pub label: String,
    pub action: String,
}

/// User preferences. `#[serde(default)]` means a missing key falls back to
/// its default instead of failing the whole load.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    pub theme: String,
    pub width: u32,
    pub height: u32,
    /// Show the title bar (name, theme picker and window buttons).
    pub show_titlebar: bool,
    /// Round the window corners. Set false for a square window.
    pub rounded_corners: bool,
    /// Saved item order per sortable list: list id -> item ids.
    pub order: BTreeMap<String, Vec<String>>,
    /// Ids of accordion panels the user has collapsed (all start expanded).
    pub collapsed: Vec<String>,
    /// Start with the window hidden, living in the tray. Ignored if the tray
    /// could not be created, so the app can never become unreachable.
    pub start_minimised: bool,
    /// Custom tray menu entries, shown between Show/Hide and Quit.
    pub tray_items: Vec<TrayItem>,
    /// Settings defined only in options.json, unknown to this struct. Kept
    /// here so saving never drops them. Values are booleans, numbers or text.
    pub extras: BTreeMap<String, Value>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            theme: "aero-blue".into(),
            width: 640,
            height: 480,
            show_titlebar: true,
            rounded_corners: true,
            order: BTreeMap::new(),
            collapsed: Vec::new(),
            start_minimised: false,
            tray_items: Vec::new(),
            extras: BTreeMap::new(),
        }
    }
}

/// Ids are lowercase slugs: letters, digits, hyphens, at most `max` long.
fn is_slug(id: &str, max: usize) -> bool {
    !id.is_empty()
        && id.len() <= max
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Extra keys are lowercase letters, digits and underscores, at most 40 long.
pub fn is_extra_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_EXTRA_KEY_LEN
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Extra values are booleans, numbers or text. Text may hold line breaks and
/// tabs but no other control characters.
fn is_extra_value(value: &Value) -> bool {
    match value {
        Value::Bool(_) | Value::Number(_) => true,
        Value::String(text) => {
            text.chars().count() <= MAX_EXTRA_TEXT_LEN
                && !text.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
        }
        _ => false,
    }
}

impl Config {
    /// Rejects values the UI should never send. Sizes are clamped; the theme
    /// must be a short slug (letters, digits, hyphens).
    pub fn validate(mut self) -> Result<Config, String> {
        if !is_slug(&self.theme, MAX_THEME_LEN) {
            return Err("invalid theme name".into());
        }
        if self.order.len() > MAX_LISTS {
            return Err("too many sortable lists".into());
        }
        for (list, items) in &self.order {
            if !is_slug(list, MAX_ID_LEN) || items.len() > MAX_ITEMS_PER_LIST {
                return Err("invalid sortable list".into());
            }
            if !items.iter().all(|id| is_slug(id, MAX_ID_LEN)) {
                return Err("invalid sortable item id".into());
            }
        }
        if self.tray_items.len() > MAX_TRAY_ITEMS {
            return Err("too many tray items".into());
        }
        for item in &self.tray_items {
            let label_ok = !item.label.trim().is_empty()
                && item.label.chars().count() <= MAX_TRAY_LABEL_LEN
                && !item.label.chars().any(char::is_control);
            if !label_ok || !TRAY_ACTIONS.contains(&item.action.as_str()) {
                return Err("invalid tray item".into());
            }
        }
        if self.collapsed.len() > MAX_COLLAPSED || !self.collapsed.iter().all(|id| is_slug(id, MAX_ID_LEN)) {
            return Err("invalid collapsed panel list".into());
        }
        if self.extras.len() > MAX_EXTRAS {
            return Err("too many extra settings".into());
        }
        for (key, value) in &self.extras {
            if !is_extra_key(key) || !is_extra_value(value) {
                return Err(format!("invalid extra setting {key:?}"));
            }
        }
        self.width = self.width.clamp(MIN_SIZE, MAX_SIZE);
        self.height = self.height.clamp(MIN_SIZE, MAX_SIZE);
        Ok(self)
    }
}

/// How the last load went. Anything other than Ok or Created blocks saving,
/// so a file we could not read is never overwritten.
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LoadStatus {
    Ok,
    Created,
    TooLarge,
    Invalid,
    Unreadable,
}

impl LoadStatus {
    pub fn saving_allowed(self) -> bool {
        matches!(self, LoadStatus::Ok | LoadStatus::Created)
    }
}

pub struct ConfigState {
    pub path: PathBuf,
    pub config: Config,
    pub status: LoadStatus,
}

/// config.json lives in the working directory, which run.sh and run.bat set
/// to the project folder.
pub fn config_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(CONFIG_FILE)
}

pub fn load(path: PathBuf) -> ConfigState {
    let (config, status) = match fs::metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let defaults = Config::default();
            match write_atomic(&path, &defaults) {
                Ok(()) => {
                    debug_log("config: not found, created with defaults");
                    (defaults, LoadStatus::Created)
                }
                Err(e) => {
                    debug_log(&format!("config: could not create file: {e}"));
                    (defaults, LoadStatus::Unreadable)
                }
            }
        }
        Err(e) => {
            debug_log(&format!("config: cannot stat file: {e}"));
            (Config::default(), LoadStatus::Unreadable)
        }
        Ok(meta) if meta.len() > MAX_CONFIG_BYTES => {
            debug_log(&format!(
                "config: {} bytes exceeds the {} byte limit, not loaded",
                meta.len(),
                MAX_CONFIG_BYTES
            ));
            (Config::default(), LoadStatus::TooLarge)
        }
        Ok(_) => match fs::read_to_string(&path) {
            Err(e) => {
                debug_log(&format!("config: read failed: {e}"));
                (Config::default(), LoadStatus::Unreadable)
            }
            Ok(text) => match serde_json::from_str::<Config>(&text)
                .map_err(|e| e.to_string())
                .and_then(Config::validate)
            {
                Ok(config) => {
                    debug_log("config: loaded");
                    (config, LoadStatus::Ok)
                }
                Err(e) => {
                    debug_log(&format!("config: invalid, using defaults: {e}"));
                    (Config::default(), LoadStatus::Invalid)
                }
            },
        },
    };
    ConfigState { path, config, status }
}

pub fn save(state: &mut ConfigState, config: Config) -> Result<(), String> {
    if !state.status.saving_allowed() {
        debug_log("config: save refused, file was not loaded cleanly");
        return Err("saving is disabled because config.json could not be loaded".into());
    }
    let config = config.validate()?;
    write_atomic(&state.path, &config).map_err(|e| {
        debug_log(&format!("config: save failed: {e}"));
        format!("could not write config.json: {e}")
    })?;
    debug_log("config: saved");
    state.config = config;
    Ok(())
}

/// Writes to a temporary file then renames, so a crash mid-write cannot
/// leave a truncated config.json.
fn write_atomic(path: &Path, config: &Config) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rust_scaffold_test_{}_{name}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir.join(CONFIG_FILE)
    }

    #[test]
    fn partial_file_fills_defaults() {
        let path = temp_path("partial");
        fs::write(&path, r#"{"theme":"slate"}"#).unwrap();
        let state = load(path);
        assert_eq!(state.status, LoadStatus::Ok);
        assert_eq!(state.config.theme, "slate");
        assert_eq!(state.config.width, 640);
    }

    #[test]
    fn invalid_file_blocks_saving_and_is_untouched() {
        let path = temp_path("invalid");
        fs::write(&path, "{nope").unwrap();
        let mut state = load(path.clone());
        assert_eq!(state.status, LoadStatus::Invalid);
        assert!(save(&mut state, Config::default()).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{nope");
    }

    #[test]
    fn oversized_file_is_not_loaded() {
        let path = temp_path("big");
        fs::write(&path, vec![b' '; MAX_CONFIG_BYTES as usize + 1]).unwrap();
        let state = load(path);
        assert_eq!(state.status, LoadStatus::TooLarge);
    }

    #[test]
    fn file_at_the_limit_is_still_read() {
        let path = temp_path("limit");
        let mut text = String::from(r#"{"theme":"rose"}"#);
        text.push_str(&" ".repeat(MAX_CONFIG_BYTES as usize - text.len()));
        fs::write(&path, text).unwrap();
        let state = load(path);
        assert_eq!(state.status, LoadStatus::Ok);
        assert_eq!(state.config.theme, "rose");
    }

    #[test]
    fn save_round_trips_and_clamps_sizes() {
        let path = temp_path("save");
        let mut state = load(path.clone());
        let cfg = Config { theme: "vapor".into(), width: 5, height: 99_999, ..Config::default() };
        save(&mut state, cfg).unwrap();
        let reloaded = load(path);
        assert_eq!(
            reloaded.config,
            Config { theme: "vapor".into(), width: 200, height: 10_000, ..Config::default() }
        );
    }

    #[test]
    fn bad_theme_names_are_rejected() {
        for bad in ["", "../etc", "Has Space", &"a".repeat(41)] {
            let cfg = Config { theme: bad.into(), ..Config::default() };
            assert!(cfg.validate().is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn old_config_without_new_keys_still_loads() {
        let path = temp_path("old");
        fs::write(&path, r#"{"theme":"rose","width":700,"height":500}"#).unwrap();
        let state = load(path);
        assert_eq!(state.status, LoadStatus::Ok);
        assert!(state.config.show_titlebar);
        assert!(state.config.rounded_corners);
        assert!(state.config.order.is_empty());
        assert!(state.config.collapsed.is_empty());
    }

    #[test]
    fn order_and_collapsed_round_trip() {
        let path = temp_path("order");
        let mut state = load(path.clone());
        let mut cfg = Config::default();
        cfg.show_titlebar = false;
        cfg.rounded_corners = false;
        cfg.order.insert("dashboard-cards".into(), vec!["b".into(), "a".into()]);
        cfg.collapsed = vec!["panel-one".into()];
        save(&mut state, cfg.clone()).unwrap();
        assert_eq!(load(path).config, cfg);
    }

    #[test]
    fn oversized_or_malformed_lists_are_rejected() {
        let mut too_many = Config::default();
        too_many.order.insert("l".into(), (0..51).map(|i| format!("i{i}")).collect());
        assert!(too_many.validate().is_err());

        let mut bad_id = Config::default();
        bad_id.order.insert("l".into(), vec!["Bad Id".into()]);
        assert!(bad_id.validate().is_err());

        let mut bad_collapsed = Config::default();
        bad_collapsed.collapsed = vec!["../x".into()];
        assert!(bad_collapsed.validate().is_err());

        let mut many_lists = Config::default();
        for i in 0..21 {
            many_lists.order.insert(format!("l{i}"), vec![]);
        }
        assert!(many_lists.validate().is_err());
    }

    #[test]
    fn tray_options_round_trip_and_default() {
        let old = Config::default();
        assert!(!old.start_minimised);
        assert!(old.tray_items.is_empty());

        let path = temp_path("tray");
        let mut state = load(path.clone());
        let mut cfg = Config::default();
        cfg.start_minimised = true;
        cfg.tray_items = vec![TrayItem { label: "Hide window".into(), action: "hide".into() }];
        save(&mut state, cfg.clone()).unwrap();
        assert_eq!(load(path).config, cfg);
    }

    #[test]
    fn bad_tray_items_are_rejected() {
        let make = |label: &str, action: &str| {
            let mut c = Config::default();
            c.tray_items = vec![TrayItem { label: label.into(), action: action.into() }];
            c
        };
        assert!(make("Run it", "rm -rf /").validate().is_err());
        assert!(make("Run it", "SHOW").validate().is_err());
        assert!(make("", "show").validate().is_err());
        assert!(make("   ", "show").validate().is_err());
        assert!(make("bad\nlabel", "show").validate().is_err());
        assert!(make(&"x".repeat(41), "show").validate().is_err());
        assert!(make("Fine", "toggle").validate().is_ok());

        let mut many = Config::default();
        many.tray_items = (0..11)
            .map(|i| TrayItem { label: format!("Item {i}"), action: "show".into() })
            .collect();
        assert!(many.validate().is_err());
    }

    #[test]
    fn extras_round_trip_and_old_files_load() {
        let path = temp_path("extras");
        fs::write(&path, r#"{"theme":"rose"}"#).unwrap();
        let mut state = load(path.clone());
        assert!(state.config.extras.is_empty());
        let mut cfg = state.config.clone();
        cfg.extras.insert("nickname".into(), Value::from("Kim\nsecond line"));
        cfg.extras.insert("volume".into(), Value::from(0.75));
        cfg.extras.insert("autosave".into(), Value::from(true));
        save(&mut state, cfg.clone()).unwrap();
        assert_eq!(load(path).config, cfg);
    }

    #[test]
    fn bad_extras_are_rejected() {
        let make = |key: &str, value: Value| {
            let mut c = Config::default();
            c.extras.insert(key.into(), value);
            c
        };
        assert!(make("Nick", Value::from("x")).validate().is_err());
        assert!(make("nick-name", Value::from("x")).validate().is_err());
        assert!(make(&"a".repeat(41), Value::from("x")).validate().is_err());
        assert!(make("nick", Value::Null).validate().is_err());
        assert!(make("nick", serde_json::json!(["a"])).validate().is_err());
        assert!(make("nick", serde_json::json!({"a": 1})).validate().is_err());
        assert!(make("nick", Value::from("bell\u{7}")).validate().is_err());
        assert!(make("nick", Value::from("x".repeat(4001))).validate().is_err());
        assert!(make("nick", Value::from("x".repeat(4000))).validate().is_ok());

        let mut many = Config::default();
        for i in 0..101 {
            many.extras.insert(format!("k{i}"), Value::from(i));
        }
        assert!(many.validate().is_err());
    }
}
