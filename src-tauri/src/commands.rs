use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;
use tauri::State;

use crate::config::{self, Config, ConfigState, LoadStatus};
use crate::automation::{Automation, AutomationReply, Bulk, KindView, Section, TestReply};
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
pub fn save_config(
    state: State<Mutex<ConfigState>>,
    jobs: State<Queue>,
    config: Config,
) -> Result<(), String> {
    let mut state = state.lock().unwrap();
    config::save(&mut state, config)?;
    logging::set_max(state.config.error_log_max);
    jobs.set_limit(state.config.job_concurrency);
    jobs.set_resource_locks(state.config.job_resource_locks);
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

/// app.js calls this once the window is shown, or deliberately left hidden
/// for start_minimised. Without it, the start-up fallback shows the window.
#[tauri::command]
pub fn ui_ready() {
    crate::startup::mark_ready();
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
// Changes to any job also arrive as `jobs:changed` window events. The Jobs
// page shows queued and running jobs under "Running now".

/// Every job still listed, the limit, and Tokio runtime figures.
#[tauri::command]
pub fn jobs_list(jobs: State<Queue>) -> JobsReply {
    jobs.list()
}

/// Kill: stops a queued or running job.
#[tauri::command]
pub fn jobs_cancel(jobs: State<Queue>, id: u64) -> Result<(), String> {
    jobs.cancel(id)
}

#[tauri::command]
pub fn jobs_inspect(jobs: State<Queue>, id: u64) -> Result<JobInspect, String> {
    jobs.inspect(id)
}

/// The kinds an Action can be: the job kinds (with an example input) and
/// the built-in ones (switch, delay, set_count ...).
#[tauri::command]
pub fn jobs_kinds(jobs: State<Queue>) -> Vec<KindView> {
    let mut kinds = crate::automation::kind_views(jobs.kinds());
    kinds.extend(crate::automation::builtin_views());
    kinds
}

// ---- Triggers, Schedules and Actions (src/automation.rs, jobs.json) --------
// Changes also arrive as `automation:changed` window events.

/// Every Trigger, Schedule and Action, with readings, next times, recent
/// results and any entries skipped as invalid.
#[tauri::command]
pub fn automation_list(automation: State<Automation>) -> AutomationReply {
    automation.list()
}

/// Reads jobs.json again after a hand edit.
#[tauri::command]
pub fn automation_reload(automation: State<Automation>) -> AutomationReply {
    automation.reload();
    automation.list()
}

/// Adds an entry (`oldName` empty) or replaces the one called `oldName`,
/// writing jobs.json. Refused, with nothing written, when not valid.
#[tauri::command]
pub fn automation_save(
    automation: State<Automation>,
    section: Section,
    old_name: Option<String>,
    definition: Value,
) -> Result<AutomationReply, String> {
    automation.save(section, old_name, definition)?;
    Ok(automation.list())
}

#[tauri::command]
pub fn automation_delete(automation: State<Automation>, section: Section, name: String) -> Result<AutomationReply, String> {
    automation.delete(section, &name)?;
    Ok(automation.list())
}

/// Copies an entry; the reply's list holds the copy. Returns the new name.
#[tauri::command]
pub fn automation_duplicate(automation: State<Automation>, section: Section, name: String) -> Result<String, String> {
    automation.duplicate(section, &name)
}

#[tauri::command]
pub fn automation_set_enabled(
    automation: State<Automation>,
    section: Section,
    name: String,
    enabled: bool,
) -> Result<AutomationReply, String> {
    automation.set_enabled(section, &name, enabled)?;
    Ok(automation.list())
}

/// The Overview's quick buttons: enable_all, disable_all, delete_all,
/// restore_demos.
#[tauri::command]
pub fn automation_bulk(automation: State<Automation>, what: Bulk) -> Result<AutomationReply, String> {
    automation.bulk(what)?;
    Ok(automation.list())
}

/// Fire now on a Trigger: reads its page at once and starts its actions
/// with the reading. Returns the reading.
#[tauri::command]
pub async fn automation_fire_trigger(automation: State<'_, Automation>, name: String) -> Result<Value, String> {
    automation.fire_trigger(&name).await
}

/// Run now on a Schedule: starts its actions at once. Returns how many.
#[tauri::command]
pub fn automation_run_schedule(automation: State<Automation>, name: String) -> Result<usize, String> {
    automation.run_schedule(&name)
}

/// The Run button of an Action. Returns its job id (none for a switch).
#[tauri::command]
pub fn automation_run(automation: State<Automation>, name: String) -> Result<Option<u64>, String> {
    automation.run(&name)
}

/// "Test now" on the Trigger form: reads the watch and checks the
/// condition. `name` is the saved Trigger being edited, if any, to compare
/// a "changes" Trigger with its last reading.
#[tauri::command]
pub async fn automation_test_trigger(
    automation: State<'_, Automation>,
    name: Option<String>,
    definition: Value,
) -> Result<TestReply, String> {
    automation.test_trigger_named(name, definition).await
}

/// The models installed in a local Ollama (for the AI job's model list).
/// Reads the model list only; no model is loaded.
#[tauri::command]
pub async fn ollama_models(endpoint: Option<String>) -> Result<Vec<crate::job_kinds::ModelInfo>, String> {
    crate::job_kinds::ollama_models(endpoint.as_deref()).await
}
