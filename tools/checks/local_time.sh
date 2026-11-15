# Local time on the Errors page. Runs the app with TZ=Asia/Tokyo (UTC+9, no
# daylight saving) and screenshots the Errors page with two entries: the
# tray warning logged during start-up (private bus, no tray host), and an
# oversized options.json warning logged later, when Options opens, so the
# second one is certainly logged while many threads run. Compare both times
# with the two printed here, by eye: Tokyo time means local time works, UTC
# means the app fell back to UTC.
# Sourced by tools/xvfb_check.sh.

export TZ=Asia/Tokyo
echo "  expected local (Tokyo): $(date '+%Y-%m-%d %H:%M')"
echo "  UTC fallback would be:  $(date -u '+%Y-%m-%d %H:%M')"
mkdir -p "$SETTINGS"
head -c $((1024 * 1024 + 1)) /dev/zero | tr '\0' ' ' >"$SETTINGS/options.json"
app_start
sleep 2
key alt+8
sleep 1
key alt+7
sleep 1
shot local_time_errors
app_stop
unset TZ
