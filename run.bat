@echo off
rem Builds the app if anything changed, then launches it from the project
rem folder (config.json and debug.log live here).
cd /d "%~dp0"

pushd src-tauri
cargo build --release --quiet
if errorlevel 1 (
  popd
  echo Build failed.
  pause
  exit /b 1
)
popd

start "" "src-tauri\target\release\rust-scaffold.exe"
