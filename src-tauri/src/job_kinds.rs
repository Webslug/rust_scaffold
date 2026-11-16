//! Every kind of job the app can run, and the list the queue uses.
//!
//! To add a kind: write a type with `impl JobKind`, then add one line,
//! `kind::<NewJob>()`, to `KINDS`. The queue (`src/jobs.rs`) does not change.
//! The window starts a job with the kind's name and its input as JSON; only
//! kinds in this list can run.
//!
//! - `command`: runs a shell script (bash on Linux, cmd on Windows) in any
//!   folder, capturing what it prints. The exit code is the result label
//!   (`exit_0`, `exit_1` ...), unless the script
//!   reports its own label, value and message as JSON in the file
//!   `$JOB_RESULT`. Progress written to `$JOB_PROGRESS` fills the bar.
//! - `notify`: a desktop notification (Linux, over D-Bus).
//!
//! Passing values on: a job started by a Trigger, a Schedule or another
//! Action is given the Trigger's reading and the output of the Action before
//! it (`JobCtx::given`, see `jobs::Given`). A command gets them as
//! environment variables (`JOB_FROM`, `JOB_VALUE`, `JOB_PREVIOUS`,
//! `JOB_OUTPUT`, `JOB_OUTPUT_FILE`), never pasted into the script, so text
//! from a web page or an AI can never run as code. An AI prompt or a
//! notification gets them through placeholders: `{value}`, `{value.FIELD}`,
//! `{previous}`, `{previous.FIELD}`, `{output}` and `{from}` (see
//! `fill_text`).
//! - `ai`: sends a prompt to Claude Code, Codex or GrokBuild (their command
//!   line tools, read-only) or to a local Ollama or llama.cpp server. The
//!   answer is the job's output. OpenRouter is left out on purpose: it needs
//!   HTTPS, which would mean adding a TLS crate; the local servers are
//!   plain HTTP, handled by the small client at the end of this file.

use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::time::Instant;

use crate::jobs::{kind, tail_start, Capability, Given, JobCtx, JobKind, JobResult, KindEntry, Outcome};

/// Every kind that may be started, by name.
pub static KINDS: &[KindEntry] = &[kind::<CommandJob>(), kind::<AiJob>(), kind::<NotifyJob>()];

/// The app's kinds plus `count`, a kind kept only for the queue and
/// automation tests (it needs no programs and runs on the paused clock).
#[cfg(test)]
pub static TEST_KINDS: &[KindEntry] =
    &[kind::<CommandJob>(), kind::<AiJob>(), kind::<NotifyJob>(), kind::<test_kind::CountJob>()];

/// Longest script or prompt.
const MAX_TEXT_BYTES: usize = 60 * 1024;
/// The end of the earlier output a command gets in `JOB_OUTPUT` (Linux
/// refuses a single environment variable over 128 KiB; the whole output is
/// in the file `JOB_OUTPUT_FILE`).
const ENV_OUTPUT_BYTES: usize = 100 * 1024;
/// The end of the earlier output put into an AI prompt. Small local models
/// read about 4,000 tokens (Ollama's default context) and drop the start of
/// a longer prompt, instructions included; 12,000 bytes is about 3,000
/// tokens, leaving room for the instructions.
pub const PROMPT_OUTPUT_BYTES: usize = 12_000;
/// Largest `$JOB_RESULT` file a script may write.
const MAX_RESULT_BYTES: u64 = 16 * 1024;
/// How often `$JOB_PROGRESS` is read while a script runs.
const PROGRESS_EVERY: Duration = Duration::from_millis(500);
/// How often a running job's last output line is sent to the window.
const NOTE_EVERY: Duration = Duration::from_millis(250);

// ---- Shared: folders, PATH, running a program ---------------------------------

/// "" or None: the home folder; "~/x": x in the home folder; anything
/// else as given. Any folder on the system may be used.
fn resolve_folder(folder: Option<&str>) -> Result<PathBuf, String> {
    let home = dirs::home_dir();
    let path = match folder.map(str::trim).filter(|f| !f.is_empty()) {
        None => home.ok_or("no home folder found; give a folder")?,
        Some("~") => home.ok_or("no home folder found")?,
        Some(f) if f.starts_with("~/") => home.ok_or("no home folder found")?.join(&f[2..]),
        Some(f) => PathBuf::from(f),
    };
    if !path.is_dir() {
        return Err(format!("the folder {} does not exist", path.display()));
    }
    Ok(path)
}

#[cfg(test)]
static TEST_BIN_DIR: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// The PATH the app was started with, plus the per-user tool folders a
/// desktop launcher leaves out (~/.local/bin, ~/.npm-global/bin, nvm's
/// newest Node, ...), so `claude`, `codex` and `grok` are found however
/// the app was started. The same list as Concilium uses.
fn search_path() -> OsString {
    let mut dirs: Vec<PathBuf> = Vec::new();
    #[cfg(test)]
    if let Some(dir) = TEST_BIN_DIR.lock().unwrap().clone() {
        dirs.push(dir);
    }
    dirs.extend(std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default());
    if let Some(home) = dirs::home_dir() {
        for extra in [".npm-global/bin", ".local/bin", ".grok/bin", ".bun/bin"] {
            dirs.push(home.join(extra));
        }
        for nvm in [".config/nvm/versions/node", ".nvm/versions/node"] {
            let mut nodes: Vec<PathBuf> = std::fs::read_dir(home.join(nvm))
                .map(|list| list.flatten().map(|e| e.path().join("bin")).collect())
                .unwrap_or_default();
            nodes.sort_by_key(|p| std::cmp::Reverse(node_version(p)));
            dirs.extend(nodes);
        }
    }
    let mut seen = Vec::new();
    dirs.retain(|d| d.is_dir() && !seen.contains(d) && {
        seen.push(d.clone());
        true
    });
    std::env::join_paths(dirs).unwrap_or_default()
}

/// ".../v24.14.0/bin" -> [24, 14, 0], so the newest Node sorts first.
fn node_version(bin: &Path) -> Vec<u32> {
    let name = bin.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    name.trim_start_matches('v').split('.').map(|d| d.parse().unwrap_or(0)).collect()
}

fn find_program(name: &str, path: &OsString) -> Option<PathBuf> {
    std::env::split_paths(path).map(|dir| dir.join(name)).find(|p| p.is_file())
}

/// Kills the program's whole process group when dropped (a kill, the time
/// limit, or the app closing), so programs a script started go too. Disarmed
/// once the program has ended on its own.
struct GroupKill(Option<u32>);

impl GroupKill {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for GroupKill {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0 {
            // SAFETY: killpg only sends a signal; the group id is the
            // program's own pid (process_group(0)), which is still ours
            // because the program has not been waited for.
            unsafe {
                libc::killpg(pid as libc::pid_t, libc::SIGKILL);
            }
        }
    }
}

/// How a program ended, and what it printed.
struct Finished {
    code: Option<i32>,
    stderr: String,
}

/// Runs a program in `folder`, feeding it `stdin`. Its standard output goes
/// to the job's output as it arrives (when `stream_stdout`), or is returned;
/// the last line is shown as the job's live note. Standard error goes to the
/// output too, when streaming.
async fn run_program(
    ctx: &JobCtx,
    program: &Path,
    args: &[OsString],
    folder: &Path,
    stdin: Option<String>,
    stream_stdout: bool,
    env: &[(&str, OsString)],
) -> Result<(Finished, String), Outcome> {
    let mut cmd = Command::new(program);
    cmd.envs(env.iter().map(|(k, v)| (*k, v)));
    cmd.args(args)
        .current_dir(folder)
        .env("PATH", search_path())
        .env("NO_COLOR", "1")
        // Claude Code refuses to start inside another Claude Code session.
        .env_remove("CLAUDECODE")
        .env_remove("CLAUDE_CODE_ENTRYPOINT")
        .env_remove("CLAUDE_CODE_SSE_PORT")
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    let name = program.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut child = cmd
        .spawn()
        .map_err(|e| Outcome::new().with_label("not_started").with_message(format!("could not start {name}: {e}")))?;
    let mut group = GroupKill(child.id());
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        // Written in the background: a program that prints a lot before
        // reading all its input must not deadlock against us.
        tokio::spawn(async move {
            let _ = pipe.write_all(text.as_bytes()).await;
        });
    }
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    // None until the first line, which is sent at once; later lines at most
    // every NOTE_EVERY.
    let last_note = std::sync::Mutex::new(None::<Instant>);
    let (out, err, status) = tokio::join!(
        pump(stdout, ctx, stream_stdout, &last_note),
        pump(stderr, ctx, stream_stdout, &last_note),
        child.wait()
    );
    group.disarm();
    let status = status.map_err(|e| Outcome::from(format!("{name}: {e}")))?;
    #[cfg(unix)]
    let code = {
        use std::os::unix::process::ExitStatusExt;
        status.code().or_else(|| status.signal().map(|s| -s))
    };
    #[cfg(not(unix))]
    let code = status.code();
    Ok((Finished { code, stderr: err }, out))
}

/// Reads a pipe to the end and returns what it read (up to the output
/// limit). With `stream`, each piece also goes to the job's output as it
/// arrives, and its last line to the live note.
async fn pump(
    pipe: Option<impl AsyncRead + Unpin>,
    ctx: &JobCtx,
    stream: bool,
    last_note: &std::sync::Mutex<Option<Instant>>,
) -> String {
    let Some(mut pipe) = pipe else { return String::new() };
    let mut kept = String::new();
    let mut buf = vec![0u8; 8192];
    loop {
        let n = match pipe.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let text = String::from_utf8_lossy(&buf[..n]);
        if stream {
            ctx.output(&text);
            if let Some(line) = text.lines().rev().find(|l| !l.trim().is_empty()) {
                ctx.note(line.trim());
                let mut last = last_note.lock().unwrap_or_else(|e| e.into_inner());
                if last.is_none_or(|at| at.elapsed() >= NOTE_EVERY) {
                    *last = Some(Instant::now());
                    // Not progress(0, 0): that would wipe a count the
                    // script reported through $JOB_PROGRESS.
                    ctx.send();
                }
            }
        }
        if kept.len() < crate::jobs::MAX_OUTPUT_BYTES {
            kept.push_str(&text);
        }
    }
    kept
}

fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

fn check_text(what: &str, text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err(format!("the {what} is empty"));
    }
    if text.len() > MAX_TEXT_BYTES {
        return Err(format!("the {what} is {} bytes (at most {MAX_TEXT_BYTES})", text.len()));
    }
    Ok(())
}

// ---- Passing values on ----------------------------------------------------------

/// The environment variables a command gets from whatever started it. The
/// whole earlier output is also written to a file in `dir`, for outputs over
/// the variable's limit or with characters a variable cannot hold.
fn given_env(g: &Given, dir: &Path) -> Result<Vec<(&'static str, OsString)>, Outcome> {
    let file = dir.join("output.txt");
    std::fs::write(&file, &g.output).map_err(|e| Outcome::from(format!("could not write the earlier output: {e}")))?;
    // A NUL byte cannot be in an environment variable.
    let tail = g.output[tail_start(&g.output, ENV_OUTPUT_BYTES)..].replace('\0', "");
    let json = |v: &Option<Value>| v.as_ref().map(Value::to_string).unwrap_or_default().replace('\0', "");
    Ok(vec![
        ("JOB_FROM", g.from.clone().into()),
        ("JOB_VALUE", json(&g.value).into()),
        ("JOB_PREVIOUS", json(&g.previous).into()),
        ("JOB_OUTPUT", tail.into()),
        ("JOB_OUTPUT_FILE", file.into()),
    ])
}

/// Fills the placeholders in a prompt or notification: `{value}` (the
/// Trigger's reading, as JSON), `{value.FIELD}` (one field, such as
/// `{value.text}` or `{value.number}`; text without quotes), `{previous}`
/// and `{previous.FIELD}` (the Trigger's reading before), `{output}` (the
/// last `output_max` bytes of what the Action before printed or answered)
/// and `{from}` (what started the job). With nothing given they are empty.
/// Unknown placeholders are left as written, and one pass is made, so text
/// from a web page or an earlier answer that looks like a placeholder is
/// left as it is.
pub(crate) fn fill_text(text: &str, given: Option<&Given>, output_max: usize) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('{') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let name = after.find('}').map(|end| &after[..end]).filter(|n| n.len() <= 80);
        match name.and_then(|n| given_part(n, given, output_max)) {
            Some(filled) => {
                out.push_str(&filled);
                rest = &after[name.unwrap_or_default().len() + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// One placeholder's text, or None when `name` is not a placeholder.
fn given_part(name: &str, given: Option<&Given>, output_max: usize) -> Option<String> {
    let (head, field) = match name.split_once('.') {
        Some((head, field)) => (head, Some(field)),
        None => (name, None),
    };
    let field_ok = |f: &str| f.split('.').all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
    let known = match head {
        "value" | "previous" => field.is_none_or(field_ok),
        "output" | "from" => field.is_none(),
        _ => false,
    };
    if !known {
        return None;
    }
    let Some(g) = given else { return Some(String::new()) };
    let json = |v: &Option<Value>| match (v, field) {
        (None, _) => String::new(),
        (Some(v), None) => v.to_string(),
        (Some(v), Some(f)) => match crate::jobs::value_field(v, f) {
            Some(Value::String(text)) => text.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        },
    };
    Some(match head {
        "value" => json(&g.value),
        "previous" => json(&g.previous),
        "output" => g.output[tail_start(&g.output, output_max)..].trim().to_string(),
        _ => g.from.clone(),
    })
}

fn fill_prompt(prompt: &str, given: Option<&Given>) -> String {
    fill_text(prompt, given, PROMPT_OUTPUT_BYTES)
}

// ---- command: run a shell script ------------------------------------------------

/// Runs a shell script and keeps what it prints. The script can do anything
/// Kim's account can, in any folder.
pub struct CommandJob;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandInput {
    /// One or more lines for bash (cmd on Windows).
    pub script: String,
    /// Where it runs: any folder; empty means the home folder, ~ works.
    #[serde(default)]
    pub folder: Option<String>,
}

impl JobKind for CommandJob {
    const NAME: &'static str = "command";
    const CAPABILITIES: &'static [Capability] = &[Capability::RunPrograms, Capability::WriteFiles, Capability::Network];
    const RESOURCES: &'static [&'static str] = &[];
    const EXAMPLE: &'static str = r#"{"script": "df -h /", "folder": ""}"#;
    type Input = CommandInput;

    fn check(input: &CommandInput) -> Result<(), String> {
        check_text("script", &input.script)
    }

    fn run(ctx: JobCtx, input: CommandInput) -> impl Future<Output = JobResult> + Send + 'static {
        async move {
            let folder = resolve_folder(input.folder.as_deref())?;
            #[cfg(unix)]
            let (shell, args) = (PathBuf::from("/bin/bash"), vec![OsString::from("-c"), OsString::from(&input.script)]);
            #[cfg(windows)]
            let (shell, args) = (PathBuf::from("cmd"), vec![OsString::from("/C"), OsString::from(&input.script)]);
            ctx.progress(0, 0);
            // The job's own temporary folder: the result and progress files
            // the script may write, and the earlier output.
            let scratch = TempFolder::new(ctx.id(), "job")?;
            let result_file = scratch.0.join("result.json");
            let progress_file = scratch.0.join("progress");
            let mut env = vec![("JOB_RESULT", OsString::from(&result_file)), ("JOB_PROGRESS", OsString::from(&progress_file))];
            if let Some(g) = ctx.given() {
                env.extend(given_env(&g, &scratch.0)?);
            }
            let run = run_program(&ctx, &shell, &args, &folder, None, true, &env);
            tokio::pin!(run);
            let mut tick = tokio::time::interval(PROGRESS_EVERY);
            let mut shown = String::new();
            let (finished, _) = loop {
                tokio::select! {
                    done = &mut run => break done?,
                    _ = tick.tick() => show_progress(&ctx, &progress_file, &mut shown),
                }
            };
            show_progress(&ctx, &progress_file, &mut shown);
            let reported = read_result(&result_file)?;
            let mut outcome = match finished.code {
                Some(0) => Outcome::new().with_label("exit_0").with_value(json!({ "exit_code": 0 })).with_message("exit code 0"),
                Some(code) if code < 0 => Outcome::new()
                    .with_label("signal")
                    .with_value(json!({ "signal": -code }))
                    .with_message(format!("stopped by signal {}", -code)),
                Some(code) => {
                    let tail = last_lines(&finished.stderr, 3);
                    let message = if tail.is_empty() { format!("exit code {code}") } else { format!("exit code {code}: {tail}") };
                    Outcome::new().with_label(&format!("exit_{code}")).with_value(json!({ "exit_code": code })).with_message(message)
                }
                None => return Err("the script ended without an exit code".into()),
            };
            // What the script reported replaces the exit code's version,
            // part by part; the exit code still decides done or failed.
            outcome.label = reported.label.or(outcome.label);
            outcome.value = reported.value.or(outcome.value);
            outcome.message = reported.message.or(outcome.message);
            if finished.code == Some(0) { Ok(outcome) } else { Err(outcome) }
        }
    }
}

/// What a script may write to `$JOB_RESULT`, all parts optional.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Reported {
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    value: Option<Value>,
    #[serde(default)]
    message: Option<String>,
}

/// Reads `$JOB_RESULT`. No file or an empty one: nothing reported. A file
/// that is not valid fails the job with the reason, so a typo shows.
fn read_result(path: &Path) -> Result<Reported, Outcome> {
    let bad = |why: String| Outcome::new().with_label("bad_result").with_message(format!("$JOB_RESULT: {why}"));
    let Ok(meta) = std::fs::metadata(path) else { return Ok(Reported::default()) };
    if meta.len() > MAX_RESULT_BYTES {
        return Err(bad(format!("{} bytes (at most {MAX_RESULT_BYTES})", meta.len())));
    }
    let text = std::fs::read_to_string(path).map_err(|e| bad(e.to_string()))?;
    if text.trim().is_empty() {
        return Ok(Reported::default());
    }
    let reported: Reported = serde_json::from_str(&text).map_err(|e| bad(format!("not valid ({e})")))?;
    if let Some(label) = &reported.label {
        let valid = !label.is_empty()
            && label.chars().count() <= 32
            && label.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !valid {
            return Err(bad(format!("label \"{label}\" must be lower-case letters, digits and _, at most 32")));
        }
    }
    Ok(reported)
}

/// Shows the last line of `$JOB_PROGRESS` as the bar: `40/100`, `40%` or
/// a plain number of steps done. Other text is ignored.
fn show_progress(ctx: &JobCtx, path: &Path, shown: &mut String) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    let Some(line) = text.lines().rev().map(str::trim).find(|l| !l.is_empty()) else { return };
    if line == shown {
        return;
    }
    if let Some((done, total)) = parse_progress(line) {
        ctx.progress(done, total);
        *shown = line.to_string();
    }
}

fn parse_progress(line: &str) -> Option<(u64, u64)> {
    if let Some(pct) = line.strip_suffix('%') {
        let pct: u64 = pct.trim().parse().ok()?;
        return Some((pct.min(100), 100));
    }
    match line.split_once('/') {
        Some((done, total)) => {
            let (done, total): (u64, u64) = (done.trim().parse().ok()?, total.trim().parse().ok()?);
            Some((done.min(total), total))
        }
        None => Some((line.parse().ok()?, 0)),
    }
}

// ---- ai: send a prompt to an AI ---------------------------------------------------

#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    ClaudeCode,
    Codex,
    GrokBuild,
    Ollama,
    LlamaCpp,
}

impl Provider {
    fn local(self) -> bool {
        matches!(self, Provider::Ollama | Provider::LlamaCpp)
    }

    fn default_endpoint(self) -> &'static str {
        match self {
            Provider::Ollama => "http://127.0.0.1:11434/api/chat",
            _ => "http://127.0.0.1:8080/v1/chat/completions",
        }
    }
}

/// Sends a prompt to an AI and keeps the answer as the job's output. The
/// command line tools run read-only (they may read files in the folder but
/// not change anything), as in Concilium. A local server that is not running
/// fails with the label `unavailable`; the job never starts one itself.
pub struct AiJob;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AiInput {
    pub provider: Provider,
    /// Model name; empty uses the tool's default (not allowed for Ollama).
    #[serde(default)]
    pub model: String,
    pub prompt: String,
    /// For the command line tools: the folder they may read. Empty: an
    /// empty temporary folder.
    #[serde(default)]
    pub folder: Option<String>,
    /// For Ollama and llama.cpp; empty uses the usual local address.
    #[serde(default)]
    pub endpoint: Option<String>,
}

impl JobKind for AiJob {
    const NAME: &'static str = "ai";
    const CAPABILITIES: &'static [Capability] =
        &[Capability::Network, Capability::RunPrograms, Capability::CostsMoney, Capability::Gpu];
    const RESOURCES: &'static [&'static str] = &[];
    const TIMEOUT: Duration = Duration::from_secs(15 * 60);
    const EXAMPLE: &'static str =
        r#"{"provider": "ollama", "model": "qwen3:1.7b", "prompt": "Say hello in five words."}"#;
    type Input = AiInput;

    fn check(input: &AiInput) -> Result<(), String> {
        check_text("prompt", &input.prompt)?;
        if input.provider == Provider::Ollama && input.model.trim().is_empty() {
            return Err("Ollama needs a model name (for example qwen3:1.7b)".into());
        }
        if input.model.len() > 200 || input.model.chars().any(char::is_whitespace) {
            return Err("the model name must be one word, at most 200 characters".into());
        }
        if let Some(endpoint) = input.endpoint.as_deref().filter(|e| !e.trim().is_empty()) {
            Url::parse(endpoint)?;
        }
        Ok(())
    }

    fn capabilities(input: &AiInput) -> &'static [Capability] {
        if input.provider.local() {
            &[Capability::Gpu]
        } else {
            &[Capability::Network, Capability::RunPrograms, Capability::CostsMoney]
        }
    }

    /// A local model loads into the graphics card's memory, so two at once
    /// could run past it; the queue runs one "gpu" job at a time.
    fn resources(input: &AiInput) -> &'static [&'static str] {
        if input.provider.local() {
            &["gpu"]
        } else {
            &[]
        }
    }

    fn run(ctx: JobCtx, input: AiInput) -> impl Future<Output = JobResult> + Send + 'static {
        async move {
            let prompt = fill_prompt(&input.prompt, ctx.given().as_deref());
            let input = AiInput { prompt, ..input };
            ctx.note(&format!("asking {}", provider_name(input.provider)));
            ctx.progress(0, 0);
            let answer = if input.provider.local() { ask_server(&input).await? } else { ask_tool(&ctx, &input).await? };
            let answer = answer.trim();
            if answer.is_empty() {
                return Err(Outcome::new().with_label("empty").with_message("the AI returned an empty answer"));
            }
            ctx.output(answer);
            let first = answer.lines().find(|l| !l.trim().is_empty()).unwrap_or_default();
            ctx.note(first.trim());
            ctx.progress(0, 0);
            Ok(Outcome::new()
                .with_label("answered")
                .with_value(json!({ "provider": input.provider, "model": input.model, "chars": answer.chars().count() }))
                .with_message(first.chars().take(200).collect::<String>()))
        }
    }
}

fn provider_name(p: Provider) -> &'static str {
    match p {
        Provider::ClaudeCode => "Claude Code",
        Provider::Codex => "Codex",
        Provider::GrokBuild => "GrokBuild",
        Provider::Ollama => "Ollama",
        Provider::LlamaCpp => "llama.cpp",
    }
}

/// An empty temporary folder, removed when dropped.
struct TempFolder(PathBuf);

impl TempFolder {
    fn new(job: u64, what: &str) -> Result<Self, String> {
        // Unique within the process even when two queues (tests) both have
        // a job 1.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("rust_scaffold_{what}_{}_{job}_{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).map_err(|e| format!("could not make a temporary folder: {e}"))?;
        Ok(TempFolder(path))
    }
}

impl Drop for TempFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Claude Code, Codex or GrokBuild, through their command line tools, with
/// the same read-only options as Concilium.
async fn ask_tool(ctx: &JobCtx, input: &AiInput) -> Result<String, Outcome> {
    let program = match input.provider {
        Provider::ClaudeCode => "claude",
        Provider::Codex => "codex",
        _ => "grok",
    };
    let path = search_path();
    let exe = find_program(program, &path).ok_or_else(|| {
        Outcome::new().with_label("unavailable").with_message(format!(
            "{program} is not installed: not on PATH or in the usual user tool folders (~/.local/bin, ~/.npm-global/bin)"
        ))
    })?;
    let given = input.folder.as_deref().filter(|f| !f.trim().is_empty());
    let empty = if given.is_none() { Some(TempFolder::new(ctx.id(), "ai")?) } else { None };
    let folder = match &empty {
        Some(t) => t.0.clone(),
        None => resolve_folder(given)?,
    };
    let scratch = TempFolder::new(ctx.id(), "ai_files")?;
    let model: Vec<OsString> =
        if input.model.trim().is_empty() { vec![] } else { vec!["--model".into(), input.model.trim().into()] };
    let os = |items: &[&str]| items.iter().map(OsString::from).collect::<Vec<_>>();
    let answer_file = scratch.0.join("answer.txt");
    let (args, stdin) = match input.provider {
        Provider::ClaudeCode => {
            let mut a = os(&["-p"]);
            a.extend(model);
            a.extend(os(&[
                "--safe-mode",
                "--tools",
                "Read,Grep,Glob",
                "--permission-mode",
                "dontAsk",
                "--no-session-persistence",
                "--output-format",
                "text",
            ]));
            (a, Some(input.prompt.clone()))
        }
        Provider::Codex => {
            let mut a = os(&["exec"]);
            a.extend(model);
            a.extend(os(&["--sandbox", "read-only", "--skip-git-repo-check", "--ephemeral", "--color", "never"]));
            a.extend([OsString::from("--cd"), folder.clone().into(), "--output-last-message".into(), answer_file.clone().into()]);
            a.push("-".into());
            (a, Some(input.prompt.clone()))
        }
        _ => {
            let prompt_file = scratch.0.join("prompt.txt");
            std::fs::write(&prompt_file, &input.prompt).map_err(|e| Outcome::from(format!("could not write the prompt: {e}")))?;
            let mut a = vec![OsString::from("--prompt-file"), prompt_file.into()];
            a.extend(model);
            a.extend([OsString::from("--cwd"), folder.clone().into()]);
            a.extend(os(&["--tools", "read_file,grep,list_dir", "--no-subagents", "--max-turns", "40", "--output-format", "plain"]));
            (a, None)
        }
    };
    let (finished, stdout) = run_program(ctx, &exe, &args, &folder, stdin, false, &[]).await?;
    if finished.code != Some(0) {
        let detail = last_lines(if finished.stderr.trim().is_empty() { &stdout } else { &finished.stderr }, 8);
        return Err(Outcome::new().with_label("error").with_message(format!(
            "{program} stopped with exit code {}: {}",
            finished.code.unwrap_or(-1),
            if detail.is_empty() { "no error message".into() } else { detail }
        )));
    }
    if input.provider == Provider::Codex {
        if let Ok(answer) = std::fs::read_to_string(&answer_file) {
            return Ok(answer);
        }
    }
    Ok(stdout)
}

/// Ollama or llama.cpp over local HTTP.
async fn ask_server(input: &AiInput) -> Result<String, Outcome> {
    let endpoint = input.endpoint.as_deref().filter(|e| !e.trim().is_empty()).unwrap_or(input.provider.default_endpoint());
    let mut body = json!({
        "model": input.model.trim(),
        "messages": [{ "role": "user", "content": input.prompt }],
    });
    if input.provider == Provider::Ollama {
        body["stream"] = json!(false);
    }
    let reply = http_post_json(endpoint, &body).await.map_err(|e| match e {
        HttpError::Unreachable(why) => Outcome::new().with_label("unavailable").with_message(format!(
            "could not reach {endpoint} ({why}). Is {} running? The job does not start it.",
            provider_name(input.provider)
        )),
        HttpError::Other(why) => Outcome::new().with_label("error").with_message(why),
    })?;
    let answer = if input.provider == Provider::Ollama {
        reply.pointer("/message/content").and_then(Value::as_str)
    } else {
        reply.pointer("/choices/0/message/content").and_then(Value::as_str)
    };
    answer.map(str::to_string).ok_or_else(|| {
        let why = reply.pointer("/error/message").or_else(|| reply.get("error")).map(|e| e.to_string());
        Outcome::new().with_label("error").with_message(why.unwrap_or_else(|| "the answer had no message in it".into()))
    })
}

// ---- notify: a desktop notification -----------------------------------------------

/// Shows a desktop notification. Its title and text can quote the Trigger's
/// reading and the Action before it (`{value.text}`, `{previous.text}`,
/// `{output}` and the rest, as in an AI prompt). On Linux it uses the
/// desktop's notification service over D-Bus (the `zbus` crate, already
/// compiled for the tray); elsewhere, or without one, it fails with
/// `unavailable`.
pub struct NotifyJob;

#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum Urgency {
    Low,
    #[default]
    Normal,
    /// Stays on screen until dismissed, on most desktops.
    Critical,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NotifyInput {
    pub title: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub urgency: Urgency,
    /// Asks the desktop to play its message sound (if it plays sounds).
    #[serde(default)]
    pub sound: bool,
}

/// Longest title and text after the placeholders are filled; notification
/// windows are small.
const NOTIFY_TITLE_CHARS: usize = 200;
const NOTIFY_TEXT_CHARS: usize = 2000;
/// How much of the earlier output `{output}` brings in.
const NOTIFY_OUTPUT_BYTES: usize = 2000;
/// How long to wait for the notification service to answer. A desktop whose
/// own service is not running may have another one that the bus starts and
/// that never answers (on Kim's LXQt, KDE's placeholder waits for a real
/// service for ever), so the job gives up with a reason instead.
#[cfg(not(test))]
const NOTIFY_WAIT: Duration = Duration::from_secs(10);
#[cfg(test)]
const NOTIFY_WAIT: Duration = Duration::from_secs(1);

impl JobKind for NotifyJob {
    const NAME: &'static str = "notify";
    const CAPABILITIES: &'static [Capability] = &[];
    const RESOURCES: &'static [&'static str] = &[];
    const TIMEOUT: Duration = Duration::from_secs(30);
    const EXAMPLE: &'static str = r#"{"title": "Now {value.text}", "text": "Was {previous.text}", "urgency": "normal", "sound": false}"#;
    type Input = NotifyInput;

    fn check(input: &NotifyInput) -> Result<(), String> {
        check_text("title", &input.title)?;
        if input.text.len() > MAX_TEXT_BYTES {
            return Err(format!("the text is {} bytes (at most {MAX_TEXT_BYTES})", input.text.len()));
        }
        Ok(())
    }

    fn run(ctx: JobCtx, input: NotifyInput) -> impl Future<Output = JobResult> + Send + 'static {
        async move {
            let given = ctx.given();
            let title = cut_chars(fill_text(&input.title, given.as_deref(), NOTIFY_OUTPUT_BYTES).trim(), NOTIFY_TITLE_CHARS);
            let text = cut_chars(fill_text(&input.text, given.as_deref(), NOTIFY_OUTPUT_BYTES).trim(), NOTIFY_TEXT_CHARS);
            ctx.output(&format!("{title}\n{text}\n"));
            let note = Notification { title: &title, text: &text, urgency: input.urgency, sound: input.sound };
            match send_notification(&note).await {
                Ok(id) => Ok(Outcome::new().with_label("shown").with_value(json!({ "id": id })).with_message(format!("shown: {title}"))),
                Err(why) => Err(Outcome::new().with_label("unavailable").with_message(format!("no notification shown: {why}"))),
            }
        }
    }
}

fn cut_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}...", &text[..at]),
        None => text.to_string(),
    }
}

struct Notification<'a> {
    title: &'a str,
    text: &'a str,
    urgency: Urgency,
    sound: bool,
}

/// The D-Bus address to send to. Test builds never use the real session
/// bus, so a test cannot put a notification on the desktop: they use the
/// address set by `use_test_bus`, or none.
#[cfg(all(target_os = "linux", feature = "desktop-notifications"))]
fn notify_bus() -> Option<String> {
    #[cfg(test)]
    return TEST_BUS.with(|b| b.borrow().clone()).or_else(|| Some("unix:path=/nonexistent/rust_scaffold_test_bus".into()));
    #[cfg(not(test))]
    None
}

#[cfg(test)]
thread_local! {
    static TEST_BUS: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Test builds: send notifications from jobs on this thread to `address`
/// (a private bus from `test_bus::start`) instead of nowhere.
#[cfg(test)]
pub(crate) fn use_test_bus(address: Option<String>) {
    TEST_BUS.with(|b| *b.borrow_mut() = address);
}

/// A private D-Bus session bus with a stand-in notification service, so
/// tests can check what a notify job sends without touching the desktop.
#[cfg(all(test, target_os = "linux", feature = "desktop-notifications"))]
pub(crate) mod test_bus {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use zbus::zvariant::OwnedValue;

    /// One notification the stand-in received.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Shown {
        pub app: String,
        pub title: String,
        pub text: String,
        pub urgency: Option<u8>,
        pub sound: Option<String>,
    }

    pub struct Bus {
        pub address: String,
        pid: i32,
        dir: PathBuf,
        _service: Option<zbus::Connection>,
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            unsafe {
                libc::kill(self.pid, libc::SIGTERM);
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    struct Service(Arc<Mutex<Vec<Shown>>>, bool);

    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl Service {
        #[allow(clippy::too_many_arguments)]
        async fn notify(
            &self,
            app_name: String,
            _replaces_id: u32,
            _app_icon: String,
            summary: String,
            body: String,
            _actions: Vec<String>,
            hints: HashMap<String, OwnedValue>,
            _expire_timeout: i32,
        ) -> u32 {
            if self.1 {
                // A service that takes far too long, like a placeholder
                // waiting for a real one.
                async_io_sleep().await;
            }
            let urgency = hints.get("urgency").and_then(|v| v.downcast_ref::<u8>().ok());
            let sound = hints.get("sound-name").and_then(|v| v.downcast_ref::<String>().ok());
            let mut list = self.0.lock().unwrap();
            list.push(Shown { app: app_name, title: summary, text: body, urgency, sound });
            list.len() as u32
        }
    }

    async fn async_io_sleep() {
        // The service runs on zbus's own executor, not Tokio's.
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let _ = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let _ = tx.send(());
        });
        blocking_wait(rx).await;
    }

    async fn blocking_wait(rx: std::sync::mpsc::Receiver<()>) {
        loop {
            if rx.try_recv().is_ok() {
                return;
            }
            futures_lite_yield().await;
        }
    }

    /// Yields once to the executor (no extra crate needed).
    async fn futures_lite_yield() {
        let mut yielded = false;
        std::future::poll_fn(|cx| {
            if yielded {
                std::task::Poll::Ready(())
            } else {
                yielded = true;
                cx.waker().wake_by_ref();
                std::task::Poll::Pending
            }
        })
        .await
    }

    /// Starts a private bus (nothing auto-started on it), with the stand-in
    /// service when `with_service` is true; `slow` makes it answer only
    /// after 5 s.
    pub async fn start(name: &str, with_service: bool) -> (Bus, Arc<Mutex<Vec<Shown>>>) {
        start_with(name, with_service, false).await
    }

    pub async fn start_with(name: &str, with_service: bool, slow: bool) -> (Bus, Arc<Mutex<Vec<Shown>>>) {
        let dir = std::env::temp_dir().join(format!("rust_scaffold_bus_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("bus.conf");
        std::fs::write(
            &conf,
            format!(
                // The same as tools/xvfb_check.sh uses (known to work).
                "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\"\n \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\n\
                 <busconfig>\n  <type>session</type>\n  <listen>unix:dir={}</listen>\n  <auth>EXTERNAL</auth>\n  <policy context=\"default\">\n    \
                 <allow send_destination=\"*\" eavesdrop=\"true\"/>\n    <allow eavesdrop=\"true\"/>\n    <allow own=\"*\"/>\n  </policy>\n</busconfig>\n",
                dir.display()
            ),
        )
        .unwrap();
        let out = std::process::Command::new("dbus-daemon")
            .arg(format!("--config-file={}", conf.display()))
            .args(["--fork", "--print-address=1", "--print-pid=1"])
            .output()
            .expect("dbus-daemon is needed for the notify tests");
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        let mut lines = text.lines();
        let address = lines.next().unwrap().trim().to_string();
        let pid: i32 = lines.next().unwrap().trim().parse().unwrap();
        let shown = Arc::new(Mutex::new(Vec::new()));
        let service = if with_service {
            let build = zbus::connection::Builder::address(address.as_str())
                .unwrap()
                .name("org.freedesktop.Notifications")
                .unwrap()
                .serve_at("/org/freedesktop/Notifications", Service(shown.clone(), slow))
                .unwrap()
                .build();
            Some(tokio::time::timeout(std::time::Duration::from_secs(10), build).await.expect("the test bus did not answer").unwrap())
        } else {
            None
        };
        (Bus { address, pid, dir, _service: service }, shown)
    }
}

/// Sends a notification through org.freedesktop.Notifications (the
/// freedesktop.org notification specification) and returns its id.
#[cfg(all(target_os = "linux", feature = "desktop-notifications"))]
async fn send_notification(note: &Notification<'_>) -> Result<u32, String> {
    use std::collections::HashMap;
    use zbus::zvariant::Value as Variant;

    let connection = match notify_bus() {
        Some(address) => zbus::connection::Builder::address(address.as_str()).map_err(|e| e.to_string())?.build().await,
        None => zbus::Connection::session().await,
    }
    .map_err(|e| format!("no session bus ({e})"))?;
    let mut hints: HashMap<&str, Variant> = HashMap::new();
    let urgency: u8 = match note.urgency {
        Urgency::Low => 0,
        Urgency::Normal => 1,
        Urgency::Critical => 2,
    };
    hints.insert("urgency", Variant::from(urgency));
    if note.sound {
        hints.insert("sound-name", Variant::from("message-new-instant"));
    }
    let actions: Vec<&str> = Vec::new();
    // -1: the desktop decides how long it stays.
    let body = ("Rust Scaffold", 0u32, "", note.title, note.text, actions, hints, -1i32);
    let call = connection.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &body,
    );
    let reply = tokio::time::timeout(NOTIFY_WAIT, call)
        .await
        .map_err(|_| {
            format!(
                "the desktop's notification service did not answer within {} s; is its notification daemon running \
                 (on LXQt: lxqt-notificationd, under Configuration Center, Session Settings, Basic Settings)?",
                NOTIFY_WAIT.as_secs()
            )
        })?
        .map_err(|e| match e {
            zbus::Error::MethodError(name, _, _) if name.as_str() == "org.freedesktop.DBus.Error.ServiceUnknown" => {
                "the desktop has no notification service running".to_string()
            }
            other => other.to_string(),
        })?;
    reply.body().deserialize::<u32>().map_err(|e| e.to_string())
}

#[cfg(not(all(target_os = "linux", feature = "desktop-notifications")))]
async fn send_notification(_note: &Notification<'_>) -> Result<u32, String> {
    Err("desktop notifications are only built for Linux (feature desktop-notifications)".into())
}

// ---- The models a local Ollama has ------------------------------------------------

/// One installed Ollama model, for the model list on the Jobs page.
#[derive(Serialize, Debug, PartialEq)]
pub struct ModelInfo {
    pub name: String,
    /// Bytes on disk.
    pub size: u64,
    /// Ollama lists "vision" among its capabilities (an image model). The page
    /// leaves these out of the list for prompts.
    pub vision: bool,
}

/// The models of the Ollama server at `endpoint` (any of its addresses;
/// only the host and port are used), largest last. Asks `/api/tags` and,
/// per model, `/api/show` for its capabilities; neither loads a model.
pub async fn ollama_models(endpoint: Option<&str>) -> Result<Vec<ModelInfo>, String> {
    let given = endpoint.map(str::trim).filter(|e| !e.is_empty()).unwrap_or(Provider::Ollama.default_endpoint());
    let url = Url::parse(given)?;
    let base = url.origin();
    let unreachable = |e: HttpError| match e {
        HttpError::Unreachable(why) => format!("Ollama is not answering at {base} ({why})"),
        HttpError::Other(why) => why,
    };
    let tags = http_json(&format!("{base}/api/tags"), None).await.map_err(unreachable)?;
    let mut models = Vec::new();
    for model in tags["models"].as_array().into_iter().flatten() {
        let Some(name) = model["name"].as_str() else { continue };
        let show = http_json(&format!("{base}/api/show"), Some(&json!({ "model": name }))).await.map_err(unreachable)?;
        let vision = show["capabilities"].as_array().is_some_and(|c| c.iter().any(|v| v == "vision"));
        models.push(ModelInfo { name: name.to_string(), size: model["size"].as_u64().unwrap_or(0), vision });
    }
    models.sort_by_key(|m| m.size);
    Ok(models)
}

// ---- A minimal HTTP/1.1 client ---------------------------------------------------
//
// Plain http for local AI servers; https as well when built with the
// web-watch feature (TLS by rustls, see the `tls` module below).

/// Largest reply read from a server.
const MAX_REPLY_BYTES: usize = 8 * 1024 * 1024;

/// Most redirects `http_get` follows.
#[cfg_attr(not(feature = "web-watch"), allow(dead_code))] // used by the web watch
const MAX_REDIRECTS: usize = 5;

#[derive(Debug)]
pub(crate) enum HttpError {
    /// Nothing listening, or the host is unknown: the server is not running.
    Unreachable(String),
    Other(String),
}

/// `http://host[:port]/path`, or `https://` with the web-watch feature.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Url {
    tls: bool,
    host: String,
    port: u16,
    path: String,
}

impl Url {
    pub(crate) fn parse(text: &str) -> Result<Url, String> {
        let text = text.trim();
        let (tls, rest) = if let Some(rest) = text.strip_prefix("http://") {
            (false, rest)
        } else if let Some(rest) = text.strip_prefix("https://") {
            if !cfg!(feature = "web-watch") {
                return Err(format!("{text}: https needs the web-watch feature"));
            }
            (true, rest)
        } else {
            return Err(format!("{text}: the address must start with http:// or https://"));
        };
        let (authority, path) = rest.split_once('/').map(|(a, p)| (a, format!("/{p}"))).unwrap_or((rest, "/".into()));
        // The part after # never goes to the server.
        let path = path.split('#').next().unwrap_or("/").to_string();
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h, p.parse::<u16>().map_err(|_| format!("{text}: the port is not a number"))?),
            None => (authority, if tls { 443 } else { 80 }),
        };
        if host.is_empty() || host.contains(['@', ' ']) {
            return Err(format!("{text}: no host name"));
        }
        Ok(Url { tls, host: host.to_string(), port, path })
    }

    fn default_port(&self) -> bool {
        self.port == if self.tls { 443 } else { 80 }
    }

    /// `host`, or `host:port` when the port is not the scheme's default.
    fn host_port(&self) -> String {
        if self.default_port() {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// `scheme://host[:port]`, without the path.
    fn origin(&self) -> String {
        format!("{}://{}", if self.tls { "https" } else { "http" }, self.host_port())
    }

    /// Where a redirect's Location header points: a full address, an
    /// address without the scheme (//host/path), a path on this server, or a
    /// path relative to the current one.
    #[cfg_attr(not(feature = "web-watch"), allow(dead_code))] // used by the web watch
    fn join(&self, location: &str) -> Result<Url, String> {
        let location = location.trim();
        if location.starts_with("http://") || location.starts_with("https://") {
            Url::parse(location)
        } else if location.starts_with("//") {
            Url::parse(&format!("{}:{location}", if self.tls { "https" } else { "http" }))
        } else if location.starts_with('/') {
            Ok(Url { path: location.to_string(), ..self.clone() })
        } else {
            let dir = &self.path[..self.path.rfind('/').map(|i| i + 1).unwrap_or(0)];
            Ok(Url { path: format!("{dir}{location}"), ..self.clone() })
        }
    }
}

/// The parts of an HTTP response the client uses.
#[derive(Debug, PartialEq)]
pub(crate) struct Response {
    pub status: u16,
    location: Option<String>,
    pub body: Vec<u8>,
}

fn request_text(method: &str, url: &Url, accept: &str, body: Option<&str>) -> String {
    let mut text = format!(
        "{method} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {}/{}\r\nAccept: {accept}\r\nConnection: close\r\n",
        url.path,
        url.host_port(),
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    );
    match body {
        Some(body) => {
            text.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len()));
            text.push_str(body);
        }
        None => text.push_str("\r\n"),
    }
    text
}

/// Sends one request and returns the parsed response, over TLS for https.
async fn send(url: &Url, request: &str, max_bytes: usize) -> Result<Response, HttpError> {
    let tcp = tokio::net::TcpStream::connect((url.host.as_str(), url.port))
        .await
        .map_err(|e| HttpError::Unreachable(e.to_string()))?;
    #[cfg(feature = "web-watch")]
    if url.tls {
        let stream = tls::connect(&url.host, tcp).await.map_err(HttpError::Other)?;
        return exchange(stream, request, max_bytes).await;
    }
    exchange(tcp, request, max_bytes).await
}

async fn exchange<S>(mut stream: S, request: &str, max_bytes: usize) -> Result<Response, HttpError>
where
    S: AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    stream.write_all(request.as_bytes()).await.map_err(|e| HttpError::Other(e.to_string()))?;
    let mut raw = Vec::new();
    match (&mut stream).take(max_bytes as u64 + 1).read_to_end(&mut raw).await {
        Ok(_) => {}
        // Many servers close a TLS connection without the closing message.
        // A reply cut short is still caught: Content-Length and chunked
        // replies are checked for completeness in parse_response.
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof && !raw.is_empty() => {}
        Err(e) => return Err(HttpError::Other(e.to_string())),
    }
    if raw.len() > max_bytes {
        return Err(HttpError::Other(format!("the reply is over {max_bytes} bytes")));
    }
    parse_response(&raw).map_err(HttpError::Other)
}

async fn http_post_json(url: &str, body: &Value) -> Result<Value, HttpError> {
    http_json(url, Some(body)).await
}

/// GET without a body, POST with one.
async fn http_json(url: &str, body: Option<&Value>) -> Result<Value, HttpError> {
    let url = Url::parse(url).map_err(HttpError::Other)?;
    let method = if body.is_some() { "POST" } else { "GET" };
    let body = body.map(Value::to_string);
    let request = request_text(method, &url, "application/json", body.as_deref());
    let reply = send(&url, &request, MAX_REPLY_BYTES).await?;
    let text = String::from_utf8_lossy(&reply.body);
    if !(200..300).contains(&reply.status) {
        let short: String = text.chars().take(500).collect();
        return Err(HttpError::Other(format!("the server answered HTTP {}: {short}", reply.status)));
    }
    serde_json::from_str(&text).map_err(|_| HttpError::Other("the server did not answer with JSON".into()))
}

/// GETs a page, following up to MAX_REDIRECTS redirects. Returns the final
/// address and the response, whatever its status.
#[cfg_attr(not(feature = "web-watch"), allow(dead_code))] // used by the web watch
pub(crate) async fn http_get(url: &str, max_bytes: usize) -> Result<(Url, Response), HttpError> {
    let mut url = Url::parse(url).map_err(HttpError::Other)?;
    for _ in 0..=MAX_REDIRECTS {
        let request = request_text("GET", &url, "text/html, */*", None);
        let reply = send(&url, &request, max_bytes).await?;
        match (reply.status, &reply.location) {
            (301 | 302 | 303 | 307 | 308, Some(location)) => url = url.join(location).map_err(HttpError::Other)?,
            _ => return Ok((url, reply)),
        }
    }
    Err(HttpError::Other(format!("more than {MAX_REDIRECTS} redirects")))
}

/// TLS for https addresses: rustls with the `ring` crypto library and the
/// root certificates from webpki-roots (Mozilla's list, built in).
#[cfg(feature = "web-watch")]
mod tls {
    use std::sync::{Arc, OnceLock};

    use tokio::net::TcpStream;
    use tokio_rustls::client::TlsStream;
    use tokio_rustls::rustls;
    use tokio_rustls::TlsConnector;

    /// Built once; a failure is kept and reported on every https request
    /// rather than ending the app.
    fn connector() -> Result<TlsConnector, String> {
        static CONNECTOR: OnceLock<Result<TlsConnector, String>> = OnceLock::new();
        CONNECTOR
            .get_or_init(|| {
                let mut roots = rustls::RootCertStore::empty();
                roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                    .with_safe_default_protocol_versions()
                    .map_err(|e| format!("TLS set-up failed: {e}"))?
                    .with_root_certificates(roots)
                    .with_no_client_auth();
                Ok(TlsConnector::from(Arc::new(config)))
            })
            .clone()
    }

    pub async fn connect(host: &str, tcp: TcpStream) -> Result<TlsStream<TcpStream>, String> {
        let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|e| format!("{host}: {e}"))?;
        connector()?.connect(name, tcp).await.map_err(|e| format!("TLS with {host} failed: {e}"))
    }
}

/// Status, Location header and body of a whole HTTP/1.1 response
/// (Content-Length, chunked, or read to the end).
fn parse_response(raw: &[u8]) -> Result<Response, String> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or("the reply has no HTTP header")?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let body = &raw[split + 4..];
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .ok_or("the reply has no HTTP status")?;
    let header = |name: &str| {
        head.lines().skip(1).find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
        })
    };
    let location = header("location");
    let body = if header("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        dechunk(body)?
    } else {
        match header("content-length").and_then(|v| v.parse::<usize>().ok()) {
            Some(n) if n <= body.len() => body[..n].to_vec(),
            Some(_) => return Err("the reply ended early".into()),
            None => body.to_vec(),
        }
    };
    Ok(Response { status, location, body })
}

fn dechunk(mut body: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let line_end = body.windows(2).position(|w| w == b"\r\n").ok_or("a chunk has no size line")?;
        let size_text = String::from_utf8_lossy(&body[..line_end]);
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16).map_err(|_| "a chunk size is not hex")?;
        body = &body[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if body.len() < size + 2 {
            return Err("the reply ended inside a chunk".into());
        }
        out.extend_from_slice(&body[..size]);
        body = &body[size + 2..];
    }
}

// ---- A kind for tests only ---------------------------------------------------------

#[cfg(test)]
pub mod test_kind {
    use super::*;

    /// Counts 1..=count with a pause between counts. It touches nothing, so
    /// the queue and scheduler tests can run it on Tokio's paused clock.
    pub struct CountJob;

    #[derive(Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    pub struct CountInput {
        pub count: u32,
        pub delay_ms: u64,
        #[serde(default)]
        pub fail_at: Option<u32>,
    }

    impl JobKind for CountJob {
        const NAME: &'static str = "count";
        const CAPABILITIES: &'static [Capability] = &[];
        const RESOURCES: &'static [&'static str] = &[];
        type Input = CountInput;

        fn check(input: &CountInput) -> Result<(), String> {
            if !(1..=100).contains(&input.count) {
                return Err("count must be 1 to 100".into());
            }
            if !(100..=10_000).contains(&input.delay_ms) {
                return Err("delay must be 100 to 10000 ms".into());
            }
            if input.fail_at.is_some_and(|step| step == 0 || step > input.count) {
                return Err("fail step must be between 1 and the count".into());
            }
            Ok(())
        }

        fn run(ctx: JobCtx, input: CountInput) -> impl Future<Output = JobResult> + Send + 'static {
            async move {
                for step in 1..=input.count {
                    if step > 1 {
                        tokio::time::sleep(Duration::from_millis(input.delay_ms)).await;
                    }
                    if input.fail_at == Some(step) {
                        return Err(Outcome::new()
                            .with_label("on_purpose")
                            .with_message(format!("failed on purpose at step {step}")));
                    }
                    ctx.detail("step", json!(step));
                    ctx.progress(u64::from(step), u64::from(input.count));
                }
                Ok(Outcome::new().with_label("counted").with_value(json!({ "counted": input.count })))
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::jobs::{Given, JobState, Origin, Queue};
    use std::sync::Arc;
    use tokio::io::AsyncBufReadExt;
    use tokio::runtime::Handle;

    #[test]
    fn every_kind_has_a_unique_plain_name() {
        let mut names: Vec<&str> = TEST_KINDS.iter().map(|entry| entry.name).collect();
        assert!(names
            .iter()
            .all(|name| !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')));
        let all = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all, "two kinds share a name");
    }

    #[test]
    fn examples_are_valid_input() {
        for entry in KINDS {
            let example: Value = serde_json::from_str(entry.example).unwrap();
            let queue_free_check = match entry.name {
                "command" => serde_json::from_value::<CommandInput>(example).map_err(|e| e.to_string()).and_then(|i| CommandJob::check(&i)),
                "ai" => serde_json::from_value::<AiInput>(example).map_err(|e| e.to_string()).and_then(|i| AiJob::check(&i)),
                "notify" => serde_json::from_value::<NotifyInput>(example).map_err(|e| e.to_string()).and_then(|i| NotifyJob::check(&i)),
                other => panic!("no example test for {other}"),
            };
            assert!(queue_free_check.is_ok(), "{}: {queue_free_check:?}", entry.name);
        }
    }

    fn queue() -> Queue {
        Queue::new(Handle::current(), 3, Arc::new(|_| {}), TEST_KINDS)
    }

    async fn finish(q: &Queue, id: u64) -> (JobState, Option<String>) {
        for _ in 0..1000 {
            match q.result(id) {
                Some((state, label)) if state.finished() => return (state, label),
                _ => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
        panic!("job {id} did not finish");
    }

    fn output(q: &Queue, id: u64) -> String {
        serde_json::to_value(q.inspect(id).unwrap()).unwrap()["output"].as_str().unwrap().to_string()
    }

    fn command(q: &Queue, script: &str, folder: Option<&str>) -> u64 {
        q.start("command", json!({ "script": script, "folder": folder }), Origin::User, None).unwrap()
    }

    #[tokio::test]
    async fn a_command_prints_and_its_exit_code_is_the_label() {
        let q = queue();
        let tmp = std::env::temp_dir();
        let ok = command(&q, "echo hello; echo oops >&2; pwd", Some(tmp.to_str().unwrap()));
        assert_eq!(finish(&q, ok).await, (JobState::Done, Some("exit_0".into())));
        let out = output(&q, ok);
        assert!(out.contains("hello") && out.contains("oops"), "{out}");
        assert!(out.contains(&std::fs::canonicalize(&tmp).unwrap().display().to_string()), "runs in the folder given: {out}");
        let bad = command(&q, "echo nearly; echo broke >&2; exit 3", None);
        assert_eq!(finish(&q, bad).await, (JobState::Failed, Some("exit_3".into())));
        let view = q.inspect(bad).unwrap();
        let text = serde_json::to_value(view).unwrap();
        assert!(text["job"]["message"].as_str().unwrap().contains("exit code 3: broke"));
        assert!(output(&q, bad).contains("nearly"));
        let home = command(&q, "pwd", Some("~"));
        finish(&q, home).await;
        assert_eq!(output(&q, home).trim(), dirs::home_dir().unwrap().display().to_string());
        let missing = command(&q, "true", Some("/no/such/folder"));
        let (state, _) = finish(&q, missing).await;
        assert_eq!(state, JobState::Failed);
        assert!(q.start("command", json!({ "script": "  " }), Origin::User, None).is_err(), "empty script refused");
    }

    #[tokio::test]
    async fn killing_a_command_stops_everything_it_started() {
        let q = queue();
        let pid_file = std::env::temp_dir().join(format!("rust_scaffold_kill_{}", std::process::id()));
        let script = format!("sleep 60 & echo $! > {}; echo started; wait", pid_file.display());
        let id = command(&q, &script, None);
        for _ in 0..200 {
            if output(&q, id).contains("started") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let pid: i32 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
        assert!(Path::new(&format!("/proc/{pid}")).exists(), "the grandchild runs");
        q.cancel(id).unwrap();
        let mut gone = false;
        for _ in 0..200 {
            let state = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            // Gone, or a zombie waiting for its parent (which was killed too).
            if state.is_empty() || state.split_whitespace().nth(2) == Some("Z") {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let _ = std::fs::remove_file(&pid_file);
        assert!(gone, "the sleep started by the script was killed");
        assert_eq!(q.result(id).unwrap().0, JobState::Cancelled);
    }

    #[tokio::test]
    async fn a_command_past_its_time_limit_is_stopped() {
        let q = queue();
        let id = q.start("command", json!({ "script": "sleep 30" }), Origin::User, Some(1_000)).unwrap();
        let started = Instant::now();
        assert_eq!(finish(&q, id).await.0, JobState::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn output_keeps_the_newest_part() {
        let q = queue();
        // About 600 KB of numbered lines; the last line must survive.
        let id = command(&q, "seq 1 100000; echo the_end", None);
        finish(&q, id).await;
        let info = serde_json::to_value(q.inspect(id).unwrap()).unwrap();
        let out = info["output"].as_str().unwrap();
        assert!(out.len() <= crate::jobs::MAX_OUTPUT_BYTES);
        assert!(out.trim_end().ends_with("the_end"));
        assert_eq!(info["output_cut"], json!(true));
    }

    fn outcome(q: &Queue, id: u64) -> Outcome {
        q.outcome(id).unwrap().1
    }

    #[tokio::test]
    async fn a_command_can_report_its_own_result() {
        let q = queue();
        // Label, value and message from $JOB_RESULT; the exit code decides
        // done or failed.
        let id = command(
            &q,
            r#"printf '{"label":"disk_high","value":{"percent":92},"message":"92%% full"}' > "$JOB_RESULT""#,
            None,
        );
        assert_eq!(finish(&q, id).await, (JobState::Done, Some("disk_high".into())));
        let o = outcome(&q, id);
        assert_eq!((o.value, o.message.as_deref()), (Some(json!({ "percent": 92 })), Some("92% full")));
        let failed = command(&q, r#"echo '{"label":"no_network"}' > "$JOB_RESULT"; exit 4"#, None);
        assert_eq!(finish(&q, failed).await, (JobState::Failed, Some("no_network".into())));
        assert_eq!(outcome(&q, failed).value, Some(json!({ "exit_code": 4 })), "parts not reported keep the exit code's");
        // Only a value: the label stays exit_0.
        let value_only = command(&q, r#"echo '{"value":{"n":3}}' > "$JOB_RESULT""#, None);
        assert_eq!(finish(&q, value_only).await, (JobState::Done, Some("exit_0".into())));
        assert_eq!(outcome(&q, value_only).value, Some(json!({ "n": 3 })));
        // Printing result-like text changes nothing: only the file counts.
        let printed = command(&q, r#"echo '{"label":"faked"}'"#, None);
        assert_eq!(finish(&q, printed).await, (JobState::Done, Some("exit_0".into())));
        // A broken file fails the job with the reason.
        for bad in [r#"echo '{"label": oops' > "$JOB_RESULT""#, r#"echo '{"label":"Bad Label"}' > "$JOB_RESULT""#, r#"echo '{"lable":"x"}' > "$JOB_RESULT""#] {
            let id = command(&q, bad, None);
            assert_eq!(finish(&q, id).await, (JobState::Failed, Some("bad_result".into())), "{bad}");
            assert!(outcome(&q, id).message.unwrap().starts_with("$JOB_RESULT:"));
        }
    }

    #[tokio::test]
    async fn a_command_can_report_progress() {
        let q = queue();
        let id = command(&q, r#"echo 3/10 > "$JOB_PROGRESS"; echo started; sleep 2; echo 10/10 >> "$JOB_PROGRESS""#, None);
        let view = |q: &Queue| serde_json::to_value(q.inspect(id).unwrap()).unwrap()["job"].clone();
        let mut seen = false;
        for _ in 0..150 {
            let v = view(&q);
            if (v["done"].as_u64(), v["total"].as_u64()) == (Some(3), Some(10)) {
                seen = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(seen, "3 of 10 shown while running");
        finish(&q, id).await;
        let v = view(&q);
        assert_eq!((v["done"].as_u64(), v["total"].as_u64()), (Some(10), Some(10)), "read once more at the end");
        assert_eq!(parse_progress("40%"), Some((40, 100)));
        assert_eq!(parse_progress("7 / 5"), Some((5, 5)));
        assert_eq!(parse_progress("12"), Some((12, 0)));
        assert_eq!(parse_progress("half"), None);
    }

    #[test]
    fn placeholders_bring_in_the_reading_and_the_output() {
        let g = Given {
            from: "trigger gbp".into(),
            value: Some(json!({ "text": "£5,212.66", "number": 5212.66, "disk": { "free_gb": 4.5 } })),
            previous: Some(json!({ "text": "£5,100.00", "number": 5100.0 })),
            output: "Summary {value.text}".into(),
        };
        let filled = fill_text(
            "{value.text} ({value.number}) was {previous.text}; {value.disk.free_gb} GB [{value.nothing}] \
             | {output} | {from} | {other} {value. {\"json\": 1} {value",
            Some(&g),
            100,
        );
        assert_eq!(
            filled,
            "£5,212.66 (5212.66) was £5,100.00; 4.5 GB [] | Summary {value.text} | trigger gbp | {other} {value. {\"json\": 1} {value",
            "one pass: placeholders inside the output stay as text; unknown ones and JSON are left"
        );
        assert_eq!(fill_text("{previous}", Some(&g), 100), r#"{"number":5100.0,"text":"£5,100.00"}"#);
        assert_eq!(fill_text("[{value.number}] [{output}]", None, 100), "[] []");
    }

    #[tokio::test]
    async fn notify_sends_the_filled_text_to_the_notification_service() {
        let (bus, shown) = test_bus::start("notify", true).await;
        use_test_bus(Some(bus.address.clone()));
        let q = queue();
        let id = q
            .start_given(
                "notify",
                json!({ "title": "Disk {value.percent}% full", "text": "{output}", "urgency": "critical", "sound": true }),
                Origin::User,
                None,
                Some(Arc::new(Given {
                    from: "trigger disk".into(),
                    value: Some(json!({ "percent": 92 })),
                    previous: None,
                    output: "Clear the cache.".into(),
                })),
            )
            .unwrap();
        assert_eq!(finish(&q, id).await, (JobState::Done, Some("shown".into())));
        let got = shown.lock().unwrap().clone();
        assert_eq!(
            got,
            [test_bus::Shown {
                app: "Rust Scaffold".into(),
                title: "Disk 92% full".into(),
                text: "Clear the cache.".into(),
                urgency: Some(2),
                sound: Some("message-new-instant".into()),
            }]
        );
        // Plain: normal urgency, no sound hint.
        let plain = q.start("notify", json!({ "title": "Hello" }), Origin::User, None).unwrap();
        finish(&q, plain).await;
        let last = shown.lock().unwrap().last().cloned().unwrap();
        assert_eq!((last.urgency, last.sound, last.text.as_str()), (Some(1), None, ""));
        use_test_bus(None);
    }

    #[tokio::test]
    async fn notify_gives_up_on_a_service_that_does_not_answer() {
        let (bus, _) = test_bus::start_with("slow", true, true).await;
        use_test_bus(Some(bus.address.clone()));
        let q = queue();
        let started = Instant::now();
        let id = q.start("notify", json!({ "title": "Hello" }), Origin::User, None).unwrap();
        assert_eq!(finish(&q, id).await, (JobState::Failed, Some("unavailable".into())));
        assert!(started.elapsed() < Duration::from_secs(4), "gave up after NOTIFY_WAIT, not the job's 30 s");
        assert!(outcome(&q, id).message.unwrap().contains("did not answer within 1 s"), "{:?}", outcome(&q, id));
        use_test_bus(None);
    }

    #[tokio::test]
    async fn notify_without_a_notification_service_is_unavailable() {
        // A bus with nothing answering for notifications.
        let (bus, _) = test_bus::start("no_service", false).await;
        use_test_bus(Some(bus.address.clone()));
        let q = queue();
        let id = q.start("notify", json!({ "title": "Hello" }), Origin::User, None).unwrap();
        assert_eq!(finish(&q, id).await, (JobState::Failed, Some("unavailable".into())));
        assert!(outcome(&q, id).message.unwrap().contains("no notification service"), "{:?}", outcome(&q, id));
        // No test bus: test builds never reach the real desktop.
        use_test_bus(None);
        let id = q.start("notify", json!({ "title": "Hello" }), Origin::User, None).unwrap();
        assert_eq!(finish(&q, id).await, (JobState::Failed, Some("unavailable".into())));
        assert!(q.start("notify", json!({ "title": " " }), Origin::User, None).is_err(), "empty title refused");
    }

    #[test]
    fn prompts_are_filled_and_a_long_output_is_cut_to_its_end() {
        assert_eq!(fill_prompt("A {output} B {from}", None), "A  B ", "nothing given: empty");
        let long = format!("{}THE END", "x".repeat(PROMPT_OUTPUT_BYTES * 2));
        let g = Given { from: "action a".into(), output: long, ..Given::default() };
        let filled = fill_prompt("[{from}] {output}", Some(&g));
        assert!(filled.starts_with("[action a] x") && filled.ends_with("THE END"));
        assert_eq!(filled.len(), "[action a] ".len() + PROMPT_OUTPUT_BYTES);
        let env = given_env(&Given { output: "a\0b".into(), ..g }, &std::env::temp_dir()).unwrap();
        let output = env.iter().find(|(name, _)| *name == "JOB_OUTPUT").unwrap();
        assert_eq!(output.1, OsString::from("ab"), "a NUL byte cannot go into a variable");
    }

    #[test]
    fn urls_and_responses_are_parsed() {
        assert_eq!(
            Url::parse("http://127.0.0.1:11434/api/chat").unwrap(),
            Url { tls: false, host: "127.0.0.1".into(), port: 11434, path: "/api/chat".into() }
        );
        assert_eq!(Url::parse("http://localhost").unwrap().port, 80);
        assert_eq!(Url::parse("http://localhost/a#part").unwrap().path, "/a", "the #part is not sent");
        assert!(Url::parse("ftp://x").is_err());
        assert!(Url::parse("http://:80/").is_err());
        assert_eq!(Url::parse("http://localhost:8080/x").unwrap().origin(), "http://localhost:8080");
        let plain = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}extra";
        assert_eq!(parse_response(plain).unwrap(), Response { status: 200, location: None, body: b"{}".to_vec() });
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n{\"a\r\n4\r\n\":1}\r\n0\r\n\r\n";
        assert_eq!(parse_response(chunked).unwrap().body, b"{\"a\":1}".to_vec());
        assert!(parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n{}").is_err());
        let moved = b"HTTP/1.1 301 Moved\r\nlocation: /probe/\r\nContent-Length: 0\r\n\r\n";
        assert_eq!(parse_response(moved).unwrap().location.as_deref(), Some("/probe/"));
    }

    #[cfg(feature = "web-watch")]
    #[test]
    fn https_addresses_and_redirect_targets() {
        let url = Url::parse("https://bitmods.com/probe/index.php?x=1").unwrap();
        assert_eq!((url.tls, url.port, url.origin()), (true, 443, "https://bitmods.com".to_string()));
        assert_eq!(Url::parse("https://h:8443/").unwrap().origin(), "https://h:8443");
        assert_eq!(url.join("/a").unwrap().path, "/a");
        assert_eq!(url.join("b.php").unwrap().path, "/probe/b.php");
        assert_eq!(url.join("//other.org/c").unwrap(), Url::parse("https://other.org/c").unwrap());
        let plain = url.join("http://h/d").unwrap();
        assert!(!plain.tls && plain.port == 80, "a redirect may change the scheme");
    }

    /// Answers each connection with the next reply and sends back the first
    /// line of each request (the method and path).
    pub(crate) async fn fake_site(replies: Vec<String>) -> (u16, tokio::sync::mpsc::UnboundedReceiver<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            for reply in replies {
                let (socket, _) = listener.accept().await.unwrap();
                let mut reader = tokio::io::BufReader::new(socket);
                let mut first = String::new();
                reader.read_line(&mut first).await.unwrap();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                let _ = tx.send(first.trim().to_string());
                reader.get_mut().write_all(reply.as_bytes()).await.unwrap();
            }
        });
        (port, rx)
    }

    fn redirect(to: &str) -> String {
        format!("HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\n\r\n")
    }

    #[tokio::test]
    async fn get_follows_redirects_and_returns_any_status() {
        let page = "<p><span id=\"probe-token\">abc</span></p>";
        let ok = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{page}", page.len());
        let (port, mut asked) = fake_site(vec![redirect("/probe/"), redirect("index.php"), ok]).await;
        let (url, reply) = http_get(&format!("http://127.0.0.1:{port}/probe"), 1024).await.unwrap();
        assert_eq!((reply.status, String::from_utf8(reply.body).unwrap()), (200, page.to_string()));
        assert_eq!(url.path, "/probe/index.php");
        let mut lines = Vec::new();
        while let Ok(line) = asked.try_recv() {
            lines.push(line);
        }
        assert_eq!(lines, ["GET /probe HTTP/1.1", "GET /probe/ HTTP/1.1", "GET /probe/index.php HTTP/1.1"]);

        let (port, _) = fake_site(vec!["HTTP/1.1 404 Not Found\r\nContent-Length: 4\r\n\r\ngone".into()]).await;
        let (_, reply) = http_get(&format!("http://127.0.0.1:{port}/"), 1024).await.unwrap();
        assert_eq!(reply.status, 404, "the caller decides what a status means");
    }

    #[tokio::test]
    async fn get_stops_after_too_many_redirects_and_large_pages() {
        let (port, _) = fake_site((0..=MAX_REDIRECTS).map(|_| redirect("/again")).collect()).await;
        match http_get(&format!("http://127.0.0.1:{port}/"), 1024).await {
            Err(HttpError::Other(why)) => assert!(why.contains("redirects"), "{why}"),
            other => panic!("expected a redirect error, got {other:?}"),
        }
        let big = "x".repeat(2000);
        let (port, _) = fake_site(vec![format!("HTTP/1.1 200 OK\r\nContent-Length: 2000\r\n\r\n{big}")]).await;
        match http_get(&format!("http://127.0.0.1:{port}/"), 1024).await {
            Err(HttpError::Other(why)) => assert!(why.contains("over 1024 bytes"), "{why}"),
            other => panic!("expected a size error, got {other:?}"),
        }
        let port = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port();
        assert!(matches!(http_get(&format!("http://127.0.0.1:{port}/"), 1024).await, Err(HttpError::Unreachable(_))));
    }

    /// Kim's probe page with the three values added on 16 November 2026.
    #[cfg(feature = "web-watch")]
    pub(crate) const PROBE_PAGE: &str = "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>probe</title></head><body>\n<p><span id=\"probe-token\">2f3a8ec2157a39f90c474db4d032a4e1</span></p>\n<p><span id=\"probe-gbp\">£3,577.30</span></p>\n<p><span id=\"probe-usd\">$3,649.36</span></p>\n<p><small>2026-11-16 12:25:53</small></p>\n</body></html>";

    #[cfg(feature = "web-watch")]
    pub(crate) fn page_reply(page: &str) -> String {
        format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\r\n{page}", page.len())
    }

    #[cfg(feature = "web-watch")]
    fn watch(url: &str, find: &str, name: &str, read_as: &str) -> crate::web_watch::WebWatch {
        serde_json::from_value(json!({ "url": url, "find": find, "name": name, "read_as": read_as })).unwrap()
    }

    #[cfg(feature = "web-watch")]
    #[tokio::test]
    async fn web_watch_reads_text_and_numbers() {
        let (port, _) = fake_site(vec![page_reply(PROBE_PAGE); 3]).await;
        let url = format!("http://127.0.0.1:{port}/probe");
        let token = watch(&url, "id", "probe-token", "text").read().await.unwrap();
        assert_eq!(token, json!({ "text": "2f3a8ec2157a39f90c474db4d032a4e1" }));
        let gbp = watch(&url, "id", "probe-gbp", "number").read().await.unwrap();
        assert_eq!(gbp, json!({ "text": "£3,577.30", "number": 3577.3 }));
        let usd = watch(&url, "id", "probe-usd", "number").read().await.unwrap();
        assert_eq!(usd["number"], json!(3649.36));
    }

    #[cfg(feature = "web-watch")]
    #[tokio::test]
    async fn web_watch_failures_are_explained() {
        let (port, _) = fake_site(vec![
            page_reply(PROBE_PAGE),
            page_reply(PROBE_PAGE),
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".into(),
        ])
        .await;
        let url = format!("http://127.0.0.1:{port}/probe");
        let missing = watch(&url, "class", "price", "text").read().await.unwrap_err();
        assert!(missing.contains(".price is not on"), "{missing}");
        let words = watch(&url, "id", "probe-token", "number").read().await;
        assert!(words.is_ok(), "the token starts with a digit, so it has a number");
        let gone = watch(&url, "id", "x", "text").read().await.unwrap_err();
        assert!(gone.contains("HTTP 404"), "{gone}");

        let (port, _) = fake_site(vec![page_reply("<p id=\"n\">sold out</p>")]).await;
        let none = watch(&format!("http://127.0.0.1:{port}/"), "id", "n", "number").read().await.unwrap_err();
        assert!(none.contains("no number"), "{none}");

        let port = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port();
        let down = watch(&format!("http://127.0.0.1:{port}/"), "id", "n", "text").read().await.unwrap_err();
        assert!(down.contains("could not reach"), "{down}");

        for bad in [
            json!({ "url": "ftp://x/", "find": "id", "name": "n" }),
            json!({ "url": "https://x/", "find": "id", "name": "" }),
            json!({ "url": "https://x/", "find": "class", "name": "two words" }),
        ] {
            let w: crate::web_watch::WebWatch = serde_json::from_value(bad.clone()).unwrap();
            assert!(w.check().is_err(), "{bad}");
        }
        for bad in [
            json!({ "url": "https://x/", "find": "tag", "name": "p" }),
            json!({ "url": "https://x/", "find": "id", "name": "n", "read_as": "html" }),
            json!({ "url": "https://x/", "find": "id", "name": "n", "extra": 1 }),
        ] {
            assert!(serde_json::from_value::<crate::web_watch::WebWatch>(bad.clone()).is_err(), "{bad}");
        }
    }

    /// Asks Kim's real probe page over HTTPS. Run with `-- --ignored`.
    #[cfg(feature = "web-watch")]
    #[tokio::test]
    #[ignore]
    async fn live_https_probe_page() {
        let (url, reply) = http_get("https://bitmods.com/probe", 2 * 1024 * 1024).await.unwrap();
        let page = String::from_utf8_lossy(&reply.body);
        assert_eq!(reply.status, 200, "{url:?}");
        assert!(page.contains("id=\"probe-token\""), "{page}");
        let token = crate::web_watch::find_text(&page, crate::web_watch::FindBy::Id, "probe-token").unwrap();
        assert!(token.as_deref().is_some_and(|t| t.len() >= 32), "{token:?}");
        println!("probe-token: {token:?}");
    }

    /// A one-shot local server: answers the first request with `reply` and
    /// sends back the request body it received.
    async fn fake_server(reply: String) -> (u16, tokio::sync::oneshot::Receiver<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut reader = tokio::io::BufReader::new(socket);
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).await.unwrap();
            let _ = tx.send(String::from_utf8(body).unwrap());
            reader.get_mut().write_all(reply.as_bytes()).await.unwrap();
        });
        (port, rx)
    }

    fn http_reply(body: &str) -> String {
        format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len())
    }

    #[tokio::test]
    async fn ollama_and_llama_cpp_answers_become_the_output() {
        let q = queue();
        let (port, sent) = fake_server(http_reply(r#"{"message":{"role":"assistant","content":"Hello there"}}"#)).await;
        let input = json!({ "provider": "ollama", "model": "qwen3:1.7b", "prompt": "Hi",
            "endpoint": format!("http://127.0.0.1:{port}/api/chat") });
        let id = q.start("ai", input, Origin::User, None).unwrap();
        assert_eq!(finish(&q, id).await, (JobState::Done, Some("answered".into())));
        assert_eq!(output(&q, id), "Hello there");
        let request: Value = serde_json::from_str(&sent.await.unwrap()).unwrap();
        assert_eq!(request["model"], "qwen3:1.7b");
        assert_eq!(request["stream"], false);
        assert_eq!(request["messages"][0]["content"], "Hi");
        let view = serde_json::to_value(q.inspect(id).unwrap()).unwrap();
        assert_eq!(view["job"]["resources"], json!(["gpu"]), "a local model holds the gpu lock");

        let (port, _) = fake_server(http_reply(r#"{"choices":[{"message":{"content":"From llama"}}]}"#)).await;
        let input = json!({ "provider": "llama_cpp", "prompt": "Hi", "endpoint": format!("http://127.0.0.1:{port}/v1/chat/completions") });
        let id = q.start("ai", input, Origin::User, None).unwrap();
        assert_eq!(finish(&q, id).await.0, JobState::Done);
        assert_eq!(output(&q, id), "From llama");
    }

    #[tokio::test]
    async fn the_model_list_marks_vision_models() {
        let tags = r#"{"models":[{"name":"big:latest","size":900},{"name":"qwen2vl:latest","size":500},{"name":"qwen3:1.7b","size":100}]}"#;
        let shows = [
            r#"{"capabilities":["completion"]}"#,
            r#"{"capabilities":["completion","vision"]}"#,
            r#"{"capabilities":["completion","tools","thinking"]}"#,
        ];
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            for reply in std::iter::once(tags).chain(shows) {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = vec![0; 4096];
                let _ = socket.read(&mut buf).await.unwrap();
                socket.write_all(http_reply(reply).as_bytes()).await.unwrap();
            }
        });
        let models = ollama_models(Some(&format!("http://127.0.0.1:{port}/api/chat"))).await.unwrap();
        let summary: Vec<(&str, bool)> = models.iter().map(|m| (m.name.as_str(), m.vision)).collect();
        assert_eq!(summary, [("qwen3:1.7b", false), ("qwen2vl:latest", true), ("big:latest", false)], "smallest first");
        let port = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port();
        let err = ollama_models(Some(&format!("http://127.0.0.1:{port}"))).await.unwrap_err();
        assert!(err.contains("not answering"), "{err}");
    }

    #[tokio::test]
    async fn a_server_that_is_not_running_is_unavailable() {
        let q = queue();
        // Bind and drop, so the port is very likely free.
        let port = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port();
        let input = json!({ "provider": "ollama", "model": "m", "prompt": "Hi", "endpoint": format!("http://127.0.0.1:{port}/api/chat") });
        let id = q.start("ai", input, Origin::User, None).unwrap();
        assert_eq!(finish(&q, id).await, (JobState::Failed, Some("unavailable".into())));
        assert!(q.start("ai", json!({ "provider": "ollama", "prompt": "Hi" }), Origin::User, None).unwrap_err().contains("model"));
    }

    /// Stand-ins for claude, codex and grok: shell scripts that print their
    /// arguments and input, so the options and the prompt can be checked
    /// without calling a real AI.
    #[tokio::test]
    async fn the_command_line_tools_get_read_only_options_and_the_prompt() {
        use std::os::unix::fs::PermissionsExt;
        let bin = std::env::temp_dir().join(format!("rust_scaffold_fake_ai_{}", std::process::id()));
        std::fs::create_dir_all(&bin).unwrap();
        let tools = [
            ("claude", "echo \"claude args: $*\"; cat"),
            // Codex writes its answer to the file after --output-last-message.
            ("codex", "out=''; prev=''; for a in \"$@\"; do [ \"$prev\" = --output-last-message ] && out=$a; prev=$a; done; { echo \"codex args: $*\"; cat; } > \"$out\"; echo progress noise"),
            ("grok", "echo \"grok args: $*\"; f=''; prev=''; for a in \"$@\"; do [ \"$prev\" = --prompt-file ] && f=$a; prev=$a; done; cat \"$f\""),
        ];
        for (name, body) in tools {
            let path = bin.join(name);
            std::fs::write(&path, format!("#!/bin/bash\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        *TEST_BIN_DIR.lock().unwrap() = Some(bin.clone());
        let q = queue();
        let ask = |provider: &str, model: &str| {
            q.start("ai", json!({ "provider": provider, "model": model, "prompt": "What is 2+2?" }), Origin::User, None).unwrap()
        };
        let claude = ask("claude_code", "sonnet");
        let codex = ask("codex", "");
        let grok = ask("grok_build", "grok-4");
        for id in [claude, codex, grok] {
            assert_eq!(finish(&q, id).await, (JobState::Done, Some("answered".into())), "{}", output(&q, id));
            assert!(output(&q, id).contains("What is 2+2?"), "{}", output(&q, id));
        }
        let c = output(&q, claude);
        assert!(c.contains("-p --model sonnet --safe-mode --tools Read,Grep,Glob --permission-mode dontAsk"), "{c}");
        let x = output(&q, codex);
        assert!(x.contains("exec --sandbox read-only") && !x.contains("--model") && !x.contains("progress noise"), "{x}");
        let g = output(&q, grok);
        assert!(g.contains("--model grok-4") && g.contains("--tools read_file,grep,list_dir"), "{g}");
        let view = serde_json::to_value(q.inspect(claude).unwrap()).unwrap();
        assert_eq!(view["job"]["resources"], json!([]), "a cloud tool does not hold the gpu lock");
        // Not by removing the stand-in: the real claude may be installed.
        assert!(find_program("rust_scaffold_no_such_tool", &search_path()).is_none());
        *TEST_BIN_DIR.lock().unwrap() = None;
        let _ = std::fs::remove_dir_all(&bin);
    }

    /// Calls the real Ollama on this machine. Skipped in normal runs (it
    /// needs a running server and loads a model into the graphics card):
    /// `cargo test --release live_ollama -- --ignored`, model from
    /// LIVE_OLLAMA_MODEL (default qwen3:1.7b).
    #[tokio::test]
    #[ignore]
    async fn live_ollama() {
        let model = std::env::var("LIVE_OLLAMA_MODEL").unwrap_or_else(|_| "qwen3:1.7b".into());
        let q = queue();
        let input = json!({ "provider": "ollama", "model": model, "prompt": "Reply with exactly one word: the colour of grass." });
        let id = q.start("ai", input, Origin::User, Some(120_000)).unwrap();
        for _ in 0..1200 {
            if q.result(id).unwrap().0.finished() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let info = serde_json::to_value(q.inspect(id).unwrap()).unwrap();
        println!("live answer: {}", info["output"]);
        println!("message: {}", info["job"]["message"]);
        assert_eq!(info["job"]["state"], "done");
        assert_eq!(info["job"]["label"], "answered");
    }
}
