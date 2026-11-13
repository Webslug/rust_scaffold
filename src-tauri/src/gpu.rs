//! GPU rendering on Linux with the NVIDIA proprietary driver.
//!
//! WebKitGTK does not use its DMA-BUF (GPU) renderer on the NVIDIA
//! proprietary driver unless forced, and with hardware buffers the window
//! stays black. Forcing the renderer with shared-memory buffers works. Both
//! variables must be set before WebKit starts, so `apply` runs first thing in
//! `run`. They are only set when a GPU buffer can really be allocated (NVIDIA
//! kernel mode setting on), otherwise the window would be black.
//! See docs/gpu_rendering.md.

use std::sync::OnceLock;

use tracing::{info};

const FORCE_RENDERER: &str = "WEBKIT_FORCE_DMABUF_RENDERER";
const FORCE_SHM: &str = "WEBKIT_DMABUF_RENDERER_FORCE_SHM";

static MODE: OnceLock<&'static str> = OnceLock::new();

/// How the page is being drawn, for the Help page.
pub fn mode() -> &'static str {
    MODE.get().copied().unwrap_or("CPU")
}

fn decide(enabled: bool) -> &'static str {
    if !enabled {
        return "CPU (hardware acceleration off in settings)";
    }
    if std::env::var_os(FORCE_RENDERER).is_some() || std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_some() {
        return "Set by WebKit environment variables";
    }
    if !nvidia_driver() {
        return "WebKit default (no NVIDIA driver)";
    }
    if !gbm_buffer_works() {
        return "CPU (GPU buffers unavailable, NVIDIA mode setting off)";
    }
    // Safe here: no other thread exists yet. In edition 2024 set_var becomes
    // an unsafe function for that reason, so this needs an unsafe block then.
    std::env::set_var(FORCE_RENDERER, "1");
    std::env::set_var(FORCE_SHM, "1");
    "GPU (NVIDIA)"
}

/// Chooses the rendering path. Call before the Tauri builder starts.
pub fn apply(enabled: bool) {
    let mode = if cfg!(target_os = "linux") { decide(enabled) } else { "WebKit default" };
    info!("rendering: {mode}");
    let _ = MODE.set(mode);
}

fn nvidia_driver() -> bool {
    std::path::Path::new("/proc/driver/nvidia/version").exists()
}

#[cfg(target_os = "linux")]
fn gbm_buffer_works() -> bool {
    use std::ffi::{c_char, c_int, c_void, CString};
    use std::os::fd::AsRawFd;

    extern "C" {
        fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlclose(handle: *mut c_void) -> c_int;
    }
    const RTLD_NOW: c_int = 2;
    const GBM_FORMAT_ARGB8888: u32 = 0x3432_5241;
    const GBM_BO_USE_RENDERING: u32 = 1 << 2;
    type CreateDevice = unsafe extern "C" fn(c_int) -> *mut c_void;
    type DestroyDevice = unsafe extern "C" fn(*mut c_void);
    type BoCreate = unsafe extern "C" fn(*mut c_void, u32, u32, u32, u32) -> *mut c_void;
    type BoDestroy = unsafe extern "C" fn(*mut c_void);

    let Ok(lib) = CString::new("libgbm.so.1") else { return false };
    // SAFETY: plain libdl calls; every symbol is checked for null before its
    // use and cast to the signature documented in gbm.h.
    unsafe {
        let handle = dlopen(lib.as_ptr(), RTLD_NOW);
        if handle.is_null() {
            return false;
        }
        let sym = |name: &str| CString::new(name).map(|n| dlsym(handle, n.as_ptr())).unwrap_or(std::ptr::null_mut());
        let (cd, dd, bc, bd) = (sym("gbm_create_device"), sym("gbm_device_destroy"), sym("gbm_bo_create"), sym("gbm_bo_destroy"));
        let mut ok = false;
        if !cd.is_null() && !dd.is_null() && !bc.is_null() && !bd.is_null() {
            let create_device: CreateDevice = std::mem::transmute(cd);
            let destroy_device: DestroyDevice = std::mem::transmute(dd);
            let bo_create: BoCreate = std::mem::transmute(bc);
            let bo_destroy: BoDestroy = std::mem::transmute(bd);
            let mut nodes: Vec<_> = std::fs::read_dir("/dev/dri")
                .map(|dir| dir.filter_map(|e| e.ok().map(|e| e.path())).collect())
                .unwrap_or_default();
            nodes.retain(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("renderD")));
            nodes.sort();
            for node in nodes {
                let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(&node) else { continue };
                let device = create_device(file.as_raw_fd());
                if device.is_null() {
                    continue;
                }
                let bo = bo_create(device, 256, 256, GBM_FORMAT_ARGB8888, GBM_BO_USE_RENDERING);
                if !bo.is_null() {
                    bo_destroy(bo);
                    ok = true;
                }
                destroy_device(device);
                if ok {
                    break;
                }
            }
        }
        dlclose(handle);
        ok
    }
}

#[cfg(not(target_os = "linux"))]
fn gbm_buffer_works() -> bool {
    false
}
