use std::sync::Mutex;

use serde::Serialize;
use tauri::State;

use crate::config::{self, Config, ConfigState, LoadStatus};
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
}

/// Example command: the UI calls this and shows the result.
#[tauri::command]
pub fn app_info(state: State<Mutex<ConfigState>>) -> AppInfo {
    let state = state.lock().unwrap();
    AppInfo {
        name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
        config_path: state.path.display().to_string(),
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
pub fn save_config(state: State<Mutex<ConfigState>>, config: Config) -> Result<(), String> {
    let mut state = state.lock().unwrap();
    config::save(&mut state, config)
}
