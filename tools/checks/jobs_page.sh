# Jobs page, Queue view: runs jobs through the real window (Rust queue, job
# kinds, job_forms.js, jobs.js): the example command (df -h /), then a
# command that prints and fails with exit code 3, then an AI prompt to the
# local Ollama model (only when Ollama answers on 127.0.0.1:11434; it must
# already be running, and the check does not start it). Screenshots of the
# table and the Inspect card with each output, to check by eye.
# Run with --size 1280x800 (click positions are relative to the 1100x760
# window centred on that screen). Sourced by tools/xvfb_check.sh.

mkdir -p "$SETTINGS"
echo '{"width":1100,"height":760}' >"$SETTINGS/config.json"
app_start
key alt+6
sleep 1
# Submitted with Enter in the folder box (Tab from the script), so the
# check does not depend on where the Run now button sits.
click 655 336; key Tab Return
sleep 2
click 655 336; key ctrl+a
type_text 'echo before; echo broke >&2; exit 3'
key Tab Return
sleep 2
shot jobs_commands
if curl -s --max-time 2 http://127.0.0.1:11434/api/tags >/dev/null; then
    # Job select by keyboard: back from the script box, Down picks "AI prompt".
    # Tab: AI, the model list (installed text models), the server address,
    # whose Enter submits the form.
    click 655 336
    key shift+Tab Down
    sleep 2
    key Tab Tab Tab Return
    sleep 20
    shot jobs_ai
    # Switching the AI must not carry Ollama's model name over (it reached
    # Claude Code as "qwen3:1.7b" on 15 November): the model box empties,
    # and switching back restores it.
    key shift+Tab shift+Tab Down
    sleep 1
    shot jobs_ai_switched
    key Up
    sleep 1
    shot jobs_ai_switched_back
    click 1000 700; key End
    sleep 1
    shot jobs_table
    # Inspect the AI job (newest, top row), then scroll to its Output box.
    click 976 509
    sleep 1
    key End
    sleep 1
    shot jobs_inspect_ai
else
    echo "  (Ollama not answering on 127.0.0.1:11434: AI step skipped)"
fi
app_stop
