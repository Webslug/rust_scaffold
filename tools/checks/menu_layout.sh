# Screenshots of the menu in both layouts, to check by eye that Options and
# Help sit together at the end (bottom of the vertical menu, right of the
# tab row). Run with --size 1280x800 or larger for the wide tab row.
# Sourced by tools/xvfb_check.sh.

mkdir -p "$SETTINGS"
echo '{"menu_layout":"vertical","width":900,"height":640}' >"$SETTINGS/config.json"
app_start
shot menu_vertical
app_stop

echo '{"menu_layout":"horizontal","width":1200,"height":640}' >"$SETTINGS/config.json"
app_start
shot menu_horizontal
app_stop

echo '{"menu_layout":"horizontal","width":640,"height":480}' >"$SETTINGS/config.json"
app_start
shot menu_horizontal_small
app_stop
