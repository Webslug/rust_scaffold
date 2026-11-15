# Schedules and rules: the real app runs a failing schedule with one retry
# and a rule that starts another schedule when it fails, then the
# Schedules, Rules and Next 24 hours views are screenshotted to check by
# eye. Run with --size 1280x800 (click positions are relative to the
# 1100x760 window). Sourced by tools/xvfb_check.sh.

# When Ollama answers on 127.0.0.1:11434 (already running; the check does
# not start it), a second rule asks the local model qwen3:1.7b after
# follow_up succeeds, so a rule chain reaches a real AI.
OLLAMA=0
curl -s --max-time 2 http://127.0.0.1:11434/api/tags >/dev/null && OLLAMA=1
AI_RULE=""
[ "$OLLAMA" = 1 ] && AI_RULE=', { "name": "then_ask", "when": { "schedule": "follow_up", "ended": "done" }, "then": { "action": "start", "schedule": "ask_ai" } }'
mkdir -p "$SETTINGS"
echo '{"width":1100,"height":760}' >"$SETTINGS/config.json"
cat >"$SETTINGS/jobs.json" <<JSON
{
  "version": 1,
  "schedules": [
    { "name": "flaky", "kind": "command", "input": { "script": "echo trying; exit 1" },
      "when": { "type": "every", "interval": "8s" }, "retries": 1, "retry_wait": "5s", "retry_doubling": false },
    { "name": "follow_up", "kind": "command", "input": { "script": "echo followed up" }, "when": { "type": "manual" } },
    { "name": "nightly", "kind": "command", "input": { "script": "echo night" }, "when": { "type": "daily", "time": "02:00" } },
    { "name": "ask_ai", "kind": "ai", "input": { "provider": "ollama", "model": "qwen3:1.7b", "prompt": "Reply with one word: the colour of the sky on a clear day." }, "when": { "type": "manual" } }
  ],
  "rules": [
    { "name": "flaky_failed", "when": { "schedule": "flaky", "ended": "failed" }, "then": { "action": "start", "schedule": "follow_up" } }$AI_RULE
  ]
}
JSON
app_start
key alt+6
sleep 1
shot sched_queue
click 439 91
# flaky: due at 8 s, fails, retried 5 s later, fails again (final), and the
# rule starts follow_up. Wait for the second timetable run too.
sleep 30
shot sched_schedules
python3 - "$SETTINGS/jobs_state.json" >"$OUT/state_check.txt" <<'PY'
import json, sys
s = json.load(open(sys.argv[1]))["schedules"]
flaky, follow = s.get("flaky", {}), s.get("follow_up", {})
bys = [h["by"] for h in flaky.get("history", [])]
print("flaky_retried", "retry 1 of 1" in bys)
print("flaky_timetable", bys.count("timetable") >= 2)
print("follow_by_rule", any(h["by"] == "rule flaky_failed" and h["state"] == "done" and "followed up" in (h.get("output") or "") for h in follow.get("history", [])))
print("flaky_label", any(h.get("label") == "exit_1" for h in flaky.get("history", [])))
print("nightly_untouched", s.get("nightly", {}).get("runs", 0) == 0)
ai = s.get("ask_ai", {}).get("history", [])
print("ai_answered", any(h["label"] == "answered" and h["by"] == "rule then_ask" and (h.get("output") or "").strip() for h in ai))
print("ai_answer", repr([h.get("output") for h in ai][:1]))
PY
cat "$OUT/state_check.txt"
for name in flaky_retried flaky_timetable follow_by_rule flaky_label nightly_untouched; do
    expect "jobs_state.json: $name" grep -qx "$name True" "$OUT/state_check.txt"
done
if [ "$OLLAMA" = 1 ]; then
    expect "jobs_state.json: the rule chain reached Ollama and kept its answer" grep -qx "ai_answered True" "$OUT/state_check.txt"
else
    echo "  (Ollama not answering on 127.0.0.1:11434: AI step skipped)"
fi
click 555 91
sleep 1
shot sched_rules
click 682 91
sleep 1
shot sched_upcoming
# The edit forms, through the real window. Add schedule focuses the name;
# Enter saves with the defaults (a command job with the example script,
# daily at 08:00).
click 439 91
sleep 1
click 344 149
sleep 1
type_text "Bad Name"
key Return
sleep 1
shot sched_form_refused
expect "a bad name is refused and jobs.json is not written" bash -c "! grep -q 'Bad Name' '$SETTINGS/jobs.json'"
key ctrl+a
type_text "made_by_check"
key Return
sleep 1
shot sched_form_saved
python3 - "$SETTINGS/jobs.json" >"$OUT/form_check.txt" <<'PY'
import json, sys
f = json.load(open(sys.argv[1]))
s = {x["name"]: x for x in f["schedules"]}
m = s.get("made_by_check", {})
print("schedule_saved", m.get("when") == {"type": "daily", "time": "08:00"} and m.get("kind") == "command" and m.get("input") == {"script": "df -h /", "folder": None})
print("others_kept", {"flaky", "follow_up", "nightly"} <= set(s))
PY
click 555 91
sleep 1
click 325 149
sleep 1
type_text "made_rule"
key Return
sleep 1
shot sched_rule_saved
python3 - "$SETTINGS/jobs.json" >>"$OUT/form_check.txt" <<'PY'
import json, sys
f = json.load(open(sys.argv[1]))
r = {x["name"]: x for x in f.get("rules", [])}
print("rule_saved", r.get("made_rule", {}).get("then", {}).get("action") == "start" and "flaky_failed" in r)
PY
# A rule with a value test: Tab from the name through schedule, ends and
# result to the value field.
click 325 149
sleep 1
type_text "value_rule"
key Tab Tab Tab Tab
type_text "percent"
key Tab
type_text "90"
sleep 1
shot sched_rule_value_form
key Return
sleep 1
python3 - "$SETTINGS/jobs.json" >>"$OUT/form_check.txt" <<'PY'
import json, sys
f = json.load(open(sys.argv[1]))
r = {x["name"]: x for x in f.get("rules", [])}
print("value_rule_saved", r.get("value_rule", {}).get("when", {}).get("value") == {"field": "percent", "above": 90})
PY
shot sched_rule_value_saved
cat "$OUT/form_check.txt"
for name in schedule_saved others_kept rule_saved value_rule_saved; do
    expect "form: $name" grep -qx "$name True" "$OUT/form_check.txt"
done
click 439 91
sleep 1
shot sched_schedules_after
app_stop
