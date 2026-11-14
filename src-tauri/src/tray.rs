use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::config::{ConfigState, TrayItem};
use tracing::{debug, info};

const ID_TOGGLE: &str = "tray-toggle";
const ID_QUIT: &str = "tray-quit";
const ID_CUSTOM_PREFIX: &str = "tray-custom-";

/// Linux reports each tray click on its own (KSNI backend, see the
/// `linux-tray-clicks` feature in Cargo.toml), never a double-click, so two
/// left clicks this close together count as one. Kim's double-clicks on the
/// LXQt panel measured 126 to 426 ms between clicks.
const DOUBLE_CLICK: Duration = Duration::from_millis(500);

/// Whether the tray icon was created. Read at startup so that
/// `start_minimised` is ignored when there is no tray to restore the window from.
pub struct TrayStatus(pub AtomicBool);

impl TrayStatus {
    pub fn available(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// The saved window size and the hide_to_tray setting, read at the moment
/// of use so a change saved on the Options tab applies at once.
fn window_settings(app: &AppHandle) -> (u32, u32, bool) {
    let defaults = crate::config::Config::default();
    app.try_state::<Mutex<ConfigState>>()
        .and_then(|state| state.lock().ok().map(|s| (s.config.width, s.config.height, s.config.hide_to_tray)))
        .unwrap_or((defaults.width, defaults.height, defaults.hide_to_tray))
}

fn tray_available(app: &AppHandle) -> bool {
    app.try_state::<TrayStatus>().is_some_and(|tray| tray.available())
}

/// Brings the window back. A hidden window is centred on the primary
/// monitor first; a minimised one is restored where it was.
fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if !window.is_visible().unwrap_or(true) {
            let (width, height, _) = window_settings(app);
            crate::centre_on_primary(&window, width, height);
        }
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Hides the window off the taskbar, or minimises it to the taskbar when
/// hide_to_tray is off. Without a tray it always minimises, so the window
/// can never become unreachable.
fn hide_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let (_, _, hide_to_tray) = window_settings(app);
        if hide_to_tray && tray_available(app) {
            let _ = window.hide();
        } else {
            let _ = window.minimize();
        }
    }
}

/// Visible and not minimised: hide it (see hide_window). Otherwise bring it back.
fn toggle_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let visible = window.is_visible().unwrap_or(false);
        let minimised = window.is_minimized().unwrap_or(false);
        if visible && !minimised {
            hide_window(app);
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

/// Windows and macOS report double-clicks themselves. On Linux each left
/// click is reported once, when the button is released, and a double-click
/// is two of them within `DOUBLE_CLICK`.
fn is_double_click(event: &TrayIconEvent, last_click: &Mutex<Option<Instant>>) -> bool {
    match event {
        TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => !cfg!(target_os = "linux"),
        TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
            if cfg!(target_os = "linux") =>
        {
            let Ok(mut last) = last_click.lock() else { return false };
            second_click(&mut last, Instant::now())
        }
        _ => false,
    }
}

/// True when `now` is the second click of a pair. A pair is used up, so a
/// third quick click starts a new pair instead of toggling again.
fn second_click(last: &mut Option<Instant>, now: Instant) -> bool {
    match last.take() {
        Some(previous) if now.duration_since(previous) <= DOUBLE_CLICK => true,
        _ => {
            *last = Some(now);
            false
        }
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
    let last_click: Mutex<Option<Instant>> = Mutex::new(None);
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
        .on_tray_icon_event(move |tray, event| {
            if is_double_click(&event, &last_click) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_quick_clicks_are_a_double_click() {
        let start = Instant::now();
        let ms = |n| start + Duration::from_millis(n);
        let mut last = None;
        assert!(!second_click(&mut last, ms(0)));
        assert!(second_click(&mut last, ms(426)), "Kim's slowest measured pair");
        // A third and fourth quick click make a second pair, not a toggle each.
        assert!(!second_click(&mut last, ms(600)));
        assert!(second_click(&mut last, ms(750)));
        // Too slow: the second click starts a new pair.
        assert!(!second_click(&mut last, ms(2000)));
        assert!(!second_click(&mut last, ms(2501)));
        assert!(second_click(&mut last, ms(2700)));
    }
}

