# Project failure handoff guide: Rust Scaffold (Tauri 2) redraw latency

Date: 12 November 2026
From: Claude Code (Opus 5.5 and Sonnet 5.5 sessions)
To: Codex
Owner: Kim
Project folder: `/opt/templates/rust`

## 1. Verdict and the one question

Kim has called this project a failure and is handing it to Codex. In Kim's words:
the app is "just not viable when the window is maximized", and "we're not expecting
our app to load fast, but we're expecting it not to take 3 seconds to load a tab when
the window is maximized". Kim knows Rust and Tauri are fast, and the Rust side of
this app uses almost no CPU, so the problem is somewhere else.

**The question to answer:** why is repaint and redraw latency so bad in this app,
especially when the window is maximised (tab switches take seconds, buttons do not
respond instantly), when `/home/kim/projects/water_tracker`, built on the same
Tauri 2 and WebKitGTK stack on the same machine, is snappy?

Kim has said the GPU and its driver are not the cause. Do not spend time there.

## 2. State of the project

The functional side is complete and documented. Only the redraw speed is unresolved.

What exists:

- A frameless, transparent, rounded, resizable Tauri 2 window, 640x480 default
  (minimum 400x300, no maximum), with a custom title bar (drag, minimise,
  maximise, close), a theme picker (14 themes, each with its own background
  gradient) and a button that hides the whole title bar.
- Three Bootstrap tab pages: Dashboard (three drag-to-reorder cards), Panels (three
  drag-to-reorder accordion panels, expanded by default, collapsed state saved) and
  Tables (three styled mock-data tables).
- `config.json` in the project folder: theme, window size, title bar visibility,
  rounded corners on or off, card and panel order, collapsed panels,
  `start_minimised`, `tray_items`. Never read if over 5 MiB, never overwritten if it
  cannot be parsed.
- A tray icon (Tauri built-in) with a Show / Hide, custom items and Quit menu.
- Pointer-event drag and drop (`ui/js/sortable.js`), saved through Rust.
- 11 Rust unit tests (`cd src-tauri && cargo test --release`), all passing.
- Size-tuned release profile: binary 4.3 MB (13.3 MB before tuning).
- `README.md`, `requirements.txt`, `docs/usage.md`, `docs/PLAN.md`, and a running
  engineering log in `AGENTS.md` (read its "KNOWN ISSUE" sections first).

Stack: plain HTML, CSS and JavaScript (no React, no Vite, no Node at build or run
time), Bootstrap 5 and Bootstrap Icons bundled locally, Rust core. Builds with plain
`cargo`; the UI in `ui/` is embedded in the binary at compile time, so any UI edit
needs a rebuild.

## 3. The symptoms, in the order Kim reported them

1. Early, at the default size: "the app doesn't feel snappy and responsive. It feels
   like there is latency when you drag the window or resize it."
2. Later: "Performance right now is abysmal" and "scrolling menus, selecting tabs,
   dragging and dropping all seem slow".
3. Kim watched Claude drive the same binary with scripted mouse input and said it
   looked snappy with "almost no latency". Kim's own use via `./run.sh` was slow.
4. Kim: "Selecting tabs, drag and dropping elements is painfully slow taking 2-3
   seconds to move things sometimes, it takes at least 1 second to select a tab."
5. Setting `WEBKIT_DISABLE_DMABUF_RENDERER=1` or `WEBKIT_DISABLE_COMPOSITING_MODE=1`
   before `./run.sh`: "marginally better, still too slow."
6. `run.sh` was changed to start the app detached from the terminal
   (`nohup ... >/dev/null 2>&1 &`), the way Claude had been starting it in its
   tests. Kim: "that partly fixes the latency issues, it's certainly more faster but
   when maximized the latency issues return with a vengeance. ... selecting tabs
   takes a few seconds when maximized, buttons don't respond instantly."
7. Final: not viable maximised.

Kim also states the water tracker "is snappy, the buttons click instantly" and "uses
transparency". (Its `tauri.conf.json` actually says `"transparent": false`; its
rounded corners are cut by the picom compositor. See section 6.)

## 4. How to reproduce

A human is needed. Scripted input (xdotool) never reproduced the slowness.

1. `cd /opt/templates/rust && ./run.sh` (builds if needed, then starts the app
   detached).
2. Click the maximise button in the title bar (or maximise with the window manager).
3. Click between the Dashboard, Panels and Tables tabs, click buttons, try dragging a
   card by its grip. Kim reports delays of several seconds.

Claude never measured or tested the maximised state for latency. Phase 2 only
checked that maximise and restore work.

## 5. Environment (verified on 12 November 2026)

- Ubuntu 22.04.5 (Lubuntu, LXQt), X11 session, Openbox window manager.
- Display: one monitor, 1920x1080 at 60 Hz. A maximised window is 1920x1028 (a 52 px
  panel at the bottom).
- Compositor: picom v9, started as `picom --backend glx`, reading
  `~/.config/picom/picom.conf`. That file sets `corner-radius = 18` and shadows
  (radius 20, opacity 0.35) only for a window titled "Water Tracker", `fading = false`.
  Our app is not matched by any of those rules.
- CPU: Intel i7-6700 (8 threads), governor `powersave` (`intel_pstate` active), 64 GB
  RAM, RTX 3060.
- Rust 1.98.0, Cargo 1.98.0, Tauri 2.11.5, wry 0.55.1, tao 0.35.3, tray-icon 0.24.2,
  WebKitGTK 2.50.4 (`webkit2gtk-4.1`), GTK 3.24.33.
- The C compiler is restricted to the `tools` group (`/usr/bin/gcc-11` is
  `root:tools 750`). Kim was added to the group but shells started earlier do not
  have it yet. Build with `sg tools -c "cd src-tauri && cargo build --release"` in
  such a shell. `run.sh` does this automatically. This is a machine policy, not a
  bug, so do not work around it by pointing cargo at another compiler.
- Kim's shells and Claude's shell have the same nice level, scheduling class, CPU
  affinity and cgroup (`session-c2.scope`) and no CPU or memory limits.

## 6. Water tracker versus this app (the comparison that matters)

Both are Tauri 2 apps on the same machine, same WebKitGTK, same picom, same
`bootstrap.min.css`.

| | Water tracker (snappy) | Rust Scaffold (slow) |
|---|---|---|
| Path | `/home/kim/projects/water_tracker` | `/opt/templates/rust` |
| Window size | fixed 360x540 | 640x480 default, resizable, maximisable |
| `resizable` | false | true |
| `transparent` | false (corners cut by picom) | true |
| `decorations`, `shadow` | false, false | false, false |
| Pixels painted maximised | cannot be maximised | about 1,974,000 (1920x1028) |
| Frontend | React 19, Vite 7, `aero.css` | plain JS, about 20 CSS modules, Bootstrap bundle JS |
| Components | one card, a progress bar, two buttons | tabs, cards, accordions, tables, dropdown, tray |
| `backdrop-filter` | none | on some themes (see section 8) |
| Binary | prebuilt, 15 MB | 4.3 MB, size-tuned |

The water tracker has never been tested maximised because it cannot be. A decisive
test is to run its page, or a trivial page, in a window with our settings and
maximise it (section 10, steps 1 and 2).

Arithmetic worth noting: 1920x1028 is about 6.4 times the area of 640x480 and about
10 times the area of 360x540.

## 7. What was measured (all by Claude, with caveats)

Every number below was taken on the detached launch (`nohup`, no terminal) with
scripted xdotool input, not with Kim's hand on a real mouse, and none were taken
maximised. "Ticks" are 10 ms units of CPU time (100 ticks is one core for one
second), read from `/proc/<pid>/stat`.

- Startup to a visible window: 0.6 to 1.0 s. Idle: effectively 0 CPU. Memory:
  106 to 114 MB.
- Rust process during a scripted corner resize: about 16 ticks. The WebKit web
  process (`WebKitWebProcess`): about 200 ticks for an 80-step resize, and 257 to 288
  ticks in a later run (window grown to 880x640).
- Nine scripted tab clicks: 118 to 184 ticks of web process CPU, so about 0.13 to
  0.2 s of CPU per tab switch, on a trivial show and hide.
- Scripted window drag (100 moves): web process about 15 ticks, picom 48 to 62,
  Xorg 59 to 87. With `WEBKIT_DISABLE_COMPOSITING_MODE=1`, picom fell to 14 and Xorg
  to 37.
- Tested with no effect on web process CPU during a resize: `transparent: false`
  (not combined with resizable false), removing the window frame's box-shadows,
  replacing the background gradient with a solid colour, `LIBGL_ALWAYS_SOFTWARE=1`,
  `GSK_RENDERER=cairo`, `WEBKIT_DISABLE_DMABUF_RENDERER=1`,
  `WEBKIT_DISABLE_COMPOSITING_MODE=1`.
- Aero Blue (which applies `backdrop-filter: blur(14px)` to cards, accordion items and
  alerts) versus Slate (no blur): the same tab and resize CPU, so the blur is not the
  cause.
- A hover-storm test (sweep the pointer at about 200 Hz to imitate a hand) was written
  but never run, because Kim interrupted it.

## 8. Ruled out and not ruled out

Ruled out by the measurements above, within their limits: Rust code speed, process
priority, CPU affinity, cgroup limits, environment variable differences, the template's
`backdrop-filter` blur, window transparency by itself at the default size, the window
frame's shadows, the background gradient, the size-tuned release profile as a cause of
the original complaint (the early complaints came before it was applied; it was never
compared maximised).

Not ruled out, and the most interesting:

- Pixel-area scaling of repaint cost (the maximised report strongly suggests it). Kim
  reported 640x480 as better and maximised as bad; the water tracker is small. This is
  a hypothesis only. It was never tested.
- CPU rasterisation inside WebKitGTK, where every repaint costs in proportion to the
  pixels repainted. The web process sits near one core during resizes and none of the
  GPU related WebKit environment switches changed the CPU figure.
- Component shadows (buttons, cards, tabs, accordion items, badges, the table
  wrapper). Only the window frame's shadows were removed in earlier tests.
- Translucent `rgba` surfaces layered over the gradient, and a full-window element
  with `overflow: hidden` plus `border-radius` clipping everything inside it.
- The transparent (ARGB) window at large sizes, in combination with picom.
- Why launching detached (no terminal, output to `/dev/null`) helped. Kim confirms it
  did. The reason was never established.
- JavaScript and IPC on resize: `ui/js/window.js` calls `appWindow.isMaximized()`
  (an IPC round trip) on every `resize` event, and `ui/js/app.js` debounces a
  `save_config` after resizing. This cannot explain slow tab clicks but may add to a
  slow maximise.
- Real mouse input being much denser than scripted input.

## 9. How the investigation went wrong (candid)

Written so Codex does not repeat it.

1. **Claude's test launches did not match Kim's.** Claude started the finished binary
   directly in the background with output discarded, no terminal and no build step.
   Kim ran `run.sh`, which built through `sg` and then ran the app in the foreground
   attached to her terminal. Claude's "it is snappy" observations therefore did not
   represent what Kim experienced. It was Kim, not Claude, who spotted that the two
   were launched differently. Detaching the launch in `run.sh` helped partly, so the
   launch method was a real factor.
2. **Claude measured the wrong thing.** It measured CPU ticks and watched scripted
   input, not human-perceived latency, and recorded "fast when Claude drives" as a
   mystery instead of treating Kim's experience as the ground truth.
3. **The maximised case was never measured or tested** until Kim reported it.
4. **Several test runs took over Kim's real mouse and screen** while Kim was using the
   machine, and two tool runs were interrupted by Kim. A few runs were also spoiled by
   Kim's own movement or by Claude's wrong click coordinates.
5. About six hours were spent in total. Much of the CSS and window polish was built
   before performance was checked, and performance was deferred twice.

## 10. Suggested plan for Codex (cheapest and most decisive first)

All of this is unverified suggestion, not established fact.

0. **Measure what Kim feels, not CPU.** Instrument the page: log `requestAnimationFrame`
   gaps, long tasks (`PerformanceObserver` with `longtask`) and the time from
   `pointerdown` to the next frame. Write the log through Rust `debug_log` (set
   `DEBUG_MODE = true` in `src-tauri/src/lib.rs`, it writes `debug.log` in the
   project folder). Have Kim use the app with a real mouse, maximised and not. This
   shows whether the seconds are spent in JavaScript, layout, paint or compositing.
1. **Blank page in our window settings.** Same `tauri.conf.json`, same tray and
   Rust, but a trivial page: a rounded gradient frame, a draggable title bar and
   three plain tab buttons, no Bootstrap. Maximise it. If it is also slow, the cost
   is the window and WebKitGTK on this machine and not our page. If it is fast, our
   CSS is the cost. Claude built this variant as `/tmp/variants/bin/blank` (source in
   `/tmp/variants/src/blank`), but `/tmp` is cleared on reboot and Kim never tried it.
2. **Water tracker page maximised.** Copy `/home/kim/projects/water_tracker`, set
   `"resizable": true`, rebuild, maximise. If it is also slow maximised, the problem
   is not our page. Its `start.sh` comments show the build command
   (`cd tauri-app && npm run tauri build -- --no-bundle`).
3. **Latency against window area.** Run our app at 360x540, 640x480, 1280x720 and
   maximised and measure the same interaction at each size. A straight line against
   pixel count would confirm CPU painting.
4. **Window variants.** Claude also built `opaque` (`transparent: false`), `fixed`
   (`resizable: false`) and `both` in `/tmp/variants/bin`, from `tauri.conf.json`
   edits only. Kim never tried them. Try each maximised where possible (`fixed` and
   `both` cannot be maximised).
5. **WebKitGTK rendering settings.** The hardware acceleration policy can be set from
   Rust through the webview handle (`with_webview` gives the underlying
   `webkit2gtk` objects). Check the WebKitGTK 2.50 environment switches for the
   Skia painter against the WebKit source (names were not verified). Kim felt
   `WEBKIT_DISABLE_DMABUF_RENDERER=1` and `WEBKIT_DISABLE_COMPOSITING_MODE=1` as
   marginally better.
6. **Flat CSS variant.** Add `*, *::before, *::after { box-shadow: none !important;
   text-shadow: none !important; transition: none !important; }` and measure again.
   Then remove the gradient, translucent surfaces and the clipping on `.app-window`
   one at a time (`ui/css/window.css`, `ui/css/variables.css`).
7. **Input realism.** Scripted xdotool events may not match a real mouse. A
   kernel-level input injector (`uinput` based) would be closer. Kim must agree first,
   because it moves the real pointer.
8. **Compositor.** The water tracker uses the same picom, so this is a lower
   priority. If tried: `--vsync`, `--backend xrender`, or no compositor.

Fallbacks if the area cost cannot be removed: set `maxWidth` and `maxHeight` in
`tauri.conf.json` and remove the maximise button (the scaffold then suits small
widget-style apps like the water tracker), or ship an opaque window like the water
tracker's.

## 11. Working agreements with Kim (her standing instructions, relevant here)

- Kim is a developer with 20 years of experience, pedantic about precision. Lead with
  the answer or fix. Skip preambles. State assumptions and ask at most one question.
- For large work: give a numbered plan first, then do one step at a time, and do not
  write code before the plan is approved.
- Be direct about poor code and weak design, with the reason and a better option.
  Before abandoning work or doing anything irreversible or expensive, state the
  cheapest test that would show whether it is needed.
- Do not run Ollama or local heavy models, and avoid heavy background processes,
  without asking first. Kim may be doing audio work. Use Zenity for any sudo prompt.
- **Ask before running tests that move the real mouse or take over the screen.**
  Kim has interrupted such runs several times. Say clearly when hands must stay off.
- Never touch git (no commits, branches, staging or pushes) unless Kim asks. Do not
  mention git workflow unprompted.
- Python, PHP and C# scripts need a `DEBUG_MODE = False` boolean. When true, log
  events, failures and large writes; when false, keep output minimal and write no
  logs.
- No inline `style=""` in HTML. Use CSS files, modular by control type, with CSS
  variables for colours, spacing and radii. Use Bootstrap and Bootstrap Icons. Do not
  edit vendor files.
- Markdown goes in `docs/` inside a project (README.md in the root was requested
  explicitly). No em dashes in public-facing documents.
- Editor is FeatherPad, never nano. Never suggest `cat` commands to copy and paste;
  write script files instead.
- Kim has said she is tired and frustrated about this project. Keep replies short and
  plain, and sympathise before problem solving.

## 12. Where things are

- `AGENTS.md`: the full engineering log, newest at the bottom. The latency sections are
  "KNOWN ISSUE: visual redraw latency", its "UPDATE" about the maximised window, and
  "run.sh now launches the app detached".
- `docs/PLAN.md`: the phased plan, all phases done except the performance pass.
- `docs/usage.md`: how to run, configure, extend and reuse the scaffold.
- `src-tauri/src/`: `lib.rs` (setup, `DEBUG_MODE`, restores window size and centres),
  `config.rs` (config rules and tests), `commands.rs`, `tray.rs`.
- `src-tauri/tauri.conf.json`: window flags (`transparent`, `resizable`,
  `decorations`, `shadow`, `visible: false` until JavaScript shows the window) and the
  content security policy (no inline scripts or styles).
- `ui/index.html`, `ui/js/` (`app.js`, `window.js`, `theme.js`, `sortable.js`) and
  `ui/css/window.css` (frame, bevel, tabs, tables, accordion tweaks).
- `/tmp/variants/` (volatile): `bin/` (`baseline`, `opaque`, `fixed`, `both`, `blank`),
  `src/` (scratch copy plus `blank/`), `build.sh`, `try.sh <name>`. It builds with
  `CARGO_TARGET_DIR=/tmp/vt` so the real `target/` is not disturbed.

## 13. Commands

    ./run.sh                                          build if needed, start detached
    cd src-tauri && cargo build --release             binary: src-tauri/target/release/rust-scaffold
    cd src-tauri && cargo test --release              11 unit tests
    sg tools -c "cd src-tauri && cargo build --release"   if the shell lacks the tools group
    /tmp/variants/try.sh blank                        run a variant (if /tmp survived)

Run the binary from the project folder so `config.json` is read and written there.
Delete `config.json` to return to the defaults. Delete `src-tauri/target` to reclaim
about 1.3 GB.
