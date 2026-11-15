//! Job queue: runs work in Rust on Tauri's Tokio runtime, at most
//! `job_concurrency` jobs at once (config.json, 1 to 3). Extra jobs wait in
//! order. Every change (queued, started, each step, finished) is pushed to the
//! window as a `jobs:changed` event, so the Jobs tab never polls.
//!
//! What a job does is defined by a job kind (`JobKind`), in
//! `src/job_kinds.rs`. This file is the queue and does not know about any
//! kind: it takes a kind name and its input as JSON, looks the kind up in the
//! list it was given, checks the input, and runs it. The window can only ask
//! for kinds in that list, never run arbitrary code.
//!
//! Every kind must declare what it touches outside the app (`Capability`)
//! and which shared resources it needs; a kind that leaves either out does
//! not compile. A running job sees only a `JobCtx`, never the queue, so one
//! job cannot change another. Each run records who started it (`Origin`)
//! and ends with an `Outcome` (a short label, a small JSON value and a
//! message) that later steps (rules) can act on. Every run has a time limit.
//!
//! Why a hand-written queue rather than a Tokio `Semaphore`: a semaphore
//! limits how many tasks run, but the waiting tasks already exist and cannot
//! be listed or reordered, and its limit cannot shrink without extra tricks.
//! Here queued jobs are plain data (no task yet), so they can be shown and
//! cancelled for free, and the limit can change while jobs run.
//!
//! Cancelling a running job uses Tokio's `AbortHandle`; the time limit uses
//! `tokio::time::timeout`. Tokio can only stop a task at an `.await` point;
//! code between two awaits always runs to the next one. A job that never
//! awaits cannot be stopped this way. Note: the release build uses
//! `panic = "abort"`, so a panic inside a job ends the whole app. Return an
//! error instead.

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use tokio::runtime::Handle;
use tokio::task::AbortHandle;
use tokio::time::Instant;
use tracing::{info, warn};

/// Limits for `job_concurrency` in config.json.
pub const MIN_CONCURRENCY: u32 = 1;
pub const MAX_CONCURRENCY: u32 = 3;
pub const DEFAULT_CONCURRENCY: u32 = 2;

/// Time limit of one run, when the kind sets none.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60 * 60);
/// Range allowed for a time limit given when a job is started.
pub const MIN_TIMEOUT_MS: u64 = 1_000;
pub const MAX_TIMEOUT_MS: u64 = 24 * 60 * 60 * 1_000;

/// Finished jobs kept for inspection; the oldest are dropped first.
const MAX_FINISHED: usize = 50;
/// Jobs waiting at once. Stops a runaway caller filling memory.
const MAX_QUEUED: usize = 200;
/// Size limits, so no job or caller can fill memory or the window.
const MAX_INPUT_BYTES: usize = 64 * 1024;
const MAX_VALUE_BYTES: usize = 4 * 1024;
const MAX_LABEL_CHARS: usize = 32;
const MAX_TEXT_CHARS: usize = 500;
/// Output kept per job (the newest part, when there is more).
pub const MAX_OUTPUT_BYTES: usize = 256 * 1024;

/// Window event carrying one job's new state.
pub const CHANGED_EVENT: &str = "jobs:changed";

/// Something a job kind touches outside the app. Every kind lists its own;
/// the Jobs page shows them, and later steps let Kim decide which may run
/// without asking.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)] // not every capability is used by the kinds so far
pub enum Capability {
    /// Talks to another computer.
    Network,
    /// Uses the graphics card (local models).
    Gpu,
    /// Writes outside the app's own settings folder.
    WriteFiles,
    /// Starts another program.
    RunPrograms,
    /// Uses a paid service.
    CostsMoney,
}

/// Who started a run. Later steps add rules and outside callers.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum Origin {
    /// A person, from the Jobs page.
    User,
    /// A schedule in jobs.json, by name.
    Schedule { name: String },
    /// A rule in jobs.json that started this schedule's job.
    Rule { rule: String, schedule: String },
}

/// The result of the job that led to this one (a rule fired on it): what it
/// printed or answered, and how it ended. A command job gets it as
/// environment variables, an AI or notify job in its text
/// (`{previous_output}`, `{previous_value}` and so on).
#[derive(Debug, Clone, PartialEq)]
pub struct Previous {
    pub schedule: String,
    pub job: u64,
    pub state: JobState,
    pub label: Option<String>,
    pub value: Option<Value>,
    pub message: Option<String>,
    pub output: String,
}

/// A field of a job's value: `percent`, or `disk.percent` for a field
/// inside an object. None when the value has no such field.
pub fn value_field<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(value, |v, name| v.get(name))
}

/// What the window sees of `Previous`: everything but the output, which can
/// be large (Inspect of the earlier job shows it).
#[derive(Serialize, Clone, Debug)]
pub struct PreviousView {
    pub schedule: String,
    pub job: u64,
    pub state: JobState,
    pub label: Option<String>,
    pub value: Option<Value>,
    pub message: Option<String>,
    pub output_bytes: usize,
}

/// How a run ended, as returned by a kind: `Ok` for success, `Err` for
/// failure. Both can carry a label (a short word such as `changed` or
/// `unavailable`, which rules will match on), a small JSON value and a
/// message for people. `Err("text".into())` is the simplest failure.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub label: Option<String>,
    pub value: Option<Value>,
    pub message: Option<String>,
}

impl Outcome {
    pub fn new() -> Self {
        Self::default()
    }

    /// Lower-case letters, digits and `_`, at most 32 characters.
    pub fn with_label(mut self, label: &str) -> Self {
        self.label = Some(label.to_string());
        self
    }

    /// At most 4 KiB as JSON.
    pub fn with_value(mut self, value: Value) -> Self {
        self.value = Some(value);
        self
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// A kind that breaks the limits is a bug in that kind; the job then
    /// fails with the reason, so the bug shows on the Errors page.
    fn check(&self) -> Result<(), String> {
        if let Some(label) = &self.label {
            let valid = !label.is_empty()
                && label.chars().count() <= MAX_LABEL_CHARS
                && label.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if !valid {
                return Err(format!(
                    "the job returned an invalid label \"{}\" (lower-case letters, digits and _, at most {MAX_LABEL_CHARS})",
                    shorten(label, MAX_LABEL_CHARS)
                ));
            }
        }
        if let Some(value) = &self.value {
            let size = serde_json::to_vec(value).map(|v| v.len()).unwrap_or(usize::MAX);
            if size > MAX_VALUE_BYTES {
                return Err(format!("the job returned a value of {size} bytes (at most {MAX_VALUE_BYTES})"));
            }
        }
        Ok(())
    }
}

impl From<String> for Outcome {
    fn from(message: String) -> Self {
        Outcome::new().with_message(message)
    }
}

impl From<&str> for Outcome {
    fn from(message: &str) -> Self {
        Outcome::new().with_message(message)
    }
}

/// What a kind's `run` returns.
pub type JobResult = Result<Outcome, Outcome>;

/// A kind of job. One type per kind, listed in `src/job_kinds.rs`.
/// `CAPABILITIES` and `RESOURCES` have no default on purpose: a kind must
/// say what it touches, even when the answer is "nothing" (`&[]`).
pub trait JobKind: 'static {
    /// The name the window and the log use, for example "count".
    const NAME: &'static str;
    /// What the kind touches outside the app.
    const CAPABILITIES: &'static [Capability];
    /// Shared resources it needs, for example `"gpu"`. Later steps let only
    /// one running job hold each resource.
    const RESOURCES: &'static [&'static str];
    /// Time limit of one run, unless the caller gives another.
    const TIMEOUT: Duration = DEFAULT_TIMEOUT;
    /// A valid input as JSON, offered when a schedule is added on the Jobs
    /// page.
    const EXAMPLE: &'static str = "{}";
    /// What the caller sends to start a job of this kind, as JSON.
    type Input: DeserializeOwned + Serialize + Send + 'static;

    /// Refuses bad input before anything is queued.
    fn check(input: &Self::Input) -> Result<(), String>;

    /// What this run touches, when it depends on the input (an AI job that
    /// calls a local model uses the graphics card; one that calls a cloud
    /// tool does not). Defaults to the kind's own lists.
    fn capabilities(_input: &Self::Input) -> &'static [Capability] {
        Self::CAPABILITIES
    }

    fn resources(_input: &Self::Input) -> &'static [&'static str] {
        Self::RESOURCES
    }

    /// The work. Every `.await` is a point where a kill or the time limit
    /// can stop it.
    fn run(ctx: JobCtx, input: Self::Input) -> impl Future<Output = JobResult> + Send + 'static;
}

type RunFuture = Pin<Box<dyn Future<Output = JobResult> + Send>>;
type Launch = Box<dyn FnOnce(JobCtx) -> RunFuture + Send>;

/// Input parsed and checked, and the work ready to start.
struct Prepared {
    /// The input as the kind understood it, for Inspect.
    input: Value,
    capabilities: &'static [Capability],
    resources: &'static [&'static str],
    launch: Launch,
}

/// One entry in the list of kinds. Made with `kind::<SomeKind>()`. The list
/// holds plain functions, because Rust cannot keep different `JobKind` types
/// (each with its own `Input`) in one list.
#[derive(Clone, Copy)]
pub struct KindEntry {
    pub name: &'static str,
    pub capabilities: &'static [Capability],
    pub resources: &'static [&'static str],
    pub timeout: Duration,
    pub example: &'static str,
    prepare: fn(Value) -> Result<Prepared, String>,
}

pub const fn kind<K: JobKind>() -> KindEntry {
    KindEntry {
        name: K::NAME,
        capabilities: K::CAPABILITIES,
        resources: K::RESOURCES,
        timeout: K::TIMEOUT,
        example: K::EXAMPLE,
        prepare: prepare::<K>,
    }
}

fn prepare<K: JobKind>(raw: Value) -> Result<Prepared, String> {
    let input: K::Input = serde_json::from_value(raw).map_err(|e| format!("bad input for {}: {e}", K::NAME))?;
    K::check(&input)?;
    let shown = serde_json::to_value(&input).map_err(|e| e.to_string())?;
    let (capabilities, resources) = (K::capabilities(&input), K::resources(&input));
    Ok(Prepared { input: shown, capabilities, resources, launch: Box::new(move |ctx| Box::pin(K::run(ctx, input))) })
}

#[derive(Serialize, serde::Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
    TimedOut,
}

impl JobState {
    pub fn finished(self) -> bool {
        !matches!(self, JobState::Queued | JobState::Running)
    }
}

/// What the window sees of a job. Times are milliseconds.
#[derive(Serialize, Clone, Debug)]
pub struct JobView {
    pub id: u64,
    /// Rises with every change to any job. The window keeps whichever copy
    /// of a job has the higher number, so a late list reply cannot undo a
    /// newer event.
    pub seq: u64,
    pub kind: &'static str,
    pub state: JobState,
    pub origin: Origin,
    pub capabilities: &'static [Capability],
    pub resources: &'static [&'static str],
    /// The input as the kind understood it.
    pub input: Value,
    /// Progress: `done` of `total` (`total` 0 means not known).
    pub done: u64,
    pub total: u64,
    /// Short live text from the job.
    pub note: Option<String>,
    /// Live figures particular to the kind, shown by Inspect.
    pub detail: BTreeMap<String, Value>,
    pub timeout_ms: u64,
    /// Wall-clock times, Unix milliseconds.
    pub created_ms: u64,
    pub started_ms: Option<u64>,
    pub finished_ms: Option<u64>,
    /// Time spent queued, and time spent running.
    pub wait_ms: Option<u64>,
    pub run_ms: Option<u64>,
    /// From the job's `Outcome`.
    pub label: Option<String>,
    pub value: Option<Value>,
    /// The outcome's message, or why the job failed, was killed or timed out.
    pub message: Option<String>,
    /// The job whose result this one was given, if a rule started it.
    pub previous: Option<PreviousView>,
}

/// Extra, live details for the Inspect button.
#[derive(Serialize)]
pub struct JobInspect {
    job: JobView,
    /// Tokio's id for the running task, while one exists.
    task_id: Option<String>,
    /// True once Tokio has fully dropped the task (after it ends or is aborted).
    task_finished: Option<bool>,
    /// Position in the queue, 1 = next to start.
    queue_position: Option<usize>,
    output: String,
    /// True when the start of the output was dropped to keep the newest.
    output_cut: bool,
}

/// Tokio runtime figures, for the Jobs tab summary.
#[derive(Serialize)]
pub struct RuntimeView {
    /// Worker threads in Tauri's Tokio runtime.
    workers: usize,
    /// Tasks alive in the runtime, including Tauri's own.
    alive_tasks: usize,
    /// Tasks waiting in the shared run queue for a free worker.
    global_queue_depth: usize,
}

#[derive(Serialize)]
pub struct JobsReply {
    /// The newest `seq` at the moment of the snapshot.
    seq: u64,
    jobs: Vec<JobView>,
    limit: usize,
    pub(crate) running: usize,
    pub(crate) queued: usize,
    runtime: RuntimeView,
}

struct Job {
    view: JobView,
    created: Instant,
    started: Option<Instant>,
    abort: Option<AbortHandle>,
    /// Taken when the job starts.
    launch: Option<Launch>,
    timeout: Duration,
    /// Text the job printed or answered (the newest MAX_OUTPUT_BYTES), and
    /// whether older text was dropped. Not in `JobView`, so window events
    /// stay small; Inspect fetches it.
    output: String,
    output_cut: bool,
    previous: Option<Arc<Previous>>,
}

struct Inner {
    jobs: BTreeMap<u64, Job>,
    pending: VecDeque<u64>,
    running: usize,
    limit: usize,
    next_id: u64,
    /// Only one running job may hold each resource (config.json
    /// `job_resource_locks`, on by default).
    locks: bool,
}

/// Called with every changed job. It runs while the queue is locked, which
/// keeps events in order (step 5 can never arrive after step 6), so it must
/// not call back into the queue.
pub type Notify = Arc<dyn Fn(&JobView) + Send + Sync>;

struct Shared {
    inner: Mutex<Inner>,
    runtime: Handle,
    notify: Notify,
    seq: AtomicU64,
    kinds: &'static [KindEntry],
}

/// Cheap to clone; every clone is the same queue.
#[derive(Clone)]
pub struct Queue(Arc<Shared>);

/// How a run ended, as seen by the queue.
enum End {
    Returned(JobResult),
    TimedOut,
}

/// What a running job can see and do: its own progress and nothing else.
pub struct JobCtx {
    queue: Queue,
    id: u64,
}

impl JobCtx {
    #[allow(dead_code)] // for kinds that log or name files by job
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Reports progress (`total` 0 means not known) and sends the change,
    /// with any note and detail set since the last one, to the window.
    pub fn progress(&self, done: u64, total: u64) {
        self.queue.update(self.id, true, |view| {
            view.done = done;
            view.total = total;
        });
    }

    /// Short live text ("connecting", "3 files left"). Sent with the next
    /// `progress`.
    pub fn note(&self, text: &str) {
        let text = shorten(text, MAX_TEXT_CHARS);
        self.queue.update(self.id, false, |view| view.note = Some(text));
    }

    /// A live figure particular to the kind, shown by Inspect. Sent with
    /// the next `progress`.
    #[allow(dead_code)] // for kinds with figures to show; tests use it
    pub fn detail(&self, name: &str, value: Value) {
        self.queue.update(self.id, false, |view| {
            view.detail.insert(name.to_string(), value);
        });
    }

    /// Adds text to the job's output (what a command printed, a model's
    /// answer). Only the newest `MAX_OUTPUT_BYTES` are kept. Not sent to
    /// the window; Inspect reads it.
    pub fn output(&self, text: &str) {
        let mut inner = self.queue.lock();
        let Some(job) = inner.jobs.get_mut(&self.id) else { return };
        if job.view.state != JobState::Running {
            return;
        }
        job.output.push_str(text);
        if job.output.len() > MAX_OUTPUT_BYTES {
            let start = tail_start(&job.output, MAX_OUTPUT_BYTES);
            job.output.drain(..start);
            job.output_cut = true;
        }
    }

    /// The result of the job that led to this one, when a rule started it
    /// (None for a job started by a person or a timetable).
    pub fn previous(&self) -> Option<Arc<Previous>> {
        self.queue.lock().jobs.get(&self.id).and_then(|job| job.previous.clone())
    }

    /// True once the job has been killed or has run out of time. Long loops
    /// that do not await should check it and return.
    #[allow(dead_code)] // for kinds with long loops that do not await
    pub fn killed(&self) -> bool {
        self.queue.lock().jobs.get(&self.id).is_none_or(|job| job.view.state != JobState::Running)
    }
}

/// Where the last `max` bytes of `text` begin, moved forward to a
/// character boundary.
pub(crate) fn tail_start(text: &str, max: usize) -> usize {
    let mut start = text.len().saturating_sub(max);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    start
}

fn unix_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

fn shorten(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => format!("{}...", &text[..cut]),
        None => text.to_string(),
    }
}

pub fn clamp_limit(limit: u32) -> u32 {
    limit.clamp(MIN_CONCURRENCY, MAX_CONCURRENCY)
}

impl Queue {
    /// `runtime` is where jobs run: Tauri's Tokio runtime in the app, the
    /// test's runtime in tests. `kinds` is every kind that may be started.
    pub fn new(runtime: Handle, limit: u32, notify: Notify, kinds: &'static [KindEntry]) -> Self {
        Queue(Arc::new(Shared {
            inner: Mutex::new(Inner {
                jobs: BTreeMap::new(),
                pending: VecDeque::new(),
                running: 0,
                limit: clamp_limit(limit) as usize,
                next_id: 1,
                locks: true,
            }),
            runtime,
            notify,
            seq: AtomicU64::new(0),
            kinds,
        }))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // A poisoned lock means a panic elsewhere; the data is still usable.
        self.0.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Always called with the queue locked, so `seq` follows the true order.
    fn notify(&self, job: &mut Job) {
        job.view.seq = self.0.seq.fetch_add(1, Ordering::Relaxed) + 1;
        (self.0.notify)(&job.view);
    }

    /// The kind, input size and time limit of a request, before parsing.
    fn request(&self, kind: &str, input: &Value, timeout_ms: Option<u64>) -> Result<(&'static KindEntry, Duration), String> {
        let entry = self
            .0
            .kinds
            .iter()
            .find(|entry| entry.name == kind)
            .ok_or_else(|| format!("unknown job kind \"{}\"", shorten(kind, MAX_LABEL_CHARS)))?;
        let size = serde_json::to_vec(input).map(|v| v.len()).unwrap_or(usize::MAX);
        if size > MAX_INPUT_BYTES {
            return Err(format!("the input is {size} bytes (at most {MAX_INPUT_BYTES})"));
        }
        let timeout = match timeout_ms {
            Some(limit) if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&limit) => {
                return Err(format!("the time limit must be {MIN_TIMEOUT_MS} to {MAX_TIMEOUT_MS} ms"));
            }
            Some(limit) => Duration::from_millis(limit),
            None => entry.timeout,
        };
        Ok((entry, timeout))
    }

    /// Every kind that may be started.
    pub fn kinds(&self) -> &'static [KindEntry] {
        self.0.kinds
    }

    /// Checks a request exactly as `start` would, without queueing it. Used
    /// to refuse a bad schedule when jobs.json is loaded, not when it is due.
    pub fn check(&self, kind: &str, input: &Value, timeout_ms: Option<u64>) -> Result<(), String> {
        let (entry, _) = self.request(kind, input, timeout_ms)?;
        (entry.prepare)(input.clone()).map(|_| ())
    }

    /// The state and result label of a job, while it is still listed.
    pub fn result(&self, id: u64) -> Option<(JobState, Option<String>)> {
        self.lock().jobs.get(&id).map(|job| (job.view.state, job.view.label.clone()))
    }

    /// A job's state and its whole outcome (label, value, message).
    pub fn outcome(&self, id: u64) -> Option<(JobState, Outcome)> {
        self.lock().jobs.get(&id).map(|job| {
            let v = &job.view;
            (v.state, Outcome { label: v.label.clone(), value: v.value.clone(), message: v.message.clone() })
        })
    }

    /// Queues a job of the named kind and starts it at once if a slot is
    /// free. `timeout_ms` replaces the kind's time limit. Bad kinds, input
    /// and limits are refused before anything is queued.
    pub fn start(&self, kind: &str, input: Value, origin: Origin, timeout_ms: Option<u64>) -> Result<u64, String> {
        self.start_after(kind, input, origin, timeout_ms, None)
    }

    /// `start`, giving the job the result of the job before it.
    pub fn start_after(
        &self,
        kind: &str,
        input: Value,
        origin: Origin,
        timeout_ms: Option<u64>,
        previous: Option<Arc<Previous>>,
    ) -> Result<u64, String> {
        let (entry, timeout) = self.request(kind, &input, timeout_ms)?;
        let prepared = (entry.prepare)(input)?;

        let mut inner = self.lock();
        if inner.pending.len() >= MAX_QUEUED {
            return Err(format!("the queue is full ({MAX_QUEUED} waiting)"));
        }
        let id = inner.next_id;
        inner.next_id += 1;
        let mut job = Job {
            view: JobView {
                id,
                seq: 0,
                kind: entry.name,
                state: JobState::Queued,
                origin,
                capabilities: prepared.capabilities,
                resources: prepared.resources,
                input: prepared.input,
                done: 0,
                total: 0,
                note: None,
                detail: BTreeMap::new(),
                timeout_ms: ms(timeout),
                created_ms: unix_ms(),
                started_ms: None,
                finished_ms: None,
                wait_ms: None,
                run_ms: None,
                label: None,
                value: None,
                message: None,
                previous: previous.as_ref().map(|p| PreviousView {
                    schedule: p.schedule.clone(),
                    job: p.job,
                    state: p.state,
                    label: p.label.clone(),
                    value: p.value.clone(),
                    message: p.message.clone(),
                    output_bytes: p.output.len(),
                }),
            },
            created: Instant::now(),
            started: None,
            abort: None,
            launch: Some(prepared.launch),
            timeout,
            output: String::new(),
            output_cut: false,
            previous,
        };
        self.notify(&mut job);
        info!("job {id} ({}): queued by {:?}", entry.name, job.view.origin);
        inner.jobs.insert(id, job);
        inner.pending.push_back(id);
        self.dispatch(&mut inner);
        Ok(id)
    }

    /// Starts queued jobs in order while fewer than `limit` are running. With
    /// resource locks on, a job whose resource (for example "gpu") is held by
    /// a running job waits, and jobs behind it that need other resources, or
    /// none, may start first.
    fn dispatch(&self, inner: &mut Inner) {
        let mut index = 0;
        while inner.running < inner.limit && index < inner.pending.len() {
            let id = inner.pending[index];
            if inner.locks && Self::blocked(inner, id) {
                index += 1;
                continue;
            }
            inner.pending.remove(index);
            let Some(job) = inner.jobs.get_mut(&id) else { continue };
            let Some(launch) = job.launch.take() else { continue };
            let now = Instant::now();
            job.view.state = JobState::Running;
            job.view.started_ms = Some(unix_ms());
            job.view.wait_ms = Some(ms(now - job.created));
            job.started = Some(now);
            let (queue, timeout) = (self.clone(), job.timeout);
            let ctx = JobCtx { queue: self.clone(), id };
            // The task cannot touch the queue until we release the lock, so the
            // abort handle is always stored before its first step.
            let task = self.0.runtime.spawn(async move {
                let end = match tokio::time::timeout(timeout, launch(ctx)).await {
                    Ok(result) => End::Returned(result),
                    Err(_) => End::TimedOut,
                };
                queue.finish(id, end);
            });
            job.abort = Some(task.abort_handle());
            inner.running += 1;
            info!("job {id} ({}): started after waiting {} ms", job.view.kind, job.view.wait_ms.unwrap_or(0));
            self.notify(job);
        }
    }

    /// True when a running job holds a resource this job needs.
    fn blocked(inner: &Inner, id: u64) -> bool {
        let Some(job) = inner.jobs.get(&id) else { return false };
        job.view.resources.iter().any(|resource| {
            inner
                .jobs
                .values()
                .any(|other| other.view.state == JobState::Running && other.view.resources.contains(resource))
        })
    }

    /// Resource locks on or off. Turning them off starts any job that was
    /// only waiting for a resource.
    pub fn set_resource_locks(&self, on: bool) {
        let mut inner = self.lock();
        if inner.locks != on {
            info!("jobs: resource locks {}", if on { "on" } else { "off" });
            inner.locks = on;
            self.dispatch(&mut inner);
        }
    }

    /// Changes a running job's view; `send` also sends it to the window.
    fn update(&self, id: u64, send: bool, change: impl FnOnce(&mut JobView)) {
        let mut inner = self.lock();
        let Some(job) = inner.jobs.get_mut(&id) else { return };
        if job.view.state != JobState::Running {
            return;
        }
        change(&mut job.view);
        if send {
            if let Some(started) = job.started {
                job.view.run_ms = Some(ms(started.elapsed()));
            }
            self.notify(job);
        }
    }

    /// Moves a running job to Done, Failed or TimedOut and frees its slot.
    fn finish(&self, id: u64, end: End) {
        let mut inner = self.lock();
        let Some(job) = inner.jobs.get_mut(&id) else { return };
        if job.view.state != JobState::Running {
            return; // cancelled meanwhile; cancel() already freed the slot
        }
        let kind = job.view.kind;
        match end {
            End::Returned(result) => {
                let (succeeded, outcome) = match result {
                    Ok(outcome) => (true, outcome),
                    Err(outcome) => (false, outcome),
                };
                match outcome.check() {
                    Ok(()) => {
                        job.view.label = outcome.label;
                        job.view.value = outcome.value;
                        job.view.message = outcome.message.map(|m| shorten(&m, MAX_TEXT_CHARS));
                        job.view.state = if succeeded { JobState::Done } else { JobState::Failed };
                    }
                    Err(problem) => {
                        job.view.state = JobState::Failed;
                        job.view.message = Some(problem);
                    }
                }
                let label = job.view.label.as_deref().map(|l| format!(" [{l}]")).unwrap_or_default();
                let message = job.view.message.as_deref().unwrap_or("no message");
                if job.view.state == JobState::Done {
                    info!("job {id} ({kind}): done{label}");
                } else {
                    warn!("job {id} ({kind}): failed{label}: {message}");
                }
            }
            End::TimedOut => {
                job.view.state = JobState::TimedOut;
                let message = format!("stopped by its time limit of {} ms", job.view.timeout_ms);
                warn!("job {id} ({kind}): {message}");
                job.view.message = Some(message);
            }
        }
        Self::close(job);
        self.notify(job);
        inner.running -= 1;
        self.dispatch(&mut inner);
        Self::prune(&mut inner);
    }

    fn close(job: &mut Job) {
        job.view.finished_ms = Some(unix_ms());
        if let Some(started) = job.started {
            job.view.run_ms = Some(ms(started.elapsed()));
        }
        job.abort = None;
        job.launch = None;
    }

    /// Cancels a queued job (it never starts) or aborts a running one at its
    /// next await point.
    pub fn cancel(&self, id: u64) -> Result<(), String> {
        let mut inner = self.lock();
        let state = inner.jobs.get(&id).map(|job| job.view.state).ok_or("no such job")?;
        match state {
            JobState::Queued => {
                inner.pending.retain(|&pending| pending != id);
                let job = inner.jobs.get_mut(&id).ok_or("no such job")?;
                job.view.state = JobState::Cancelled;
                job.view.message = Some("cancelled while queued".into());
                job.view.finished_ms = Some(unix_ms());
                job.view.wait_ms = Some(ms(job.created.elapsed()));
                job.launch = None;
                info!("job {id} ({}): cancelled while queued", job.view.kind);
                self.notify(job);
            }
            JobState::Running => {
                let job = inner.jobs.get_mut(&id).ok_or("no such job")?;
                if let Some(abort) = &job.abort {
                    abort.abort();
                }
                job.view.state = JobState::Cancelled;
                job.view.message = Some(format!("cancelled while running, at {} of {}", job.view.done, job.view.total));
                Self::close(job);
                info!("job {id} ({}): cancelled while running", job.view.kind);
                self.notify(job);
                inner.running -= 1;
                self.dispatch(&mut inner);
            }
            _ => return Err("the job has already finished".into()),
        }
        Self::prune(&mut inner);
        Ok(())
    }

    /// Cancels every queued job first (so none starts), then the running ones.
    /// Returns how many were cancelled.
    pub fn cancel_all(&self) -> usize {
        let (queued, running): (Vec<u64>, Vec<u64>) = {
            let inner = self.lock();
            let running = inner
                .jobs
                .values()
                .filter(|job| job.view.state == JobState::Running)
                .map(|job| job.view.id)
                .collect();
            (inner.pending.iter().copied().collect(), running)
        };
        queued.into_iter().chain(running).filter(|&id| self.cancel(id).is_ok()).count()
    }

    /// Removes finished jobs from the list. Returns how many.
    pub fn clear_finished(&self) -> usize {
        let mut inner = self.lock();
        let before = inner.jobs.len();
        inner.jobs.retain(|_, job| !job.view.state.finished());
        before - inner.jobs.len()
    }

    /// Keeps at most MAX_FINISHED finished jobs, dropping the oldest.
    fn prune(inner: &mut Inner) {
        let finished: Vec<u64> =
            inner.jobs.values().filter(|job| job.view.state.finished()).map(|job| job.view.id).collect();
        let extra = finished.len().saturating_sub(MAX_FINISHED);
        for id in finished.into_iter().take(extra) {
            inner.jobs.remove(&id);
        }
    }

    /// New limit. Raising it starts waiting jobs at once; lowering it lets
    /// running jobs finish and starts no new ones until below the limit.
    pub fn set_limit(&self, limit: u32) {
        let mut inner = self.lock();
        let limit = clamp_limit(limit) as usize;
        if inner.limit != limit {
            info!("jobs: {limit} at once (was {})", inner.limit);
            inner.limit = limit;
            self.dispatch(&mut inner);
        }
    }

    pub fn list(&self) -> JobsReply {
        let inner = self.lock();
        let metrics = self.0.runtime.metrics();
        JobsReply {
            seq: self.0.seq.load(Ordering::Relaxed),
            jobs: inner.jobs.values().map(|job| job.view.clone()).collect(),
            limit: inner.limit,
            running: inner.running,
            queued: inner.pending.len(),
            runtime: RuntimeView {
                workers: metrics.num_workers(),
                alive_tasks: metrics.num_alive_tasks(),
                global_queue_depth: metrics.global_queue_depth(),
            },
        }
    }

    pub fn inspect(&self, id: u64) -> Result<JobInspect, String> {
        let inner = self.lock();
        let job = inner.jobs.get(&id).ok_or("no such job")?;
        Ok(JobInspect {
            job: job.view.clone(),
            task_id: job.abort.as_ref().map(|abort| abort.id().to_string()),
            task_finished: job.abort.as_ref().map(AbortHandle::is_finished),
            queue_position: inner.pending.iter().position(|&pending| pending == id).map(|i| i + 1),
            output: job.output.clone(),
            output_cut: job.output_cut,
        })
    }

    /// The last `max` bytes (or fewer, on a character boundary) of a job's
    /// output, for a schedule's history.
    pub fn output_tail(&self, id: u64, max: usize) -> Option<String> {
        let inner = self.lock();
        let output = &inner.jobs.get(&id)?.output;
        if output.is_empty() {
            return None;
        }
        Some(output[tail_start(output, max)..].to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job_kinds::test_kind::CountJob;
    use serde_json::json;

    /// Waits far longer than any time limit, for the time limit tests.
    struct Sleepy;
    impl JobKind for Sleepy {
        const NAME: &'static str = "sleepy";
        const CAPABILITIES: &'static [Capability] = &[Capability::Gpu];
        const RESOURCES: &'static [&'static str] = &["gpu"];
        const TIMEOUT: Duration = Duration::from_secs(5);
        type Input = Value;
        fn check(_: &Value) -> Result<(), String> {
            Ok(())
        }
        fn run(ctx: JobCtx, _: Value) -> impl Future<Output = JobResult> + Send + 'static {
            async move {
                ctx.progress(1, 0);
                tokio::time::sleep(Duration::from_secs(3600)).await;
                Ok(Outcome::new())
            }
        }
    }

    /// Returns whatever outcome its input describes, to test the checks.
    struct Echo;
    #[derive(serde::Deserialize, Serialize)]
    struct EchoInput {
        ok: bool,
        label: Option<String>,
        value_bytes: usize,
    }
    impl JobKind for Echo {
        const NAME: &'static str = "echo";
        const CAPABILITIES: &'static [Capability] = &[];
        const RESOURCES: &'static [&'static str] = &[];
        type Input = EchoInput;
        fn check(_: &EchoInput) -> Result<(), String> {
            Ok(())
        }
        fn run(_: JobCtx, input: EchoInput) -> impl Future<Output = JobResult> + Send + 'static {
            async move {
                let mut outcome = Outcome::new().with_message("echo");
                if let Some(label) = &input.label {
                    outcome = outcome.with_label(label);
                }
                if input.value_bytes > 0 {
                    outcome = outcome.with_value(json!("x".repeat(input.value_bytes)));
                }
                if input.ok {
                    Ok(outcome)
                } else {
                    Err(outcome)
                }
            }
        }
    }

    static TEST_KINDS: &[KindEntry] = &[kind::<CountJob>(), kind::<Sleepy>(), kind::<Echo>()];

    // Tests run with Tokio's clock paused: sleeps finish instantly in
    // simulated time, so a 100-step job takes milliseconds to test.

    fn queue(limit: u32) -> Queue {
        Queue::new(Handle::current(), limit, Arc::new(|_| {}), TEST_KINDS)
    }

    fn count(q: &Queue, count: u32, delay_ms: u64, fail_at: Option<u32>) -> Result<u64, String> {
        q.start("count", json!({ "count": count, "delay_ms": delay_ms, "fail_at": fail_at }), Origin::User, None)
    }

    fn state(q: &Queue, id: u64) -> JobState {
        q.inspect(id).unwrap().job.state
    }

    fn view(q: &Queue, id: u64) -> JobView {
        q.inspect(id).unwrap().job
    }

    async fn wait(millis: u64) {
        tokio::time::sleep(Duration::from_millis(millis)).await;
    }

    #[tokio::test(start_paused = true)]
    async fn one_at_a_time_runs_in_order() {
        let q = queue(1);
        let ids: Vec<u64> = (0..3).map(|_| count(&q, 3, 100, None).unwrap()).collect();
        assert_eq!(state(&q, ids[0]), JobState::Running);
        assert_eq!(state(&q, ids[1]), JobState::Queued);
        assert_eq!(q.inspect(ids[2]).unwrap().queue_position, Some(2));
        wait(1000).await;
        for &id in &ids {
            assert_eq!(state(&q, id), JobState::Done);
            assert_eq!(view(&q, id).done, 3);
        }
        // Each waited for the previous one: two 100 ms sleeps per job.
        assert_eq!(view(&q, ids[0]).wait_ms, Some(0));
        assert_eq!(view(&q, ids[1]).wait_ms, Some(200));
        assert_eq!(view(&q, ids[2]).wait_ms, Some(400));
        assert_eq!(q.list().running, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn limit_is_respected_and_can_be_raised() {
        let q = queue(2);
        let ids: Vec<u64> = (0..3).map(|_| count(&q, 10, 1000, None).unwrap()).collect();
        assert_eq!(q.list().running, 2);
        assert_eq!(state(&q, ids[2]), JobState::Queued);
        q.set_limit(3);
        assert_eq!(state(&q, ids[2]), JobState::Running);
        q.set_limit(9); // clamped to 3
        assert_eq!(q.list().limit, 3);
    }

    #[tokio::test(start_paused = true)]
    async fn lowering_the_limit_lets_running_jobs_finish() {
        let q = queue(3);
        let ids: Vec<u64> = (0..4).map(|_| count(&q, 2, 100, None).unwrap()).collect();
        q.set_limit(1);
        assert_eq!(q.list().running, 3, "running jobs are not stopped");
        wait(150).await;
        // The three finished; only one more may start.
        assert_eq!(state(&q, ids[3]), JobState::Running);
        assert_eq!(q.list().running, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_while_queued_never_starts() {
        let q = queue(1);
        let first = count(&q, 2, 100, None).unwrap();
        let second = count(&q, 2, 100, None).unwrap();
        q.cancel(second).unwrap();
        wait(1000).await;
        assert_eq!(state(&q, first), JobState::Done);
        let cancelled = view(&q, second);
        assert_eq!(cancelled.state, JobState::Cancelled);
        assert_eq!(cancelled.started_ms, None);
        assert_eq!(cancelled.done, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_while_running_stops_counting_and_frees_the_slot() {
        let q = queue(1);
        let long = count(&q, 100, 1000, None).unwrap();
        let next = count(&q, 2, 100, None).unwrap();
        wait(2500).await;
        assert_eq!(view(&q, long).done, 3);
        let task = q.inspect(long).unwrap();
        assert!(task.task_id.is_some());
        q.cancel(long).unwrap();
        assert_eq!(state(&q, next), JobState::Running, "the next job takes the slot");
        wait(5000).await;
        let cancelled = view(&q, long);
        assert_eq!(cancelled.state, JobState::Cancelled);
        assert_eq!(cancelled.done, 3, "no counting after the abort");
        assert_eq!(state(&q, next), JobState::Done);
        assert!(q.cancel(long).is_err(), "cannot cancel twice");
    }

    #[tokio::test(start_paused = true)]
    async fn abort_really_ends_the_tokio_task() {
        let q = queue(1);
        let id = count(&q, 100, 1000, None).unwrap();
        wait(10).await;
        // Keep our own handle: the queue drops its copy on cancel.
        let abort = q.0.inner.lock().unwrap().jobs[&id].abort.clone().unwrap();
        assert!(!abort.is_finished());
        q.cancel(id).unwrap();
        wait(10).await;
        assert!(abort.is_finished(), "Tokio dropped the task at its sleep");
    }

    #[tokio::test(start_paused = true)]
    async fn fail_on_purpose_frees_the_slot() {
        let q = queue(1);
        let failing = count(&q, 10, 100, Some(4)).unwrap();
        let next = count(&q, 1, 100, None).unwrap();
        wait(1000).await;
        let failed = view(&q, failing);
        assert_eq!(failed.state, JobState::Failed);
        assert_eq!(failed.done, 3);
        assert!(failed.message.unwrap().contains("step 4"));
        assert_eq!(state(&q, next), JobState::Done);
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_all_and_clear_finished() {
        let q = queue(2);
        for _ in 0..5 {
            count(&q, 50, 1000, None).unwrap();
        }
        assert_eq!(q.cancel_all(), 5);
        let list = q.list();
        assert_eq!((list.running, list.queued), (0, 0));
        assert_eq!(q.clear_finished(), 5);
        assert!(q.list().jobs.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn bad_requests_are_refused() {
        let q = queue(1);
        assert!(count(&q, 0, 1000, None).is_err());
        assert!(count(&q, 101, 1000, None).is_err());
        assert!(count(&q, 10, 99, None).is_err());
        assert!(count(&q, 10, 10_001, None).is_err());
        assert!(count(&q, 10, 1000, Some(0)).is_err());
        assert!(count(&q, 10, 1000, Some(11)).is_err());
        assert!(q.cancel(42).is_err());
        assert!(q.list().jobs.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn events_arrive_in_order() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let q = Queue::new(
            Handle::current(),
            1,
            Arc::new(move |job: &JobView| log.lock().unwrap().push((job.state, job.done))),
            TEST_KINDS,
        );
        count(&q, 3, 100, None).unwrap();
        wait(1000).await;
        let seen = seen.lock().unwrap().clone();
        use JobState::*;
        assert_eq!(seen, vec![(Queued, 0), (Running, 0), (Running, 1), (Running, 2), (Running, 3), (Done, 3)]);
    }

    #[tokio::test(start_paused = true)]
    async fn seq_rises_with_every_change() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let q = Queue::new(Handle::current(), 2, Arc::new(move |job: &JobView| log.lock().unwrap().push(job.seq)), TEST_KINDS);
        let a = count(&q, 3, 100, None).unwrap();
        count(&q, 3, 100, None).unwrap();
        wait(1000).await;
        let seen = seen.lock().unwrap().clone();
        assert!(seen.windows(2).all(|pair| pair[1] == pair[0] + 1), "{seen:?}");
        let list = q.list();
        assert_eq!(list.seq, *seen.last().unwrap(), "the snapshot knows the newest change");
        assert!(list.jobs.iter().all(|job| job.seq <= list.seq));
        assert_eq!(view(&q, a).state, JobState::Done);
    }

    #[tokio::test(start_paused = true)]
    async fn only_fifty_finished_jobs_are_kept() {
        let q = queue(3);
        for _ in 0..60 {
            count(&q, 1, 100, None).unwrap();
        }
        wait(1000).await;
        let jobs = q.list().jobs;
        assert_eq!(jobs.len(), MAX_FINISHED);
        assert_eq!(jobs[0].id, 11, "the oldest were dropped");
    }

    #[tokio::test(start_paused = true)]
    async fn one_job_per_resource_at_a_time() {
        let q = queue(3);
        let first = q.start("sleepy", json!(1), Origin::User, None).unwrap();
        let second = q.start("sleepy", json!(2), Origin::User, None).unwrap();
        let other = count(&q, 2, 100, None).unwrap();
        assert_eq!(state(&q, first), JobState::Running);
        assert_eq!(state(&q, second), JobState::Queued, "the gpu is held by the first");
        assert_eq!(state(&q, other), JobState::Running, "a job without the resource goes ahead");
        q.cancel(first).unwrap();
        assert_eq!(state(&q, second), JobState::Running, "the gpu was released");
        // With locks off, both run at once.
        q.cancel(second).unwrap();
        q.set_resource_locks(false);
        let a = q.start("sleepy", json!(3), Origin::User, None).unwrap();
        let b = q.start("sleepy", json!(4), Origin::User, None).unwrap();
        assert_eq!((state(&q, a), state(&q, b)), (JobState::Running, JobState::Running));
    }

    #[tokio::test(start_paused = true)]
    async fn turning_locks_off_starts_waiting_jobs() {
        let q = queue(3);
        q.start("sleepy", json!(1), Origin::User, None).unwrap();
        let waiting = q.start("sleepy", json!(2), Origin::User, None).unwrap();
        assert_eq!(state(&q, waiting), JobState::Queued);
        q.set_resource_locks(false);
        assert_eq!(state(&q, waiting), JobState::Running);
    }

    #[tokio::test(start_paused = true)]
    async fn check_refuses_what_start_would_refuse() {
        let q = queue(1);
        assert!(q.check("count", &json!({ "count": 3, "delay_ms": 100 }), None).is_ok());
        assert!(q.check("count", &json!({ "count": 0, "delay_ms": 100 }), None).is_err());
        assert!(q.check("nope", &json!({}), None).is_err());
        assert!(q.check("count", &json!({ "count": 3, "delay_ms": 100 }), Some(1)).is_err());
        assert!(q.list().jobs.is_empty(), "check queues nothing");
    }

    #[tokio::test(start_paused = true)]
    async fn unknown_kinds_and_bad_input_are_refused() {
        let q = queue(1);
        let refused = |kind: &str, input: Value| q.start(kind, input, Origin::User, None).unwrap_err();
        assert!(refused("format_disk", json!({})).contains("unknown job kind"));
        assert!(refused("count", json!({ "count": 3 })).contains("bad input"), "missing field");
        assert!(refused("count", json!({ "count": "3", "delay_ms": 100 })).contains("bad input"), "wrong type");
        assert!(
            refused("count", json!({ "count": 3, "delay_ms": 100, "run": "rm -rf ~" })).contains("bad input"),
            "unknown field"
        );
        assert!(refused("sleepy", json!("x".repeat(MAX_INPUT_BYTES))).contains("bytes"), "input too large");
        let fine = json!({ "count": 3, "delay_ms": 100 });
        assert!(q.start("count", fine.clone(), Origin::User, Some(MIN_TIMEOUT_MS - 1)).is_err());
        assert!(q.start("count", fine, Origin::User, Some(MAX_TIMEOUT_MS + 1)).is_err());
        assert!(q.list().jobs.is_empty(), "nothing was queued");
    }

    #[tokio::test(start_paused = true)]
    async fn time_limit_stops_the_job_and_frees_the_slot() {
        let q = queue(1);
        let slow = q.start("sleepy", json!(null), Origin::User, None).unwrap();
        let next = count(&q, 2, 100, None).unwrap();
        assert_eq!(view(&q, slow).timeout_ms, 5000, "the kind's own limit");
        let abort = q.0.inner.lock().unwrap().jobs[&slow].abort.clone().unwrap();
        wait(4900).await;
        assert_eq!(state(&q, slow), JobState::Running);
        wait(200).await;
        let stopped = view(&q, slow);
        assert_eq!(stopped.state, JobState::TimedOut);
        assert!(stopped.message.unwrap().contains("time limit"));
        assert!(abort.is_finished(), "the Tokio task ended");
        assert_eq!(state(&q, next), JobState::Running, "the next job took the slot");
        // A limit given at start replaces the kind's.
        let short = q.start("sleepy", json!(null), Origin::User, Some(1000)).unwrap();
        wait(1500).await;
        assert_eq!(state(&q, short), JobState::TimedOut);
    }

    #[tokio::test(start_paused = true)]
    async fn the_view_shows_origin_capabilities_and_input() {
        let q = queue(2);
        let id = q.start("sleepy", json!({ "model": "test" }), Origin::User, None).unwrap();
        let job = view(&q, id);
        assert_eq!(job.origin, Origin::User);
        assert_eq!(job.capabilities, &[Capability::Gpu]);
        assert_eq!(job.resources, &["gpu"]);
        assert_eq!(job.input, json!({ "model": "test" }));
        let counted = count(&q, 3, 100, None).unwrap();
        assert_eq!(view(&q, counted).input, json!({ "count": 3, "delay_ms": 100, "fail_at": null }));
        assert!(view(&q, counted).capabilities.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn outcomes_are_kept() {
        let q = queue(3);
        let done = count(&q, 3, 100, None).unwrap();
        let failed = count(&q, 3, 100, Some(2)).unwrap();
        let labelled = q
            .start("echo", json!({ "ok": false, "label": "unavailable", "value_bytes": 0 }), Origin::User, None)
            .unwrap();
        wait(1000).await;
        let done = view(&q, done);
        assert_eq!(done.state, JobState::Done);
        assert_eq!(done.label.as_deref(), Some("counted"));
        assert_eq!(done.value, Some(json!({ "counted": 3 })));
        assert_eq!(done.detail.get("step"), Some(&json!(3)), "the kind's live figures");
        assert_eq!(view(&q, failed).label.as_deref(), Some("on_purpose"));
        let labelled = view(&q, labelled);
        assert_eq!(labelled.state, JobState::Failed);
        assert_eq!(labelled.label.as_deref(), Some("unavailable"));
        assert_eq!(labelled.message.as_deref(), Some("echo"));
    }

    #[tokio::test(start_paused = true)]
    async fn outcomes_over_the_limits_fail_the_job() {
        let q = queue(3);
        let start = |ok: bool, label: Option<String>, value_bytes: usize| {
            q.start("echo", json!({ "ok": ok, "label": label, "value_bytes": value_bytes }), Origin::User, None)
                .unwrap()
        };
        let bad_label = start(true, Some("Not OK!".into()), 0);
        let long_label = start(true, Some("a".repeat(MAX_LABEL_CHARS + 1)), 0);
        let big_value = start(true, None, MAX_VALUE_BYTES);
        wait(10).await;
        for id in [bad_label, long_label, big_value] {
            let job = view(&q, id);
            assert_eq!(job.state, JobState::Failed, "job {id}");
            assert!(job.label.is_none() && job.value.is_none(), "nothing from a bad outcome is kept");
        }
        assert!(view(&q, bad_label).message.unwrap().contains("invalid label"));
        assert!(view(&q, big_value).message.unwrap().contains("bytes"));
        let fine = start(true, Some("a".repeat(MAX_LABEL_CHARS)), MAX_VALUE_BYTES - 2);
        wait(10).await;
        assert_eq!(state(&q, fine), JobState::Done, "exactly at the limits is allowed");
    }
}
