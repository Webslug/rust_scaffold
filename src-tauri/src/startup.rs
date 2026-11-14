//! Start-up fallback. The window starts hidden and `app.js` shows it once the
//! settings are loaded (or keeps it hidden for start_minimised), then calls
//! `ui_ready`. If that call has not arrived after `FALLBACK_AFTER`, the
//! interface failed to start: the window is shown anyway and an error is
//! logged, so the Errors page can say what went wrong instead of the app
//! running with no window.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tracing::{error, info};

pub const FALLBACK_AFTER: Duration = Duration::from_secs(5);

static UI_READY: AtomicBool = AtomicBool::new(false);

/// Called by the `ui_ready` command.
pub fn mark_ready() {
    if !UI_READY.swap(true, Ordering::SeqCst) {
        info!("interface ready");
    }
}

/// Starts the timer. Runs on Tauri's Tokio runtime; the sleep is an await
/// point, so nothing is blocked while it waits.
pub fn arm(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FALLBACK_AFTER).await;
        if UI_READY.load(Ordering::SeqCst) {
            return;
        }
        error!(
            "interface did not report ready within {} s; window shown by the start-up fallback (see any script errors listed before this one)",
            FALLBACK_AFTER.as_secs()
        );
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
    });
}
