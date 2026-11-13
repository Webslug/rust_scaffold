#!/usr/bin/env bash
# Build the standalone release executable and copy it to dist/ without
# launching the app. run.sh launches the dist/ copy.
set -euo pipefail

# While true, the build cache (src-tauri/target) is kept for faster
# incremental rebuilds. Set to false to delete it after each build.
DEV_MODE=true

PROJECT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
cd "$PROJECT_DIR"

if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust Cargo is not installed or is not on PATH." >&2
  exit 127
fi

# On Kim's machine cc is restricted to the tools group. A new login picks up
# the group normally; sg covers existing login sessions.
if ! cc --version >/dev/null 2>&1; then
  if command -v sg >/dev/null 2>&1 && getent group tools | grep -qw "$(id -un)"; then
    echo "Compiler needs the tools group; building through sg."
    quoted_script="$(printf '%q' "$PROJECT_DIR/build.sh")"
    exec sg tools -c "bash $quoted_script"
  fi
  echo "C compiler cc is unavailable. Install a C toolchain or check its permissions." >&2
  exit 1
fi

echo "Building the release executable..."
(cd src-tauri && nice -n 10 cargo build --release --locked -j 4)

BIN="$PROJECT_DIR/src-tauri/target/release/rust-scaffold"
if [ ! -x "$BIN" ]; then
  echo "Cargo returned success, but the expected executable is missing: $BIN" >&2
  exit 1
fi

mkdir -p "$PROJECT_DIR/dist"
cp -f "$BIN" "$PROJECT_DIR/dist/rust-scaffold"
echo "Build complete: $BIN"
echo "Copied to: $PROJECT_DIR/dist/rust-scaffold"

# Remove the build cache (src-tauri/target only) once the dist/ copy is
# confirmed, unless DEV_MODE is true.
TARGET_DIR="$PROJECT_DIR/src-tauri/target"
if [ "$DEV_MODE" = true ]; then
  echo "DEV_MODE is true: leaving $TARGET_DIR in place."
elif [ -x "$PROJECT_DIR/dist/rust-scaffold" ] && [ -d "$TARGET_DIR" ] && [ "$(basename "$TARGET_DIR")" = "target" ]; then
  rm -rf -- "$TARGET_DIR"
  echo "Removed build cache: $TARGET_DIR"
fi
