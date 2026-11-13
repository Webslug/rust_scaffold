#!/usr/bin/env python3
"""Paint-cost benchmark: CPU time used while the maximised app cycles tabs.

Runs a copy of a release binary from a temporary folder with a fixture
config, maximises the window, focuses the first tab, then repeatedly presses
Right (Bootstrap moves to and shows the next tab) and scrolls the page down
and up. Reports the CPU time of the WebKit web process (page layout and
painting) and of the compositor (picom) during that sequence. Finer than
bench.py's screenshot timing for small styling costs, because it sums all the
work instead of timing one visible change.

Usage:
  cpu.py run --label NAME [--theme slate] [--layout horizontal|vertical]
             [--binary PATH] [--rounds 3] [--steps 36]
"""
import argparse
import json
import os
import shutil
import statistics
import subprocess
import tempfile
import time
from pathlib import Path

import bench

TICKS = os.sysconf("SC_CLK_TCK")
STEP_PAUSE = 0.15
SCROLL_CLICKS = 3
# Where to click to focus the first tab (Dashboard), per menu layout, in
# window coordinates at the default title bar and text size.
FIRST_TAB = {"horizontal": ("80", "75"), "vertical": ("80", "169")}


def cpu_ticks(pid):
    """utime + stime of one process, in clock ticks."""
    with open(f"/proc/{pid}/stat", encoding="ascii") as handle:
        fields = handle.read().rsplit(")", 1)[1].split()
    return int(fields[11]) + int(fields[12])


def descendants(pid):
    found = []
    result = subprocess.run(["pgrep", "-P", str(pid)], capture_output=True, text=True)
    for child in result.stdout.split():
        found.append(int(child))
        found.extend(descendants(int(child)))
    return found


def web_processes(app_pid):
    pids = []
    for pid in descendants(app_pid):
        try:
            with open(f"/proc/{pid}/comm", encoding="ascii") as handle:
                if handle.read().strip() == "WebKitWebProces":
                    pids.append(pid)
        except FileNotFoundError:
            pass
    return pids


def compositor_pid():
    result = subprocess.run(["pgrep", "-x", "picom"], capture_output=True, text=True)
    pids = result.stdout.split()
    return int(pids[0]) if pids else None


def sample(pids):
    total = 0
    for pid in pids:
        try:
            total += cpu_ticks(pid)
        except FileNotFoundError:
            pass
    return total


def one_round(binary, fixture, steps, layout):
    with tempfile.TemporaryDirectory(prefix="rust-scaffold-cpu-") as directory:
        workdir = Path(directory)
        (workdir / "config.json").write_text(json.dumps(fixture, indent=2) + "\n", encoding="utf-8")
        copy = workdir / binary.name
        shutil.copy2(binary, copy)
        process = subprocess.Popen(
            [str(copy)], cwd=workdir, stdin=subprocess.DEVNULL,
            env={**os.environ, "RUST_SCAFFOLD_DIR": str(workdir)},
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True,
        )
        try:
            window_id = bench.wait_for_window(process)
            size = bench.prepare_window(window_id, "maximized")
            time.sleep(1.0)
            # Focus the first tab (Dashboard), then let everything settle.
            x, y = FIRST_TAB[layout]
            bench.command("xdotool", "mousemove", "--window", str(window_id), x, y, "click", "1")
            bench.command("xdotool", "mousemove", "--window", str(window_id),
                          str(size["width"] // 2), str(size["height"] // 2))
            time.sleep(1.0)
            web = web_processes(process.pid)
            if not web:
                raise RuntimeError("WebKit web process not found")
            picom = compositor_pid()
            web_start = sample(web)
            picom_start = sample([picom]) if picom else 0
            started = time.monotonic()
            for step in range(steps):
                # Arrow keys act on the focused tab; wrap from the last tab.
                key = "Right" if step % 6 != 5 else "Home"
                bench.command("xdotool", "key", key)
                time.sleep(STEP_PAUSE)
                for button in ("5", "4"):
                    for _ in range(SCROLL_CLICKS):
                        bench.command("xdotool", "click", button)
                time.sleep(STEP_PAUSE)
            elapsed = time.monotonic() - started
            time.sleep(0.5)
            web_used = sample(web) - web_start
            picom_used = (sample([picom]) - picom_start) if picom else None
            return {
                "web_cpu_s": web_used / TICKS,
                "picom_cpu_s": picom_used / TICKS if picom_used is not None else None,
                "elapsed_s": elapsed,
                "window": size,
            }
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="action", required=True)
    run = sub.add_parser("run")
    run.add_argument("--label", required=True)
    run.add_argument("--theme", default="slate")
    run.add_argument("--layout", choices=sorted(FIRST_TAB), default="horizontal")
    run.add_argument("--binary", type=Path, default=bench.BIN)
    run.add_argument("--rounds", type=int, default=3)
    run.add_argument("--steps", type=int, default=36)
    args = parser.parse_args()

    fixture = bench.config_fixture(args.theme)
    fixture["menu_layout"] = args.layout
    fixture["menu_collapsed"] = False
    active, pointer = bench.previous_desktop_state()
    rounds = []
    try:
        for index in range(args.rounds):
            result = one_round(args.binary.resolve(), fixture, args.steps, args.layout)
            rounds.append(result)
            picom = f"{result['picom_cpu_s']:.2f} s" if result["picom_cpu_s"] is not None else "n/a"
            print(f"  round {index + 1}: web {result['web_cpu_s']:.2f} s, picom {picom}, "
                  f"{result['elapsed_s']:.1f} s wall", flush=True)
    finally:
        bench.restore_desktop_state(active, pointer)
    web = [r["web_cpu_s"] for r in rounds]
    print(f"{args.label} ({args.theme}, {args.layout}): web median {statistics.median(web):.2f} s, "
          f"range {min(web):.2f} to {max(web):.2f} s")
    bench.RESULTS_DIR.mkdir(exist_ok=True)
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    out = bench.RESULTS_DIR / f"{stamp}_cpu_{args.label}_{args.theme}.json"
    out.write_text(json.dumps({
        "label": args.label, "theme": args.theme, "layout": args.layout, "binary": str(args.binary),
        "steps": args.steps, "rounds": rounds,
    }, indent=2) + "\n", encoding="utf-8")
    print(f"Saved: {out}")


if __name__ == "__main__":
    main()
