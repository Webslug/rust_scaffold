@echo off
rem Launches the app from dist\ without building. Run "run.bat --build" to
rem build the release executable, copy it to dist\, and then launch it.
rem The app keeps config.json, options.json and debug.log in
rem %APPDATA%\rust_scaffold (see docs\usage.md).
setlocal
cd /d "%~dp0"

set "BIN=dist\rust-scaffold.exe"

if /i "%~1"=="--build" (
  where cargo >nul 2>nul
  if errorlevel 1 (
    echo Rust Cargo is not installed or is not on PATH.
    exit /b 127
  )
  echo Building the release executable...
  pushd src-tauri
  cargo build --release --locked
  if errorlevel 1 (
    popd
    echo Build failed. dist\ was not changed.
    exit /b 1
  )
  popd
  if not exist "src-tauri\target\release\rust-scaffold.exe" (
    echo Cargo returned success, but the executable is missing.
    exit /b 1
  )
  if not exist dist mkdir dist
  copy /y "src-tauri\target\release\rust-scaffold.exe" "%BIN%" >nul
  if errorlevel 1 (
    echo Could not copy the executable to dist\. Is the app still running?
    exit /b 1
  )
  echo Copied to %BIN%
)

if not exist "%BIN%" (
  echo No executable at %BIN%. Run "run.bat --build" first.
  exit /b 1
)

rem start returns at once; the app runs on after this window closes.
start "" "%BIN%"
endlocal
