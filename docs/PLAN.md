# Rust + Tauri Scaffold: Project Plan

Date: 12 November 2026
Status: approved 12 November 2026. Phase 4 complete; Phase 5 complete. Remaining: performance pass (see AGENTS.md known issue).

Additions agreed at approval:
- No React, no Vite (confirmed).
- `config.json` lives in the project folder (confirmed).
- Resizing is optional; drop it if frameless resizing on Linux proves troublesome.
- Each theme paints the window with a subtle top-to-bottom gradient (light themes: tint to white; dark themes: lifted tinted shade to the theme's base colour). Tokens `--window-bg-top` / `--window-bg-bottom` in `ui/css/variables.css`.
- Subtle bevelling on all buttons and containers. The template's `--bevel-shadow` tokens already do this for buttons, cards, accordions, alerts and dialogs; the window frame and title bar get their own bevel in `window.css`.
Source of requirements: `/opt/templates/rust/AGENTS.md`

---

## Findings from inspection

- `/opt/templates/rust` holds only `AGENTS.md` and empty `backup/` and `docs/` folders. Everything is built fresh.
- Reference app `/home/kim/projects/water_tracker/tauri-app` is Tauri 2 + React 19 + Vite 7. Its `lib.rs` is the untouched `greet` template. Its window config is 360x540, `resizable: false`, `decorations: false`, `transparent: false` (transparency was switched off again after corner artifacts, see its `docs/HANDOFF.md`).
- Useful lessons from that app:
  - Dragging needs the capability `core:window:allow-start-dragging` plus `data-tauri-drag-region` on the drag surface.
  - Transparency on LXQt/X11 needs a compositor. picom is installed; its own shadows must be off (`shadow = false`) or corners go dirty.
  - Corner halo was caused by two stacked elements with separate rounded corners and a semi-transparent border. Use one rounded outer element.
  - Release binary was 15 MB unoptimised.
- Installed: rustc/cargo 1.98, Node 24.14, npm 11.9, WebKitGTK 4.1 (2.50.4), GTK 3.24. The Tauri CLI is **not** installed globally (`cargo tauri` fails); water_tracker used the npm `@tauri-apps/cli`.
- Bootstrap template `/opt/templates/bootstrap/assets` (2.1 MB) already provides modular CSS (`index.css` + modules), `variables.css` with 15 `data-theme` themes, Bootstrap Icons (woff2), `theme-switcher.js` and `sortable.js`. Both scripts persist to `localStorage`, not to a file.

---

## Key decisions (veto any of these before Phase 1)

1. **Drop React and Vite.** Plain `index.html` + modular CSS + vanilla JS files. No bundler, no `node_modules` at runtime. Tauri exposes its API via `app.withGlobalTauri: true` (`window.__TAURI__`).
2. **No Node dependency at all if possible.** Build and run with plain `cargo` from `src-tauri/`. Release builds enable `tauri/custom-protocol` through a Cargo feature so the frontend is embedded. Fallback if this misbehaves: the npm `@tauri-apps/cli` as the single dev dependency. Packaging into `.deb`/AppImage needs the CLI and is out of scope for the scaffold.
3. **Config location:** `config.json` in the project root. `run.sh`/`run.bat` change into the project folder before launching, and Rust resolves the file relative to the working directory. Trade-off: an installed copy in `/usr/bin` could not write there; that is a later concern, not a scaffold one.
4. **Oversized config (> 5 MB):** not loaded, defaults used, the file is **not** overwritten (so nothing is destroyed), and the UI shows a warning. Saving stays disabled for that session.
5. **DEBUG_MODE** `const` in Rust, `false` by default. When `true`: log config loads, saves, size rejections, failures and window events to `debug.log` in the project root. When `false`: no log file, minimal console output.
6. **Theme persistence moves to the config file.** Write a small `theme.js` for this app rather than editing the template's `theme-switcher.js` (keeps the template untouched and reusable).
7. **Copy only the Bootstrap assets used**, not the whole template (fonts are the bulk).

---

## Proposed layout (root kept spartan)

```
/opt/templates/rust
  AGENTS.md
  run.sh
  run.bat
  .gitignore            (target/, config.json, debug.log)
  config.json           (created at first run)
  docs/PLAN.md
  ui/
    index.html
    css/                (index.css, variables.css, window.css, copied modules, bootstrap*.css)
    js/                 (bootstrap.bundle.min.js, window.js, theme.js, app.js)
    fonts/              (only the families the themes need)
  src-tauri/
    Cargo.toml
    build.rs
    tauri.conf.json
    capabilities/default.json
    icons/
    src/
      main.rs           (entry only)
      lib.rs            (builder, plugin and command registration)
      config.rs         (load, save, defaults, 5 MB guard)
      commands.rs       (example command + config commands)
```

---

## Phases

### Phase 1: Skeleton window
- Create `src-tauri/` (Tauri 2, `tauri` + `serde` + `serde_json` only; no opener plugin).
- `tauri.conf.json`: 640x480, `decorations: false`, `transparent: true`, `shadow: false`, `resizable: true`, `center: true`, sensible `minWidth`/`minHeight` (e.g. 400x300).
- Copy the chosen Bootstrap CSS, JS, icons and fonts into `ui/`.
- `index.html` with one rounded outer frame on a transparent `body`.
- `run.sh` (build if needed, start picom if absent, launch) and `run.bat` (build, launch).
- **Test:** window opens at 640x480 with rounded transparent corners over a light and a dark background. Record the release binary size.

### Phase 2: Window chrome
- Custom title bar with `data-tauri-drag-region` (left-click hold to drag).
- Minimise, maximise/restore, close buttons using Bootstrap Icons, top right.
- Capabilities: `allow-start-dragging`, `allow-minimize`, `allow-toggle-maximize`, `allow-close`, `allow-start-resize-dragging`.
- Resizing a frameless window on Linux: test native edge resizing first. If it does not work, add thin invisible edge and corner handles that call `startResizeDragging(direction)`.
- Drop the rounded corners while maximised.
- **Test:** drag, minimise, maximise, restore, close, resize from every edge and corner, on LXQt.

### Phase 3: Rust commands and config file
- `config.rs`: struct with `theme`, `width`, `height` (+ serde defaults so missing keys never break loading). Create the file if missing. Reject files over 5 MB (decision 4). Malformed JSON falls back to defaults without overwriting.
- `commands.rs`: `get_config`, `save_config`, and one plain example command (e.g. `app_info` returning name, version and config path) shown on screen.
- `DEBUG_MODE` and `debug.log` behaviour (decision 5).
- **Test:** delete config, run, file appears. Corrupt it, run, defaults used and file untouched. Pad it past 5 MB, run, warning shown and file untouched.

### Phase 4: Runtime themes and saved window size
- Theme picker (Bootstrap dropdown) listing the template themes. Applies `data-theme` instantly, saves via `save_config`.
- Apply saved theme before first paint to avoid a flash of the default theme.
- Restore saved size on start. Save size on close and on a debounced resize. Do not save the maximised size.
- **Test:** change theme and size, close, reopen: both restored.

### Phase 5: Size, demo content, documentation
- Release profile for size: `opt-level = "s"`, `lto = true`, `codegen-units = 1`, `strip = true`, `panic = "abort"`. Measure binary and total folder size against the 50 MB target.
- Optional demo: drag-and-drop reorderable cards or accordion using the template's `sortable.js` (answers the open question in AGENTS.md). Its order currently saves to `localStorage`; either accept that or route it into `config.json` (decide at the time).
- Tray icon (Tauri built-in, option A, agreed 12 November 2026; always visible, created at startup regardless of window state): icon plus context menu (Show/Hide first, `tray_items` from config, Quit last); `start_minimised` in `config.json`, default false while debugging; on Windows a double-click toggles the window; Linux has no tray click events, see AGENTS.md. Reuse `src-tauri/icons/32x32.png`.
- Write `README.md`: what the project is, and how to install Rust, Tauri prerequisites and build tools on Linux and Windows, then build, run and copy into a new project. Location to confirm: project root (convention) or `docs/` (per the global rule).
- Write `requirements.txt`: Rust has no pip equivalent (dependencies live in `Cargo.toml` and `Cargo.lock`), so this file lists the system prerequisites with minimum versions (Rust, WebKitGTK 4.1 and GTK 3 dev packages, a C toolchain, WebView2 and MSVC Build Tools on Windows).
- Write `docs/usage.md` (how to copy into a new project, rename the app identifier, change themes).
- Update `AGENTS.md` with successes, failures and platform notes.

---

## Platform limitations and dependencies

- **Transparency and rounded corners on Linux/X11 require a running compositor.** Without picom the corners render black. `run.sh` starts picom if it is not running. Wayland compositors handle this themselves.
- **WebKitGTK 4.1** is the Linux runtime (already installed). End users on other distros need `libwebkit2gtk-4.1-0`.
- **Windows (`run.bat`)** uses WebView2 (preinstalled on Windows 10/11). Untested here; Kim has no Windows machine in this setup.
- **Frameless resize on Linux** may need the JS edge handles from Phase 2.
- **No new system packages** are needed for Linux, assuming the plain-cargo route in decision 2 works.
