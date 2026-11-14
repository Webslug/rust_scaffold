# Settings files: nothing copied from the executable's folder, oversized
# files set aside as NAME.json.corrupt and recreated, a deleted settings
# folder recreated. Uses the dirs crate path (no override), under the
# scratch XDG_CONFIG_HOME. Sourced by tools/xvfb_check.sh.
# options.json is only read when the Options tab opens, so the check opens
# it with Alt+8 (the default keys.json binding).

for f in config options search keys; do
    echo '{"from":"exe"}' >"$BIN_DIR/$f.json"
done

app_start xdg
expect "settings folder is the dirs path" test -d "$SETTINGS"
for f in config.json keys.json search.json; do
    expect "$f created" test -s "$SETTINGS/$f"
    expect "$f not copied from the executable's folder" bash -c "! grep -q '\"from\"' '$SETTINGS/$f'"
done
app_stop

# Over the limits: config 5 MiB, search 1 MiB, keys 64 KiB.
head -c $((5 * 1024 * 1024 + 1)) /dev/zero | tr '\0' ' ' >"$SETTINGS/config.json"
head -c $((1024 * 1024 + 1)) /dev/zero | tr '\0' ' ' >"$SETTINGS/search.json"
head -c $((64 * 1024 + 1)) /dev/zero | tr '\0' ' ' >"$SETTINGS/keys.json"
head -c $((1024 * 1024 + 1)) /dev/zero | tr '\0' ' ' >"$SETTINGS/options.json"
app_start xdg
expect "options.json not touched before the Options tab opens" test ! -e "$SETTINGS/options.json.corrupt"
key alt+8
sleep 1
shot options_recreated
for pair in config.json:$((5 * 1024 * 1024 + 1)) search.json:$((1024 * 1024 + 1)) keys.json:$((64 * 1024 + 1)) options.json:$((1024 * 1024 + 1)); do
    f="${pair%%:*}"; size="${pair##*:}"
    expect "$f set aside as $f.corrupt ($size bytes)" test "$(stat -c %s "$SETTINGS/$f.corrupt" 2>/dev/null)" = "$size"
    expect "$f recreated small" test "$(stat -c %s "$SETTINGS/$f")" -lt 65536
done
app_stop

rm -rf "$SETTINGS"
app_start xdg
for f in config.json keys.json search.json; do
    expect "$f recreated after the folder was deleted" test -s "$SETTINGS/$f"
done
app_stop
