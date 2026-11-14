## Rust Scaffold: boilerplate for Kim's future Rust apps

User: Kim.
Version: 0.2.4
Date: 14 November 2026
Project folder: /opt/templates/rust
Full history before this file was rewritten: `docs/AGENTS_OLD.bak` (424 lines, every phase, test and failure in detail).

---

## What this project is

A lean, reusable Rust and Tauri 2 desktop application for Linux (Windows is coded but untested). It is not an app in its own right. It is the scaffold Kim copies when she starts a new Rust project, whatever that project turns out to be: tools, dashboards, utilities, front ends for AI and voice work, and so on. Because the future uses are unknown, the scaffold has working, tested examples of the pieces most apps need, each easy to keep or delete:

- A frameless, transparent window with rounded corners, a custom title bar, drag and resize, a tray icon, and 14 Bootstrap themes switchable at runtime.
- A vertical or horizontal menu with search (Ctrl+K), and demo pages: Dashboard, Panels (accordions), Tables (with search), Cards (drag to reorder), Charts (offline SVG), Options, Jobs, Errors and Help.
- Settings in JSON files, validated in Rust, with an Options page drawn from `options.json`.
- Live error reporting (Errors page) using the `tracing` crate.
- A Tokio job queue that runs tasks in Rust asynchronously, with a Jobs page to watch, inspect and kill them.

Principles: keep it small, keep dependencies few, and prove things with tests. We are cautious about crates and only add small ones, preferably ones Tauri already compiles. Current direct crates: tauri, serde, serde_json, tokio, regex, time, dirs, tracing, tracing-subscriber (436 crates in total including dependencies). The distributed app is a single file of about 4.9 MB (target: under 50 MB).

We continue to use this file to log successes, failures, tweaks and adjustments. Add a dated entry to the log at the bottom after every change.

---

## Rules for anyone working here

- Kim is new to Rust and wants to learn. Explain the reasons behind Rust and Tokio decisions in plain terms, use comparisons, and correct misconceptions directly.
- For anything large: give a short numbered plan, get Kim's approval, then work one step at a time and stop after each step for her check.
- Back up before changes: a verified zip in `backup/` (exclude `src-tauri/target`).
- `build.sh` copies the build into `dist/`, which is the copy Kim runs. Only approved code goes there. Build experiments with `cargo build --release` and copy the binary to a separate folder (for example `$CLAUDE_JOB_DIR/tmp`), run it with `RUST_SCAFFOLD_DIR` pointing at a temporary settings folder, and restore any source you changed byte for byte.
- Never drive the mouse or keyboard (xdotool tests, benchmarks) without asking Kim first that the desktop is free. Do not open windows while she is watching something full screen.
- Do not say "works", "fixed" or "ruled out" without a test that could have failed under Kim's real conditions (her launch path `./run.sh`, her window size, maximised, her themes). Report what was tested and what was not.
- No shadows or blur for now (see History). No inline `style=""` in HTML; the Content-Security-Policy blocks it anyway. Colours, sizes and spacing come from tokens in `ui/css/variables.css`. One CSS file per control type, imported by `ui/css/index.css`.
- Markdown documents go in `docs/`. No em dashes in public documents. Python scripts get a `DEBUG_MODE` flag, false by default.
- After every change, add a short dated note to `change_log.txt` (root, newest day first, plain words) as well as the detailed entry in the Log below.
- A test binary handed to Kim must be built after the last source change it is meant to test: compare the binary's time with the newest source file before handing it over.
- Allowed without asking (Kim, 14 November 2026): while the app is in development, the settings JSON files (`config.json`, `options.json`, `search.json`, `keys.json`, and any `*.corrupt` copies) and any SQLite databases the app creates may be deleted when testing or adding settings, in Kim's real settings folder (`~/.config/rust_scaffold`) as well as scratch folders. Nothing valuable is kept in them and the app recreates them. Say in the reply and the Log which files were deleted. This allowance does not cover anything else: not `backup/`, `dist/`, source, docs or the `*.default.json` files.
- Hidden-screen checks use `tools/xvfb_check.sh` and check files in `tools/checks/`. Never write a one-off xdotool or screenshot script.
- Never commit, stage, branch, push or pull. Kim manages git.

---

## Where things are

Source lives in two folders: Rust in `src-tauri/src/`, the interface in `ui/` (embedded into the binary at compile time, so any HTML, CSS or JS change needs a rebuild).

Root:
- `build.sh`: release build (`cargo build --release --locked -j 4` at low priority), copies the binary to `dist/`.
- `run.sh` / `run.bat`: start `dist/rust-scaffold` detached (`run.bat --build` builds first on Windows, untested).
- `dist/rust-scaffold`: the app Kim runs.
- `change_log.txt`: short daily notes of changes, newest first.
- `tools/xvfb_check.sh`, `tools/checks/*.sh`: the hidden-screen test tool and its checks (isolation, smoke, settings_files, errors_badge, menu_layout).
- `README.md`, `requirements.txt`, `docs/usage.md`: user and developer documentation. `docs/` also holds the GPU, GRUB recovery, performance and failure write-ups.
- `backup/`: zips taken before each milestone.

Rust (`src-tauri/`):
- `Cargo.toml`, `Cargo.lock`, `tauri.conf.json` (window, CSP; no version, Tauri takes it from `Cargo.toml`), `capabilities/default.json` (window permissions).
- `build.rs`: records build time, rustc and cargo versions and the crate list for the Help page.
- `src/lib.rs`: start-up order, `settings_dir()`, `DEBUG_MODE`, `APP_FOLDER`, `DIR_OVERRIDE`, command list.
- `src/commands.rs`: every command the interface can call.
- `src/config.rs`: `config.json` load, validate, atomic save, 5 MB limit (oversized file set aside and recreated).
- `src/options.rs`, `src/search.rs`, `src/keys.rs`, `src/files.rs`: `options.json`, `search.json` and `keys.json` (shortcut keys), shared read-or-create code.
- `src/logging.rs`: tracing layer, Errors list, `debug.log`.
- `src/jobs.rs`: the Tokio job queue.
- `src/tray.rs`, `src/gpu.rs`: tray icon; GPU rendering switch for WebKitGTK on NVIDIA.
- `src/startup.rs`: start-up fallback; shows the window and logs an error if `app.js` has not called `ui_ready` within 5 s.
- `options.default.json`, `search.default.json`, `keys.default.json`: copied into the settings folder when missing.
- `benchmark/`: benchmark scripts and raw results (see below).

Interface (`ui/`):
- `index.html`: all pages. `js/`: one script per feature (`app.js` loads config and shows the window, `options.js`, `jobs.js`, `errors.js` which loads first to catch script errors, `search.js`, `keys.js` (shortcut keys), `sortable.js`, `tabs_fit.js`, `theme.js`, `help.js`, chart and table scripts). `css/`: modules plus `variables.css` (theme tokens) and `bootstrap-flat.css` (switches off Bootstrap's shadows without editing the vendor file).

---

## Settings and log files: the `dirs` crate

The app uses the `dirs` crate to find the per-user configuration folder, so it never has to guess, and works when installed somewhere read-only such as `C:\Program Files`:

- Windows: `%APPDATA%\rust_scaffold` (Roaming, per user).
- Linux: `~/.config/rust_scaffold`.

`config.json`, `options.json`, `search.json`, `keys.json` and `debug.log` live there. Nothing is copied from the executable's folder. Each JSON file is created from its built-in default whenever it is missing, so any of them, or the whole folder, can be deleted at any time. A file over its size limit (`config.json` 5 MB, `options.json` and `search.json` 1 MB, `keys.json` 64 KB) is treated as corrupt: renamed to `NAME.json.corrupt` (one copy kept, replaced each time) and recreated with defaults (`files::set_aside`, status `recreated`). If that fails it is left alone and not loaded. Invalid JSON under the limit is never replaced. The environment variable `RUST_SCAFFOLD_DIR` overrides the folder; tests, benchmarks and experiments use it so Kim's real settings are never touched. If `dirs` finds nothing, the executable's folder is used. When making a new app, change `APP_FOLDER` and `DIR_OVERRIDE` in `lib.rs`.

---

## Logging and the Errors page: the `tracing` crate

Rust code logs with `error!`, `warn!`, `info!` and `debug!`. A custom layer in `logging.rs` keeps the newest warnings and errors in memory (`error_log_max`, default 100, 10 to 1000) and pushes each new one to the window as a `log:entry` event, so the Errors page updates instantly without polling. JavaScript errors are reported to Rust too. With `DEBUG_MODE` true in `lib.rs`, everything from debug level up is also appended to `debug.log`, which is cut to its newest half when it passes 1 MiB. With `DEBUG_MODE` false no log file is written.

---

## The Tokio job queue (async tasks in Rust)

`src/jobs.rs` is a small async pipeline: callers queue jobs, at most `job_concurrency` (1 to 3, config.json, default 2) run at once on Tauri's own Tokio runtime, and the rest wait in order. Each change is pushed to the window as a `jobs:changed` event carrying a rising `seq` number, so the page always keeps the newest copy of a job. The only job kind today is a mock "count" job (count 1 to N, N at most 100, with a pause of 100 to 10000 ms between counts, optional fail-on-purpose step), used to prove that async work can be started, queued, inspected and stopped. Real apps add their own job kinds in Rust; the interface can only request kinds defined there.

Things to know:
- Queued jobs are plain data, not tasks, so they can be listed and cancelled for free, and the limit can change while jobs run. Lowering it lets running jobs finish.
- Kill uses Tokio's `AbortHandle`. Tokio can only stop a task at an `.await` point. A job that computes for a long time without awaiting cannot be killed this way; split long work or use `spawn_blocking` with a cancel flag.
- The release build uses `panic = "abort"`: a panic inside a job ends the whole app. Return an error instead (the job shows as Failed).
- Inspect shows the Tokio task id and whether Tokio has dropped the task. The summary shows Tokio's worker threads, live tasks and run queue depth. "Late by" measures how much later than asked each pause ended.

---

## Build, run, test

- Build for Kim: `./build.sh`, about 1 minute incremental, about 2 minutes clean. Run: `./run.sh`.
- Hidden-screen checks: `tools/xvfb_check.sh smoke settings_files` (needs `cargo build --release` first; never touches the desktop or real settings; see `docs/usage.md`).
- Tests: `cd src-tauri && nice -n 10 cargo test --release -j 4` (54 tests: config, options, search, keys, settings files, settings folder, logging, job queue). Job tests use Tokio's paused clock, so a 100-step job is tested in milliseconds.
- The version number lives only in `src-tauri/Cargo.toml` (`tauri.conf.json` has no `version`, so Tauri uses the Cargo one; the About page reads `CARGO_PKG_VERSION`). After a version change run `cargo update --workspace --offline` (because `build.sh` uses `--locked`) and add a line to the change log on the Help page.

---

## Benchmark scripts (`src-tauri/benchmark/`)

They drive the real desktop with xdotool, so ask Kim first. Both start the app with `RUST_SCAFFOLD_DIR` set to a temporary folder.
- `bench.py run --label NAME`: builds with `build.sh`, then times 20 Dashboard-to-Panels tab switches at 640x480 and maximised, by polling a screenshot pixel. Saves raw JSON in `results/`. Floor of about 30 to 90 ms, so small differences are noise.
- `bench.py compare BEFORE.json AFTER.json` (rules in `compare.py`): median and p90 changes, flags incompatible runs and calls small or noisy differences inconclusive. The click and poll scenario is in `tab_switch.py`.
- `cpu.py`: CPU time used by the WebKit web process while the maximised app cycles pages and scrolls (`--layout vertical|horizontal`). Better than click timing for comparing paint cost.
- `gpu_check.py`, `gpu_check_launcher.sh`, `modeset_test/`: tools from the GPU rendering investigation.
- `README.md` in that folder has the commands and limits. Always check responsiveness by hand as well.

---

## History in a nutshell, and what we learned

1. Compiler blocked: `cc` was restricted to a `tools` group. Lesson: check `cc --version` before blaming Rust.
2. Slow when maximised: the copied Bootstrap template's stacked shadows and blur made every repaint expensive in WebKitGTK, which drew on the CPU. Claude called the project a failure and marked causes "ruled out" without proper tests. Codex then measured each effect group separately and found shadows were the cause (tab switch 600 to 1200 ms down to about 95 ms). Lessons: run the cheapest discriminating test first (a blank page in the same window, maximised); never declare failure or "ruled out" without a test that could detect the problem under Kim's conditions; test with her launch path and window size.
3. A light-theme shadow experiment was built with `build.sh` and reached `dist/` before it was measured. Kim found it slow. Lessons: experiments never go through `build.sh`; even one soft shadow on broad elements was too costly then.
4. GPU rendering: WebKitGTK refused the GPU because NVIDIA kernel mode setting was off (`nomodeset`). Fixed at boot level (recovery notes in `docs/grub_recovery.md`); the app now enables WebKit's DMA-BUF renderer when a GBM buffer can be allocated (`gpu.rs`). Lesson: the initramfs held its own copy of the setting; check every layer.
5. Settings once lived beside the executable, which fails in read-only install folders. Moved to the `dirs` folder in 0.2.2.
6. A test copy for Kim was built before the change it was meant to show (hide to the tray), so Kim tested old code (14 November). Lesson: compare build time with the newest source change before handing a binary over.
7. Smaller lessons: a test that moves the real mouse is ruined if Kim touches it; poll for the window instead of sleeping; diff config files after any aborted experiment; a late reply can overwrite newer events (fixed in the Jobs page with `seq`).

---

## Current state and next work

- Done in 0.2.2: Rust 1.100.0, Tauri 2.12.5, new crates (tokio, regex, time, dirs, tracing), per-user settings folder, Errors page, Tokio job queue and Jobs page.
- Now: adding crates, tweaking and refining what exists.
- Later: with GPU rendering working, low-cost ("lo-fi") shadows may be tried again. Rule: one restrained shadow on one component type, built outside `dist/`, measured with `cpu.py` and by Kim by hand, maximised, before keeping it.
- Leftovers: view all 14 themes on the newer pages; test on Windows; unused `dist/*.json` copies from 0.2.1 can be deleted; `src-tauri/target` grows past 2 GB and can be deleted safely (next build takes about 2 minutes).

---

## Log

### 13 November 2026: Jobs page (0.2.2 step 4), select fix, Errors toolbar, AGENTS.md rewritten (Claude)
- Backup before changes: `backup/rust_template_pre_jobs.zip` (verified). Old AGENTS.md copied to `docs/AGENTS_OLD.bak` (byte-identical).
- Select boxes: `forms.css` gave every `.form-select` `padding: 0.75rem 1rem`, which removed Bootstrap's right padding, so the arrow sat on the text (Kim's screenshot of the Errors filter). Now `padding-right: var(--select-chevron-space)` (2.5rem). Applies to every select in the app.
- Errors toolbar: Copy, Clear and Test are centred (three-column grid, filter left), the status line sits below; stacks when narrower than 560 px.
- Job queue: new `src-tauri/src/jobs.rs`, config key `job_concurrency` (1 to 3, default 2, clamped), Options entry in group "Jobs" (added to `options.default.json` and Kim's `~/.config/rust_scaffold/options.json`, nothing else changed), six commands (`jobs_list`, `jobs_start`, `jobs_cancel`, `jobs_cancel_all`, `jobs_clear_finished`, `jobs_inspect`). tokio gained the `rt` feature (runtime handle and metrics); dev-dependency tokio with `test-util` for a paused clock. No new crates in `Cargo.lock`.
- Jobs page: `ui/js/jobs.js`, `ui/css/jobs.css`, tokens in `variables.css`. Native `<progress>` bars (no inline widths). "At once" saves `job_concurrency` through `AppConfig.commit`.
- Tests: 44 pass (13 new: order with one at a time, limit raise and lower, cancel queued, cancel running, abort really ends the Tokio task, fail on purpose, kill all and clear, bad requests refused, event order, seq order, pruning to 50 finished). Mutation check: with `abort()` removed, the abort test fails.
- Self-test build (temporary script clicking inside the page, separate folder, override settings, no desktop input, removed afterwards; `index.html` verified identical): 5 + 1 jobs at 2 at once ran in order, a running job was killed at 4 of 8, a job failed at step 3, "Late by" about 1 to 1.6 ms. It found a bug: a late `jobs_list` reply erased jobs added just before it. Fixed with the `seq` number and covered by a Rust test; the fix was NOT re-checked on screen (Kim was watching a video full screen).
- Built with `build.sh` into `dist/`: 4,925,808 bytes.
- Not tested: the fixed page on screen, Kill all and Clear finished clicks, "At once" saving from the Jobs page and from Options, themes other than Aero Blue, the 640x480 default size (the table will scroll sideways there), Windows.

### 13 November 2026: Grok's set_var warning, Jobs table at 640x480 (Claude)
- Grok said `std::env::set_var` in `gpu.rs` is deprecated in favour of `set_var_os`. Checked: no such function exists, and set_var is not deprecated. Recompiling `gpu.rs` with Rust 1.100 gave zero warnings. In edition 2024 set_var becomes an `unsafe` function, because changing the environment while other threads run is unsafe on Linux. We use edition 2021, and `gpu::apply` runs before any thread starts. Added a comment saying so.
- Jobs table: Kim saw a sideways scroll bar at 640x480. The table now measures its own width (a CSS container query on `.jobs-table-wrap`). Under 40rem it hides Waited and Late by. Under 28rem it also hides Ran and the bar, keeping the count, and tightens the cell padding. Inspect still shows all of these.
- Found with a temporary on-page probe: the narrow padding first had no effect, because `window.css` (`.app-content .table-custom td`) loads later with equal weight. Fixed by raising the selector.
- FAILURE: tests ran on a hidden Xvfb screen (:99) so Kim's desktop was not touched. One chained command failed at `cp` ("Text file busy") before `export DISPLAY=:99`. The xdotool steps that followed used her real desktop: a click, a click plus Return, and 14 scroll steps on her browser. A screenshot of her screen was also taken. Lesson: set DISPLAY on each xdotool and import call itself, in a script with `set -euo pipefail`, and stop the old copy by its exact PID before copying.
- Re-test with such a script: at 640x480 the table is 317 px in 317 px of room, no scroll bar; at Kim's 854x638 it is 552 px in 552 px with the bar and Ran shown. Probe removed, `index.html` verified identical.

### 13 November 2026: version 0.2.3, backup, target cleared (Claude)
- Version 0.2.3 in `Cargo.toml`, `tauri.conf.json`, `Cargo.lock` (`cargo update --workspace --offline`) and the Help change log.
- Built with `build.sh`: `dist/rust-scaffold` 4,975,744 bytes. Tests not re-run (only CSS, JS, a comment and the version changed since the last 44 passes).
- Backup: `backup/rust_template_0.2.3.zip`, 5.9 MB, 811 files, verified with `unzip -t` (excludes `target/` and `backup/`). Note: the AGENTS.md inside it is the stale 401-line copy.
- `src-tauri/target` had reached 4.8 GB. Not one old version: `target/release/build/rust-scaffold` alone held 49 copies of about 60 MB, one per build configuration (test builds, feature changes, toolchain update). Cleared with `cargo clean`; the next build takes about 2 minutes. `cargo clean` is always safe.
- AGENTS.md was overwritten at 19:11: Kim changed the version to 0.2.3 in an editor window that still held the old, pre-rewrite text, and saved it. Restored from the last commit plus these entries. Kim will not edit AGENTS.md from now on.

### 13 November 2026: shortcut keys, keys.json (0.2.3) (Claude)
- Backup first: `backup/rust_template_pre_keys.zip` (verified).
- New `src-tauri/src/keys.rs`, `keys.default.json`, command `get_keys`; `ui/js/keys.js`; Help card "Shortcut keys" (cheat sheet drawn from the loaded file, status line, skipped bindings). Ctrl+K moved out of `search.js` into keys.json; `search.js` now exposes `window.AppSearch.open()`. Search library entry "Shortcut keys" added to `search.default.json` and Kim's `~/.config/rust_scaffold/search.json` (nothing else changed there).
- Defaults: / and Ctrl+K search, Escape leaves a text box, Alt+1 to Alt+9 the nine pages. No F1 (Kim: likely an OS key).
- Design: the file only picks among actions defined in `keys.js` (focus-search, leave-field, page:NAME); it cannot run code. User JavaScript (`user.js`) was discussed and deferred by Kim: anything able to write the settings folder could run code with all the app's commands, and the CSP would have to be loosened. Needs its own design, opt-in and off by default.
- Rules: Rust writes keys in one form (Ctrl+Alt+Shift+K); Shift only with letters and named keys ("?" not "Shift+/"); Tab and Ctrl+A/C/V/X/Y/Z refused; duplicates skipped. Keys without Ctrl or Alt are ignored while typing in a text box (except leave-field). Keys already handled by another script (defaultPrevented) are left alone, so Escape in the search box still closes the list first.
- Tests: 49 pass (5 new: default file valid and has no F1, standard form, bad keys refused, bad and duplicate bindings skipped, missing file created and bad files left alone).
- Hidden-screen test (Xvfb :99, 854x638, `set -euo pipefail` script, no input to Kim's desktop): keys.json created on first start; / focused search and "dash" listed results; three Escapes closed the list, cleared the text, left the box; Ctrl+K then typing "a/b" typed the slash; Alt+7 opened Jobs, Alt+9 Help; after adding bad bindings and a new Alt+0 to the file, reopening Help applied Alt+0 and listed both skipped bindings with reasons; the Errors badge counted the warnings.
- Built with `build.sh`: `dist/rust-scaffold` 4,995,208 bytes.
- Not tested: Kim's real desktop (Xvfb has no window manager, so LXQt or Openbox might take some Alt keys), the horizontal menu (/ does nothing there by design), Escape in other text boxes, the cheat sheet at 640x480, other themes, Windows.
- Job kinds proposal written to `docs/job_kinds.md` (not built; Kim to decide).

### 13 November 2026: 0.2.4 proposal (Claude)
- Wrote `docs/roadmap_0.2.4.md`: a summary of 0.2.3 and a proposed 0.2.4 theme, "ready to fork". Steps: (1) Kim checks the 0.2.3 open items on her real desktop; (2) the safe Xvfb test as a permanent tool; (3) a fork script that renames the about 12 name spots in 9 files and checks none are left; (4) job kinds per `docs/job_kinds.md`; (5) a fallback that shows the window if `app.js` fails; (6) optional Options page tidy. Nothing built; awaiting Kim's approval.

### 14 November 2026: settings files recreated, never copied (Claude)
- Kim's request: settings files must never be copied from the executable's folder; a missing file is recreated by the app; a file over its size limit is corrupt, never loaded, and recreated. Kim chose to keep the per-file limits (config 5 MB, options and search 1 MB, keys 64 KB).
- Backup first: `backup/rust_template_pre_settings_recreate.zip` (verified, excludes `target/`, `backup/`, `.git/`).
- `lib.rs`: `MIGRATED_FILES` and the copy step removed. Steps 2 and 3 of `settings_dir` moved into `pick_user_dir` so a test can pass temporary folders. The executable's folder is still the last-resort settings folder when `dirs` finds nothing.
- `files.rs`: oversized file renamed to `NAME.json.corrupt` (one copy, replaced each time; keeps the evidence instead of destroying it), default written, new status `recreated`. If the rename or write fails: `too_large`, file untouched, not loaded (old behaviour). New `write_new` creates the folder first, so deleting the whole settings folder while the app runs is safe. `config.rs` does the same for `config.json`; `recreated` allows saving. Invalid JSON under the limit is still never replaced (a hand-edit typo is not thrown away).
- JS messages for `recreated` in `app.js`, `options.js`, `search.js`, `keys.js`. Docs: `docs/usage.md`, `README.md`, this file.
- Tests: 54 pass (5 new: files.rs missing folder, oversized set aside and replaced twice, rename blocked leaves file alone; config oversized and deleted folder; lib.rs nothing copied from the executable folder; four old "too large" tests rewritten). Mutation check: with the rename removed from `set_aside`, 3 tests fail; source restored byte for byte.
- End-to-end on Xvfb :99 (script, no input, `XDG_CONFIG_HOME` in /tmp, release binary copied to /tmp with junk JSON beside it): nothing copied; 6 MB config, 70 KB keys and 2 MB search set aside and recreated at start; whole folder deleted then recreated on start.
- Not tested: oversized `options.json` in the real app (read only when the Options tab opens, which needs a click; unit test only); the warning banner text on screen; `./run.sh` and Kim's real desktop; Windows (`fs::rename` replaces an existing file there too, per the Rust docs, untested). `dist/` NOT updated: awaiting Kim's check before `build.sh`.

### 14 November 2026: hidden-screen test tool, change_log.txt (Claude)
- Roadmap 0.2.4 Step 1 done by Kim; Step 2 done here. Backup first: `backup/rust_template_pre_xvfb_tool.zip` (verified).
- `tools/xvfb_check.sh`: starts its own Xvfb (`-displayfd`, so Xvfb picks a free number; it got :1 here), refuses if that is the real display or not up, unsets `DISPLAY` and `WAYLAND_DISPLAY` so an unwrapped xdotool call fails, names the hidden screen on every helper call, `set -euo pipefail` plus an ERR message, kills only its own Xvfb and app PIDs, copies the binary to a scratch folder with scratch `RUST_SCAFFOLD_DIR` (or `XDG_CONFIG_HOME` with `app_start xdg`), `XDG_CACHE_HOME`, `XDG_DATA_HOME`. Scratch kept on failure or with `--keep`, else deleted.
- Checks: `tools/checks/smoke.sh` (4 assertions) and `tools/checks/settings_files.sh` (19, the settings-file work from this morning, now including oversized `options.json` via Alt+6). Both pass: 23 of 23.
- Rail test: a check that called `xdotool` directly got "Can't open display: (null)" and the run stopped there; a deliberate failed assertion gave exit 1 and kept the scratch folder; no Xvfb or app left running. Screenshots checked by eye: the app draws on the hidden screen, and the Options page and banner show the new "recreated" messages.
- New `change_log.txt` in the root; rule added above.
- Found, NOT fixed: the Errors badge showed 2 while the page listed 4 warnings (four oversized files at start). Likely cause (hypothesis, from reading `ui/js/errors.js`): `load()` replaces `entries` with the `get_errors` reply, dropping live `log:entry` events that arrived while the request was in flight; `unseen = entries.length` then undercounts. Same pattern as the Jobs `seq` bug. Fix would keep entries with an id above the reply's newest id.
- Not tested: the tool under Kim's own session other than from this terminal; `click` and `type_text` helpers (written, not yet used by a check).

### 14 November 2026: Errors badge, start-up fallback (Step 5), Options beside Help (Claude)
- Backup first: `backup/rust_template_pre_errors_fallback_nav.zip` (verified).
- Errors badge: `ui/js/errors.js` `load()` replaced the list with the `get_errors` reply, dropping live `log:entry` events that arrived while the request was out. Now each request in flight collects live entries and merges those the reply lacks (by id). Proof: experimental build with the old `errors.js`: badge 2 in 3 of 3 runs with four start-up warnings; fixed build: 4 in 3 of 3 (check `errors_badge`, screenshots by eye).
- Roadmap Step 5: new `src/startup.rs` (`arm` at the end of setup, a Tokio sleep of `FALLBACK_AFTER` = 5 s on Tauri's runtime; `mark_ready`), command `ui_ready`, called by `app.js` after it shows the window or deliberately leaves it hidden (start_minimised). If it never arrives, the window is shown and an error logged. Proof: experimental build with `throw` at the top of `app.js`: window visible after 5497 ms, Errors page listed the script error and the fallback error. Normal build: window visible after about 850 to 890 ms in every run (smoke check now asserts under 4000 ms). Sources restored byte for byte (`cmp`), `target/release` rebuilt from them.
- Menu: Options moved to sit with Help at the end of the menu (bottom of the vertical menu, right of the tab row), as Kim asked: users look for settings there. `nav-item-help` replaced by `nav-item-end` on Options (CSS in `navigation.css` moved with it). Screenshots: vertical 900x640, horizontal 1200x640, horizontal 640x480 (icons only), all correct by eye.
- Not changed, Kim to decide: the default Alt keys are by page name, so Alt+6 still opens Options and Alt+7/8 Jobs/Errors, which no longer matches the on-screen order (Jobs and Errors now sit 6th and 7th).
- `xvfb_check.sh` now reports `$WINDOW_MS` (launch to visible window). New checks `errors_badge` and `menu_layout` (screenshots, checked by eye).
- Tests: 54 Rust tests pass; hidden-screen checks 24 of 24 on the final build.
- Not tested: Kim's real desktop and `./run.sh`; themes other than Aero Blue; the start_minimised path with the fallback (tray present, window must stay hidden and `ui_ready` still sent; by reading only); maximised.

### 14 November 2026: Racing Green default, keys renumbered, build, tray double-click research (Claude)
- Default theme `racing-green` (was `aero-blue`): `config.rs` default, `theme.js` `DEFAULT_THEME`, `index.html` `data-theme` and the title bar label, README example, `docs/usage.md` table. Only affects new or recreated `config.json`; Kim's own file already says racing-green.
- Default keys follow the new menu order: Alt+6 Jobs, Alt+7 Errors, Alt+8 Options (Alt+1 to 5 and 9 unchanged). Kim's `~/.config/rust_scaffold/keys.json` was byte-identical to the old default; copied to `backup/kim_keys_before_renumber_2026-11-14.json`, then deleted at Kim's request so the app recreates it. Checks updated (`settings_files` opens Options with Alt+8 and only passes if that opens Options; `errors_badge` uses Alt+7).
- Tests: 54 Rust tests pass; hidden-screen checks 24 of 24; screenshot shows Racing Green on a fresh settings folder.
- Built with `build.sh` at Kim's approval: `dist/rust-scaffold` 5,000,096 bytes; `xvfb_check.sh --bin dist/rust-scaffold smoke` passes (window by app.js in 791 ms).
- Tray double-click on Linux, research only, nothing changed: Tauri 2.12.5 uses `tray-icon` 0.25.1, which now has a KSNI backend (pure Rust StatusNotifierItem over D-Bus) besides AppIndicator. Its docs: AppIndicator emits no tray events (our old limit); KSNI emits left- and middle-click activation, no double-click (`DoubleClick` is Windows only), right click is handled by the panel. Tauri has no feature for it; adding `tray-icon = { version = "0.25", default-features = false, features = ["ksni"] }` to our Cargo.toml would switch it on (Cargo feature unification). Cost measured in a /tmp copy: 41 new crates (zbus, zvariant, async-io, rustix and helpers), normal dependency tree 342 to 454 lines. Kim's LXQt panel has the `statusnotifier` plugin and a host registered on D-Bus, so a KSNI icon should show. Hypothesis, untested: LXQt sends one Activate per click, so a double-click arrives as two activations, and the app can time them (two within about 400 ms = toggle). Awaiting Kim's decision.

### 14 November 2026: hide_to_tray option, tray click test build (Claude)
- Kim approved the tray click test and asked for a setting choosing hide (off the taskbar, default) or minimise.
- Test build (not in `dist/`): `tray-icon` with `ksni` added and every `TrayIconEvent` appended to `/tmp/tray_test/events.log`; binary `/tmp/tray_test/rust-scaffold-ksni`, 6,325,712 bytes (5,000,096 without KSNI, so about +1.3 MB). `Cargo.toml`, `Cargo.lock`, `tray.rs` restored byte for byte (`cmp`). Launcher `/tmp/tray_test/run_tray_test.sh` (scratch settings, refuses if the normal app runs). Awaiting Kim's clicks.
- New config key `hide_to_tray` (default true) and Options switch "Hide to the tray" (group Window; added to `options.default.json` and, after a backup to `backup/kim_options_before_hide_to_tray_2026-11-14.json`, one line inserted into Kim's `options.json`, nothing else changed). `tray.rs`: Show/Hide and the "hide" action now hide (or minimise when the setting is off, or when there is no tray); showing a hidden window centres it on the primary monitor first, a minimised one is restored in place. Settings are read at the moment of use, so a saved change applies at once. Centring moved from `lib.rs` setup into `centre_on_primary`, shared by start-up and the tray.
- Tests: 54 pass (the tray config test now covers `hide_to_tray`); release build clean; Options page screenshot on the hidden screen shows the switch on by default.
- Not tested: hide, minimise and centring on Kim's real desktop (Xvfb has no window manager or tray; the window manager may re-place a re-shown window); not built into `dist/`.

### 14 November 2026: tray double-click on Linux (KSNI), stale test copy, private D-Bus for checks (Claude)
- FAILURE: the click-test copy (`/tmp/tray_test/rust-scaffold-ksni`) was built at 12:00, before `hide_to_tray` was added, so Kim's "Hide to the tray" test ran old code (Show / Hide still minimised). The Options switch appeared only because Kim's `options.json` already had the entry. That old copy also saved the switch as `extras.hide_to_tray` in Kim's `config.json`. Rule added above (check build time against the newest source change).
- The click log answered the real question: 41 events, each `Click { button: Left, button_state: Up }`, one per click; inside Kim's double-clicks the clicks were 126 to 426 ms apart. The KSNI icon showed in her LXQt panel.
- Built in: Cargo feature `linux-tray-clicks` (default on) enabling an optional Linux-only `tray-icon` 0.25 dependency with `ksni`; `cargo tree -i tray-icon` shows one copy (0.25.1), shared with Tauri. `Cargo.lock` now lists 485 packages. `tray.rs`: `is_double_click` (Windows and macOS: their `DoubleClick` event; Linux: two left releases within `DOUBLE_CLICK` = 500 ms, via `second_click`, a pair is used up so a third click starts a new pair) toggles the window.
- `config.rs`: extras whose key is a built-in field are dropped in `validate` (names taken from the serialised defaults), so Kim's stray `extras.hide_to_tray` (true, same as the default) goes on the next load or save.
- Tests: 56 pass (new: `two_quick_clicks_are_a_double_click`, `extras_that_name_a_built_in_field_are_dropped`). Release binary 6,336,752 bytes.
- Test tool hole found and fixed: hidden runs used Kim's real session bus, so each run probably showed a tray icon in her panel for a few seconds (not verified for earlier runs). Now a private `dbus-daemon` with a config that has no service folders (the standard session config auto-started the desktop portal, which waited 25 s for the keyring: window at 26.6 s, found with `dbus-monitor`; accessibility ruled out by testing `NO_AT_BRIDGE=1`, no change). New check `isolation`: the real panel's StatusNotifierItem list is identical before and during a run (passes). All checks: 26 distinct assertions pass (the temporary panel check and `isolation` are the same test; an earlier report to Kim of 28 counted it twice).
- Candidate for Kim (not in `dist/`): `/tmp/tray_test/rust-scaffold-candidate`, copy of the 12:20 build (`tray.rs` last changed 12:17), launched by `/tmp/tray_test/run_candidate.sh` with Kim's real settings; refuses if any copy runs.
- Not tested: the double-click, hide, minimise and centring on Kim's desktop (needs her clicks); right-click menu under KSNI (Kim did not report it); Windows and macOS double-click (code only).

### 14 November 2026: version 0.2.4 built into dist/ (Claude)
- Kim tested the candidate on her desktop: tray double-click hides (off the taskbar) and shows (centred), and with "Hide to the tray" off it minimises instead. Approved for `dist/`.
- Backup first: `backup/rust_template_pre_0.2.4.zip` (verified). Version 0.2.4 in `Cargo.toml`, `tauri.conf.json`, `Cargo.lock` (`cargo update --workspace --offline`); Help change log entry; `change_log.txt` heading.
- Tests: 56 pass. Built with `build.sh`: `dist/rust-scaffold` 6,337,008 bytes, identical to the fresh `target/release` build (`cmp`).
- Checks run against `dist/rust-scaffold`: isolation, smoke, settings_files, errors_badge, 26 of 26 pass. Screenshots: Help shows the 0.2.4 entry, About shows 0.2.4, Build lists `tray-icon 0.25.1` and 484 crates. The Errors badge there showed 1: "tray: could not be created", expected on the private test bus (no StatusNotifier host).
- Backup after: `backup/rust_template_0.2.4.zip` (verified).
- Not tested in this build: Kim's `./run.sh` launch of this exact `dist/` file (the candidate she tested was built from the same source, minus only the version number and the Help text).


### 14 November 2026: roadmap rewritten, version in Cargo.toml only, deletion allowance (Claude)
- Kim checked the 0.2.4 `dist/` build with `./run.sh`: it starts, tray double-click hides and shows, the tray menu and Quit work.
- Backup first: `backup/rust_template_pre_roadmap_jobkinds.zip` (verified).
- Version test (roadmap "something to check"): scratch copy in /tmp with `Cargo.toml` at 0.2.99 and the `version` line removed from `tauri.conf.json`, plus a temporary test printing `generate_context!()` package info: 0.2.99, config version None. Control with the line back: 0.2.4. The About page cannot show this (it reads `CARGO_PKG_VERSION`), so the probe was needed. Scratch copy deleted; project source untouched (`git status` clean) until the change below.
- Applied: `version` removed from `src-tauri/tauri.conf.json`; AGENTS.md and README.md now say the version lives only in `Cargo.toml`.
- `docs/roadmap_0.2.4.md` rewritten: Steps 1, 2 and 5 and the extra work marked done; next are job kinds, then "how to" guides for new apps, then the optional Options tidy. The fork script moved to a later version by Kim's decision (project still changing); measured scope now 95 lines in 33 files, not 12 places in 9. Forks start without this project's history.
- New rule above: settings JSON files and SQLite databases may be deleted without asking while in development (Kim's decision).
- Tests: 56 pass; release build clean; `xvfb_check.sh smoke` 5 of 5. `dist/` not rebuilt (no change a user can see).
