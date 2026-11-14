//! Shared loader for the JSON files in the settings folder that are re-read on
//! request (options.json, search.json, keys.json). A missing file is created
//! from its built-in default. A file over its size limit is treated as
//! corrupt: it is renamed to `NAME.json.corrupt` (replacing any older one) and
//! a fresh default is written. A file that cannot be read is never touched.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use tracing::{error, info, warn};

#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Ok,
    Created,
    /// Was over the size limit; set aside as `.corrupt` and replaced by the default.
    Recreated,
    /// Was over the size limit and could not be set aside; not loaded, not changed.
    TooLarge,
    Invalid,
    Unreadable,
}

/// Returns the status and, when the file could be used, its text. `name` is
/// only used in log lines.
pub fn read_or_create(path: &Path, default: &str, max_bytes: u64, name: &str) -> (FileStatus, Option<String>) {
    match fs::metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => match write_new(path, default) {
            Ok(()) => {
                info!("{name}: not found, created with defaults");
                (FileStatus::Created, Some(default.to_string()))
            }
            Err(e) => {
                error!("{name}: could not create file: {e}");
                (FileStatus::Unreadable, None)
            }
        },
        Err(e) => {
            error!("{name}: cannot stat file: {e}");
            (FileStatus::Unreadable, None)
        }
        Ok(meta) if meta.len() > max_bytes => {
            let size = meta.len();
            match set_aside(path).and_then(|aside| write_new(path, default).map(|()| aside)) {
                Ok(aside) => {
                    warn!(
                        "{name}: {size} bytes exceeds the {max_bytes} byte limit; moved to {} and recreated with defaults",
                        aside.display()
                    );
                    (FileStatus::Recreated, Some(default.to_string()))
                }
                Err(e) => {
                    error!("{name}: {size} bytes exceeds the {max_bytes} byte limit and could not be replaced: {e}; not loaded");
                    (FileStatus::TooLarge, None)
                }
            }
        }
        Ok(_) => match fs::read_to_string(path) {
            Ok(text) => (FileStatus::Ok, Some(text)),
            Err(e) => {
                error!("{name}: read failed: {e}");
                (FileStatus::Unreadable, None)
            }
        },
    }
}

/// Writes a file, creating its folder first in case the whole settings
/// folder was deleted while the app was running.
pub fn write_new(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, contents)
}

/// Renames `NAME.json` to `NAME.json.corrupt`, replacing an older copy, so
/// only one set-aside file per name is ever kept.
pub fn set_aside(path: &Path) -> io::Result<PathBuf> {
    let mut aside = path.as_os_str().to_owned();
    aside.push(".corrupt");
    let aside = PathBuf::from(aside);
    fs::rename(path, &aside)?;
    Ok(aside)
}

/// Text that is 1..=max characters long with no control characters.
pub fn short_text(text: &str, max: usize) -> bool {
    !text.trim().is_empty() && text.chars().count() <= max && !text.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT: &str = "{\"default\":true}";

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rust_scaffold_files_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn missing_folder_and_file_are_created() {
        let path = temp_dir("missing").join("sub").join("x.json");
        assert_eq!(read_or_create(&path, DEFAULT, 100, "x"), (FileStatus::Created, Some(DEFAULT.into())));
        assert_eq!(fs::read_to_string(&path).unwrap(), DEFAULT);
        assert_eq!(read_or_create(&path, DEFAULT, 100, "x").0, FileStatus::Ok);
    }

    #[test]
    fn oversized_file_is_set_aside_and_recreated() {
        let dir = temp_dir("big");
        fs::create_dir_all(&dir).unwrap();
        let (path, aside) = (dir.join("x.json"), dir.join("x.json.corrupt"));
        fs::write(&path, "a".repeat(101)).unwrap();
        assert_eq!(read_or_create(&path, DEFAULT, 100, "x"), (FileStatus::Recreated, Some(DEFAULT.into())));
        assert_eq!(fs::read_to_string(&path).unwrap(), DEFAULT);
        assert_eq!(fs::read_to_string(&aside).unwrap(), "a".repeat(101));

        fs::write(&path, "b".repeat(101)).unwrap();
        assert_eq!(read_or_create(&path, DEFAULT, 100, "x").0, FileStatus::Recreated);
        assert_eq!(fs::read_to_string(&aside).unwrap(), "b".repeat(101), "older .corrupt copy replaced");

        // Exactly at the limit is still read.
        fs::write(&path, "c".repeat(100)).unwrap();
        assert_eq!(read_or_create(&path, DEFAULT, 100, "x").0, FileStatus::Ok);
    }

    #[test]
    fn oversized_file_that_cannot_be_set_aside_is_left_alone() {
        let dir = temp_dir("stuck");
        // A non-empty folder named like the .corrupt copy makes the rename fail.
        fs::create_dir_all(dir.join("x.json.corrupt").join("block")).unwrap();
        let path = dir.join("x.json");
        fs::write(&path, "a".repeat(101)).unwrap();
        assert_eq!(read_or_create(&path, DEFAULT, 100, "x"), (FileStatus::TooLarge, None));
        assert_eq!(fs::read_to_string(&path).unwrap(), "a".repeat(101));
    }
}
