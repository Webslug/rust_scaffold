# Desktop performance benchmark

This toolkit measures Dashboard-to-Panels tab changes in the release app at
640 x 480 and maximised size. It is for the Lubuntu/LXQt X11 desktop. It adds
no runtime dependency to the application and does not change app code or your
project `config.json`.

## Requirements

Python 3.10 or newer, `xdotool`, ImageMagick's `import`, `xprop`, `xrandr`, and
the Rust build tools already used by `build.sh`. These tools were present on
the test machine. Nothing is installed by this toolkit. Run it in the normal
X11 desktop session with your usual compositor already running. The benchmark
does not start or reconfigure the compositor.

The benchmark opens the app, takes focus and moves the pointer for about a
minute. Leave the mouse and keyboard alone during a run. It restores the
previous active window and pointer position afterward. The app reads its
settings from the folder that holds the executable, so the benchmark copies the
release binary into a temporary folder together with a copy of your config
(`dist/config.json`) with a fixed theme, 640 x 480 starting size, visible title
bar, and default card order. Your config is checked for changes after the run.

## Commands

From the project root:

    python3 src-tauri/benchmark/bench.py run --label baseline
    python3 src-tauri/benchmark/bench.py run --label after_cards
    python3 src-tauri/benchmark/bench.py compare src-tauri/benchmark/results/BASELINE.json src-tauri/benchmark/results/AFTER.json

Replace the two example filenames with the paths printed by your runs. Labels
must be short lowercase names. The default is 20 measured switches per window
size, plus four warmups. You can use `--iterations 30` or `--theme aero-blue`,
but compare only runs with matching conditions. Results are dated JSON files
under `results/`. A failed run leaves a `.partial.json` file with any completed
scenario's raw data; incomplete results cannot be compared.

Run `build.sh` yourself if you only want to compile. The benchmark calls it
before every run and stops on any build error. It records the SHA-256 hash of
the executable as the build identifier. It does not call `run.sh`, because
that launcher can fall back to an older executable after a build failure and
may start picom. Instead it launches the verified release binary from the
temporary directory, detached with standard input and output discarded like
`run.sh`. This difference is recorded in the method.

## What the numbers mean

`xdotool` clicks a tab. ImageMagick repeatedly captures a single pixel in the
content until it matches the colour calibrated for the target page. The
reported duration runs from before the click command to the screenshot that
first sees the target colour. It includes command launch time, screenshot
polling and any delay between the first visible change and the next poll.
It does not measure physical mouse latency or full-page paint completion.

The tool saves every measured duration and poll count, eight raw screenshot
poll times, actual window geometry, theme, build hash, display mode, desktop,
compositor state and tool versions. It prints a median for a typical switch
and p90 for the slower end. Printed times are rounded to 10 ms; the JSON keeps
the raw measured values. The median duration of one screenshot poll is the
run's approximate measurement floor. Earlier manual polling took about 90 ms,
so values near that floor must not be interpreted as precise input latency.

The comparison reports absolute and percentage changes in median and the p90
change. It flags incompatible themes, geometry, display or tool conditions,
unusually variable slow results, and differences smaller than a conservative
threshold based on polling overhead and run variability. Such a result is
**inconclusive**, even if one rounded number is smaller. A large difference
that clears the threshold is an indication, not a statistical proof of a
cause. Repeat the run if the desktop was busy or you touched the mouse.

Always also try the app by hand at normal and maximised sizes. Check tab
changes, scrolling, dragging, resizing and the new feature itself. A passing
tab benchmark cannot establish that other interactions remain responsive.

## Initial baseline on 12 November 2026

The first completed run is
`results/20261112T155705Z_baseline_7ebc7813.json`. A repeat with the same
release binary is `results/20261112T155759Z_repeat_7ebc7813.json`. Both used
Slate on LXQt/X11 at a 1920 x 1080 display mode. Each contains 20 raw
switches per window size. The binary hash begins `7ebc7813f8fb`.

| Window | Baseline median / p90 | Repeat median / p90 | Screenshot poll floor |
| --- | --- | --- | --- |
| 640 x 480 | about 200 / 230 ms | about 190 / 200 ms | about 30 to 40 ms |
| Maximised, 1920 x 1028 | about 210 / 230 ms | about 210 / 250 ms | about 90 ms |

The comparison calls both differences inconclusive. These are observed
click-command-to-pixel times, not precise input latency. Earlier attempts
left `.partial.json` files after calibration failed; they contain no completed
scenario and cannot be compared. The original project config remained
unchanged.

## Search and Cards feature check on 12 November 2026

The pre-change run is `results/20261112T165545Z_pre_search_cards_7ebc7813.json`.
The post-change run is
`results/20261112T170707Z_after_search_cards_dcb02d7e.json`. Both used the
same Slate, LXQt/X11 and 1920 x 1080 display conditions, with 20 switches per
size. The pre-change median was about 170 ms normal and 220 ms maximised; the
post-change median was about 180 ms normal and 210 ms maximised. Both
differences were below the comparison threshold and are **inconclusive**.
This does not demonstrate a speed improvement or regression. The new table
search and card reordering were checked separately with real desktop input.

## Charts feature check on 12 November 2026

The pre-change run is `results/20261112T172544Z_pre_charts_dcb02d7e.json`;
the post-change run is `results/20261112T174003Z_after_charts_b9f11ece.json`.
Both used Slate, LXQt/X11, the same display and 20 switches per size. Both
reported about 190 ms median normal and 220 ms maximised. The comparison
calls both differences **inconclusive** under its noise thresholds. This
scenario switches Dashboard and Panels; it does not time chart rendering,
filtering or search. Those controls were checked separately on the desktop.

## Paint-cost check (cpu.py)

`bench.py` times one visible change and its noise threshold is about 200 ms, so
it cannot see small styling costs. `cpu.py` measures the CPU time of the WebKit
web process (layout and painting) and of picom while the maximised app cycles
through all tabs with the arrow keys and scrolls each page:

    python3 src-tauri/benchmark/cpu.py run --label NAME --theme aero-blue \
      --binary /path/to/variant/rust-scaffold --rounds 2

Keep each variant binary in its own folder and named `rust-scaffold` (the
window is found by that name). Run variants alternately in one session and
compare the medians; on the test machine repeated rounds of one build agreed
within about 0.1 s of 19 s. It takes over the mouse and keyboard for about 35
seconds per round, so ask before running it. Results are saved as
`results/*_cpu_*.json`. A smaller number means less painting work; it does not
replace a hand check of how the window feels.

## Extending the checks

`tab_switch.py` contains the UI points, colour calibration and action loop.
`bench.py` handles builds, isolated launches, environment capture and result
files. `compare.py` handles summaries and comparisons. A future table search
or chart check can use another scenario module and add its result to the
`scenarios` object in `bench.py`. Give it a distinct scenario and method ID;
keep the raw samples and calibration overhead so comparisons remain honest.
