//! Scheduler: starts jobs at set times, from `jobs.json` in the settings
//! folder. Each schedule names a job kind and its input (as on the Jobs
//! page) and when to run it: once at a time, every so often, daily, weekly
//! or monthly, optionally a limited number of times or until a date.
//!
//! Jobs run only while the app is open. A run missed while it was closed is
//! made up once when the app starts (`catch_up`, on by default) or skipped.
//! If a schedule's previous job is still queued or running when it is due
//! again, that run is skipped rather than piling up.
//!
//! Rules in the same file react to results: when schedule A ends a given
//! way (optionally with a given label), N times in a row, start, enable or
//! disable schedule B. A chain of rules stops after `MAX_CHAIN` links, so
//! "A starts B starts A" cannot run forever. A schedule can also retry a
//! failed run after a wait; only the final result counts for the failure
//! limit and for rules. "Pause everything" stops all starts by timetable
//! and rules (Run now still works).
//!
//! The Jobs page adds, changes and deletes schedules and rules by
//! rewriting jobs.json (checked first, written atomically). Hand edits still
//! work: press Reload.
//!
//! Run counts and results are kept in `jobs_state.json`, so the app never
//! rewrites the file a person edits. Deleting it resets counts only.
//!
//! One task on Tauri's Tokio runtime does the timing. It sleeps until the
//! next run is due, but never longer than 60 s at a time, because Tokio's
//! clock stops while the computer is suspended: on waking it checks the
//! wall clock again. Times in jobs.json are local; each one is converted
//! with the local UTC offset of that moment, so "daily at 08:00" stays at
//! 08:00 across a clock change.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use time::macros::format_description;
use time::{Date, Month, OffsetDateTime, PlainDateTime, Time, UtcOffset, Weekday};
use tokio::runtime::Handle;
use tracing::{info, warn};

use crate::files::{self, FileStatus};
use crate::jobs::{JobState, KindEntry, Origin, Outcome, Previous, Queue, MAX_OUTPUT_BYTES};

pub const SCHEDULES_FILE: &str = "jobs.json";
pub const STATE_FILE: &str = "jobs_state.json";
pub const DEFAULT_SCHEDULES: &str = include_str!("../jobs.default.json");
const EMPTY_STATE: &str = "{\"version\":1,\"schedules\":{}}";
pub const MAX_SCHEDULES_BYTES: u64 = 1024 * 1024;
/// jobs_state.json: at most 100 schedules x 20 runs x 400 bytes of output,
/// plus counts, is well under this.
const MAX_STATE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SCHEDULES: usize = 100;
const MAX_RULES: usize = 100;
const MAX_NAME_CHARS: usize = 32;
/// Most retries after a failed run, and the default wait before the first.
const MAX_RETRIES: u32 = 10;
const DEFAULT_RETRY_WAIT: &str = "1m";
/// Most "in a row" a rule can ask for.
const MAX_TIMES: u32 = 1000;
/// Longest chain of rules (A starts B starts C ...) before the loop guard
/// stops it.
pub const MAX_CHAIN: u32 = 10;
/// Runs kept per schedule in jobs_state.json, and how much of each run's
/// output (its end) is kept with it. The full output is on Inspect while
/// the job is listed.
const MAX_HISTORY: usize = 20;
const HISTORY_OUTPUT_BYTES: usize = 400;
/// How far ahead the Jobs page preview looks, and how many runs it lists.
const PREVIEW: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_PREVIEW: usize = 100;
/// Shortest and longest `every` interval.
pub const MIN_INTERVAL: Duration = Duration::from_secs(5);
pub const MAX_INTERVAL: Duration = Duration::from_secs(366 * 24 * 60 * 60);
/// Longest single sleep, so a suspend cannot delay a run by more than this.
const MAX_SLEEP: Duration = Duration::from_secs(60);
/// How often to look for results while a scheduled job is running.
const RESULT_POLL: Duration = Duration::from_secs(1);

pub const DEFAULT_FAILURE_LIMIT: u32 = 3;
pub const MAX_FAILURE_LIMIT: u32 = 100;

/// Window event carrying the whole schedule list after any change.
pub const CHANGED_EVENT: &str = "schedules:changed";

/// When to run, as written in jobs.json.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum When {
    /// `{"type":"manual"}`: only Run now or a rule starts it.
    Manual,
    /// `{"type":"once","at":"2026-12-01 09:00"}`
    Once { at: String },
    /// `{"type":"every","interval":"10m"}`: a number and s, m, h or d.
    Every { interval: String },
    /// `{"type":"daily","time":"08:00"}`
    Daily { time: String },
    /// `{"type":"weekly","day":"sunday","time":"03:00"}`
    Weekly { day: String, time: String },
    /// `{"type":"monthly","day":31,"time":"23:00"}`: day 31 in a shorter
    /// month means its last day.
    Monthly { day: u8, time: String },
}

/// `When`, checked and parsed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Trigger {
    Manual,
    Once(PlainDateTime),
    Every(Duration),
    Daily(Time),
    Weekly(Weekday, Time),
    Monthly(u8, Time),
}

fn parse_time(text: &str) -> Result<Time, String> {
    Time::parse(text, format_description!("[hour]:[minute]")).map_err(|_| format!("time \"{text}\" is not HH:MM"))
}

pub fn parse_local(text: &str) -> Result<PlainDateTime, String> {
    PlainDateTime::parse(text, format_description!("[year]-[month]-[day] [hour]:[minute]"))
        .map_err(|_| format!("date \"{text}\" is not YYYY-MM-DD HH:MM"))
}

fn parse_interval(text: &str) -> Result<Duration, String> {
    let text = text.trim();
    let bad = || format!("interval \"{text}\" is not a number followed by s, m, h or d (for example 10m)");
    let unit = text.chars().last().ok_or_else(bad)?;
    let number: u64 = text[..text.len() - unit.len_utf8()].trim().parse().map_err(|_| bad())?;
    let seconds = match unit {
        's' => Some(number),
        'm' => number.checked_mul(60),
        'h' => number.checked_mul(60 * 60),
        'd' => number.checked_mul(24 * 60 * 60),
        _ => return Err(bad()),
    };
    let interval = Duration::from_secs(seconds.ok_or_else(bad)?);
    if !(MIN_INTERVAL..=MAX_INTERVAL).contains(&interval) {
        return Err(format!("interval \"{text}\" must be 5s to 366d"));
    }
    Ok(interval)
}

fn parse_weekday(text: &str) -> Result<Weekday, String> {
    let day = match text.to_ascii_lowercase().as_str() {
        "monday" => Weekday::Monday,
        "tuesday" => Weekday::Tuesday,
        "wednesday" => Weekday::Wednesday,
        "thursday" => Weekday::Thursday,
        "friday" => Weekday::Friday,
        "saturday" => Weekday::Saturday,
        "sunday" => Weekday::Sunday,
        _ => return Err(format!("day \"{text}\" is not a weekday name")),
    };
    Ok(day)
}

impl When {
    pub fn parse(&self) -> Result<Trigger, String> {
        Ok(match self {
            When::Manual => Trigger::Manual,
            When::Once { at } => Trigger::Once(parse_local(at)?),
            When::Every { interval } => Trigger::Every(parse_interval(interval)?),
            When::Daily { time } => Trigger::Daily(parse_time(time)?),
            When::Weekly { day, time } => Trigger::Weekly(parse_weekday(day)?, parse_time(time)?),
            When::Monthly { day, time } => {
                if !(1..=31).contains(day) {
                    return Err(format!("monthly day {day} must be 1 to 31"));
                }
                Trigger::Monthly(*day, parse_time(time)?)
            }
        })
    }
}

fn hhmm(time: Time) -> String {
    format!("{:02}:{:02}", time.hour(), time.minute())
}

fn plural(n: u64, unit: &str) -> String {
    if n == 1 {
        format!("1 {unit}")
    } else {
        format!("{n} {unit}s")
    }
}

impl Trigger {
    /// In words, for the Jobs page.
    pub fn describe(&self) -> String {
        match *self {
            Trigger::Manual => "manual (Run now or a rule)".into(),
            Trigger::Once(at) => format!("once, {} {}", at.date(), hhmm(at.time())),
            Trigger::Every(interval) => {
                let s = interval.as_secs();
                let text = if s % 86_400 == 0 {
                    plural(s / 86_400, "day")
                } else if s % 3600 == 0 {
                    plural(s / 3600, "hour")
                } else if s % 60 == 0 {
                    plural(s / 60, "minute")
                } else {
                    plural(s, "second")
                };
                format!("every {text}")
            }
            Trigger::Daily(time) => format!("daily at {}", hhmm(time)),
            Trigger::Weekly(day, time) => format!("every {day} at {}", hhmm(time)),
            Trigger::Monthly(day, time) if day > 28 => {
                format!("monthly on day {day} at {} (or the month's last day)", hhmm(time))
            }
            Trigger::Monthly(day, time) => format!("monthly on day {day} at {}", hhmm(time)),
        }
    }
}

/// The local UTC offset at an instant. Tests pass their own.
pub type OffsetAt = fn(OffsetDateTime) -> UtcOffset;

pub fn system_offset(at: OffsetDateTime) -> UtcOffset {
    UtcOffset::local_offset_at(at).unwrap_or(UtcOffset::UTC)
}

/// A local wall-clock time as an instant: guess with the offset at that
/// time read as UTC, then correct with the offset at the guess.
fn local_instant(local: PlainDateTime, offset_at: OffsetAt) -> OffsetDateTime {
    let guess = local.assume_offset(offset_at(local.assume_utc()));
    local.assume_offset(offset_at(guess))
}

fn local_date(at: OffsetDateTime, offset_at: OffsetAt) -> Date {
    at.to_offset(offset_at(at)).date()
}

fn add_month(date: Date) -> Option<(i32, Month)> {
    let (year, month) = (date.year(), date.month());
    Some(if month == Month::December { (year.checked_add(1)?, Month::January) } else { (year, month.next()) })
}

/// The first run strictly after `after`, or None when there is none (a
/// `once` time already past).
pub fn next_after(trigger: Trigger, after: OffsetDateTime, offset_at: OffsetAt) -> Option<OffsetDateTime> {
    let later = |local: PlainDateTime| Some(local_instant(local, offset_at)).filter(|t| *t > after);
    match trigger {
        Trigger::Manual => None,
        Trigger::Once(at) => later(at),
        Trigger::Every(interval) => after.checked_add(interval.try_into().ok()?),
        Trigger::Daily(time) => {
            let mut date = local_date(after, offset_at);
            for _ in 0..3 {
                if let Some(t) = later(date.with_time(time)) {
                    return Some(t);
                }
                date = date.next_day()?;
            }
            None
        }
        Trigger::Weekly(day, time) => {
            let mut date = local_date(after, offset_at);
            for _ in 0..9 {
                if date.weekday() == day {
                    if let Some(t) = later(date.with_time(time)) {
                        return Some(t);
                    }
                }
                date = date.next_day()?;
            }
            None
        }
        Trigger::Monthly(day, time) => {
            let start = local_date(after, offset_at);
            let (mut year, mut month) = (start.year(), start.month());
            for _ in 0..14 {
                let date = Date::from_calendar_date(year, month, day.min(month.length(year))).ok()?;
                if let Some(t) = later(date.with_time(time)) {
                    return Some(t);
                }
                (year, month) = add_month(date)?;
            }
            None
        }
    }
}

/// One schedule from jobs.json, as written.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScheduleText {
    name: String,
    kind: String,
    #[serde(default)]
    input: Value,
    when: When,
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default)]
    max_runs: Option<u64>,
    #[serde(default)]
    until: Option<String>,
    #[serde(default = "yes")]
    catch_up: bool,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    retries: u32,
    #[serde(default)]
    retry_wait: Option<String>,
    #[serde(default = "yes")]
    retry_doubling: bool,
    /// Free text for people; ignored.
    #[serde(default)]
    #[allow(dead_code)]
    note: Option<String>,
}

fn yes() -> bool {
    true
}

fn one() -> u32 {
    1
}

/// Names of schedules and rules, and result labels: lower-case letters,
/// digits and _, at most 32.
fn valid_word(text: &str) -> bool {
    !text.is_empty()
        && text.chars().count() <= MAX_NAME_CHARS
        && text.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// How a run must end for a rule to count it.
#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Ended {
    Done,
    Failed,
    TimedOut,
    Killed,
    /// Failed, timed out or killed.
    NotDone,
    Any,
}

impl Ended {
    fn matches(self, state: JobState) -> bool {
        match self {
            Ended::Done => state == JobState::Done,
            Ended::Failed => state == JobState::Failed,
            Ended::TimedOut => state == JobState::TimedOut,
            Ended::Killed => state == JobState::Cancelled,
            Ended::NotDone => state != JobState::Done,
            Ended::Any => true,
        }
    }

    fn words(self) -> &'static str {
        match self {
            Ended::Done => "succeeds",
            Ended::Failed => "fails",
            Ended::TimedOut => "times out",
            Ended::Killed => "is killed",
            Ended::NotDone => "does not succeed",
            Ended::Any => "finishes",
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Start,
    Enable,
    Disable,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleWhenText {
    schedule: String,
    ended: Ended,
    #[serde(default)]
    label: Option<String>,
    #[serde(default = "one")]
    times: u32,
    #[serde(default)]
    value: Option<ValueTest>,
    #[serde(default)]
    changed: bool,
}

/// A test on one field of a run's result value: `{"field": "percent",
/// "above": 90}`. Several tests in one must all pass.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ValueTest {
    pub field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub above: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub below: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals: Option<Value>,
}

impl ValueTest {
    fn check(&self) -> Result<(), String> {
        let path_ok = !self.field.is_empty()
            && self.field.len() <= 100
            && self.field.split('.').all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
        if !path_ok {
            return Err("the value field must be a name such as percent, or disk.percent for a field inside another".into());
        }
        if self.above.is_none() && self.below.is_none() && self.equals.is_none() {
            return Err("the value test needs above, below or equals".into());
        }
        if self.above.is_some_and(|a| !a.is_finite()) || self.below.is_some_and(|b| !b.is_finite()) {
            return Err("above and below must be numbers".into());
        }
        Ok(())
    }

    /// Whether `value` passes. A missing field, or text where a number is
    /// needed, does not pass.
    fn passes(&self, value: Option<&Value>) -> bool {
        let Some(found) = value.and_then(|v| crate::jobs::value_field(v, &self.field)) else { return false };
        if let Some(want) = &self.equals {
            let same = match (found.as_f64(), want.as_f64()) {
                (Some(a), Some(b)) => a == b,
                _ => found == want,
            };
            if !same {
                return false;
            }
        }
        if self.above.is_some() || self.below.is_some() {
            let Some(n) = found.as_f64() else { return false };
            if self.above.is_some_and(|a| n <= a) || self.below.is_some_and(|b| n >= b) {
                return false;
            }
        }
        true
    }

    fn words(&self) -> String {
        let mut parts = Vec::new();
        if let Some(a) = self.above {
            parts.push(format!("above {a}"));
        }
        if let Some(b) = self.below {
            parts.push(format!("below {b}"));
        }
        if let Some(e) = &self.equals {
            parts.push(format!("equal to {e}"));
        }
        format!("{} {}", self.field, parts.join(" and "))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleThenText {
    action: Action,
    schedule: String,
}

/// One rule from jobs.json, as written.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleText {
    name: String,
    #[serde(default = "yes")]
    enabled: bool,
    when: RuleWhenText,
    then: RuleThenText,
    #[serde(default)]
    #[allow(dead_code)]
    note: Option<String>,
}

/// A rule, checked: when `source` ends as `ended` (with `label`, if set),
/// `times` in a row, do `action` to `target`.
#[derive(Clone, Debug)]
pub struct Rule {
    pub name: String,
    pub enabled: bool,
    pub source: String,
    pub ended: Ended,
    pub label: Option<String>,
    pub times: u32,
    pub value: Option<ValueTest>,
    pub changed: bool,
    pub action: Action,
    pub target: String,
    /// As written in the file, for the edit form.
    pub raw: Value,
}

impl Rule {
    fn matches(&self, done: &Finished) -> bool {
        self.ended.matches(done.state)
            && self.label.as_deref().is_none_or(|want| done.outcome.label.as_deref() == Some(want))
            && self.value.as_ref().is_none_or(|test| test.passes(done.outcome.value.as_ref()))
            && (!self.changed || done.changed)
    }

    /// In words, for the Jobs page.
    pub fn describe(&self) -> String {
        let mut label = self.label.as_ref().map(|l| format!(" with result \"{l}\"")).unwrap_or_default();
        if let Some(test) = &self.value {
            label.push_str(&format!(", value {}", test.words()));
        }
        if self.changed {
            label.push_str(", changed since the last success");
        }
        let times = if self.times > 1 { format!(", {} times in a row", self.times) } else { String::new() };
        let action = match self.action {
            Action::Start => "start",
            Action::Enable => "enable",
            Action::Disable => "disable",
        };
        format!("when {} {}{label}{times}: {action} {}", self.source, self.ended.words(), self.target)
    }
}

/// An entry of jobs.json that was skipped, and why.
#[derive(Clone, Debug)]
pub struct Skip {
    pub what: &'static str,
    pub index: usize,
    pub name: Option<String>,
    pub reason: String,
}

impl Skip {
    pub fn text(&self) -> String {
        let name = self.name.as_ref().map(|n| format!(" \"{n}\"")).unwrap_or_default();
        format!("{} {}{name}: {}", self.what, self.index + 1, self.reason)
    }
}

pub struct Parsed {
    pub schedules: Vec<Schedule>,
    pub rules: Vec<Rule>,
    pub skipped: Vec<Skip>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileText {
    version: u32,
    #[serde(default)]
    note: Option<Value>,
    schedules: Vec<Value>,
    #[serde(default)]
    rules: Vec<Value>,
}

/// A schedule, checked.
#[derive(Clone, Debug)]
pub struct Schedule {
    pub name: String,
    pub kind: String,
    pub input: Value,
    pub trigger: Trigger,
    pub enabled: bool,
    pub max_runs: Option<u64>,
    pub until: Option<PlainDateTime>,
    pub catch_up: bool,
    pub timeout_ms: Option<u64>,
    pub retries: u32,
    pub retry_wait: Duration,
    pub retry_doubling: bool,
    /// As written in the file, for the edit form.
    pub raw: Value,
}

impl Schedule {
    /// The wait before retry number `attempt` (1 for the first).
    fn retry_after(&self, attempt: u32) -> Duration {
        if !self.retry_doubling {
            return self.retry_wait;
        }
        let factor = 1u32.checked_shl(attempt.saturating_sub(1)).unwrap_or(u32::MAX);
        self.retry_wait.checked_mul(factor).unwrap_or(MAX_INTERVAL).min(MAX_INTERVAL)
    }
}

/// Parses jobs.json. Bad schedules and rules are skipped with a reason, so
/// one typo does not stop the rest; a file that is not valid at all is an
/// error. A rule naming a schedule that was skipped is skipped too.
pub fn parse(text: &str, queue: &Queue) -> Result<Parsed, String> {
    let file: FileText = serde_json::from_str(text).map_err(|e| format!("not valid: {e}"))?;
    let _ = file.note;
    if file.version != 1 {
        return Err(format!("version {} is not known (expected 1)", file.version));
    }
    if file.schedules.len() > MAX_SCHEDULES {
        return Err(format!("{} schedules (at most {MAX_SCHEDULES})", file.schedules.len()));
    }
    if file.rules.len() > MAX_RULES {
        return Err(format!("{} rules (at most {MAX_RULES})", file.rules.len()));
    }
    let name_of = |value: &Value| value.get("name").and_then(Value::as_str).map(str::to_string);
    let mut schedules: Vec<Schedule> = Vec::new();
    let mut skipped = Vec::new();
    for (index, value) in file.schedules.into_iter().enumerate() {
        let name = name_of(&value);
        let raw = value.clone();
        let checked = serde_json::from_value::<ScheduleText>(value).map_err(|e| e.to_string()).and_then(|s| {
            if !valid_word(&s.name) {
                return Err("the name must be lower-case letters, digits and _, at most 32".into());
            }
            if schedules.iter().any(|other| other.name == s.name) {
                return Err("the name is used twice".into());
            }
            let trigger = s.when.parse()?;
            let until = s.until.as_deref().map(parse_local).transpose()?;
            if s.max_runs == Some(0) {
                return Err("max_runs must be at least 1".into());
            }
            if s.retries > MAX_RETRIES {
                return Err(format!("retries must be 0 to {MAX_RETRIES}"));
            }
            let retry_wait = parse_interval(s.retry_wait.as_deref().unwrap_or(DEFAULT_RETRY_WAIT))?;
            queue.check(&s.kind, &s.input, s.timeout_ms)?;
            Ok(Schedule {
                name: s.name,
                kind: s.kind,
                input: s.input,
                trigger,
                enabled: s.enabled,
                max_runs: s.max_runs,
                until,
                catch_up: s.catch_up,
                timeout_ms: s.timeout_ms,
                retries: s.retries,
                retry_wait,
                retry_doubling: s.retry_doubling,
                raw,
            })
        });
        match checked {
            Ok(schedule) => schedules.push(schedule),
            Err(reason) => skipped.push(Skip { what: "schedule", index, name, reason }),
        }
    }
    let mut rules: Vec<Rule> = Vec::new();
    for (index, value) in file.rules.into_iter().enumerate() {
        let name = name_of(&value);
        let raw = value.clone();
        let checked = serde_json::from_value::<RuleText>(value).map_err(|e| e.to_string()).and_then(|r| {
            if !valid_word(&r.name) {
                return Err("the name must be lower-case letters, digits and _, at most 32".into());
            }
            if rules.iter().any(|other| other.name == r.name) {
                return Err("the name is used twice".into());
            }
            for wanted in [&r.when.schedule, &r.then.schedule] {
                if !schedules.iter().any(|s| &s.name == wanted) {
                    return Err(format!("there is no schedule \"{wanted}\""));
                }
            }
            if r.when.label.as_deref().is_some_and(|l| !valid_word(l)) {
                return Err("the result label must be lower-case letters, digits and _, at most 32".into());
            }
            if !(1..=MAX_TIMES).contains(&r.when.times) {
                return Err(format!("times must be 1 to {MAX_TIMES}"));
            }
            if let Some(test) = &r.when.value {
                test.check()?;
            }
            if r.when.changed && !matches!(r.when.ended, Ended::Done | Ended::Any) {
                return Err("changed compares successful runs, so it needs ended done or any".into());
            }
            Ok(Rule {
                name: r.name,
                enabled: r.enabled,
                source: r.when.schedule,
                ended: r.when.ended,
                label: r.when.label,
                times: r.when.times,
                value: r.when.value,
                changed: r.when.changed,
                action: r.then.action,
                target: r.then.schedule,
                raw,
            })
        });
        match checked {
            Ok(rule) => rules.push(rule),
            Err(reason) => skipped.push(Skip { what: "rule", index, name, reason }),
        }
    }
    Ok(Parsed { schedules, rules, skipped })
}

/// What jobs_state.json keeps per schedule.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct RunState {
    pub runs: u64,
    pub last_run_ms: Option<i64>,
    /// State and result label of the last job that finished.
    pub last_state: Option<JobState>,
    pub last_label: Option<String>,
    pub failures_in_a_row: u32,
    pub paused: bool,
    pub paused_reason: Option<String>,
    /// Set by a rule's enable or disable; replaces `enabled` from the file
    /// until the schedule is saved from the Jobs page.
    pub enabled_by_rule: Option<bool>,
    /// Retries used since the last final result, and when the next is due.
    pub retry: u32,
    pub retry_due_ms: Option<i64>,
    /// The newest runs, oldest first.
    pub history: Vec<HistoryEntry>,
    /// Fingerprint of the last successful run's output and value, for
    /// rules with `changed`.
    pub last_fingerprint: Option<String>,
}

/// One finished run of a schedule.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HistoryEntry {
    pub at_ms: i64,
    /// None when the job could not be started.
    pub job: Option<u64>,
    pub state: JobState,
    pub label: Option<String>,
    /// "timetable", "you", "rule NAME" or "retry N of M".
    pub by: String,
    /// The end of what the job printed or answered.
    #[serde(default)]
    pub output: Option<String>,
    /// The result value the job returned, for example `{"percent": 82}`.
    #[serde(default)]
    pub value: Option<Value>,
}

/// A short fingerprint of a run's output and value (64-bit FNV-1a, as hex).
/// Written out by hand rather than with Rust's `DefaultHasher`, whose
/// results may change between Rust versions; this one is kept in
/// jobs_state.json across updates.
fn fingerprint(output: &str, value: Option<&Value>) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let value = value.map(Value::to_string).unwrap_or_default();
    for byte in output.bytes().chain([0u8]).chain(value.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn remember(st: &mut RunState, entry: HistoryEntry) {
    st.history.push(entry);
    let extra = st.history.len().saturating_sub(MAX_HISTORY);
    st.history.drain(..extra);
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct StateText {
    version: u32,
    paused_all: bool,
    schedules: BTreeMap<String, RunState>,
    /// Each rule's count of matching results in a row.
    rules: BTreeMap<String, u32>,
}

fn from_ms(ms: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000).unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

fn to_ms(at: OffsetDateTime) -> i64 {
    (at.unix_timestamp_nanos() / 1_000_000) as i64
}

/// When a schedule should next run, seen from `now`: None when it never will
/// (run limit reached, end date passed, a `once` already done). A run missed
/// while the app was closed is due at once with `catch_up`, else skipped.
pub fn next_due(s: &Schedule, st: &RunState, now: OffsetDateTime, offset_at: OffsetAt) -> Option<OffsetDateTime> {
    if s.max_runs.is_some_and(|max| st.runs >= max) {
        return None;
    }
    let next = match (st.last_run_ms, s.trigger) {
        (Some(last), trigger) => next_after(trigger, from_ms(last), offset_at)?,
        (None, Trigger::Once(at)) => local_instant(at, offset_at),
        (None, trigger) => next_after(trigger, now, offset_at)?,
    };
    let due = if next > now {
        next
    } else if s.catch_up {
        now
    } else {
        next_after(s.trigger, now, offset_at)?
    };
    match s.until {
        Some(until) if due > local_instant(until, offset_at) => None,
        _ => Some(due),
    }
}

/// What the Jobs page sees of a schedule. Times are Unix milliseconds.
#[derive(Serialize, Clone, Debug)]
pub struct ScheduleView {
    pub name: String,
    pub kind: String,
    pub when: String,
    /// Whether the timetable runs it: `enabled_in_file`, unless a rule
    /// changed it.
    pub enabled: bool,
    pub enabled_in_file: bool,
    pub retries: u32,
    pub definition: Value,
    pub catch_up: bool,
    pub max_runs: Option<u64>,
    pub next_ms: Option<i64>,
    /// The job id while one started by this schedule is queued or running.
    pub running_job: Option<u64>,
    #[serde(flatten)]
    pub state: RunState,
}

#[derive(Serialize, Clone, Debug)]
pub struct RuleView {
    pub name: String,
    pub enabled: bool,
    pub text: String,
    /// Matching results in a row so far, and how many fire it.
    pub streak: u32,
    pub times: u32,
    pub definition: Value,
}

/// A run expected in the next 24 hours.
#[derive(Serialize, Clone, Debug)]
pub struct Upcoming {
    pub name: String,
    pub at_ms: i64,
    pub why: &'static str,
}

#[derive(Serialize, Clone, Debug)]
pub struct SchedulesReply {
    pub status: FileStatus,
    pub path: String,
    pub schedules: Vec<ScheduleView>,
    pub rules: Vec<RuleView>,
    pub upcoming: Vec<Upcoming>,
    pub skipped: Vec<String>,
    pub failure_limit: u32,
    pub paused_all: bool,
    pub max_chain: u32,
}

/// A job kind as the edit form needs it.
#[derive(Serialize, Clone, Debug)]
pub struct KindView {
    pub name: &'static str,
    pub example: &'static str,
    pub capabilities: Value,
    pub resources: &'static [&'static str],
}

pub fn kind_views(kinds: &[KindEntry]) -> Vec<KindView> {
    kinds
        .iter()
        .map(|k| KindView {
            name: k.name,
            example: k.example,
            capabilities: serde_json::to_value(k.capabilities).unwrap_or(Value::Null),
            resources: k.resources,
        })
        .collect()
}

/// A run that ended with a final result (not one that will be retried),
/// for the rules.
struct Finished {
    schedule: String,
    job: u64,
    state: JobState,
    outcome: Outcome,
    /// A successful run whose output or value differs from the last
    /// successful run's (false for the first one).
    changed: bool,
    depth: u32,
}

struct Entry {
    schedule: Schedule,
    next: Option<OffsetDateTime>,
    /// The last job this schedule started, until its result is recorded.
    job: Option<u64>,
    /// Who started that job, for the history.
    by: String,
    /// How many rules in a chain led to that job (0: not a rule).
    depth: u32,
    /// The result that job was given, kept so a retry gets it too.
    previous: Option<Arc<Previous>>,
}

struct Inner {
    entries: Vec<Entry>,
    rules: Vec<Rule>,
    states: BTreeMap<String, RunState>,
    streaks: BTreeMap<String, u32>,
    paused_all: bool,
    status: FileStatus,
    skipped: Vec<String>,
    failure_limit: u32,
}

impl Inner {
    fn enabled(&self, entry: &Entry) -> bool {
        self.states.get(&entry.schedule.name).and_then(|st| st.enabled_by_rule).unwrap_or(entry.schedule.enabled)
    }
}

pub type NotifyUi = Arc<dyn Fn(&SchedulesReply) + Send + Sync>;
pub type Clock = Arc<dyn Fn() -> OffsetDateTime + Send + Sync>;

struct Shared {
    inner: Mutex<Inner>,
    wake: tokio::sync::Notify,
    queue: Queue,
    dir: PathBuf,
    notify: NotifyUi,
    clock: Clock,
    offset_at: OffsetAt,
}

/// Cheap to clone; every clone is the same scheduler.
#[derive(Clone)]
pub struct Scheduler(Arc<Shared>);

impl Scheduler {
    /// Loads jobs.json and jobs_state.json from `dir` (creating them when
    /// missing). Call `spawn` to start the timing.
    pub fn new(dir: PathBuf, queue: Queue, failure_limit: u32, notify: NotifyUi, clock: Clock, offset_at: OffsetAt) -> Self {
        let scheduler = Scheduler(Arc::new(Shared {
            inner: Mutex::new(Inner {
                entries: Vec::new(),
                rules: Vec::new(),
                states: BTreeMap::new(),
                streaks: BTreeMap::new(),
                paused_all: false,
                status: FileStatus::Ok,
                skipped: Vec::new(),
                failure_limit: failure_limit.min(MAX_FAILURE_LIMIT),
            }),
            wake: tokio::sync::Notify::new(),
            queue,
            dir,
            notify,
            clock,
            offset_at,
        }));
        scheduler.load_state();
        scheduler.reload();
        scheduler
    }

    /// The app's scheduler: the real clock and the system's time zone.
    pub fn for_app(dir: PathBuf, queue: Queue, failure_limit: u32, notify: NotifyUi) -> Self {
        Self::new(dir, queue, failure_limit, notify, Arc::new(OffsetDateTime::now_utc), system_offset)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn schedules_path(&self) -> PathBuf {
        self.0.dir.join(SCHEDULES_FILE)
    }

    fn state_path(&self) -> PathBuf {
        self.0.dir.join(STATE_FILE)
    }

    fn load_state(&self) {
        let (status, text) = files::read_or_create(&self.state_path(), EMPTY_STATE, MAX_STATE_BYTES, "jobs_state");
        let file = match text.map(|t| serde_json::from_str::<StateText>(&t)) {
            Some(Ok(file)) => file,
            Some(Err(e)) => {
                warn!("jobs_state: not valid ({e}); run counts start again");
                StateText::default()
            }
            None => {
                warn!("jobs_state: could not be read ({status:?}); run counts start again");
                StateText::default()
            }
        };
        if file.paused_all {
            warn!("jobs: everything is paused (from the last session); resume on the Jobs page");
        }
        let mut inner = self.lock();
        inner.states = file.schedules;
        inner.streaks = file.rules;
        inner.paused_all = file.paused_all;
    }

    /// Writes jobs_state.json (atomically), keeping only schedules that
    /// still exist.
    fn save_state(&self, inner: &Inner) {
        let schedules: BTreeMap<String, RunState> = inner
            .entries
            .iter()
            .filter_map(|e| inner.states.get(&e.schedule.name).map(|st| (e.schedule.name.clone(), st.clone())))
            .collect();
        let rules = inner
            .rules
            .iter()
            .filter_map(|r| inner.streaks.get(&r.name).map(|n| (r.name.clone(), *n)))
            .collect();
        let file = StateText { version: 1, paused_all: inner.paused_all, schedules, rules };
        let path = self.state_path();
        let tmp = path.with_extension("json.tmp");
        let result = serde_json::to_string_pretty(&file)
            .map_err(|e| e.to_string())
            .and_then(|json| files::write_new(&tmp, json).map_err(|e| e.to_string()))
            .and_then(|()| fs::rename(&tmp, &path).map_err(|e| e.to_string()));
        if let Err(e) = result {
            warn!("jobs_state: could not be saved: {e}");
        }
    }

    /// Reads jobs.json again. Run counts and running jobs of schedules that
    /// keep their name carry over.
    pub fn reload(&self) {
        let (status, text) =
            files::read_or_create(&self.schedules_path(), DEFAULT_SCHEDULES, MAX_SCHEDULES_BYTES, "jobs");
        let parsed = text.map(|t| parse(&t, &self.0.queue));
        let now = (self.0.clock)();
        let mut inner = self.lock();
        let old_jobs: BTreeMap<String, (u64, String, u32)> = inner
            .entries
            .iter()
            .filter_map(|e| e.job.map(|id| (e.schedule.name.clone(), (id, e.by.clone(), e.depth))))
            .collect();
        inner.status = status;
        inner.skipped.clear();
        match parsed {
            Some(Ok(Parsed { schedules, rules, skipped })) => {
                let skipped: Vec<String> = skipped.iter().map(Skip::text).collect();
                if skipped.is_empty() {
                    info!("jobs: {} schedules and {} rules loaded", schedules.len(), rules.len());
                } else {
                    warn!(
                        "jobs: {} schedules and {} rules loaded, {} skipped: {}",
                        schedules.len(),
                        rules.len(),
                        skipped.len(),
                        skipped.join("; ")
                    );
                }
                inner.skipped = skipped;
                inner.rules = rules;
                let entries = schedules
                    .into_iter()
                    .map(|schedule| {
                        let st = inner.states.get(&schedule.name).cloned().unwrap_or_default();
                        let next = next_due(&schedule, &st, now, self.0.offset_at);
                        let (job, by, depth) = match old_jobs.get(&schedule.name) {
                            Some((id, by, depth)) => (Some(*id), by.clone(), *depth),
                            None => (None, String::new(), 0),
                        };
                        Entry { schedule, next, job, by, depth, previous: None }
                    })
                    .collect();
                inner.entries = entries;
            }
            Some(Err(e)) => {
                warn!("jobs: {e}; no schedules run until it is fixed and reloaded");
                inner.status = FileStatus::Invalid;
                inner.skipped = vec![e];
                inner.entries.clear();
                inner.rules.clear();
            }
            None => {
                inner.entries.clear();
                inner.rules.clear();
            }
        }
        self.publish(&inner);
        drop(inner);
        self.0.wake.notify_one();
    }

    fn reply(&self, inner: &Inner) -> SchedulesReply {
        SchedulesReply {
            status: inner.status,
            path: self.schedules_path().display().to_string(),
            upcoming: self.upcoming(inner),
            rules: inner
                .rules
                .iter()
                .map(|r| RuleView {
                    name: r.name.clone(),
                    enabled: r.enabled,
                    text: r.describe(),
                    streak: inner.streaks.get(&r.name).copied().unwrap_or(0),
                    times: r.times,
                    definition: r.raw.clone(),
                })
                .collect(),
            paused_all: inner.paused_all,
            max_chain: MAX_CHAIN,
            schedules: inner
                .entries
                .iter()
                .map(|e| ScheduleView {
                    name: e.schedule.name.clone(),
                    kind: e.schedule.kind.clone(),
                    when: e.schedule.trigger.describe(),
                    enabled: inner.enabled(e),
                    enabled_in_file: e.schedule.enabled,
                    retries: e.schedule.retries,
                    definition: e.schedule.raw.clone(),
                    catch_up: e.schedule.catch_up,
                    max_runs: e.schedule.max_runs,
                    next_ms: e.next.map(to_ms),
                    running_job: e.job,
                    state: inner.states.get(&e.schedule.name).cloned().unwrap_or_default(),
                })
                .collect(),
            skipped: inner.skipped.clone(),
            failure_limit: inner.failure_limit,
        }
    }

    /// Runs expected in the next 24 hours, soonest first. Rules are not
    /// predicted; they depend on results.
    fn upcoming(&self, inner: &Inner) -> Vec<Upcoming> {
        let mut list = Vec::new();
        if inner.paused_all {
            return list;
        }
        let now = (self.0.clock)();
        let Ok(horizon) = time::Duration::try_from(PREVIEW) else { return list };
        let horizon = now + horizon;
        for entry in &inner.entries {
            let st = inner.states.get(&entry.schedule.name).cloned().unwrap_or_default();
            if !inner.enabled(entry) || st.paused {
                continue;
            }
            let name = entry.schedule.name.clone();
            if let Some(due) = st.retry_due_ms {
                list.push(Upcoming { name: name.clone(), at_ms: due.max(to_ms(now)), why: "retry" });
            }
            let s = &entry.schedule;
            let mut runs = st.runs;
            let mut next = entry.next;
            while let Some(at) = next.filter(|at| *at <= horizon) {
                if s.max_runs.is_some_and(|max| runs >= max) || list.len() >= MAX_PREVIEW * 2 {
                    break;
                }
                list.push(Upcoming { name: name.clone(), at_ms: to_ms(at), why: "timetable" });
                runs += 1;
                next = next_after(s.trigger, at, self.0.offset_at)
                    .filter(|t| s.until.is_none_or(|until| *t <= local_instant(until, self.0.offset_at)));
            }
        }
        list.sort_by_key(|u| u.at_ms);
        list.truncate(MAX_PREVIEW);
        list
    }

    fn publish(&self, inner: &Inner) {
        (self.0.notify)(&self.reply(inner));
    }

    pub fn list(&self) -> SchedulesReply {
        self.reply(&self.lock())
    }

    pub fn set_failure_limit(&self, limit: u32) {
        let mut inner = self.lock();
        let limit = limit.min(MAX_FAILURE_LIMIT);
        if inner.failure_limit != limit {
            info!("jobs: pause a schedule after {limit} failures in a row (0 = never)");
            inner.failure_limit = limit;
            self.publish(&inner);
        }
    }

    /// Pauses or resumes a schedule. Resuming clears its failure count and
    /// does not make up runs missed while paused.
    pub fn set_paused(&self, name: &str, paused: bool) -> Result<(), String> {
        let now = (self.0.clock)();
        let mut inner = self.lock();
        let index = inner.entries.iter().position(|e| e.schedule.name == name).ok_or("no such schedule")?;
        let offset_at = self.0.offset_at;
        let st = inner.states.entry(name.to_string()).or_default();
        st.paused = paused;
        if !paused {
            st.failures_in_a_row = 0;
            st.paused_reason = None;
        }
        let st = st.clone();
        let entry = &mut inner.entries[index];
        if !paused {
            let no_catch_up = Schedule { catch_up: false, ..entry.schedule.clone() };
            entry.next = next_due(&no_catch_up, &st, now, offset_at);
        }
        info!("schedule {name}: {}", if paused { "paused" } else { "resumed" });
        self.save_state(&inner);
        self.publish(&inner);
        drop(inner);
        self.0.wake.notify_one();
        Ok(())
    }

    /// Starts a schedule's job now, whatever its timetable, enabled or
    /// paused state. Its result counts like any other run's.
    pub fn run_now(&self, name: &str) -> Result<u64, String> {
        let mut inner = self.lock();
        let entry = inner.entries.iter_mut().find(|e| e.schedule.name == name).ok_or("no such schedule")?;
        if entry.job.is_some_and(|id| self.active(id)) {
            return Err("its last job is still queued or running".into());
        }
        let s = &entry.schedule;
        let id = self.0.queue.start(&s.kind, s.input.clone(), Origin::Schedule { name: s.name.clone() }, s.timeout_ms)?;
        entry.job = Some(id);
        entry.by = "you".into();
        entry.depth = 0;
        entry.previous = None;
        info!("schedule {name}: run now, job {id}");
        self.publish(&inner);
        drop(inner);
        self.0.wake.notify_one();
        Ok(id)
    }

    fn active(&self, id: u64) -> bool {
        self.0.queue.result(id).is_some_and(|(state, _)| !state.finished())
    }

    /// Starts a schedule's job. On failure to start, records it as a
    /// failed run. Returns whether a job was queued.
    #[allow(clippy::too_many_arguments)]
    fn launch(
        &self,
        entry: &mut Entry,
        st: &mut RunState,
        origin: Origin,
        by: String,
        depth: u32,
        previous: Option<Arc<Previous>>,
        now: OffsetDateTime,
    ) -> bool {
        let s = &entry.schedule;
        match self.0.queue.start_after(&s.kind, s.input.clone(), origin, s.timeout_ms, previous.clone()) {
            Ok(id) => {
                info!("schedule {}: started job {id} ({by})", s.name);
                entry.job = Some(id);
                entry.by = by;
                entry.depth = depth;
                entry.previous = previous;
                true
            }
            Err(e) => {
                warn!("schedule {}: could not start ({by}): {e}", s.name);
                st.last_state = Some(JobState::Failed);
                st.last_label = None;
                st.failures_in_a_row += 1;
                remember(st, HistoryEntry { at_ms: to_ms(now), job: None, state: JobState::Failed, label: None, by, output: None, value: None });
                false
            }
        }
    }

    /// One pass: records finished jobs, applies rules to them, starts what
    /// is due, and returns how long to sleep before the next pass.
    pub fn tick(&self) -> Duration {
        let now = (self.0.clock)();
        let offset_at = self.0.offset_at;
        let mut inner = self.lock();
        let limit = inner.failure_limit;
        let paused_all = inner.paused_all;
        let mut changed = false;
        let mut finished: Vec<Finished> = Vec::new();
        let Inner { entries, states, .. } = &mut *inner;

        for entry in entries.iter_mut() {
            let name = entry.schedule.name.clone();
            let Some(id) = entry.job else { continue };
            match self.0.queue.outcome(id) {
                Some((state, _)) if !state.finished() => continue,
                Some((state, outcome)) => {
                    let label = outcome.label.clone();
                    let st = states.entry(name.clone()).or_default();
                    let by = std::mem::take(&mut entry.by);
                    let output = self.0.queue.output_tail(id, HISTORY_OUTPUT_BYTES);
                    let value = outcome.value.clone();
                    remember(st, HistoryEntry { at_ms: to_ms(now), job: Some(id), state, label: label.clone(), by, output, value });
                    st.last_state = Some(state);
                    st.last_label = label.clone();
                    let failed = matches!(state, JobState::Failed | JobState::TimedOut);
                    if failed && st.retry < entry.schedule.retries {
                        st.retry += 1;
                        let wait = entry.schedule.retry_after(st.retry);
                        let due = now + time::Duration::try_from(wait).unwrap_or(time::Duration::ZERO);
                        st.retry_due_ms = Some(to_ms(due));
                        info!("schedule {name}: retry {} of {} in {}s", st.retry, entry.schedule.retries, wait.as_secs());
                    } else {
                        st.retry = 0;
                        st.retry_due_ms = None;
                        if state == JobState::Done {
                            st.failures_in_a_row = 0;
                        } else if failed {
                            st.failures_in_a_row += 1;
                            if limit > 0 && st.failures_in_a_row >= limit && !st.paused {
                                st.paused = true;
                                let reason = format!("paused after {} failures in a row", st.failures_in_a_row);
                                warn!("schedule {name}: {reason}");
                                st.paused_reason = Some(reason);
                            }
                        }
                        let mut changed = false;
                        if state == JobState::Done {
                            let output = self.0.queue.output_tail(id, MAX_OUTPUT_BYTES).unwrap_or_default();
                            let print = fingerprint(&output, outcome.value.as_ref());
                            changed = st.last_fingerprint.as_ref().is_some_and(|last| *last != print);
                            st.last_fingerprint = Some(print);
                        }
                        finished.push(Finished { schedule: name, job: id, state, outcome, changed, depth: entry.depth });
                    }
                }
                None => {} // cleared from the Jobs page before it was seen
            }
            entry.job = None;
            changed = true;
        }

        for entry in entries.iter_mut() {
            let st = states.entry(entry.schedule.name.clone()).or_default();
            let enabled = st.enabled_by_rule.unwrap_or(entry.schedule.enabled);
            if !enabled || st.paused || paused_all {
                continue;
            }
            let busy = entry.job.is_some_and(|id| self.active(id));
            if entry.next.is_some_and(|due| due <= now) {
                if busy {
                    info!("schedule {}: due while its last job is still running; this run skipped", entry.schedule.name);
                } else {
                    st.retry = 0;
                    st.retry_due_ms = None;
                    let origin = Origin::Schedule { name: entry.schedule.name.clone() };
                    self.launch(entry, st, origin, "timetable".into(), 0, None, now);
                    // Skipped runs do not count towards max_runs.
                    st.runs += 1;
                }
                st.last_run_ms = Some(to_ms(now));
                entry.next = next_due(&entry.schedule, st, now, offset_at);
                changed = true;
            } else if !busy && st.retry_due_ms.is_some_and(|due| from_ms(due) <= now) {
                st.retry_due_ms = None;
                let by = format!("retry {} of {}", st.retry, entry.schedule.retries);
                let origin = Origin::Schedule { name: entry.schedule.name.clone() };
                let (depth, previous) = (entry.depth, entry.previous.clone());
                self.launch(entry, st, origin, by, depth, previous, now);
                changed = true;
            }
        }

        for done in finished {
            changed |= self.apply_rules(&mut inner, &done, now);
        }

        if changed {
            self.save_state(&inner);
            self.publish(&inner);
        }
        let mut sleep = MAX_SLEEP;
        for entry in &inner.entries {
            if entry.job.is_some() {
                sleep = sleep.min(RESULT_POLL);
            }
            // A disabled or paused schedule keeps its old due time; it must
            // not make the loop spin.
            let st = inner.states.get(&entry.schedule.name);
            if !inner.enabled(entry) || st.is_some_and(|st| st.paused) || inner.paused_all {
                continue;
            }
            let retry = st.and_then(|st| st.retry_due_ms).map(from_ms);
            for due in [entry.next, retry].into_iter().flatten() {
                let wait = (due - now).try_into().unwrap_or(Duration::ZERO);
                sleep = sleep.min(wait);
            }
        }
        sleep
    }

    /// Counts a final result against every rule watching its schedule, and
    /// carries out those that reach their count. Returns whether anything
    /// changed.
    fn apply_rules(&self, inner: &mut Inner, done: &Finished, now: OffsetDateTime) -> bool {
        let mut fire = Vec::new();
        let Inner { rules, streaks, .. } = &mut *inner;
        for rule in rules.iter().filter(|r| r.enabled && r.source == done.schedule) {
            let streak = streaks.entry(rule.name.clone()).or_default();
            if !rule.matches(done) {
                *streak = 0;
                continue;
            }
            *streak += 1;
            if *streak >= rule.times {
                *streak = 0;
                fire.push(rule.clone());
            }
        }
        let changed = !fire.is_empty() || inner.rules.iter().any(|r| r.source == done.schedule);
        if fire.is_empty() {
            return changed;
        }
        // Read once, shared by every rule that fires on this result.
        let previous = Arc::new(Previous {
            schedule: done.schedule.clone(),
            job: done.job,
            state: done.state,
            label: done.outcome.label.clone(),
            value: done.outcome.value.clone(),
            message: done.outcome.message.clone(),
            output: self.0.queue.output_tail(done.job, MAX_OUTPUT_BYTES).unwrap_or_default(),
        });
        for rule in fire {
            self.apply(inner, &rule, done.depth, &previous, now);
        }
        changed
    }

    fn apply(&self, inner: &mut Inner, rule: &Rule, depth: u32, previous: &Arc<Previous>, now: OffsetDateTime) {
        let offset_at = self.0.offset_at;
        let paused_all = inner.paused_all;
        let Inner { entries, states, .. } = &mut *inner;
        let Some(entry) = entries.iter_mut().find(|e| e.schedule.name == rule.target) else { return };
        let st = states.entry(rule.target.clone()).or_default();
        let target = &rule.target;
        match rule.action {
            Action::Enable | Action::Disable => {
                let enable = rule.action == Action::Enable;
                st.enabled_by_rule = Some(enable);
                if enable {
                    let no_catch_up = Schedule { catch_up: false, ..entry.schedule.clone() };
                    entry.next = next_due(&no_catch_up, st, now, offset_at);
                }
                info!("rule {}: {} schedule {target}", rule.name, if enable { "enabled" } else { "disabled" });
            }
            Action::Start => {
                let refused = if depth >= MAX_CHAIN {
                    Some(format!("a chain of {MAX_CHAIN} rules was reached (loop guard)"))
                } else if paused_all {
                    Some("everything is paused".to_string())
                } else if st.paused {
                    Some(format!("schedule {target} is paused"))
                } else if entry.schedule.max_runs.is_some_and(|max| st.runs >= max) {
                    Some(format!("schedule {target} has used all its runs"))
                } else if entry.job.is_some_and(|id| self.active(id)) {
                    Some(format!("schedule {target}'s last job is still running"))
                } else {
                    None
                };
                match refused {
                    Some(reason) if depth >= MAX_CHAIN => warn!("rule {}: did not start {target}: {reason}", rule.name),
                    Some(reason) => info!("rule {}: did not start {target}: {reason}", rule.name),
                    None => {
                        let origin = Origin::Rule { rule: rule.name.clone(), schedule: target.clone() };
                        let by = format!("rule {}", rule.name);
                        if self.launch(entry, st, origin, by, depth + 1, Some(previous.clone()), now) {
                            st.runs += 1;
                        }
                    }
                }
            }
        }
    }

    /// Stops or allows every start by timetable and rule. Kept across
    /// restarts. Run now still works while paused.
    pub fn set_paused_all(&self, paused: bool) {
        let mut inner = self.lock();
        inner.paused_all = paused;
        if paused {
            warn!("jobs: everything paused; no schedule or rule starts a job until resumed");
        } else {
            // Runs missed while paused are not made up.
            let now = (self.0.clock)();
            let offset_at = self.0.offset_at;
            let Inner { entries, states, .. } = &mut *inner;
            for entry in entries.iter_mut() {
                let st = states.get(&entry.schedule.name).cloned().unwrap_or_default();
                let no_catch_up = Schedule { catch_up: false, ..entry.schedule.clone() };
                entry.next = next_due(&no_catch_up, &st, now, offset_at);
            }
            info!("jobs: resumed");
        }
        self.save_state(&inner);
        self.publish(&inner);
        drop(inner);
        self.0.wake.notify_one();
    }

    /// Changes jobs.json: reads it, lets `change` edit it, checks the result
    /// and writes it atomically. `entry` (kind and name) is the schedule or
    /// rule being saved: if the check refuses it, nothing is written. A file
    /// that is not valid JSON is never rewritten (a hand edit would be lost).
    fn edit_file(
        &self,
        entry: Option<(&str, &str)>,
        change: impl FnOnce(&mut Map<String, Value>) -> Result<(), String>,
    ) -> Result<(), String> {
        let path = self.schedules_path();
        let text = fs::read_to_string(&path).map_err(|e| format!("jobs.json could not be read: {e}"))?;
        let mut file: Value = serde_json::from_str(&text)
            .map_err(|e| format!("jobs.json is not valid JSON ({e}); fix it by hand and press Reload first"))?;
        change(file.as_object_mut().ok_or("jobs.json is not a JSON object")?)?;
        let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())? + "\n";
        if json.len() as u64 > MAX_SCHEDULES_BYTES {
            return Err(format!("jobs.json would be over {MAX_SCHEDULES_BYTES} bytes"));
        }
        let parsed = parse(&json, &self.0.queue)?;
        if let Some(reason) = entry.and_then(|(what, name)| Self::refusal(&parsed, what, name)) {
            return Err(reason);
        }
        let tmp = path.with_extension("json.tmp");
        files::write_new(&tmp, json).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }

    fn list_mut<'a>(file: &'a mut Map<String, Value>, key: &str) -> Result<&'a mut Vec<Value>, String> {
        file.entry(key).or_insert_with(|| Value::Array(Vec::new())).as_array_mut().ok_or(format!("\"{key}\" is not a list"))
    }

    fn name_in(value: &Value) -> Option<&str> {
        value.get("name").and_then(Value::as_str)
    }

    /// The Skip for the entry being saved, if the check refused it.
    fn refusal(parsed: &Parsed, what: &str, name: &str) -> Option<String> {
        parsed.skipped.iter().find(|s| s.what == what && s.name.as_deref() == Some(name)).map(|s| s.reason.clone())
    }

    /// Adds a schedule, or replaces the one called `old_name` (renaming it
    /// in rules too). Refused, with nothing written, if it is not valid.
    pub fn save_schedule(&self, old_name: Option<String>, definition: Value) -> Result<(), String> {
        let name = Self::name_in(&definition).ok_or("the schedule needs a name")?.to_string();
        self.edit_file(Some(("schedule", &name)), |file| {
            let list = Self::list_mut(file, "schedules")?;
            let at = old_name.as_deref().and_then(|old| list.iter().position(|v| Self::name_in(v) == Some(old)));
            if list.iter().enumerate().any(|(i, v)| Some(i) != at && Self::name_in(v) == Some(&name)) {
                return Err(format!("a schedule called \"{name}\" already exists"));
            }
            match at {
                Some(i) => list[i] = definition,
                None => list.push(definition),
            }
            if let Some(old) = old_name.as_deref().filter(|old| *old != name) {
                for rule in Self::list_mut(file, "rules")? {
                    for part in ["when", "then"] {
                        if let Some(slot) = rule.get_mut(part).and_then(|p| p.get_mut("schedule")) {
                            if slot.as_str() == Some(old) {
                                *slot = Value::String(name.clone());
                            }
                        }
                    }
                }
            }
            Ok(())
        })?;
        {
            let mut inner = self.lock();
            if let Some(old) = old_name.as_deref().filter(|old| *old != name) {
                if let Some(st) = inner.states.remove(old) {
                    inner.states.insert(name.clone(), st);
                }
            }
            if let Some(st) = inner.states.get_mut(&name) {
                st.enabled_by_rule = None;
            }
        }
        info!("schedule {name}: saved from the Jobs page");
        self.reload();
        self.save_state(&self.lock());
        Ok(())
    }

    pub fn delete_schedule(&self, name: &str) -> Result<(), String> {
        self.edit_file(None, |file| {
            let used: Vec<String> = Self::list_mut(file, "rules")?
                .iter()
                .filter(|r| {
                    ["when", "then"].iter().any(|p| r.get(*p).and_then(|x| x.get("schedule")).and_then(Value::as_str) == Some(name))
                })
                .filter_map(|r| Self::name_in(r).map(str::to_string))
                .collect();
            if !used.is_empty() {
                return Err(format!("rule {} uses it; change or delete the rule first", used.join(", ")));
            }
            let list = Self::list_mut(file, "schedules")?;
            let before = list.len();
            list.retain(|v| Self::name_in(v) != Some(name));
            if list.len() == before {
                return Err("no such schedule".into());
            }
            Ok(())
        })?;
        info!("schedule {name}: deleted from the Jobs page");
        self.reload();
        self.save_state(&self.lock());
        Ok(())
    }

    /// Turns a schedule's timetable on or off in jobs.json (and drops any
    /// change a rule made).
    pub fn set_enabled(&self, name: &str, enabled: bool) -> Result<(), String> {
        self.edit_file(Some(("schedule", name)), |file| {
            let list = Self::list_mut(file, "schedules")?;
            let entry = list.iter_mut().find(|v| Self::name_in(v) == Some(name)).ok_or("no such schedule")?;
            entry.as_object_mut().ok_or("not an object")?.insert("enabled".into(), Value::Bool(enabled));
            Ok(())
        })?;
        if let Some(st) = self.lock().states.get_mut(name) {
            st.enabled_by_rule = None;
        }
        info!("schedule {name}: {} from the Jobs page", if enabled { "enabled" } else { "disabled" });
        self.reload();
        self.save_state(&self.lock());
        Ok(())
    }

    /// Adds a rule, or replaces the one called `old_name`.
    pub fn save_rule(&self, old_name: Option<String>, definition: Value) -> Result<(), String> {
        let name = Self::name_in(&definition).ok_or("the rule needs a name")?.to_string();
        self.edit_file(Some(("rule", &name)), |file| {
            let list = Self::list_mut(file, "rules")?;
            let at = old_name.as_deref().and_then(|old| list.iter().position(|v| Self::name_in(v) == Some(old)));
            if list.iter().enumerate().any(|(i, v)| Some(i) != at && Self::name_in(v) == Some(&name)) {
                return Err(format!("a rule called \"{name}\" already exists"));
            }
            match at {
                Some(i) => list[i] = definition,
                None => list.push(definition),
            }
            Ok(())
        })?;
        if let Some(old) = old_name.as_deref().filter(|old| *old != name) {
            let mut inner = self.lock();
            if let Some(n) = inner.streaks.remove(old) {
                inner.streaks.insert(name.clone(), n);
            }
        }
        info!("rule {name}: saved from the Jobs page");
        self.reload();
        self.save_state(&self.lock());
        Ok(())
    }

    pub fn delete_rule(&self, name: &str) -> Result<(), String> {
        self.edit_file(None, |file| {
            let list = Self::list_mut(file, "rules")?;
            let before = list.len();
            list.retain(|v| Self::name_in(v) != Some(name));
            if list.len() == before {
                return Err("no such rule".into());
            }
            Ok(())
        })?;
        self.lock().streaks.remove(name);
        info!("rule {name}: deleted from the Jobs page");
        self.reload();
        self.save_state(&self.lock());
        Ok(())
    }

    /// Runs `tick` for as long as the app runs, on Tauri's Tokio runtime.
    pub fn spawn(&self, runtime: &Handle) {
        let scheduler = self.clone();
        runtime.spawn(async move {
            loop {
                let sleep = scheduler.tick();
                tokio::select! {
                    _ = tokio::time::sleep(sleep) => {}
                    _ = scheduler.0.wake.notified() => {}
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job_kinds::TEST_KINDS as KINDS;
    use serde_json::json;
    use time::macros::{date, datetime, offset, time};

    fn utc(_: OffsetDateTime) -> UtcOffset {
        UtcOffset::UTC
    }

    /// UK-like: GMT, then BST (UTC+1) from 2027-03-28 01:00 UTC.
    fn uk(at: OffsetDateTime) -> UtcOffset {
        if at >= datetime!(2027-03-28 01:00 UTC) {
            offset!(+1)
        } else {
            UtcOffset::UTC
        }
    }

    fn schedule(trigger: Trigger) -> Schedule {
        Schedule {
            name: "t".into(),
            kind: "count".into(),
            input: json!({ "count": 1, "delay_ms": 100 }),
            trigger,
            enabled: true,
            max_runs: None,
            until: None,
            catch_up: true,
            timeout_ms: None,
            retries: 0,
            retry_wait: Duration::from_secs(60),
            retry_doubling: true,
            raw: Value::Null,
        }
    }

    #[test]
    fn when_is_parsed_and_bad_values_refused() {
        let parse = |v: Value| serde_json::from_value::<When>(v).map_err(|e| e.to_string()).and_then(|w| w.parse());
        assert_eq!(parse(json!({"type":"every","interval":"10m"})), Ok(Trigger::Every(Duration::from_secs(600))));
        assert_eq!(parse(json!({"type":"daily","time":"08:05"})), Ok(Trigger::Daily(time!(08:05))));
        assert_eq!(parse(json!({"type":"weekly","day":"Sunday","time":"03:00"})), Ok(Trigger::Weekly(Weekday::Sunday, time!(03:00))));
        assert_eq!(parse(json!({"type":"once","at":"2026-12-01 09:00"})), Ok(Trigger::Once(datetime!(2026-12-01 09:00))));
        for bad in [
            json!({"type":"every","interval":"4s"}),
            json!({"type":"every","interval":"367d"}),
            json!({"type":"every","interval":"10 parsecs"}),
            json!({"type":"every","interval":"99999999999999999999d"}),
            json!({"type":"daily","time":"25:00"}),
            json!({"type":"daily","time":"8am"}),
            json!({"type":"weekly","day":"funday","time":"03:00"}),
            json!({"type":"monthly","day":0,"time":"03:00"}),
            json!({"type":"monthly","day":32,"time":"03:00"}),
            json!({"type":"once","at":"tomorrow"}),
            json!({"type":"hourly"}),
            json!({"type":"daily","time":"08:00","extra":1}),
        ] {
            assert!(parse(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn next_runs_follow_the_calendar() {
        let at = datetime!(2027-01-30 10:00 UTC);
        let next = |t| next_after(t, at, utc);
        assert_eq!(next(Trigger::Daily(time!(11:00))), Some(datetime!(2027-01-30 11:00 UTC)), "later today");
        assert_eq!(next(Trigger::Daily(time!(10:00))), Some(datetime!(2027-01-31 10:00 UTC)), "strictly after");
        assert_eq!(next(Trigger::Weekly(Weekday::Monday, time!(09:00))), Some(datetime!(2027-02-01 09:00 UTC)));
        assert_eq!(next(Trigger::Weekly(Weekday::Saturday, time!(09:00))), Some(datetime!(2027-02-06 09:00 UTC)), "2027-01-30 is a Saturday, 09:00 has passed");
        assert_eq!(next(Trigger::Monthly(31, time!(23:00))), Some(datetime!(2027-01-31 23:00 UTC)));
        let feb = datetime!(2027-02-01 00:00 UTC);
        assert_eq!(next_after(Trigger::Monthly(31, time!(23:00)), feb, utc), Some(datetime!(2027-02-28 23:00 UTC)), "last day of a short month");
        let leap = datetime!(2028-02-01 00:00 UTC);
        assert_eq!(next_after(Trigger::Monthly(30, time!(00:00)), leap, utc), Some(datetime!(2028-02-29 00:00 UTC)), "leap year");
        let dec = datetime!(2027-12-31 23:30 UTC);
        assert_eq!(next_after(Trigger::Monthly(1, time!(00:00)), dec, utc), Some(datetime!(2028-01-01 00:00 UTC)), "into the next year");
        assert_eq!(next(Trigger::Every(Duration::from_secs(90))), Some(datetime!(2027-01-30 10:01:30 UTC)));
        assert_eq!(next(Trigger::Once(datetime!(2027-01-30 09:00))), None, "already past");
        assert_eq!(next(Trigger::Once(datetime!(2027-02-01 09:00))), Some(datetime!(2027-02-01 09:00 UTC)));
    }

    #[test]
    fn local_times_survive_a_clock_change() {
        // 08:00 local is 08:00 UTC before the change and 07:00 UTC after it.
        let before = datetime!(2027-03-27 09:00 UTC);
        let daily = Trigger::Daily(time!(08:00));
        let first = next_after(daily, before, uk).unwrap();
        assert_eq!(first, datetime!(2027-03-28 07:00 UTC), "08:00 BST on the day of the change");
        assert_eq!(next_after(daily, first, uk), Some(datetime!(2027-03-29 07:00 UTC)));
        // 01:30 does not exist that night (01:00 GMT jumps to 02:00 BST): it runs an hour late.
        let gap = next_after(Trigger::Daily(time!(01:30)), before, uk).unwrap();
        assert_eq!(gap, datetime!(2027-03-28 01:30 UTC), "02:30 BST, once");
    }

    #[test]
    fn missed_runs_limits_and_end_dates() {
        let now = datetime!(2027-01-30 12:00 UTC);
        let daily = schedule(Trigger::Daily(time!(08:00)));
        let ran = |at: OffsetDateTime| RunState { runs: 1, last_run_ms: Some(to_ms(at)), ..RunState::default() };
        // Last ran two days ago: today's 08:00 was missed.
        let st = ran(datetime!(2027-01-28 08:00 UTC));
        assert_eq!(next_due(&daily, &st, now, utc), Some(now), "caught up at once");
        let skip = Schedule { catch_up: false, ..daily.clone() };
        assert_eq!(next_due(&skip, &st, now, utc), Some(datetime!(2027-01-31 08:00 UTC)), "skipped");
        // Never run: the next time from now, not at once.
        assert_eq!(next_due(&daily, &RunState::default(), now, utc), Some(datetime!(2027-01-31 08:00 UTC)));
        let every = schedule(Trigger::Every(Duration::from_secs(60)));
        assert_eq!(next_due(&every, &RunState::default(), now, utc), Some(datetime!(2027-01-30 12:01 UTC)));
        // A once that was due while closed runs at start, once.
        let once = schedule(Trigger::Once(datetime!(2027-01-30 09:00)));
        assert_eq!(next_due(&once, &RunState::default(), now, utc), Some(now));
        assert_eq!(next_due(&once, &ran(now), now, utc), None, "done");
        // Limits.
        let limited = Schedule { max_runs: Some(1), ..daily.clone() };
        assert_eq!(next_due(&limited, &st, now, utc), None);
        let ended = Schedule { until: Some(datetime!(2027-01-30 23:00)), ..daily };
        assert_eq!(next_due(&ended, &ran(now), now, utc), None, "tomorrow is after the end date");
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rust_scaffold_sched_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    struct Rig {
        scheduler: Scheduler,
        queue: Queue,
        clock: Arc<Mutex<OffsetDateTime>>,
        dir: PathBuf,
    }

    impl Rig {
        fn new(name: &str, jobs_json: Value, limit: u32) -> Rig {
            let dir = temp_dir(name);
            fs::write(dir.join(SCHEDULES_FILE), jobs_json.to_string()).unwrap();
            Self::open(dir, limit)
        }

        fn open(dir: PathBuf, limit: u32) -> Rig {
            let queue = Queue::new(Handle::current(), 3, Arc::new(|_| {}), KINDS);
            let clock = Arc::new(Mutex::new(datetime!(2027-01-30 12:00 UTC)));
            let read = clock.clone();
            let scheduler =
                Scheduler::new(dir.clone(), queue.clone(), limit, Arc::new(|_| {}), Arc::new(move || *read.lock().unwrap()), utc);
            Rig { scheduler, queue, clock, dir }
        }

        /// Moves both clocks on: the wall clock the scheduler reads, and
        /// Tokio's paused clock the jobs run on.
        async fn advance(&self, seconds: u64) {
            *self.clock.lock().unwrap() += time::Duration::seconds(seconds as i64);
            tokio::time::sleep(Duration::from_secs(seconds)).await;
            self.scheduler.tick();
        }

        fn view(&self, name: &str) -> ScheduleView {
            self.scheduler.list().schedules.into_iter().find(|s| s.name == name).unwrap()
        }
    }

    fn every_5s(name: &str, input: Value) -> Value {
        json!({ "name": name, "kind": "count", "input": input, "when": { "type": "every", "interval": "5s" } })
    }

    #[tokio::test(start_paused = true)]
    async fn the_default_file_is_valid_and_its_demo_is_off() {
        let queue = Queue::new(Handle::current(), 1, Arc::new(|_| {}), KINDS);
        let Parsed { schedules, skipped, .. } = parse(DEFAULT_SCHEDULES, &queue).unwrap();
        assert!(skipped.is_empty(), "{skipped:?}");
        assert!(!schedules.is_empty());
        assert!(schedules.iter().all(|s| !s.enabled), "a new app runs nothing on its own");
    }

    #[tokio::test(start_paused = true)]
    async fn bad_schedules_are_skipped_with_a_reason() {
        let rig = Rig::new(
            "bad",
            json!({ "version": 1, "schedules": [
                every_5s("good", json!({ "count": 1, "delay_ms": 100 })),
                every_5s("good", json!({ "count": 1, "delay_ms": 100 })),
                every_5s("Bad Name", json!({ "count": 1, "delay_ms": 100 })),
                every_5s("bad_input", json!({ "count": 0, "delay_ms": 100 })),
                { "name": "bad_kind", "kind": "format_disk", "when": { "type": "every", "interval": "5s" } },
                { "name": "typo", "kind": "count", "input": {}, "when": { "type": "every", "interval": "5s" }, "enabeld": false }
            ]}),
            3,
        );
        let reply = rig.scheduler.list();
        assert_eq!(reply.schedules.len(), 1);
        assert_eq!(reply.skipped.len(), 5, "{:?}", reply.skipped);
        assert!(reply.skipped[0].contains("used twice"));
        assert!(reply.skipped[3].contains("unknown job kind"));
        assert!(reply.skipped[4].contains("enabeld"));
    }

    #[tokio::test(start_paused = true)]
    async fn schedules_run_on_time_and_counts_survive_a_restart() {
        let rig = Rig::new(
            "runs",
            json!({ "version": 1, "schedules": [ every_5s("tick", json!({ "count": 2, "delay_ms": 100 })) ] }),
            3,
        );
        rig.scheduler.tick();
        assert_eq!(rig.view("tick").state.runs, 0, "not before its time");
        rig.advance(5).await;
        let view = rig.view("tick");
        assert_eq!(view.state.runs, 1);
        let job = view.running_job.unwrap();
        let started = rig.queue.inspect(job).unwrap();
        let text = serde_json::to_value(&started).unwrap();
        assert_eq!(text["job"]["origin"], json!({ "by": "schedule", "name": "tick" }));
        rig.advance(1).await;
        let view = rig.view("tick");
        assert_eq!(view.state.last_state, Some(JobState::Done));
        assert_eq!(view.state.last_label.as_deref(), Some("counted"));
        rig.advance(4).await;
        rig.advance(5).await;
        assert_eq!(rig.view("tick").state.runs, 3);
        // A new scheduler on the same folder keeps the count.
        let again = Rig::open(rig.dir.clone(), 3);
        assert_eq!(again.view("tick").state.runs, 3);
    }

    #[tokio::test(start_paused = true)]
    async fn a_run_is_skipped_while_the_last_is_still_going() {
        let rig = Rig::new(
            "overlap",
            json!({ "version": 1, "schedules": [ every_5s("slow", json!({ "count": 20, "delay_ms": 1000 })) ] }),
            3,
        );
        rig.advance(5).await;
        let first = rig.view("slow").running_job.unwrap();
        rig.advance(5).await;
        rig.advance(5).await;
        assert_eq!(rig.view("slow").running_job, Some(first), "no second job was started");
        assert_eq!(rig.queue.list().running, 1);
        assert_eq!(rig.view("slow").state.runs, 1, "skipped runs are not counted");
    }

    #[tokio::test(start_paused = true)]
    async fn repeated_failures_pause_a_schedule_and_resume_clears_them() {
        let failing = json!({ "count": 2, "delay_ms": 100, "fail_at": 1 });
        let rig = Rig::new("breaker", json!({ "version": 1, "schedules": [ every_5s("broken", failing) ] }), 3);
        for _ in 0..4 {
            rig.advance(5).await;
            rig.advance(1).await;
        }
        let view = rig.view("broken");
        assert!(view.state.paused, "{view:?}");
        assert_eq!(view.state.failures_in_a_row, 3);
        assert!(view.state.paused_reason.unwrap().contains("3 failures"));
        let runs = view.state.runs;
        rig.advance(30).await;
        assert_eq!(rig.view("broken").state.runs, runs, "paused: nothing more runs");
        rig.scheduler.set_paused("broken", false).unwrap();
        let view = rig.view("broken");
        assert!(!view.state.paused);
        assert_eq!(view.state.failures_in_a_row, 0);
        rig.advance(5).await;
        assert_eq!(rig.view("broken").state.runs, runs + 1, "runs again after resuming");
    }

    #[tokio::test(start_paused = true)]
    async fn a_paused_schedule_does_not_make_the_loop_spin() {
        let rig = Rig::new(
            "spin",
            json!({ "version": 1, "schedules": [ every_5s("p", json!({ "count": 1, "delay_ms": 100 })) ] }),
            3,
        );
        rig.scheduler.set_paused("p", true).unwrap();
        rig.advance(60).await;
        assert!(rig.scheduler.tick() >= Duration::from_secs(59), "sleeps instead of spinning");
    }

    #[tokio::test(start_paused = true)]
    async fn a_limit_of_zero_never_pauses() {
        let failing = json!({ "count": 2, "delay_ms": 100, "fail_at": 1 });
        let rig = Rig::new("no_breaker", json!({ "version": 1, "schedules": [ every_5s("broken", failing) ] }), 0);
        for _ in 0..5 {
            rig.advance(5).await;
            rig.advance(1).await;
        }
        let view = rig.view("broken");
        assert!(!view.state.paused);
        assert_eq!(view.state.failures_in_a_row, 5);
    }

    #[tokio::test(start_paused = true)]
    async fn run_now_ignores_the_timetable_and_disabled_schedules_wait() {
        let mut off = every_5s("off", json!({ "count": 1, "delay_ms": 100 }));
        off["enabled"] = json!(false);
        let rig = Rig::new("now", json!({ "version": 1, "schedules": [ off ] }), 3);
        rig.advance(30).await;
        assert_eq!(rig.view("off").state.runs, 0, "disabled in the file");
        let id = rig.scheduler.run_now("off").unwrap();
        assert!(rig.scheduler.run_now("off").is_err(), "not twice at once");
        rig.advance(1).await;
        assert_eq!(rig.queue.result(id).unwrap().0, JobState::Done);
        assert_eq!(rig.view("off").state.last_label.as_deref(), Some("counted"));
        assert!(rig.scheduler.run_now("nope").is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn missing_files_are_created_and_a_bad_file_runs_nothing() {
        let dir = temp_dir("files");
        let rig = Rig::open(dir.clone(), 3);
        assert!(dir.join(SCHEDULES_FILE).exists() && dir.join(STATE_FILE).exists());
        assert_eq!(rig.scheduler.list().status, FileStatus::Created);
        fs::write(dir.join(SCHEDULES_FILE), "{ not json").unwrap();
        rig.scheduler.reload();
        let reply = rig.scheduler.list();
        assert_eq!(reply.status, FileStatus::Invalid);
        assert!(reply.schedules.is_empty());
        assert_eq!(fs::read_to_string(dir.join(SCHEDULES_FILE)).unwrap(), "{ not json", "never overwritten");
    }

    #[test]
    fn descriptions_read_well() {
        assert_eq!(Trigger::Every(Duration::from_secs(600)).describe(), "every 10 minutes");
        assert_eq!(Trigger::Every(Duration::from_secs(3600)).describe(), "every 1 hour");
        assert_eq!(Trigger::Every(Duration::from_secs(14 * 86_400)).describe(), "every 14 days");
        assert_eq!(Trigger::Every(Duration::from_secs(90)).describe(), "every 90 seconds");
        assert_eq!(Trigger::Weekly(Weekday::Sunday, time!(03:00)).describe(), "every Sunday at 03:00");
        assert_eq!(Trigger::Monthly(31, time!(23:00)).describe(), "monthly on day 31 at 23:00 (or the month's last day)");
        assert_eq!(Trigger::Once(datetime!(2026-12-01 09:00)).describe(), "once, 2026-12-01 09:00");
        let _ = date!(2027-01-01);
    }
    fn manual(name: &str, input: Value) -> Value {
        json!({ "name": name, "kind": "count", "input": input, "when": { "type": "manual" } })
    }

    fn rule(name: &str, source: &str, ended: &str, action: &str, target: &str) -> Value {
        json!({ "name": name, "when": { "schedule": source, "ended": ended }, "then": { "action": action, "schedule": target } })
    }

    const OK: fn() -> Value = || json!({ "count": 1, "delay_ms": 100 });
    const FAIL: fn() -> Value = || json!({ "count": 2, "delay_ms": 100, "fail_at": 1 });

    #[tokio::test(start_paused = true)]
    async fn a_manual_schedule_runs_only_when_asked() {
        let rig = Rig::new("manual", json!({ "version": 1, "schedules": [ manual("m", OK()) ] }), 3);
        rig.advance(3600).await;
        assert_eq!(rig.view("m").state.runs, 0);
        assert_eq!(rig.view("m").next_ms, None);
        rig.scheduler.run_now("m").unwrap();
        rig.advance(1).await;
        let view = rig.view("m");
        assert_eq!(view.state.history.len(), 1);
        assert_eq!(view.state.history[0].by, "you");
    }

    #[tokio::test(start_paused = true)]
    async fn a_failure_in_x_starts_y_and_a_failure_in_y_starts_z() {
        let rig = Rig::new(
            "chain",
            json!({ "version": 1,
                "schedules": [ every_5s("x", FAIL()), manual("y", FAIL()), manual("z", OK()) ],
                "rules": [ rule("x_to_y", "x", "failed", "start", "y"), rule("y_to_z", "y", "failed", "start", "z") ] }),
            0,
        );
        rig.advance(5).await;
        rig.advance(1).await;
        assert_eq!(rig.view("y").state.runs, 1, "x failed, so y started");
        assert_eq!(rig.view("z").state.runs, 0);
        rig.advance(1).await;
        assert_eq!(rig.view("z").state.runs, 1, "y failed, so z started");
        rig.advance(1).await;
        let z = rig.view("z");
        assert_eq!(z.state.last_state, Some(JobState::Done));
        assert_eq!(z.state.history[0].by, "rule y_to_z");
        let job = z.state.history[0].job.unwrap();
        let origin = serde_json::to_value(rig.queue.inspect(job).unwrap()).unwrap()["job"]["origin"].clone();
        assert_eq!(origin, json!({ "by": "rule", "rule": "y_to_z", "schedule": "z" }));
    }

    #[tokio::test(start_paused = true)]
    async fn a_label_n_times_in_a_row_enables_the_next_job_and_a_miss_resets_the_count() {
        let mut next = every_5s("next", OK());
        next["enabled"] = json!(false);
        let mut watch = rule("fourth", "x", "done", "enable", "next");
        watch["when"]["label"] = json!("counted");
        watch["when"]["times"] = json!(4);
        let rig = Rig::new("streak", json!({ "version": 1, "schedules": [ manual("x", OK()), next ], "rules": [ watch ] }), 0);
        let run_x = || async {
            rig.scheduler.run_now("x").unwrap();
            rig.advance(1).await;
        };
        for _ in 0..3 {
            run_x().await;
        }
        assert_eq!(rig.scheduler.list().rules[0].streak, 3);
        assert!(!rig.view("next").enabled);
        run_x().await;
        assert!(rig.view("next").enabled, "enabled on the fourth");
        assert!(!rig.view("next").enabled_in_file, "the file is not changed by a rule");
        assert_eq!(rig.scheduler.list().rules[0].streak, 0, "the count starts again");
        rig.advance(5).await;
        assert_eq!(rig.view("next").state.runs, 1, "now runs on its timetable");
        // A different result in between resets the count.
        let rig = Rig::new(
            "streak_reset",
            json!({ "version": 1, "schedules": [ manual("x", OK()) ], "rules": [ rule("r", "x", "done", "disable", "x") ] }),
            0,
        );
        let mut r = rig.scheduler.list().rules[0].definition.clone();
        r["when"]["times"] = json!(2);
        rig.scheduler.save_rule(Some("r".into()), r).unwrap();
        rig.scheduler.run_now("x").unwrap();
        rig.advance(1).await;
        assert_eq!(rig.scheduler.list().rules[0].streak, 1);
        let mut failing = rig.view("x").definition;
        failing["input"] = FAIL();
        rig.scheduler.save_schedule(Some("x".into()), failing).unwrap();
        rig.scheduler.run_now("x").unwrap();
        rig.advance(1).await;
        assert_eq!(rig.scheduler.list().rules[0].streak, 0, "a failure broke the run of successes");
    }

    #[tokio::test(start_paused = true)]
    async fn the_loop_guard_stops_a_rule_that_restarts_its_own_job() {
        let rig = Rig::new(
            "loop",
            json!({ "version": 1, "schedules": [ manual("a", FAIL()) ], "rules": [ rule("again", "a", "failed", "start", "a") ] }),
            0,
        );
        rig.scheduler.run_now("a").unwrap();
        for _ in 0..30 {
            rig.advance(1).await;
        }
        let view = rig.view("a");
        assert_eq!(view.state.runs, u64::from(MAX_CHAIN), "{MAX_CHAIN} rule starts, then the guard");
        assert!(view.running_job.is_none());
        assert_eq!(view.state.history.len(), MAX_CHAIN as usize + 1, "the first run plus the chain");
    }

    #[tokio::test(start_paused = true)]
    async fn retries_wait_longer_each_time_and_only_the_final_result_counts() {
        let mut flaky = manual("flaky", FAIL());
        flaky["retries"] = json!(2);
        flaky["retry_wait"] = json!("10s");
        let rig = Rig::new(
            "retry",
            json!({ "version": 1, "schedules": [ flaky, manual("after", OK()) ], "rules": [ rule("on_fail", "flaky", "failed", "start", "after") ] }),
            3,
        );
        rig.scheduler.run_now("flaky").unwrap();
        rig.advance(1).await;
        let view = rig.view("flaky");
        assert_eq!(view.state.retry, 1);
        assert_eq!(view.state.failures_in_a_row, 0, "not final yet");
        assert_eq!(rig.view("after").state.runs, 0, "rules wait for the final result");
        rig.advance(9).await;
        assert!(rig.view("flaky").running_job.is_none(), "not before 10 s");
        rig.advance(1).await;
        assert!(rig.view("flaky").running_job.is_some(), "first retry 10 s after the failure was seen");
        rig.advance(1).await;
        rig.advance(19).await;
        assert!(rig.view("flaky").running_job.is_none(), "second retry waits 20 s");
        rig.advance(1).await;
        rig.advance(1).await;
        let view = rig.view("flaky");
        assert_eq!(view.state.retry, 0);
        assert_eq!(view.state.failures_in_a_row, 1, "three attempts, one failure");
        assert_eq!(view.state.history.iter().map(|h| h.by.as_str()).collect::<Vec<_>>(), ["you", "retry 1 of 2", "retry 2 of 2"]);
        assert_eq!(rig.view("after").state.runs, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn pause_everything_stops_timetables_and_rules_and_survives_a_restart() {
        let rig = Rig::new(
            "pause_all",
            json!({ "version": 1, "schedules": [ every_5s("t", OK()), manual("m", FAIL()), manual("n", OK()) ],
                "rules": [ rule("r", "m", "failed", "start", "n") ] }),
            0,
        );
        rig.scheduler.set_paused_all(true);
        assert!(rig.scheduler.list().upcoming.is_empty());
        rig.advance(30).await;
        assert_eq!(rig.view("t").state.runs, 0);
        rig.scheduler.run_now("m").unwrap();
        rig.advance(1).await;
        assert_eq!(rig.view("n").state.runs, 0, "rules do not start jobs while paused");
        let again = Rig::open(rig.dir.clone(), 0);
        assert!(again.scheduler.list().paused_all);
        again.scheduler.set_paused_all(false);
        again.advance(5).await;
        assert_eq!(again.view("t").state.runs, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn the_page_saves_renames_and_deletes_and_refuses_bad_entries_without_writing() {
        let rig = Rig::new(
            "edit",
            json!({ "version": 1, "note": "kept", "schedules": [ manual("a", OK()), manual("b", OK()) ],
                "rules": [ rule("r", "a", "done", "start", "b") ] }),
            3,
        );
        let path = rig.dir.join(SCHEDULES_FILE);
        let before = fs::read_to_string(&path).unwrap();
        let refused = [
            json!({ "name": "c", "kind": "count", "input": { "count": 0, "delay_ms": 100 }, "when": { "type": "manual" } }),
            json!({ "name": "c", "kind": "count", "input": {}, "when": { "type": "daily", "time": "25:00" } }),
            json!({ "name": "B", "kind": "count", "when": { "type": "manual" } }),
        ];
        for bad in refused {
            assert!(rig.scheduler.save_schedule(None, bad.clone()).is_err(), "{bad}");
        }
        assert!(rig.scheduler.save_schedule(None, manual("b", OK())).unwrap_err().contains("already exists"));
        assert!(rig.scheduler.save_rule(None, rule("r2", "a", "done", "start", "nobody")).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), before, "nothing written");

        let mut daily = json!({ "name": "c", "kind": "count", "input": OK(), "when": { "type": "daily", "time": "08:00" } });
        rig.scheduler.save_schedule(None, daily.clone()).unwrap();
        assert_eq!(rig.view("c").when, "daily at 08:00");
        daily["when"]["time"] = json!("09:30");
        rig.scheduler.save_schedule(Some("c".into()), daily).unwrap();
        assert_eq!(rig.view("c").when, "daily at 09:30");
        assert_eq!(rig.scheduler.list().schedules.len(), 3);

        rig.scheduler.run_now("a").unwrap();
        rig.advance(1).await;
        rig.scheduler.save_schedule(Some("a".into()), manual("first", OK())).unwrap();
        assert_eq!(rig.view("first").state.runs, 0);
        assert_eq!(rig.view("first").state.history.len(), 1, "history moves with a rename");
        assert!(rig.scheduler.list().rules[0].text.starts_with("when first succeeds"), "rules follow a rename");
        assert!(rig.scheduler.delete_schedule("b").unwrap_err().contains("rule r"));
        rig.scheduler.delete_rule("r").unwrap();
        rig.scheduler.delete_schedule("b").unwrap();
        rig.scheduler.set_enabled("c", false).unwrap();
        assert!(!rig.view("c").enabled);
        let file: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(file["note"], "kept");
        assert_eq!(file["schedules"].as_array().unwrap().len(), 2);
        assert_eq!(file["rules"], json!([]));

        fs::write(&path, "{ broken").unwrap();
        assert!(rig.scheduler.save_schedule(None, manual("d", OK())).unwrap_err().contains("by hand"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ broken", "a hand-edited file is never overwritten");
    }

    #[tokio::test(start_paused = true)]
    async fn the_preview_lists_the_next_24_hours() {
        let mut limited = json!({ "name": "hourly", "kind": "count", "input": OK(), "when": { "type": "every", "interval": "1h" } });
        limited["max_runs"] = json!(5);
        let mut off = json!({ "name": "off", "kind": "count", "input": OK(), "when": { "type": "every", "interval": "1h" } });
        off["enabled"] = json!(false);
        let daily = json!({ "name": "daily", "kind": "count", "input": OK(), "when": { "type": "daily", "time": "08:00" } });
        let rig = Rig::new("preview", json!({ "version": 1, "schedules": [ limited, off, daily, manual("m", OK()) ] }), 3);
        let upcoming = rig.scheduler.list().upcoming;
        let names: Vec<&str> = upcoming.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names.iter().filter(|n| **n == "hourly").count(), 5, "stops at max_runs");
        assert_eq!(names.iter().filter(|n| **n == "daily").count(), 1);
        assert!(!names.contains(&"off") && !names.contains(&"m"));
        assert!(upcoming.windows(2).all(|w| w[0].at_ms <= w[1].at_ms), "soonest first");
    }

    #[tokio::test(start_paused = true)]
    async fn history_keeps_the_newest_runs() {
        let rig = Rig::new("history", json!({ "version": 1, "schedules": [ every_5s("t", OK()) ] }), 3);
        for _ in 0..25 {
            rig.advance(5).await;
        }
        rig.advance(1).await;
        let history = rig.view("t").state.history;
        assert_eq!(history.len(), MAX_HISTORY);
        assert!(history.windows(2).all(|w| w[0].at_ms < w[1].at_ms));
    }

    // ---- Pipelines: a rule passes the finished job's result on ----------------
    // Real processes on the real clock (a paused clock would jump past the
    // time limit while bash runs), with the scheduler's wall clock moved by hand.

    fn command(name: &str, script: &str) -> Value {
        json!({ "name": name, "kind": "command", "input": { "script": script }, "when": { "type": "manual" } })
    }

    /// Ticks until `done` holds, up to 10 s.
    async fn settle(rig: &Rig, done: impl Fn(&Rig) -> bool) {
        for _ in 0..500 {
            rig.scheduler.tick();
            if done(rig) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the pipeline did not finish");
    }

    fn last_output(rig: &Rig, name: &str) -> String {
        rig.view(name).state.history.last().and_then(|h| h.output.clone()).unwrap_or_default()
    }

    #[tokio::test]
    async fn a_command_started_by_a_rule_gets_the_previous_result_in_its_environment() {
        let rig = Rig::new(
            "pipe_env",
            json!({ "version": 1,
                "schedules": [
                    command("gather", "echo 'ERROR disk full'; echo 'ERROR fan stopped'; exit 2"),
                    command("act", "printf '%s|%s|%s|%s\n' \"$PREVIOUS_SCHEDULE\" \"$PREVIOUS_LABEL\" \"$PREVIOUS_STATE\" \"$PREVIOUS_JOB\"; echo \"$PREVIOUS_OUTPUT\"; wc -l < \"$PREVIOUS_OUTPUT_FILE\"")
                ],
                "rules": [ rule("on_errors", "gather", "failed", "start", "act") ] }),
            0,
        );
        let source = rig.scheduler.run_now("gather").unwrap();
        settle(&rig, |r| !r.view("act").state.history.is_empty()).await;
        let out = last_output(&rig, "act");
        assert!(out.starts_with(&format!("gather|exit_2|failed|{source}\n")), "{out}");
        assert!(out.contains("ERROR disk full\nERROR fan stopped"), "{out}");
        assert!(out.trim_end().ends_with('2'), "the file holds both lines: {out}");
        // Started by a person: no previous result, the variables are unset.
        rig.scheduler.run_now("act").unwrap();
        settle(&rig, |r| r.view("act").state.history.len() == 2).await;
        assert!(last_output(&rig, "act").starts_with("|||\n"), "{}", last_output(&rig, "act"));
    }

    #[tokio::test]
    async fn text_from_a_previous_job_never_runs_as_code() {
        let marker = std::env::temp_dir().join(format!("rust_scaffold_injection_{}", std::process::id()));
        let _ = fs::remove_file(&marker);
        let hostile = format!("echo 'innocent $(touch {m}) `touch {m}`'", m = marker.display());
        let rig = Rig::new(
            "pipe_injection",
            json!({ "version": 1,
                "schedules": [ command("source", &hostile), command("sink", "echo \"got: $PREVIOUS_OUTPUT\"; eval true") ],
                "rules": [ rule("pass", "source", "done", "start", "sink") ] }),
            0,
        );
        rig.scheduler.run_now("source").unwrap();
        settle(&rig, |r| !r.view("sink").state.history.is_empty()).await;
        let out = last_output(&rig, "sink");
        assert!(out.contains("innocent $(touch"), "passed on as text: {out}");
        assert!(!marker.exists(), "the text was not run");
    }

    #[tokio::test]
    async fn an_ai_prompt_gets_the_previous_output_and_a_retry_gets_it_again() {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
        // A fake Ollama: the first request fails (HTTP 500), the second
        // answers; both prompts are sent back for checking.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            for attempt in 0..2 {
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
                let request: Value = serde_json::from_slice(&body).unwrap();
                tx.send(request["messages"][0]["content"].as_str().unwrap().to_string()).unwrap();
                let reply = if attempt == 0 {
                    "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 2\r\n\r\n{}".to_string()
                } else {
                    let body = r#"{"message":{"content":"Two errors: disk and fan."}}"#;
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len())
                };
                reader.get_mut().write_all(reply.as_bytes()).await.unwrap();
            }
        });
        let ai = json!({ "name": "summarise", "kind": "ai", "when": { "type": "manual" }, "retries": 1, "retry_wait": "5s",
            "input": { "provider": "ollama", "model": "m", "endpoint": format!("http://127.0.0.1:{port}/api/chat"),
                "prompt": "Summarise these {previous_label} lines from {previous_schedule}:\n{previous_output}" } });
        let rig = Rig::new(
            "pipe_ai",
            json!({ "version": 1,
                "schedules": [ command("gather", "echo 'ERROR disk full'; echo 'ERROR fan {previous_output}'"), ai ],
                "rules": [ rule("ask", "gather", "done", "start", "summarise") ] }),
            0,
        );
        rig.scheduler.run_now("gather").unwrap();
        settle(&rig, |r| r.view("summarise").state.retry == 1).await;
        let want = "Summarise these exit_0 lines from gather:\nERROR disk full\nERROR fan {previous_output}";
        assert_eq!(rx.recv().await.unwrap(), want, "placeholders filled; text inside the output left alone");
        *rig.clock.lock().unwrap() += time::Duration::seconds(5);
        settle(&rig, |r| r.view("summarise").state.last_state == Some(JobState::Done)).await;
        assert_eq!(rx.recv().await.unwrap(), want, "the retry was given the same previous output");
        assert_eq!(last_output(&rig, "summarise"), "Two errors: disk and fan.");
    }


    /// A command that reports the next number from `file` as its value
    /// `{"percent": N}` (one number per line, used up in order).
    fn reporter(name: &str, file: &std::path::Path) -> Value {
        command(
            name,
            &format!(
                r#"f='{}'; v=$(head -n 1 "$f"); sed -i 1d "$f"; printf '{{"value":{{"percent":%s}},"message":"%s%% full"}}' "$v" "$v" > "$JOB_RESULT""#,
                file.display()
            ),
        )
    }

    async fn run_and_settle(rig: &Rig, name: &str) {
        let before = rig.view(name).state.history.len();
        rig.scheduler.run_now(name).unwrap();
        settle(rig, |r| r.view(name).state.history.len() > before).await;
        // One more pass, so rules fired by the result have started theirs.
        settle(rig, |r| r.view("alert").running_job.is_none()).await;
    }

    #[tokio::test]
    async fn a_rule_reacts_to_a_value_n_times_in_a_row() {
        let rig_dir = temp_dir("value_rule_numbers");
        let numbers = rig_dir.join("numbers.txt");
        fs::write(&numbers, "80\n95\n50\n91\n96\n").unwrap();
        let mut over = rule("too_full", "disk", "done", "start", "alert");
        over["when"]["value"] = json!({ "field": "percent", "above": 90 });
        over["when"]["times"] = json!(2);
        let rig = Rig::new(
            "value_rule",
            json!({ "version": 1,
                "schedules": [ reporter("disk", &numbers), command("alert", "echo \"$PREVIOUS_VALUE|$PREVIOUS_MESSAGE\"") ],
                "rules": [ over ] }),
            0,
        );
        assert!(rig.scheduler.list().rules[0].text.contains("value percent above 90"), "{}", rig.scheduler.list().rules[0].text);
        // 80: no. 95: one in a row. 50: back to none. 91, 96: two in a row.
        for (step, fired) in [(1, 0), (2, 0), (3, 0), (4, 0), (5, 1)] {
            run_and_settle(&rig, "disk").await;
            assert_eq!(rig.view("alert").state.history.len(), fired, "after run {step}");
        }
        assert_eq!(rig.view("disk").state.history.last().unwrap().value, Some(json!({ "percent": 96 })), "history keeps the value");
        assert_eq!(last_output(&rig, "alert").trim(), r#"{"percent":96}|96% full"#);
    }

    #[tokio::test]
    async fn a_rule_reacts_when_the_output_changes() {
        let dir = temp_dir("changed_rule_page");
        let page = dir.join("page.txt");
        let mut watch = rule("on_change", "watch", "done", "start", "alert");
        watch["when"]["changed"] = json!(true);
        let rig = Rig::new(
            "changed_rule",
            json!({ "version": 1,
                "schedules": [ command("watch", &format!("cat '{}'", page.display())), command("alert", "echo changed") ],
                "rules": [ watch ] }),
            0,
        );
        // First run: nothing to compare with. Same text: no change.
        for (text, fired) in [("price 10", 0), ("price 10", 0), ("price 12", 1), ("price 12", 1), ("price 10", 2)] {
            fs::write(&page, text).unwrap();
            run_and_settle(&rig, "watch").await;
            assert_eq!(rig.view("alert").state.history.len(), fired, "after \"{text}\"");
        }
        // A failed run neither counts as a change nor replaces the last
        // successful output.
        fs::remove_file(&page).unwrap();
        run_and_settle(&rig, "watch").await;
        fs::write(&page, "price 10").unwrap();
        run_and_settle(&rig, "watch").await;
        assert_eq!(rig.view("alert").state.history.len(), 2);
        // The fingerprint is kept across a restart.
        let again = Rig::open(rig.dir.clone(), 0);
        assert!(again.view("watch").state.last_fingerprint.is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn bad_value_and_changed_rules_are_refused() {
        let with = |when: Value| {
            let mut r = rule("r", "a", "done", "start", "b");
            for (k, v) in when.as_object().unwrap() {
                r["when"][k] = v.clone();
            }
            r
        };
        let rig = Rig::new(
            "bad_value_rules",
            json!({ "version": 1, "schedules": [ manual("a", OK()), manual("b", OK()) ], "rules": [
                with(json!({ "value": { "field": "percent" } })),
                with(json!({ "value": { "field": "a b", "above": 1 } })),
                with(json!({ "value": { "field": "percent", "over": 1 } })),
                with(json!({ "ended": "failed", "changed": true })),
            ]}),
            3,
        );
        let skipped = rig.scheduler.list().skipped;
        assert_eq!(skipped.len(), 4, "{skipped:?}");
        assert!(skipped[0].contains("above, below or equals"));
        assert!(skipped[1].contains("value field"));
        assert!(skipped[2].contains("over"));
        assert!(skipped[3].contains("changed compares successful runs"));
    }

    #[test]
    fn value_tests_compare_numbers_and_text() {
        let t = |v: Value| serde_json::from_value::<ValueTest>(v).unwrap();
        let v = json!({ "percent": 92, "state": "ok", "disk": { "free": 3 } });
        assert!(t(json!({ "field": "percent", "above": 90 })).passes(Some(&v)));
        assert!(!t(json!({ "field": "percent", "above": 92 })).passes(Some(&v)), "above is strict");
        assert!(t(json!({ "field": "percent", "above": 90, "below": 95 })).passes(Some(&v)));
        assert!(t(json!({ "field": "percent", "equals": 92.0 })).passes(Some(&v)), "92 equals 92.0");
        assert!(t(json!({ "field": "state", "equals": "ok" })).passes(Some(&v)));
        assert!(!t(json!({ "field": "state", "above": 1 })).passes(Some(&v)), "text is not a number");
        assert!(t(json!({ "field": "disk.free", "below": 5 })).passes(Some(&v)));
        assert!(!t(json!({ "field": "missing", "below": 5 })).passes(Some(&v)));
        assert!(!t(json!({ "field": "percent", "above": 1 })).passes(None));
    }

    #[test]
    fn fingerprints_are_stable() {
        // Kept in jobs_state.json, so they must not change between builds.
        assert_eq!(fingerprint("", None), "af63bd4c8601b7df");
        assert_ne!(fingerprint("a", None), fingerprint("", Some(&json!("a"))));
    }
}
