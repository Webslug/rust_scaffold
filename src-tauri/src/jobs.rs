//! Job queue: runs work in Rust on Tauri's Tokio runtime, at most
//! `job_concurrency` jobs at once (config.json, 1 to 3). Extra jobs wait in
//! order. Every change (queued, started, each step, finished) is pushed to the
//! window as a `jobs:changed` event, so the Jobs tab never polls.
//!
//! The only job kind is a mock "count" job: it counts from 1 to N (at most
//! 100) with a pause between counts. It exists to exercise starting, queueing
//! and stopping async work. Real apps add their own kinds next to it; the UI
//! can only ask for kinds defined here, never run arbitrary code.
//!
//! Why a hand-written queue rather than a Tokio `Semaphore`: a semaphore
//! limits how many tasks run, but the waiting tasks already exist and cannot
//! be listed or reordered, and its limit cannot shrink without extra tricks.
//! Here queued jobs are plain data (no task yet), so they can be shown and
//! cancelled for free, and the limit can change while jobs run.
//!
//! Cancelling a running job uses Tokio's `AbortHandle`. Tokio can only stop a
//! task at an `.await` point (here the sleep between counts); code between two
//! awaits always runs to the next one. A job that never awaits cannot be
//! stopped this way. Note: the release build uses `panic = "abort"`, so a
//! panic inside a job ends the whole app. Return an error instead.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::runtime::Handle;
use tokio::task::AbortHandle;
use tokio::time::Instant;
use tracing::{info, warn};

/// Limits for `job_concurrency` in config.json.
pub const MIN_CONCURRENCY: u32 = 1;
pub const MAX_CONCURRENCY: u32 = 3;
pub const DEFAULT_CONCURRENCY: u32 = 2;

/// Limits for a count job.
pub const MAX_COUNT: u32 = 100;
pub const MIN_DELAY_MS: u64 = 100;
pub const MAX_DELAY_MS: u64 = 10_000;

/// Finished jobs kept for inspection; the oldest are dropped first.
const MAX_FINISHED: usize = 50;
/// Jobs waiting at once. Stops a runaway caller filling memory.
const MAX_QUEUED: usize = 200;

/// Window event carrying one job's new state.
pub const CHANGED_EVENT: &str = "jobs:changed";

#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobState {
    fn finished(self) -> bool {
        matches!(self, JobState::Done | JobState::Failed | JobState::Cancelled)
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
    /// Count up to this number.
    pub count: u32,
    pub delay_ms: u64,
    /// Fail on purpose when this step is reached (for testing the Failed state).
    pub fail_at: Option<u32>,
    /// Last number counted.
    pub progress: u32,
    /// Wall-clock times, Unix milliseconds.
    pub created_ms: u64,
    pub started_ms: Option<u64>,
    pub finished_ms: Option<u64>,
    /// Time spent queued, and time spent running.
    pub wait_ms: Option<u64>,
    pub run_ms: Option<u64>,
    /// How much later than asked each sleep woke up, measured by the job.
    /// Shows how busy the runtime and the machine are.
    pub late_avg_ms: f64,
    pub late_max_ms: f64,
    /// Why it failed or was cancelled.
    pub message: Option<String>,
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
    running: usize,
    queued: usize,
    runtime: RuntimeView,
}

struct Job {
    view: JobView,
    created: Instant,
    started: Option<Instant>,
    abort: Option<AbortHandle>,
    late_total: Duration,
    sleeps: u32,
}

struct Inner {
    jobs: BTreeMap<u64, Job>,
    pending: VecDeque<u64>,
    running: usize,
    limit: usize,
    next_id: u64,
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
}

/// Cheap to clone; every clone is the same queue.
#[derive(Clone)]
pub struct Queue(Arc<Shared>);

fn unix_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

pub fn clamp_limit(limit: u32) -> u32 {
    limit.clamp(MIN_CONCURRENCY, MAX_CONCURRENCY)
}

impl Queue {
    /// `runtime` is where jobs run: Tauri's Tokio runtime in the app, the
    /// test's runtime in tests.
    pub fn new(runtime: Handle, limit: u32, notify: Notify) -> Self {
        Queue(Arc::new(Shared {
            inner: Mutex::new(Inner {
                jobs: BTreeMap::new(),
                pending: VecDeque::new(),
                running: 0,
                limit: clamp_limit(limit) as usize,
                next_id: 1,
            }),
            runtime,
            notify,
            seq: AtomicU64::new(0),
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

    /// Queues a count job and starts it at once if a slot is free.
    pub fn start_count(&self, count: u32, delay_ms: u64, fail_at: Option<u32>) -> Result<u64, String> {
        if !(1..=MAX_COUNT).contains(&count) {
            return Err(format!("count must be 1 to {MAX_COUNT}"));
        }
        if !(MIN_DELAY_MS..=MAX_DELAY_MS).contains(&delay_ms) {
            return Err(format!("delay must be {MIN_DELAY_MS} to {MAX_DELAY_MS} ms"));
        }
        if fail_at.is_some_and(|step| step == 0 || step > count) {
            return Err("fail step must be between 1 and the count".into());
        }
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
                kind: "count",
                state: JobState::Queued,
                count,
                delay_ms,
                fail_at,
                progress: 0,
                created_ms: unix_ms(),
                started_ms: None,
                finished_ms: None,
                wait_ms: None,
                run_ms: None,
                late_avg_ms: 0.0,
                late_max_ms: 0.0,
                message: None,
            },
            created: Instant::now(),
            started: None,
            abort: None,
            late_total: Duration::ZERO,
            sleeps: 0,
        };
        self.notify(&mut job);
        inner.jobs.insert(id, job);
        inner.pending.push_back(id);
        info!("job {id}: queued, count to {count}, {delay_ms} ms apart");
        self.dispatch(&mut inner);
        Ok(id)
    }

    /// Starts queued jobs while fewer than `limit` are running.
    fn dispatch(&self, inner: &mut Inner) {
        while inner.running < inner.limit {
            let Some(id) = inner.pending.pop_front() else { break };
            let Some(job) = inner.jobs.get_mut(&id) else { continue };
            let now = Instant::now();
            job.view.state = JobState::Running;
            job.view.started_ms = Some(unix_ms());
            job.view.wait_ms = Some(ms(now - job.created));
            job.started = Some(now);
            let (count, delay, fail_at) = (job.view.count, Duration::from_millis(job.view.delay_ms), job.view.fail_at);
            // The task cannot touch the queue until we release the lock, so the
            // abort handle is always stored before its first step.
            let task = self.0.runtime.spawn(run_count(self.clone(), id, count, delay, fail_at));
            job.abort = Some(task.abort_handle());
            inner.running += 1;
            info!("job {id}: started after waiting {} ms", job.view.wait_ms.unwrap_or(0));
            self.notify(job);
        }
    }

    /// Records a step. Returns false when the job is no longer running (it
    /// was cancelled), so the task stops.
    fn step(&self, id: u64, progress: u32, late: Option<Duration>) -> bool {
        let mut inner = self.lock();
        let Some(job) = inner.jobs.get_mut(&id) else { return false };
        if job.view.state != JobState::Running {
            return false;
        }
        job.view.progress = progress;
        if let Some(late) = late {
            job.sleeps += 1;
            job.late_total += late;
            let late_ms = late.as_secs_f64() * 1000.0;
            job.view.late_max_ms = job.view.late_max_ms.max(late_ms);
            job.view.late_avg_ms = job.late_total.as_secs_f64() * 1000.0 / job.sleeps as f64;
        }
        if let Some(started) = job.started {
            job.view.run_ms = Some(ms(started.elapsed()));
        }
        self.notify(job);
        true
    }

    /// Moves a running job to Done or Failed and frees its slot.
    fn finish(&self, id: u64, result: Result<(), String>) {
        let mut inner = self.lock();
        let Some(job) = inner.jobs.get_mut(&id) else { return };
        if job.view.state != JobState::Running {
            return; // cancelled meanwhile; cancel() already freed the slot
        }
        match result {
            Ok(()) => {
                job.view.state = JobState::Done;
                info!("job {id}: done");
            }
            Err(message) => {
                job.view.state = JobState::Failed;
                warn!("job {id}: failed: {message}");
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
                info!("job {id}: cancelled while queued");
                self.notify(job);
            }
            JobState::Running => {
                let job = inner.jobs.get_mut(&id).ok_or("no such job")?;
                if let Some(abort) = &job.abort {
                    abort.abort();
                }
                job.view.state = JobState::Cancelled;
                job.view.message = Some(format!("cancelled while running, at {} of {}", job.view.progress, job.view.count));
                Self::close(job);
                info!("job {id}: cancelled while running");
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
        })
    }
}

/// The mock job: counts 1..=count, sleeping `delay` between counts. Each
/// sleep is an await point, so this is where an abort takes effect.
async fn run_count(queue: Queue, id: u64, count: u32, delay: Duration, fail_at: Option<u32>) {
    for step in 1..=count {
        let late = if step > 1 {
            let before = Instant::now();
            tokio::time::sleep(delay).await;
            Some(before.elapsed().saturating_sub(delay))
        } else {
            None
        };
        if fail_at == Some(step) {
            queue.finish(id, Err(format!("failed on purpose at step {step}")));
            return;
        }
        if !queue.step(id, step, late) {
            return;
        }
    }
    queue.finish(id, Ok(()));
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests run with Tokio's clock paused: sleeps finish instantly in
    // simulated time, so a 100-step job takes milliseconds to test.

    fn queue(limit: u32) -> Queue {
        Queue::new(Handle::current(), limit, Arc::new(|_| {}))
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
        let ids: Vec<u64> = (0..3).map(|_| q.start_count(3, 100, None).unwrap()).collect();
        assert_eq!(state(&q, ids[0]), JobState::Running);
        assert_eq!(state(&q, ids[1]), JobState::Queued);
        assert_eq!(q.inspect(ids[2]).unwrap().queue_position, Some(2));
        wait(1000).await;
        for &id in &ids {
            assert_eq!(state(&q, id), JobState::Done);
            assert_eq!(view(&q, id).progress, 3);
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
        let ids: Vec<u64> = (0..3).map(|_| q.start_count(10, 1000, None).unwrap()).collect();
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
        let ids: Vec<u64> = (0..4).map(|_| q.start_count(2, 100, None).unwrap()).collect();
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
        let first = q.start_count(2, 100, None).unwrap();
        let second = q.start_count(2, 100, None).unwrap();
        q.cancel(second).unwrap();
        wait(1000).await;
        assert_eq!(state(&q, first), JobState::Done);
        let cancelled = view(&q, second);
        assert_eq!(cancelled.state, JobState::Cancelled);
        assert_eq!(cancelled.started_ms, None);
        assert_eq!(cancelled.progress, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_while_running_stops_counting_and_frees_the_slot() {
        let q = queue(1);
        let long = q.start_count(100, 1000, None).unwrap();
        let next = q.start_count(2, 100, None).unwrap();
        wait(2500).await;
        assert_eq!(view(&q, long).progress, 3);
        let task = q.inspect(long).unwrap();
        assert!(task.task_id.is_some());
        q.cancel(long).unwrap();
        assert_eq!(state(&q, next), JobState::Running, "the next job takes the slot");
        wait(5000).await;
        let cancelled = view(&q, long);
        assert_eq!(cancelled.state, JobState::Cancelled);
        assert_eq!(cancelled.progress, 3, "no counting after the abort");
        assert_eq!(state(&q, next), JobState::Done);
        assert!(q.cancel(long).is_err(), "cannot cancel twice");
    }

    #[tokio::test(start_paused = true)]
    async fn abort_really_ends_the_tokio_task() {
        let q = queue(1);
        let id = q.start_count(100, 1000, None).unwrap();
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
        let failing = q.start_count(10, 100, Some(4)).unwrap();
        let next = q.start_count(1, 100, None).unwrap();
        wait(1000).await;
        let failed = view(&q, failing);
        assert_eq!(failed.state, JobState::Failed);
        assert_eq!(failed.progress, 3);
        assert!(failed.message.unwrap().contains("step 4"));
        assert_eq!(state(&q, next), JobState::Done);
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_all_and_clear_finished() {
        let q = queue(2);
        for _ in 0..5 {
            q.start_count(50, 1000, None).unwrap();
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
        assert!(q.start_count(0, 1000, None).is_err());
        assert!(q.start_count(101, 1000, None).is_err());
        assert!(q.start_count(10, 99, None).is_err());
        assert!(q.start_count(10, 10_001, None).is_err());
        assert!(q.start_count(10, 1000, Some(0)).is_err());
        assert!(q.start_count(10, 1000, Some(11)).is_err());
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
            Arc::new(move |job: &JobView| log.lock().unwrap().push((job.state, job.progress))),
        );
        q.start_count(3, 100, None).unwrap();
        wait(1000).await;
        let seen = seen.lock().unwrap().clone();
        use JobState::*;
        assert_eq!(seen, vec![(Queued, 0), (Running, 0), (Running, 1), (Running, 2), (Running, 3), (Done, 3)]);
    }

    #[tokio::test(start_paused = true)]
    async fn seq_rises_with_every_change() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let q = Queue::new(Handle::current(), 2, Arc::new(move |job: &JobView| log.lock().unwrap().push(job.seq)));
        let a = q.start_count(3, 100, None).unwrap();
        q.start_count(3, 100, None).unwrap();
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
            q.start_count(1, 100, None).unwrap();
        }
        wait(1000).await;
        let jobs = q.list().jobs;
        assert_eq!(jobs.len(), MAX_FINISHED);
        assert_eq!(jobs[0].id, 11, "the oldest were dropped");
    }
}
