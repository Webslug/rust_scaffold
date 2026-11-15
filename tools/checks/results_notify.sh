# Job results, value rules, notifications and kept errors, in the real app.
#   1. disk reports {"percent": N} through $JOB_RESULT (50, then 95, then
#      97). A rule "percent above 90, twice in a row" starts tell, a notify
#      job whose title and text quote the value and message. A stand-in
#      notification service on the private test bus (fake_notifications.py)
#      records what arrives; nothing reaches the desktop.
#   2. If Ollama answers with qwen3:1.7b, the same rule also asks the model
#      to react ({previous_value.percent} in the prompt), and its answer is
#      shown by a second notify job. Skipped otherwise.
#   3. Errors survive a restart: errors.jsonl holds the warnings of the
#      first run, and the second run lists them (greyed, checked by eye in
#      errors_kept.png).
# Sourced by tools/xvfb_check.sh.

CHECK_DIR="$(dirname "${BASH_SOURCE[0]}")"
mkdir -p "$SETTINGS"
printf '50\n95\n97\n' >"$SETTINGS/numbers.txt"
OLLAMA=no
if curl -s --max-time 2 http://127.0.0.1:11434/api/tags | grep -q '"qwen3:1.7b"'; then
    OLLAMA=yes
fi
python3 - "$SETTINGS" "$OLLAMA" <<'PY'
import json, os, sys
settings, ollama = sys.argv[1], sys.argv[2] == "yes"
numbers = os.path.join(settings, "numbers.txt")
script = (f"f='{numbers}'; v=$(head -n 1 \"$f\"); sed -i 1d \"$f\"; "
          "printf '{\"value\":{\"percent\":%s},\"message\":\"The disk is %s%% full.\"}' \"$v\" \"$v\" > \"$JOB_RESULT\"; echo checked")
schedules = [
    {"name": "disk", "kind": "command", "when": {"type": "every", "interval": "5s"}, "max_runs": 3, "input": {"script": script}},
    {"name": "tell", "kind": "notify", "when": {"type": "manual"},
     "input": {"title": "Disk {previous_value.percent}% full", "text": "{previous_message}", "urgency": "critical", "sound": True}},
]
rules = [{"name": "too_full", "when": {"schedule": "disk", "ended": "done", "value": {"field": "percent", "above": 90}, "times": 2},
          "then": {"action": "start", "schedule": "tell"}}]
if ollama:
    schedules += [
        {"name": "react", "kind": "ai", "when": {"type": "manual"},
         "input": {"provider": "ollama", "model": "qwen3:1.7b",
                   "prompt": "A disk check reports the disk is {previous_value.percent} percent full. In one short sentence, say how urgent that is."}},
        {"name": "tell_ai", "kind": "notify", "when": {"type": "manual"}, "input": {"title": "AI says", "text": "{previous_output}"}},
    ]
    rules += [
        {"name": "ask_ai", "when": {"schedule": "disk", "ended": "done", "value": {"field": "percent", "above": 90}, "times": 2},
         "then": {"action": "start", "schedule": "react"}},
        {"name": "show_ai", "when": {"schedule": "react", "ended": "done"}, "then": {"action": "start", "schedule": "tell_ai"}},
    ]
json.dump({"version": 1, "schedules": schedules, "rules": rules}, open(os.path.join(settings, "jobs.json"), "w"), indent=2)
PY

NOTES="$SETTINGS/notifications.jsonl"
: >"$NOTES"
python3 "$CHECK_DIR/fake_notifications.py" "$PRIVATE_BUS" "$NOTES" &
FAKE_PID=$!
sleep 1
app_start
want=1
[ "$OLLAMA" = yes ] && want=2
for _ in $(seq 1 90); do
    [ "$(wc -l <"$NOTES")" -ge "$want" ] && break
    sleep 1
done
sleep 1
python3 - "$SETTINGS/jobs_state.json" "$NOTES" >"$OUT/results_notify.txt" <<'PY'
import json, sys
state = json.load(open(sys.argv[1]))["schedules"]
for name in ("disk", "tell", "react", "tell_ai"):
    for h in state.get(name, {}).get("history", []):
        print(f"{name}: {h['state']} {h.get('label')} value={json.dumps(h.get('value'))} by={h['by']}")
for line in open(sys.argv[2]):
    print("notification:", line.strip())
PY
cat "$OUT/results_notify.txt"
expect "disk history keeps the reported values" grep -q 'disk: done exit_0 value={"percent": 97}' "$OUT/results_notify.txt"
expect "value rule started tell once, after two runs over 90" \
    test "$(grep -c '^tell: done shown' "$OUT/results_notify.txt")" -eq 1
expect "notification carries the filled title, text, urgency and sound" \
    grep -q '"title": "Disk 97% full", "text": "The disk is 97% full.", "urgency": 2, "sound": "message-new-instant"' "$NOTES"
if [ "$OLLAMA" = yes ]; then
    expect "the AI reacted and its answer was shown" grep -q '^tell_ai: done shown' "$OUT/results_notify.txt"
    expect "the AI's notification has text" python3 -c "
import json,sys
lines=[json.loads(l) for l in open(sys.argv[1])]
sys.exit(0 if any(l['title']=='AI says' and l['text'].strip() for l in lines) else 1)" "$NOTES"
else
    echo "  (Ollama with qwen3:1.7b not answering: AI reaction part skipped)"
fi
key alt+7
sleep 1
shot results_errors_first
app_stop
kill "$FAKE_PID" 2>/dev/null || true

# 3. Errors kept across a restart. The first run logged at least one
# warning (the tray cannot be created on the private bus).
expect "errors.jsonl was written" test -s "$SETTINGS/errors.jsonl"
first=$(wc -l <"$SETTINGS/errors.jsonl")
app_start
sleep 2
key alt+7
sleep 1
shot errors_kept
app_stop
second=$(wc -l <"$SETTINGS/errors.jsonl")
echo "  errors.jsonl: $first lines after the first run, $second after the second"
expect "the second run kept the first run's entries and added its own" test "$second" -gt "$first"
