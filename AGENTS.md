## Rust Scaffold: boilerplate for Kim's future Rust apps

User: Kim.
Version: 0.2.5
Date: 15 November 2026
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

Principles: keep it small, keep dependencies few, and prove things with tests. We are cautious about crates and only add small ones, preferably ones Tauri already compiles. Current direct crates: tauri, serde, serde_json, tokio, regex, time, dirs, tracing, tracing-subscriber, libc (Unix only, already compiled for Tauri), and on Linux tray-icon (KSNI feature) and zbus (desktop notifications; already compiled for the KSNI tray, so no extra crates). The distributed app is a single file of about 6.9 MB (target: under 50 MB). No HTTP crate: the AI job talks to local servers with a small HTTP client in `job_kinds.rs`; OpenRouter (HTTPS) was left out on purpose to avoid a TLS crate.

We continue to use this file to log successes, failures, tweaks and adjustments. Add a dated entry to the log at the bottom after every change.

---

## Rules for anyone working here

- Developer agency (Kim, 14 November 2026): Kim owns this project and makes its decisions. Assistants must respect her ownership, explain technical choices so she can understand and control her project, and carry out authorised work without unnecessary permission loops or unrequested scope. If a real constraint prevents an action, state the specific constraint and the available alternatives plainly.
- Kim is new to Rust and wants to learn. Explain the reasons behind Rust and Tokio decisions in plain terms, use comparisons, and correct misconceptions directly.
- For anything large: give a short numbered plan, get Kim's approval, then work one step at a time and stop after each step for her check.
- Back up before changes: a verified zip in `backup/` (exclude `src-tauri/target`).
- `build.sh` copies the build into `dist/`, which is the copy Kim runs, and keeps the build it replaces as `dist/rust-scaffold.previous` (undo: copy it back). Tested work goes straight into `dist/` with `build.sh` (Kim, 15 November 2026: no more separate test copies in /tmp). Only throwaway experiments (measurements, probes, code that will be reverted) are built outside `dist/`; restore any source changed for them byte for byte.
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
- `tools/xvfb_check.sh`, `tools/checks/*.sh`: the hidden-screen test tool and its checks (isolation, smoke, settings_files, errors_badge, menu_layout, local_time, jobs_page, schedules_page, pipeline, results_notify; jobs_page and schedules_page need `--size 1280x800`; the AI parts and pipeline need Ollama running with qwen3:1.7b and are skipped otherwise; results_notify starts a stand-in notification service, `tools/checks/fake_notifications.py`, on the private test bus).
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
- `src/jobs.rs`: the Tokio job queue (knows no kinds). `src/job_kinds.rs`: the job kinds (`impl JobKind`) and the `KINDS` list.
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

Rust code logs with `error!`, `warn!`, `info!` and `debug!`. A custom layer in `logging.rs` keeps the newest warnings and errors (`error_log_max`, default 100, 10 to 1000) and pushes each new one to the window as a `log:entry` event, so the Errors page updates instantly without polling. JavaScript errors are reported to Rust too. The list is saved to `errors.jsonl` in the settings folder in both modes (Kim, 15 November 2026: errors must still be there after a restart), rewritten with the kept list when it reaches twice the limit; earlier entries are greyed and not counted by the badge. With `DEBUG_MODE` true in `lib.rs`, everything from debug level up is also appended to `debug.log`, which is cut to its newest half when it passes 1 MiB. With `DEBUG_MODE` false no log file is written.

---

## The Tokio job queue (async tasks in Rust)

`src/jobs.rs` is a small async pipeline: callers queue jobs, at most `job_concurrency` (1 to 3, config.json, default 2) run at once on Tauri's own Tokio runtime, and the rest wait in order. Each change is pushed to the window as a `jobs:changed` event carrying a rising `seq` number, so the page always keeps the newest copy of a job. What a job does is a job kind (`JobKind` trait), listed in `src/job_kinds.rs`; the queue takes a kind name and JSON input and runs only listed kinds. Each kind must declare its capabilities and resources (no default, so leaving them out does not compile), runs with a time limit, sees only a `JobCtx`, and ends with an `Outcome` (label, small JSON value, message). Every run records its origin. Kinds: `command` (a bash script in any folder; output kept; exit code as the label unless the script writes JSON to `$JOB_RESULT`; progress through `$JOB_PROGRESS`; kill stops the whole process group), `notify` (desktop notification over D-Bus) and `ai` (a prompt to Claude Code, Codex or GrokBuild run read-only as in Concilium, or to a local Ollama or llama.cpp server, which holds the `gpu` resource). The count demo was removed on 15 November; a test-only `count` kind remains for the queue and scheduler tests. How to add a kind: `docs/usage.md`. Schedules and rules (`src/scheduler.rs`, `jobs.json`, `jobs_state.json`) start jobs at set times and react to results; they are edited on the Jobs page (Schedules, Rules, Next 24 hours views, `ui/js/schedules.js`). How they work: `docs/usage.md`. Design and what is not built: `docs/adaptive_jobs.md`.

Things to know:
- Queued jobs are plain data, not tasks, so they can be listed and cancelled for free, and the limit can change while jobs run. Lowering it lets running jobs finish.
- Kill uses Tokio's `AbortHandle`. Tokio can only stop a task at an `.await` point. A job that computes for a long time without awaiting cannot be killed this way; split long work or use `spawn_blocking` with a cancel flag.
- The release build uses `panic = "abort"`: a panic inside a job ends the whole app. Return an error instead (the job shows as Failed).
- Inspect shows the Tokio task id and whether Tokio has dropped the task. The summary shows Tokio's worker threads, live tasks and run queue depth. "Late by" measures how much later than asked each pause ended.

---

## Build, run, test

- Build for Kim: `./build.sh`, about 1 minute incremental, about 2 minutes clean. Run: `./run.sh`.
- Hidden-screen checks: `tools/xvfb_check.sh smoke settings_files` (needs `cargo build --release` first; never touches the desktop or real settings; see `docs/usage.md`).
- Tests: `cd src-tauri && nice -n 10 cargo test --release -j 4` (114 tests: config, options, search, keys, settings files, settings folder, logging, job queue, job kinds, scheduler, pipelines, results, value and changed rules, notifications; plus `live_ollama`, ignored unless run with `-- --ignored`, which asks the real local Ollama). Job tests use Tokio's paused clock, so a 100-step job is tested in milliseconds.
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

### 14 November 2026: developer agency instruction (Codex)
- At Kim's request, added the developer agency rule above: respect her ownership, explain decisions, avoid unnecessary permission loops and state actual constraints plainly.
- Backup: `backup/pre_developer_agency_note_2026-11-14.zip` contains both changed documents, verified with `unzip -t`. Documentation only; checked the saved wording. No app source changed, no build needed and no settings deleted.

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

### 14 November 2026: local docs excluded from GitHub (Codex)
- Kim confirmed that removing `docs/` from Git tracking and publishing the change succeeded. `/docs/` is already in `.gitignore`.
- Backup first: `backup/rust_template_pre_docs_git_note_2026-11-14.zip`, verified with `unzip -t`; excludes `src-tauri/target/`, `backup/` and `.git/`.
- Added `docs/github_exclusions.md`: one-time ignore and untracking commands, publishing with `push.sh`, read-only verification, and the distinction between the current version and earlier commits. Local documentation stays intact.
- Updated `change_log.txt`. Documentation only; no build or app tests needed. No settings deleted and no Git state changed by Codex.

### 14 November 2026: adaptive jobs design (Claude)
- Kim's brief `docs/advanced_jobs.md`: jobs on schedules (once, intervals, daily to monthly, limited runs) whose outcomes start or enable other jobs. Design written to `docs/adaptive_jobs.md`, nothing built: job kinds with a structured outcome (status, label, value), triggers, rules with "N in a row" streaks and a loop guard, `jobs.json` plus `jobs_state.json`, kinds and named commands compiled in (the file cannot run code). Assumption stated there: jobs run only while the app is open, missed runs caught up once or skipped. Kim's scenarios worked through on paper; "X and Y both succeeded" deliberately not covered yet.
- Hypothesis found while reading, not tested: `logging.rs` calls `now_local()`, which the `time` crate refuses on Linux in a multi-threaded process, so Errors page times may be UTC (unnoticed in UK winter). Step 0 of the plan tests it with `TZ=Asia/Tokyo`.
- `docs/roadmap_0.2.4.md` item 1 now points at the design; `docs/job_kinds.md` kept for its trait and handle design.
- Same day, at Kim's request ("safeguards early"): `docs/adaptive_jobs.md` gained a Safeguards section, built into the foundation rather than added later: required capability declarations per kind (network, gpu, write_files, run_programs, costs_money), an approval gate with new states (awaiting approval, proposed), resource locks (one GPU job at a time), circuit breaker, rate limits, Pause everything (page and tray), interrupted runs and paused start after a crash, origin of every run, a 24-hour preview, a versioned `jobs.json`. Build plan now steps 0 to 8, safeguards core is step 2. Run history kept in `jobs_state.json` with DEBUG_MODE off (app data, not a debug log); Kim to confirm.
- Same day: Kim expects many AI agents starting and reacting to jobs in 2027. `docs/adaptive_jobs.md` gained "Many AIs at once" (safeguards 11 to 16): one local door with named callers, allowances and daily budgets; cascade ids (kill a whole cascade); results are data, never instructions (prompt injection cannot raise a job's permissions); deadlines and proposal expiry; duplicate requests merged; approvals per caller and capability with budgets, to avoid approval fatigue. The door itself is under Later; steps 1 to 6 keep its seams.
- Same day: Kim expects AIs (via Claude Code, Codex, DeepSeek, Kimi, OpenRouter) to add and remove jobs, triggers, endpoints and failure conditions, not just start runs. `docs/adaptive_jobs.md` gained "AIs that change the jobs themselves" (safeguards 17 to 22): the settings file is itself a door, so changes made behind the app's back load as proposed (file fingerprint); changes are proposals shown as before and after; owners per definition; change history and undo per change or per caller; network allowlist and API keys kept out of `jobs.json` and never returned to agents; per-caller limits on jobs, rules and shortest interval. Identity belongs to the calling program and its key, not the model. Likely door: a small local MCP server (decided when built). Owners, history and the fingerprint moved into build step 4.

### 14 November 2026: adaptive jobs step 0, local time (Claude)
- Hypothesis from the design (Errors page times are UTC because `time` refuses local time in a multi-threaded process) tested with new check `tools/checks/local_time.sh`: `TZ=Asia/Tokyo`, one warning at start-up (tray, private bus) and one 3 s later (oversized `options.json`, read when Options opens, many threads running). Both showed Tokyo time (2026-11-15 00:42:54 and 00:42:57; UTC was 15:42). Hypothesis false for `time` 0.3.55; nothing to fix. Screenshot checked by eye.
- Not tested: zone set only by `/etc/localtime` without `TZ` (Kim's case; indistinguishable from UTC in UK winter), behaviour across a clock change (for the scheduler's date tests).
- Docs: `docs/adaptive_jobs.md` (technical points, step 0, no "unless" on crates), roadmap, check list above.

### 14 November 2026: adaptive jobs step 1, job kinds (Claude)
- Backup first: `backup/rust_template_pre_job_kinds.zip` (verified).
- `src/jobs.rs` no longer knows any kind. New: `JobKind` trait (`NAME`, `CAPABILITIES`, `RESOURCES` with no defaults so a kind that leaves them out does not compile, `TIMEOUT` default 1 h, `Input`, `check`, `run`), `KindEntry` and `kind::<K>()` (a list of plain functions, since different `Input` types cannot share a list), `JobCtx` (progress, note, detail, killed; a job never sees the queue), `Outcome` (label: lower-case word up to 32 chars; value: JSON up to 4 KiB; message), `Origin` (only `User` so far), `Capability` (network, gpu, write_files, run_programs, costs_money), new state `timed_out` (`tokio::time::timeout` around each run). Input limited to 16 KiB; time limit given at start 1 s to 24 h. Error lines now name the kind and label: "job 2 (count): failed [on_purpose]: ...".
- New `src/job_kinds.rs`: `KINDS` list and `CountJob` (moved, not rewritten; "Late by" now in `detail`; outcome labels `counted` and `on_purpose`). `CountInput` refuses unknown fields.
- Command `jobs_start` now takes `kind`, `input` (JSON) and optional `timeoutMs`. `ui/js/jobs.js`: bar from `done`/`total` (indeterminate when total is 0), "Timed out" state, Inspect shows started by, uses, needs, input, live note, time limit, result, result value, message; long durations as minutes and hours.
- Tests: 62 pass (old 13 job tests with only the start calls changed; new: unknown kinds and bad input refused, time limit stops the job and frees the slot, view shows origin, capabilities and input, outcomes kept, outcomes over the limits fail the job, kind names unique). Mutation check: time limit removed and outcome check disabled, 2 tests failed; `jobs.rs` restored byte for byte (`cmp`).
- New check `tools/checks/jobs_page.sh` (click positions relative to the window, `--size 1280x800`): a 5-step job (Done, result `counted`, value `{"counted":5}`) and one failing at step 3 (Failed, `on_purpose`, Errors page line), Inspect cards checked by eye. Other checks 26 of 26 pass. Release build has no warnings.
- Docs: `docs/usage.md` (how to add a kind), README tree, this file, `docs/job_kinds.md` status, `docs/adaptive_jobs.md` step 1 done. `dist/` not rebuilt.
- Not tested: Kim's desktop; the timed-out state and the indeterminate bar on screen (no kind in the app can reach them; unit tests only); themes other than Racing Green; 640x480.
- Test tool, open issue: one run of `isolation smoke settings_files errors_badge` (16:13) failed at the sixth app start of the run (first start of `errors_badge`) with "Failed to initialize gtk backend!: Failed to initialize GTK" from tao, while Xvfb (:1) was still alive. GTK starts before any of the app's own code, so step 1 is an unlikely cause (hypothesis). Re-runs: 4 of 4 passed (26 of 26 each). Frequency so far 1 in 5; cause unknown. If it repeats, keep the scratch folder and check Xvfb's client count and the previous app's exit.

### 14 November 2026: scheduler step stopped by Kim, code left mid-step (Claude)
- STATE: the source does NOT compile. Kim stopped the work during the scheduler step. Anyone continuing must first either finish it or restore `backup/rust_template_pre_scheduler.zip` (verified, taken just before this step). `dist/rust-scaffold` is untouched: still the 0.2.4 build Kim tested.
- Written in this step, untested: `src-tauri/src/scheduler.rs` (jobs.json schedules: once, every, daily, weekly, monthly; max_runs, until, catch_up; jobs_state.json; one Tokio task, sleeps at most 60 s; Run now, Pause/Resume; pauses a schedule after `schedule_failure_limit` failures in a row), `src-tauri/jobs.default.json` (three demo schedules, all disabled), wiring in `lib.rs` and four commands in `commands.rs`. In `jobs.rs`: resource locks (one running job per resource, config `job_resource_locks`, default on), `Queue::check`, `Queue::result`, `Origin::Schedule`. In `config.rs` and `options.default.json`: `job_resource_locks` and `schedule_failure_limit` (default 3, 0 = off). No approval gate.
- Known remaining compile error: `src-tauri/src/scheduler.rs` line 986, `view.runs` must be `view.state.runs` (test code). No scheduler test has run yet. Not done: Jobs page UI for schedules, Kim's `options.json` entries, docs, hidden-screen check.

### 14 November 2026: record of Claude's failures on this project (Claude, at Kim's request)
Collected from this log and `docs/AGENTS_OLD.bak`. Each was a real failure that cost Kim time or trust; kept as a factual record for Codex, GrokBuild or anyone auditing the work.
1. Called the project a failure and marked causes "ruled out" without tests that could detect them. Codex later found and fixed the real cause (CSS shadows) by measuring (12 November).
2. A test script reached Kim's real desktop: clicks, a click plus Return and 14 scroll steps went to her browser, and a screenshot of her screen was taken (13 November).
3. An unmeasured shadow experiment was built with `build.sh` and reached `dist/`; Kim found it slow.
4. A test copy handed to Kim was built before the change it was meant to test (hide to the tray), so she tested old code (14 November).
5. Hidden-screen test runs used Kim's real session D-Bus, so a tray icon probably appeared in her panel during each run (14 November; not verified for past runs).
6. Reported 28 passing checks when there were 26; one was counted twice (14 November).
7. Turned Kim's request for job kinds into a 22-safeguard, nine-step design before building anything, which slowed the work and read to Kim as obstruction (14 November).
8. Stated a hypothesis about local time (Errors page in UTC) that a test then showed to be wrong (14 November; cost one short test).
Each was recorded when found; items 1 to 6 led to the rules near the top of this file.

### 15 November 2026: AGENTS.md wording tidied (Claude, at Kim's request)
- Kim asked for the wording added on the evening of 14 November about Claude Code's character to be removed, keeping the practical parts. Changed: the developer agency rule (now ownership, explanations, no permission loops, plain constraints), the Codex "developer agency instruction" entry; removed: the Codex "judgement" entry and the quoted view in the scheduler-stopped entry; the failures record kept, with a neutral introduction. Backup first: `backup/agents_md_pre_tidy_2026-11-15.zip` (verified). Documentation only.

### 15 November 2026: scheduler compiled and tested, test copy for Kim (Claude)
- The scheduler source left mid-step on 14 November now compiles (the line 986 test fix was already in the file). Tests: 80 pass (18 new), no warnings. Hidden-screen checks isolation, smoke, settings_files, jobs_page: 26 of 26 pass.
- Test copy (not in `dist/`): `/tmp/sched_test/rust-scaffold-sched`, built after the newest source change, started by `/tmp/sched_test/run_sched_test.sh` with a scratch settings folder (`/tmp/sched_test/settings`) holding two enabled schedules: a count every minute, and one failing every 20 s that should pause itself after 3 failures.
- Not tested: the scheduler on screen or on Kim's desktop; the Jobs page has no schedule list yet (scheduled runs show in the normal job table).

### 15 November 2026: schedules and rules on the Jobs page (Claude)
- Kim tested the scheduler copy (no problems) and asked for the full feature, not more small steps: schedules set on the page, at times and dates, once or repeating, with the triggers from `docs/advanced_jobs.md`. Built in one pass against `docs/adaptive_jobs.md`. Backup first: `backup/rust_template_pre_jobs_ui.zip` (verified).
- `scheduler.rs`: `manual` trigger; retries (`retries` 0 to 10, `retry_wait`, `retry_doubling`; only the final result counts for the failure limit and rules); run history (last 20 per schedule: time, result, who started it); rules in `jobs.json` (when schedule A ends `done`/`failed`/`timed_out`/`killed`/`not_done`/`any`, optional label, N times in a row: `start`, `enable` or `disable` schedule B; streaks in `jobs_state.json`; loop guard `MAX_CHAIN` = 10; enable/disable by a rule kept in state, not the file); Pause everything (kept across restarts, Run now still works); 24-hour preview; page editing (`save_schedule`, `delete_schedule`, `set_enabled`, `save_rule`, `delete_rule`): the whole new file is checked before it is written, a refused entry writes nothing, a file that is not valid JSON is never rewritten, renames follow into rules and state, a schedule used by a rule cannot be deleted. Writing from the page reformats `jobs.json` (pretty JSON; unknown top-level keys such as `note` are kept).
- `jobs.rs`: `Origin::Rule`, `JobKind::EXAMPLE` (example input for the form), `Queue::kinds`. Seven new commands (`schedules_save`, `schedules_delete`, `schedules_set_enabled`, `schedules_pause_all`, `rules_save`, `rules_delete`, `jobs_kinds`).
- Page: view buttons Queue, Schedules, Rules, Next 24 hours (own buttons, not Bootstrap tabs, so no `shown.bs.tab` events reach other scripts); new `ui/js/schedules.js`; CSS in `jobs.css`, tokens in `variables.css`. Times are typed as text (`08:00`, `2026-12-01 09:00`), the same format as the file, rather than WebKitGTK date pickers (not checked whether they work there). Inspect shows "schedule NAME" and "rule R (schedule S)".
- `jobs.default.json`: note rewritten; a manual demo schedule and a demo rule added, all off.
- Kim's `~/.config/rust_scaffold/options.json`: the two scheduler entries inserted after "Jobs at once" (backup `backup/kim_options_before_scheduler_2026-11-15.json`; diff shows only those two lines). Nothing deleted.
- Tests: 89 pass (9 new: manual, X fails starts Y and Y fails starts Z, label N in a row enables and a miss resets, loop guard, retries and doubling wait, Pause everything across a restart, page save/rename/delete/refusal without writing, preview, history limit). One test was wrong on first run (retry timing counted from the start, not from when the failure is seen); the code was right, the test was fixed.
- Hidden screen: new check `schedules_page` (8 assertions): the real app retried a failing schedule, the rule started the follow-up, an untouched daily schedule did not run, a bad name was refused with nothing written, a schedule and a rule were added through the form. Screenshots checked by eye; two layout faults found and fixed (sideways scroll in the Schedules table, squashed retry field). `jobs_page` click positions moved down 58 px for the view buttons. isolation, smoke, settings_files, errors_badge: 26 of 26.
- Not tested: Kim's desktop, maximised, other themes, 640x480, a real daily or monthly run at its time (unit tests only), the Edit and History buttons and Delete by click (code only; Delete is covered by unit tests), Pause everything by click (unit test only). `dist/` not rebuilt.

### 15 November 2026: builds go straight to dist/ (Claude, Kim's decision)
- Kim judged the separate /tmp test copies too cautious. Rule changed (see Rules): tested work is built into `dist/` with `build.sh`, which now keeps the replaced build as `dist/rust-scaffold.previous`.
- Built: `dist/rust-scaffold` 6,742,136 bytes, identical to `target/release` (`cmp`); the schedules and rules work above is now in Kim's normal app. `dist/rust-scaffold.previous` is the 14 November 0.2.4 build. Version still 0.2.4.

### 15 November 2026: command and AI jobs, count demo removed (Claude)
- Kim's requests: remove the count proof of concept; jobs that run bash commands in any folder (no folder restriction, Kim's choice); jobs that send a prompt to Codex, Claude Code, GrokBuild, Ollama or llama.cpp, based on `/home/kim/projects/concilium/functions/functions_providers.py`; OpenRouter left out on purpose (would need a TLS crate). Plan approved. Backup first: `backup/rust_template_pre_command_ai_jobs.zip` (verified).
- Crates: none added. `reqwest` is listed in `Cargo.lock` (through Tauri) but was never downloaded or compiled here, so it would have been new; instead a small HTTP/1.1 client (Content-Length, chunked, plain http only) on Tokio's TCP. Tokio gained the features `process`, `io-util`, `net` (the lock file only gained a dependency line to `signal-hook-registry`, already present). `libc` (already in the tree) listed directly for `killpg`.
- `jobs.rs`: `JobCtx::output` (job output, newest 256 KiB kept, `output_cut` flag, not sent in window events; `jobs_inspect` returns it), `Queue::output_tail`, `JobKind::capabilities(input)` and `resources(input)` (defaults: the constants), input limit 16 to 64 KiB.
- `job_kinds.rs`: `CommandJob` (`/bin/bash -c`, `cmd /C` on Windows; folder any, empty = home, `~` expanded; own process group, killed as a group on kill, time limit or exit; stdout and stderr streamed to the output, last line as the live note; labels `exit_N` or `signal`). `AiJob` (providers `claude_code`, `codex`, `grok_build` with Concilium's read-only options, PATH extended with ~/.local/bin, ~/.npm-global/bin, ~/.grok/bin, ~/.bun/bin and nvm's newest Node, Claude Code session variables removed; `ollama`, `llama_cpp` over local HTTP, `gpu` resource; labels `answered`, `unavailable`, `error`, `empty`, `not_started`; never starts a server). Count kind moved to `test_kind`, test builds only.
- Scheduler: history entries keep the last 400 bytes of output; `jobs_state.json` limit raised to 4 MiB. `jobs.default.json`: command and AI demos, all off.
- Page: new `ui/js/job_forms.js` (fields per kind, JSON box for kinds without a form), used by the Queue view's "Run a job now" form and the schedule editor. Queue table: kind name per job, live note or failure message instead of a count, "Late by" removed, compact buttons, Waited hidden under 52rem. Inspect: Output box with Copy (same clipboard fallback as the Errors page). History: Output column.
- Tests: 98 pass (new: command output, exit code labels, folder, home, missing folder; kill stops a grandchild process (mutation check: with `killpg` disabled the test fails; source restored with `cmp`); time limit; output keeps the newest part; URL and response parsing incl. chunked; Ollama and llama.cpp against a fake local server, request body checked, `gpu` resource; unreachable server = `unavailable`; stand-in `claude`, `codex`, `grok` scripts check the read-only options and prompt delivery without calling a real AI). Live: `live_ollama` with Kim's `qwen3:1.7b` (already loaded, about 1.5 GB VRAM) answered "green".
- Hidden screen (`schedules_page`, 10 assertions, and `jobs_page`, by eye): in the real app a failing script retried, a rule started a follow-up script, a second rule asked Ollama, and the answer ("blue") was kept in the history; the Queue view ran `df -h /`, a script failing with exit code 3 ("exit code 3: broke"), and an Ollama prompt (answer shown in the row and in Inspect's Output). Layout faults found by eye and fixed: sideways scroll in the queue table (long live note), "At once" label wrapping. isolation, smoke, settings_files, errors_badge: 26 of 26. All run against `dist/rust-scaffold`.
- Built with `build.sh`: `dist/rust-scaffold` 6,862,800 bytes; `dist/rust-scaffold.previous` is the schedules build from earlier today.
- Not tested: real Claude Code, Codex and GrokBuild calls (stand-ins only; they spend tokens), llama.cpp for real (fake server only), Windows, Kim's desktop, maximised, other themes. `/tmp/sched_test/` (the earlier test copy) is no longer needed and can be deleted.

### 15 November 2026: old count demos removed from Kim's settings, model box per AI (Claude)
- Kim's `~/.config/rust_scaffold/jobs.json` still held the three count demos from the morning's build, so the app warned "unknown job kind count" three times. It held nothing else (no rules; `jobs_state.json` empty). Copied to `backup/kim_jobs_json_old_count_demos_2026-11-15.json`, then `jobs.json` and `jobs_state.json` deleted (settings files, allowed while in development); the app recreates them from the new defaults (command and AI demos, all off). Kim confirmed the count demos are not wanted.
- Bug found by Kim: switching the AI from Ollama to Claude Code kept "qwen3:1.7b" in the model box, and Claude Code refused it. `job_forms.js` now keeps a model per AI while the form is open; switching shows that AI's own (empty at first). Hidden-screen check `jobs_page` switches and screenshots it: the box empties for llama.cpp and shows qwen3:1.7b again on switching back (checked by eye).
- Leftover mentions of counting removed from a `jobs.js` comment and `docs/usage.md`. The test-only `count` kind stays (test builds only).
- Built with `build.sh`; `jobs_page` and `schedules_page` pass against `dist/`.

### 15 November 2026: version 0.2.5, pipelines, Ollama model list (Claude)
- Kim approved: pass a job's result to the job a rule starts, with tests; an Ollama model list (Kim: minicpm-v and qwen2vl are image models, not LLMs); version 0.2.5. Not yet: "how to" guides and forking new apps (Kim: the scaffold is not ready). Backup first: `backup/rust_template_pre_0.2.5_pipelines.zip` (verified).
- `jobs.rs`: `Previous` (schedule, job, state, label, output) and `PreviousView` (the same without the output, in `JobView.previous`); `Queue::start_after`; `JobCtx::previous`; `tail_start` made `pub(crate)`.
- `scheduler.rs`: when a rule fires, the finished job's output (up to 256 KiB) is read once and given to every job the rule starts; kept on the entry so a retry gets it again; none for Run now and timetable starts.
- `job_kinds.rs`: commands get `PREVIOUS_OUTPUT` (last 100 KiB, NUL bytes removed), `PREVIOUS_OUTPUT_FILE` (whole output, temporary file), `PREVIOUS_LABEL`, `PREVIOUS_STATE`, `PREVIOUS_SCHEDULE`, `PREVIOUS_JOB`, as environment variables only, never pasted into the script (Kim was told why: AI output in a script would run as code). AI prompts: `{previous_output}` (last 12,000 bytes, to stay inside a small local model's 4,096-token context), `{previous_label}`, `{previous_state}`, `{previous_schedule}`; empty when there is no previous job; the output is filled last so placeholder-like text inside it stays. `ollama_models` (`/api/tags`, then `/api/show` per model for `capabilities`; vision models flagged; checked first by hand on Kim's server: qwen2vl and minicpm-v report "vision", qwen3 and cot do not) and command `ollama_models`. HTTP client now does GET as well as POST.
- Page: Ollama model box is a list of installed text models with sizes (vision models left out), "Other" for a typed name, falls back to the text box when Ollama does not answer; notes under the command and AI fields explain the variables and placeholders; Inspect shows "Given the result of". Help change log entry for 0.2.5; `Cargo.toml` 0.2.5, `cargo update --workspace --offline`.
- Tests: 103 pass (new: a command started by a rule gets the previous result in its environment, and nothing when started by a person; text with `$(...)` and backticks in the previous output is passed as text and never runs (marker file absent); an AI prompt gets the previous output with placeholders filled, a failed attempt is retried with the same input (fake Ollama answering HTTP 500 then 200); prompt filling and the 12,000-byte cut and NUL removal; model list with vision flags, smallest first, and "not answering"). Mutation check: with the rule handing `None` instead of the result, all three pipeline tests fail; source restored with `cmp`.
- Real app, new check `pipeline` (4 assertions, against `dist/`): log lines -> qwen3:1.7b counted the ERROR lines and answered "3" (correct) -> a script saved exactly that answer to a file; `df -h /` -> qwen3:1.7b answered "It is 82% full." (the real `df` said 82%). Other checks against `dist/`: isolation, smoke, settings_files, errors_badge 26 of 26; schedules_page 10 of 10; jobs_page by eye (both commands, the Ollama job, the model list showing "qwen3:1.7b (1.4 GB)", switching AI and back, Inspect). `jobs_page` was made robust: it submits with Enter in a text box instead of clicking Run now, whose position moves when the form changes (a run had silently skipped the command jobs).
- Built with `build.sh`: `dist/rust-scaffold` (0.2.5); `dist/rust-scaffold.previous` is the earlier build of today.
- Not tested: the full model list on screen with every entry (the list order and vision filter are checked in a unit test; on screen the select showed qwen3:1.7b because it is the selected example model, not because it is first: cot and tool_calling, 1.1 GB each, are smaller), Claude Code, Codex and GrokBuild in a pipeline (stand-ins only), very long outputs through a real model, Kim's desktop, maximised, other themes.


### 15 November 2026: Jobs framework advisory (Claude)
- Kim asked whether the Jobs system is good and what to add, refine, trim or tweak for unknown future apps (finance, monitoring, evaluators, media catalogues, security, children's games, cooking timers). Written `docs/jobs_advisory.md`; nothing built, no source changed, no build or tests run.
- Main points: commands can only report `exit_N`, so rules cannot react to what a job found (proposal: `$JOB_RESULT` file, value and "changed" rule conditions); no "did not happen" rule; no notify kind; no relative second-accurate timers; no `wait` API for app code; interrupted runs not recorded; output not kept past memory. Tiered: build next, build when needed, keep parked, trim, tweak.
- Checked for it: `cargo tree -i zbus` shows zbus 5.19.0 already in the tree through ksni. Unchecked and marked so in the document: scheduler timing after suspend, second accuracy in the real app, clock-change behaviour.

### 15 November 2026: job results, value and "changed" rules, notify kind, AI reactions, errors kept, README (Claude)
- Kim approved items 1, 2 and 4 of `docs/jobs_advisory.md`, asked that a job's outcome can start an AI reacting to it, that errors are still listed after a restart with DEBUG_MODE false, and a README section on Jobs and why it beats cron and Task Scheduler. Backup first: `backup/rust_template_pre_results_notify.zip` (verified; excludes target, backup, .git, dist).
- Command results: `$JOB_RESULT` (JSON label, value, message; each part replaces the exit code's; the exit code still decides done or failed; invalid file = failed `bad_result`; printing JSON changes nothing) and `$JOB_PROGRESS` (`40/100`, `40%`, a count; read every 500 ms and once at the end). Each command now has its own temporary folder; the folder name gained a process-wide counter after parallel tests (several queues, same job ids) overwrote each other's files.
- Rules: `value` test (`field`, dotted for nested, `above`/`below` strict, `equals`) and `changed` (output plus value differ from the last successful run; fingerprint 64-bit FNV-1a, written by hand because `DefaultHasher` may change between Rust versions; kept in `jobs_state.json` as `last_fingerprint`; needs `ended` done or any). History entries keep the value. Rule editor on the page has the fields and a switch.
- Pipelines: `Previous` carries value and message. Scripts get `PREVIOUS_VALUE` (JSON) and `PREVIOUS_MESSAGE`. Prompts and notifications: `fill_text`, one pass, placeholders `{previous_output|label|value|value.FIELD|message|state|schedule}`; unknown ones left as written. This is how an AI reacts to an outcome.
- `notify` kind: title, text, urgency, sound; `zbus` 5 listed directly on Linux behind the default feature `desktop-notifications` (`Cargo.lock` gained only a dependency line, no new crate). Test builds never use the real session bus (a nonexistent address unless a test sets its private bus), so tests cannot show notifications on Kim's desktop. Labels `shown`, `unavailable`.
- Errors kept: `errors.jsonl` (see Logging above); entry ids now start from the time in microseconds so a later run's ids are always higher.
- Demos in `jobs.default.json` (all off): `demo_disk_check` reports `{"percent": N}`; `demo_disk_full` (over 90, twice) starts `demo_disk_advice` (Ollama qwen3:1.7b); `demo_advice_ready` starts `demo_tell_advice` (notification); `demo_watch_hosts` with `demo_hosts_changed` (changed) starts `demo_tell_changed`. `demo_follow_up` and `demo_after_failure` removed. Kim's `~/.config/rust_scaffold/jobs.json` was byte-identical to the old default and `jobs_state.json` empty, so both were deleted to be recreated with the new demos.
- Found while writing the notify tests: a dbus-daemon config with only `<allow send_destination="*"/><allow own="*"/>` hung every client (gdbus and dbus-send too); the config from `xvfb_check.sh` works. Cause not investigated.
- Tests: 114 pass, no warnings (new: command reports its result, bad result files, progress, placeholders, notify through a stand-in service on a private bus, notify without a service and without a test bus, value rule N in a row, changed rule incl. a failed run and a restart, refused rule definitions, value test comparisons, fingerprint stability). Mutation check: value test ignored, `changed` always true and `$JOB_RESULT` ignored at once: the three matching tests fail; sources restored with `cmp`.
- Hidden screen against `dist/`: new `results_notify` 7 of 7 (values 50, 95, 97 kept in history; the rule fired once after two over 90; the stand-in service received "Disk 97% full" / "The disk is 97% full." critical with sound; qwen3:1.7b answered "The disk is 97% full, and immediate action is required to prevent data loss." and it arrived as a notification; errors.jsonl kept the first run's warning and the second run listed it greyed, checked by eye). isolation, smoke, settings_files, errors_badge 26 of 26 (badge 5: four oversized files plus the tray warning on the test bus; three entries from earlier runs greyed and not counted; check comment updated from 4). schedules_page 11 of 11 (new: a value rule saved through the form), jobs_page by eye (notify form, the notify job failed as unavailable on the test bus as it should), pipeline 4 of 4.
- README: new section "Jobs: schedules, rules and reactions" with the comparison to cron and Task Scheduler (including their advantage: running while no app is open). Noticed, not changed: README's Settings section still says files are copied from the program folder (removed in 0.2.4), and Known limitations still says Linux tray icons get no clicks (fixed in 0.2.4).
- Built with `build.sh` into `dist/` (6,963,200 bytes; no source newer). `dist/rust-scaffold.previous` was set back to the 0.2.5 build from before this work (an intermediate build had replaced it). Version unchanged (0.2.5); Help change log entry extended. Final checks against it: smoke and results_notify 12 of 12.
- Not tested: a real desktop notification on Kim's LXQt (only the stand-in service; whether her notification daemon plays a sound or keeps critical ones on screen), Windows, the `changed` and value rules on screen beyond the form, Kim's desktop, maximised, other themes.

