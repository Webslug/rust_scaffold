#!/usr/bin/env bash
# Runs the app on a private, hidden X screen (Xvfb) and runs check scripts
# against it. Nothing is drawn on, typed into or clicked on the real desktop,
# and the real settings folder is never read or written.
#
# Usage: tools/xvfb_check.sh [--bin PATH] [--size WxH] [--keep] CHECK [CHECK...]
#   CHECK       name of a file in tools/checks/ (without .sh), or a path
#   --bin PATH  binary to test (default: src-tauri/target/release/rust-scaffold)
#   --size WxH  hidden screen size (default: 854x638)
#   --keep      keep the scratch folder even when every check passes
#   --list      list the checks in tools/checks/ and exit
#
# Safety rules this script enforces:
#   - DISPLAY and WAYLAND_DISPLAY are unset before anything runs, so a command
#     that forgets the hidden screen fails instead of reaching the desktop.
#   - The app gets a private D-Bus session bus, started and stopped here, so
#     its tray icon and other D-Bus traffic never reach the real desktop
#     panel. DBUS_SESSION_BUS_ADDRESS is unset for the same reason.
#   - The hidden screen is started by this script (Xvfb picks a free display
#     number) and every input and screenshot helper names it on each call.
#   - It refuses to run if the hidden screen is the real one or is not up.
#   - It stops on the first command error (set -euo pipefail).
#   - It only stops the processes it started, by their exact PIDs.
#   - The app runs from a scratch copy, with scratch settings, cache and data
#     folders. The binary given with --bin is never run in place.
#
# Check scripts are sourced in this shell and use the helpers below:
#   app_start [settings]  start the scratch copy, wait for its window
#                         settings: "override" (default, RUST_SCAFFOLD_DIR) or
#                         "xdg" (no override, the dirs crate path under
#                         the scratch XDG_CONFIG_HOME)
#   app_stop              stop the copy this script started
#   key KEY...            xdotool key names, for example ctrl+k Escape alt+7
#   type_text TEXT        type text into the focused field
#   click X Y             left click at X,Y inside the app window
#   shot NAME             save a screenshot of the hidden screen as NAME.png
#   ok MESSAGE / bad MESSAGE   record a passed or failed assertion
#   expect DESCRIPTION COMMAND...   run COMMAND; pass if it succeeds
# Variables: $SCRATCH (scratch root), $BIN_DIR (folder holding the copy),
#   $REAL_BUS (the real session bus address, for read-only comparisons),
#   $SETTINGS (settings folder the app uses; $SCRATCH/settings before the
#   first app_start), $OUT (screenshots and app output), $WINDOW_MS
#   (milliseconds from launch until the window was visible, set by app_start).

set -euo pipefail

# Names a fork of the scaffold must change (the fork script checks for them).
APP_BIN_NAME="rust-scaffold"
APP_FOLDER="rust_scaffold"
DIR_OVERRIDE="RUST_SCAFFOLD_DIR"

ROOT="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
CHECKS_DIR="$ROOT/tools/checks"
WINDOW_TIMEOUT_S="${XVFB_CHECK_WINDOW_TIMEOUT:-20}"

# Captured, then removed, so nothing below can reach the desktop by default.
REAL_DISPLAY="${DISPLAY:-}"
REAL_BUS="${DBUS_SESSION_BUS_ADDRESS:-}"
unset DISPLAY WAYLAND_DISPLAY DBUS_SESSION_BUS_ADDRESS

SOURCE_BIN="$ROOT/src-tauri/target/release/$APP_BIN_NAME"
SIZE="854x638"
KEEP=0
CHECKS=()

usage() { sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; }

while [ $# -gt 0 ]; do
    case "$1" in
        --bin) SOURCE_BIN="$(readlink -f "$2")"; shift 2 ;;
        --size) SIZE="$2"; shift 2 ;;
        --keep) KEEP=1; shift ;;
        --list) for f in "$CHECKS_DIR"/*.sh; do basename "$f" .sh; done; exit 0 ;;
        -h|--help) usage; exit 0 ;;
        -*) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
        *) CHECKS+=("$1"); shift ;;
    esac
done

[ ${#CHECKS[@]} -gt 0 ] || { usage >&2; exit 2; }
[[ "$SIZE" =~ ^[0-9]+x[0-9]+$ ]] || { echo "Bad --size: $SIZE" >&2; exit 2; }
[ -x "$SOURCE_BIN" ] || { echo "No executable at $SOURCE_BIN (build it first)." >&2; exit 2; }
for tool in Xvfb xdotool xdpyinfo import dbus-daemon; do
    command -v "$tool" >/dev/null || { echo "Missing tool: $tool" >&2; exit 2; }
done

CHECK_FILES=()
for c in "${CHECKS[@]}"; do
    if [ -f "$c" ]; then CHECK_FILES+=("$(readlink -f "$c")")
    elif [ -f "$CHECKS_DIR/$c.sh" ]; then CHECK_FILES+=("$CHECKS_DIR/$c.sh")
    else echo "No check named $c (see --list)." >&2; exit 2
    fi
done

SCRATCH="$(mktemp -d /tmp/xvfb_check.XXXXXX)"
BIN_DIR="$SCRATCH/bin"
OUT="$SCRATCH/out"
SETTINGS="$SCRATCH/settings"
WINDOW_MS=0
mkdir -p "$BIN_DIR" "$OUT" "$SCRATCH/xdg_config" "$SCRATCH/xdg_cache" "$SCRATCH/xdg_data"
cp "$SOURCE_BIN" "$BIN_DIR/$APP_BIN_NAME"

XVFB_PID=""
BUS_PID=""
APP_PID=""
APP_WIN=""
HIDDEN=""
PASSED=0
FAILED=0

cleanup() {
    local status=$?
    if [ -n "$APP_PID" ]; then kill "$APP_PID" 2>/dev/null || true; wait "$APP_PID" 2>/dev/null || true; fi
    if [ -n "$XVFB_PID" ]; then kill "$XVFB_PID" 2>/dev/null || true; wait "$XVFB_PID" 2>/dev/null || true; fi
    if [ -n "$BUS_PID" ]; then kill "$BUS_PID" 2>/dev/null || true; fi
    if [ "$status" -ne 0 ] || [ "$FAILED" -gt 0 ] || [ "$KEEP" -eq 1 ]; then
        echo "Scratch folder kept: $SCRATCH"
    else
        rm -rf "$SCRATCH"
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'echo "STOPPED: command failed at line $LINENO: $BASH_COMMAND" >&2' ERR

# Start a private Xvfb. -displayfd makes Xvfb pick a free number and report it.
Xvfb -displayfd 3 -nolisten tcp -screen 0 "${SIZE}x24" >"$OUT/xvfb.log" 2>&1 3>"$SCRATCH/display" &
XVFB_PID=$!
for _ in $(seq 1 50); do
    [ -s "$SCRATCH/display" ] && break
    kill -0 "$XVFB_PID" 2>/dev/null || { echo "Xvfb exited, see $OUT/xvfb.log" >&2; exit 1; }
    sleep 0.1
done
HIDDEN=":$(tr -dc '0-9' <"$SCRATCH/display")"

assert_hidden() {
    if [ "$HIDDEN" = ":" ] || [ "$HIDDEN" = "$REAL_DISPLAY" ]; then
        echo "REFUSED: hidden screen '$HIDDEN' is not a private screen (desktop is '$REAL_DISPLAY')." >&2
        exit 1
    fi
    kill -0 "$XVFB_PID" 2>/dev/null || { echo "REFUSED: the hidden screen is not running." >&2; exit 1; }
}
assert_hidden
xdpyinfo -display "$HIDDEN" >/dev/null || { echo "REFUSED: cannot reach hidden screen $HIDDEN." >&2; exit 1; }
# Private session bus: first line of output is the address, second the PID.
# Its config has no service folders, so nothing is auto-started on it. With
# the standard session config the desktop portal is started and then waits
# 25 s for the keyring, delaying the window by that much.
cat >"$SCRATCH/bus.conf" <<BUSCONF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:dir=$SCRATCH</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
BUSCONF
mapfile -t bus_info < <(dbus-daemon --config-file="$SCRATCH/bus.conf" --fork --print-address=1 --print-pid=1)
PRIVATE_BUS="${bus_info[0]:-}"
BUS_PID="${bus_info[1]:-}"
if [ -z "$PRIVATE_BUS" ] || [ -z "$BUS_PID" ] || [ "$PRIVATE_BUS" = "$REAL_BUS" ]; then
    echo "REFUSED: could not start a private D-Bus session bus." >&2
    exit 1
fi
echo "Hidden screen $HIDDEN ($SIZE), Xvfb PID $XVFB_PID, private bus PID $BUS_PID, scratch $SCRATCH"

# Every X call goes through this, with the hidden screen named on the call.
x() { assert_hidden; DISPLAY="$HIDDEN" xdotool "$@"; }

ok() { PASSED=$((PASSED + 1)); echo "  PASS: $*"; }
bad() { FAILED=$((FAILED + 1)); echo "  FAIL: $*"; }
expect() {
    local what="$1"; shift
    if "$@"; then ok "$what"; else bad "$what"; fi
}

APP_RUN=0
app_start() {
    local mode="${1:-override}"
    [ -z "$APP_PID" ] || { echo "app_start: the app is already running (PID $APP_PID)." >&2; exit 1; }
    assert_hidden
    APP_RUN=$((APP_RUN + 1))
    local env_args=(env -u "$DIR_OVERRIDE" -u WAYLAND_DISPLAY DISPLAY="$HIDDEN" DBUS_SESSION_BUS_ADDRESS="$PRIVATE_BUS"
        XDG_CONFIG_HOME="$SCRATCH/xdg_config" XDG_CACHE_HOME="$SCRATCH/xdg_cache"
        XDG_DATA_HOME="$SCRATCH/xdg_data")
    case "$mode" in
        override) SETTINGS="$SCRATCH/settings"; env_args+=("$DIR_OVERRIDE=$SETTINGS") ;;
        xdg) SETTINGS="$SCRATCH/xdg_config/$APP_FOLDER" ;;
        *) echo "app_start: unknown settings mode $mode" >&2; exit 1 ;;
    esac
    local started_us="${EPOCHREALTIME/./}"
    "${env_args[@]}" "$BIN_DIR/$APP_BIN_NAME" >"$OUT/app_$APP_RUN.out" 2>&1 &
    APP_PID=$!
    APP_WIN=""
    local deadline=$((SECONDS + WINDOW_TIMEOUT_S))
    while [ -z "$APP_WIN" ]; do
        kill -0 "$APP_PID" 2>/dev/null || { echo "app_start: the app exited, see $OUT/app_$APP_RUN.out" >&2; APP_PID=""; exit 1; }
        [ "$SECONDS" -lt "$deadline" ] || { echo "app_start: no window after ${WINDOW_TIMEOUT_S}s." >&2; exit 1; }
        APP_WIN="$(x search --onlyvisible --pid "$APP_PID" 2>/dev/null | head -n 1 || true)"
        [ -n "$APP_WIN" ] || sleep 0.1
    done
    WINDOW_MS=$(( (${EPOCHREALTIME/./} - started_us) / 1000 ))
    echo "  window visible after ${WINDOW_MS} ms"
    x windowfocus --sync "$APP_WIN"
    sleep 1
}

app_stop() {
    [ -n "$APP_PID" ] || return 0
    kill "$APP_PID" 2>/dev/null || true
    wait "$APP_PID" 2>/dev/null || true
    APP_PID=""
    APP_WIN=""
}

need_app() { [ -n "$APP_PID" ] || { echo "No app running; call app_start first." >&2; exit 1; }; }
key() { need_app; x windowfocus --sync "$APP_WIN"; x key --delay 60 "$@"; sleep 0.3; }
type_text() { need_app; x windowfocus --sync "$APP_WIN"; x type --delay 40 "$1"; sleep 0.3; }
click() { need_app; x mousemove --sync --window "$APP_WIN" "$1" "$2" click 1; sleep 0.3; }
shot() { assert_hidden; import -display "$HIDDEN" -window root "$OUT/$1.png"; echo "  shot: $OUT/$1.png"; }

for file in "${CHECK_FILES[@]}"; do
    echo "== $(basename "$file" .sh)"
    # shellcheck source=/dev/null
    source "$file"
    app_stop
done

echo "Done: $PASSED passed, $FAILED failed."
[ "$FAILED" -eq 0 ]
