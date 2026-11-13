use std::sync::Mutex;

use serde::Serialize;
use tauri::State;

use crate::config::{self, Config, ConfigState, LoadStatus};
use crate::jobs::{JobInspect, JobsReply, Queue};
use crate::keys::{self, KeysReply};
use crate::logging::{self, LogEntry};
use crate::options::{self, OptionsReply};
use crate::search::{self, SearchReply};
use crate::tray::TrayStatus;

#[derive(Serialize)]
pub struct ConfigReply {
    config: Config,
    status: LoadStatus,
    path: String,
    /// True when the window should stay hidden at startup: start_minimised is
    /// set and the tray exists to bring it back.
    start_hidden: bool,
}

#[derive(Serialize)]
pub struct AppInfo {
    name: &'static str,
    version: &'static str,
    config_path: String,
    rendering: &'static str,
    build: BuildInfo,
}

/// Recorded by build.rs at compile time.
#[derive(Serialize)]
pub struct BuildInfo {
    /// Unix seconds when build.rs last ran; the UI formats it.
    unix_time: u64,
    rustc: &'static str,
    cargo: &'static str,
    profile: &'static str,
    /// Direct dependencies as "name version", with the locked version.
    crates: Vec<&'static str>,
    /// Every crate in Cargo.lock except this app.
    crate_count: usize,
}

impl BuildInfo {
    fn current() -> Self {
        BuildInfo {
            unix_time: env!("BUILD_UNIX_TIME").parse().unwrap_or(0),
            rustc: env!("BUILD_RUSTC"),
            cargo: env!("BUILD_CARGO"),
            profile: env!("BUILD_PROFILE"),
            crates: env!("BUILD_CRATES").split(';').filter(|s| !s.is_empty()).collect(),
            crate_count: env!("BUILD_CRATE_COUNT").parse().unwrap_or(0),
        }
    }
}

/// Example command: the UI calls this and shows the result.
#[tauri::command]
pub fn app_info(state: State<Mutex<ConfigState>>) -> AppInfo {
    let state = state.lock().unwrap();
    AppInfo {
        name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
        config_path: state.path.display().to_string(),
        rendering: crate::gpu::mode(),
        build: BuildInfo::current(),
    }
}

#[tauri::command]
pub fn get_config(state: State<Mutex<ConfigState>>, tray: State<TrayStatus>) -> ConfigReply {
    let state = state.lock().unwrap();
    ConfigReply {
        config: state.config.clone(),
        status: state.status,
        path: state.path.display().to_string(),
        start_hidden: state.config.start_minimised && tray.available(),
    }
}

#[tauri::command]
pub fn save_config(state: State<Mutex<ConfigState>>, jobs: State<Queue>, config: Config) -> Result<(), String> {
    let mut state = state.lock().unwrap();
    config::save(&mut state, config)?;
    logging::set_max(state.config.error_log_max);
    jobs.set_limit(state.config.job_concurrency);
    Ok(())
}

/// Reads options.json fresh on every call, so edits appear without a restart.
#[tauri::command]
pub fn get_options() -> OptionsReply {
    options::load(options::options_path())
}

/// Reads search.json fresh on every call, so edits appear without a rebuild.
#[tauri::command]
pub fn get_search() -> SearchReply {
    search::load(search::search_path())
}

/// Reads keys.json fresh on every call; the page asks at start and when the
/// Help page opens.
#[tauri::command]
pub fn get_keys() -> KeysReply {
    keys::load(keys::keys_path())
}

#[derive(Serialize)]
pub struct ErrorsReply {
    /// Kept warnings and errors, oldest first.
    entries: Vec<LogEntry>,
    /// How many are kept (error_log_max).
    max: usize,
    /// Full path of debug.log, or None when DEBUG_MODE is false.
    log_file: Option<String>,
}

/// Errors and warnings recorded since the app started. New ones also arrive
/// as `log:entry` window events.
#[tauri::command]
pub fn get_errors() -> ErrorsReply {
    let (entries, max) = logging::entries();
    ErrorsReply {
        entries,
        max,
        log_file: logging::log_file().map(|p| p.display().to_string()),
    }
}

/// Empties the Errors tab list. debug.log is not changed.
#[tauri::command]
pub fn clear_errors() {
    logging::clear();
}

/// A JavaScript error or unhandled promise rejection from the window.
/// `location` is "file:line:column" where known. Long text is shortened by
/// the logging layer.
#[tauri::command]
pub fn report_ui_error(message: String, location: String) {
    tracing::error!(target: "ui", "{message} ({location})");
}

/// The Errors tab's test button: records a warning or an error on purpose.
#[tauri::command]
pub fn log_test_entry(level: String) -> Result<(), String> {
    match level.as_str() {
        "error" => tracing::error!(target: "test", "Test error from the Errors tab"),
        "warn" => tracing::warn!(target: "test", "Test warning from the Errors tab"),
        _ => return Err("level must be error or warn".into()),
    }
    Ok(())
}

// ---- Job queue (src/jobs.rs) ---------------------------------------------
// Changes to any job also arrive as `jobs:changed` window events.

/// Every job still listed, the limit, and Tokio runtime figures.
#[tauri::command]
pub fn jobs_list(jobs: State<Queue>) -> JobsReply {
    jobs.list()
}

/// Queues a mock count job (1 to 100, 100 to 10000 ms apart). `fail_at`
/// makes it fail on purpose at that step. Returns the job id.
#[tauri::command]
pub fn jobs_start(jobs: State<Queue>, count: u32, delay_ms: u64, fail_at: Option<u32>) -> Result<u64, String> {
    jobs.start_count(count, delay_ms, fail_at)
}

#[tauri::command]
pub fn jobs_cancel(jobs: State<Queue>, id: u64) -> Result<(), String> {
    jobs.cancel(id)
}

#[tauri::command]
pub fn jobs_cancel_all(jobs: State<Queue>) -> usize {
    jobs.cancel_all()
}

#[tauri::command]
pub fn jobs_clear_finished(jobs: State<Queue>) -> usize {
    jobs.clear_finished()
}

#[tauri::command]
pub fn jobs_inspect(jobs: State<Queue>, id: u64) -> Result<JobInspect, String> {
    jobs.inspect(id)
}
