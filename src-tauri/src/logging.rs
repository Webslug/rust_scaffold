//! Program events, recorded with the `tracing` crate.
//!
//! Code reports events with `tracing::error!`, `warn!`, `info!` or `debug!`.
//! One layer receives every event:
//! - Warnings and errors go into a list shown on the Errors tab. The newest
//!   `error_log_max` entries (config.json) are kept. Each new entry is pushed
//!   to the window at once as a `log:entry` event, so the tab needs no
//!   polling. The list is also saved to `errors.jsonl` in the settings
//!   folder (one JSON line per entry, whatever `DEBUG_MODE` says), so the
//!   Errors tab still shows them after a restart, marked as earlier. The
//!   file is rewritten with the kept entries when it grows to twice the
//!   limit, so it never holds more than about 2 x `error_log_max` lines.
//! - When `DEBUG_MODE` is true, every event from debug level up is also
//!   appended to `debug.log` in the settings folder. When the file passes
//!   `LOG_FILE_MAX` bytes, its older half is removed. With `DEBUG_MODE` false
//!   no debug.log is written.
//!
//! Tauri's own libraries report through the older `log` crate; the
//! `tracing-log` feature forwards those messages into the same stream.

use std::cell::Cell;
use std::collections::VecDeque;
use std::fmt::{self, Write as _};
use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use time::macros::format_description;
use time::OffsetDateTime;
use tracing::field::{Field, Visit};
use tracing::level_filters::LevelFilter;
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;

use crate::DEBUG_MODE;

/// Entries kept on the Errors tab unless config.json says otherwise.
pub const DEFAULT_MAX_ENTRIES: u32 = 100;
/// Limits for `error_log_max` in config.json.
pub const MIN_ENTRIES: u32 = 10;
pub const MAX_ENTRIES: u32 = 1000;

const LOG_FILE: &str = "debug.log";
/// Warnings and errors kept across restarts, one JSON object per line.
const SAVED_FILE: &str = "errors.jsonl";
/// A larger errors.jsonl is not read (something else wrote it) and is
/// replaced.
const SAVED_FILE_MAX: u64 = 4 * 1024 * 1024;
/// debug.log is cut to its newest half when it grows past this size.
pub const LOG_FILE_MAX: u64 = 1024 * 1024;
/// Longer messages are shortened, so one runaway message cannot fill memory.
const MAX_MESSAGE_CHARS: usize = 2000;
/// Name of the window event that carries each new entry.
const ENTRY_EVENT: &str = "log:entry";

#[derive(Serialize, Clone, Debug)]
pub struct LogEntry {
    pub id: u64,
    /// Milliseconds since 1970, for sorting in the UI.
    pub unix_ms: i64,
    /// Local time, "2026-11-13 17:20:05".
    pub time: String,
    /// "error" or "warn".
    pub level: &'static str,
    /// Where the event came from: a Rust module, a library, or "ui".
    pub source: String,
    pub message: String,
    /// Recorded before this start of the app (read from errors.jsonl).
    pub earlier: bool,
}

/// One line of errors.jsonl.
#[derive(Serialize, serde::Deserialize)]
struct SavedLine {
    id: u64,
    unix_ms: i64,
    time: String,
    level: String,
    source: String,
    message: String,
}

impl SavedLine {
    fn of(e: &LogEntry) -> Self {
        SavedLine {
            id: e.id,
            unix_ms: e.unix_ms,
            time: e.time.clone(),
            level: e.level.to_string(),
            source: e.source.clone(),
            message: e.message.clone(),
        }
    }

    fn into_entry(self) -> LogEntry {
        LogEntry {
            id: self.id,
            unix_ms: self.unix_ms,
            time: self.time,
            level: if self.level == "warn" { "warn" } else { "error" },
            source: self.source,
            message: shorten(self.message),
            earlier: true,
        }
    }
}

/// errors.jsonl once `restore` has run, and how many lines it holds.
struct SavedFile {
    path: PathBuf,
    lines: usize,
}

static SAVED: Mutex<Option<SavedFile>> = Mutex::new(None);

struct ErrorList {
    entries: VecDeque<LogEntry>,
    max: usize,
}

impl ErrorList {
    fn push(&mut self, entry: LogEntry) {
        self.entries.push_back(entry);
        self.prune();
    }

    /// Drops the oldest entries beyond `max`.
    fn prune(&mut self) {
        while self.entries.len() > self.max {
            self.entries.pop_front();
        }
    }
}

static LIST: Mutex<ErrorList> = Mutex::new(ErrorList {
    entries: VecDeque::new(),
    max: DEFAULT_MAX_ENTRIES as usize,
});
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static APP: OnceLock<AppHandle> = OnceLock::new();
/// Serialises writes to debug.log, including the trimming step.
static FILE_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    /// Set while an event is being handled on this thread. An event raised
    /// while handling another (for example by Tauri while emitting) is
    /// dropped instead of looping.
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

/// Installs the layer. Call once, first thing in `run()`.
pub fn init() {
    // Ids start from the time in microseconds, so entries from a later
    // start of the app always have higher ids than saved ones.
    let micros = (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1000).max(1) as u64;
    NEXT_ID.fetch_max(micros, Ordering::Relaxed);
    let _ = tracing_subscriber::registry().with(AppLayer).try_init();
}

/// Loads the warnings and errors saved by earlier runs from `dir` and
/// starts saving new ones there. Call once the settings folder and
/// `error_log_max` are known. Entries recorded before this are kept too.
pub fn restore(dir: &Path) {
    let path = dir.join(SAVED_FILE);
    let mut list = LIST.lock().unwrap_or_else(|e| e.into_inner());
    let earlier = read_saved(&path, list.max);
    if let Some(newest) = earlier.iter().map(|e| e.id).max() {
        NEXT_ID.fetch_max(newest + 1, Ordering::Relaxed);
    }
    let now: Vec<LogEntry> = list.entries.drain(..).collect();
    list.entries.extend(earlier);
    list.entries.extend(now);
    list.prune();
    let _ = write_saved(&path, list.entries.iter());
    *SAVED.lock().unwrap_or_else(|e| e.into_inner()) = Some(SavedFile { path, lines: list.entries.len() });
}

/// The newest `max` entries of errors.jsonl, oldest first. A missing,
/// oversized or unreadable file gives none; bad lines are skipped.
fn read_saved(path: &Path, max: usize) -> Vec<LogEntry> {
    if fs::metadata(path).map_or(true, |m| m.len() > SAVED_FILE_MAX) {
        return Vec::new();
    }
    let Ok(text) = fs::read_to_string(path) else { return Vec::new() };
    let mut entries: Vec<LogEntry> = text
        .lines()
        .filter_map(|line| serde_json::from_str::<SavedLine>(line).ok())
        .map(SavedLine::into_entry)
        .collect();
    let extra = entries.len().saturating_sub(max);
    entries.drain(..extra);
    entries
}

/// Replaces errors.jsonl with `entries` (through a temporary file, so a
/// crash cannot leave half a file).
fn write_saved<'a>(path: &Path, entries: impl Iterator<Item = &'a LogEntry>) -> io::Result<()> {
    let mut text = String::new();
    for e in entries {
        text.push_str(&serde_json::to_string(&SavedLine::of(e)).unwrap_or_default());
        text.push('\n');
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("jsonl.tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

/// Adds one entry to errors.jsonl, rewriting the file with the kept list
/// when it has grown to twice the limit.
fn save_entry(list: &ErrorList, entry: &LogEntry) {
    let mut saved = SAVED.lock().unwrap_or_else(|e| e.into_inner());
    let Some(file) = saved.as_mut() else { return };
    if file.lines + 1 > list.max * 2 {
        if write_saved(&file.path, list.entries.iter()).is_ok() {
            file.lines = list.entries.len();
        }
        return;
    }
    let line = serde_json::to_string(&SavedLine::of(entry)).unwrap_or_default();
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&file.path) {
        if writeln!(f, "{line}").is_ok() {
            file.lines += 1;
        }
    }
}

/// Lets new entries be pushed to the window. Entries recorded before this
/// are still in the list; the Errors tab loads them when it starts.
pub fn set_app(app: AppHandle) {
    let _ = APP.set(app);
}

/// Changes how many entries are kept and prunes the list at once.
pub fn set_max(max: u32) {
    let mut list = LIST.lock().unwrap_or_else(|e| e.into_inner());
    list.max = max.clamp(MIN_ENTRIES, MAX_ENTRIES) as usize;
    list.prune();
}

/// The kept entries, oldest first, and the current limit.
pub fn entries() -> (Vec<LogEntry>, usize) {
    let list = LIST.lock().unwrap_or_else(|e| e.into_inner());
    (list.entries.iter().cloned().collect(), list.max)
}

/// Empties the list and errors.jsonl.
pub fn clear() {
    let mut list = LIST.lock().unwrap_or_else(|e| e.into_inner());
    list.entries.clear();
    if let Some(file) = SAVED.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        if write_saved(&file.path, list.entries.iter()).is_ok() {
            file.lines = 0;
        }
    }
}

/// Where debug.log is written, or None when DEBUG_MODE is false.
pub fn log_file() -> Option<PathBuf> {
    DEBUG_MODE.then(|| crate::settings_dir().join(LOG_FILE))
}

fn max_level() -> Level {
    if DEBUG_MODE {
        Level::DEBUG
    } else {
        Level::WARN
    }
}

struct AppLayer;

impl<S: Subscriber> Layer<S> for AppLayer {
    fn enabled(&self, metadata: &Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        // Lower levels are more severe: ERROR < WARN < INFO < DEBUG.
        *metadata.level() <= max_level()
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(LevelFilter::from_level(max_level()))
    }

    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        if BUSY.with(|busy| busy.replace(true)) {
            return;
        }
        handle(event);
        BUSY.with(|busy| busy.set(false));
    }
}

/// Collects an event's message and extra fields. Events forwarded from the
/// `log` crate carry their real origin in a `log.target` field.
#[derive(Default)]
struct Fields {
    message: String,
    extra: String,
    log_target: Option<String>,
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => self.message.push_str(value),
            "log.target" => self.log_target = Some(value.to_string()),
            name if name.starts_with("log.") => {}
            name => {
                let _ = write!(self.extra, " {name}={value}");
            }
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        match field.name() {
            "message" => {
                let _ = write!(self.message, "{value:?}");
            }
            name if name.starts_with("log.") => {}
            name => {
                let _ = write!(self.extra, " {name}={value:?}");
            }
        }
    }
}

fn handle(event: &Event<'_>) {
    let meta = event.metadata();
    let mut fields = Fields::default();
    event.record(&mut fields);
    let source = fields.log_target.unwrap_or_else(|| meta.target().to_string());
    let message = shorten(format!("{}{}", fields.message, fields.extra));
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let time = now
        .format(format_description!("[year]-[month]-[day] [hour]:[minute]:[second]"))
        .unwrap_or_default();
    let level = *meta.level();

    if DEBUG_MODE {
        append_line(&crate::settings_dir().join(LOG_FILE), &format!("{time} {level:<5} {source}: {message}"));
    }

    let name = match level {
        Level::ERROR => "error",
        Level::WARN => "warn",
        _ => return,
    };
    let entry = LogEntry {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        unix_ms: (now.unix_timestamp_nanos() / 1_000_000) as i64,
        time,
        level: name,
        source,
        message,
        earlier: false,
    };
    {
        let mut list = LIST.lock().unwrap_or_else(|e| e.into_inner());
        list.push(entry.clone());
        save_entry(&list, &entry);
    }
    if let Some(app) = APP.get() {
        let _ = app.emit(ENTRY_EVENT, &entry);
    }
}

fn shorten(mut text: String) -> String {
    if let Some((cut, _)) = text.char_indices().nth(MAX_MESSAGE_CHARS) {
        text.truncate(cut);
        text.push_str(" [shortened]");
    }
    text
}

/// Appends one line, first trimming the file if it has passed the limit.
/// Failures are ignored so logging can never stop the app.
fn append_line(path: &Path, line: &str) {
    let _guard = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if fs::metadata(path).is_ok_and(|m| m.len() > LOG_FILE_MAX) {
        let _ = trim_file(path, LOG_FILE_MAX / 2);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}

/// Keeps roughly the newest `keep` bytes of the file, starting at a line
/// boundary, behind a note saying older lines were removed. Written to a
/// temporary file and renamed, so a crash cannot leave half a file.
fn trim_file(path: &Path, keep: u64) -> io::Result<()> {
    let data = fs::read(path)?;
    let start = data.len().saturating_sub(keep as usize);
    let cut = data[start..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(data.len(), |p| start + p + 1);
    let mut out = b"[older lines removed to keep debug.log under its size limit]\n".to_vec();
    out.extend_from_slice(&data[cut..]);
    let tmp = path.with_extension("log.tmp");
    fs::write(&tmp, out)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64) -> LogEntry {
        LogEntry {
            id,
            unix_ms: 0,
            time: String::new(),
            level: "error",
            source: "test".into(),
            message: format!("entry {id}"),
            earlier: false,
        }
    }

    #[test]
    fn saved_entries_come_back_marked_earlier() {
        let path = std::env::temp_dir().join(format!("rust-scaffold-errors-{}.jsonl", std::process::id()));
        let written: Vec<LogEntry> = (1..=5).map(entry).collect();
        write_saved(&path, written.iter()).unwrap();
        // A damaged line is skipped, the rest still load.
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(f, "{{not json").unwrap();
        writeln!(f, "{}", serde_json::to_string(&SavedLine::of(&entry(6))).unwrap()).unwrap();
        let back = read_saved(&path, 3);
        let ids: Vec<u64> = back.iter().map(|e| e.id).collect();
        assert_eq!(ids, [4, 5, 6], "only the newest `max` are read");
        assert!(back.iter().all(|e| e.earlier && e.level == "error" && e.message.starts_with("entry ")));
        // Oversized: not read.
        fs::write(&path, "x".repeat(SAVED_FILE_MAX as usize + 1)).unwrap();
        assert!(read_saved(&path, 100).is_empty());
        let _ = fs::remove_file(&path);
        assert!(read_saved(&path, 100).is_empty(), "missing file: nothing");
    }

    #[test]
    fn list_keeps_only_the_newest() {
        let mut list = ErrorList { entries: VecDeque::new(), max: 3 };
        for id in 1..=5 {
            list.push(entry(id));
        }
        let ids: Vec<u64> = list.entries.iter().map(|e| e.id).collect();
        assert_eq!(ids, [3, 4, 5]);
        list.max = 1;
        list.prune();
        assert_eq!(list.entries.front().map(|e| e.id), Some(5));
    }

    #[test]
    fn tracing_and_log_messages_reach_the_list() {
        init();
        tracing::error!(target: "probe_tracing", "probe one");
        // The way Tauri's libraries report (through the `log` crate).
        log::warn!(target: "probe_log", "probe two");
        log::info!(target: "probe_log", "probe three is below warn and not kept");
        let (entries, _) = entries();
        let find = |source: &str| entries.iter().filter(|e| e.source == source).cloned().collect::<Vec<_>>();
        let traced = find("probe_tracing");
        assert_eq!((traced.len(), traced[0].level, traced[0].message.as_str()), (1, "error", "probe one"));
        let logged = find("probe_log");
        assert_eq!((logged.len(), logged[0].level, logged[0].message.as_str()), (1, "warn", "probe two"));
    }

    #[test]
    fn long_messages_are_shortened() {
        let text = shorten("é".repeat(MAX_MESSAGE_CHARS + 10));
        assert!(text.ends_with(" [shortened]"));
        assert_eq!(text.chars().count(), MAX_MESSAGE_CHARS + " [shortened]".len());
        assert_eq!(shorten("short".into()), "short");
    }

    #[test]
    fn trimming_keeps_newest_whole_lines() {
        let path = std::env::temp_dir().join(format!("rust-scaffold-log-test-{}.log", std::process::id()));
        let text: String = (0..1000).map(|i| format!("line {i:04}\n")).collect();
        fs::write(&path, &text).unwrap();
        trim_file(&path, 100).unwrap();
        let kept = fs::read_to_string(&path).unwrap();
        let mut lines = kept.lines();
        assert!(lines.next().unwrap().starts_with("[older lines removed"));
        let rest: Vec<&str> = lines.collect();
        assert!(!rest.is_empty() && rest.len() <= 10);
        assert_eq!(*rest.last().unwrap(), "line 0999");
        assert!(rest.iter().all(|l| l.len() == 9 && l.starts_with("line ")));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn append_trims_when_over_the_limit() {
        let path = std::env::temp_dir().join(format!("rust-scaffold-log-limit-{}.log", std::process::id()));
        fs::write(&path, "x".repeat(LOG_FILE_MAX as usize) + "\nold\n").unwrap();
        append_line(&path, "new");
        let len = fs::metadata(&path).unwrap().len();
        assert!(len <= LOG_FILE_MAX / 2 + 100, "file is {len} bytes");
        assert!(fs::read_to_string(&path).unwrap().ends_with("old\nnew\n"));
        let _ = fs::remove_file(&path);
    }
}
