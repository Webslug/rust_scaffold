//! options.json describes the controls on the Options tab: which setting each
//! control edits, its type, label, choices and limits. It is read on every
//! request, so edits show up the next time the tab opens, without a rebuild.
//! Bad entries are skipped one by one with a reason; the rest still load.

use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::is_extra_key;
use crate::debug_log;
use crate::files::{self, FileStatus};

const OPTIONS_FILE: &str = "options.json";
/// The default file written when options.json is missing.
const DEFAULT_OPTIONS: &str = include_str!("../options.default.json");

pub const MAX_OPTIONS_BYTES: u64 = 1024 * 1024;
const MAX_ENTRIES: usize = 200;
const MAX_CHOICES: usize = 50;
const MAX_LABEL_LEN: usize = 80;
const MAX_HELP_LEN: usize = 300;
const MAX_GROUP_LEN: usize = 60;
const MAX_ROWS: u32 = 20;

pub const CONTROLS: [&str; 10] = [
    "button", "input", "number", "switch", "check", "radio", "range", "list", "select", "text",
];
const SIZES: [&str; 4] = ["sm", "md", "lg", "full"];
const BUTTON_ACTIONS: [&str; 2] = ["revert", "defaults"];
/// The special choices value that means "every installed theme".
const THEME_CHOICES: &str = "themes";

/// One control on the Options tab. Unknown fields (for example "_note") are
/// ignored so the file can carry comments.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct OptionSpec {
    pub control: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub help: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub readonly: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

pub type OptionsStatus = FileStatus;

#[derive(Serialize, Debug)]
pub struct OptionsReply {
    pub status: OptionsStatus,
    pub path: String,
    pub options: Vec<OptionSpec>,
    /// One line per skipped entry, saying which entry and why.
    pub skipped: Vec<String>,
}

/// What kind of value a control holds.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Bool,
    Number,
    Text,
    Choice,
    Nothing,
}

fn kind_of(control: &str) -> Kind {
    match control {
        "switch" | "check" => Kind::Bool,
        "number" | "range" => Kind::Number,
        "input" | "text" => Kind::Text,
        "radio" | "list" | "select" => Kind::Choice,
        _ => Kind::Nothing,
    }
}

/// Built-in config.json fields the Options tab may edit, and the value kinds
/// allowed for each. Internal fields (order, collapsed, tray_items, extras)
/// are deliberately absent.
fn known_field(key: &str) -> Option<&'static [Kind]> {
    match key {
        "theme" => Some(&[Kind::Choice]),
        "width" | "height" => Some(&[Kind::Number]),
        "show_titlebar" | "rounded_corners" | "start_minimised" | "menu_collapsed" | "high_contrast"
        | "large_targets" | "underline_links" | "strong_focus" | "reduce_motion"
        | "hardware_acceleration" => Some(&[Kind::Bool]),
        "menu_layout" | "titlebar_scale" | "text_size" => Some(&[Kind::Choice]),
        _ => None,
    }
}

const RESERVED_KEYS: [&str; 4] = ["order", "collapsed", "tray_items", "extras"];

fn clean_text(text: &str, max: usize) -> bool {
    text.chars().count() <= max && !text.chars().any(char::is_control)
}

/// Accepts "themes", or 1..=50 entries that are strings or
/// {"value": string or number, "label": string}. Returns the allowed values,
/// or None for the theme list (checked by the UI and by Rust's theme rule).
fn choice_values(choices: &Value) -> Result<Option<Vec<Value>>, String> {
    if choices.as_str() == Some(THEME_CHOICES) {
        return Ok(None);
    }
    let items = choices.as_array().ok_or("choices must be a list or \"themes\"")?;
    if items.is_empty() || items.len() > MAX_CHOICES {
        return Err(format!("choices must hold 1 to {MAX_CHOICES} entries"));
    }
    let mut values = Vec::new();
    for item in items {
        let value = match item {
            Value::String(s) if clean_text(s, MAX_LABEL_LEN) && !s.is_empty() => item.clone(),
            Value::Object(map) => {
                let value = map.get("value").ok_or("a choice is missing its value")?;
                let label_ok = map
                    .get("label")
                    .and_then(Value::as_str)
                    .is_some_and(|l| !l.is_empty() && clean_text(l, MAX_LABEL_LEN));
                if !label_ok {
                    return Err("a choice needs a short text label".into());
                }
                match value {
                    Value::String(s) if clean_text(s, MAX_LABEL_LEN) => value.clone(),
                    Value::Number(_) => value.clone(),
                    _ => return Err("a choice value must be short text or a number".into()),
                }
            }
            _ => return Err("each choice must be text or {value, label}".into()),
        };
        if values.contains(&value) {
            return Err("choices contain a duplicate value".into());
        }
        values.push(value);
    }
    Ok(Some(values))
}

/// Checks one entry. Returns a reason when it must be skipped.
fn check(spec: &OptionSpec, seen: &mut HashSet<String>) -> Result<(), String> {
    if !CONTROLS.contains(&spec.control.as_str()) {
        return Err(format!("unknown control {:?}", spec.control));
    }
    if spec.label.trim().is_empty() || !clean_text(&spec.label, MAX_LABEL_LEN) {
        return Err("label must be 1 to 80 characters with no control characters".into());
    }
    if !clean_text(&spec.help, MAX_HELP_LEN) || !clean_text(&spec.group, MAX_GROUP_LEN) {
        return Err("help or group text is too long or has control characters".into());
    }
    if let Some(size) = &spec.size {
        if !SIZES.contains(&size.as_str()) {
            return Err("size must be sm, md, lg or full".into());
        }
    }
    if let Some(placeholder) = &spec.placeholder {
        if !clean_text(placeholder, MAX_LABEL_LEN) {
            return Err("placeholder is too long".into());
        }
    }

    let kind = kind_of(&spec.control);
    if kind == Kind::Nothing {
        if spec.key.is_some() {
            return Err("a button has no key".into());
        }
        let action_ok = spec.action.as_deref().is_some_and(|a| BUTTON_ACTIONS.contains(&a));
        return if action_ok { Ok(()) } else { Err("button action must be revert or defaults".into()) };
    }

    let key = spec.key.as_deref().ok_or("missing key")?;
    if RESERVED_KEYS.contains(&key) {
        return Err(format!("{key:?} is managed by the app and cannot be an option"));
    }
    match known_field(key) {
        Some(kinds) if !kinds.contains(&kind) => {
            return Err(format!("{key:?} cannot use a {} control", spec.control));
        }
        Some(_) => {}
        None if !is_extra_key(key) => {
            return Err("key must be lowercase letters, digits and underscores, at most 40".into());
        }
        None => {}
    }
    if !seen.insert(key.to_string()) {
        return Err(format!("{key:?} appears more than once"));
    }

    if let Some(rows) = spec.rows {
        if spec.control != "text" || rows == 0 || rows > MAX_ROWS {
            return Err(format!("rows is for text controls only, 1 to {MAX_ROWS}"));
        }
    }

    let mut allowed: Option<Vec<Value>> = None;
    if kind == Kind::Choice {
        let choices = spec.choices.as_ref().ok_or("radio, list and select need choices")?;
        allowed = choice_values(choices)?;
        if allowed.is_none() && key != "theme" {
            return Err("\"themes\" choices are only for the theme key".into());
        }
    } else if spec.choices.is_some() {
        return Err("choices are only for radio, list and select".into());
    }

    if kind == Kind::Number {
        for v in [spec.min, spec.max, spec.step].into_iter().flatten() {
            if !v.is_finite() {
                return Err("min, max and step must be finite numbers".into());
            }
        }
        if spec.control == "range" && (spec.min.is_none() || spec.max.is_none()) {
            return Err("a range needs min and max".into());
        }
        if let (Some(min), Some(max)) = (spec.min, spec.max) {
            if min >= max {
                return Err("min must be less than max".into());
            }
        }
        if spec.step.is_some_and(|s| s <= 0.0) {
            return Err("step must be above zero".into());
        }
    } else if spec.min.is_some() || spec.max.is_some() || spec.step.is_some() {
        return Err("min, max and step are only for number and range".into());
    }

    if let Some(default) = &spec.default {
        if known_field(key).is_some() {
            return Err("built-in settings take their default from Rust, not options.json".into());
        }
        let fits = match kind {
            Kind::Bool => default.is_boolean(),
            Kind::Number => default.as_f64().is_some_and(|n| {
                spec.min.is_none_or(|min| n >= min) && spec.max.is_none_or(|max| n <= max)
            }),
            Kind::Text => default.as_str().is_some_and(|s| s.chars().count() <= 4000),
            Kind::Choice => allowed.as_ref().is_some_and(|values| values.contains(default)),
            Kind::Nothing => false,
        };
        if !fits {
            return Err("default does not suit this control".into());
        }
    }
    Ok(())
}

/// Validates the parsed file and returns the usable entries and skip reasons.
fn parse(text: &str) -> Result<(Vec<OptionSpec>, Vec<String>), String> {
    let root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let entries = root
        .get("options")
        .and_then(Value::as_array)
        .ok_or("the file must be an object with an \"options\" list")?;
    if entries.len() > MAX_ENTRIES {
        return Err(format!("more than {MAX_ENTRIES} entries"));
    }
    let mut options = Vec::new();
    let mut skipped = Vec::new();
    let mut seen = HashSet::new();
    for (index, entry) in entries.iter().enumerate() {
        let name = entry
            .get("key")
            .or_else(|| entry.get("label"))
            .and_then(Value::as_str)
            .map(|s| s.chars().take(40).collect::<String>())
            .unwrap_or_default();
        let result = serde_json::from_value::<OptionSpec>(entry.clone())
            .map_err(|e| e.to_string())
            .and_then(|spec| check(&spec, &mut seen).map(|()| spec));
        match result {
            Ok(spec) => options.push(spec),
            Err(reason) => skipped.push(format!("entry {} {name:?}: {reason}", index + 1)),
        }
    }
    Ok((options, skipped))
}

/// options.json sits next to config.json, beside the executable.
pub fn options_path() -> PathBuf {
    crate::app_dir().join(OPTIONS_FILE)
}

pub fn load(path: PathBuf) -> OptionsReply {
    let mut reply = OptionsReply {
        status: OptionsStatus::Ok,
        path: path.display().to_string(),
        options: Vec::new(),
        skipped: Vec::new(),
    };
    let (status, text) = files::read_or_create(&path, DEFAULT_OPTIONS, MAX_OPTIONS_BYTES, "options");
    reply.status = status;
    let Some(text) = text else { return reply };
    match parse(&text) {
        Ok((options, skipped)) => {
            debug_log(&format!("options: {} loaded, {} skipped", options.len(), skipped.len()));
            reply.options = options;
            reply.skipped = skipped;
        }
        Err(e) => {
            debug_log(&format!("options: invalid file: {e}"));
            reply.status = OptionsStatus::Invalid;
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
        let dir = std::env::temp_dir().join(format!("rust_scaffold_opt_{}_{name}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir.join(OPTIONS_FILE)
    }

    fn reasons(json: &str) -> Vec<String> {
        parse(&format!(r#"{{"options":[{json}]}}"#)).unwrap().1
    }

    #[test]
    fn default_file_is_valid() {
        let (options, skipped) = parse(DEFAULT_OPTIONS).unwrap();
        assert!(skipped.is_empty(), "{skipped:?}");
        assert!(!options.is_empty());
    }

    #[test]
    fn missing_file_is_created_and_loaded() {
        let path = temp_path("missing");
        let _ = fs::remove_file(&path);
        let reply = load(path.clone());
        assert_eq!(reply.status, OptionsStatus::Created);
        assert_eq!(fs::read_to_string(path).unwrap(), DEFAULT_OPTIONS);
        assert!(!reply.options.is_empty());
    }

    #[test]
    fn oversized_and_invalid_files_are_not_loaded_or_changed() {
        let path = temp_path("big");
        fs::write(&path, vec![b' '; MAX_OPTIONS_BYTES as usize + 1]).unwrap();
        assert_eq!(load(path).status, OptionsStatus::TooLarge);

        let path = temp_path("bad");
        fs::write(&path, "{nope").unwrap();
        let reply = load(path.clone());
        assert_eq!(reply.status, OptionsStatus::Invalid);
        assert_eq!(fs::read_to_string(path).unwrap(), "{nope");

        let path = temp_path("shape");
        fs::write(&path, "[]").unwrap();
        assert_eq!(load(path).status, OptionsStatus::Invalid);
    }

    #[test]
    fn good_entries_survive_a_bad_neighbour() {
        let (options, skipped) = parse(
            r#"{"options":[
                {"control":"switch","key":"autosave","label":"Autosave","default":true},
                {"control":"teleport","key":"x","label":"X"},
                {"control":"text","key":"notes","label":"Notes","rows":4,"size":"full"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(options.len(), 2);
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].starts_with("entry 2 \"x\""));
    }

    #[test]
    fn rules_are_enforced() {
        let bad = [
            r#"{"control":"switch","key":"order","label":"Order"}"#,
            r#"{"control":"switch","key":"theme","label":"Theme"}"#,
            r#"{"control":"input","key":"Bad Key","label":"X"}"#,
            r#"{"control":"input","key":"a","label":""}"#,
            r#"{"control":"select","key":"a","label":"A"}"#,
            r#"{"control":"select","key":"a","label":"A","choices":"themes"}"#,
            r#"{"control":"select","key":"a","label":"A","choices":[]}"#,
            r#"{"control":"radio","key":"a","label":"A","choices":["x","x"]}"#,
            r#"{"control":"range","key":"a","label":"A","min":0}"#,
            r#"{"control":"range","key":"a","label":"A","min":5,"max":1}"#,
            r#"{"control":"range","key":"a","label":"A","min":0,"max":10,"default":11}"#,
            r#"{"control":"switch","key":"a","label":"A","default":"yes"}"#,
            r#"{"control":"select","key":"a","label":"A","choices":["x"],"default":"y"}"#,
            r#"{"control":"input","key":"a","label":"A","rows":3}"#,
            r#"{"control":"text","key":"a","label":"A","rows":21}"#,
            r#"{"control":"input","key":"a","label":"A","size":"huge"}"#,
            r#"{"control":"button","label":"Go","action":"rm"}"#,
            r#"{"control":"button","key":"a","label":"Go","action":"revert"}"#,
            r#"{"control":"switch","key":"show_titlebar","label":"T","default":false}"#,
        ];
        for entry in bad {
            assert_eq!(reasons(entry).len(), 1, "accepted {entry}");
        }
        let dup = reasons(
            r#"{"control":"switch","key":"a","label":"A"},{"control":"check","key":"a","label":"B"}"#,
        );
        assert_eq!(dup.len(), 1);
    }

    #[test]
    fn every_control_has_a_valid_form() {
        let good = [
            r#"{"control":"select","key":"theme","label":"Theme","choices":"themes"}"#,
            r#"{"control":"number","key":"width","label":"Width","readonly":true}"#,
            r#"{"control":"switch","key":"rounded_corners","label":"Round"}"#,
            r#"{"control":"check","key":"start_minimised","label":"Start in tray"}"#,
            r#"{"control":"input","key":"nickname","label":"Nickname","default":"","size":"md"}"#,
            r#"{"control":"text","key":"notes","label":"Notes","rows":3}"#,
            r#"{"control":"radio","key":"mode","label":"Mode","choices":[{"value":"a","label":"A"},{"value":2,"label":"Two"}],"default":2}"#,
            r#"{"control":"list","key":"view","label":"View","choices":["grid","list"],"default":"grid"}"#,
            r#"{"control":"range","key":"volume","label":"Volume","min":0,"max":100,"step":5,"default":50}"#,
            r#"{"control":"button","label":"Defaults","action":"defaults"}"#,
        ];
        for entry in good {
            assert!(reasons(entry).is_empty(), "rejected {entry}: {:?}", reasons(entry));
        }
    }
}
