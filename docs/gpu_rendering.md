# GPU rendering investigation (13 November 2026)

## Result

The app cannot render on the GPU on this machine as it is configured today.
WebKitGTK forces its hardware acceleration policy to "never", and refuses a
request to change it. The cause is outside the app: the NVIDIA GBM buffer
allocation that WebKitGTK needs fails because NVIDIA kernel mode setting is
off (`nomodeset` on the kernel command line).

No cpu.py or bench.py measurement of a "GPU build" was made, because no GPU
build could be produced: the variant rendered exactly like the normal build.

## Machine

- WebKitGTK 2.50.4, GTK 3.24.33, X11, LXQt, picom.
- NVIDIA driver 535.309.01, RTX 3060, `glxinfo`: direct rendering yes,
  renderer "NVIDIA GeForce RTX 3060".
- Kernel command line: `... quiet splash nomodeset module.sig_unenforce ...`.

## Evidence, in the order it was found

1. `nvidia-smi` lists the app's main process as a GPU client (2 MiB) but
   never the `WebKitWebProcess` that paints the page. Same with
   `WEBKIT_DISABLE_DMABUF_RENDERER=1`, `WEBKIT_FORCE_COMPOSITING_MODE=1` and
   `WEBKIT_DMABUF_RENDERER_FORCE_SHM=1`.
2. wry (Tauri's webview layer) never sets a hardware acceleration policy, so
   WebKit's own default applies.
3. A variant build read and set the policy through the `webkit2gtk` crate
   (code below). Reported: "policy before Never, after Never" with
   `always`, `never` and unset. WebKit ignores the request.
4. WebKit's own MiniBrowser on `webkit://gpu` reports
   `"Hardware Acceleration Information": { "Policy": "never" }`. So the
   decision is system-wide, not caused by Tauri or this app.
5. `/dev/dri/renderD128` (NVIDIA) exists and opens fine for user kim, so it
   is not a permissions problem.
6. Creating a GBM buffer on that device (what WebKitGTK does for its DMA-BUF
   renderer) fails:
   `GBM-DRV error (nv_gbm_bo_create): DRM_IOCTL_NVIDIA_GEM_ALLOC_NVKMS_MEMORY failed (ret=-1)`.
   The GBM device itself opens (backend "nvidia"); only the allocation fails.
   NVKMS allocation needs `nvidia-drm` kernel mode setting.

Not verified: the value of `/sys/module/nvidia_drm/parameters/modeset`
(readable by root only). Step 6 is the practical test.

## What would change it

Enabling NVIDIA kernel mode setting: add `nvidia-drm.modeset=1` to
`GRUB_CMDLINE_LINUX_DEFAULT` and remove `nomodeset`, run `update-grub`,
reboot. This is not a driver update, but it changes how the machine boots,
and `nomodeset` may have been added on purpose (for example to fix a black
screen at install time). Only do it after a backup. If the machine then fails
to show a desktop, press `e` at the GRUB menu and put `nomodeset` back for
that boot.

Cheapest check afterwards, before touching the app: run the GBM test in
step 6 again, then open `webkit://gpu` in
`/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/MiniBrowser`. If the policy is no
longer "never", build the variant below and measure with `cpu.py` in both a
light and a dark theme, maximised.

Other routes, not recommended: a different web engine (CEF or Electron) would
use GLX without GBM, but adds well over 100 MB and replaces Tauri's webview.

## Variant code (not in the source tree)

`src-tauri/Cargo.toml`:

    [target.'cfg(target_os = "linux")'.dependencies]
    # Same version as Tauri uses, so the types match.
    webkit2gtk = { version = "=2.0.2", features = ["v2_16"] }

`src-tauri/src/gpu.rs`, called as `gpu::apply(&window);` in `lib.rs` setup
right after `set_size` (add `mod gpu;`):

    #[cfg(target_os = "linux")]
    pub fn apply(window: &tauri::WebviewWindow) {
        let wanted = std::env::var("RS_GPU_POLICY").ok();
        let _ = window.with_webview(move |webview| {
            use webkit2gtk::{HardwareAccelerationPolicy as Policy, SettingsExt, WebViewExt};
            let view = webview.inner();
            let Some(settings) = WebViewExt::settings(&view) else { return };
            let before = settings.hardware_acceleration_policy();
            let policy = match wanted.as_deref() {
                Some("always") => Some(Policy::Always),
                Some("never") => Some(Policy::Never),
                Some("on_demand") => Some(Policy::OnDemand),
                _ => None,
            };
            if let Some(p) = policy {
                settings.set_hardware_acceleration_policy(p);
            }
            if std::env::var("RS_GPU_REPORT").is_ok() {
                eprintln!("gpu: policy before {:?}, after {:?}",
                          before, settings.hardware_acceleration_policy());
            }
        });
    }

    #[cfg(not(target_os = "linux"))]
    pub fn apply(_window: &tauri::WebviewWindow) {}

A "Hardware acceleration" switch in `config.json` (on by default, applied at
startup, needs a restart) would replace `RS_GPU_POLICY` with a config value.
It was not added: on this machine it would do nothing and could not be tested.

## Update, 13 November 2026: GPU rendering now works

- NVIDIA mode setting was switched on permanently (see
  `docs/grub_recovery.md`). GBM buffers then allocate fine.
- WebKitGTK still chooses policy "never" on the NVIDIA proprietary driver
  by default. `WEBKIT_FORCE_DMABUF_RENDERER=1` turns it on (policy
  "always", GL renderer RTX 3060), but the window is black ("AcceleratedSurface
  was unable to construct a complete framebuffer"). Adding
  `WEBKIT_DMABUF_RENDERER_FORCE_SHM=1` makes it render correctly, and the
  `WebKitWebProcess` then appears in `nvidia-smi` as a GPU client (29 to
  34 MiB).
- `cpu.py` (vertical, maximised, 2 rounds): web process CPU Slate 16.50 s
  normal versus 15.10 s GPU, Aero Blue 18.14 s versus 16.33 s (8 to 10 %
  less). Kim watched the runs and saw much less tearing while scrolling in
  Aero Blue in GPU mode. picom was not running during those runs.
- The app now does this itself (`src-tauri/src/gpu.rs`, called first in
  `run()`): with `hardware_acceleration` true in `config.json` (the
  default), the NVIDIA driver present and a test GBM buffer allocating, it
  sets both variables before WebKit starts. Otherwise it leaves WebKit's
  default (CPU here), so the window is never black, for example when an
  older kernel without mode setting is booted. Variables already set in the
  environment are respected. The Help page "Rendering" line shows the
  decision. The switch is on the Options tab (Window group) and applies at
  the next start. `run_gpu.sh` was removed; `run.sh` is the only launcher.
- Verified on the release build: with the switch on, `WebKitWebProcess`
  is a GPU client in `nvidia-smi` and the window renders; with it off, it is
  not. 25 Rust tests pass. Not tested: the fallback on a boot without mode
  setting (needs the old kernel), Windows, other GPUs.
