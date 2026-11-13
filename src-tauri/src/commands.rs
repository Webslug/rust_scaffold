use std::sync::Mutex;

use serde::Serialize;
use tauri::State;

use crate::config::{self, Config, ConfigState, LoadStatus};
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
