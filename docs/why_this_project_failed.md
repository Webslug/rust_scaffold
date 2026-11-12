# Why the Rust Scaffold failed

Update, 12 November 2026: The maximised tab delay was repaired by disabling CSS shadows. This document records the earlier failure. See [the performance fix](performance_fix_2026-11-12.md) for measurements and current status.

Date: 12 November 2026
Project: `/opt/templates/rust` (Rust, Tauri 2, WebKitGTK on Lubuntu/LXQt/X11)
Status: abandoned as a template.

## Verdict

The scaffold works at its default 640x480 size but is not usable when maximised. In Kim's words, maximised "it's like using a 486 DX 100 or a Pentium 1 processor". Selecting a tab, clicking a button or dragging a card takes from one to several seconds. A desktop template that behaves like that cannot be copied into future projects, so the project is a failure, whatever else it does well.

The cause is the page, not the platform. The page was built by copying the Bootstrap template from `/opt/templates/bootstrap/assets` almost wholesale, with all of its visual effects, and placing it inside a WebKitGTK window. Claude Code made that decision and never tested its cost in a maximised window.

## The test that settled it

A "blank" variant of the app was built earlier and finally tried by Kim at the end of the day. It used the identical window setup:

- the same Tauri configuration: transparent, frameless, resizable;
- a rounded frame clipping its content, a theme-like gradient background, a translucent panel;
- three tabs switched by a few lines of JavaScript;
- the same binary settings, compositor (picom) and launch method.

What it did not have: Bootstrap, Bootstrap Icons, the template stylesheets, web fonts, shadows, bevels and transitions.

Result: maximised (double-click on its title bar), the blank variant was fast and responsive. The real app, maximised, was slow.

Conclusion: the window type, transparency, rounded corners, the gradient, Rust, Tauri and the machine are not the cause. The cost lies in what the page asks WebKitGTK to paint.

## What was imported

The page is three tabs with a few cards, one accordion and three mock tables. To draw that, the app loads:

| Item | Size | Notes |
|---|---|---|
| `bootstrap.min.css` | 232 KB | Full Bootstrap 5, every component |
| `bootstrap.bundle.min.js` | 81 KB | Full Bootstrap JavaScript |
| Template stylesheets | 22 of the template's 32 files | Including dialogs, forms, lists, dividers, alerts and accessibility, most of them unused by the page |
| `variables.css` | 37 KB | All 14 themes, 42 theme blocks |
| All CSS together | 435 KB | For a three-tab page |

The size on disk is not the main problem. Unused rules mostly cost parse time at startup. The real damage is what the template does to every element it does style:

- **Layered shadows everywhere.** The template's bevel shadow (`--bevel-shadow`, several stacked insets) is applied 57 times across the stylesheets: buttons, cards, tabs, accordion items, badges, tables, alerts. Claude then added eight more inset shadows on the window frame for the bevel Kim asked for.
- **Translucent surfaces over a gradient.** Several themes draw cards and panels in semi-transparent colours over the window gradient, so each repaint must blend layers rather than fill flat colour.
- **Backdrop blur.** Aero Blue (Kim's theme) blurred the area behind every card, accordion item, alert, panel and dialog. Baby Pink and Quartz still do. The Aero Blue blur was removed on 12 November 2026; it made no noticeable difference on its own, which shows the cost is spread across the whole style stack rather than in one rule.
- **Transitions.** 21 transition declarations animate colour and shadow changes, so a click repaints several frames rather than one.

WebKitGTK on this machine appears to rasterise the page on the CPU. Every one of the effects above is paid for per pixel, and a maximised window has about 6.4 times the pixels of the default window (about 1,974,000 against 307,000). At 640x480 the cost is tolerable; maximised it is not. The CPU rasterisation point fits all the measurements but was not proven directly.

Not established: which single rule or group of rules costs the most. Finding that would mean removing groups of styles one at a time and testing maximised after each. Kim chose not to continue.

## What Kim asked for, and what Claude did

- `AGENTS.md` asked to "use the existing Bootstrap theme and template", "implement bootstrap into the project" from `/opt/templates/bootstrap/assets`, and to "keep dependencies and complexity to a minimum".
- Claude's own plan (`docs/PLAN.md`, key decision 7) said: "Copy only the Bootstrap assets used, not the whole template (fonts are the bulk)."
- In Phase 5 Kim wrote: "We don't want to see every bootstrap control but just some of them."

Claude did not follow its own plan or the spirit of Kim's instructions. It treated "only the assets used" as "leave out the template's extra fonts and scripts", copied 22 stylesheets and the full Bootstrap CSS and JavaScript, and kept every visual effect the template applies. The page then showed only some controls, as asked, but carried the full weight of the template behind them.

## How the investigation went wrong

The slowness was reported early (Phase 5, step 1) and repeatedly. These mistakes kept the real cause hidden for most of the day:

1. **Features were built on top of an open performance complaint.** Kim's first report of slowness came before the title bar toggle, sortable cards, accordions, tables and tray icon were built. Each feature added more styled elements. Performance should have been a gate: no new interface work until the base page was fast at every window size.
2. **The maximised case was never tested.** All of Claude's measurements were at 640x480 or similar. The failure case was the one size nobody measured.
3. **The wrong thing was measured.** Claude counted CPU ticks of the WebKit process during scripted xdotool drags. That measures work done, not the delay Kim feels between clicking and seeing the result. A tab switch "costing 0.15 to 0.2 s of CPU" does not refute a two-second visible delay.
4. **Claude's test runs did not match Kim's.** Claude launched the binary detached, with no terminal; `run.sh` ran it attached to Kim's terminal after a build step. Claude reported its runs as responsive without checking that they represented Kim's experience. Kim noticed the difference, not Claude. Detaching the launch helped somewhat, but did not fix the maximised case.
5. **Weak tests were reported as conclusions.** The Aero Blue blur was written up in `AGENTS.md` as "ruled out" on the basis of a CPU comparison at 640x480. Window transparency, the gradient and the frame shadows were "ruled out" the same way. None of those tests could detect a cost that grows with window size.
6. **Attention went to the wrong suspects.** Hours were spent on compositor settings, environment variables, GPU power states and launch methods. The cheapest discriminating test, a blank page in the same window, maximised, was built but not run until the very end. It took one minute and settled the question.
7. **Kim's own comparison was underused.** The water tracker on the same stack is snappy. Its page is small and plain. That pointed at the page from the start.

## Lessons

For any future Tauri (or other WebKitGTK) desktop app:

1. **Start from a blank page and test it maximised first.** Before any styling, confirm that a minimal page in the final window configuration is fast at full screen, launched exactly the way the user will launch it.
2. **Add styling in small steps and re-test maximised after each one.** A theme, a component type, a set of shadows: one at a time. Stop as soon as the feel degrades and find out why.
3. **Do not import a web template wholesale into a desktop window.** A template built for browsers on a GPU-accelerated page is not a free choice inside WebKitGTK. Copy the variables and the specific component rules the app uses, then strip expensive effects: stacked box-shadows, `backdrop-filter`, translucency over gradients and transitions.
4. **Prefer flat surfaces.** Solid colours and at most one simple shadow per element. A bevel can be a single 1 px border in two colours rather than several stacked inset shadows.
5. **Treat a performance complaint as a blocker, not a later task.** No further features until the complaint is resolved or explicitly accepted.
6. **Measure what the user feels.** Time from input to visible result (for example `requestAnimationFrame` timestamps after a click), under real mouse input, at the window size the user actually uses. CPU usage is a secondary signal.
7. **Never call something "ruled out" from a test that could not have detected it.** State the conditions of every test and what it cannot show.
8. **Test the user's launch path, not a convenient one.** If the developer's run is faster than the user's, that difference is the first thing to investigate.
9. **Run the cheapest discriminating test first.** A blank page, a single change, a one-minute comparison beats hours of measuring around the problem.

## What still works

None of these parts is involved in the redraw problem. They could be reused in a rebuilt, lightly styled scaffold:

- `src-tauri/src/config.rs`: `config.json` load and save with the 5 MB limit, validation, atomic writes, 11 unit tests.
- `src-tauri/src/tray.rs`: tray icon with a configurable menu (Linux has no click events; menu only).
- `ui/js/sortable.js`: pointer and keyboard drag-to-reorder with order saved to the config.
- `ui/js/window.js`: frameless window controls.
- `run.sh`: build through `sg tools` when needed, detached launch, picom start.

## Related documents

- `AGENTS.md`: the full dated engineering log, including the known-issue sections.
- `docs/project_failure_handoff_guide.md`: the handoff written for Codex.
- `docs/project_failure_postmortem.md` and `docs/performance_investigation_report.md`: Codex's independent reports.
- `/tmp/variants/`: the variant binaries, including `blank`. This folder is lost on reboot; copy `/tmp/variants/src/blank/` somewhere permanent if the blank page should be kept as a starting point.
