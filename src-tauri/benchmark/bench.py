#!/usr/bin/env python3
"""Build, run and compare X11 desktop benchmarks for the Rust scaffold."""

import argparse
import ctypes
import ctypes.util
import datetime as dt
import hashlib
import json
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

# Keep the benchmark folder limited to scripts, instructions and results.
sys.dont_write_bytecode = True

import compare
import tab_switch

DEBUG_MODE = False

BENCH_DIR = Path(__file__).resolve().parent
PROJECT_DIR = BENCH_DIR.parent.parent
BIN = PROJECT_DIR / "src-tauri" / "target" / "release" / "rust-scaffold"
RESULTS_DIR = BENCH_DIR / "results"
LABEL_RE = re.compile(r"[a-z0-9][a-z0-9_-]{0,39}\Z")
DEFAULT_CONFIG = {"theme": "slate", "width": 640, "height": 480}


def command(*args, timeout=10, check=True):
    result = subprocess.run(args, text=True, capture_output=True, timeout=timeout)
    if check and result.returncode != 0:
        raise RuntimeError(f"{' '.join(str(a) for a in args)}: {result.stderr.strip()}")
    return result


def first_line(*args):
    try:
        result = command(*args, check=False)
    except OSError:
        return "unavailable"
    return result.stdout.splitlines()[0].strip() if result.stdout else "unavailable"


def compositor_active():
    """Read the X11 compositor selection without changing desktop settings."""
    library = ctypes.util.find_library("X11")
    if not library:
        return "unknown"
    x11 = ctypes.CDLL(library)
    x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
    x11.XOpenDisplay.restype = ctypes.c_void_p
    x11.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
    x11.XInternAtom.restype = ctypes.c_ulong
    x11.XGetSelectionOwner.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
    x11.XGetSelectionOwner.restype = ctypes.c_ulong
    x11.XCloseDisplay.argtypes = [ctypes.c_void_p]
    display = x11.XOpenDisplay(None)
    if not display:
        return "unknown"
    try:
        atom = x11.XInternAtom(display, b"_NET_WM_CM_S0", 1)
        return "active" if atom and x11.XGetSelectionOwner(display, atom) else "inactive"
    finally:
        x11.XCloseDisplay(display)


def environment():
    xrandr = command("xrandr", "--current").stdout
    active_modes = [line.strip() for line in xrandr.splitlines() if "*" in line]
    return {
        "theme": "slate",  # overwritten by the selected test theme
        "window_size": [640, 480],
        "display": os.environ.get("DISPLAY", ""),
        "session_type": os.environ.get("XDG_SESSION_TYPE", ""),
        "desktop": os.environ.get("XDG_CURRENT_DESKTOP", ""),
        "screen_mode": active_modes,
        "compositor": compositor_active(),
        "picom_version": first_line("picom", "--version"),
        "xdotool_version": first_line("xdotool", "--version"),
        "imagemagick_version": first_line("import", "-version"),
        "system": platform.platform(),
        "gdk_scale": os.environ.get("GDK_SCALE", ""),
        "webkit_disable_compositing": os.environ.get("WEBKIT_DISABLE_COMPOSITING_MODE", ""),
    }


def project_config():
    """The user's live config: beside the dist/ binary, else the old root copy."""
    for path in (PROJECT_DIR / "dist" / "config.json", PROJECT_DIR / "config.json"):
        if path.exists():
            return path
    return PROJECT_DIR / "dist" / "config.json"


def config_fixture(theme):
    source = project_config()
    if source.exists():
        if source.stat().st_size > 5 * 1024 * 1024:
            raise RuntimeError("Project config exceeds 5 MB; refusing to copy it")
        with source.open(encoding="utf-8") as handle:
            config = json.load(handle)
        if not isinstance(config, dict):
            raise RuntimeError("Project config must be a JSON object")
    else:
        config = dict(DEFAULT_CONFIG)
    config.update({
        "theme": theme,
        "width": 640,
        "height": 480,
        "show_titlebar": True,
        "rounded_corners": True,
        "start_minimised": False,
        "order": {},
        "collapsed": [],
    })
    return config


def build_and_hash():
    # build.sh exits on failure. run.sh is deliberately excluded because it
    # can launch an older executable after a failed build.
    subprocess.run([str(PROJECT_DIR / "build.sh")], cwd=PROJECT_DIR, check=True)
    if not BIN.is_file() or not os.access(BIN, os.X_OK):
        raise RuntimeError(f"Build reported success but {BIN} is missing")
    digest = hashlib.sha256()
    with BIN.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest(), BIN.stat().st_size


def x11_window_for_pid(pid):
    # Tauri creates the window hidden, then app.js shows it after loading the
    # theme. The tray icon also has a tiny X11 window with the same PID and
    # WM_CLASS, so require the desktop window's minimum configured size.
    result = command("xdotool", "search", "--onlyvisible", "--pid", str(pid), check=False)
    candidates = []
    for item in result.stdout.splitlines():
        if item.isdigit():
            details = command("xprop", "-id", item, "WM_CLASS", check=False).stdout
            if "rust-scaffold" in details.lower():
                size = geometry(int(item))
                if size["width"] >= 400 and size["height"] >= 300:
                    candidates.append((size["width"] * size["height"], int(item)))
    return max(candidates)[1] if candidates else None


def wait_for_window(process):
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"App exited before showing a window (code {process.returncode})")
        window_id = x11_window_for_pid(process.pid)
        if window_id is not None:
            return window_id
        time.sleep(0.2)
    raise RuntimeError("App window did not appear within 12 seconds")


def geometry(window_id):
    output = command("xdotool", "getwindowgeometry", "--shell", str(window_id)).stdout
    values = dict(line.split("=", 1) for line in output.splitlines() if "=" in line)
    return {key.lower(): int(values[key]) for key in ("X", "Y", "WIDTH", "HEIGHT")}


def prepare_window(window_id, state):
    command("xdotool", "windowactivate", "--sync", str(window_id))
    command("xdotool", "windowraise", str(window_id))
    if state == "maximized":
        width = geometry(window_id)["width"]
        command("xdotool", "mousemove", "--window", str(window_id),
                str(width - 55), "20", "click", "1")
        deadline = time.monotonic() + 4
        while time.monotonic() < deadline:
            state_text = command("xprop", "-id", str(window_id), "_NET_WM_STATE").stdout
            if "_NET_WM_STATE_MAXIMIZED_HORZ" in state_text and \
                    "_NET_WM_STATE_MAXIMIZED_VERT" in state_text:
                break
            time.sleep(0.1)
        else:
            raise RuntimeError("Window did not enter the maximised state")
    time.sleep(0.5)
    size = geometry(window_id)
    if state == "normal" and (size["width"], size["height"]) != (640, 480):
        raise RuntimeError(f"Expected 640 x 480 normal window, got {size}")
    if state == "maximized" and size["width"] <= 640:
        raise RuntimeError(f"Maximised window is unexpectedly small: {size}")
    return size


def previous_desktop_state():
    active = command("xdotool", "getactivewindow", check=False).stdout.strip()
    pointer = command("xdotool", "getmouselocation", "--shell", check=False).stdout
    values = dict(line.split("=", 1) for line in pointer.splitlines() if "=" in line)
    return active if active.isdigit() else None, values


def restore_desktop_state(active, pointer):
    if active:
        command("xdotool", "windowactivate", str(active), check=False)
    if "X" in pointer and "Y" in pointer:
        command("xdotool", "mousemove", pointer["X"], pointer["Y"], check=False)


def run_one(state, fixture, iterations):
    with tempfile.TemporaryDirectory(prefix="rust-scaffold-benchmark-") as directory:
        workdir = Path(directory)
        (workdir / "config.json").write_text(json.dumps(fixture, indent=2) + "\n", encoding="utf-8")
        # The app reads config.json beside its executable, so run a copy of
        # the binary from the temporary folder.
        binary = workdir / BIN.name
        shutil.copy2(BIN, binary)
        process = subprocess.Popen(
            [str(binary)], cwd=workdir, stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        try:
            window_id = wait_for_window(process)
            size = prepare_window(window_id, state)
            result = tab_switch.run(window_id, iterations)
            result["geometry"] = size
            result["window_state"] = state
            result["summary"] = compare.summary(result)
            return result
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=4)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=4)


def save_partial(path, result):
    path.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")


def run_benchmark(label, theme, iterations):
    if os.environ.get("XDG_SESSION_TYPE") != "x11" or not os.environ.get("DISPLAY"):
        raise RuntimeError("This benchmark must run in the normal X11 desktop session")
    for tool in ("xdotool", "import", "xprop", "xrandr"):
        if not shutil.which(tool):
            raise RuntimeError(f"Required existing desktop tool is missing: {tool}")
    theme_source = (PROJECT_DIR / "ui" / "js" / "theme.js").read_text(encoding="utf-8")
    available_themes = set(re.findall(r'id:\s*"([a-z0-9-]+)"', theme_source))
    if theme not in available_themes:
        raise RuntimeError(f"Theme {theme!r} is not listed in ui/js/theme.js")
    fixture = config_fixture(theme)
    active, pointer = previous_desktop_state()
    source_config = project_config()
    original_config_hash = hashlib.sha256(source_config.read_bytes()).hexdigest() if source_config.exists() else None
    digest, binary_size = build_and_hash()
    started = dt.datetime.now(dt.timezone.utc)
    metadata = environment()
    metadata["theme"] = theme
    result = {
        "schema_version": 1,
        "status": "incomplete",
        "label": label,
        "started_utc": started.isoformat(timespec="seconds"),
        "binary_sha256": digest,
        "binary_size_bytes": binary_size,
        "environment": metadata,
        "scenarios": {},
    }
    RESULTS_DIR.mkdir(exist_ok=True)
    name = f"{started.strftime('%Y%m%dT%H%M%SZ')}_{label}_{digest[:8]}"
    partial = RESULTS_DIR / f"{name}.partial.json"
    final = RESULTS_DIR / f"{name}.json"
    with partial.open("x", encoding="utf-8") as destination:
        json.dump(result, destination, indent=2)
        destination.write("\n")
    try:
        for state in ("normal", "maximized"):
            print(f"Measuring {state} window...", flush=True)
            scenario = run_one(state, fixture, iterations)
            result["scenarios"][state] = scenario
            save_partial(partial, result)
            stats = scenario["summary"]
            print(f"  median {compare.rounded_ms(stats['median_ms'])} ms; "
                  f"p90 {compare.rounded_ms(stats['p90_ms'])} ms; "
                  f"poll floor about {compare.rounded_ms(stats['poll_floor_ms'])} ms")
        result["status"] = "complete"
        save_partial(partial, result)
        partial.replace(final)
        print(f"Saved raw results: {final}")
    finally:
        restore_desktop_state(active, pointer)
        current_config_hash = hashlib.sha256(source_config.read_bytes()).hexdigest() if source_config.exists() else None
        if current_config_hash != original_config_hash:
            raise RuntimeError("Project config.json changed during the benchmark")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)
    run_parser = subcommands.add_parser("run", help="build and measure both window sizes")
    run_parser.add_argument("--label", required=True, help="short result label, e.g. baseline")
    run_parser.add_argument("--theme", default="slate", help="fixed test theme (default: slate)")
    run_parser.add_argument("--iterations", type=int, default=20, help="switches per size (min: 10)")
    compare_parser = subcommands.add_parser("compare", help="compare two saved JSON runs")
    compare_parser.add_argument("before")
    compare_parser.add_argument("after")
    args = parser.parse_args()
    if args.command == "compare":
        compare.compare(args.before, args.after)
        return
    if not LABEL_RE.fullmatch(args.label):
        parser.error("--label must be 1 to 40 lowercase letters, digits, _ or -")
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]{0,39}", args.theme):
        parser.error("--theme must be a lowercase theme slug")
    if args.iterations < 10 or args.iterations % 2:
        parser.error("--iterations must be even and at least 10")
    run_benchmark(args.label, args.theme, args.iterations)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError,
            subprocess.TimeoutExpired) as error:
        print(f"Benchmark failed: {error}", file=sys.stderr)
        sys.exit(1)
