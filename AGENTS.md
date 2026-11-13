## Rust Boiler plate Scaffold

User: Kim.
Version: 1.0
Date: 12 November 2026
Project folder: /opt/templates/rust

Create a lean, reusable Rust and Tauri desktop application scaffold for Linux.
It should be application-agnostic and suitable for copying into future projects.
Please don't hesitate to add successes, failures, suggestions, concerns, updates, tweaks to AGENTS.md

---

## Requirements

- Use the existing Tauri application as the starting point.
- Use the existing Bootstrap theme and template, bundled locally.
- Keep HTML, CSS, JavaScript and Rust in separate, clearly named files.
- Create a window with rounded corners and transparency, without native window decorations.
- Keep the window resizable at runtime; test this on Linux.
- Provide a simple example of the interface calling a Rust command.
- Save and load a small configuration file for user preferences in the project folder, loaded at runtime.  The app should create the configuration file does not, if the configuration file is greater than 5MB do not load it.
- Keep dependencies and complexity to a minimum.
- Do not add SQLite or login functionality to the initial scaffold.
- We already have an example here which has rounded corners and transparency, "/home/kim/projects/water_tracker"
- We wish to implement bootstrap into the project from here "/opt/templates/bootstrap/assets" allowing us to change the theme dynamically at runtime. The selected theme & windows size should be saved in the configuration file. 
- Inspect the existing application before proposing changes.
- Present a short numbered plan and obtain approval before writing code.
- Work one step at a time and preserve existing files.
- Explain any platform limitations or additional dependencies.
- Keep the root folder spartan, try not to create dozens of nested sub folders.
- A run.sh and run.bat file to launch the application.
- Default window size should be something like 640 x 480
- Since the form/app does not have a windows title bar, we need to be able to left click and hold to drag it around.
- Use bootstrap icons if possible, in the top right we probably want a stylized bootstrap minimize, maximize and close icon.
- Use /docs for any markdown documents.
- With each milestone and version we will aspire to backup the project source code in /backup in a compressed archive.

---

## Project Size

- Rust applications can get quite large we aspire to keep this project small, the distributed application should be under 50MB if possible.

---

## Phase 1

- Create the application, copy any assets we need from /opt/templates/bootstrap/assets

--- 

## Questions

- Perhaps in our app, we can use our drag and droppable - reordering bootstrap accordion menus or cards?

---

## Plan

- Phased plan and key decisions: `docs/PLAN.md` (written 12 November 2026, awaiting approval).

---

## Notes, successes and failures

### 12 November 2026: compiler permission failure (resolved)
- Symptom: every `cargo build` failed instantly with `error: could not exec the linker cc ... Permission denied (os error 13)`.
- Cause: `/usr/bin/cc -> /usr/bin/gcc -> gcc-11 -> x86_64-linux-gnu-gcc-11`, which is `root:tools 750`. Rust needs `cc` as its linker driver, and user `kim` was not in the `tools` group. This is also why water_tracker has no `target/` folder.
- Fix: `kim` was added to the `tools` group. Group membership only applies to new login sessions; until you log out and back in, build inside `sg tools -c "cargo build --release"`.
- Check: `id -nG` must list `tools` (or `cc --version` must succeed) before building.
- Not used: pointing cargo at the world-executable `gcc-12` would have bypassed a deliberate restriction.

### 12 November 2026: GCC restriction lifted
- An administrator authorised and ran `sudo chmod o+rx /usr/bin/x86_64-linux-gnu-gcc-11` to lift the `tools` group restriction on the GCC 11 binary.
- Verified: the binary now has mode `755`, both `cc` and `gcc` resolve to it, and `cc --version` succeeds. The separate C++ compiler was not changed. Running Guardian's `restrict-tools.sh` again can reapply the restriction.

### 12 November 2026: Phase 1 complete
- Plain `cargo build --release` works with no Tauri CLI and no Node. A Cargo feature `custom-protocol` (on by default) embeds `ui/` into the binary. First build 1m 54s on the i7-6700 at `nice 10`, `-j 6`.
- Release binary: 11.4 MB before any size tuning (Phase 5 adds the size profile).
- Real per-pixel transparency works with Tauri 2.11 / WebKitGTK 2.50 / picom (glx backend). Corners checked zoomed over black and white backgrounds: clean anti-aliasing, no halo. The water_tracker workaround (opaque window clipped by picom matched on window title) is not needed.
- Window reports `WM_CLASS = "rust-scaffold", "Rust-scaffold"`, 640x480.
- Aero Blue and Slate checked by screenshot. The other 12 theme gradients are written but not yet viewed (Phase 4).
- Gotcha: `ui/` is embedded at compile time, so any HTML, CSS or JS change needs a rebuild (incremental, about 20 s). `run.sh` always runs `cargo build`, which is a no-op when nothing changed.
- Without a compositor on X11 the transparent corners render black. `run.sh` starts picom if it is not running; picom keeps running after the app closes.

### 12 November 2026: Phase 2 complete
- Files: `ui/js/window.js` (controls and `maximized` class), title bar markup in `ui/index.html`, control styles at the end of `ui/css/window.css`, window permissions in `src-tauri/capabilities/default.json`.
- Tested on LXQt/X11 with xdotool (real mouse events): drag from the title bar moves the window; native resizing works from the right edge, left edge, top edge and bottom-right corner with no JS edge handles needed (Tauri/tao handles undecorated resizing itself); maximise, restore (icon swaps, corners square off while maximised), minimise (`_NET_WM_STATE_HIDDEN`) and close (process exits) all work.
- Not tested: top-left, top-right, bottom-left corners and bottom edge individually (same mechanism as the tested ones); double-click on the title bar to maximise; Windows.
- Gotcha for testing: xdotool needs `mousemove_relative -- -10 0` for negative offsets, and geometry must be read about a second after each action.
- Capabilities used: allow-start-dragging, allow-start-resize-dragging, allow-minimize, allow-toggle-maximize, allow-is-maximized, allow-close.

### 12 November 2026: Phase 3 complete
- Files: `src-tauri/src/config.rs` (load, save, validation, 5 MB guard, 6 unit tests), `src-tauri/src/commands.rs` (`app_info`, `get_config`, `save_config`), `ui/js/app.js` (example Rust call and warning banner).
- Behaviour, all verified against the real file: missing file is created with defaults; corrupt JSON loads defaults and the file is byte-identical afterwards; a file over the limit (5 MiB, 5,242,880 bytes) is not read and is untouched; a file exactly at the limit is still read. Partial files fill missing keys with defaults.
- Design decision: saving is disabled for the session whenever the load status is `too_large`, `invalid` or `unreadable`, so a file we could not read is never overwritten. The UI shows a warning banner for those states.
- Saves are atomic (write `config.json.tmp`, then rename). Theme must be a lowercase slug of at most 40 characters; width and height are clamped to 200..10000.
- `DEBUG_MODE` (const in `lib.rs`, false) verified both ways: true writes `debug.log` ("app starting", "config: ..."); false creates no log file. Timestamps are Unix seconds (no chrono dependency).
- "Ask Rust" button verified: returns name, version and the absolute config path.
- Not yet used: `save_config` has no UI caller until Phase 4. Not tested: a read-only project folder.
- Suggestion: the warning alert is legible but slightly muddy on Aero Blue (`--warning-soft` over a gradient); review alert colours in Phase 4 when all themes are viewed.
- Building and testing need `sg tools -c "cargo ..."` until the next login (see the compiler note above).

### 12 November 2026: Phase 4 complete
- Files: `ui/js/theme.js` (theme list, `data-theme` switching), `ui/js/app.js` (load config, apply theme, save theme and size, show window), theme picker and styles in the title bar (`ui/index.html`, `ui/css/window.css`), restore-size code in `src-tauri/src/lib.rs`.
- Flash-free start: the window is created hidden (`visible: false`) and `app.js` shows it after applying the saved theme. If `app.js` fails before `show()`, the window never appears; the `.then` after the catch covers load errors, but a syntax error in a script would still leave it hidden.
- All 14 themes viewed by screenshot (loaded from `config.json`): gradients render, dark themes keep readable contrast. Theme list: aero-blue, baby-pink, materia, sketchy, slate, dark-mode, lilac, regal, racing-green, orange-black, regal-blue, rose, vapor, quartz.
- Verified persistence: theme chosen from the menu is written to `config.json`; a resize is saved after a 500 ms debounce; maximising does not overwrite the saved size; relaunch restores both.
- Failure found and fixed: the theme picker inside the card was clipped by the card's `overflow: hidden` (only 2 of 14 themes reachable). Moved into the title bar. The template's dropdown is translucent glass in Aero Blue, Baby Pink and Quartz and was unreadable over text, so `window.css` gives the menu a solid themed background. Re-checked in Baby Pink and Quartz; Aero Blue was not re-viewed after that fix (same rule, white background).
- Failure found and fixed: Openbox ignores `center()` on a window that is created hidden and shown later (it appeared at x=640 for an 860 px window). The position is now computed in Rust from the primary monitor and set explicitly. Verified at 860x600 and 640x480. The window position itself is not saved, only the size.
- Warning banner colour on Aero Blue still looks slightly muddy (`--warning-soft` over the gradient); not changed.
- Not tested: multi-monitor placement, HiDPI scale factors other than 1, the window being closed by the window manager (Alt+F4) rather than the button (saving is debounced on resize, so size is already stored), Windows.
- Release binary: 11.5 MB.

### 12 November 2026: Phase 5, step 1 complete (config schema), and a perf finding
- `config.json` now also holds `show_title` (bool, default true), `order` (list id -> item ids) and `collapsed` (ids of collapsed accordion panels). Old files without these keys still load. Ids must be lowercase slugs of at most 40 characters; limits: 20 lists, 50 items per list, 100 collapsed ids. 9 unit tests pass.
- Drag and resize latency (reported by Kim): measured with scripted xdotool drags. Dragging costs little in the app or WebKit (about 15 ticks) but picom and Xorg each use about a quarter of a core compositing the transparent window; `WEBKIT_DISABLE_COMPOSITING_MODE=1` cut picom's share to about a quarter of that. Resizing keeps the WebKit web process near one full core. Removing transparency, box-shadows or the gradient did NOT change that figure, and none of `WEBKIT_DISABLE_DMABUF_RENDERER`, `WEBKIT_DISABLE_COMPOSITING_MODE`, `LIBGL_ALWAYS_SOFTWARE` or `GSK_RENDERER=cairo` helped. The cause is not identified. A later benchmark was inconclusive (some runs showed no resize at all) and Kim chose to stop the investigation. Treat as a known limitation: resizing feels less snappy than a native window.
- Lesson: an interrupted test run left a modified `tauri.conf.json` (blank-page test) behind. After any aborted experiment, diff the config files against a backup before continuing.

### 12 November 2026: Phase 5, step 2 complete (title bar toggle, deeper bevel) and tray findings
- Correction of terminology: the toggle hides the whole title bar (name, theme picker and window buttons), not just the title text. Config key renamed `show_title` to `show_titlebar` (default true; old files simply ignore the old key). When hidden: content fills the rounded window, a faint reveal button sits in the top right corner (`#btn-title-reveal`), and the content background stays draggable (`data-tauri-drag-region` on `<main>`). Cards and other child elements do not drag, only the empty padding does. With the bar hidden the only ways to close the window are the reveal button, then the close button, or Alt+F4; the tray menu will add another.
- Also added: `"rounded_corners": false` in `config.json` gives a square window (class `square-corners` on `<html>`, sets `--window-radius: 0px`).
- Verified by real mouse events: toggle writes the config, the state survives a relaunch, reveal button restores the bar, drag by background moves the window, square corners render. 9 unit tests pass.
- Bevel: the window frame now has a sharp 1 px edge plus a softer 2 px inner ring (`--window-highlight-soft` / `--window-lowlight-soft` in `variables.css`, set per light and dark group). Clear on light themes; on dark themes the shaded lower edge is hard to see by nature. Only Aero Blue and Slate were checked at zoom.
- Test pitfall: my first run waited only 4 s for the window and found nothing; the window normally appears after about 1 s, so that was a flaky wait, not an app failure. Poll for the window instead of sleeping.
- Known weakness: the window starts hidden and is shown by `app.js`. Any script error before `show()` leaves an invisible process. A fallback timer in Rust that shows the window after about 3 s would remove this risk (not yet done).
- Reveal button is faint (opacity 0.5) on pale themes; consider raising it if it is hard to find.

### Tray icon: findings and decision (12 November 2026)
- Tauri 2 supports tray icons (Cargo feature `tray-icon` on the `tauri` crate), and the right-click menu works on Linux. The runtime library `libayatana-appindicator3` is already installed.
- PROBLEM: on Linux, Tauri does not emit tray click or double-click events (documented in `tauri-2.11.5/src/tray/mod.rs`: "Linux: Unsupported. The event is not emitted even though the icon is shown"). They work on Windows and macOS. The PyQt5 project `/home/kim/projects/tray` gets its double-click toggle because LXQt's StatusNotifier host sends two `Trigger` events for a double-click and its code recognises the pair; the appindicator layer used by Tauri never delivers those events to the app.
- Also on Linux: once a tray menu is set it cannot be removed or replaced, only its contents changed; and the icon is sometimes invisible unless a menu is set. Use PNG icons (the tray project found SVG-only icons invisible in LXQt).
- Decision (Kim, 12 November 2026): Option A. Use Tauri's built-in tray. Menu with Show/Hide toggle first, user-defined items from `tray_items` in `config.json` (fixed action set: show, hide, toggle, quit), Quit last. Double-click toggles the window on Windows only; on Linux the menu does the job.
- Rejected for now, revisit only if the menu-only toggle proves inadequate on Linux: Option B, the `ksni` crate (pure-Rust StatusNotifierItem) which would receive the LXQt double-trigger. Costs extra dependencies (D-Bus stack), a few hundred KB, and a Linux-only code path.
- Planned config: `start_minimised` (bool). Must default to false while debugging; when true the window starts hidden in the tray. If no tray is available the window must still open.

### Compiler restriction: where it comes from (12 November 2026, read-only investigation)
- It is NOT polkit. No polkit rule mentions it. It is plain Unix ownership and mode: `/usr/bin/x86_64-linux-gnu-gcc-11`, `/usr/bin/x86_64-linux-gnu-g++-11` and `/usr/bin/node` are all `root:tools 750`. `gcc-12` is `root:root 755`, which is why `cc` (via gcc-11) failed but gcc-12 would not.
- No ACLs and no immutable flags. No `dpkg-statoverride` entry, so a package upgrade of gcc-11 or nodejs would probably reset the mode to 755 and silently lift the restriction (and vice versa nothing re-applies it).
- None of the scripts in `/usr/local/sbin` or the systemd units in `/etc/systemd/system` that I searched contain a `chgrp` or `chmod` for these files, and `nursery-lockdown` only lists node, python3 and similar as a commented-out optional block. So whoever applied it did so by hand or by a mechanism I did not find. I did not look inside root-only files.
- Group membership only takes effect in new login sessions. Shells started before Kim was added (including Claude Code) still lack `tools`; use `sg tools -c "..."` there. Plain `cc --version` in a fresh terminal after login is the check.

### Water tracker report (12 November 2026)
- Kim reported the water tracker no longer works. From this session it launches, shows its window (360x540), the button takes the count from 0 to 500 ml after two clicks, and dragging moves it. Nothing under `/home/kim/projects/water_tracker` has changed since 5 October and `~/.config/picom/picom.conf` since 29 August. The tools group does not affect it: it runs a prebuilt binary and never invokes the compiler.
- Not reproduced. One oddity seen: the "I drank a glass" label renders black although the shipped CSS says `color:#fff`; unknown whether this is new. A pink tint covered the whole screen in one screenshot and was gone in the next, so it was a screen overlay, not the app.
- I started picom in this session (`picom --backend glx`, reads the default `~/.config/picom/picom.conf`). The water tracker's `start.sh` starts picom only if none is running, so it will use that one; if picom dies the script restarts it.

### 12 November 2026: Phase 5, step 3 complete (pages)
- `ui/index.html` now has two Bootstrap tab pages inside `<main>`: Dashboard (the Rust command example card) and Tables (placeholder until step 6). Tabs use the template's `nav-tabs-custom` / `tab-content-custom` styles, compacted in `window.css`. Tabs live in the content area, not the title bar, so they stay visible when the title bar is hidden. No fade animation on pane switches.
- `backdrop-filter` is switched off on the tab content panel (the template blurs it on Aero Blue); blur forces a repaint on every resize and drag/resize latency is already a known complaint.
- Verified with real clicks: switching to Tables and back, the Ask Rust call still works on the Dashboard tab, Aero Blue and Slate render correctly. Active tab keeps the template's fairly strong drop shadow; left as is.
- Not done: the active tab is not saved in `config.json` (not requested).
- Water tracker: `/home/kim/projects/water_tracker/start.sh` fails because it hard-codes `BIN="/home/kim/water_tracker/bin/water-tracker"`, a folder that no longer exists (the project moved to `/home/kim/projects/`). Running `bin/water-tracker` directly works. `start-dev.sh` has the same stale path in `APP_DIR`. Not changed, pending Kim's decision.
- Tray requirement added by Kim (12 November 2026): the tray icon must show at all times, whether the window is visible, minimised or hidden. Create it at startup, independent of `start_minimised`, and never remove it when the window hides. Not built yet; nothing tray-related exists in the app so far.

### 12 November 2026: run.sh fix, and Phase 5 step 4 complete (drag-to-reorder cards)
- `run.sh` failed for Kim because her shell has not picked up the `tools` group yet and a UI change forces a relink, which needs `cc`. Fixed: `run.sh` now detects "cc not runnable but the account is in group tools" and builds through `sg tools`. If a build fails and a previous binary exists it launches that and says so (this fallback path was not exercised in testing; only the successful rebuild path was). Verified from a plain shell: rebuilt in about 24 s and the window appeared.
- New `ui/js/sortable.js`: pointer-event drag reordering (mouse, pen, touch) plus keyboard Up, Down, Home, End on the grip, with an aria-live announcement. It stores nothing itself; `ui/js/app.js` saves the order into `config.json` (`order["dashboard-cards"]`). Unknown ids in a saved order are ignored and missing ones keep their default place. "Reset order" button restores the default and removes the saved key.
- Dashboard now holds three sortable cards: Rust command example, Saved settings (shows live theme and window size), Reordering (help and reset). Styling reuses the template's `sort-handle` / `sortable-list-cards` classes.
- Bug found and fixed: listening for `pointermove` and `pointerup` on the grip failed after the first swap, because moving an element in the DOM releases pointer capture on it. Only one swap happened and the order was never saved. Fix: listen on `document`, filtered by pointer id. Do not go back to `setPointerCapture` on the handle.
- Verified with real mouse and keyboard events: drag of the third card to the top saved the order; the order was restored after relaunch; keyboard Down moved a card and saved; Reset restored the default and cleared the saved key; a saved order with an unknown id and missing ids loaded sanely.
- Not tested: touch input (the pointer-event code path supports it, `touch-action: none` is set on the grip, but no touch device was available), auto-scroll while dragging near the edge of a scrolled list, dragging while the window is maximised.
- Test pitfall: a test that moves the real mouse is ruined if Kim touches the mouse during the run (one run was lost that way). Ask Kim to keep hands off while xdotool tests run, and always retest before blaming the code.

### 12 November 2026: tray icon complete (Tauri built-in, option A)
- Files: `src-tauri/src/tray.rs` (menu, actions, icon), `Cargo.toml` (`tauri` feature `tray-icon`), `config.rs` (`start_minimised`, `tray_items`), `commands.rs` (`get_config` returns `start_hidden`), `ui/js/app.js` (skips `show()` when `start_hidden`).
- The tray icon is created at startup and kept for the life of the app, whether the window is visible, minimised or hidden (Kim's requirement). Verified on LXQt: the item registers with `org.kde.StatusNotifierWatcher` on D-Bus and appears in the panel (grey gear, the Tauri template icon from `icons/`; replace `icons/32x32.png` and friends for a custom icon). The tray stayed registered after minimise, hide, show and restore.
- Menu: "Show / Hide" first, then the custom entries from `tray_items`, then "Quit". "Show / Hide" minimises a visible window and restores a minimised or hidden one. Actions allowed in `tray_items`: `show` (unminimise, show, focus), `hide` (hide completely, no taskbar entry), `toggle`, `quit`. Anything else is rejected when the config is validated, so `config.json` can never run commands. Limits: 10 items, labels up to 40 characters, no control characters.
- `start_minimised` (default false, as agreed while debugging): the window stays hidden at launch and the tray brings it back. It is ignored if the tray could not be created, so the app can never become unreachable.
- Verified with real clicks on the real panel: right-click opens the menu; Show/Hide minimised then restored the window; with `start_minimised` and two custom items the window did not appear at launch, "Show window" showed it, "Hide to tray" hid it (process and tray remained), and "Quit" ended the process and removed the tray icon. 11 unit tests pass (config round trips and rejection of bad tray items).
- Not tested: Windows (double-click toggle is coded via `TrayIconEvent::DoubleClick` and untested), the no-tray fallback (no way to remove the panel's tray host here), a custom icon, left-click behaviour (Tauri emits nothing on Linux, see the tray findings above), any tray host other than LXQt's.
- Closing the window with the close button still quits the app; "hide to tray on close" was not requested.
- Linux runtime requirement: `libayatana-appindicator3-1` (loaded at run time with libloading, so no dev package or sudo is needed to build). Put this in README.md and requirements.txt.
- Release binary is now 13.3 MB (was 11.5 MB before the tray).

### 12 November 2026: tab fixes and Phase 5 step 5 complete (accordions)
- Tab fixes (Kim's report, with screenshot): removed the template's heavy drop shadow under the selected tab (all tab states now carry only the bevel highlight `inset 0 1px 0 var(--bevel-top-hover)`), and removed the bottom border of the tab row. That border ran the full width, so on Baby Pink (content panel rounded at the top right) it stuck out past the corner, and on Sketchy it doubled up with the panel's own top border. The panel's top border is now the baseline. Checked by screenshot on Baby Pink, Sketchy, Aero Blue and Slate; all clean. Other themes were not checked.
- New "Panels" tab (Dashboard, Panels, Tables) holding a Bootstrap accordion of three panels: Appearance, Window, Tray icon. All start expanded. Each has a grip and is sortable (`order["panels-accordion"]`, same `sortable.js` as the cards). No `data-bs-parent`, so several panels can be open at once.
- Collapsed state is saved in `config.json` `collapsed` (ids of collapsed panels). Collapsed panels are applied before the window is shown, so there is no animation or flash. Toggling adds or removes only the panel's own id, so ids from other sources are kept (verified with a `ghost` id).
- The height animation of collapsing is switched off (`.accordion-collapse.collapsing { transition: none }`) because it repaints every frame.
- Verified with real clicks and drags: collapse saved `["window"]`; relaunch showed Window collapsed; expanding cleared it; dragging the third panel to the top saved `["tray","appearance","window"]`; an unknown saved id survived a toggle. Not tested: touch, keyboard toggling of a panel, other themes' accordion styling.
- The active tab is not remembered between launches; the app always opens on Dashboard.
- Kim's note: performance is "abysmal" and will be revisited after all the controls exist. Pending item: investigate drag/resize latency (see the earlier measurements; next experiment is a blank page to see whether the cost is fixed or caused by the page).

### KNOWN ISSUE: visual redraw latency (to be addressed in a later version)
Reported by Kim on 12 November 2026, repeatedly: dragging the window, resizing it, scrolling, switching tabs and drag-and-drop reordering feel slow and low-FPS when Kim runs the app by hand with `/opt/templates/rust/run.sh`. Kim also noticed that when Claude drove the same binary with xdotool there was "almost no latency". Not fixed; documented here so a later version can address it.

What is known:
- The binary is a release build, startup is about 0.6 to 1 s and idle CPU is about zero. This is not a "debug build" or start-up problem.
- Scripted drags and resizes (measured with xdotool): dragging costs little in the app and WebKit, but picom and Xorg each use about a quarter of a core compositing the transparent window. During a resize the WebKit web process (`WebKitWebProcess`) sits near one full core. Removing window transparency, all box-shadows, or the gradient made no measurable difference. `WEBKIT_DISABLE_DMABUF_RENDERER`, `WEBKIT_DISABLE_COMPOSITING_MODE`, `LIBGL_ALWAYS_SOFTWARE` and `GSK_RENDERER=cairo` did not help the resize figure. `WEBKIT_DISABLE_COMPOSITING_MODE=1` did cut picom's share during drags to about a quarter.
- Environment: the relevant variables (GDK, GTK, WEBKIT, LIBGL, __GL, QT, XDG, DISPLAY and so on) are identical between Claude's shell and Kim's qterminal shell, so a stray environment setting is not the explanation for fast-when-Claude-drives.
- Hardware and state: Intel i7-6700 with `intel_pstate` in `active` mode, governor `powersave`, energy preference `balance_performance`; RTX 3060 whose clock swings between 210 MHz (P8) and 1777 MHz (P0) depending on what else is running (a video was playing in Firefox during some tests).

Leads, in order of cheapness (none confirmed):
1. Power-state ramp-up. xdotool sends a continuous stream of events every 20 to 30 ms, which may keep CPU and GPU clocks high; hand use is bursty and may leave them in a low state. Test: set the NVIDIA PowerMizer mode to "Prefer Maximum Performance" (`nvidia-settings -a "[gpu:0]/GpuPowerMizerMode=1"`, needs no root) and set the CPU energy preference to `performance`, then compare feel with `run.sh`. If that fixes it, the app is fine and the machine's power policy is the cause.
2. Synthetic versus physical input. xdotool events may bypass the cursor and pointer-motion handling a real mouse triggers (high-rate motion events, compositor cursor updates). Compare with a real mouse while watching `WebKitWebProcess` CPU.
3. Blank-page control. Replace `ui/` with an empty transparent page (set `frontendDist` to a folder with a tiny `index.html` and `visible: true`) and resize it by hand. If a blank page also feels slow, the cost is WebKitGTK + picom + the NVIDIA driver and not our page; if it is fast, bisect the CSS.
4. picom settings. It was started without `--vsync` and with the default config; try `--vsync`, `--backend xrender`, or running without picom to see the difference.
5. Heavy CSS that remains: many layered `box-shadow` insets on the window and cards, translucent `rgba` surfaces over a gradient on several themes, `backdrop-filter` on Baby Pink and Quartz (via the template's `bootswatch.css`; already disabled for the tab panel only).

### 12 November 2026: Phase 5 step 6 complete (Tables page)
- The Tables tab now holds three mock-data cards, each a Bootstrap table in the template's `table-custom` style: Team (name, role, status badge, progress bar, updated; striped and hover), Invoices (right-aligned amounts with tabular figures, status badge; hover), Services (uptime, latency, state icon; striped). Fictional names and numbers only. `tables.css` was copied from the template into `ui/css/` and imported in `index.css`.
- Status badges (`badge-success`, `-warning`, `-danger`, `-info`, `-neutral`) and state icons are defined in `window.css` from each theme's soft semantic tokens and AA-safe text colours. Progress bars use the template's `progress-custom` and Bootstrap's `w-25`, `w-50`, `w-75`, `w-100` classes for the fill, because inline `style=""` is not allowed (and the CSP would block it). Only those four fill values exist; for other values add a utility class in `window.css`.
- Each table has a visually hidden `<caption>` and `scope` attributes on headers for screen readers.
- Checked by screenshot on Slate, Aero Blue, Baby Pink and Sketchy (full page, 640 px wide window): readable, badges and bars themed correctly in each. The other ten themes were not viewed on this page.
- Not tested: horizontal scrolling of the tables in a window narrower than their content (`table-responsive` and `white-space: nowrap` are set, the 640 px default fits without scrolling), touch scrolling, and clicking column headers (not sortable; deliberately plain).
- Test pitfall: a screenshot run captured Kim's file manager in front of the app because Kim was using the desktop; raise the window with `xdotool windowraise` before capturing.

### 12 November 2026: Phase 5 complete (release settings and documentation)
- Release profile added to `src-tauri/Cargo.toml`: `opt-level = "s"`, `lto = true`, `codegen-units = 1`, `strip = true`, `panic = "abort"`.
- Result: binary 13,348,192 bytes (13.3 MB) down to 4,341,176 bytes (4.3 MB). The distributable is just that one file (the UI is embedded), far under the 50 MB target. `src-tauri/target` is about 1.3 GB of build files and is not part of the distribution.
- Cost: full rebuild 129 s, incremental rebuild after a UI edit 56 s (was about 25 s without LTO). This slows the edit-and-run loop that `run.sh` uses. If it becomes annoying, add a second profile, for example `[profile.fast] inherits = "release"`, `lto = false`, `codegen-units = 16`, and let `run.sh` build with `--profile fast` during development; ship with `release`. Not done, not requested.
- Not measured: whether `opt-level = "s"` and `panic = "abort"` change runtime feel. The redraw latency lives mostly in WebKitGTK and picom, not in the Rust code, but it was not re-measured after this change.
- Verified after the change: 11 unit tests pass; the binary starts, creates `config.json`, shows the window, registers the tray icon, answers "Ask Rust" (name, version, config path) and exits cleanly with the tray's Quit, which removes the icon. Memory is about 112 MB. Not re-verified after the profile change: drag, resize, themes, accordions and the tables page (code unchanged, only compiler settings).
- Written: `README.md` (project root, image `ui/img/app.jpg` centered at the top; what it is, features, install steps for Linux and Windows, build and run, settings, layout, template use, known limitations), `requirements.txt` (system requirements with minimum and tested versions; Rust has no pip equivalent), `docs/usage.md` (run, build, config keys and rules, window controls, tray, how to add a Rust command, tab, sortable list, accordion panel and theme, template renaming checklist, debug mode, troubleshooting, limitations). The documents contain no em dashes (public-facing rule).
- `ui/img/app.jpg` is concept art (1672x941, 144 KB), not a screenshot of the app; the README alt text says so. It is inside `ui/`, so it is embedded in the binary even though the app never displays it. Moving it out of `ui/` would save about 144 KB.
- Windows: `run.bat`, the install steps and the DoubleClick tray handler are written from the standard Tauri 2 documentation and have never been run. Treat as untested.
- Leftover work, in order: (1) the redraw latency investigation (see the KNOWN ISSUE section; cheapest first test is NVIDIA PowerMizer set to maximum performance), (2) the optional `fast` build profile above, (3) view the remaining ten themes on the Tables, Panels and Dashboard pages, (4) a fallback timer in Rust that shows the window if `app.js` fails, (5) test on Windows.

### 12 November 2026: run.sh now launches the app detached (Kim's request)
- Kim observed that the app felt snappy when Claude started it and slow when she ran `./run.sh`. Claude had been starting the finished binary directly with `nohup src-tauri/target/release/rust-scaffold >/dev/null 2>&1 &` (no terminal attached, no build step); `run.sh` ran it with `exec` in the foreground attached to her terminal after a build. The scheduler view was identical (same nice level, scheduling class, CPU affinity, cgroup and no limits for both shells), and the relevant environment variables matched.
- Change: `run.sh` now ends with `nohup "$BIN" >/dev/null 2>&1 &` followed by `disown`, so the prompt returns immediately and closing the terminal does not close the app. Verified: it returns in about 0.3 s, the window appears, and the process has stdin, stdout and stderr on `/dev/null`. Nothing is printed by the app in this mode, so startup errors are invisible; run the binary directly in a terminal if the window fails to appear.
- UNCONFIRMED: whether this fixes the slowness. Kim has not yet reported how it feels. If it is still slow, the terminal attachment was not the cause and the remaining difference is real mouse input versus synthetic input. Untested ideas, cheapest first: a "hover storm" measurement (sweep the pointer at 200 Hz and watch the web process CPU), a flat-CSS variant with every `box-shadow` and `transition` removed (only the window frame's shadows were removed in earlier tests, the component shadows were not), and the four variant binaries Claude built in `/tmp/variants` (`try.sh baseline|opaque|fixed|both|blank`, all built but never tried by Kim).
- Measured while investigating, relevant to the latency: Aero Blue (with `backdrop-filter` blur) and Slate (without) cost the same in the web process, so the blur is not the cause. A tab switch costs about 0.15 to 0.2 s of web process CPU in either theme, and a resize about 260 to 290 ticks, with a 4.3 MB size-tuned binary.
- `run.bat` was not changed.

### KNOWN ISSUE UPDATE: redraw latency is much worse when the window is maximised (reported by Kim, 12 November 2026)
- Kim ran the app with the detached launcher (`run.sh` now starts it with `nohup ... >/dev/null 2>&1 &`). Result in her words: it "partly fixes the latency issues, it's certainly faster", but when the window is MAXIMISED the latency returns badly. Selecting a tab takes a few seconds when maximised, and buttons do not respond instantly.
- Kim's verdict: the app is not viable when maximised, and she is ready to call the project a failure. At the default 640x480 size, launched with the new `run.sh`, it is faster but still not described as snappy.
- Observations that fit the new report (not proven): cost grows with window area. 640x480 is about 307,000 pixels; a maximised 1920x1028 window is about 1,974,000 pixels (6.4 times more). The water tracker, which Kim says is snappy, is a fixed 360x540 window (about 194,000 pixels, 10 times fewer than maximised) and cannot be maximised. Earlier measurements also show the WebKit web process using close to one full core during resizes while Rust used almost nothing, and none of `LIBGL_ALWAYS_SOFTWARE`, `GSK_RENDERER=cairo`, `WEBKIT_DISABLE_DMABUF_RENDERER` or `WEBKIT_DISABLE_COMPOSITING_MODE` changed that CPU figure (Kim felt the last two as marginally better). Together this points to the page being rasterised on the CPU, so every repaint costs time in proportion to the pixels repainted, and a maximised window repaints about six times more than the default one. This is a hypothesis; it was not tested.
- Checked and ruled out (CPU measurements, 12 November 2026): `backdrop-filter` blur (Aero Blue with blur and Slate without cost the same), window transparency alone, the window frame's box-shadows, the background gradient, process priority, CPU affinity, cgroup limits and environment differences between Claude's shell and Kim's terminal.
- Not ruled out: the box-shadows on components (buttons, cards, tabs, accordion items, badges, table wrapper; only the window frame's shadows were removed in earlier tests), translucent `rgba` surfaces layered over the gradient, the sheer number of layered elements on the Dashboard, real mouse input being denser than the scripted input used in tests, and the transparent (ARGB) window itself at large sizes.
- Cheapest tests that would settle it, none run yet:
  1. Set `"width": 360, "height": 540` in `config.json` and run `./run.sh`. If it is snappy like the water tracker, window area is the cause.
  2. Maximise the `blank` variant (`/tmp/variants/try.sh blank`, same window settings, trivial page). If a blank page is also slow maximised, the cost is the window and WebKitGTK on this machine, not our page. If it is fast, our CSS is the cause and a flat-CSS variant (all `box-shadow`, `text-shadow`, `transition` removed) is the next step.
  3. Try the `opaque` variant maximised (`/tmp/variants/try.sh opaque`). The four variants (`baseline`, `opaque`, `fixed`, `both`, `blank`) were built and never tried by Kim; `/tmp` is cleared on reboot.
- Design options if the area cost cannot be removed: cap the window size with `maxWidth` and `maxHeight` in `tauri.conf.json` and drop the maximise button (the scaffold then suits small widget-style apps like the water tracker), or ship an opaque window (`transparent: false`, corners clipped by picom as the water tracker does).
- Status: unresolved. The functional side of the scaffold (config file, themes, tray, sortable lists, accordions, tables, tests, documentation) is complete and independent of this problem.

### 12 November 2026: project abandoned, cause found
- Kim maximised the `blank` variant (same transparent, frameless, resizable window, rounded clip and gradient, plain page with three tabs) and it was fast and responsive. The real app maximised is very slow. The cause is the page styling: the Bootstrap template was copied almost wholesale (full `bootstrap.min.css` and bundle, 22 of 32 template stylesheets, 435 KB of CSS) with its stacked bevel shadows, translucent surfaces, backdrop blur and transitions, all painted per pixel by WebKitGTK.
- The Aero Blue `backdrop-filter` rules were removed from `cards.css`, `accordion.css`, `alerts.css`, `containers.css`, `tabs.css` and `dialogs.css` beforehand (originals in `/tmp/css_backup_aero/`); that alone made no noticeable difference.
- Full write-up and lessons: `docs/why_this_project_failed.md`.

### 12 November 2026: Codex maximised UI responsiveness fix
- Before changes, archived the project source and settings as `backup/rust_template_pre_codex.zip` and verified the ZIP. Cargo build files were excluded.
- A release build with all shadows, blur and transitions disabled switched tabs in about 90 to 128 ms when maximised. A second diagnostic build isolated the effect groups: original styles took 592 to 1203 ms; shadows off alone took 94 to 96 ms; blur off alone took 603 to 1195 ms; motion off alone took 594 to 950 ms. This was measured by polling a one-pixel screenshot after `xdotool` tab clicks at 1920 x 1028. The polling floor was about 90 ms. All modes kept the full Bootstrap CSS and JavaScript, so their size was not the cause of the repeated repaint delay.
- Added `ui/css/performance.css` as the final import to disable shadows and backdrop filters. Removed six unused template module imports from `ui/css/index.css`; their files remain. The core Bootstrap CSS, JavaScript and Icons remain loaded. The production release binary compiled and measured 94 to 100 ms per tab switch in Slate, Aero Blue and Quartz. Kim watched the interaction and reported a huge visible speed difference.
- `run.sh` opened the rebuilt app; Rust command, tabs, accordion, tables and theme picker worked. Slate, Aero Blue, Baby Pink and Quartz were viewed. All 11 Rust tests passed. `config.json` was restored from the backup after testing.
- Full method, values and limits: `docs/performance_fix_2026-11-12.md`. Earlier failure notes above are historical. Drag and resize, every theme, and Windows have not been retested.

#### Visual effects guidance for projects copied from this scaffold
- The measured slowdown came from the page's `box-shadow` effects as a group. Wide cards and panels used bevel tokens that combined inset highlights with soft drop shadows; buttons, tabs, accordion parts and other controls added more. Disabling shadows alone reduced maximised tab response from 592 to 1203 ms to about 94 to 96 ms. Disabling blur or transitions alone did not produce that improvement in the tested Slate theme.
- `ui/css/performance.css` currently disables all `box-shadow`, `text-shadow` and `backdrop-filter` effects, including shadows supplied by Bootstrap. It uses `!important`, so adding a shadow to another component file will have no visible effect until this rule is deliberately adjusted. Borders and focus outlines remain; use colour and 1 px borders for depth before considering shadows.
- Small, simple shadows are **not proven impossible**. We tested all shadows on versus all shadows off, not one low-cost shadow at a time. For a future app, add one restrained shadow to one component type, maximise the window on the target machine, then compare tab changes, scrolling, drag and resize with the flat baseline using the normal launch path and a real mouse. Keep it only if responsiveness remains acceptable. Avoid large blurred shadows, stacked inset and outer shadows, and shadows on broad panels until they pass that check.

### 12 November 2026: standalone release build script
- Added executable `build.sh` at the project root. It builds with `cargo build --release --locked -j 4` at low priority, handles the existing `tools` compiler group with `sg` when needed, reports the binary path, and exits on failure without launching an older build. Tested from `/tmp`: the build completed and reported `src-tauri/target/release/rust-scaffold`.

### 12 November 2026: desktop benchmark baseline
- Added a separate toolkit at `src-tauri/benchmark/` with scripts, instructions and raw JSON results. It requires `build.sh` to succeed, hashes the release executable, runs it with a temporary config, and measures Dashboard-to-Panels visible pixel changes at 640 x 480 and maximised size on the normal LXQt/X11 desktop. It does not modify the app or project config.
- The first complete baseline (`results/20261112T155705Z_baseline_7ebc7813.json`) measured about 200 ms median normal and 210 ms maximised, 20 switches each, Slate theme. A same-binary repeat measured about 190 ms normal and 210 ms maximised. The comparison correctly calls both differences inconclusive. The screenshot polling floors were about 30 to 40 ms normal and 90 ms maximised; these are click-command-to-observed-pixel times, not precise physical input latency.
- `src-tauri/benchmark/README.md` gives commands, method, limits and extension guidance. Synthetic comparison checks flagged changed themes, large regressions and noisy results correctly. Two early attempts left partial results with no completed scenario: one tried to focus Tauri's still-hidden window, then one did not see the calibrated pixel within five seconds. Visible-window detection was fixed; the second failure was not isolated. Two subsequent complete runs passed. Manual responsiveness checks remain necessary after future changes.

#### Benchmark toolkit quick reference
- `src-tauri/benchmark/bench.py run --label NAME` uses `build.sh` to verify the release build, then measures 20 Dashboard/Panel tab switches at 640 x 480 and maximised size with a temporary config. It saves raw, dated JSON under `src-tauri/benchmark/results/` and leaves the project config untouched.
- `src-tauri/benchmark/bench.py compare BEFORE.json AFTER.json` reports median and p90 changes, flags incompatible conditions and calls small or noisy differences inconclusive. `tab_switch.py` holds the click and screenshot-poll scenario; `compare.py` holds the comparison rules. See `src-tauri/benchmark/README.md` for commands and limits. These times include click-command and screenshot overhead, so always check responsiveness by hand as well.

#### Theme brightness for future UI work
- Light themes with white or near-white content and window backgrounds: **Aero Blue, Baby Pink, Materia, Sketchy**. Their window gradients end in white, and Materia and Sketchy use white main surfaces. Check contrast and icon visibility on all four when adding controls or visual effects.
- Dark themes: **Slate, Dark Mode, Lilac, Regal, Racing Green, Orange + Black, Regal Blue, Rose, Vapor, Quartz**. Their backgrounds are dark, but several are coloured dark red, green, blue or purple rather than literally black or grey. Check effects and text contrast on representative coloured dark themes too.

### 12 November 2026: table search and sortable Cards tab
- Archived the pre-change source as `backup/rust_template_pre_search_cards.zip` and verified it. A fresh release tab-switch baseline was saved before UI edits at `src-tauri/benchmark/results/20261112T165545Z_pre_search_cards_7ebc7813.json`.
- Each mock table now has a large Bootstrap search input with a Bootstrap Icons magnifying glass. `ui/js/table_search.js` filters only that table's rows, case-insensitively across columns, and updates a match count and empty row. Search text is not saved. Styling is in `ui/css/tables.css`. Real desktop checks covered matching across two terms, no results, and independent Team, Invoices and Services searches.
- Added a Cards tab with five mock Bootstrap cards. The existing `ui/js/sortable.js` handles pointer and keyboard reordering; `ui/js/app.js` saves `order["showcase-cards"]`. Reset buttons now target one list each, including Dashboard and Panels. Keyboard and real mouse moves saved the Cards order, relaunch restored it, and Reset removed only the Cards key. Panel Reset left the Dashboard key untouched. Tests used a temporary config; the project `config.json` hash was unchanged.
- Rebuilt the release app, checked JavaScript syntax, and viewed the search and Cards pages in Slate and all four light themes. The post-change benchmark `src-tauri/benchmark/results/20261112T170707Z_after_search_cards_dcb02d7e.json` measured about 180 ms median normal and 210 ms maximised versus 170 ms and 220 ms before. Both differences are below the measurement threshold and **inconclusive**; no performance improvement or regression is claimed. Search latency, scrolling and dragging at maximised size were not separately timed.

### 12 November 2026: Kim's assessment of the Codex Sol work
- Kim reports that the UI now feels fast and responsive, including when maximised, and says she is very happy with the result. She considers Codex using the Sol model substantially more effective on this project than her earlier Claude Code sessions. This is Kim's assessment, not a comparative model benchmark.
- The concrete work behind that assessment: disabled the template's layered box shadows, text shadows and backdrop blur that made large WebKitGTK repaints costly; trimmed unused CSS imports; added the standalone `build.sh`; created the desktop benchmark toolkit and saved raw baseline and comparison runs; added independent search to all three mock tables; and added the sortable Cards tab with order saved in `config.json` and a scoped reset button.
- The old "KNOWN ISSUE: visual redraw latency" section above describes the earlier state. Kim's current hands-on report says the lag is resolved. Keep the low-cost paint rules in `ui/css/performance.css` when adding future controls. The screenshot-polling benchmark has a substantial measurement floor, so its recent small differences are inconclusive rather than proof of a speed change.

### 12 November 2026: offline Charts tab
- Backed up the source before changes as `backup/rust_template_pre_charts.zip`, verified with `unzip -tq`. The Charts tab uses the Bootstrap template's bundled SVG renderer and CSS copied into `ui/js/charts.js` and `ui/css/charts.css`. It loads no online chart assets and adds no Rust or JavaScript package dependency.
- Four fictional examples: line chart for sales and costs, stacked bars for sign-ups by source, doughnut for customer retention groups, and pie chart for customer segments. Each card has its own label search, select filter, visible count, empty state and Chart/Table switch. Pie and doughnut percentages are recalculated from visible groups. `ui/js/charts_page.js` holds sample data and filters; `ui/css/charts-page.css` holds layout and controls. Existing `ui/css/performance.css` still removes shadows and blur.
- The copied SVG renderer was extended with an in-place update method to reuse each chart's listeners and observer, stable color slots when filtering pie categories, and a full-circle case for a single remaining pie or doughnut category. The latter was found by a real desktop check: the unmodified template renderer showed an empty ring for one item.
- Release build succeeded. Real LXQt/X11 checks covered independent search, combined select and search, no-result recovery, table view and return to chart, single-slice doughnut, normal and maximised layout. Viewed the Charts tab in Slate and all four light themes (Aero Blue, Baby Pink, Materia, Sketchy). Other dark themes, touch input and Windows were not checked on this feature.
- Pre-change raw tab-switch run: `src-tauri/benchmark/results/20261112T172544Z_pre_charts_dcb02d7e.json`; post-change: `src-tauri/benchmark/results/20261112T174003Z_after_charts_b9f11ece.json`. Both had 20 switches per size, Slate on the same LXQt/X11 desktop. Medians were about 190 ms normal and 220 ms maximised in both. The comparison flags both as **inconclusive** because of the measurement floor and noise threshold. It measures Dashboard/Panel switching, not chart search or chart drawing, so do not infer a chart performance improvement from it.

### 13 November 2026: Options tab, phase 1 of docs/new_tabs.md (Claude)
- Design decision (Kim chose "Option B"): no control-type prefixes on config keys. `config.json` keeps plain JSON values; a separate `options.json` (project folder, next to `config.json`) describes each control. Reasons: prefixes would rename and orphan existing keys, cannot carry choices, limits, size, rows or read-only, and would tie stored data to a widget.
- Rust: `config.rs` gained `extras` (settings unknown to the struct, so saving never drops them; max 100 keys, keys `[a-z0-9_]` up to 40, values bool, number or text up to 4000 chars). New `options.rs` reads `options.json` on every `get_options` call (1 MiB limit, created from `src-tauri/options.default.json` when missing, invalid file never overwritten). Each entry is validated separately; bad ones are skipped with a reason shown in the UI. Built-in keys are limited to fitting controls; `order`, `collapsed`, `tray_items`, `extras` are refused.
- UI: `ui/js/options.js` (renderer, staged edits, Save/Revert, `revert` and `defaults` button actions), `ui/css/options.css`, width tokens in `variables.css`, `forms.css` and `lists.css` imported. `app.js` exposes `window.AppConfig.commit(changes)`: applies changes to the live config first, then saves, and rolls the keys back if Rust refuses. This avoids a resize save racing an Options save and writing old values. `theme.js` exposes `AppTheme.list()`.
- Controls: switch, check, radio, input (one line), text (multi-line, `rows`), number, range, list (list group), select, button. Full format in `docs/usage.md` ("Options tab").
- Tests: 19 Rust unit tests pass (8 new). Release app driven with real xdotool clicks in a temporary folder (project `config.json` hash unchanged): every control type edited and saved with the correct JSON type (number stays number, newline kept in text, read-only value not written); theme and square corners applied immediately on Save; Revert and Restore defaults worked; relaunch restored all values; adding a good and a bad entry to `options.json` while the app ran showed the new control and a skip warning after reopening the tab, no rebuild.
- Viewed on Slate, Aero Blue, Baby Pink, Materia, Sketchy, Vapor, Regal. Table search boxes checked on Aero Blue after the `forms.css` import (Bootstrap's `input-group-lg` rules are more specific, so they are unchanged).
- Benchmark `src-tauri/benchmark/results/20261113T002048Z_after_options_1cc6f54e.json` against `..._after_charts_b9f11ece.json` (the identical pre-change binary, run 7 hours earlier): maximised median about 220 to 240 ms, INCONCLUSIVE; normal size flagged INCOMPATIBLE (poll floor 50 ms versus 30 to 40 ms). No regression or improvement is claimed. The Options page itself (render, typing, slider drag, maximised) was not timed. Kim's hands-on check maximised is still needed.
- Seen once, not reproduced: on the very first open of Options the content area was scrolled about 200 px down. Repeated switches opened at the top.
- Weak spots: on Baby Pink and Vapor read-only fields look nearly the same as editable ones (theme overrides). Not tested: Rust rejecting a save from the Options tab (rollback path), keyboard-only use of the list group, touch, other seven themes, Windows. `start_page` in the default file is an example and is not used by the app.
- Backup before changes: `backup/rust_template_pre_options.zip` (verified).

### 13 November 2026: icon-only tabs when the labels do not fit (Claude)
- Problem (Kim, screenshot): with six tabs the Options tab wrapped onto a second row below about 700 px.
- Decision: switch on fit, not on a fixed 700 px breakpoint, because the number of tabs varies per app. `ui/js/tabs_fit.js` (any list with `data-tabs-fit`) measures with full labels and adds `.tabs-icons` when the last tab sits lower than the first. A ResizeObserver, a `data-theme` MutationObserver (fonts differ per theme) and `document.fonts.ready` trigger the check, once per animation frame at most. Labels are in `<span class="tab-label">` and are clipped (not removed) in icon mode, so screen readers still read them. A Bootstrap tooltip per tab is enabled only in icon mode and hidden on click.
- If icons alone do not fit, the row scrolls sideways (`flex-wrap: nowrap`, `overflow-x: auto`) with a 6 px themed WebKit scrollbar, and the selected tab is scrolled into view. Gotcha: setting the standard `scrollbar-width` makes WebKitGTK ignore the `::-webkit-scrollbar` rules; do not add it back.
- Tooltip styles copied from the template's `containers.css` (not loaded) into new `ui/css/tooltips.css`, made fully opaque (Bootstrap's 0.9 let text show through). Icon-mode tabs use smaller side padding so six icons fit at the 400 px minimum width without scrolling.
- Tested on the release build with xdotool in a temporary folder: Racing Green and Sketchy at 900, 700 and 400 px (labels at 900; icons at 700 in Racing Green, labels still fit in Sketchy at 700; icons at 400, no scrollbar); tooltip on hover; clicking a tab switches it and leaves no tooltip. With 14 temporary extra tabs (20 in total, removed afterwards and rebuilt): icons at 1000 px, sideways scroll at 600 px, End key selected and revealed the last tab. Not tested: other themes, touch, maximised-window responsiveness after this change (no benchmark run), Windows.
- Minimum window size was not changed: `tauri.conf.json` already sets `minWidth` 400 and `minHeight` 300.
- Observed, not caused by this work: at about 00:23 the project-root `config.json` disappeared and `dist/` now holds `config.json` and `options.json` (last written 00:37). The app keeps its settings in its working folder, so starting `dist/rust-scaffold` directly (for example by double-clicking it) uses `dist/`, while `run.sh` starts it from the project root. Left as found for Kim to decide.
- Backup before changes: `backup/rust_template_pre_tab_icons.zip` (verified).

### 13 November 2026: settings live beside the executable (Claude, Kim's decision)
- `config.json`, `options.json` and `debug.log` are now found through `app_dir()` in `lib.rs`: the folder of `current_exe()`, with symlinks resolved, falling back to the working directory only if that fails. The working directory no longer matters. With `run.sh` that folder is `dist/`.
- Benchmark updated to match: `bench.py` copies the release binary into its temporary folder with the fixture config and runs that copy (otherwise it would have written into `src-tauri/target/release/`). It reads and guards `dist/config.json`, falling back to the old root copy. Not yet run after this change.
- Tested: 19 unit tests pass; the binary started from `/` wrote its config beside itself and nothing in `/`; started through a symlink in another folder it used the real binary's folder; `./run.sh` opened at 725 x 560 from `dist/config.json` (Kim's settings) and left that file unchanged.
- Stale files: the project root again holds `config.json` and `options.json` dated 00:55 (Aero Blue, 939 x 599), created before this change by a run of the old binary from the project root. The new build ignores them. Left for Kim to delete.
- Windows: `run.bat` still builds and starts `src-tauri\target\release\rust-scaffold.exe`, so on Windows the settings would sit in that build folder. Untested; align it with `dist\` when Windows is tested.
- Backup before changes: `backup/rust_template_pre_exe_dir.zip` (verified).
