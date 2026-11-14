# Starts the app with an empty settings folder, checks that the window
# appears and the settings files are created, and saves a screenshot.
# Sourced by tools/xvfb_check.sh.

app_start
# The start-up fallback (src-tauri/src/startup.rs) shows the window after
# 5 s; a normal start must be well under that, shown by app.js.
expect "window shown by app.js, not the fallback ($WINDOW_MS ms)" test "$WINDOW_MS" -lt 4000
for f in config.json keys.json search.json; do
    expect "$f created" test -s "$SETTINGS/$f"
done
expect "no log file with DEBUG_MODE off" test ! -e "$SETTINGS/debug.log"
shot smoke_start
app_stop
