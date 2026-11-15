# Pipelines in the real app: a job started by a rule is given the result of
# the job before it. Needs Ollama already running on 127.0.0.1:11434 with
# qwen3:1.7b (the check never starts it); skipped otherwise.
#   1. gather prints three ERROR lines among others; a rule gives them to
#      qwen3 ({previous_output} in the prompt) to count; a second rule gives
#      the answer to a script ($PREVIOUS_OUTPUT), which saves it to a file.
#   2. disk runs `df -h /`; a rule asks qwen3 how full the disk is.
# The answers are printed (a model's wording varies, so only their presence
# and the hand-over are asserted). Sourced by tools/xvfb_check.sh.

if ! curl -s --max-time 2 http://127.0.0.1:11434/api/tags | grep -q '"qwen3:1.7b"'; then
    echo "  (Ollama with qwen3:1.7b not answering on 127.0.0.1:11434: pipeline check skipped)"
    return 0
fi
mkdir -p "$SETTINGS"
cat >"$SETTINGS/jobs.json" <<'JSON'
{
  "version": 1,
  "schedules": [
    { "name": "gather", "kind": "command", "when": { "type": "every", "interval": "5s" }, "max_runs": 1,
      "input": { "script": "printf '%s\\n' 'INFO started' 'ERROR disk full on /data' 'INFO user login' 'ERROR fan stopped' 'WARN slow reply' 'ERROR backup failed'" } },
    { "name": "count_errors", "kind": "ai", "when": { "type": "manual" },
      "input": { "provider": "ollama", "model": "qwen3:1.7b",
        "prompt": "Here are log lines from the schedule {previous_schedule}:\n{previous_output}\n\nHow many lines start with ERROR? Reply with the number only." } },
    { "name": "save_answer", "kind": "command", "when": { "type": "manual" },
      "input": { "script": "printf '%s' \"$PREVIOUS_OUTPUT\" > pipeline_answer.txt; echo \"saved answer from $PREVIOUS_SCHEDULE ($PREVIOUS_LABEL)\"" } },
    { "name": "disk", "kind": "command", "when": { "type": "every", "interval": "5s" }, "max_runs": 1,
      "input": { "script": "df -h /" } },
    { "name": "disk_report", "kind": "ai", "when": { "type": "manual" },
      "input": { "provider": "ollama", "model": "qwen3:1.7b",
        "prompt": "This is the output of df -h for the root file system:\n{previous_output}\n\nIn one short sentence: how full is it, as a percentage?" } }
  ],
  "rules": [
    { "name": "ask_ai", "when": { "schedule": "gather", "ended": "done" }, "then": { "action": "start", "schedule": "count_errors" } },
    { "name": "then_save", "when": { "schedule": "count_errors", "ended": "done", "label": "answered" }, "then": { "action": "start", "schedule": "save_answer" } },
    { "name": "report", "when": { "schedule": "disk", "ended": "done" }, "then": { "action": "start", "schedule": "disk_report" } }
  ]
}
JSON
# The folder save_answer runs in: the scratch settings folder.
python3 - "$SETTINGS/jobs.json" <<'PY'
import json, sys
p = sys.argv[1]; d = json.load(open(p))
for s in d["schedules"]:
    if s["name"] == "save_answer":
        s["input"]["folder"] = __import__("os").path.dirname(p)
json.dump(d, open(p, "w"), indent=2)
PY
app_start
# Both chains start at 5 s; local AI jobs run one at a time.
for _ in $(seq 1 60); do
    [ -s "$SETTINGS/pipeline_answer.txt" ] && grep -q '"disk_report"' "$SETTINGS/jobs_state.json" 2>/dev/null \
        && python3 -c "import json,sys; s=json.load(open(sys.argv[1]))['schedules']; sys.exit(0 if s.get('disk_report',{}).get('history') else 1)" "$SETTINGS/jobs_state.json" && break
    sleep 1
done
python3 - "$SETTINGS/jobs_state.json" "$SETTINGS/pipeline_answer.txt" >"$OUT/pipeline.txt" <<'PY'
import json, sys, os
s = json.load(open(sys.argv[1]))["schedules"]
def last(name):
    h = s.get(name, {}).get("history", [])
    return h[-1] if h else {}
count, save, report = last("count_errors"), last("save_answer"), last("disk_report")
saved = open(sys.argv[2]).read() if os.path.exists(sys.argv[2]) else ""
print("count_answered", count.get("label") == "answered" and count.get("by") == "rule ask_ai")
print("save_got_answer", save.get("state") == "done" and "saved answer from count_errors (answered)" in (save.get("output") or ""))
print("file_holds_answer", bool(saved.strip()) and saved.strip() == (count.get("output") or "").strip())
print("report_answered", report.get("label") == "answered" and report.get("by") == "rule report")
print("ANSWER count_errors:", repr((count.get("output") or "").strip()))
print("ANSWER disk_report:", repr((report.get("output") or "").strip()))
PY
cat "$OUT/pipeline.txt"
for name in count_answered save_got_answer file_holds_answer report_answered; do
    expect "pipeline: $name" grep -qx "$name True" "$OUT/pipeline.txt"
done
df -h / | tail -n 1 | sed 's/^/  real df: /'
app_stop
