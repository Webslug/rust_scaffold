# Four warnings logged while the app starts (oversized settings files) must
# all be counted by the Errors badge, then listed on the Errors page.
# Checked by eye: the badge in errors_badge.png should read 4, and the page
# in errors_page.png should say "4 kept". Sourced by tools/xvfb_check.sh.

app_start
app_stop
head -c $((5 * 1024 * 1024 + 1)) /dev/zero | tr '\0' ' ' >"$SETTINGS/config.json"
head -c $((1024 * 1024 + 1)) /dev/zero | tr '\0' ' ' >"$SETTINGS/search.json"
head -c $((64 * 1024 + 1)) /dev/zero | tr '\0' ' ' >"$SETTINGS/keys.json"
head -c $((1024 * 1024 + 1)) /dev/zero | tr '\0' ' ' >"$SETTINGS/options.json"
app_start
key alt+8
sleep 1
shot errors_badge
key alt+7
sleep 1
shot errors_page
app_stop
