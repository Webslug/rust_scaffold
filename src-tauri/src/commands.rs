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
