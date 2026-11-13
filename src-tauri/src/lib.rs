mod commands;
mod config;
mod options;
mod tray;

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use tauri::{LogicalSize, Manager, PhysicalPosition};
use std::time::{SystemTime, UNIX_EPOCH};

/// When true, program events are appended to `debug.log` in the working
/// directory (the project folder when launched via run.sh / run.bat).
pub const DEBUG_MODE: bool = false;

const DEBUG_LOG: &str = "debug.log";

/// Appends one timestamped line to the debug log. Does nothing unless
/// DEBUG_MODE is true; logging failures are ignored so they never stop the app.
pub fn debug_log(message: &str) {
    if !DEBUG_MODE {
        return;
    }
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(DEBUG_LOG) {
        let _ = writeln!(file, "[{secs}] {message}");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    debug_log("app starting");
    let state = config::load(config::config_path());
    let (width, height) = (state.config.width, state.config.height);
    let tray_items = state.config.tray_items.clone();
    tauri::Builder::default()
        .manage(Mutex::new(state))
        .setup(move |app| {
            // Restore the saved size while the window is still hidden.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_size(LogicalSize::new(width as f64, height as f64));
                // Centre explicitly: the window manager re-places a hidden
                // window when it is shown, ignoring a plain center() call.
                if let Ok(Some(monitor)) = window.primary_monitor() {
                    let scale = monitor.scale_factor();
                    let (w, h) = ((width as f64 * scale) as i32, (height as f64 * scale) as i32);
                    let size = monitor.size();
                    let origin = monitor.position();
                    let x = origin.x + (size.width as i32 - w) / 2;
                    let y = origin.y + (size.height as i32 - h) / 2;
                    let _ = window.set_position(PhysicalPosition::new(x.max(origin.x), y.max(origin.y)));
                }
                debug_log(&format!("window: restored size {width}x{height}"));
            }
            let tray_ok = match tray::setup(app.handle(), &tray_items) {
                Ok(()) => true,
                Err(e) => {
                    debug_log(&format!("tray: could not be created: {e}"));
                    false
                }
            };
            app.manage(tray::TrayStatus(AtomicBool::new(tray_ok)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::get_config,
            commands::save_config,
            commands::get_options
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
