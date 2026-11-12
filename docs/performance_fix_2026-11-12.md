# Maximised window responsiveness: diagnosis and fix

Date: 12 November 2026  
Platform tested: Lubuntu LXQt, X11, WebKitGTK, Tauri 2, 1920 x 1028 maximised window.

## Result

The release app is responsive when maximised after disabling CSS shadows. In a repeated scripted tab-switch check, the original page took about 0.6 to 1.2 seconds before the changed content appeared in a screenshot. The fixed app took about 0.09 to 0.10 seconds in Slate, Aero Blue and Quartz. Kim watched the app being operated and reported a huge visible speed difference.

This is a repair of the observed maximised tab delay, not a claim that every interaction on every machine has been benchmarked.

## What established the cause

The earlier blank-page variant used the same Tauri window settings and was fast when maximised. That showed that the page mattered, but did not identify the costly rule. This investigation used a second release build of the full page with visual effects disabled, followed by a diagnostic build that could switch effect groups at runtime. Each variant was tested on the normal X11 desktop. A sandboxed GUI run reported that GTK could not create a GL context, so its timing was discarded.

A release executable already existed before this work. Its timestamp was later than the CSS files, so the project had been compiled. That did not establish that the maximised case had been tested. This investigation then compiled the project again and tested the new executable.

For each tab switch, `xdotool` clicked Dashboard or Panels. A loop captured a one-pixel screenshot until its colour changed to the target page's colour. The values include screenshot polling overhead, about 90 ms at the fastest. They are measurements of visible change under scripted input, not physical mouse latency. Each group below used four alternating tab switches in the same maximised 1920 x 1028 window with the Slate theme:

| CSS mode | Visible-change times |
| --- | --- |
| Original effects | 1203, 592, 981, 614 ms |
| Shadows and text shadows disabled | 94, 94, 95, 96 ms |
| Blur and filters disabled | 1195, 778, 603, 795 ms |
| Transitions and animations disabled | 950, 783, 594, 797 ms |
| Shadows plus motion disabled | 97, 99, 100, 106 ms |
| Shadows, blur and motion disabled | 99, 97, 91, 96 ms |

An earlier flat-style build that disabled all these groups measured 90 to 128 ms. The decisive result is the shadow-only mode: it kept the full Bootstrap CSS and JavaScript, the page structure, the gradient, transparency, tabs and the other effects, yet became fast. The size of the Bootstrap files and the unused CSS rules were excessive, but they were not the demonstrated cause of the repeated tab repaint delay. The particular shadow declaration that contributes most has not been isolated. The page uses shadows on wide cards, the tab content panel and other controls. The maximised frame's shadow is already disabled by `.maximized .app-window`.

## Changes applied

- Added `ui/css/performance.css` as the final CSS module. It disables `box-shadow` and `text-shadow` globally. It also disables `backdrop-filter` globally so the Baby Pink and Quartz glass effects cannot add a separate large-area paint cost. Borders and the existing keyboard focus outline remain.
- Removed unused template modules from `ui/css/index.css`: forms, dialogs, icons, containers, lists and the Bootstrap bridge. Together they account for about 19 KB of CSS that WebKit no longer loads through `index.css`. Their files were preserved. The full local Bootstrap CSS, Bootstrap JavaScript and Bootstrap Icons are still loaded because this change aimed to fix the measured repaint cost without replacing the app's controls. Removing those core assets would need a separate functional review.
- The original source and settings were archived first at `backup/rust_template_pre_codex.zip`. The archive passed ZIP integrity verification. The test changed theme and accordion settings through the UI; `config.json` was restored byte-for-byte from that archive afterward.

The global shadow rule uses `!important` intentionally because both Bootstrap and template components set shadows in many selectors. Future components will also be flat by default. If a shadow is reintroduced, measure it maximised before keeping it.

## Verification of the actual project

- `cargo build --release` completed successfully using the existing `tools` compiler group. The CSS is embedded in the release binary, so a rebuild was required.
- `run.sh`, Kim's normal launch path, opened the rebuilt 640 x 480 window successfully after the release build.
- The rebuilt app measured 94 to 97 ms per scripted tab switch in Slate, 93 to 100 ms in Aero Blue, and 95 to 99 ms in Quartz, all maximised. These values are at the screenshot polling floor, so the actual paint may be quicker.
- The Rust command button returned the app name, version and project config path. Dashboard, Panels and Tables displayed; accordion collapse and the theme picker worked. Slate, Aero Blue, Baby Pink and Quartz were visually checked.
- All 11 Rust tests passed under the release profile.
- Kim observed a large improvement while the rebuilt app was being operated. A physical mouse timing measurement was not made.

## Limits and next checks

The diagnostic tests isolated shadows as a group. They did not profile WebKit's individual paint operations, and they did not test every theme, drag and drop, scrolling or Windows. The screenshot method has about 90 ms of measurement overhead. If any theme still feels slow in normal use, test it maximised and inspect its specific effects before changing the window or compositor. The old failure postmortems remain a record of the state before this fix; their verdicts should not be read as the current measured result.

One launcher caveat remains: `run.sh` starts the previous executable if its build fails and prints a warning. In the tool sandbox, `sg tools` could not switch groups, so that fallback occurred. The normal desktop launch check succeeded after the new binary had compiled. If a future change appears ineffective, check the build output before judging the window.
