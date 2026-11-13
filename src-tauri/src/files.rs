//! Shared loader for the JSON files that sit beside the executable and are
//! re-read on request (options.json, search.json). A missing file is created
//! from its built-in default; a file over the size limit or unreadable is
//! never read or overwritten.

use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::debug_log;

#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Ok,
    Created,
    TooLarge,
    Invalid,
    Unreadable,
}

/// Returns the status and, when the file could be used, its text. `name` is
/// only used in debug log lines.
pub fn read_or_create(path: &Path, default: &str, max_bytes: u64, name: &str) -> (FileStatus, Option<String>) {
    match fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => match fs::write(path, default) {
            Ok(()) => {
                debug_log(&format!("{name}: not found, created with defaults"));
                (FileStatus::Created, Some(default.to_string()))
            }
            Err(e) => {
                debug_log(&format!("{name}: could not create file: {e}"));
                (FileStatus::Unreadable, None)
            }
        },
        Err(e) => {
            debug_log(&format!("{name}: cannot stat file: {e}"));
            (FileStatus::Unreadable, None)
        }
        Ok(meta) if meta.len() > max_bytes => {
            debug_log(&format!("{name}: {} bytes exceeds the limit, not loaded", meta.len()));
            (FileStatus::TooLarge, None)
        }
        Ok(_) => match fs::read_to_string(path) {
            Ok(text) => (FileStatus::Ok, Some(text)),
            Err(e) => {
                debug_log(&format!("{name}: read failed: {e}"));
                (FileStatus::Unreadable, None)
            }
        },
    }
}

/// Text that is 1..=max characters long with no control characters.
pub fn short_text(text: &str, max: usize) -> bool {
    !text.trim().is_empty() && text.chars().count() <= max && !text.chars().any(char::is_control)
}
