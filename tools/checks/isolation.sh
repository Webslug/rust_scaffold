# Proves the hidden run stays off the real desktop's D-Bus: the real panel's
# list of tray icons (StatusNotifierItems) must be the same before and while
# the app runs. Skipped when the real session has no StatusNotifier host.
# Sourced by tools/xvfb_check.sh.

real_tray_items() {
    [ -n "$REAL_BUS" ] || return 0
    DBUS_SESSION_BUS_ADDRESS="$REAL_BUS" dbus-send --session --print-reply \
        --dest=org.kde.StatusNotifierWatcher /StatusNotifierWatcher \
        org.freedesktop.DBus.Properties.Get string:org.kde.StatusNotifierWatcher \
        string:RegisteredStatusNotifierItems 2>/dev/null | grep -o '"[^"]*"' | sort | tr '\n' ' ' || true
}

before="$(real_tray_items)"
app_start
sleep 2
during="$(real_tray_items)"
if [ -z "$before" ] && [ -z "$during" ]; then
    echo "  skipped: no StatusNotifier host on the real session bus"
else
    expect "no tray icon added to the real panel" test "$before" = "$during"
fi
expect "app still running with no tray host" kill -0 "$APP_PID"
app_stop
