use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::config::TrayItem;
use tracing::{debug, info};

const ID_TOGGLE: &str = "tray-toggle";
const ID_QUIT: &str = "tray-quit";
const ID_CUSTOM_PREFIX: &str = "tray-custom-";

/// Whether the tray icon was created. Read at startup so that
/// `start_minimised` is ignored when there is no tray to restore the window from.
pub struct TrayStatus(pub AtomicBool);

impl TrayStatus {
    pub fn available(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Brings the window back from hidden or minimised.
fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn hide_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

/// Visible and not minimised: minimise it. Otherwise bring it back.
fn toggle_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let visible = window.is_visible().unwrap_or(false);
        let minimised = window.is_minimized().unwrap_or(false);
        if visible && !minimised {
            let _ = window.minimize();
        } else {
            show_window(app);
        }
    }
}

fn run_action(app: &AppHandle, action: &str) {
    debug!("tray: action {action}");
    match action {
        "show" => show_window(app),
        "hide" => hide_window(app),
        "toggle" => toggle_window(app),
        "quit" => app.exit(0),
        _ => {}
    }
}

/// Creates the tray icon. It lives as long as the app does, whether the
/// window is visible, minimised or hidden. Menu: Show/Hide first, the custom
/// items from config.json, Quit last.
pub fn setup(app: &AppHandle, items: &[TrayItem]) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, ID_TOGGLE, "Show / Hide", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, ID_QUIT, "Quit", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;

    let custom: Vec<MenuItem<_>> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            MenuItem::with_id(app, format!("{ID_CUSTOM_PREFIX}{i}"), &item.label, true, None::<&str>)
        })
        .collect::<tauri::Result<_>>()?;

    let mut entries: Vec<&dyn tauri::menu::IsMenuItem<_>> = vec![&toggle];
    if !custom.is_empty() {
        entries.push(&separator);
        entries.extend(custom.iter().map(|c| c as &dyn tauri::menu::IsMenuItem<_>));
    }
    let separator_last = PredefinedMenuItem::separator(app)?;
    entries.push(&separator_last);
    entries.push(&quit);
    let menu = Menu::with_items(app, &entries)?;

    let actions: Vec<String> = items.iter().map(|i| i.action.clone()).collect();
    let mut builder = TrayIconBuilder::new()
        .tooltip(env!("CARGO_PKG_NAME"))
        .menu(&menu)
        // Left click opens the menu only where the platform does it anyway;
        // on Windows a double-click toggles the window instead.
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| {
            let id = event.id().as_ref();
            if id == ID_TOGGLE {
                run_action(app, "toggle");
            } else if id == ID_QUIT {
                run_action(app, "quit");
            } else if let Some(index) = id.strip_prefix(ID_CUSTOM_PREFIX).and_then(|n| n.parse::<usize>().ok()) {
                if let Some(action) = actions.get(index) {
                    run_action(app, action);
                }
            }
        })
        // Not emitted on Linux (see AGENTS.md); works on Windows and macOS.
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::DoubleClick { .. } = event {
                toggle_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    info!("tray: created");
    Ok(())
}
