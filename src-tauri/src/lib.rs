mod commands;
mod config;
mod files;
mod gpu;
mod jobs;
mod keys;
mod logging;
mod options;
mod search;
mod startup;
mod tray;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, OnceLock};

use tauri::{Emitter, LogicalSize, Manager, PhysicalPosition};
use tracing::{info, warn};

/// When true, program events from debug level up are appended to
/// `debug.log` in the settings folder (see `logging.rs`). Warnings and errors
/// reach the Errors tab either way.
pub const DEBUG_MODE: bool = false;

/// Name of the per-user settings folder, for example `~/.config/rust_scaffold`
/// on Linux and `%APPDATA%\rust_scaffold` on Windows. Change this when a new
/// app is made from the template.
const APP_FOLDER: &str = "rust_scaffold";

/// Environment variable that points the app at another settings folder. The
/// benchmark and tests use it so they never touch the real settings.
pub const DIR_OVERRIDE: &str = "RUST_SCAFFOLD_DIR";

static SETTINGS_DIR: OnceLock<PathBuf> = OnceLock::new();

/// The folder holding the executable, with symlinks followed to the real
/// file. Falls back to the working directory if the path cannot be found.
pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .and_then(|exe| exe.canonicalize())
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The folder holding config.json, options.json, search.json, keys.json and
/// debug.log.
/// Worked out once per run:
/// 1. `RUST_SCAFFOLD_DIR`, if set (created if missing).
/// 2. The per-user config folder from the `dirs` crate plus `APP_FOLDER`.
/// 3. The executable's folder, if neither of the above can be used.
/// Nothing is copied in from anywhere: each loader writes its built-in
/// default when its file is missing.
pub fn settings_dir() -> &'static Path {
    let mut notes = Vec::new();
    let dir = SETTINGS_DIR.get_or_init(|| find_settings_dir(&mut notes));
    // Logged after the folder is set: the log file itself lives there.
    for (problem, note) in notes {
        if problem {
            warn!("{note}");
        } else {
            info!("{note}");
        }
    }
    dir
}

/// Notes are (is a problem, text).
fn find_settings_dir(notes: &mut Vec<(bool, String)>) -> PathBuf {
    if let Some(dir) = std::env::var_os(DIR_OVERRIDE).filter(|v| !v.is_empty()).map(PathBuf::from) {
        match fs::create_dir_all(&dir) {
            Ok(()) => {
                notes.push((false, format!("settings: {} (from {DIR_OVERRIDE})", dir.display())));
                return dir;
            }
            Err(e) => notes.push((true, format!("settings: cannot use {DIR_OVERRIDE} {}: {e}", dir.display()))),
        }
    }
    pick_user_dir(dirs::config_dir(), exe_dir(), notes)
}

/// Steps 2 and 3 of `settings_dir`, split out so tests can pass temporary
/// folders instead of the real user config folder.
fn pick_user_dir(config_root: Option<PathBuf>, exe: PathBuf, notes: &mut Vec<(bool, String)>) -> PathBuf {
    let Some(dir) = config_root.map(|d| d.join(APP_FOLDER)) else {
        notes.push((true, format!("settings: no user config folder, using {}", exe.display())));
        return exe;
    };
    if let Err(e) = fs::create_dir_all(&dir) {
        notes.push((true, format!("settings: cannot create {}: {e}; using {}", dir.display(), exe.display())));
        return exe;
    }
    notes.push((false, format!("settings: {}", dir.display())));
    dir
}

/// Places the window in the middle of the primary monitor, for a window of
/// `width` x `height` logical pixels. Call it while the window is hidden:
/// the window manager re-places a hidden window when it is shown, ignoring a
/// plain center() call.
pub(crate) fn centre_on_primary(window: &tauri::WebviewWindow, width: u32, height: u32) {
    if let Ok(Some(monitor)) = window.primary_monitor() {
        let scale = monitor.scale_factor();
        let (w, h) = ((width as f64 * scale) as i32, (height as f64 * scale) as i32);
        let size = monitor.size();
        let origin = monitor.position();
        let x = origin.x + (size.width as i32 - w) / 2;
        let y = origin.y + (size.height as i32 - h) / 2;
        let _ = window.set_position(PhysicalPosition::new(x.max(origin.x), y.max(origin.y)));
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logging::init();
    // Work out the settings folder now, so its notes are logged in order.
    settings_dir();
    info!("app starting, version {}", env!("CARGO_PKG_VERSION"));
    let state = config::load(config::config_path());
    logging::set_max(state.config.error_log_max);
    // Must run before WebKit starts: it may set environment variables.
    gpu::apply(state.config.hardware_acceleration);
    let (width, height) = (state.config.width, state.config.height);
    let tray_items = state.config.tray_items.clone();
    let job_limit = state.config.job_concurrency;
    tauri::Builder::default()
        .manage(Mutex::new(state))
        .setup(move |app| {
            logging::set_app(app.handle().clone());
            // Jobs run on Tauri's own Tokio runtime; each change is pushed to
            // the window.
            let events = app.handle().clone();
            let notify: jobs::Notify = std::sync::Arc::new(move |job: &jobs::JobView| {
                let _ = events.emit(jobs::CHANGED_EVENT, job);
            });
            app.manage(jobs::Queue::new(tauri::async_runtime::handle().inner().clone(), job_limit, notify));
            // Restore the saved size while the window is still hidden.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_size(LogicalSize::new(width as f64, height as f64));
                centre_on_primary(&window, width, height);
                info!("window: restored size {width}x{height}");
            }
            let tray_ok = match tray::setup(app.handle(), &tray_items) {
                Ok(()) => true,
                Err(e) => {
                    warn!("tray: could not be created: {e}");
                    false
                }
            };
            app.manage(tray::TrayStatus(AtomicBool::new(tray_ok)));
            // Shows the window if app.js never reports ready.
            startup::arm(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::get_config,
            commands::save_config,
            commands::get_options,
            commands::get_search,
            commands::get_keys,
            commands::get_errors,
            commands::clear_errors,
            commands::report_ui_error,
            commands::log_test_entry,
            commands::ui_ready,
            commands::jobs_list,
            commands::jobs_start,
            commands::jobs_cancel,
            commands::jobs_cancel_all,
            commands::jobs_clear_finished,
            commands::jobs_inspect
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_copied_from_the_executable_folder() {
        let base = std::env::temp_dir().join(format!("rust_scaffold_lib_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let (root, exe) = (base.join("config"), base.join("bin"));
        fs::create_dir_all(&exe).unwrap();
        for name in ["config.json", "options.json", "search.json", "keys.json"] {
            fs::write(exe.join(name), "{\"from\":\"exe\"}").unwrap();
        }
        let mut notes = Vec::new();
        let dir = pick_user_dir(Some(root.clone()), exe.clone(), &mut notes);
        assert_eq!(dir, root.join(APP_FOLDER));
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0, "settings folder must start empty");

        // The loaders then write their defaults, not the executable's copies.
        let state = config::load(dir.join("config.json"));
        assert_eq!(state.status, config::LoadStatus::Created);
        assert!(!fs::read_to_string(dir.join("config.json")).unwrap().contains("exe"));

        // No user config folder: the executable's folder is used as is.
        assert_eq!(pick_user_dir(None, exe.clone(), &mut notes), exe);
        let _ = fs::remove_dir_all(&base);
    }
}
