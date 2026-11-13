## Rust Scaffold: boilerplate for Kim's future Rust apps

User: Kim.
Version: 0.2.2
Date: 13 November 2026
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
- Never commit, stage, branch, push or pull. Kim manages git.

---

## Where things are

Source lives in two folders: Rust in `src-tauri/src/`, the interface in `ui/` (embedded into the binary at compile time, so any HTML, CSS or JS change needs a rebuild).

Root:
- `build.sh`: release build (`cargo build --release --locked -j 4` at low priority), copies the binary to `dist/`.
- `run.sh` / `run.bat`: start `dist/rust-scaffold` detached (`run.bat --build` builds first on Windows, untested).
- `dist/rust-scaffold`: the app Kim runs.
- `README.md`, `requirements.txt`, `docs/usage.md`: user and developer documentation. `docs/` also holds the GPU, GRUB recovery, performance and failure write-ups.
- `backup/`: zips taken before each milestone.

Rust (`src-tauri/`):
- `Cargo.toml`, `Cargo.lock`, `tauri.conf.json` (window, version, CSP), `capabilities/default.json` (window permissions).
- `build.rs`: records build time, rustc and cargo versions and the crate list for the Help page.
- `src/lib.rs`: start-up order, `settings_dir()`, `DEBUG_MODE`, `APP_FOLDER`, `DIR_OVERRIDE`, command list.
- `src/commands.rs`: every command the interface can call.
- `src/config.rs`: `config.json` load, validate, atomic save, 5 MB limit.
- `src/options.rs`, `src/search.rs`, `src/files.rs`: `options.json` and `search.json`, shared read-or-create code.
- `src/logging.rs`: tracing layer, Errors list, `debug.log`.
- `src/jobs.rs`: the Tokio job queue.
- `src/tray.rs`, `src/gpu.rs`: tray icon; GPU rendering switch for WebKitGTK on NVIDIA.
- `options.default.json`, `search.default.json`: copied into the settings folder when missing.
- `benchmark/`: benchmark scripts and raw results (see below).

Interface (`ui/`):
- `index.html`: all pages. `js/`: one script per feature (`app.js` loads config and shows the window, `options.js`, `jobs.js`, `errors.js` which loads first to catch script errors, `search.js`, `sortable.js`, `tabs_fit.js`, `theme.js`, `help.js`, chart and table scripts). `css/`: modules plus `variables.css` (theme tokens) and `bootstrap-flat.css` (switches off Bootstrap's shadows without editing the vendor file).

---

## Settings and log files: the `dirs` crate

The app uses the `dirs` crate to find the per-user configuration folder, so it never has to guess, and works when installed somewhere read-only such as `C:\Program Files`:

- Windows: `%APPDATA%\rust_scaffold` (Roaming, per user).
- Linux: `~/.config/rust_scaffold`.

`config.json`, `options.json`, `search.json` and `debug.log` live there. On first start the three JSON files are copied once from the executable's folder, if present. The environment variable `RUST_SCAFFOLD_DIR` overrides the folder; tests, benchmarks and experiments use it so Kim's real settings are never touched. If `dirs` finds nothing, the executable's folder is used. When making a new app, change `APP_FOLDER` and `DIR_OVERRIDE` in `lib.rs`.

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
- Tests: `cd src-tauri && nice -n 10 cargo test --release -j 4` (44 tests: config, options, search, logging, job queue). Job tests use Tokio's paused clock, so a 100-step job is tested in milliseconds.
- Version numbers live in `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`. After a version change run `cargo update --workspace --offline` (because `build.sh` uses `--locked`) and add a line to the change log on the Help page.

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
6. Smaller lessons: a test that moves the real mouse is ruined if Kim touches it; poll for the window instead of sleeping; diff config files after any aborted experiment; a late reply can overwrite newer events (fixed in the Jobs page with `seq`).

---

## Current state and next work

- Done in 0.2.2: Rust 1.100.0, Tauri 2.12.5, new crates (tokio, regex, time, dirs, tracing), per-user settings folder, Errors page, Tokio job queue and Jobs page.
- Now: adding crates, tweaking and refining what exists.
- Later: with GPU rendering working, low-cost ("lo-fi") shadows may be tried again. Rule: one restrained shadow on one component type, built outside `dist/`, measured with `cpu.py` and by Kim by hand, maximised, before keeping it.
- Leftovers: view all 14 themes on the newer pages; a fallback timer that shows the window if `app.js` fails; test on Windows; unused `dist/*.json` copies from 0.2.1 can be deleted; `src-tauri/target` grows past 2 GB and can be deleted safely (next build takes about 2 minutes).

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
