# Jobs page (Jobs 2.0): everything is done through the page itself, by
# typing and clicking, never by writing jobs.json.
#   1. A trigger "GBP watch" is made in the trigger editor: a local copy of
#      Kim's probe page (served on 127.0.0.1 for this check), id probe-gbp,
#      read as a number, checked every 30 seconds, "when it changes". Its
#      notification action is made with New action inside the trigger
#      editor. The page then changes from £3,577.30 to £3,600.00, and the
#      stand-in notification service on the private test bus
#      (fake_notifications.py) must receive "Now £3,600.00" / "Was
#      £3,577.30". Nothing reaches the desktop.
#   2. Running now and Kill: a command action (sleep 60) made on the
#      Actions view, run with Run, killed from the Running now strip.
#   3. Duplicate (the copy is off) and Delete (two clicks) of the trigger.
#   4. After a restart the trigger, its reading and the action's result are
#      still there. Fire now shows the notification at once, without
#      waiting for the next check.
#   5. Overview: a section dragged by its handle (order saved in
#      config.json), a row clicked opens its editor, then the quick buttons
#      Enable all, Disable all, Delete all (two presses) and Restore demos.
#   6. Menu search (Ctrl+K) finds an action by name and opens its editor.
#   7. A Hardware-Watch trigger made through the form: a file, checked every
#      5 seconds; creating the file fires it.
#   8. A schedule whose date is chosen in the date and time picker.
# Run with --size 1920x1080: Kim's window size (960x1028) is used and the
# click positions are relative to that window. Screenshots in $OUT are for
# checking by eye. Sourced by tools/xvfb_check.sh.

CHECK_DIR="$(dirname "${BASH_SOURCE[0]}")"
mkdir -p "$SETTINGS" "$SCRATCH/site/probe"
echo '{"width":960,"height":1028}' >"$SETTINGS/config.json"

# The probe page, as on bitmods.com (16 November 2026).
probe_page() {
    printf '<!doctype html>\n<html><head><meta charset="utf-8"><title>probe</title></head><body>\n<p><span id="probe-token">9ecf08f2b581cf6059594fdc6f3acee0</span></p>\n<p><span id="probe-gbp">%s</span></p>\n<p><span id="probe-usd">$5,675.46</span></p>\n</body></html>\n' "$1" \
        >"$SCRATCH/site/probe/index.html"
}
probe_page '£3,577.30'
PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
# Stops on its own after 10 minutes, should the check stop half way.
timeout 600 python3 -m http.server "$PORT" --bind 127.0.0.1 --directory "$SCRATCH/site" >/dev/null 2>&1 &
SITE_PID=$!

NOTES="$SETTINGS/notifications.jsonl"
: >"$NOTES"
python3 "$CHECK_DIR/fake_notifications.py" "$PRIVATE_BUS" "$NOTES" &
FAKE_PID=$!
sleep 1

# Reads a value from jobs.json or jobs_state.json: json_get FILE PYTHON_EXPRESSION
# (the file is `f`), printing the result.
json_get() {
    python3 -c "import json,sys; f=json.load(open(sys.argv[1])); print($2)" "$1" 2>/dev/null || true
}

app_start
key alt+6
sleep 2
shot jobs_start

# ---- 1. Trigger and its action, through the forms --------------------------
click 450 90                        # Triggers view (the page opens on the Overview)
sleep 1
click 337 141                       # New trigger
sleep 1
type_text 'GBP watch'
key Tab Tab                         # past Watch (a value on a web page)
type_text "http://127.0.0.1:$PORT/probe/"
key Tab Tab                         # past Find by (id)
type_text 'probe-gbp'
key Tab Down                        # Read as: Number
key Tab ctrl+a
type_text '30'
key Tab Up                          # minutes -> seconds
sleep 0.5
shot trigger_filled
click 552 850                       # New action, inside the trigger editor
sleep 1
shot action_inside
type_text 'Tell me GBP'
key Return                          # saves the action; focus moves to the trigger's Save
sleep 1
shot trigger_with_action
key Return                          # saves the trigger
sleep 2
shot trigger_saved

FILE="$SETTINGS/jobs.json"
STATE="$SETTINGS/jobs_state.json"
expect "the trigger was saved from the form" \
    test "$(json_get "$FILE" '[t for t in f["triggers"] if t["name"]=="GBP watch"][0]["watch"]["name"]')" = probe-gbp
expect "it checks every 30 seconds, reads a number, starts Tell me GBP" \
    test "$(json_get "$FILE" '[(t["every_seconds"], t["watch"]["read_as"], t["then"]) for t in f["triggers"] if t["name"]=="GBP watch"][0]')" = "(30, 'number', ['Tell me GBP'])"
expect "the action was saved from the form inside it" \
    test "$(json_get "$FILE" '[a["kind"] for a in f["actions"] if a["name"]=="Tell me GBP"][0]')" = notify

# The first check after saving only records the value.
for _ in $(seq 1 15); do
    [ -n "$(json_get "$STATE" 'f["triggers"]["GBP watch"]["last_value"]["text"]')" ] && break
    sleep 1
done
expect "the first reading was recorded" \
    test "$(json_get "$STATE" 'f["triggers"]["GBP watch"]["last_value"]["number"]')" = 3577.3
expect "and showed no notification" test ! -s "$NOTES"
shot trigger_first_reading

probe_page '£3,600.00'
echo "  page changed to £3,600.00; waiting for the next check (30 s)"
for _ in $(seq 1 45); do
    [ -s "$NOTES" ] && break
    sleep 1
done
sleep 1
shot trigger_fired
cat "$NOTES" | sed 's/^/  notification: /'
expect "the change was noticed and the notification arrived with both values" python3 -c "
import json,sys
n=[json.loads(l) for l in open(sys.argv[1])]
sys.exit(0 if (n[0]['title'], n[0]['text']) == ('Now £3,600.00', 'Was £3,577.30') else 1)" "$NOTES"
expect "exactly one notification" test "$(wc -l <"$NOTES")" -eq 1
expect "the trigger counts one fire" test "$(json_get "$STATE" 'f["triggers"]["GBP watch"]["fires"]')" = 1
expect "the action's result was kept" \
    test "$(json_get "$STATE" 'f["actions"]["Tell me GBP"]["history"][-1]["state"]')" = done

# ---- 2. Running now and Kill -------------------------------------------------
click 708 90                        # Actions view
sleep 1
click 334 170                       # New action
sleep 1
type_text 'Wait a minute'
key Tab Down                        # Kind: Notification -> Command
sleep 0.5
key Tab ctrl+a
type_text 'echo waiting; sleep 60'
key Tab Return                      # Enter in the folder box saves
sleep 2
shot action_saved
click 809 435                       # Run "Wait a minute"
sleep 2
shot running_now
expect "Running now lists the command while it runs" \
    test "$(json_get "$STATE" 'len(f["actions"].get("Wait a minute", {}).get("history", []))')" = 0
click 879 145                       # Kill, in the Running now strip
sleep 3
shot killed
expect "Kill stopped it: its result is Killed" \
    test "$(json_get "$STATE" 'f["actions"]["Wait a minute"]["history"][-1]["state"]')" = cancelled
# -x: the whole command line must be exactly "sleep 60" (a pattern search
# would find this check's own command line).
expect "no sleep 60 left running" bash -c '! pgrep -u "$(id -u)" -fx "sleep 60"'

# ---- 3. Duplicate and Delete ----------------------------------------------------
click 450 90                        # Triggers view
sleep 1
click 855 470                       # Duplicate "GBP watch"
sleep 2
shot duplicated
expect "Duplicate made \"GBP watch copy\", switched off" \
    test "$(json_get "$FILE" '[t["enabled"] for t in f["triggers"] if t["name"]=="GBP watch copy"]')" = "[False]"
click 891 608                       # Delete "GBP watch copy": first click asks
sleep 0.5
shot delete_asks
expect "the first click deletes nothing" \
    test "$(json_get "$FILE" 'len(f["triggers"])')" = 3
# The pointer moves away and back before the second click. On the hidden
# screen a second click at exactly the same spot with no movement in between
# is lost (the button was not replaced, and no tooltip change; cause not
# found). A hand on a real mouse always moves it a little. Not yet tried by
# hand on Kim's desktop.
x mousemove --window "$APP_WIN" 600 700
sleep 0.3
click 891 608                       # second click deletes
sleep 1.5
shot deleted
expect "the second click deleted the copy and nothing else" \
    test "$(json_get "$FILE" '[t["name"] for t in f["triggers"]]')" = "['GBP price', 'GBP watch']"

# ---- 4. Everything is kept across a restart ------------------------------------
app_stop
app_start
key alt+6
sleep 4
shot after_restart
expect "after a restart the trigger is still there and on" \
    test "$(json_get "$FILE" '[t["enabled"] for t in f["triggers"] if t["name"]=="GBP watch"]')" = "[True]"
expect "its reading is kept" \
    test "$(json_get "$STATE" 'f["triggers"]["GBP watch"]["last_value"]["text"]')" = '£3,600.00'
expect "the action results are kept" \
    test "$(json_get "$STATE" '[h["state"] for h in f["actions"]["Wait a minute"]["history"]]')" = "['cancelled']"
expect "an unchanged page after the restart gives no second notification" test "$(wc -l <"$NOTES")" -eq 1

# ---- 4b. Fire now: no waiting for the next check ------------------------------
click 450 90                        # Triggers view
sleep 1
click 859 405                       # Fire now on "GBP watch" (the page has not changed)
for _ in $(seq 1 15); do
    [ "$(wc -l <"$NOTES")" -ge 2 ] && break
    sleep 1
done
shot fired_by_hand
expect "Fire now showed a notification at once, with the reading and the one before the last change" python3 -c "
import json,sys
n=[json.loads(l) for l in open(sys.argv[1])]
sys.exit(0 if len(n) == 2 and (n[1]['title'], n[1]['text']) == ('Now £3,600.00', 'Was £3,577.30') else 1)" "$NOTES"
expect "a fire by hand does not count towards fire at most" \
    test "$(json_get "$STATE" 'f["triggers"]["GBP watch"]["fires"]')" = 1
click 327 90                        # back to the Overview; a status line is now shown
sleep 1
# ---- 5. Overview ---------------------------------------------------------------
# The Jobs page opens on the Overview. Drag the Triggers section down by its
# handle; the new order is saved in config.json.
CONFIG="$SETTINGS/config.json"
x mousemove --sync --window "$APP_WIN" 283 335
x mousedown 1
for y in 370 430 490 550 610 670 730 790; do x mousemove --sync --window "$APP_WIN" 283 $y; sleep 0.1; done
x mouseup 1
sleep 1.5
shot overview_dragged
expect "dragging a section saves the new order" \
    test "$(json_get "$CONFIG" 'f["order"]["jobs-overview"][0]')" != triggers
# Clicking a row opens that entry's editor; Enter saves it, which starts the
# trigger afresh (its fire count goes back to 0).
click 442 703                       # the "GBP watch" row
sleep 1
shot overview_row_opened
key Return
sleep 1.5
expect "clicking a row opened its editor (saved with Enter: fires back to 0)" \
    test "$(json_get "$STATE" 'f["triggers"]["GBP watch"]["fires"]')" = 0
click 327 90                        # back to the Overview; a status line is now shown
sleep 1
# The quick buttons. Enable all also switches on the "GBP price" demo, which
# then asks the real bitmods.com/probe once; nothing here depends on it.
click 328 225                       # Enable all
sleep 1.5
expect "Enable all switched every trigger on" \
    test "$(json_get "$FILE" '[t["enabled"] for t in f["triggers"]]')" = "[True, True]"
click 462 225                       # Disable all
sleep 1.5
expect "Disable all switched every trigger off" \
    test "$(json_get "$FILE" '[t["enabled"] for t in f["triggers"]]')" = "[False, False]"
click 765 225                       # Delete all: first press asks
sleep 0.5
x mousemove --window "$APP_WIN" 600 700
sleep 0.3
expect "the first press of Delete all deletes nothing" test "$(json_get "$FILE" 'len(f["actions"])')" = 3
click 765 225                       # second press deletes
sleep 1.5
shot overview_deleted_all
expect "the second press deleted every trigger, schedule and action" \
    test "$(json_get "$FILE" '[len(f["triggers"]), len(f["schedules"]), len(f["actions"])]')" = "[0, 0, 0]"
click 615 225                       # Restore demos
sleep 1.5
shot overview_restored
expect "Restore demos brought back the demonstration trigger and its action" \
    test "$(json_get "$FILE" '[t["name"] for t in f["triggers"]] + [a["name"] for a in f["actions"]]')" = "['GBP price', 'Notify GBP price']"

# ---- 6. Menu search finds jobs by name ------------------------------------------
# Ctrl+K, "Notify GBP", Enter opens that action's editor with the cursor in
# its name; typing " 2" and Enter renames it, which proves the right editor
# opened. The trigger's list follows the rename.
key ctrl+k
type_text 'Notify GBP'
sleep 1
shot search_results
key Return
sleep 1
type_text ' 2'
key Return
sleep 1.5
shot search_opened
expect "search opened the action's editor (renamed through it)" \
    test "$(json_get "$FILE" '[a["name"] for a in f["actions"]]')" = "['Notify GBP price 2']"
expect "the trigger's list followed the rename" \
    test "$(json_get "$FILE" 'f["triggers"][0]["then"]')" = "['Notify GBP price 2']"

# ---- 7. A hardware trigger: a file, made through the form ------------------------
WATCHED="$SCRATCH/watched.txt"
click 450 90                        # Triggers view
sleep 1
click 337 171                       # New trigger (below the status line)
sleep 1
type_text 'Notes file'
key Tab Down Down Down Down Down    # Watch: A file or folder
sleep 0.5
key Tab
type_text "$WATCHED"
key Tab Tab ctrl+a                  # past Watch for (changes to it), to Check every
type_text '5'
key Tab Up                          # minutes -> seconds
key Tab Tab Tab Tab Down            # When, Fire at most, Test now, then "Add an action": the first one
sleep 0.5
shot file_trigger_filled
click 525 861                       # Save
sleep 2
shot file_trigger_saved
expect "the file trigger was saved from the form" \
    test "$(json_get "$FILE" '[(t["watch"]["type"], t["every_seconds"], t["then"]) for t in f["triggers"] if t["name"]=="Notes file"]')" = "[('file', 5, ['Notify GBP price 2'])]"
for _ in $(seq 1 10); do
    [ "$(json_get "$STATE" 'f["triggers"]["Notes file"]["last_value"]["text"]')" = missing ] && break
    sleep 1
done
expect "its first reading (no file yet) was recorded" \
    test "$(json_get "$STATE" 'f["triggers"]["Notes file"]["last_value"]["text"]')" = missing
before=$(wc -l <"$NOTES")
echo "hello" >"$WATCHED"
for _ in $(seq 1 15); do
    [ "$(wc -l <"$NOTES")" -gt "$before" ] && break
    sleep 1
done
expect "creating the file fired it: a notification about the file arrived" python3 -c "
import json,sys
n=[json.loads(l) for l in open(sys.argv[1])][-1]
sys.exit(0 if n['title'].startswith('GBP now 6 bytes, modified') and n['text'] == 'It was missing' else 1)" "$NOTES"
tail -n 1 "$NOTES" | sed 's/^/  notification: /'

# ---- 8. A schedule with the date picker --------------------------------------------
click 585 90                        # Schedules view
sleep 1
click 345 170                       # New schedule (below the status line)
sleep 1
type_text 'Monthly report'
key Tab Up Up                       # Run: daily -> once, at
sleep 0.5
key Tab                             # into Date and time: the picker opens
sleep 1
shot picker_open
click 917 397                       # next month
sleep 0.5
# Where the 15th of next month sits: the grid starts on Monday, 7 columns
# about 37 px apart, rows about 39 px apart (window-relative, status line shown).
read -r DAY_X DAY_Y WANT <<<"$(python3 -c "
import datetime
t = datetime.date.today()
first = (t.replace(day=1) + datetime.timedelta(days=32)).replace(day=1)
i = first.weekday() + 14
print(708 + 37 * (i % 7), 467 + 39 * (i // 7), first.strftime('%Y-%m-15'))")"
click "$DAY_X" "$DAY_Y"             # the 15th: written into the box at once
sleep 0.5
shot picker_day_chosen
key Escape                          # close the picker; the cursor is back in the box
key Return                          # Enter saves the schedule
sleep 1.5
shot schedule_saved
expect "the date picker filled in the date (the 15th of next month), saved through the form" \
    test "$(json_get "$FILE" '[s["when"]["at"][:10] for s in f["schedules"] if s["name"]=="Monthly report"]')" = "['$WANT']"

app_stop
kill "$FAKE_PID" "$SITE_PID" 2>/dev/null || true
