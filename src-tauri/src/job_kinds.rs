//! Every kind of job the app can run, and the list the queue uses.
//!
//! To add a kind: write a type with `impl JobKind`, then add one line,
//! `kind::<NewJob>()`, to `KINDS`. The queue (`src/jobs.rs`) does not change.
//! The window starts a job with the kind's name and its input as JSON; only
//! kinds in this list can run.
//!
//! - `command`: runs a shell script (bash on Linux, cmd on Windows) in any
//!   folder, capturing what it prints. The exit code is the result label
//!   (`exit_0`, `exit_1` ...), so rules can react to it.
//!
//! Pipelines: when a rule starts a job, the job is given the result of the
//! job that fired the rule (`JobCtx::previous`). A command gets it as
//! environment variables (`PREVIOUS_OUTPUT`, `PREVIOUS_OUTPUT_FILE`,
//! `PREVIOUS_LABEL`, `PREVIOUS_STATE`, `PREVIOUS_SCHEDULE`, `PREVIOUS_JOB`),
//! never pasted into the script, so text an AI wrote can never run as
//! code. An AI prompt gets it where it says `{previous_output}` (and
//! `{previous_label}`, `{previous_state}`, `{previous_schedule}`).
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

use crate::jobs::{kind, tail_start, Capability, JobCtx, JobKind, JobResult, JobState, KindEntry, Outcome, Previous};

/// Every kind that may be started, by name.
pub static KINDS: &[KindEntry] = &[kind::<CommandJob>(), kind::<AiJob>()];

/// The app's kinds plus `count`, a kind kept only for the queue and
/// scheduler tests (it needs no programs and runs on the paused clock).
#[cfg(test)]
pub static TEST_KINDS: &[KindEntry] = &[kind::<CommandJob>(), kind::<AiJob>(), kind::<test_kind::CountJob>()];

/// Longest script or prompt.
const MAX_TEXT_BYTES: usize = 60 * 1024;
/// The end of the previous output a command gets in `PREVIOUS_OUTPUT`
/// (Linux refuses a single environment variable over 128 KiB; the whole
/// output is in the file `PREVIOUS_OUTPUT_FILE`).
const ENV_PREVIOUS_BYTES: usize = 100 * 1024;
/// The end of the previous output put into an AI prompt. Small local models
/// read about 4,000 tokens (Ollama's default context) and drop the start of
/// a longer prompt, instructions included; 12,000 bytes is about 3,000
/// tokens, leaving room for the instructions.
pub const PROMPT_PREVIOUS_BYTES: usize = 12_000;
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
    let last_note = std::sync::Mutex::new(Instant::now());
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
    last_note: &std::sync::Mutex<Instant>,
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
                if last.elapsed() >= NOTE_EVERY {
                    *last = Instant::now();
                    ctx.progress(0, 0);
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

// ---- Passing a result on (pipelines) ---------------------------------------------

fn state_name(state: JobState) -> String {
    serde_json::to_value(state).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// The environment variables a command started by a rule gets. The whole
/// output is also written to a file in `dir`, for outputs over the
/// variable's limit or with characters a variable cannot hold.
fn previous_env(p: &Previous, dir: &Path) -> Result<Vec<(&'static str, OsString)>, Outcome> {
    let file = dir.join("previous_output.txt");
    std::fs::write(&file, &p.output).map_err(|e| Outcome::from(format!("could not write the previous output: {e}")))?;
    // A NUL byte cannot be in an environment variable.
    let tail = p.output[tail_start(&p.output, ENV_PREVIOUS_BYTES)..].replace('\0', "");
    Ok(vec![
        ("PREVIOUS_OUTPUT", tail.into()),
        ("PREVIOUS_OUTPUT_FILE", file.into()),
        ("PREVIOUS_LABEL", p.label.clone().unwrap_or_default().into()),
        ("PREVIOUS_STATE", state_name(p.state).into()),
        ("PREVIOUS_SCHEDULE", p.schedule.clone().into()),
        ("PREVIOUS_JOB", p.job.to_string().into()),
    ])
}

/// Fills `{previous_output}`, `{previous_label}`, `{previous_state}` and
/// `{previous_schedule}` in a prompt. With no previous job (started by a
/// person or a timetable) they become empty. The output is cut to its last
/// `PROMPT_PREVIOUS_BYTES`.
fn fill_prompt(prompt: &str, previous: Option<&Previous>) -> String {
    let (output, label, state, schedule) = match previous {
        Some(p) => {
            let output = &p.output[tail_start(&p.output, PROMPT_PREVIOUS_BYTES)..];
            (output.trim().to_string(), p.label.clone().unwrap_or_default(), state_name(p.state), p.schedule.clone())
        }
        None => Default::default(),
    };
    prompt
        .replace("{previous_label}", &label)
        .replace("{previous_state}", &state)
        .replace("{previous_schedule}", &schedule)
        // Last, so text inside the output that looks like a placeholder is
        // left as it is.
        .replace("{previous_output}", &output)
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
            let previous = ctx.previous();
            let scratch = match &previous {
                Some(_) => Some(TempFolder::new(ctx.id(), "previous")?),
                None => None,
            };
            let env = match (&previous, &scratch) {
                (Some(p), Some(dir)) => previous_env(p, &dir.0)?,
                _ => Vec::new(),
            };
            let (finished, _) = run_program(&ctx, &shell, &args, &folder, None, true, &env).await?;
            match finished.code {
                Some(0) => Ok(Outcome::new().with_label("exit_0").with_value(json!({ "exit_code": 0 })).with_message("exit code 0")),
                Some(code) if code < 0 => Err(Outcome::new()
                    .with_label("signal")
                    .with_value(json!({ "signal": -code }))
                    .with_message(format!("stopped by signal {}", -code))),
                Some(code) => {
                    let tail = last_lines(&finished.stderr, 3);
                    let message = if tail.is_empty() { format!("exit code {code}") } else { format!("exit code {code}: {tail}") };
                    Err(Outcome::new().with_label(&format!("exit_{code}")).with_value(json!({ "exit_code": code })).with_message(message))
                }
                None => Err("the script ended without an exit code".into()),
            }
        }
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
            let prompt = fill_prompt(&input.prompt, ctx.previous().as_deref());
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
        let path = std::env::temp_dir().join(format!("rust_scaffold_{what}_{}_{job}", std::process::id()));
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
    let base = format!("http://{}:{}", url.host, url.port);
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

// ---- A minimal HTTP/1.1 client for local servers ----------------------------------

/// Largest reply read from a server.
const MAX_REPLY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug)]
enum HttpError {
    /// Nothing listening, or the host is unknown: the server is not running.
    Unreachable(String),
    Other(String),
}

/// `http://host[:port]/path`. Plain HTTP only.
#[derive(Debug, PartialEq)]
struct Url {
    host: String,
    port: u16,
    path: String,
}

impl Url {
    fn parse(text: &str) -> Result<Url, String> {
        let text = text.trim();
        let rest = text.strip_prefix("http://").ok_or_else(|| {
            if text.starts_with("https://") {
                format!("{text}: https is not supported (local servers use http)")
            } else {
                format!("{text}: the address must start with http://")
            }
        })?;
        let (authority, path) = rest.split_once('/').map(|(a, p)| (a, format!("/{p}"))).unwrap_or((rest, "/".into()));
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h, p.parse::<u16>().map_err(|_| format!("{text}: the port is not a number"))?),
            None => (authority, 80),
        };
        if host.is_empty() || host.contains(['@', ' ']) {
            return Err(format!("{text}: no host name"));
        }
        Ok(Url { host: host.to_string(), port, path })
    }
}

async fn http_post_json(url: &str, body: &Value) -> Result<Value, HttpError> {
    http_json(url, Some(body)).await
}

/// GET without a body, POST with one.
async fn http_json(url: &str, body: Option<&Value>) -> Result<Value, HttpError> {
    let url = Url::parse(url).map_err(HttpError::Other)?;
    let mut stream = tokio::net::TcpStream::connect((url.host.as_str(), url.port))
        .await
        .map_err(|e| HttpError::Unreachable(e.to_string()))?;
    let method = if body.is_some() { "POST" } else { "GET" };
    let body = body.map(Value::to_string).unwrap_or_default();
    let request = format!(
        "{method} {} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nAccept: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        url.path,
        url.host,
        url.port,
        body.len(),
        body
    );
    stream.write_all(request.as_bytes()).await.map_err(|e| HttpError::Other(e.to_string()))?;
    let mut raw = Vec::new();
    (&mut stream).take(MAX_REPLY_BYTES as u64 + 1).read_to_end(&mut raw).await.map_err(|e| HttpError::Other(e.to_string()))?;
    if raw.len() > MAX_REPLY_BYTES {
        return Err(HttpError::Other(format!("the reply is over {MAX_REPLY_BYTES} bytes")));
    }
    let (status, body) = parse_response(&raw).map_err(HttpError::Other)?;
    let text = String::from_utf8_lossy(&body);
    if !(200..300).contains(&status) {
        let short: String = text.chars().take(500).collect();
        return Err(HttpError::Other(format!("the server answered HTTP {status}: {short}")));
    }
    serde_json::from_str(&text).map_err(|_| HttpError::Other("the server did not answer with JSON".into()))
}

/// Status code and body of a whole HTTP/1.1 response (Content-Length,
/// chunked, or read to the end).
fn parse_response(raw: &[u8]) -> Result<(u16, Vec<u8>), String> {
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
    if header("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        return Ok((status, dechunk(body)?));
    }
    match header("content-length").and_then(|v| v.parse::<usize>().ok()) {
        Some(n) if n <= body.len() => Ok((status, body[..n].to_vec())),
        Some(_) => Err("the reply ended early".into()),
        None => Ok((status, body.to_vec())),
    }
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
mod tests {
    use super::*;
    use crate::jobs::{JobState, Origin, Queue};
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

    #[test]
    fn prompts_are_filled_and_a_long_output_is_cut_to_its_end() {
        assert_eq!(fill_prompt("A {previous_output} B {previous_label}", None), "A  B ", "nothing before: empty");
        let long = format!("{}THE END", "x".repeat(PROMPT_PREVIOUS_BYTES * 2));
        let p = Previous { schedule: "s".into(), job: 7, state: JobState::TimedOut, label: None, output: long };
        let filled = fill_prompt("[{previous_state}] {previous_output}", Some(&p));
        assert!(filled.starts_with("[timed_out] x") && filled.ends_with("THE END"));
        assert_eq!(filled.len(), "[timed_out] ".len() + PROMPT_PREVIOUS_BYTES);
        let env = previous_env(&Previous { output: "a\0b".into(), ..p }, &std::env::temp_dir()).unwrap();
        assert_eq!(env[0].1, OsString::from("ab"), "a NUL byte cannot go into a variable");
    }

    #[test]
    fn urls_and_responses_are_parsed() {
        assert_eq!(
            Url::parse("http://127.0.0.1:11434/api/chat").unwrap(),
            Url { host: "127.0.0.1".into(), port: 11434, path: "/api/chat".into() }
        );
        assert_eq!(Url::parse("http://localhost").unwrap().port, 80);
        assert!(Url::parse("https://openrouter.ai/api").unwrap_err().contains("https is not supported"));
        assert!(Url::parse("ftp://x").is_err());
        assert!(Url::parse("http://:80/").is_err());
        let plain = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}extra";
        assert_eq!(parse_response(plain).unwrap(), (200, b"{}".to_vec()));
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n{\"a\r\n4\r\n\":1}\r\n0\r\n\r\n";
        assert_eq!(parse_response(chunked).unwrap(), (200, b"{\"a\":1}".to_vec()));
        assert!(parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n{}").is_err());
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
