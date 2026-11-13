<p align="center">
  <img src="ui/img/app.jpg" alt="Concept illustration of the Rust Scaffold desktop app: a frosted glass window with a sidebar, form controls, a colour palette and a code card, under the Rust logo" width="720">
</p>

<h1 align="center">Rust Scaffold</h1>

<p align="center">
  A lean, reusable desktop application scaffold built with Rust and Tauri 2.<br>
  Frameless, transparent, rounded window with 14 switchable themes, a tray icon and a saved settings file.
</p>

---

## What it is

A starting point for small desktop apps. It is application-agnostic: copy the
folder, rename it, and replace the example content. It uses plain HTML, CSS and
JavaScript for the interface (no React, no Vite, no Node.js) and a small Rust
core. The release binary is about 4.3 MB, well under the 50 MB target.

## Features

- Frameless window with rounded corners and real transparency, resizable, with
  a custom title bar (drag to move, minimise, maximise and close buttons).
- Hide or show the title bar. Optional square corners.
- 14 themes, switched at runtime, each with its own subtle background gradient
  and bevelled buttons and containers. The theme and window size are saved.
- Tabs, searchable mock tables, sortable cards and expandable accordion panels.
  Drag-and-drop reordering works with mouse, pen and touch, and with the
  keyboard. The Cards tab has its own saved order and reset button. The order
  and collapsed panels are saved; table searches are local to each table.
- Offline SVG examples of line, stacked bar, doughnut and pie charts. Each chart
  has its own search, filter and data-table view; colors follow the selected theme.
- A tray icon with a context menu you can extend from the settings file, and an
  optional start-minimised mode.
- A settings file (`config.json`) in the project folder. It is created on first
  run, is never read if it is larger than 5 MiB, and is never overwritten when
  it cannot be read.
- An Options tab that edits settings in the app. Its controls (switches,
  checkboxes, radio buttons, text boxes, number boxes, sliders, list groups,
  selects and buttons) are described in `options.json`, which you can edit
  without rebuilding. New settings need no Rust changes.
- Example of the interface calling a Rust command.
- Bootstrap 5 and Bootstrap Icons, bundled locally. The app works offline.

## Requirements

Rust 1.77.2 or newer and the Tauri system libraries. You do not need Node.js,
npm or the Tauri CLI. The full list is in [requirements.txt](requirements.txt).

## Install Rust and Tauri

### Linux (Debian, Ubuntu, Lubuntu)

Tested on Ubuntu 22.04 with LXQt on X11.

1. Install the build tools and Tauri's system libraries:

       sudo apt update
       sudo apt install build-essential pkg-config curl wget file \
         libwebkit2gtk-4.1-dev libgtk-3-dev libssl-dev librsvg2-dev \
         libayatana-appindicator3-dev

   The official Tauri guide also lists `libxdo-dev`. This project built and
   ran without it. Other distributions: see https://tauri.app/start/prerequisites/

2. Install Rust with rustup:

       curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

   Then restart the terminal, or run `. "$HOME/.cargo/env"`.

3. Check:

       rustc --version
       cargo --version

4. Optional: install a compositor for the transparent rounded corners on X11
   (`sudo apt install picom`). Without one the corners render black. Wayland
   compositors handle transparency themselves.

### Windows 10 and 11

Not tested on this project. These are the standard Tauri 2 steps.

1. Install the Microsoft C++ Build Tools from
   https://visualstudio.microsoft.com/visual-cpp-build-tools/ and select the
   "Desktop development with C++" workload.
2. WebView2 is preinstalled on Windows 11 and on current Windows 10. If it is
   missing, install the Evergreen runtime from
   https://developer.microsoft.com/microsoft-edge/webview2/
3. Install Rust from https://rustup.rs (download and run `rustup-init.exe`).
   Keep the default `x86_64-pc-windows-msvc` toolchain.
4. Open a new terminal and check:

       rustc --version
       cargo --version

## Build and run

    ./build.sh        Linux: build the release executable only
    ./run.sh          Linux
    run.bat           Windows

`build.sh` reports build failures and never launches an older executable.
`run.sh` builds the app if anything changed, then starts it from the project
folder. The first build downloads and compiles the dependencies and takes
a few minutes (about two on the test machine). Later builds are incremental
and take about a minute.

Or by hand:

    cd src-tauri
    cargo build --release
    ./target/release/rust-scaffold      (Linux, run from the project folder)

Run it from the project folder so `config.json` is created there.

## Settings

`config.json` is created next to `run.sh` on first run. The app writes your
theme, window size, title bar state, panel order and collapsed panels to it. You
can also edit it by hand while the app is closed.

    {
      "theme": "aero-blue",
      "width": 640,
      "height": 480,
      "show_titlebar": true,
      "rounded_corners": true,
      "order": {},
      "collapsed": [],
      "start_minimised": false,
      "tray_items": [],
      "extras": {}
    }

`options.json`, also created on first run, decides which settings appear on
the Options tab and how. Every key is documented in [docs/usage.md](docs/usage.md).

## Project layout

    build.sh              build release executable only
    run.sh, run.bat       build and launch
    requirements.txt      system requirements
    config.json           created at first run
    options.json          Options tab controls, created at first run
    docs/usage.md         how to use, change and reuse the scaffold
    ui/                   the interface (embedded in the binary at build time)
      index.html
      css/                Bootstrap, the theme tokens and window.css
      js/                 window controls, themes, sortable lists, app logic
      fonts/, img/
    src-tauri/            the Rust side
      src/                main.rs, lib.rs, config.rs, options.rs, commands.rs, tray.rs
      benchmark/          repeatable X11 desktop tests and saved results
      options.default.json  written as options.json when that file is missing
      tauri.conf.json, capabilities/, icons/, Cargo.toml

## Using it as a template

Copy the folder (without `src-tauri/target`), then rename the package, the
library, the product name, the identifier and the window title. The exact files
and fields are listed in [docs/usage.md](docs/usage.md).

## Known limitations

- **Rendering.** Layered CSS shadows made the app slow when maximised on the
  test machine. They are now disabled; maximised tab switching was measured
  at about 0.1 seconds under scripted input. See
  [the performance report](docs/performance_fix_2026-11-12.md) for the method
  and remaining test limits. Drag, resize and Windows performance have not
  been remeasured.
- **Linux tray icons** do not receive click or double-click events from Tauri,
  so use the tray menu. On Windows a double-click toggles the window.
- **Transparency** needs a compositor on X11.
- **Windows** support is written but has not been tested.

## Credits

Built with [Tauri](https://tauri.app), [Rust](https://www.rust-lang.org),
[Bootstrap](https://getbootstrap.com) and
[Bootstrap Icons](https://icons.getbootstrap.com).
