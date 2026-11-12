#!/usr/bin/env bash
# Builds the app if anything changed, then launches it from the project
# folder (config.json and debug.log live here).
set -uo pipefail

cd "$(dirname "$(readlink -f "$0")")"

if ! command -v cargo >/dev/null && [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
fi

BIN="src-tauri/target/release/rust-scaffold"

build() {
  # Low priority so a build does not disturb audio or other foreground work.
  (cd src-tauri && nice -n 10 cargo build --release --quiet)
}

# The C compiler is restricted to the "tools" group (see AGENTS.md). Group
# membership only reaches a session at login, so if this shell lacks it but
# the account is a member, run the build through sg.
if ! cc --version >/dev/null 2>&1 && getent group tools | grep -qw "$(id -un)"; then
  echo "Compiler needs the tools group; using sg until you next log in."
  sg tools -c "cd src-tauri && nice -n 10 cargo build --release --quiet"
else
  build
fi
status=$?

if [ "$status" -ne 0 ]; then
  if [ -x "$BIN" ]; then
    echo "Build failed (exit $status). Launching the previous build." >&2
  else
    echo "Build failed (exit $status) and there is no previous build." >&2
    exit "$status"
  fi
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
