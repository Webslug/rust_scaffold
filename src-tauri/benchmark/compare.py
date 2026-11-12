"""Summarise and compare raw benchmark result files."""

import json
import math
import statistics
from pathlib import Path

DEBUG_MODE = False


def nearest_rank(values, fraction):
    ordered = sorted(values)
    return ordered[max(0, math.ceil(fraction * len(ordered)) - 1)]


def summary(scenario):
    values = [sample["elapsed_ms"] for sample in scenario["samples"]]
    if not values:
        raise ValueError("A scenario contains no measurements")
    median = statistics.median(values)
    return {
        "count": len(values),
        "median_ms": median,
        "p90_ms": nearest_rank(values, 0.90),
        "mad_ms": statistics.median(abs(value - median) for value in values),
        "poll_floor_ms": scenario["poll_median_ms"],
    }


def load(path):
    with Path(path).open(encoding="utf-8") as source:
        result = json.load(source)
    if result.get("schema_version") != 1:
        raise ValueError(f"Unsupported result schema in {path}")
    if result.get("status") != "complete":
        raise ValueError(f"Result is incomplete: {path}")
    return result


def mismatches(before, after, left, right):
    differences = []
    for key in ("scenario", "method", "sample_pixel", "click_points", "warmups", "pause_ms"):
        if left.get(key) != right.get(key):
            differences.append(key)
    for key in ("theme", "window_size", "display", "session_type", "desktop",
                "screen_mode", "compositor", "picom_version", "xdotool_version",
                "imagemagick_version", "system", "gdk_scale", "webkit_disable_compositing"):
        if before["environment"].get(key) != after["environment"].get(key):
            differences.append(key)
    if len(left["samples"]) != len(right["samples"]):
        differences.append("sample count")
    if (left["geometry"]["width"], left["geometry"]["height"]) != (
        right["geometry"]["width"], right["geometry"]["height"]
    ):
        differences.append("actual window geometry")
    low = min(left["poll_median_ms"], right["poll_median_ms"])
    high = max(left["poll_median_ms"], right["poll_median_ms"])
    if low <= 0 or high / low > 1.25:
        differences.append("screenshot polling overhead")
    return differences


def noisy(stats):
    return stats["p90_ms"] - stats["median_ms"] > max(
        2 * stats["poll_floor_ms"], 0.5 * stats["median_ms"]
    )


def rounded_ms(value):
    return int(round(value / 10.0) * 10)


def compare(before_path, after_path):
    before = load(before_path)
    after = load(after_path)
    old_scenarios = before["scenarios"]
    new_scenarios = after["scenarios"]
    if set(old_scenarios) != set(new_scenarios):
        raise ValueError("Result files contain different scenario sets")

    print(f"Before: {before['label']} ({before['started_utc']}, {before['binary_sha256'][:12]})")
    print(f"After:  {after['label']} ({after['started_utc']}, {after['binary_sha256'][:12]})")
    for name in sorted(old_scenarios):
        left, right = old_scenarios[name], new_scenarios[name]
        a, b = summary(left), summary(right)
        delta = b["median_ms"] - a["median_ms"]
        shown_a, shown_b = rounded_ms(a["median_ms"]), rounded_ms(b["median_ms"])
        shown_p90_a, shown_p90_b = rounded_ms(a["p90_ms"]), rounded_ms(b["p90_ms"])
        shown_delta = shown_b - shown_a
        shown_percent = 100 * shown_delta / shown_a if shown_a else 0
        shown_p90_delta = shown_p90_b - shown_p90_a
        shown_p90_percent = 100 * shown_p90_delta / shown_p90_a if shown_p90_a else 0
        print(f"\n{name} ({a['count']} and {b['count']} samples):")
        print(f"  median ~{shown_a} -> ~{shown_b} ms "
              f"({shown_delta:+d} ms, {shown_percent:+.0f}%)")
        print(f"  p90    ~{shown_p90_a} -> ~{shown_p90_b} ms "
              f"({shown_p90_delta:+d} ms, {shown_p90_percent:+.0f}%)")
        differences = mismatches(before, after, left, right)
        if differences:
            print("  INCOMPATIBLE: " + ", ".join(differences))
            continue
        if noisy(a) or noisy(b):
            print("  INCONCLUSIVE: the slow tail is too variable; repeat both runs")
            continue
        floor = max(a["poll_floor_ms"], b["poll_floor_ms"])
        threshold = max(2 * floor, 0.1 * a["median_ms"], 2 * (a["mad_ms"] + b["mad_ms"]))
        if abs(delta) < threshold:
            note = "both medians are near the polling floor" if min(
                a["median_ms"], b["median_ms"]
            ) <= 1.5 * floor else "difference is below the noise threshold"
            print(f"  INCONCLUSIVE: {note} (threshold about {rounded_ms(threshold)} ms)")
        else:
            direction = "IMPROVED" if delta < 0 else "REGRESSED"
            print(f"  {direction}: difference exceeds the conservative threshold "
                  f"of about {rounded_ms(threshold)} ms")
    print("\nTimes include click and screenshot polling; they are not precise input latency.")
