# Rust Scaffold performance investigation

Date: 12 November 2026  
Status: Unresolved. This is a read-only code review and assessment of earlier measurements, not a new runtime benchmark.

## Finding

The app's display path is the strongest suspect for the slow tab switches, menu scrolling and button response, especially when the window is maximised. The exact bottleneck has **not** been established. No maximised latency measurement or controlled comparison with a blank page exists, so it would be inaccurate to name one CSS effect, WebKitGTK, or the compositor as the proven cause.

The Rust work is small and is not on the tab-switch path. The three tabs use Bootstrap's normal show/hide behaviour. Clicking a tab does not call a Rust command, read the config file or save it. Slow tab changes therefore point toward input delivery, layout, painting or compositing after the click. This conclusion comes from inspection of `ui/index.html`, `ui/js/app.js`, `ui/js/window.js` and `ui/js/sortable.js`.

## Why maximising matters

The normal window is 640 x 480, about 307,000 pixels. The reported maximised area is 1920 x 1028, about 1,974,000 pixels, or 6.4 times as large. The page has a transparent native window containing a full-window rounded, clipped gradient frame (`ui/css/window.css`). Its content uses nested cards, shadows and some theme-dependent translucent or blurred surfaces. A large area could make repainting much more expensive. Pixel count alone does not prove that the whole window is repainted on each action or that painting scales linearly.

Earlier scripted measurements in `AGENTS.md` and `docs/project_failure_handoff_guide.md` found substantial WebKit web-process CPU during resizing and about 0.13 to 0.2 seconds of web-process CPU per tab switch at smaller sizes. They did not measure the maximised state or the several-second delay Kim experiences with a real mouse. Those measurements support investigation of rendering but cannot explain the full perceived delay by themselves.

## Specific code findings

- `ui/js/window.js` calls Tauri's asynchronous `isMaximized()` on every browser `resize` event, then changes the root class and button icon. That is unnecessary work during a continuous resize and may add to maximise latency. It does not account for a slow tab click once resizing has stopped.
- `ui/js/app.js` debounces saving window size for 500 ms and calls `isMaximized()` before saving. This affects resize activity, not ordinary tab switching.
- `ui/js/sortable.js` installs its document-level pointer-move listener only while a card or panel is being dragged. It is not a standing listener that would explain ordinary scrolling or tab clicks.
- `ui/css/window.css` gives the entire app a rounded, clipped outer frame and scrollable inner content. `ui/css/cards.css`, `tabs.css`, `accordion.css` and other modules add shadows; Aero Blue adds blur to some components. These are plausible paint costs, but no maximised comparison has isolated any one of them.
- The tab content blur is explicitly disabled by `ui/css/window.css`. Earlier Aero Blue versus Slate CPU comparisons did not show a meaningful difference, so the blur alone has weak support as the principal cause.

## Launch method and comparison app

Kim observed that launching the binary detached from the terminal partly improved responsiveness. `run.sh` now uses `nohup` with output redirected to `/dev/null`, matching the way Claude launched its tests. The reason for that improvement was not established. It did not resolve the maximised slowdown.

The water tracker is responsive on the same machine and Tauri/WebKitGTK stack, but it has a fixed 360 x 540 window and cannot be maximised. Its Tauri configuration sets `transparent: false`; picom clips its corners. It is therefore not a like-for-like test of a large transparent window. Its good performance does show that small Tauri windows can feel responsive here.

Earlier experiments at smaller sizes found no decisive improvement from changing the window's transparency alone, the outer frame's shadows, the gradient, process priority, CPU affinity, or several WebKit and graphics environment settings. Those results do not rule out the same factors in combination at maximised size. Component shadows, large-window clipping, translucent layers and differences between real and scripted pointer input remain untested there.

## Conclusion and limit

The evidence supports a rendering or input-path problem in this particular large window. It does **not** establish that Rust or Tauri is inherently too slow, that the machine is at fault, or that this project must be abandoned. It also does not prove a specific CSS rule is responsible. The blank-page variant already built under `/tmp/variants/` is the cheapest discriminating test: if its maximised tabs are slow, investigate the large transparent WebKit window; if they are fast, isolate the scaffold's CSS and page structure. Kim declined further testing for this report, so that distinction remains open.

No source code, launcher, configuration or runtime process was changed for this investigation.
