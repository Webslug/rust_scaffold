# Rust Scaffold: usage guide

How to run, change and reuse the scaffold. For installing the tools, see
README.md in the project root.

## Run

    ./run.sh          Linux
    run.bat           Windows

`run.sh` launches `dist/rust-scaffold` (run `./build.sh` first, or use
`./run.sh --build`). It starts the app detached from the terminal with its
output discarded and returns the prompt at once (set `DEBUG_MODE` to get a log
file instead, see below). `run.bat` does the same on Windows: it launches
`dist\rust-scaffold.exe`, and `run.bat --build` first builds it with cargo and
copies it to `dist\` (stopping without launching if the build fails). Not yet
tested on Windows.

Settings live beside the executable, not in the working directory:
`config.json`, `options.json` and `debug.log` are read from and written to the
folder that holds the binary (normally `dist/`). Starting the app from a
terminal, a file manager or a shortcut makes no difference, and a symlink to
the binary uses the real file's folder. To move the app, move the binary
together with its two JSON files.

The UI in `ui/` is embedded in the binary at compile time. After editing any
HTML, CSS or JavaScript, run the script again (an incremental rebuild takes
about a minute with the size-tuned release profile) or run
`cargo build --release` inside `src-tauri`.

## Build and test by hand

    ./build.sh                 build release executable from the project root

The script prints the executable path and exits with an error if the build
fails. It handles the local `tools` compiler group in older login sessions.
It does not launch the app or fall back to an earlier build.

    cd src-tauri
    cargo build --release       binary: src-tauri/target/release/rust-scaffold
    cargo test --release        unit tests for the config rules

The release profile in `src-tauri/Cargo.toml` is tuned for size (`opt-level = "s"`,
LTO, one codegen unit, stripped, `panic = "abort"`). It makes the binary about
4.3 MB instead of 13.3 MB. A full rebuild takes about two minutes and an
incremental rebuild about a minute (about 25 seconds without LTO).
Delete `src-tauri/target` to reclaim about
1.3 GB of build files at any time; it is rebuilt on the next run.

## Configuration file

`config.json` is created on first run and rewritten whenever you change a
setting in the app. You can also edit it by hand while the app is closed.

| Key | Default | Meaning |
|-----|---------|---------|
| `theme` | `"aero-blue"` | Theme id, a lowercase slug (see `ui/js/theme.js`) |
| `width`, `height` | `640`, `480` | Window size, clamped to 200..10000 |
| `show_titlebar` | `true` | `false` hides the whole title bar |
| `rounded_corners` | `true` | `false` gives a square window |
| `order` | `{}` | Saved item order per sortable list |
| `collapsed` | `[]` | Ids of collapsed accordion panels |
| `start_minimised` | `false` | Start hidden in the tray |
| `tray_items` | `[]` | Extra tray menu entries |
| `menu_layout` | `"vertical"` | `"vertical"` (menu down the left) or `"horizontal"` (tabs across the top) |
| `menu_collapsed` | `false` | Vertical menu contracted to icons |
| `titlebar_scale` | `100` | Title bar size in percent: `100`, `150` or `200` |
| `text_size` | `100` | Text size in percent: `100`, `115` or `130` |
| `high_contrast` | `false` | Full-strength text and borders, solid surfaces |
| `large_targets` | `false` | Taller buttons, menu items and fields, bigger checkboxes |
| `underline_links` | `false` | Underline every link |
| `strong_focus` | `false` | Thicker keyboard focus ring |
| `reduce_motion` | `false` | No transitions or animations, whatever the system setting |
| `hardware_acceleration` | `true` | Draw the page on the graphics card where supported (NVIDIA on Linux needs kernel mode setting, see `docs/gpu_rendering.md`). Applies at the next start |
| `extras` | `{}` | Settings defined only in `options.json` (see Options tab) |

Rules enforced by `src-tauri/src/config.rs`:

- A file larger than 5 MiB (5,242,880 bytes) is not read.
- Invalid JSON or invalid values load the defaults, and saving is switched off
  for that session so the file is never overwritten. The window shows a
  warning banner.
- Missing keys fall back to their defaults, so old files keep working.
- Ids are lowercase letters, digits and hyphens, at most 40 characters. Limits:
  20 sortable lists, 50 items per list, 100 collapsed panels, 10 tray items,
  tray labels up to 40 characters.
- `extras`: at most 100 keys. A key is lowercase letters, digits and
  underscores, at most 40 characters. A value is `true`/`false`, a number, or
  text of at most 4000 characters (line breaks and tabs allowed, no other
  control characters). Lists, objects and `null` are rejected.
- `menu_layout`, `titlebar_scale` and `text_size` accept only the values
  listed above. Any other value falls back to the default instead of
  rejecting the whole file.
- Saves are atomic (a temporary file is written, then renamed).

## Options tab

The Options tab edits settings inside the app. Which controls it shows, and
how, comes from `options.json` beside the executable, not from the code:

- `config.json` holds the values.
- `options.json` describes the controls: a label on each shelf, not the
  contents. Changing a control's type or label never touches the saved value.

`options.json` is read every time the Options tab opens. Edit it, switch to
another tab and back, and the change appears. No rebuild and no restart. If the
file is missing, the app writes the default copy
(`src-tauri/options.default.json`). A file over 1 MiB is not read. Invalid JSON
shows a warning and no controls; the file is never overwritten.

Changes are staged. **Save** writes only the values you changed. **Revert**
discards unsaved changes. Opening the tab again also discards them. Theme, title
bar and corner changes apply as soon as they are saved; `start_minimised`
applies at the next start.

### File format

    {
      "options": [
        { "group": "Examples", "control": "switch", "key": "autosave",
          "label": "Autosave", "default": true }
      ]
    }

Other top-level keys (for example `"_readme"`) and unknown fields in an entry
are ignored, so you can leave notes in the file.

| Field | Used by | Meaning |
|-------|---------|---------|
| `control` | all | `switch`, `check`, `radio`, `input`, `text`, `number`, `range`, `list`, `select`, `button` |
| `key` | all but `button` | The setting it edits |
| `label` | all | Visible label, 1 to 80 characters |
| `group` | all | Card heading; entries with the same group share a card. Default "Options" |
| `help` | all | Small text under the control, up to 300 characters |
| `readonly` | all | `true` shows the value but does not allow changes |
| `default` | extra settings | Value shown until the setting is saved for the first time |
| `size` | text, number, select, list, range | Width: `sm`, `md`, `lg` or `full` |
| `rows` | `text` | Height in lines, 1 to 20 |
| `placeholder` | `input`, `text` | Hint shown in an empty box |
| `choices` | `radio`, `list`, `select` | List of text values, or `{"value": ..., "label": "..."}` objects. `"themes"` means every theme (theme key only) |
| `min`, `max`, `step` | `number`, `range` | Limits; a range needs `min` and `max` |
| `action` | `button` | `revert` (discard changes) or `defaults` (set every entry that has a `default` back to it; press Save to keep) |

What each control stores:

| Control | Looks like | Value |
|---------|-----------|-------|
| `switch` | Bootstrap switch | `true` or `false` |
| `check` | Checkbox | `true` or `false` |
| `input` | One-line text box | text |
| `text` | Multi-line text box | text |
| `number` | Number box | number |
| `range` | Slider with its value beside it | number |
| `radio` | Radio buttons | the chosen choice's value |
| `select` | Drop-down list | the chosen choice's value |
| `list` | Bootstrap list group, one item active | the chosen choice's value |
| `button` | Button | nothing; runs its `action` |

### Built-in settings and extra settings

If `key` is one of the app's own settings, the control edits that setting. Only
these are allowed, with these controls:

| Key | Controls |
|-----|----------|
| `theme` | `select`, `radio`, `list` (use `"choices": "themes"`) |
| `width`, `height` | `number`, `range` |
| `show_titlebar`, `rounded_corners`, `start_minimised`, `menu_collapsed`, `high_contrast`, `large_targets`, `underline_links`, `strong_focus`, `reduce_motion`, `hardware_acceleration` | `switch`, `check` |
| `menu_layout`, `titlebar_scale`, `text_size` | `select`, `radio`, `list` (choices must use the allowed values; numbers for the two sizes) |

`order`, `collapsed`, `tray_items` and `extras` are managed by the app and
cannot be options. Built-in settings take their defaults from Rust, so a
`default` on them is rejected.

Any other key is an extra setting, stored under `extras` in `config.json`.
Adding one needs no Rust change: add the entry to `options.json`, reopen the
tab, change the value and press Save. Your own code reads it from
`config.extras` in JavaScript or `config.extras` in Rust (a
`serde_json::Value`).

### When an entry is wrong

Rust checks every entry (`src-tauri/src/options.rs`). A bad entry is skipped and
listed in a warning at the top of the tab, with its position and the reason,
for example `entry 2 "oops": unknown control "teleport"`. The other entries
still load. Rules include: known control and key format, no duplicate keys,
choices present and unique for choice controls, `min` below `max`, a
`default` that suits the control and lies within its limits and choices, at
most 200 entries and 50 choices each.

If a saved value no longer suits its control (for example a choice was removed
from `options.json`), the control shows its `default` instead, and the old
value stays in `config.json` until you save that control.

## Window controls

- Drag the title bar, or the empty background of the content area, to move
  the window. Drag any edge or corner to resize.
- Title bar buttons: hide the title bar (T), theme picker, minimise,
  maximise or restore, close.
- With the title bar hidden, a small button in the top right corner brings it
  back.

## Menu layout and search

`menu_layout` (Options tab, Appearance) chooses the navigation:

- **Horizontal**: Bootstrap tabs above the page. When the labels do
  not fit, the tabs show icons only (see "Add a tab page"). Help sits at the
  far right.
- **Vertical** (default): a menu down the left side with larger text that is never
  hidden by resizing. The button at the top left contracts the menu to a strip
  of icons with tooltips and expands it again; the state is saved in
  `menu_collapsed`. Help sits at the bottom.

Both layouts use the same tab list (`#app-tabs`), restyled by
`ui/css/navigation.css` when `<html data-menu="vertical">` is set. A new tab
appears in both without extra work.

### Search

The vertical menu has a search box (Ctrl+K moves to it; in the contracted menu
the magnifying glass expands the menu first). After three characters it lists
up to eight matching places. Up and Down choose, Enter goes, Escape closes the
list and a second Escape clears the box. Every word typed must match the
label, a keyword or the page name; labels that start with the text rank first.

Going to a result selects its tab, opens a closed accordion panel that holds
the target, scrolls the target into view, focuses it (or the card or row
around it) and outlines it for two seconds. Pages that draw their content
after being shown (the Options tab, marked `data-async-render`) send a
`page:rendered` event; the search waits for it, at most two seconds.

The places come from `search.json`, beside `config.json`. It is created from
`src-tauri/search.default.json` when missing and re-read each time the search
box gets focus, so edits need no rebuild or restart. Every tab is also found
by its own name without an entry.

    { "label": "Theme", "page": "options", "target": "opt-theme",
      "keywords": ["colour", "color", "palette"], "icon": "palette" }

| Field | Required | Meaning |
|-------|----------|---------|
| `label` | yes | Text in the list, 1 to 80 characters |
| `page` | yes | Tab id without `tab-`: `dashboard` means `#tab-dashboard` and `#page-dashboard` |
| `target` | no | Id of an element on that page. Options tab controls are `opt-` plus the key, for example `opt-text_size` |
| `keywords` | no | Up to 20 extra words or phrases, each up to 40 characters |
| `icon` | no | Bootstrap Icons name without `bi-` |

Rust (`src-tauri/src/search.rs`) skips a bad entry with a reason and keeps the
rest: limits are 1 MiB, 500 entries, and ids made of letters, digits, hyphens
and underscores. The page also skips entries whose page or target does not
exist. Skipped entries are listed on the Help page, which also
shows how many places were loaded.

## Title bar size and accessibility

All of these are on the Options tab and take effect when you press Save. They
are applied before the window first appears, so nothing flashes.

| Setting | Effect |
|---------|--------|
| Title bar size | Multiplies the bar's height, text, icons and buttons by 1, 1.5 or 2 (`--titlebar-scale`). Long titles shorten with an ellipsis |
| Text size | Sets the root font size to 100, 115 or 130 percent; everything sized in rem follows, the menu and title bar included |
| High contrast | Muted text, borders and the window edge use the full text colour; the glass themes (Aero Blue, Baby Pink, Quartz) get solid surfaces |
| Larger click targets | Buttons, tabs, menu items, fields and search results at least 2.75rem tall; bigger checkboxes and switches |
| Underline links | Underlines `<a>` links that are not buttons, tabs or menu items. The scaffold itself has almost no such links; this is for apps built on it |
| Strong focus ring | Keyboard focus outline 5 px instead of 3 px |
| Reduce motion | Every transition and animation off, whatever the system setting |

The rules are in `ui/css/a11y-prefs.css` (classes and attributes on `<html>`,
set by `ui/js/app.js`) and the tokens in `ui/css/variables.css`. The system
settings `prefers-contrast` and `prefers-reduced-motion` still work on their
own, through `accessibility.css`.

## Help

The last tab. It explains how to use the app, holds a short change log, names
the developer (Kim Watts) and shows the version, the settings file and the
search library status from Rust. Edit the text in `ui/index.html`
(`#page-help`); styles are in `ui/css/help.css`, live values in
`ui/js/help.js`.

## Tray icon

The tray icon is created at startup and stays while the window is visible,
minimised or hidden. Menu: Show / Hide first, your `tray_items`, Quit last.

    "tray_items": [
      { "label": "Hide to tray", "action": "hide" },
      { "label": "Show window",  "action": "show" }
    ]

Allowed actions are `show`, `hide`, `toggle` and `quit`. Anything else is
rejected, so the config file can never run commands. "Show / Hide" minimises a
visible window and restores a minimised or hidden one.

With `"start_minimised": true` the window stays hidden at launch. If the tray
cannot be created the window opens anyway, so the app is never unreachable.

On Linux, Tauri does not deliver tray click or double-click events, so there is
no double-click toggle; use the menu. On Windows a double-click toggles the
window.

## Change things

### Add a Rust command

1. Write the function in `src-tauri/src/commands.rs`:

       #[tauri::command]
       pub fn hello(name: String) -> String {
           format!("Hello, {name}")
       }

2. Register it in `src-tauri/src/lib.rs` inside `tauri::generate_handler![ ... ]`.
3. Call it from JavaScript:

       window.__TAURI__.core.invoke("hello", { name: "Kim" }).then(console.log);

### Add a tab page

Add a tab button and a matching pane in `ui/index.html`, copying the Dashboard
pair: the button needs `data-bs-target="#page-name"`, the pane needs
`id="page-name"`. Give every tab an icon and put its text in
`<span class="tab-label">`:

    <i class="bi bi-sliders"></i><span class="tab-label">Options</span>

Help stays last, in `<li class="nav-item nav-item-help">`. Add new
tabs before it. To make a page findable by its parts, add entries to
`search.json` (the tab itself is found by name automatically).

When the labels no longer fit on one row (between 760 and 860 px wide in Aero
Blue with the seven tabs here; the point moves with the number of tabs, the
text size and the theme's font),
`ui/js/tabs_fit.js` hides the labels and shows icons only, each with a Bootstrap
tooltip giving its name on hover or keyboard focus. Hidden labels are still read
by screen readers. If even the icons do not fit, the row scrolls sideways with a
slim scrollbar, and the selected tab is scrolled into view. Any tab row with the
`data-tabs-fit` attribute gets this behaviour; it needs no other setup.

### Add a sortable list

Wrap the items in an element with `data-sortable="list-id"` and give each item
`class="sortable-item"`, a unique `data-sortable-id`, and a grip
(`<span class="sort-handle" role="button" tabindex="0" aria-label="...">`). The
page picks up every `[data-sortable]` list automatically and saves the order in
`config.json` under `order`. Dragging uses pointer events, so it also works
with touch. Keyboard: focus the grip, then Up, Down, Home or End.
Add a reset button with `data-reset-sortable="list-id"` to restore only that
list's default order. The Cards tab is a complete example; its order is saved
under `order["showcase-cards"]`. The Dashboard and Panels reset buttons are
also scoped to their own lists.

### Search the mock tables

Each search field on the Tables tab filters only the table below it. Terms are
case-insensitive and can match different columns in one row. The visible
match count and a no-results row update immediately. Search text is not saved
in `config.json`. The markup is in `ui/index.html`, filtering logic is in
`ui/js/table_search.js`, and theme-aware styling is in `ui/css/tables.css`.

### Chart examples

The Charts tab shows fictional sales, sign-up, retention and customer-segment
data. Each chart's search and filter affect only that chart. The Table button
shows the same filtered values as a table. For pie and doughnut charts, shares
are recalculated from the visible groups. None of the chart controls are saved
to `config.json`.

The renderer in `ui/js/charts.js` and `ui/css/charts.css` was copied from the
local Bootstrap template. It uses SVG and no network or JavaScript package.
Mock data and filter rules live in `ui/js/charts_page.js`; the layout and
controls are in `ui/index.html` and `ui/css/charts-page.css`. Add a new data
entry and chart card there to extend the examples. Palette and background
tokens live in `ui/css/variables.css`. Do not give charts or their cards
shadows (see "No shadows or blur" below).

### Add an accordion panel

Copy a panel from the Panels tab in `ui/index.html`. The collapsing element
needs `data-panel-id="panel-id"` and the `show` class. Collapsing it adds its id
to `collapsed` in `config.json`.

### Add or change a theme

1. Add a `[data-theme="my-theme"]` block to `ui/css/variables.css` (copy an
   existing one) and its window gradient tokens `--window-bg-top` and
   `--window-bg-bottom` at the end of the same file.
2. Add `{ id: "my-theme", label: "My Theme" }` to the list in `ui/js/theme.js`.

### Style rules used here

No inline `style=""` attributes (the content security policy would block them).
Add a class to a CSS file instead. Colours, spacing and radii come from the
tokens in `ui/css/variables.css`. App-specific rules live in `ui/css/window.css`.

### No shadows or blur

WebKitGTK on Linux paints the page on the CPU, and a shadow or blur makes every
repaint of the elements under it more expensive. In a maximised window this is
felt directly: tab switches, scrolling and clicks lag. Measured history on the
test machine (i7-6700, RTX 3060, LXQt/X11 with picom):

- The template's layered bevel shadows made maximised tab switches 6 to 12
  times slower (see [the performance report](performance_fix_2026-11-12.md)).
- On 13 November 2026 a single small shadow (`0 1px 3px`, 12% opacity) on
  cards, accordion panels and buttons, light themes only, made the light
  themes noticeably slow when maximised by hand. It was removed.

So the stylesheets contain no `box-shadow`, `text-shadow`, `backdrop-filter` or
`filter: drop-shadow()`. Show depth with borders, background tints and
spacing; show keyboard focus with outlines (`ui/css/accessibility.css`).
Bootstrap's own shadows (focus rings, buttons, cards, dropdowns, modals,
offcanvas panels, toasts, popovers and table cells) are switched off in
`ui/css/bootstrap-flat.css`, without editing `bootstrap.min.css`. Table
striping and hover are plain backgrounds in `ui/css/tables.css`, because
Bootstrap draws them with a shadow on every cell.

If you want to try an effect anyway: build it to a separate folder (not with
`build.sh`, which replaces `dist/rust-scaffold`), compare it with the current
build using `src-tauri/benchmark/cpu.py` in the target theme, and check it by
hand with the window maximised before keeping it.

## Use as a template for a new project

Copy the folder, delete `src-tauri/target` and `dist/` (which holds the built
binary, `config.json`, `options.json` and `debug.log`), then
rename:

| File | What to change |
|------|----------------|
| `src-tauri/Cargo.toml` | package `name`, and the library `name` (keep the `_lib` suffix) |
| `src-tauri/src/main.rs` | the library name in `rust_scaffold_lib::run()` |
| `src-tauri/tauri.conf.json` | `productName`, `identifier`, window `title` |
| `build.sh`, `run.sh`, `run.bat` | the binary name in `BIN` and the `.exe` path |
| `ui/index.html` | `<title>` and the title bar text |
| `src-tauri/icons/` | replace the icons; the tray uses the default window icon |

Then run `./run.sh`. Do not copy `target/`.

## Debug mode

`DEBUG_MODE` is a constant at the top of `src-tauri/src/lib.rs`, `false` by
default. Set it to `true` and rebuild to write program events (startup,
config load, save and refusal, size restore, tray actions) to `debug.log` beside the
executable. With `false` no log file is written.

## Troubleshooting

- **No window appears.** The window starts hidden and is shown by
  `ui/js/app.js` once the saved theme is applied. A JavaScript error before
  that point leaves a running but invisible process. Check each script for
  syntax errors with `node --check ui/js/app.js` (and the other files), or
  revert the last script edit.
- **Corners are black on X11.** No compositor is running. Start picom, or use
  `run.sh`, which starts it for you if it is installed.
- **Build fails with "could not exec the linker cc".** Install the C toolchain
  (`build-essential` on Debian and Ubuntu).
- **Tray icon is missing.** Install `libayatana-appindicator3-1` and make sure
  the desktop panel has a system tray.
- **config.json warning banner.** The file was too large, invalid or
  unreadable. Fix or delete it and restart; until then settings are not saved.

## Known limitations

- Shadows and blur make the app slow when maximised, so there are none (see
  "No shadows or blur"). Drag and resize performance has not been remeasured
  since they were removed.
- Linux tray icons have no click or double-click events (see above).
- Transparent windows need a compositor on X11.
- Windows support is written but has not been tested.
- The expanded vertical menu is 14.5rem wide (more at larger text sizes) and
  does not contract by itself in a narrow window; at the 400 px minimum width
  the page is cramped until you contract it.
