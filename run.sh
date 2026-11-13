#!/usr/bin/env bash
# Launches the app from dist/ without building. Run ./build.sh first, or
# pass --build to build and then launch. The app keeps config.json,
# options.json and debug.log beside its executable, in dist/.
set -uo pipefail

cd "$(dirname "$(readlink -f "$0")")"

BIN="dist/rust-scaffold"

if [ "${1:-}" = "--build" ]; then
  ./build.sh || exit $?
fi

if [ ! -x "$BIN" ]; then
  echo "No executable at $BIN. Run ./build.sh first (or ./run.sh --build)." >&2
  exit 1
fi

# On X11 the transparent rounded corners need a compositor.
if [ "${XDG_SESSION_TYPE:-}" = "x11" ] && command -v picom >/dev/null && ! pgrep -x picom >/dev/null; then
  echo "Starting picom compositor for window transparency..."
  picom --backend glx >/dev/null 2>&1 &
  disown
  sleep 1
fi

# Start the app detached from the terminal with its output discarded, then
# return the prompt at once. Closing the terminal does not close the app.
nohup "$BIN" >/dev/null 2>&1 &
disown
