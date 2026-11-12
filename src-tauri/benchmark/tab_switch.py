"""Dashboard/Panel tab benchmark using X11 clicks and one-pixel screenshots."""

import re
import statistics
import subprocess
import time

DEBUG_MODE = False

SCENARIO_ID = "dashboard-panels"
METHOD_ID = "xdotool-click-imagemagick-pixel-v1"
SAMPLE_X, SAMPLE_Y = 100, 245
TAB_Y = 72
TAB_X = {"dashboard": 45, "panels": 190}
PAUSE_SECONDS = 0.25
MAX_CHANGE_SECONDS = 5.0
PIXEL_RE = re.compile(r"#([0-9A-Fa-f]{6}|[0-9A-Fa-f]{12})\b")


def command(*args, timeout=5):
    result = subprocess.run(args, text=True, capture_output=True, timeout=timeout)
    if result.returncode != 0:
        raise RuntimeError(f"{' '.join(args)} failed: {result.stderr.strip()}")
    return result.stdout


def pixel(window_id):
    output = command(
        "import", "-window", str(window_id),
        "-crop", f"1x1+{SAMPLE_X}+{SAMPLE_Y}", "txt:-", timeout=5,
    )
    match = PIXEL_RE.search(output)
    if not match:
        raise RuntimeError(f"Could not read the sample pixel: {output[-200:]!r}")
    return match.group(1).upper()


def click_tab(window_id, tab):
    command(
        "xdotool", "mousemove", "--window", str(window_id),
        str(TAB_X[tab]), str(TAB_Y), "click", "1",
    )


def wait_for_pixel(window_id, expected, started_ns):
    polls = 0
    seen = None
    while (time.perf_counter_ns() - started_ns) / 1e9 < MAX_CHANGE_SECONDS:
        polls += 1
        seen = pixel(window_id)
        if seen == expected:
            return {
                "elapsed_ms": round((time.perf_counter_ns() - started_ns) / 1e6, 2),
                "polls": polls,
            }
    raise RuntimeError(
        f"Tab pixel did not change within 5 seconds "
        f"(expected #{expected}, last #{seen}, {polls} polls)"
    )


def run(window_id, iterations):
    """Return raw alternating tab measurements; the caller owns the window."""
    command("xdotool", "windowactivate", "--sync", str(window_id))
    click_tab(window_id, "dashboard")
    time.sleep(0.45)
    dashboard_pixel = pixel(window_id)
    click_tab(window_id, "panels")
    time.sleep(0.45)
    panels_pixel = pixel(window_id)
    if dashboard_pixel == panels_pixel:
        raise RuntimeError(
            "Dashboard and Panels have the same sample pixel; update the "
            "scenario's SAMPLE_X/SAMPLE_Y before benchmarking this UI"
        )
    click_tab(window_id, "dashboard")
    time.sleep(0.45)

    poll_times = []
    for _ in range(8):
        started = time.perf_counter_ns()
        pixel(window_id)
        poll_times.append((time.perf_counter_ns() - started) / 1e6)
    poll_median_ms = round(statistics.median(poll_times), 2)

    targets = {"dashboard": dashboard_pixel, "panels": panels_pixel}
    for index in range(4):
        target = "panels" if index % 2 == 0 else "dashboard"
        started = time.perf_counter_ns()
        click_tab(window_id, target)
        wait_for_pixel(window_id, targets[target], started)
        time.sleep(PAUSE_SECONDS)

    samples = []
    for index in range(iterations):
        target = "panels" if index % 2 == 0 else "dashboard"
        started = time.perf_counter_ns()
        click_tab(window_id, target)
        measurement = wait_for_pixel(window_id, targets[target], started)
        measurement["to"] = target
        samples.append(measurement)
        time.sleep(PAUSE_SECONDS)

    return {
        "scenario": SCENARIO_ID,
        "method": METHOD_ID,
        "sample_pixel": [SAMPLE_X, SAMPLE_Y],
        "click_points": {name: [x, TAB_Y] for name, x in TAB_X.items()},
        "poll_median_ms": poll_median_ms,
        "poll_raw_ms": [round(value, 2) for value in poll_times],
        "warmups": 4,
        "pause_ms": int(PAUSE_SECONDS * 1000),
        "samples": samples,
    }
