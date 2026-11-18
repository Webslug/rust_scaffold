//! Jobs 2.0 (`docs/revisions_jobs_tasks_v2.md`): Triggers, Schedules and
//! Actions, saved in `jobs.json` in the settings folder.
//!
//! - A Trigger checks a watched value (an element on a web page) every N
//!   seconds and starts its Actions when its condition becomes true: the
//!   value changes, or goes above, below or equal to a set value. It fires
//!   once per crossing, not on every check, and the first check after it is
//!   saved or switched on only records the value.
//! - A Schedule starts its Actions at set times: once, every so often,
//!   daily, weekly or monthly. A run missed while the app was closed is made
//!   up once when it starts, however many were missed; runs missed while
//!   the Schedule was off are not made up.
//! - An Action does the work: a command, an AI prompt or a notification
//!   (job kinds in `src/job_kinds.rs`, run on the job queue, so they show
//!   under Running now and can be killed), or switches a Trigger, Schedule
//!   or Action on or off. When it succeeds it can start further Actions,
//!   passing on what it printed (`{output}`) and the Trigger's reading
//!   (`{value}`, `{previous}`).
//!
//! Triggers and Schedules only start Actions; everything else is an Action.
//! A chain of Actions stops after `MAX_CHAIN` steps and an Action cannot
//! list itself, so a loop cannot run for ever.
//!
//! Last readings, counts and recent results are kept in `jobs_state.json`,
//! so the app never rewrites jobs.json for its own bookkeeping. The Jobs
//! page changes jobs.json through `save`, `delete`, `duplicate` and
//! `set_enabled`: the whole new file is checked first and written
//! atomically, and a refused change writes nothing.
//!
//! One task on Tauri's Tokio runtime does the timing. It sleeps until the
//! next check or run is due, but never longer than 60 s at a time, because
//! Tokio's clock stops while the computer is suspended: on waking it checks
//! the wall clock again. Times in jobs.json are local; each one is converted
//! with the local UTC offset of that moment, so "daily at 08:00" stays at
//! 08:00 across a clock change.

use std::collections::{BTreeMap, VecDeque};
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
use crate::jobs::{Given, JobState, KindEntry, Origin, Queue, MAX_OUTPUT_BYTES};

pub const FILE: &str = "jobs.json";
pub const STATE_FILE: &str = "jobs_state.json";
pub const DEFAULT_FILE: &str = include_str!("../jobs.default.json");
const EMPTY_STATE: &str = "{\"version\":2}";
const FILE_VERSION: u64 = 2;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_STATE_BYTES: u64 = 4 * 1024 * 1024;
/// Most Triggers, Schedules or Actions in one section.
const MAX_ITEMS: usize = 500;
const MAX_NAME_CHARS: usize = 60;
/// Most Actions one Trigger, Schedule or Action may start.
const MAX_THEN: usize = 10;
/// Longest chain of Actions (A starts B starts C ...) before the loop guard
/// stops it.
pub const MAX_CHAIN: u32 = 10;
/// Highest "fire at most" and "run at most" count.
const MAX_COUNT: u64 = 999_999;
/// Jobs of one Action queued or running at once; more starts are skipped.
const MAX_RUNNING: usize = 5;
/// Recent results kept per Action, and how much of each output (its end).
const MAX_HISTORY: usize = 10;
const HISTORY_OUTPUT_BYTES: usize = 400;
/// Longest Trigger interval.
const MAX_EVERY_SECONDS: u64 = 366 * 24 * 60 * 60;
/// Shortest and longest Schedule interval.
pub const MIN_INTERVAL: Duration = Duration::from_secs(5);
pub const MAX_INTERVAL: Duration = Duration::from_secs(MAX_EVERY_SECONDS);
/// Longest single sleep, so a suspend cannot delay a run by more than this.
const MAX_SLEEP: Duration = Duration::from_secs(60);
/// How often to look for results while an Action's job is running.
const RESULT_POLL: Duration = Duration::from_secs(1);
/// Trigger checks at once (each fetches a web page).
const MAX_CHECKS: usize = 4;
/// The kind name of the built-in "switch on or off" Action.
pub const SWITCH: &str = "switch";

/// Window event carrying the whole reply after any change.
pub const CHANGED_EVENT: &str = "automation:changed";

/// The three sections of the Jobs page and of jobs.json.
#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Trigger,
    Schedule,
    Action,
}

impl Section {
    fn key(self) -> &'static str {
        match self {
            Section::Trigger => "triggers",
            Section::Schedule => "schedules",
            Section::Action => "actions",
        }
    }

    fn word(self) -> &'static str {
        match self {
            Section::Trigger => "trigger",
            Section::Schedule => "schedule",
            Section::Action => "action",
        }
    }
}

/// The Overview's quick buttons (`Automation::bulk`).
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Bulk {
    EnableAll,
    DisableAll,
    DeleteAll,
    RestoreDemos,
}

// ---- Times (Schedules) ---------------------------------------------------------

/// When a Schedule runs, as written in jobs.json.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum When {
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
pub enum Timing {
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
    pub fn parse(&self) -> Result<Timing, String> {
        Ok(match self {
            When::Once { at } => Timing::Once(parse_local(at)?),
            When::Every { interval } => Timing::Every(parse_interval(interval)?),
            When::Daily { time } => Timing::Daily(parse_time(time)?),
            When::Weekly { day, time } => Timing::Weekly(parse_weekday(day)?, parse_time(time)?),
            When::Monthly { day, time } => {
                if !(1..=31).contains(day) {
                    return Err(format!("monthly day {day} must be 1 to 31"));
                }
                Timing::Monthly(*day, parse_time(time)?)
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

/// "30 seconds", "5 minutes", "2 hours", "1 day".
fn duration_words(seconds: u64) -> String {
    if seconds % 86_400 == 0 {
        plural(seconds / 86_400, "day")
    } else if seconds % 3600 == 0 {
        plural(seconds / 3600, "hour")
    } else if seconds % 60 == 0 {
        plural(seconds / 60, "minute")
    } else {
        plural(seconds, "second")
    }
}

impl Timing {
    /// In words, for the Jobs page.
    pub fn describe(&self) -> String {
        match *self {
            Timing::Once(at) => format!("once, {} {}", at.date(), hhmm(at.time())),
            Timing::Every(interval) => format!("every {}", duration_words(interval.as_secs())),
            Timing::Daily(time) => format!("daily at {}", hhmm(time)),
            Timing::Weekly(day, time) => format!("every {day} at {}", hhmm(time)),
            Timing::Monthly(day, time) if day > 28 => {
                format!("monthly on day {day} at {} (or the month's last day)", hhmm(time))
            }
            Timing::Monthly(day, time) => format!("monthly on day {day} at {}", hhmm(time)),
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
pub fn next_after(timing: Timing, after: OffsetDateTime, offset_at: OffsetAt) -> Option<OffsetDateTime> {
    let later = |local: PlainDateTime| Some(local_instant(local, offset_at)).filter(|t| *t > after);
    match timing {
        Timing::Once(at) => later(at),
        Timing::Every(interval) => after.checked_add(interval.try_into().ok()?),
        Timing::Daily(time) => {
            let mut date = local_date(after, offset_at);
            for _ in 0..3 {
                if let Some(t) = later(date.with_time(time)) {
                    return Some(t);
                }
                date = date.next_day()?;
            }
            None
        }
        Timing::Weekly(day, time) => {
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
        Timing::Monthly(day, time) => {
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

// ---- What a Trigger watches, and its condition --------------------------------

/// What a Trigger watches, as written in jobs.json: a web page element
/// (`{"type": "web_watch", "url": ..., "find": "id", "name": ...,
/// "read_as": "number"}`) or this computer (Hardware-Watch, see
/// `src/hardware_watch.rs`): `{"type": "cpu"}`, `{"type": "memory"}`,
/// `{"type": "disk", "path": "/", "measure": "used_percent"}` (or
/// "free_gb"), `{"type": "vram"}`, `{"type": "file", "path": ...,
/// "watch_for": "contents"}` (or "permissions").
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Watch {
    #[cfg(feature = "web-watch")]
    WebWatch(crate::web_watch::WebWatch),
    Cpu {},
    Memory {},
    Disk(crate::hardware_watch::DiskWatch),
    Vram {},
    File(crate::hardware_watch::FileWatch),
}

impl Watch {
    fn check(&self) -> Result<(), String> {
        match self {
            #[cfg(feature = "web-watch")]
            Watch::WebWatch(w) => w.check(),
            Watch::Cpu {} | Watch::Memory {} | Watch::Vram {} => Ok(()),
            Watch::Disk(d) => d.check(),
            Watch::File(f) => f.check(),
        }
    }

    /// Whether the reading has a number (needed by "is above" and "is below").
    fn reads_numbers(&self) -> bool {
        match self {
            #[cfg(feature = "web-watch")]
            Watch::WebWatch(w) => w.read_as == crate::web_watch::ReadAs::Number,
            Watch::File(_) => false,
            _ => true,
        }
    }

    fn min_every(&self) -> u64 {
        match self {
            #[cfg(feature = "web-watch")]
            Watch::WebWatch(_) => crate::web_watch::MIN_EVERY_SECONDS,
            _ => crate::hardware_watch::MIN_EVERY_SECONDS,
        }
    }

    /// In words: "bitmods.com/probe #probe-gbp", "CPU load".
    fn describe(&self) -> String {
        use crate::hardware_watch::{DiskMeasure, FileChange};
        match self {
            #[cfg(feature = "web-watch")]
            Watch::WebWatch(w) => {
                let place = w.url.split_once("://").map_or(w.url.as_str(), |(_, rest)| rest).trim_end_matches('/');
                format!("{place} {}", w.shown())
            }
            Watch::Cpu {} => "CPU load (%)".into(),
            Watch::Memory {} => "memory used (%)".into(),
            Watch::Disk(d) if d.measure == DiskMeasure::FreeGb => format!("disk free at {} (GB)", d.path),
            Watch::Disk(d) => format!("disk used at {} (%)", d.path),
            Watch::Vram {} => "graphics card memory used (%)".into(),
            Watch::File(f) if f.watch_for == FileChange::Permissions => format!("permissions of {}", f.path),
            Watch::File(f) => format!("file {}", f.path),
        }
    }

    /// Reads the value: `{"text": ...}`, with `"number"` when it has one.
    /// Errors are sentences for people.
    async fn read(&self) -> Result<Value, String> {
        use crate::hardware_watch as hw;
        match self {
            #[cfg(feature = "web-watch")]
            Watch::WebWatch(w) => match tokio::time::timeout(crate::web_watch::CHECK_TIMEOUT, w.read()).await {
                Ok(result) => result,
                Err(_) => Err(format!("{} did not answer within {} seconds", w.url, crate::web_watch::CHECK_TIMEOUT.as_secs())),
            },
            Watch::Cpu {} => hw::cpu().await,
            Watch::Memory {} => hw::memory(),
            Watch::Disk(d) => hw::disk(d),
            Watch::Vram {} => hw::vram().await,
            Watch::File(f) => hw::file(f),
        }
    }
}

/// When a Trigger fires, as written in jobs.json: `{"is": "changes"}`,
/// `{"is": "above", "value": 5000}`, `{"is": "below", "value": 10}` or
/// `{"is": "equals", "value": "sold out"}`.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "is", rename_all = "snake_case", deny_unknown_fields)]
pub enum Condition {
    Changes,
    Above { value: f64 },
    Below { value: f64 },
    Equals { value: Value },
}

/// The number in a reading, if it has one.
fn number_of(reading: &Value) -> Option<f64> {
    reading.get("number").and_then(Value::as_f64)
}

fn text_of(reading: &Value) -> &str {
    reading.get("text").and_then(Value::as_str).unwrap_or_default()
}

/// Two readings are the same when their numbers are equal (both read as a
/// number) or else their texts are.
pub fn same_reading(a: &Value, b: &Value) -> bool {
    match (number_of(a), number_of(b)) {
        (Some(x), Some(y)) => x == y,
        _ => text_of(a) == text_of(b),
    }
}

/// A number written in jobs.json as a number or as text ("5000").
fn as_number(value: &Value) -> Option<f64> {
    value.as_f64().or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
}

impl Condition {
    fn check(&self, reads_numbers: bool) -> Result<(), String> {
        match self {
            Condition::Changes => Ok(()),
            Condition::Above { value } | Condition::Below { value } => {
                if !reads_numbers {
                    return Err("\"is above\" and \"is below\" need Read as: Number".into());
                }
                if !value.is_finite() {
                    return Err("the value to compare with must be a number".into());
                }
                Ok(())
            }
            Condition::Equals { value } => match value {
                Value::String(text) if text.chars().count() <= 1000 => Ok(()),
                Value::Number(_) => Ok(()),
                _ => Err("\"equals\" needs a number or a text of at most 1000 characters".into()),
            },
        }
    }

    /// In words, for the Jobs page.
    pub fn describe(&self) -> String {
        match self {
            Condition::Changes => "changes".into(),
            Condition::Above { value } => format!("is above {value}"),
            Condition::Below { value } => format!("is below {value}"),
            Condition::Equals { value: Value::String(text) } => format!("equals \"{text}\""),
            Condition::Equals { value } => format!("equals {value}"),
        }
    }

    /// Whether a reading meets "is above", "is below" or "equals" (strictly
    /// above or below). None for "changes", which compares two readings.
    pub fn holds(&self, reading: &Value) -> Option<bool> {
        Some(match self {
            Condition::Changes => return None,
            Condition::Above { value } => number_of(reading).is_some_and(|n| n > *value),
            Condition::Below { value } => number_of(reading).is_some_and(|n| n < *value),
            Condition::Equals { value } => match (number_of(reading), as_number(value)) {
                (Some(n), Some(want)) => n == want,
                _ => {
                    let want = value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string());
                    text_of(reading).trim() == want.trim()
                }
            },
        })
    }
}

// ---- Definitions, as written in jobs.json --------------------------------------

fn yes() -> bool {
    true
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TriggerDef {
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub watch: Watch,
    pub every_seconds: u64,
    pub when: Condition,
    /// Switch off after firing this many times (None: never).
    #[serde(default)]
    pub max_fires: Option<u64>,
    /// The Actions it starts, by name.
    #[serde(default)]
    pub then: Vec<String>,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScheduleDef {
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub when: When,
    #[serde(default)]
    pub max_runs: Option<u64>,
    #[serde(default)]
    pub then: Vec<String>,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ActionDef {
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// A job kind (`command`, `ai`, `notify`) or `switch`.
    pub kind: String,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Actions started when this one succeeds.
    #[serde(default)]
    pub then: Vec<String>,
}

/// The input of a `switch` Action: switch a Trigger, Schedule or Action on
/// or off.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SwitchInput {
    pub section: Section,
    pub name: String,
    pub enabled: bool,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
struct DelayInput {
    schedule: String,
    /// A number and s, m, h or d: "2h".
    by: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
struct SetCountInput {
    section: Section,
    name: String,
    /// The new "fire at most" or "run at most"; None for no limit.
    #[serde(default)]
    count: Option<u64>,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
struct TargetInput {
    section: Section,
    name: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
struct FireInput {
    trigger: String,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
struct LogInput {
    level: LogLevel,
    /// Placeholders work as in a notification.
    text: String,
}

/// What an `app` Action does to the application itself.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppDo {
    Show,
    Hide,
    Quit,
    Restart,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
struct AppInput {
    #[serde(rename = "do")]
    what: AppDo,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
struct NoInput {}

/// The Actions the engine carries out itself, rather than as a job on the
/// queue: they change Triggers, Schedules and Actions, the log or the app.
pub const BUILTIN_KINDS: &[&str] = &[SWITCH, "delay", "set_count", "delete", "fire", "log", "clear_errors", "app"];

#[derive(Clone, Debug, PartialEq)]
pub enum Builtin {
    Switch(SwitchInput),
    Delay { schedule: String, by: Duration },
    SetCount { section: Section, name: String, count: Option<u64> },
    Delete { section: Section, name: String },
    Fire { trigger: String },
    Log { level: LogLevel, text: String },
    ClearErrors,
    App(AppDo),
}

impl Builtin {
    /// Parses a built-in kind's input; None when `kind` is a job kind.
    fn parse(kind: &str, input: &Value) -> Option<Result<Builtin, String>> {
        fn read<T: serde::de::DeserializeOwned>(kind: &str, input: &Value) -> Result<T, String> {
            let input = if input.is_null() { Value::Object(Map::new()) } else { input.clone() };
            serde_json::from_value(input).map_err(|e| format!("bad input for {kind}: {e}"))
        }
        let built = match kind {
            SWITCH => read::<SwitchInput>(kind, input).map(Builtin::Switch),
            "delay" => read::<DelayInput>(kind, input)
                .and_then(|d| Ok(Builtin::Delay { by: parse_interval(&d.by)?, schedule: d.schedule })),
            "set_count" => read::<SetCountInput>(kind, input).and_then(|c| {
                if c.section == Section::Action {
                    return Err("only a trigger's or a schedule's count can be set".into());
                }
                check_count("the count", c.count)?;
                Ok(Builtin::SetCount { section: c.section, name: c.name, count: c.count })
            }),
            "delete" => read::<TargetInput>(kind, input).map(|t| Builtin::Delete { section: t.section, name: t.name }),
            "fire" => read::<FireInput>(kind, input).map(|f| Builtin::Fire { trigger: f.trigger }),
            "log" => read::<LogInput>(kind, input).and_then(|l| {
                if l.text.trim().is_empty() || l.text.len() > 2000 {
                    return Err("the log text must be 1 to 2000 characters".into());
                }
                Ok(Builtin::Log { level: l.level, text: l.text })
            }),
            "clear_errors" => read::<NoInput>(kind, input).map(|_| Builtin::ClearErrors),
            "app" => read::<AppInput>(kind, input).map(|a| Builtin::App(a.what)),
            _ => return None,
        };
        Some(built.and_then(|b| {
            if let Some((_, name)) = b.target() {
                check_name(name)?;
            }
            Ok(b)
        }))
    }

    /// The entry it acts on, if any.
    fn target(&self) -> Option<(Section, &str)> {
        match self {
            Builtin::Switch(s) => Some((s.section, &s.name)),
            Builtin::Delay { schedule, .. } => Some((Section::Schedule, schedule)),
            Builtin::SetCount { section, name, .. } | Builtin::Delete { section, name } => Some((*section, name)),
            Builtin::Fire { trigger } => Some((Section::Trigger, trigger)),
            _ => None,
        }
    }

    /// In words, for the Actions list.
    fn describe(&self) -> String {
        match self {
            Builtin::Switch(s) => format!("switch {} {} \"{}\"", if s.enabled { "on" } else { "off" }, s.section.word(), s.name),
            Builtin::Delay { schedule, by } => format!("delay schedule \"{schedule}\" by {}", duration_words(by.as_secs())),
            Builtin::SetCount { section, name, count: Some(n) } => format!("set {} \"{name}\" to at most {n}", section.word()),
            Builtin::SetCount { section, name, count: None } => format!("set {} \"{name}\" to no limit", section.word()),
            Builtin::Delete { section, name } => format!("delete {} \"{name}\"", section.word()),
            Builtin::Fire { trigger } => format!("fire trigger \"{trigger}\""),
            Builtin::Log { level, text } => format!("log {}: {}", format!("{level:?}").to_lowercase(), first_line(text, 50)),
            Builtin::ClearErrors => "clear the Errors list".into(),
            Builtin::App(AppDo::Show) => "show the app".into(),
            Builtin::App(AppDo::Hide) => "hide the app".into(),
            Builtin::App(AppDo::Quit) => "quit the app".into(),
            Builtin::App(AppDo::Restart) => "restart the app".into(),
        }
    }
}

/// The field of a built-in Action's input that names the entry it acts on,
/// and that entry's section (fixed, or read from the input's "section").
fn target_field(kind: &str) -> Option<(Option<Section>, &'static str)> {
    match kind {
        SWITCH | "set_count" | "delete" => Some((None, "name")),
        "delay" => Some((Some(Section::Schedule), "schedule")),
        "fire" => Some((Some(Section::Trigger), "trigger")),
        _ => None,
    }
}

/// Whether a built-in Action (as written in jobs.json) acts on `section`
/// `name`.
fn raw_targets(action: &Value, section: Section, name: &str) -> bool {
    let kind = action.get("kind").and_then(Value::as_str).unwrap_or_default();
    let Some((fixed, field)) = target_field(kind) else { return false };
    let Some(input) = action.get("input") else { return false };
    let in_section = match fixed {
        Some(s) => s == section,
        None => input.get("section").and_then(Value::as_str) == Some(section.word()),
    };
    in_section && input.get(field).and_then(Value::as_str) == Some(name)
}

/// An entry of jobs.json that was skipped, and why.
#[derive(Clone, Debug)]
pub struct Skip {
    pub section: Section,
    pub index: usize,
    pub name: Option<String>,
    pub reason: String,
}

impl Skip {
    pub fn text(&self) -> String {
        let name = self.name.as_ref().map(|n| format!(" \"{n}\"")).unwrap_or_default();
        format!("{} {}{name}: {}", self.section.word(), self.index + 1, self.reason)
    }
}

#[derive(Clone, Debug)]
pub struct TriggerItem {
    pub def: TriggerDef,
    /// As written in the file, for the edit form.
    pub raw: Value,
}

#[derive(Clone, Debug)]
pub struct ScheduleItem {
    pub def: ScheduleDef,
    pub timing: Timing,
    pub raw: Value,
}

#[derive(Clone, Debug)]
pub struct ActionItem {
    pub def: ActionDef,
    /// Set for a built-in Action (switch, delay, set_count ...).
    pub builtin: Option<Builtin>,
    pub raw: Value,
}

pub struct Parsed {
    pub triggers: Vec<TriggerItem>,
    pub schedules: Vec<ScheduleItem>,
    pub actions: Vec<ActionItem>,
    pub skipped: Vec<Skip>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileText {
    version: u64,
    #[serde(default)]
    note: Option<Value>,
    #[serde(default)]
    triggers: Vec<Value>,
    #[serde(default)]
    schedules: Vec<Value>,
    #[serde(default)]
    actions: Vec<Value>,
}

/// Names: any text people can read, at most 60 characters, unique within
/// its section.
pub fn check_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("the name is empty".into());
    }
    if name.trim() != name {
        return Err("the name starts or ends with a space".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("the name is over {MAX_NAME_CHARS} characters"));
    }
    if name.chars().any(char::is_control) {
        return Err("the name contains a control character".into());
    }
    Ok(())
}

fn check_count(what: &str, count: Option<u64>) -> Result<(), String> {
    match count {
        Some(n) if !(1..=MAX_COUNT).contains(&n) => Err(format!("{what} must be 1 to {MAX_COUNT}")),
        _ => Ok(()),
    }
}

/// The Actions list of a Trigger, Schedule or Action: no repeats, at most
/// `MAX_THEN`. Whether each exists is checked once all Actions are read.
fn check_then(then: &[String]) -> Result<(), String> {
    if then.len() > MAX_THEN {
        return Err(format!("it starts {} actions (at most {MAX_THEN})", then.len()));
    }
    for (i, name) in then.iter().enumerate() {
        if then[..i].contains(name) {
            return Err(format!("action \"{name}\" is listed twice"));
        }
    }
    Ok(())
}

fn name_of(value: &Value) -> Option<String> {
    value.get("name").and_then(Value::as_str).map(str::to_string)
}

/// Parses jobs.json. Bad entries are skipped with a reason, so one mistake
/// does not stop the rest; a file that is not valid at all is an error. An
/// entry that starts or switches an Action, Trigger or Schedule that is
/// missing (or was skipped) is skipped too.
pub fn parse(text: &str, queue: &Queue) -> Result<Parsed, String> {
    let file: FileText = serde_json::from_str(text).map_err(|e| format!("not valid: {e}"))?;
    let _ = file.note;
    if file.version != FILE_VERSION {
        return Err(format!("version {} is not known (expected {FILE_VERSION})", file.version));
    }
    for (section, list) in [(Section::Trigger, &file.triggers), (Section::Schedule, &file.schedules), (Section::Action, &file.actions)] {
        if list.len() > MAX_ITEMS {
            return Err(format!("{} {} (at most {MAX_ITEMS})", list.len(), section.key()));
        }
    }
    let mut skipped = Vec::new();

    let mut actions: Vec<ActionItem> = Vec::new();
    for (index, value) in file.actions.into_iter().enumerate() {
        let name = name_of(&value);
        let raw = value.clone();
        let checked = serde_json::from_value::<ActionDef>(value).map_err(|e| e.to_string()).and_then(|def| {
            check_name(&def.name)?;
            if actions.iter().any(|a| a.def.name == def.name) {
                return Err("the name is used twice".into());
            }
            check_then(&def.then)?;
            if def.then.contains(&def.name) {
                return Err("an action cannot start itself".into());
            }
            let builtin = match Builtin::parse(&def.kind, &def.input) {
                Some(built) => Some(built?),
                None => {
                    queue.check(&def.kind, &def.input, def.timeout_ms)?;
                    None
                }
            };
            Ok(ActionItem { def, builtin, raw })
        });
        match checked {
            Ok(item) => actions.push(item),
            Err(reason) => skipped.push(Skip { section: Section::Action, index, name, reason }),
        }
    }

    let mut triggers: Vec<TriggerItem> = Vec::new();
    for (index, value) in file.triggers.into_iter().enumerate() {
        let name = name_of(&value);
        let raw = value.clone();
        let checked = serde_json::from_value::<TriggerDef>(value).map_err(|e| e.to_string()).and_then(|def| {
            check_name(&def.name)?;
            if triggers.iter().any(|t| t.def.name == def.name) {
                return Err("the name is used twice".into());
            }
            def.watch.check()?;
            def.when.check(def.watch.reads_numbers())?;
            let min = def.watch.min_every();
            if !(min..=MAX_EVERY_SECONDS).contains(&def.every_seconds) {
                return Err(format!("it may check at most every {} (and at least every 366 days)", duration_words(min)));
            }
            check_count("fire at most", def.max_fires)?;
            check_then(&def.then)?;
            Ok(TriggerItem { def, raw })
        });
        match checked {
            Ok(item) => triggers.push(item),
            Err(reason) => skipped.push(Skip { section: Section::Trigger, index, name, reason }),
        }
    }

    let mut schedules: Vec<ScheduleItem> = Vec::new();
    for (index, value) in file.schedules.into_iter().enumerate() {
        let name = name_of(&value);
        let raw = value.clone();
        let checked = serde_json::from_value::<ScheduleDef>(value).map_err(|e| e.to_string()).and_then(|def| {
            check_name(&def.name)?;
            if schedules.iter().any(|s| s.def.name == def.name) {
                return Err("the name is used twice".into());
            }
            let timing = def.when.parse()?;
            check_count("run at most", def.max_runs)?;
            check_then(&def.then)?;
            Ok(ScheduleItem { def, timing, raw })
        });
        match checked {
            Ok(item) => schedules.push(item),
            Err(reason) => skipped.push(Skip { section: Section::Schedule, index, name, reason }),
        }
    }

    // References, checked against the entries that passed on their own.
    let action_names: Vec<String> = actions.iter().map(|a| a.def.name.clone()).collect();
    let missing = |then: &[String]| then.iter().find(|n| !action_names.contains(n)).map(|n| format!("there is no action \"{n}\""));
    let mut keep = |section: Section, index: usize, name: &str, problem: Option<String>| match problem {
        Some(reason) => {
            skipped.push(Skip { section, index, name: Some(name.to_string()), reason });
            false
        }
        None => true,
    };
    let trigger_names: Vec<String> = triggers.iter().map(|t| t.def.name.clone()).collect();
    let schedule_names: Vec<String> = schedules.iter().map(|s| s.def.name.clone()).collect();
    let mut index = 0;
    triggers.retain(|t| {
        index += 1;
        keep(Section::Trigger, index - 1, &t.def.name, missing(&t.def.then))
    });
    index = 0;
    schedules.retain(|s| {
        index += 1;
        keep(Section::Schedule, index - 1, &s.def.name, missing(&s.def.then))
    });
    index = 0;
    actions.retain(|a| {
        index += 1;
        let target = a.builtin.as_ref().and_then(Builtin::target).and_then(|(section, name)| {
            let names = match section {
                Section::Trigger => &trigger_names,
                Section::Schedule => &schedule_names,
                Section::Action => &action_names,
            };
            (!names.iter().any(|n| n == name)).then(|| format!("there is no {} \"{name}\" for it to act on", section.word()))
        });
        keep(Section::Action, index - 1, &a.def.name, missing(&a.def.then).or(target))
    });
    Ok(Parsed { triggers, schedules, actions, skipped })
}

// ---- State, kept in jobs_state.json ---------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct TriggerState {
    /// The last reading (`{"text": ..., "number": ...}`) and when it was
    /// checked; the error of the last check, if it failed.
    pub last_value: Option<Value>,
    /// The reading before the last change, for `{previous...}` when the
    /// Trigger is fired by hand or its Action is run by hand.
    pub previous_value: Option<Value>,
    pub last_checked_ms: Option<i64>,
    pub last_error: Option<String>,
    /// Whether "is above", "is below" or "equals" held at the last reading.
    pub held: Option<bool>,
    /// False until the first reading after the Trigger was saved or
    /// switched on; that reading never fires.
    pub primed: bool,
    /// Fires since it was saved or switched on, and the last one.
    pub fires: u64,
    pub last_fired_ms: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct ScheduleState {
    pub runs: u64,
    pub last_run_ms: Option<i64>,
    /// When it was last saved or switched on. Runs due before then are not
    /// made up.
    pub since_ms: Option<i64>,
    /// Set by a "delay" Action: the next run is not before this time.
    pub delayed_until_ms: Option<i64>,
}

/// One finished run of an Action.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RunEntry {
    pub at_ms: i64,
    /// None when no job was started (a switch, or a start that failed).
    pub job: Option<u64>,
    pub state: JobState,
    pub label: Option<String>,
    pub message: Option<String>,
    /// "you", "trigger NAME", "schedule NAME" or "action NAME".
    pub by: String,
    /// The end of what the job printed or answered.
    pub output: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct ActionState {
    /// The newest runs, oldest first.
    pub history: Vec<RunEntry>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct StateText {
    version: u64,
    triggers: BTreeMap<String, TriggerState>,
    schedules: BTreeMap<String, ScheduleState>,
    actions: BTreeMap<String, ActionState>,
}

fn from_ms(ms: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000).unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

fn to_ms(at: OffsetDateTime) -> i64 {
    (at.unix_timestamp_nanos() / 1_000_000) as i64
}

fn seconds(n: u64) -> time::Duration {
    time::Duration::seconds(i64::try_from(n).unwrap_or(i64::MAX))
}

/// When a Schedule should next run, seen from `now`: None when it never
/// will (run limit reached, a `once` already done or set in the past). The
/// count starts from its last run, or from when it was saved or switched on
/// if that is later. A run that fell due while the app was closed is due at
/// once: one run, however many were missed.
pub fn next_due(item: &ScheduleItem, st: &ScheduleState, now: OffsetDateTime, offset_at: OffsetAt) -> Option<OffsetDateTime> {
    if item.def.max_runs.is_some_and(|max| st.runs >= max) {
        return None;
    }
    let base = st.last_run_ms.max(st.since_ms).map_or(now, from_ms);
    let next = next_after(item.timing, base, offset_at)?.max(now);
    Some(st.delayed_until_ms.map_or(next, |until| next.max(from_ms(until))))
}

/// Whether a Schedule's due run is one missed while the app was closed (it
/// was due before `now` and is being made up).
fn made_up(item: &ScheduleItem, st: &ScheduleState, now: OffsetDateTime, offset_at: OffsetAt) -> bool {
    let base = st.last_run_ms.max(st.since_ms).map_or(now, from_ms);
    next_after(item.timing, base, offset_at).is_some_and(|due| due < now - time::Duration::minutes(1))
}

/// When a Trigger should next check: at once if it never has, else its
/// interval after the last check.
fn next_check(item: &TriggerItem, st: &TriggerState, now: OffsetDateTime) -> OffsetDateTime {
    match st.last_checked_ms {
        Some(last) => (from_ms(last) + seconds(item.def.every_seconds)).max(now),
        None => now,
    }
}

// ---- What the Jobs page sees ---------------------------------------------------

#[derive(Serialize, Clone, Debug)]
pub struct TriggerView {
    pub name: String,
    pub enabled: bool,
    pub definition: Value,
    /// "bitmods.com/probe #probe-gbp"
    pub watching: String,
    /// "changes", "is above 5000"
    pub when: String,
    pub every_seconds: u64,
    pub max_fires: Option<u64>,
    pub then: Vec<String>,
    pub next_ms: Option<i64>,
    pub checking: bool,
    #[serde(flatten)]
    pub state: TriggerState,
}

#[derive(Serialize, Clone, Debug)]
pub struct ScheduleView {
    pub name: String,
    pub enabled: bool,
    pub definition: Value,
    pub when: String,
    pub max_runs: Option<u64>,
    pub then: Vec<String>,
    pub next_ms: Option<i64>,
    #[serde(flatten)]
    pub state: ScheduleState,
}

#[derive(Serialize, Clone, Debug)]
pub struct ActionView {
    pub name: String,
    pub enabled: bool,
    pub definition: Value,
    pub kind: String,
    /// One line about what it does.
    pub summary: String,
    pub then: Vec<String>,
    /// Its jobs that are queued or running.
    pub running: Vec<u64>,
    #[serde(flatten)]
    pub state: ActionState,
}

#[derive(Serialize, Clone, Debug)]
pub struct AutomationReply {
    pub status: FileStatus,
    pub path: String,
    pub triggers: Vec<TriggerView>,
    pub schedules: Vec<ScheduleView>,
    pub actions: Vec<ActionView>,
    pub skipped: Vec<String>,
    pub max_chain: u32,
    pub max_then: usize,
}

/// What "Test now" found.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TestReply {
    /// The reading, or why it could not be read.
    pub value: Option<Value>,
    pub error: Option<String>,
    /// Whether "is above", "is below" or "equals" holds for it.
    pub holds: Option<bool>,
    /// For "changes": whether it differs from the saved Trigger's last
    /// reading (None when there is none to compare with).
    pub changed: Option<bool>,
}

/// A job kind as the Action form needs it.
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

/// The built-in Action kinds, for the Action form's Kind list (after the
/// job kinds).
pub fn builtin_views() -> Vec<KindView> {
    BUILTIN_KINDS
        .iter()
        .map(|name| KindView { name, example: "{}", capabilities: Value::Array(Vec::new()), resources: &[] })
        .collect()
}

fn first_line(text: &str, max: usize) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    match line.char_indices().nth(max) {
        Some((cut, _)) => format!("{}...", &line[..cut]),
        None => line.to_string(),
    }
}

fn summary(item: &ActionItem) -> String {
    if let Some(b) = &item.builtin {
        return b.describe();
    }
    let field = |name: &str| item.def.input.get(name).and_then(Value::as_str).unwrap_or_default();
    match item.def.kind.as_str() {
        "notify" => format!("notification: {}", first_line(field("title"), 60)),
        "command" => format!("command: {}", first_line(field("script"), 60)),
        "ai" => format!("{}: {}", field("provider"), first_line(field("prompt"), 60)),
        other => other.to_string(),
    }
}

// ---- The engine ----------------------------------------------------------------

/// What started an Action.
#[derive(Clone, Debug)]
enum By {
    Person,
    Trigger(String),
    Schedule(String),
    Action(String),
}

impl By {
    fn origin(&self, action: &str) -> Origin {
        let action = action.to_string();
        match self {
            By::Person => Origin::Run { action },
            By::Trigger(trigger) => Origin::Trigger { action, trigger: trigger.clone() },
            By::Schedule(schedule) => Origin::Schedule { action, schedule: schedule.clone() },
            By::Action(after) => Origin::Action { action, after: after.clone() },
        }
    }

    fn text(&self) -> String {
        match self {
            By::Person => "you".into(),
            By::Trigger(name) => format!("trigger {name}"),
            By::Schedule(name) => format!("schedule {name}"),
            By::Action(name) => format!("action {name}"),
        }
    }
}

/// A request to start an Action.
struct Start {
    action: String,
    by: By,
    given: Arc<Given>,
    /// How many Actions in a row led to this one (0: started by a Trigger,
    /// a Schedule or a person).
    depth: u32,
    /// Started by a Schedule's run that was missed while the app was closed
    /// (quit and restart are then skipped, or the app would end at start).
    made_up: bool,
}

impl Start {
    /// Starts of the Actions in `then`, all given the same values.
    fn each(then: &[String], by: &By, given: Given, depth: u32) -> Vec<Start> {
        let given = Arc::new(given);
        then.iter().map(|action| Start { action: action.clone(), by: by.clone(), given: given.clone(), depth, made_up: false }).collect()
    }
}

struct Running {
    job: u64,
    depth: u32,
    given: Arc<Given>,
    by: String,
}

struct TriggerEntry {
    item: TriggerItem,
    next: Option<OffsetDateTime>,
    checking: bool,
}

struct ScheduleEntry {
    item: ScheduleItem,
    next: Option<OffsetDateTime>,
}

struct ActionEntry {
    item: ActionItem,
    running: Vec<Running>,
}

#[derive(Default)]
struct Inner {
    triggers: Vec<TriggerEntry>,
    schedules: Vec<ScheduleEntry>,
    actions: Vec<ActionEntry>,
    trigger_state: BTreeMap<String, TriggerState>,
    schedule_state: BTreeMap<String, ScheduleState>,
    action_state: BTreeMap<String, ActionState>,
    status: Option<FileStatus>,
    skipped: Vec<String>,
}

pub type NotifyUi = Arc<dyn Fn(&AutomationReply) + Send + Sync>;
/// Shows, hides, quits or restarts the app for an `app` Action (the app
/// passes one using its window and tray; tests record the calls).
pub type AppControl = Arc<dyn Fn(AppDo) -> Result<(), String> + Send + Sync>;
pub type Clock = Arc<dyn Fn() -> OffsetDateTime + Send + Sync>;

struct Shared {
    inner: Mutex<Inner>,
    wake: tokio::sync::Notify,
    checks: tokio::sync::Semaphore,
    queue: Queue,
    runtime: Handle,
    dir: PathBuf,
    notify: NotifyUi,
    app: AppControl,
    clock: Clock,
    offset_at: OffsetAt,
}

/// Cheap to clone; every clone is the same engine.
#[derive(Clone)]
pub struct Automation(Arc<Shared>);

fn remember(st: &mut ActionState, entry: RunEntry) {
    st.history.push(entry);
    let extra = st.history.len().saturating_sub(MAX_HISTORY);
    st.history.drain(..extra);
}

impl Automation {
    /// Loads jobs.json and jobs_state.json from `dir` (creating them when
    /// missing). Call `spawn` to start the timing.
    pub fn new(dir: PathBuf, queue: Queue, runtime: Handle, notify: NotifyUi, app: AppControl, clock: Clock, offset_at: OffsetAt) -> Self {
        let automation = Automation(Arc::new(Shared {
            inner: Mutex::new(Inner::default()),
            wake: tokio::sync::Notify::new(),
            checks: tokio::sync::Semaphore::new(MAX_CHECKS),
            queue,
            runtime,
            dir,
            notify,
            app,
            clock,
            offset_at,
        }));
        automation.load_state();
        automation.reload();
        automation
    }

    /// The app's engine: the real clock and the system's time zone.
    pub fn for_app(dir: PathBuf, queue: Queue, runtime: Handle, notify: NotifyUi, app: AppControl) -> Self {
        Self::new(dir, queue, runtime, notify, app, Arc::new(OffsetDateTime::now_utc), system_offset)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn now(&self) -> OffsetDateTime {
        (self.0.clock)()
    }

    fn file_path(&self) -> PathBuf {
        self.0.dir.join(FILE)
    }

    fn state_path(&self) -> PathBuf {
        self.0.dir.join(STATE_FILE)
    }

    fn load_state(&self) {
        let (status, text) = files::read_or_create(&self.state_path(), EMPTY_STATE, MAX_STATE_BYTES, "jobs_state");
        let file = match text.map(|t| serde_json::from_str::<StateText>(&t)) {
            Some(Ok(file)) => file,
            Some(Err(e)) => {
                warn!("jobs_state: not valid ({e}); readings and counts start again");
                StateText::default()
            }
            None => {
                warn!("jobs_state: could not be read ({status:?}); readings and counts start again");
                StateText::default()
            }
        };
        let mut inner = self.lock();
        inner.trigger_state = file.triggers;
        inner.schedule_state = file.schedules;
        inner.action_state = file.actions;
    }

    /// Writes jobs_state.json (atomically), keeping only entries that still
    /// exist.
    fn save_state(&self, inner: &Inner) {
        fn kept<'a, S: Clone + 'a>(names: impl Iterator<Item = &'a String>, map: &BTreeMap<String, S>) -> BTreeMap<String, S> {
            names.filter_map(|name| map.get(name).map(|st| (name.clone(), st.clone()))).collect()
        }
        let file = StateText {
            version: FILE_VERSION,
            triggers: kept(inner.triggers.iter().map(|e| &e.item.def.name), &inner.trigger_state),
            schedules: kept(inner.schedules.iter().map(|e| &e.item.def.name), &inner.schedule_state),
            actions: kept(inner.actions.iter().map(|e| &e.item.def.name), &inner.action_state),
        };
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

    /// A Jobs 1.0 file (version 1) is kept as `jobs.json.v1` and replaced by
    /// the new defaults.
    fn set_aside_old_file(&self) {
        let path = self.file_path();
        let Ok(text) = fs::read_to_string(&path) else { return };
        let version = serde_json::from_str::<Value>(&text).ok().and_then(|v| v.get("version").and_then(Value::as_u64));
        if version == Some(1) {
            let old = path.with_extension("json.v1");
            match fs::rename(&path, &old) {
                Ok(()) => warn!("jobs: jobs.json was a Jobs 1.0 file; kept as {} and replaced with the new defaults", old.display()),
                Err(e) => warn!("jobs: jobs.json is a Jobs 1.0 file and could not be set aside: {e}"),
            }
        }
    }

    /// Reads jobs.json again. Readings, counts and running jobs of entries
    /// that keep their name carry over.
    pub fn reload(&self) {
        self.set_aside_old_file();
        let (status, text) = files::read_or_create(&self.file_path(), DEFAULT_FILE, MAX_FILE_BYTES, "jobs");
        let parsed = text.map(|t| parse(&t, &self.0.queue));
        let now = self.now();
        let offset_at = self.0.offset_at;
        let mut inner = self.lock();
        inner.status = Some(status);
        inner.skipped.clear();
        let mut running: BTreeMap<String, Vec<Running>> =
            inner.actions.iter_mut().map(|a| (a.item.def.name.clone(), std::mem::take(&mut a.running))).collect();
        let checking: Vec<TriggerDef> = inner.triggers.iter().filter(|t| t.checking).map(|t| t.item.def.clone()).collect();
        match parsed {
            Some(Ok(Parsed { triggers, schedules, actions, skipped })) => {
                let skipped: Vec<String> = skipped.iter().map(Skip::text).collect();
                let counts = format!("{} triggers, {} schedules and {} actions", triggers.len(), schedules.len(), actions.len());
                if skipped.is_empty() {
                    info!("jobs: {counts} loaded");
                } else {
                    warn!("jobs: {counts} loaded, {} skipped: {}", skipped.len(), skipped.join("; "));
                }
                inner.skipped = skipped;
                let Inner { trigger_state, schedule_state, .. } = &mut *inner;
                let triggers = triggers
                    .into_iter()
                    .map(|item| {
                        let st = trigger_state.entry(item.def.name.clone()).or_default();
                        let next = item.def.enabled.then(|| next_check(&item, st, now));
                        let checking = checking.contains(&item.def);
                        TriggerEntry { item, next, checking }
                    })
                    .collect();
                let schedules = schedules
                    .into_iter()
                    .map(|item| {
                        let st = schedule_state.entry(item.def.name.clone()).or_default();
                        let next = next_due(&item, st, now, offset_at);
                        ScheduleEntry { item, next }
                    })
                    .collect();
                let actions = actions
                    .into_iter()
                    .map(|item| {
                        let running = running.remove(&item.def.name).unwrap_or_default();
                        ActionEntry { item, running }
                    })
                    .collect();
                inner.triggers = triggers;
                inner.schedules = schedules;
                inner.actions = actions;
            }
            Some(Err(e)) => {
                warn!("jobs: jobs.json {e}; nothing runs until it is fixed and reloaded");
                inner.status = Some(FileStatus::Invalid);
                inner.skipped = vec![e];
                inner.triggers.clear();
                inner.schedules.clear();
                inner.actions.clear();
            }
            None => {
                inner.triggers.clear();
                inner.schedules.clear();
                inner.actions.clear();
            }
        }
        if !running.is_empty() {
            let left: usize = running.values().map(Vec::len).sum();
            if left > 0 {
                info!("jobs: {left} running jobs belong to actions that are gone; they finish on their own");
            }
        }
        self.publish(&inner);
        drop(inner);
        self.0.wake.notify_one();
    }

    fn reply(&self, inner: &Inner) -> AutomationReply {
        AutomationReply {
            status: inner.status.unwrap_or(FileStatus::Ok),
            path: self.file_path().display().to_string(),
            triggers: inner
                .triggers
                .iter()
                .map(|e| {
                    let d = &e.item.def;
                    TriggerView {
                        name: d.name.clone(),
                        enabled: d.enabled,
                        definition: e.item.raw.clone(),
                        watching: d.watch.describe(),
                        when: d.when.describe(),
                        every_seconds: d.every_seconds,
                        max_fires: d.max_fires,
                        then: d.then.clone(),
                        next_ms: e.next.map(to_ms),
                        checking: e.checking,
                        state: inner.trigger_state.get(&d.name).cloned().unwrap_or_default(),
                    }
                })
                .collect(),
            schedules: inner
                .schedules
                .iter()
                .map(|e| {
                    let d = &e.item.def;
                    ScheduleView {
                        name: d.name.clone(),
                        enabled: d.enabled,
                        definition: e.item.raw.clone(),
                        when: e.item.timing.describe(),
                        max_runs: d.max_runs,
                        then: d.then.clone(),
                        next_ms: e.next.filter(|_| d.enabled).map(to_ms),
                        state: inner.schedule_state.get(&d.name).cloned().unwrap_or_default(),
                    }
                })
                .collect(),
            actions: inner
                .actions
                .iter()
                .map(|e| {
                    let d = &e.item.def;
                    ActionView {
                        name: d.name.clone(),
                        enabled: d.enabled,
                        definition: e.item.raw.clone(),
                        kind: d.kind.clone(),
                        summary: summary(&e.item),
                        then: d.then.clone(),
                        running: e.running.iter().map(|r| r.job).collect(),
                        state: inner.action_state.get(&d.name).cloned().unwrap_or_default(),
                    }
                })
                .collect(),
            skipped: inner.skipped.clone(),
            max_chain: MAX_CHAIN,
            max_then: MAX_THEN,
        }
    }

    fn publish(&self, inner: &Inner) {
        (self.0.notify)(&self.reply(inner));
    }

    pub fn list(&self) -> AutomationReply {
        self.reply(&self.lock())
    }

    // ---- Starting Actions ----

    /// The Run button: starts an Action now, whether it is on or off.
    /// Returns its job id (None for a switch, which has no job). It is
    /// given the latest reading (and the one before the last change) of the
    /// first Trigger that starts it, so `{value.text}` is not empty.
    pub fn run(&self, name: &str) -> Result<Option<u64>, String> {
        let given = {
            let inner = self.lock();
            if !inner.actions.iter().any(|a| a.item.def.name == name) {
                return Err(format!("there is no action \"{name}\""));
            }
            let source = inner.triggers.iter().find(|t| t.item.def.then.iter().any(|n| n == name));
            let st = source.and_then(|t| inner.trigger_state.get(&t.item.def.name));
            Given {
                from: By::Person.text(),
                value: st.and_then(|st| st.last_value.clone()),
                previous: st.and_then(|st| st.previous_value.clone()),
                output: String::new(),
            }
        };
        let start = Start { action: name.to_string(), by: By::Person, given: Arc::new(given), depth: 0, made_up: false };
        self.run_starts(vec![start]).map_err(|e| e.unwrap_or_default())
    }

    /// Run now on a Schedule: starts its Actions at once, whether it is on
    /// or off. Its timetable and run count are not changed.
    pub fn run_schedule(&self, name: &str) -> Result<usize, String> {
        let then = self
            .lock()
            .schedules
            .iter()
            .find(|s| s.item.def.name == name)
            .map(|s| s.item.def.then.clone())
            .ok_or(format!("there is no schedule \"{name}\""))?;
        if then.is_empty() {
            return Err(format!("schedule \"{name}\" starts no actions yet"));
        }
        info!("schedule {name}: run by hand, starting {}", then.join(", "));
        let by = By::Schedule(name.to_string());
        let given = Given { from: by.text(), ..Given::default() };
        let _ = self.run_starts(Start::each(&then, &by, given, 0));
        Ok(then.len())
    }

    /// Starts Actions, and the Actions that `switch` Actions start after
    /// them. Returns the first start's job id, or why it did not start.
    fn run_starts(&self, starts: Vec<Start>) -> Result<Option<u64>, Option<String>> {
        let mut work: VecDeque<Start> = starts.into();
        let mut first: Option<Result<Option<u64>, Option<String>>> = None;
        while let Some(start) = work.pop_front() {
            let result = self.start_one(start, &mut work);
            first.get_or_insert(result);
        }
        first.unwrap_or(Ok(None))
    }

    fn start_one(&self, start: Start, work: &mut VecDeque<Start>) -> Result<Option<u64>, Option<String>> {
        let now = self.now();
        let mut inner = self.lock();
        let Some(index) = inner.actions.iter().position(|a| a.item.def.name == start.action) else {
            warn!("{}: there is no action \"{}\" to start", start.by.text(), start.action);
            return Err(Some(format!("there is no action \"{}\"", start.action)));
        };
        let item = inner.actions[index].item.clone();
        let name = item.def.name.clone();
        let by = start.by.text();
        let refused = if start.depth >= MAX_CHAIN {
            Some(format!("not started: a chain of {MAX_CHAIN} actions was reached (loop guard)"))
        } else if inner.actions[index].running.len() >= MAX_RUNNING {
            Some(format!("not started: {MAX_RUNNING} of its jobs are already queued or running"))
        } else {
            None
        };
        if let Some(reason) = refused {
            warn!("action {name} ({by}): {reason}");
            let entry = RunEntry { at_ms: to_ms(now), job: None, state: JobState::Failed, label: None, message: Some(reason.clone()), by, output: None };
            remember(inner.action_state.entry(name).or_default(), entry);
            self.save_state(&inner);
            self.publish(&inner);
            return Err(Some(reason));
        }
        if !item.def.enabled && !matches!(start.by, By::Person) {
            info!("action {name} is off; not started by {by}");
            return Err(Some(format!("action \"{name}\" is off")));
        }

        if let Some(builtin) = item.builtin {
            drop(inner);
            let quits = matches!(builtin, Builtin::App(AppDo::Quit | AppDo::Restart));
            let result = if quits && start.made_up {
                Err("not done: it was a run made up after the app was closed, and it would end the app as it starts".to_string())
            } else if quits {
                // Recorded first: the app ends with this call.
                Ok(builtin.describe())
            } else {
                self.run_builtin(&builtin, &start.given)
            };
            let (state, message) = match &result {
                Ok(done) => (JobState::Done, done.clone()),
                Err(e) => (JobState::Failed, e.clone()),
            };
            info!("action {name} ({by}): {message}");
            let mut inner = self.lock();
            let entry = RunEntry { at_ms: to_ms(now), job: None, state, label: None, message: Some(message.clone()), by, output: None };
            remember(inner.action_state.entry(name.clone()).or_default(), entry);
            self.save_state(&inner);
            self.publish(&inner);
            drop(inner);
            if result.is_err() {
                return Err(Some(message));
            }
            if quits {
                if let Builtin::App(what) = builtin {
                    if let Err(e) = (self.0.app)(what) {
                        warn!("action {name}: {e}");
                    }
                }
                return Ok(None);
            }
            let given = Given { from: By::Action(name.clone()).text(), output: String::new(), ..(*start.given).clone() };
            work.extend(Start::each(&item.def.then, &By::Action(name), given, start.depth + 1));
            return Ok(None);
        }

        let origin = start.by.origin(&name);
        match self.0.queue.start_given(&item.def.kind, item.def.input.clone(), origin, item.def.timeout_ms, Some(start.given.clone())) {
            Ok(job) => {
                info!("action {name} ({by}): job {job}");
                inner.actions[index].running.push(Running { job, depth: start.depth, given: start.given, by });
                self.publish(&inner);
                drop(inner);
                self.0.wake.notify_one();
                Ok(Some(job))
            }
            Err(e) => {
                warn!("action {name} ({by}): could not start: {e}");
                let entry = RunEntry { at_ms: to_ms(now), job: None, state: JobState::Failed, label: None, message: Some(e.clone()), by, output: None };
                remember(inner.action_state.entry(name).or_default(), entry);
                self.save_state(&inner);
                self.publish(&inner);
                Err(Some(e))
            }
        }
    }

    /// Carries out a built-in Action (not quit or restart, which
    /// `start_one` handles). Returns what was done, in words.
    fn run_builtin(&self, builtin: &Builtin, given: &Given) -> Result<String, String> {
        match builtin {
            Builtin::Switch(s) => self.set_enabled(s.section, &s.name, s.enabled).map(|()| builtin.describe().replacen("switch", "switched", 1)),
            Builtin::Delay { schedule, by } => self.delay_schedule(schedule, *by),
            Builtin::SetCount { section, name, count } => self.set_count(*section, name, *count).map(|()| builtin.describe()),
            Builtin::Delete { section, name } => self.delete(*section, name).map(|()| format!("deleted {} \"{name}\"", section.word())),
            Builtin::Fire { trigger } => {
                if !self.lock().triggers.iter().any(|t| t.item.def.name == *trigger) {
                    return Err(format!("there is no trigger \"{trigger}\""));
                }
                let me = self.clone();
                let trigger = trigger.clone();
                self.0.runtime.spawn(async move {
                    if let Err(e) = me.fire_trigger(&trigger).await {
                        warn!("trigger {trigger}: fired by an action, but {e}");
                    }
                });
                Ok(builtin.describe().replacen("fire", "firing", 1))
            }
            Builtin::Log { level, text } => {
                let text = crate::job_kinds::fill_text(text, Some(given), 2000);
                match level {
                    LogLevel::Info => info!(target: "jobs", "{text}"),
                    LogLevel::Warn => warn!(target: "jobs", "{text}"),
                    LogLevel::Error => tracing::error!(target: "jobs", "{text}"),
                }
                Ok(format!("logged: {text}"))
            }
            Builtin::ClearErrors => {
                crate::logging::clear();
                Ok("cleared the Errors list".into())
            }
            Builtin::App(what) => (self.0.app)(*what).map(|()| builtin.describe()),
        }
    }

    /// Moves a Schedule's next run later by `by` (a "delay" Action). The run
    /// after it follows the timetable again.
    pub fn delay_schedule(&self, name: &str, by: Duration) -> Result<String, String> {
        let now = self.now();
        let offset_at = self.0.offset_at;
        let mut inner = self.lock();
        let Inner { schedules, schedule_state, .. } = &mut *inner;
        let entry = schedules.iter_mut().find(|s| s.item.def.name == name).ok_or(format!("there is no schedule \"{name}\""))?;
        let st = schedule_state.entry(name.to_string()).or_default();
        let next = next_due(&entry.item, st, now, offset_at).ok_or(format!("schedule \"{name}\" has no next run to delay"))?;
        let until = next + time::Duration::try_from(by).map_err(|e| e.to_string())?;
        st.delayed_until_ms = Some(to_ms(until));
        entry.next = Some(until);
        info!("schedule {name}: next run delayed to {until}");
        self.save_state(&inner);
        self.publish(&inner);
        drop(inner);
        self.0.wake.notify_one();
        Ok(format!("delayed schedule \"{name}\" by {}", duration_words(by.as_secs())))
    }

    /// Sets a Trigger's "fire at most" or a Schedule's "run at most" (None:
    /// no limit) in jobs.json.
    pub fn set_count(&self, section: Section, name: &str, count: Option<u64>) -> Result<(), String> {
        let field = match section {
            Section::Trigger => "max_fires",
            Section::Schedule => "max_runs",
            Section::Action => return Err("an action has no count".into()),
        };
        self.edit_file(Some((section, name)), |file| {
            let list = Self::list_mut(file, section)?;
            let entry = list.iter_mut().find(|v| Self::name_in(v) == Some(name)).ok_or(format!("there is no {} \"{name}\"", section.word()))?;
            let object = entry.as_object_mut().ok_or("not an object")?;
            match count {
                Some(n) => object.insert(field.into(), Value::from(n)),
                None => object.remove(field),
            };
            Ok(())
        })?;
        info!("{} {name}: {field} set to {count:?}", section.word());
        self.reload();
        Ok(())
    }

    // ---- Triggers ----

    /// Reads a Trigger's watch now and records the result: the Jobs page
    /// list shows it, and the Trigger fires if its condition became true.
    async fn check_trigger(&self, def: TriggerDef) {
        let result = match self.0.checks.acquire().await {
            Ok(_permit) => def.watch.read().await,
            Err(_) => Err("checks are closed".into()),
        };
        self.record_check(&def, result);
    }

    /// Records a reading (or the reason there is none) for the Trigger
    /// defined as `def`. Ignored when the Trigger was changed or deleted
    /// while it was being read.
    fn record_check(&self, def: &TriggerDef, result: Result<Value, String>) {
        self.record(def, result, false);
    }

    /// `record_check`; with `forced` (Fire now) a successful reading starts
    /// the Actions whatever the condition, and does not count towards "fire
    /// at most".
    fn record(&self, def: &TriggerDef, result: Result<Value, String>, forced: bool) {
        let now = self.now();
        let mut inner = self.lock();
        let Inner { triggers, trigger_state, .. } = &mut *inner;
        let Some(entry) = triggers.iter_mut().find(|t| t.item.def.name == def.name) else { return };
        entry.checking = false;
        if entry.item.def != *def {
            drop(inner);
            self.0.wake.notify_one();
            return;
        }
        let name = def.name.clone();
        let st = trigger_state.entry(name.clone()).or_default();
        st.last_checked_ms = Some(to_ms(now));
        entry.next = Some(now + seconds(def.every_seconds));
        let mut starts = Vec::new();
        let mut switch_off = false;
        match result {
            Err(e) => {
                if st.last_error.as_ref() != Some(&e) {
                    warn!("trigger {name}: {e}");
                }
                st.last_error = Some(e);
            }
            Ok(reading) => {
                st.last_error = None;
                let before = st.last_value.replace(reading.clone());
                let changed = before.as_ref().is_some_and(|b| !same_reading(b, &reading));
                if changed {
                    st.previous_value = before.clone();
                }
                let holds = def.when.holds(&reading);
                let fire = forced
                    || st.primed
                        && match holds {
                            Some(now_true) => now_true && st.held != Some(true),
                            None => changed,
                        };
                st.primed = true;
                st.held = holds;
                if fire {
                    st.last_fired_ms = Some(to_ms(now));
                    // Fired by hand on an unchanged page: "before" is the
                    // same reading, so the one before the last change is used.
                    let previous = if changed { before } else { st.previous_value.clone() };
                    if forced {
                        info!("trigger {name}: fired by hand ({}), starting {}", text_of(&reading), def.then.join(", "));
                    } else {
                        st.fires += 1;
                        info!("trigger {name}: fired ({} {}), starting {}", text_of(&reading), def.when.describe(), def.then.join(", "));
                    }
                    let by = By::Trigger(name.clone());
                    let given = Given { from: by.text(), value: Some(reading), previous, output: String::new() };
                    starts = Start::each(&def.then, &by, given, 0);
                    if !forced && def.max_fires.is_some_and(|max| st.fires >= max) {
                        info!("trigger {name}: fired {} times, the most it may; switching it off", st.fires);
                        switch_off = true;
                    }
                }
            }
        }
        self.save_state(&inner);
        self.publish(&inner);
        drop(inner);
        if switch_off {
            if let Err(e) = self.set_enabled(Section::Trigger, &name, false) {
                warn!("trigger {name}: could not be switched off: {e}");
            }
        }
        let _ = self.run_starts(starts);
        self.0.wake.notify_one();
    }

    /// Fire now: reads the Trigger's page at once and starts its Actions
    /// with that reading, whatever its condition, on or off. Returns the
    /// reading, or why there is none.
    pub async fn fire_trigger(&self, name: &str) -> Result<Value, String> {
        let def = self
            .lock()
            .triggers
            .iter()
            .find(|t| t.item.def.name == name)
            .map(|t| t.item.def.clone())
            .ok_or(format!("there is no trigger \"{name}\""))?;
        let result = match self.0.checks.acquire().await {
            Ok(_permit) => def.watch.read().await,
            Err(_) => Err("checks are closed".into()),
        };
        self.record(&def, result.clone(), true);
        result
    }

    /// "Test now" on the Trigger form: reads the watch of a Trigger as
    /// typed (saved or not) and says whether its condition holds.
    pub async fn test_trigger(&self, definition: Value) -> Result<TestReply, String> {
        let mut definition = definition;
        if let Some(object) = definition.as_object_mut() {
            // A Trigger can be tested before it has a name or Actions.
            object.insert("name".into(), Value::String(String::new()));
            object.remove("then");
        }
        let def: TriggerDef = serde_json::from_value(definition).map_err(|e| e.to_string())?;
        def.watch.check()?;
        def.when.check(def.watch.reads_numbers())?;
        Ok(match def.watch.read().await {
            Err(e) => TestReply { value: None, error: Some(e), holds: None, changed: None },
            Ok(reading) => {
                let holds = def.when.holds(&reading);
                TestReply { value: Some(reading), error: None, holds, changed: None }
            }
        })
    }

    /// `test_trigger`, comparing a "changes" Trigger with the last reading
    /// of the saved Trigger called `name`.
    pub async fn test_trigger_named(&self, name: Option<String>, definition: Value) -> Result<TestReply, String> {
        let last = name.and_then(|n| self.lock().trigger_state.get(&n).and_then(|st| st.last_value.clone()));
        let mut reply = self.test_trigger(definition).await?;
        if reply.holds.is_none() {
            if let (Some(value), Some(last)) = (&reply.value, &last) {
                reply.changed = Some(!same_reading(last, value));
            }
        }
        Ok(reply)
    }

    // ---- Timing ----

    /// One pass: records finished jobs and starts the Actions after them,
    /// starts Schedules that are due, starts checks of Triggers that are
    /// due, and returns how long to sleep before the next pass.
    pub fn tick(&self) -> Duration {
        let now = self.now();
        let offset_at = self.0.offset_at;
        let mut starts: Vec<Start> = Vec::new();
        let mut checks: Vec<TriggerDef> = Vec::new();
        let mut inner = self.lock();
        let mut changed = false;
        let Inner { triggers, schedules, actions, schedule_state, action_state, .. } = &mut *inner;

        for entry in actions.iter_mut() {
            let name = entry.item.def.name.clone();
            let mut still = Vec::new();
            for run in std::mem::take(&mut entry.running) {
                match self.0.queue.outcome(run.job) {
                    Some((state, _)) if !state.finished() => still.push(run),
                    Some((state, outcome)) => {
                        changed = true;
                        let output = self.0.queue.output_tail(run.job, HISTORY_OUTPUT_BYTES);
                        let entry_ = RunEntry {
                            at_ms: to_ms(now),
                            job: Some(run.job),
                            state,
                            label: outcome.label,
                            message: outcome.message,
                            by: run.by,
                            output,
                        };
                        remember(action_state.entry(name.clone()).or_default(), entry_);
                        if state == JobState::Done && !entry.item.def.then.is_empty() {
                            let by = By::Action(name.clone());
                            let given = Given {
                                from: by.text(),
                                value: run.given.value.clone(),
                                previous: run.given.previous.clone(),
                                output: self.0.queue.output_tail(run.job, MAX_OUTPUT_BYTES).unwrap_or_default(),
                            };
                            starts.extend(Start::each(&entry.item.def.then, &by, given, run.depth + 1));
                        }
                    }
                    None => changed = true, // no longer listed by the queue
                }
            }
            entry.running = still;
        }

        for entry in schedules.iter_mut() {
            if !entry.item.def.enabled || !entry.next.is_some_and(|due| due <= now) {
                continue;
            }
            let name = entry.item.def.name.clone();
            let st = schedule_state.entry(name.clone()).or_default();
            let missed = made_up(&entry.item, st, now, offset_at);
            st.runs += 1;
            st.last_run_ms = Some(to_ms(now));
            st.delayed_until_ms = None;
            entry.next = next_due(&entry.item, st, now, offset_at);
            info!("schedule {name}: {}, starting {}", if missed { "made up a run missed while the app was closed" } else { "due" }, entry.item.def.then.join(", "));
            let by = By::Schedule(name);
            let given = Given { from: by.text(), ..Given::default() };
            starts.extend(Start::each(&entry.item.def.then, &by, given, 0).into_iter().map(|s| Start { made_up: missed, ..s }));
            changed = true;
        }

        for entry in triggers.iter_mut() {
            if entry.item.def.enabled && !entry.checking && entry.next.is_some_and(|due| due <= now) {
                entry.checking = true;
                checks.push(entry.item.def.clone());
                changed = true;
            }
        }

        if changed {
            self.save_state(&inner);
            self.publish(&inner);
        }
        let mut sleep = MAX_SLEEP;
        if inner.actions.iter().any(|a| !a.running.is_empty()) {
            sleep = sleep.min(RESULT_POLL);
        }
        let dues = inner
            .schedules
            .iter()
            .filter(|s| s.item.def.enabled)
            .filter_map(|s| s.next)
            .chain(inner.triggers.iter().filter(|t| t.item.def.enabled && !t.checking).filter_map(|t| t.next));
        for due in dues {
            sleep = sleep.min((due - now).try_into().unwrap_or(Duration::ZERO));
        }
        drop(inner);

        for def in checks {
            let me = self.clone();
            self.0.runtime.spawn(async move { me.check_trigger(def).await });
        }
        let _ = self.run_starts(starts);
        sleep
    }

    /// Runs `tick` for as long as the app runs, on Tauri's Tokio runtime.
    pub fn spawn(&self) {
        let me = self.clone();
        self.0.runtime.spawn(async move {
            loop {
                let sleep = me.tick();
                tokio::select! {
                    _ = tokio::time::sleep(sleep) => {}
                    _ = me.0.wake.notified() => {}
                }
            }
        });
    }

    // ---- Changing jobs.json ----

    /// Changes jobs.json: reads it, lets `change` edit it, checks the result
    /// and writes it atomically. `entry` is the item being saved: if the
    /// check refuses it, nothing is written. A file that is not valid JSON
    /// is never rewritten (a hand edit would be lost).
    fn edit_file(&self, entry: Option<(Section, &str)>, change: impl FnOnce(&mut Map<String, Value>) -> Result<(), String>) -> Result<(), String> {
        let path = self.file_path();
        let text = fs::read_to_string(&path).map_err(|e| format!("jobs.json could not be read: {e}"))?;
        let mut file: Value = serde_json::from_str(&text)
            .map_err(|e| format!("jobs.json is not valid JSON ({e}); fix it by hand and press Reload first"))?;
        change(file.as_object_mut().ok_or("jobs.json is not a JSON object")?)?;
        let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())? + "\n";
        if json.len() as u64 > MAX_FILE_BYTES {
            return Err(format!("jobs.json would be over {MAX_FILE_BYTES} bytes"));
        }
        let parsed = parse(&json, &self.0.queue)?;
        if let Some((section, name)) = entry {
            if let Some(skip) = parsed.skipped.iter().find(|s| s.section == section && s.name.as_deref() == Some(name)) {
                return Err(skip.reason.clone());
            }
        }
        let tmp = path.with_extension("json.tmp");
        files::write_new(&tmp, json).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }

    fn list_mut<'a>(file: &'a mut Map<String, Value>, section: Section) -> Result<&'a mut Vec<Value>, String> {
        let key = section.key();
        file.entry(key).or_insert_with(|| Value::Array(Vec::new())).as_array_mut().ok_or(format!("\"{key}\" is not a list"))
    }

    fn name_in(value: &Value) -> Option<&str> {
        value.get("name").and_then(Value::as_str)
    }

    /// Every `then` list in the file.
    fn then_lists(file: &mut Map<String, Value>) -> impl Iterator<Item = &mut Vec<Value>> {
        file.iter_mut()
            .filter(|(key, _)| ["triggers", "schedules", "actions"].contains(&key.as_str()))
            .filter_map(|(_, list)| list.as_array_mut())
            .flat_map(|list| list.iter_mut())
            .filter_map(|item| item.get_mut("then").and_then(Value::as_array_mut))
    }

    /// Every built-in Action in the file that acts on `section` `name`.
    fn acting_on<'a>(file: &'a mut Map<String, Value>, section: Section, name: &'a str) -> impl Iterator<Item = &'a mut Value> {
        file.get_mut("actions")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flat_map(|list| list.iter_mut())
            .filter(move |a| raw_targets(a, section, name))
    }

    /// The state of a saved or switched-on entry starts afresh: a Trigger's
    /// next reading only records the value, and a Schedule makes up no runs
    /// from before now.
    fn restart_state(&self, section: Section, name: &str) {
        let now = self.now();
        let mut inner = self.lock();
        match section {
            Section::Trigger => {
                let st = inner.trigger_state.entry(name.to_string()).or_default();
                st.primed = false;
                st.held = None;
                st.fires = 0;
                st.last_checked_ms = None;
            }
            Section::Schedule => inner.schedule_state.entry(name.to_string()).or_default().since_ms = Some(to_ms(now)),
            Section::Action => {}
        }
    }

    /// Drops what is kept about a deleted entry (reading, counts, results),
    /// so a new entry with the same name starts afresh.
    fn forget(&self, section: Section, name: &str) {
        let mut inner = self.lock();
        match section {
            Section::Trigger => drop(inner.trigger_state.remove(name)),
            Section::Schedule => drop(inner.schedule_state.remove(name)),
            Section::Action => drop(inner.action_state.remove(name)),
        }
    }

    /// Adds an entry (`old_name` None) or replaces the one called
    /// `old_name`. A rename follows into every `then` list and `switch`
    /// Action. Refused, with nothing written, if it is not valid.
    pub fn save(&self, section: Section, old_name: Option<String>, definition: Value) -> Result<(), String> {
        let name = Self::name_in(&definition).ok_or(format!("the {} needs a name", section.word()))?.to_string();
        check_name(&name)?;
        let renamed = old_name.as_deref().filter(|old| *old != name).map(str::to_string);
        self.edit_file(Some((section, &name)), |file| {
            let list = Self::list_mut(file, section)?;
            let at = match old_name.as_deref() {
                Some(old) => Some(list.iter().position(|v| Self::name_in(v) == Some(old)).ok_or(format!("there is no {} \"{old}\"", section.word()))?),
                None => None,
            };
            if list.iter().enumerate().any(|(i, v)| Some(i) != at && Self::name_in(v) == Some(&name)) {
                return Err(format!("a {} called \"{name}\" already exists", section.word()));
            }
            match at {
                Some(i) => list[i] = definition,
                None => list.push(definition),
            }
            if let Some(old) = &renamed {
                if section == Section::Action {
                    for then in Self::then_lists(file) {
                        for slot in then.iter_mut().filter(|s| s.as_str() == Some(old)) {
                            *slot = Value::String(name.clone());
                        }
                    }
                }
                for action in Self::acting_on(file, section, old) {
                    let kind = action.get("kind").and_then(Value::as_str).unwrap_or_default().to_string();
                    if let Some((_, field)) = target_field(&kind) {
                        action["input"][field] = Value::String(name.clone());
                    }
                }
            }
            Ok(())
        })?;
        if let Some(old) = &renamed {
            let mut inner = self.lock();
            match section {
                Section::Trigger => {
                    if let Some(st) = inner.trigger_state.remove(old) {
                        inner.trigger_state.insert(name.clone(), st);
                    }
                }
                Section::Schedule => {
                    if let Some(st) = inner.schedule_state.remove(old) {
                        inner.schedule_state.insert(name.clone(), st);
                    }
                }
                Section::Action => {
                    if let Some(st) = inner.action_state.remove(old) {
                        inner.action_state.insert(name.clone(), st);
                    }
                    if let Some(entry) = inner.actions.iter_mut().find(|a| a.item.def.name == *old) {
                        entry.item.def.name = name.clone();
                    }
                }
            }
        }
        self.restart_state(section, &name);
        info!("{} {name}: saved from the Jobs page", section.word());
        self.reload();
        self.save_state(&self.lock());
        Ok(())
    }

    /// Deletes an entry. A deleted Action is also taken out of every `then`
    /// list. Refused while a `switch` Action switches it (change or delete
    /// that Action first).
    pub fn delete(&self, section: Section, name: &str) -> Result<(), String> {
        let mut gone: Vec<String> = Vec::new();
        self.edit_file(None, |file| {
            let is_delete = |a: &Value| a.get("kind").and_then(Value::as_str) == Some("delete");
            let actions: Vec<Value> = file.get("actions").and_then(Value::as_array).cloned().unwrap_or_default();
            let users: Vec<String> = actions
                .iter()
                .filter(|a| raw_targets(a, section, name) && !is_delete(a))
                .filter(|a| !(section == Section::Action && Self::name_in(a) == Some(name)))
                .filter_map(|a| Self::name_in(a).map(|n| format!("\"{n}\"")))
                .collect();
            if !users.is_empty() {
                return Err(format!("action {} acts on it; change or delete that first", users.join(", ")));
            }
            let list = Self::list_mut(file, section)?;
            let before = list.len();
            list.retain(|v| Self::name_in(v) != Some(name));
            if list.len() == before {
                return Err(format!("there is no {} \"{name}\"", section.word()));
            }
            // "Delete" Actions aimed at it have done their job: they go too.
            gone = actions
                .iter()
                .filter(|a| raw_targets(a, section, name) && is_delete(a))
                .filter_map(|a| Self::name_in(a).map(str::to_string))
                .collect();
            if section == Section::Action {
                gone.push(name.to_string());
            }
            let actions = Self::list_mut(file, Section::Action)?;
            actions.retain(|a| !(section != Section::Action && Self::name_in(a).is_some_and(|n| gone.iter().any(|g| g == n))));
            for then in Self::then_lists(file) {
                then.retain(|s| !s.as_str().is_some_and(|n| gone.iter().any(|g| g == n)));
            }
            Ok(())
        })?;
        self.forget(section, name);
        for action in gone.iter().filter(|a| section != Section::Action || *a != name) {
            self.forget(Section::Action, action);
        }
        info!("{} {name}: deleted", section.word());
        self.reload();
        self.save_state(&self.lock());
        Ok(())
    }

    /// Copies an entry as "NAME copy" (or "NAME copy 2" ...). A copied
    /// Trigger or Schedule starts switched off, so it does not fire twice
    /// alongside the original. Returns the new name.
    pub fn duplicate(&self, section: Section, name: &str) -> Result<String, String> {
        let (mut definition, taken): (Value, Vec<String>) = {
            let inner = self.lock();
            let (raw, names): (Option<Value>, Vec<String>) = match section {
                Section::Trigger => (
                    inner.triggers.iter().find(|e| e.item.def.name == name).map(|e| e.item.raw.clone()),
                    inner.triggers.iter().map(|e| e.item.def.name.clone()).collect(),
                ),
                Section::Schedule => (
                    inner.schedules.iter().find(|e| e.item.def.name == name).map(|e| e.item.raw.clone()),
                    inner.schedules.iter().map(|e| e.item.def.name.clone()).collect(),
                ),
                Section::Action => (
                    inner.actions.iter().find(|e| e.item.def.name == name).map(|e| e.item.raw.clone()),
                    inner.actions.iter().map(|e| e.item.def.name.clone()).collect(),
                ),
            };
            (raw.ok_or(format!("there is no {} \"{name}\"", section.word()))?, names)
        };
        let base: String = name.chars().take(MAX_NAME_CHARS - " copy 999".len()).collect();
        let base = base.trim_end();
        let copy = (1..1000)
            .map(|n| if n == 1 { format!("{base} copy") } else { format!("{base} copy {n}") })
            .find(|candidate| !taken.contains(candidate))
            .ok_or("too many copies")?;
        let object = definition.as_object_mut().ok_or("not an object")?;
        object.insert("name".into(), Value::String(copy.clone()));
        if section != Section::Action {
            object.insert("enabled".into(), Value::Bool(false));
        }
        self.save(section, None, definition)?;
        Ok(copy)
    }

    /// The Overview's quick buttons. `EnableAll` and `DisableAll` switch every
    /// Trigger and Schedule (Actions are only started by them, or by Run, so
    /// they are left as they are). `DeleteAll` empties all three sections.
    /// `RestoreDemos` adds back the demonstration entries from the built-in
    /// defaults that are missing by name; entries with those names are left
    /// alone. Returns how many entries changed.
    pub fn bulk(&self, what: Bulk) -> Result<usize, String> {
        let mut changed = 0;
        let mut switched_on: Vec<(Section, String)> = Vec::new();
        self.edit_file(None, |file| {
            match what {
                Bulk::EnableAll | Bulk::DisableAll => {
                    let on = what == Bulk::EnableAll;
                    for section in [Section::Trigger, Section::Schedule] {
                        for item in Self::list_mut(file, section)?.iter_mut() {
                            let Some(object) = item.as_object_mut() else { continue };
                            let was = object.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                            if was != on {
                                object.insert("enabled".into(), Value::Bool(on));
                                changed += 1;
                                if on {
                                    if let Some(name) = object.get("name").and_then(Value::as_str) {
                                        switched_on.push((section, name.to_string()));
                                    }
                                }
                            }
                        }
                    }
                }
                Bulk::DeleteAll => {
                    for section in [Section::Trigger, Section::Schedule, Section::Action] {
                        let list = Self::list_mut(file, section)?;
                        changed += list.len();
                        list.clear();
                    }
                }
                Bulk::RestoreDemos => {
                    let defaults: Value = serde_json::from_str(DEFAULT_FILE).map_err(|e| e.to_string())?;
                    for section in [Section::Trigger, Section::Schedule, Section::Action] {
                        let demos = defaults.get(section.key()).and_then(Value::as_array).cloned().unwrap_or_default();
                        let list = Self::list_mut(file, section)?;
                        for demo in demos {
                            let name = Self::name_in(&demo).map(str::to_string);
                            if !list.iter().any(|v| Self::name_in(v).map(str::to_string) == name) {
                                list.push(demo);
                                changed += 1;
                            }
                        }
                    }
                }
            }
            Ok(())
        })?;
        for (section, name) in &switched_on {
            self.restart_state(*section, name);
        }
        if what == Bulk::DeleteAll {
            let mut inner = self.lock();
            inner.trigger_state.clear();
            inner.schedule_state.clear();
            inner.action_state.clear();
        }
        info!("jobs: {what:?} from the Overview, {changed} changed");
        self.reload();
        self.save_state(&self.lock());
        Ok(changed)
    }

    /// Switches an entry on or off in jobs.json. Switching on starts it
    /// afresh (see `restart_state`).
    pub fn set_enabled(&self, section: Section, name: &str, enabled: bool) -> Result<(), String> {
        self.edit_file(Some((section, name)), |file| {
            let list = Self::list_mut(file, section)?;
            let entry = list.iter_mut().find(|v| Self::name_in(v) == Some(name)).ok_or(format!("there is no {} \"{name}\"", section.word()))?;
            entry.as_object_mut().ok_or("not an object")?.insert("enabled".into(), Value::Bool(enabled));
            Ok(())
        })?;
        if enabled {
            self.restart_state(section, name);
        }
        info!("{} {name}: switched {}", section.word(), if enabled { "on" } else { "off" });
        self.reload();
        self.save_state(&self.lock());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job_kinds::TEST_KINDS;
    use serde_json::json;
    use time::macros::{datetime, time};

    fn utc(_: OffsetDateTime) -> UtcOffset {
        UtcOffset::UTC
    }

    struct Setup {
        auto: Automation,
        queue: Queue,
        dir: PathBuf,
        clock: Arc<Mutex<OffsetDateTime>>,
    }

    impl Setup {
        fn new(name: &str, file: Value) -> Setup {
            let dir = std::env::temp_dir().join(format!("rust_scaffold_automation_{}_{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(FILE), serde_json::to_string_pretty(&file).unwrap()).unwrap();
            let queue = Queue::new(Handle::current(), 3, Arc::new(|_| {}), TEST_KINDS);
            let clock = Arc::new(Mutex::new(datetime!(2027-01-30 12:00 UTC)));
            let auto = Self::open(&dir, &queue, &clock);
            Setup { auto, queue, dir, clock }
        }

        /// The engine on `dir`, as the app opens it at start.
        fn open(dir: &std::path::Path, queue: &Queue, clock: &Arc<Mutex<OffsetDateTime>>) -> Automation {
            let c = clock.clone();
            Automation::new(dir.to_path_buf(), queue.clone(), Handle::current(), Arc::new(|_| {}), Arc::new(|_| Ok(())), Arc::new(move || *c.lock().unwrap()), utc)
        }

        fn advance(&self, by: time::Duration) {
            *self.clock.lock().unwrap() += by;
        }

        fn file(&self) -> Value {
            serde_json::from_str(&fs::read_to_string(self.dir.join(FILE)).unwrap()).unwrap()
        }

        fn trigger(&self, name: &str) -> TriggerView {
            self.auto.list().triggers.into_iter().find(|t| t.name == name).unwrap()
        }

        fn action(&self, name: &str) -> ActionView {
            self.auto.list().actions.into_iter().find(|a| a.name == name).unwrap()
        }

        fn trigger_def(&self, name: &str) -> TriggerDef {
            self.auto.lock().triggers.iter().find(|t| t.item.def.name == name).unwrap().item.def.clone()
        }

        /// Ticks until no Action has a job queued or running.
        async fn settle(&self) {
            for _ in 0..1000 {
                self.auto.tick();
                if self.auto.list().actions.iter().all(|a| a.running.is_empty()) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            panic!("the jobs did not finish");
        }

        /// What the job was given, as the window sees it.
        fn given(&self, job: u64) -> Value {
            serde_json::to_value(self.queue.inspect(job).unwrap()).unwrap()["job"]["given"].clone()
        }
    }

    impl Drop for Setup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn file(triggers: Value, schedules: Value, actions: Value) -> Value {
        json!({ "version": 2, "triggers": triggers, "schedules": schedules, "actions": actions })
    }

    fn count(name: &str, then: &[&str]) -> Value {
        json!({ "name": name, "kind": "count", "input": { "count": 1, "delay_ms": 100 }, "then": then })
    }

    fn watch(read_as: &str) -> Value {
        json!({ "type": "web_watch", "url": "http://127.0.0.1:9/", "find": "id", "name": "price", "read_as": read_as })
    }

    /// A Trigger that is off, so the timing never checks it on its own; the
    /// tests hand it readings with `record_check`.
    fn trigger(name: &str, when: Value, then: &[&str]) -> Value {
        json!({ "name": name, "enabled": false, "watch": watch("number"), "every_seconds": 30, "when": when, "then": then })
    }

    fn reading(text: &str, number: f64) -> Result<Value, String> {
        Ok(json!({ "text": text, "number": number }))
    }

    #[cfg(feature = "web-watch")]
    #[tokio::test]
    async fn every_entry_is_checked_and_bad_ones_are_skipped_with_a_reason() {
        let q = Queue::new(Handle::current(), 1, Arc::new(|_| {}), TEST_KINDS);
        let text = file(
            json!([
                trigger("good trigger", json!({ "is": "changes" }), &["A"]),
                { "name": "too often", "watch": watch("number"), "every_seconds": 10, "when": { "is": "changes" } },
                { "name": "above text", "watch": watch("text"), "every_seconds": 30, "when": { "is": "above", "value": 5 } },
                { "name": "no such action", "watch": watch("text"), "every_seconds": 30, "when": { "is": "changes" }, "then": ["Z"] },
                { "name": "never fires", "watch": watch("text"), "every_seconds": 30, "when": { "is": "changes" }, "max_fires": 0 },
            ]),
            json!([
                { "name": "good schedule", "when": { "type": "daily", "time": "08:00" }, "then": ["A"] },
                { "name": "bad time", "when": { "type": "daily", "time": "25:00" } },
                { "name": " spaced", "when": { "type": "daily", "time": "08:00" } },
            ]),
            json!([
                count("A", &[]),
                count("A", &[]),
                count("loops", &["loops"]),
                { "name": "unknown kind", "kind": "teleport" },
                { "name": "switch missing", "kind": "switch", "input": { "section": "trigger", "name": "ghost", "enabled": false } },
                { "name": "switch good", "kind": "switch", "input": { "section": "trigger", "name": "good trigger", "enabled": true } },
                { "name": "twice", "kind": "count", "input": { "count": 1, "delay_ms": 100 }, "then": ["A", "A"] },
            ]),
        )
        .to_string();
        let parsed = parse(&text, &q).unwrap();
        let names = |v: Vec<String>| v;
        assert_eq!(names(parsed.triggers.iter().map(|t| t.def.name.clone()).collect()), ["good trigger"]);
        assert_eq!(names(parsed.schedules.iter().map(|s| s.def.name.clone()).collect()), ["good schedule"]);
        assert_eq!(names(parsed.actions.iter().map(|a| a.def.name.clone()).collect()), ["A", "switch good"]);
        let reasons: Vec<String> = parsed.skipped.iter().map(Skip::text).collect();
        for want in [
            "the name is used twice",
            "an action cannot start itself",
            "unknown job kind",
            "there is no trigger \"ghost\" for it to act on",
            "listed twice",
            "at most every 30 seconds",
            "need Read as: Number",
            "there is no action \"Z\"",
            "fire at most must be 1 to 999999",
            "time \"25:00\" is not HH:MM",
            "starts or ends with a space",
        ] {
            assert!(reasons.iter().any(|r| r.contains(want)), "no skip saying {want:?} in {reasons:#?}");
        }
        assert_eq!(reasons.len(), 11, "{reasons:#?}");
        assert!(parse(&json!({ "version": 1, "schedules": [] }).to_string(), &q).err().unwrap().contains("version 1"));
    }

    #[cfg(feature = "web-watch")]
    #[tokio::test]
    async fn the_default_file_has_the_probe_demos_switched_off() {
        let q = Queue::new(Handle::current(), 1, Arc::new(|_| {}), crate::job_kinds::KINDS);
        let parsed = parse(DEFAULT_FILE, &q).unwrap();
        assert!(parsed.skipped.is_empty(), "{:?}", parsed.skipped.iter().map(Skip::text).collect::<Vec<_>>());
        let names: Vec<_> = parsed.triggers.iter().map(|t| t.def.name.as_str()).collect();
        assert_eq!(names, ["GBP price", "USD price", "Probe token"]);
        assert!(parsed.triggers.iter().all(|t| !t.def.enabled), "every demo trigger is off");
        assert_eq!(parsed.triggers[0].def.then, ["Notify GBP price"]);
        assert_eq!(parsed.triggers[1].def.then, ["Notify USD price"]);
        assert_eq!(parsed.triggers[2].def.then, ["Notify probe token"]);
        assert!(parsed.actions.iter().all(|a| a.def.kind == "notify"));
    }

    #[test]
    fn conditions_hold_and_readings_compare() {
        let gbp = json!({ "text": "£5,212.66", "number": 5212.66 });
        let above = |v: f64| Condition::Above { value: v }.holds(&gbp);
        assert_eq!(above(5000.0), Some(true));
        assert_eq!(above(5212.66), Some(false), "strictly above");
        assert_eq!(Condition::Below { value: 6000.0 }.holds(&gbp), Some(true));
        assert_eq!(Condition::Equals { value: json!(5212.66) }.holds(&gbp), Some(true));
        assert_eq!(Condition::Equals { value: json!("5212.66") }.holds(&gbp), Some(true), "a number typed as text");
        let text = json!({ "text": "Sold out" });
        assert_eq!(Condition::Equals { value: json!(" Sold out ") }.holds(&text), Some(true));
        assert_eq!(Condition::Equals { value: json!("In stock") }.holds(&text), Some(false));
        assert_eq!(Condition::Above { value: 1.0 }.holds(&text), Some(false), "no number never passes");
        assert_eq!(Condition::Changes.holds(&gbp), None);
        assert!(same_reading(&gbp, &json!({ "text": "£5212.66", "number": 5212.66 })), "same number, other formatting");
        assert!(!same_reading(&gbp, &json!({ "text": "£5,212.67", "number": 5212.67 })));
        assert!(!same_reading(&text, &json!({ "text": "In stock" })));
        assert_eq!(Condition::Above { value: 5000.0 }.describe(), "is above 5000");
        assert_eq!(Condition::Equals { value: json!("Sold out") }.describe(), "equals \"Sold out\"");
    }

    #[cfg(feature = "web-watch")]
    #[tokio::test]
    async fn a_trigger_fires_once_each_time_its_condition_becomes_true() {
        let s = Setup::new(
            "fires",
            file(json!([trigger("high", json!({ "is": "above", "value": 5000 }), &["A"])]), json!([]), json!([count("A", &[])])),
        );
        let def = s.trigger_def("high");
        let mut jobs = Vec::new();
        for (text, number, fires) in [
            ("£5,100.00", 5100.0, false), // first reading after saving: recorded only
            ("£4,900.00", 4900.0, false),
            ("£5,100.00", 5100.0, true), // crossed
            ("£5,200.00", 5200.0, false), // still above: no second fire
            ("£4,800.00", 4800.0, false),
            ("£5,300.00", 5300.0, true),
        ] {
            s.auto.record_check(&def, reading(text, number));
            let running = s.action("A").running;
            assert_eq!(!running.is_empty(), fires, "{text}");
            jobs.extend(running);
            s.settle().await;
        }
        assert_eq!(s.trigger("high").state.fires, 2);
        assert_eq!(s.action("A").state.history.len(), 2);
        assert_eq!(s.action("A").state.history[0].by, "trigger high");
        let given = s.given(jobs[0]);
        assert_eq!(given["from"], "trigger high");
        assert_eq!(given["value"], json!({ "text": "£5,100.00", "number": 5100.0 }));
        assert_eq!(given["previous"], json!({ "text": "£4,900.00", "number": 4900.0 }));
    }

    #[cfg(feature = "web-watch")]
    #[tokio::test]
    async fn changes_fires_on_each_new_value_and_errors_change_nothing() {
        let s = Setup::new("changes", file(json!([trigger("gbp", json!({ "is": "changes" }), &["A"])]), json!([]), json!([count("A", &[])])));
        let def = s.trigger_def("gbp");
        for (result, fires) in [
            (reading("£1.00", 1.0), false),
            (reading("£1.00", 1.0), false),
            (reading("£2.00", 2.0), true),
            (Err("could not reach the page".to_string()), false),
            (reading("£2.00", 2.0), false),
            (reading("£3.00", 3.0), true),
        ] {
            let failed = result.is_err();
            s.auto.record_check(&def, result);
            assert_eq!(!s.action("A").running.is_empty(), fires);
            assert_eq!(s.trigger("gbp").state.last_error.is_some(), failed);
            s.settle().await;
        }
        let t = s.trigger("gbp");
        assert_eq!((t.state.fires, t.state.last_value), (2, Some(json!({ "text": "£3.00", "number": 3.0 }))));
    }

    #[cfg(feature = "web-watch")]
    #[tokio::test]
    async fn fire_at_most_switches_the_trigger_off_and_switching_on_starts_afresh() {
        let mut once = trigger("once", json!({ "is": "changes" }), &["A"]);
        once["max_fires"] = json!(1);
        let s = Setup::new("max_fires", file(json!([once]), json!([]), json!([count("A", &[])])));
        let def = s.trigger_def("once");
        s.auto.record_check(&def, reading("1", 1.0));
        s.auto.record_check(&def, reading("2", 2.0));
        assert_eq!(s.file()["triggers"][0]["enabled"], json!(false), "switched off in jobs.json");
        assert!(!s.trigger("once").enabled);
        s.settle().await;

        s.auto.set_enabled(Section::Trigger, "once", true).unwrap();
        let t = s.trigger("once");
        assert!(t.enabled && !t.state.primed && t.state.fires == 0);
        assert_eq!(t.state.last_value, Some(json!({ "text": "2", "number": 2.0 })), "the last value is still shown");
        let def = s.trigger_def("once");
        s.auto.record_check(&def, reading("3", 3.0));
        assert!(s.action("A").running.is_empty(), "the first reading after switching on only records");
        s.auto.record_check(&def, reading("4", 4.0));
        assert_eq!(s.action("A").running.len(), 1);
        s.settle().await;
    }

    #[cfg(feature = "web-watch")]
    #[tokio::test]
    async fn actions_pass_the_reading_and_their_output_forward() {
        let first = json!({ "name": "first", "kind": "command", "input": { "script": "printf 'from=%s value=%s previous=%s' \"$JOB_FROM\" \"$JOB_VALUE\" \"$JOB_PREVIOUS\"" }, "then": ["second"] });
        let second = json!({ "name": "second", "kind": "command", "input": { "script": "printf 'got[%s] from=%s value=%s' \"$JOB_OUTPUT\" \"$JOB_FROM\" \"$JOB_VALUE\"" } });
        let s = Setup::new("forward", file(json!([trigger("t", json!({ "is": "changes" }), &["first"])]), json!([]), json!([first, second])));
        let def = s.trigger_def("t");
        s.auto.record_check(&def, reading("£1", 1.0));
        s.auto.record_check(&def, reading("£2", 2.0));
        s.settle().await;
        let first_out = s.action("first").state.history[0].output.clone().unwrap();
        assert_eq!(first_out, r#"from=trigger t value={"number":2.0,"text":"£2"} previous={"number":1.0,"text":"£1"}"#);
        let second = &s.action("second").state.history[0];
        assert_eq!(second.by, "action first");
        assert_eq!(second.output.as_deref(), Some(format!("got[{first_out}] from=action first value={{\"number\":2.0,\"text\":\"£2\"}}").as_str()));
    }

    #[tokio::test]
    async fn a_loop_of_actions_stops_at_the_chain_limit() {
        let s = Setup::new("loop", file(json!([]), json!([]), json!([count("ping", &["pong"]), count("pong", &["ping"])])));
        s.auto.run("ping").unwrap();
        s.settle().await;
        let ping = s.action("ping").state.history;
        let pong = s.action("pong").state.history;
        assert_eq!(pong.len(), 5);
        assert_eq!(ping.len(), 6, "5 runs and the refused 11th start");
        assert_eq!(ping.iter().filter(|r| r.state == JobState::Done).count() + pong.len(), MAX_CHAIN as usize);
        let last = ping.last().unwrap();
        assert_eq!(last.state, JobState::Failed);
        assert!(last.message.as_deref().unwrap().contains("loop guard"), "{last:?}");
    }

    #[tokio::test]
    async fn an_action_that_is_off_only_runs_with_the_run_button() {
        let mut off = count("off", &[]);
        off["enabled"] = json!(false);
        let s = Setup::new("off", file(json!([]), json!([{ "name": "s", "when": { "type": "every", "interval": "1m" }, "then": ["off"] }]), json!([off])));
        s.advance(time::Duration::minutes(1));
        s.auto.tick();
        assert!(s.action("off").running.is_empty(), "a schedule does not start it");
        assert!(s.auto.run("off").unwrap().is_some(), "Run does");
        s.settle().await;
        assert_eq!(s.action("off").state.history[0].by, "you");
    }

    fn schedule_item(timing: Timing, max_runs: Option<u64>) -> ScheduleItem {
        let when = When::Daily { time: "08:00".into() };
        ScheduleItem { def: ScheduleDef { name: "s".into(), enabled: true, when, max_runs, then: vec![] }, timing, raw: Value::Null }
    }

    #[test]
    fn missed_runs_are_made_up_once_and_not_from_before_switching_on() {
        let now = datetime!(2027-01-30 12:00 UTC);
        let ms = |at: OffsetDateTime| Some(to_ms(at));
        let daily = schedule_item(Timing::Daily(time!(08:00)), None);
        let ran_two_days_ago = ScheduleState { runs: 1, last_run_ms: ms(datetime!(2027-01-28 08:00 UTC)), since_ms: None, delayed_until_ms: None };
        assert_eq!(next_due(&daily, &ran_two_days_ago, now, utc), Some(now), "made up at once, one run");
        let ran_now = ScheduleState { runs: 2, last_run_ms: ms(now), since_ms: None, delayed_until_ms: None };
        assert_eq!(next_due(&daily, &ran_now, now, utc), Some(datetime!(2027-01-31 08:00 UTC)), "then back on the timetable");
        let switched_on_now = ScheduleState { since_ms: ms(now), ..ran_two_days_ago.clone() };
        assert_eq!(next_due(&daily, &switched_on_now, now, utc), Some(datetime!(2027-01-31 08:00 UTC)), "runs missed while off are not made up");
        let every = schedule_item(Timing::Every(Duration::from_secs(60)), None);
        let a_week_ago = ScheduleState { runs: 5, last_run_ms: ms(datetime!(2027-01-23 12:00 UTC)), since_ms: None, delayed_until_ms: None };
        assert_eq!(next_due(&every, &a_week_ago, now, utc), Some(now), "10,080 missed runs: one, now");
        let once = schedule_item(Timing::Once(datetime!(2027-01-30 09:00)), None);
        let saved_before = ScheduleState { since_ms: ms(datetime!(2027-01-30 08:00 UTC)), ..ScheduleState::default() };
        assert_eq!(next_due(&once, &saved_before, now, utc), Some(now), "due while the app was closed");
        let saved_after = ScheduleState { since_ms: ms(datetime!(2027-01-30 10:00 UTC)), ..ScheduleState::default() };
        assert_eq!(next_due(&once, &saved_after, now, utc), None, "set in the past");
        let limited = schedule_item(Timing::Daily(time!(08:00)), Some(2));
        assert_eq!(next_due(&limited, &ran_now, now, utc), None, "run limit reached");
    }

    #[tokio::test]
    async fn a_due_schedule_starts_its_actions() {
        let s = Setup::new(
            "schedule",
            file(json!([]), json!([{ "name": "minutely", "when": { "type": "every", "interval": "1m" }, "max_runs": 2, "then": ["A", "B"] }]), json!([count("A", &[]), count("B", &[])])),
        );
        let next = s.auto.list().schedules[0].next_ms.unwrap();
        assert_eq!(next, to_ms(datetime!(2027-01-30 12:01 UTC)));
        s.auto.tick();
        assert!(s.action("A").running.is_empty(), "not due yet");
        for _ in 0..3 {
            s.advance(time::Duration::minutes(1));
            s.auto.tick();
            s.settle().await;
        }
        let view = s.auto.list().schedules[0].clone();
        assert_eq!((view.state.runs, view.next_ms), (2, None), "stops at its run limit");
        assert_eq!(s.action("A").state.history.len(), 2);
        assert_eq!(s.action("B").state.history[0].by, "schedule minutely");
    }

    #[cfg(feature = "web-watch")]
    #[tokio::test]
    async fn renames_follow_deletes_tidy_up_and_copies_start_off() {
        let s = Setup::new(
            "edit",
            file(
                json!([trigger("T", json!({ "is": "changes" }), &["A"])]),
                json!([{ "name": "S", "when": { "type": "daily", "time": "08:00" }, "then": ["A"] }]),
                json!([count("A", &[]), { "name": "stop T", "kind": "switch", "input": { "section": "trigger", "name": "T", "enabled": false } }]),
            ),
        );
        let mut a = s.file()["actions"][0].clone();
        a["name"] = json!("A2");
        s.auto.save(Section::Action, Some("A".into()), a).unwrap();
        assert_eq!(s.file()["triggers"][0]["then"], json!(["A2"]));
        assert_eq!(s.file()["schedules"][0]["then"], json!(["A2"]));

        let mut t = s.file()["triggers"][0].clone();
        t["name"] = json!("T2");
        s.auto.save(Section::Trigger, Some("T".into()), t).unwrap();
        assert_eq!(s.file()["actions"][1]["input"]["name"], "T2", "the switch follows the rename");
        let refused = s.auto.delete(Section::Trigger, "T2").unwrap_err();
        assert!(refused.contains("\"stop T\""), "{refused}");

        s.auto.delete(Section::Action, "A2").unwrap();
        assert!(!s.auto.lock().action_state.contains_key("A2"), "its results are forgotten");
        assert_eq!(s.file()["triggers"][0]["then"], json!([]));
        assert_eq!(s.file()["schedules"][0]["then"], json!([]));

        assert_eq!(s.auto.duplicate(Section::Schedule, "S").unwrap(), "S copy");
        assert_eq!(s.auto.duplicate(Section::Schedule, "S").unwrap(), "S copy 2");
        let copy = s.auto.list().schedules.into_iter().find(|v| v.name == "S copy").unwrap();
        assert!(!copy.enabled, "a copy starts off");

        let before = fs::read_to_string(s.dir.join(FILE)).unwrap();
        let mut bad = s.file()["triggers"][0].clone();
        bad["every_seconds"] = json!(5);
        let refused = s.auto.save(Section::Trigger, Some("T2".into()), bad).unwrap_err();
        assert!(refused.contains("at most every 30 seconds"), "{refused}");
        let mut taken = s.file()["schedules"][0].clone();
        taken["name"] = json!("S copy");
        assert!(s.auto.save(Section::Schedule, Some("S".into()), taken).unwrap_err().contains("already exists"));
        assert_eq!(fs::read_to_string(s.dir.join(FILE)).unwrap(), before, "a refused change writes nothing");

        s.auto.run("stop T").unwrap();
        assert_eq!(s.file()["triggers"][0]["enabled"], json!(false));
        assert_eq!(s.action("stop T").state.history[0].message.as_deref(), Some("switched off trigger \"T2\""));
    }

    #[cfg(feature = "web-watch")]
    #[tokio::test]
    async fn definitions_readings_and_results_are_kept_across_a_restart() {
        let s = Setup::new(
            "restart",
            file(
                json!([trigger("gbp", json!({ "is": "changes" }), &["A"])]),
                json!([{ "name": "S", "when": { "type": "every", "interval": "1m" }, "then": ["A"] }]),
                json!([count("A", &[])]),
            ),
        );
        let def = s.trigger_def("gbp");
        s.auto.record_check(&def, reading("£1", 1.0));
        s.advance(time::Duration::minutes(1));
        s.auto.tick();
        s.settle().await;
        drop(s.auto.clone());

        let again = Setup::open(&s.dir, &s.queue, &s.clock);
        let list = again.list();
        let t = &list.triggers[0];
        assert!(t.state.primed, "the next reading after a restart can fire");
        assert_eq!(t.state.last_value, Some(json!({ "text": "£1", "number": 1.0 })));
        assert_eq!(list.schedules[0].state.runs, 1);
        assert_eq!(list.actions[0].state.history.len(), 1);
        again.record_check(&def, reading("£2", 2.0));
        assert_eq!(again.list().actions[0].running.len(), 1, "a change while the app was closed fires");
    }

    #[cfg(feature = "web-watch")]
    #[tokio::test]
    async fn the_quick_buttons_switch_delete_and_restore() {
        let mut off = json!({ "name": "S", "enabled": false, "when": { "type": "daily", "time": "08:00" }, "then": ["A"] });
        let s = Setup::new("bulk", file(json!([trigger("T", json!({ "is": "changes" }), &["A"])]), json!([off.take()]), json!([count("A", &[])])));
        assert_eq!(s.auto.bulk(Bulk::EnableAll).unwrap(), 2, "the trigger and the schedule");
        assert_eq!(s.file()["triggers"][0]["enabled"], json!(true));
        assert_eq!(s.file()["schedules"][0]["enabled"], json!(true));
        assert!(!s.trigger("T").state.primed, "switched on afresh");
        assert_eq!(s.auto.bulk(Bulk::EnableAll).unwrap(), 0, "nothing left to switch on");
        assert_eq!(s.auto.bulk(Bulk::DisableAll).unwrap(), 2);
        assert!(s.auto.list().triggers.iter().all(|t| !t.enabled));
        assert_eq!(s.auto.bulk(Bulk::RestoreDemos).unwrap(), 6, "the three demo triggers and their actions");
        assert_eq!(s.auto.bulk(Bulk::RestoreDemos).unwrap(), 0, "already there");
        let demo = s.trigger_def("GBP price");
        s.auto.record_check(&demo, reading("£5,479.38", 5479.38));
        assert!(s.trigger("GBP price").state.primed);
        assert_eq!(s.auto.bulk(Bulk::DeleteAll).unwrap(), 9);
        let list = s.auto.list();
        assert!(list.triggers.is_empty() && list.schedules.is_empty() && list.actions.is_empty());
        assert_eq!(s.auto.bulk(Bulk::RestoreDemos).unwrap(), 6);
        assert_eq!(s.trigger("GBP price").state, TriggerState::default(), "nothing kept from before the delete");
    }

    #[cfg(feature = "web-watch")]
    #[tokio::test]
    async fn fire_now_and_run_by_hand_give_the_latest_reading() {
        use crate::job_kinds::tests::{fake_site, page_reply};
        let page = |gbp: &str| page_reply(&format!("<p id=\"price\">{gbp}</p>"));
        let (port, _) = fake_site(vec![page("£2"), page("£2")]).await;
        let mut t = trigger("gbp", json!({ "is": "changes" }), &["A"]);
        t["watch"]["url"] = json!(format!("http://127.0.0.1:{port}/"));
        let s = Setup::new(
            "fire",
            file(json!([t]), json!([{ "name": "S", "when": { "type": "daily", "time": "08:00" }, "then": ["A"] }]), json!([count("A", &[])])),
        );
        let def = s.trigger_def("gbp");
        s.auto.record_check(&def, reading("£1", 1.0));
        s.auto.record_check(&def, reading("£2", 2.0));
        s.settle().await;
        assert_eq!(s.trigger("gbp").state.fires, 1);

        // Fire now on the unchanged page (the trigger is off): starts A with
        // the reading now and the one before the last change.
        let read = s.auto.fire_trigger("gbp").await.unwrap();
        assert_eq!(read["text"], "£2");
        let job = s.action("A").running[0];
        let given = s.given(job);
        assert_eq!((given["value"]["text"].clone(), given["previous"]["text"].clone()), (json!("£2"), json!("£1")));
        s.settle().await;
        assert_eq!(s.trigger("gbp").state.fires, 1, "a fire by hand does not count towards fire at most");

        // Run on the action by hand: the same reading.
        let job = s.auto.run("A").unwrap().unwrap();
        let given = s.given(job);
        assert_eq!((given["value"]["text"].clone(), given["previous"]["text"].clone(), given["from"].clone()), (json!("£2"), json!("£1"), json!("you")));
        s.settle().await;

        // Run now on a schedule: its actions start; its count does not change.
        assert_eq!(s.auto.run_schedule("S").unwrap(), 1);
        assert_eq!(s.action("A").running.len(), 1);
        s.settle().await;
        assert_eq!(s.auto.list().schedules[0].state.runs, 0);
        assert_eq!(s.action("A").state.history.last().unwrap().by, "schedule S");
        assert!(s.auto.fire_trigger("nope").await.is_err());
    }

    #[tokio::test]
    async fn hardware_watches_are_checked_and_a_file_trigger_fires() {
        let q = Queue::new(Handle::current(), 1, Arc::new(|_| {}), TEST_KINDS);
        let watch_file = |path: &str| json!({ "type": "file", "path": path });
        let text = file(
            json!([
                { "name": "cpu", "watch": { "type": "cpu" }, "every_seconds": 5, "when": { "is": "above", "value": 90 } },
                { "name": "too often", "watch": { "type": "memory" }, "every_seconds": 4, "when": { "is": "changes" } },
                { "name": "disk free", "watch": { "type": "disk", "path": "/", "measure": "free_gb" }, "every_seconds": 60, "when": { "is": "below", "value": 10 } },
                { "name": "file above", "watch": watch_file("/tmp/x"), "every_seconds": 5, "when": { "is": "above", "value": 1 } },
                { "name": "no path", "watch": { "type": "disk", "path": "" }, "every_seconds": 60, "when": { "is": "changes" } },
            ]),
            json!([]),
            json!([]),
        )
        .to_string();
        let parsed = parse(&text, &q).unwrap();
        assert_eq!(parsed.triggers.iter().map(|t| t.def.name.as_str()).collect::<Vec<_>>(), ["cpu", "disk free"]);
        let reasons: Vec<String> = parsed.skipped.iter().map(Skip::text).collect();
        assert!(reasons.iter().any(|r| r.contains("too often") && r.contains("every 5 seconds")), "{reasons:?}");
        assert!(reasons.iter().any(|r| r.contains("file above") && r.contains("Number")), "{reasons:?}");
        assert!(reasons.iter().any(|r| r.contains("no path") && r.contains("empty")), "{reasons:?}");

        let dir = std::env::temp_dir().join(format!("rust_scaffold_hw_trigger_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let watched = dir.join("watched.txt");
        let t = json!({ "name": "file", "enabled": false, "watch": watch_file(&watched.display().to_string()), "every_seconds": 5, "when": { "is": "changes" }, "then": ["A"] });
        let s = Setup::new("hw", file(json!([t]), json!([]), json!([count("A", &[])])));
        let def = s.trigger_def("file");
        s.auto.check_trigger(def.clone()).await;
        assert_eq!(s.trigger("file").state.last_value.unwrap()["text"], "missing");
        assert!(s.action("A").running.is_empty(), "the first reading only records");
        fs::write(&watched, "new").unwrap();
        s.auto.check_trigger(def).await;
        let job = s.action("A").running[0];
        assert_eq!(s.given(job)["previous"]["text"], "missing", "the file appeared");
        s.settle().await;
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn built_in_actions_are_checked_and_do_their_work() {
        let q = Queue::new(Handle::current(), 1, Arc::new(|_| {}), TEST_KINDS);
        let schedule = json!({ "name": "S", "when": { "type": "daily", "time": "08:00" } });
        let bad = file(
            json!([]),
            json!([schedule.clone()]),
            json!([
                { "name": "delay ok", "kind": "delay", "input": { "schedule": "S", "by": "2h" } },
                { "name": "delay bad", "kind": "delay", "input": { "schedule": "S", "by": "soon" } },
                { "name": "count action", "kind": "set_count", "input": { "section": "action", "name": "delay ok", "count": 3 } },
                { "name": "fire nothing", "kind": "fire", "input": { "trigger": "ghost" } },
                { "name": "app dance", "kind": "app", "input": { "do": "dance" } },
                { "name": "log empty", "kind": "log", "input": { "level": "warn", "text": " " } },
                { "name": "clear", "kind": "clear_errors" },
            ]),
        )
        .to_string();
        let parsed = parse(&bad, &q).unwrap();
        assert_eq!(parsed.actions.iter().map(|a| a.def.name.as_str()).collect::<Vec<_>>(), ["delay ok", "clear"]);
        assert_eq!(parsed.skipped.len(), 5, "{:?}", parsed.skipped.iter().map(Skip::text).collect::<Vec<_>>());

        // The work, with the app's show, hide, quit and restart recorded.
        let dir = std::env::temp_dir().join(format!("rust_scaffold_automation_{}_builtin", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let watched = dir.join("watched.txt");
        let t = json!({ "name": "T", "enabled": false, "watch": { "type": "file", "path": watched.display().to_string() }, "every_seconds": 5, "when": { "is": "changes" }, "then": ["A"] });
        let f = file(
            json!([t, { "name": "Old", "enabled": false, "watch": { "type": "cpu" }, "every_seconds": 60, "when": { "is": "above", "value": 99 } }]),
            json!([schedule]),
            json!([
                count("A", &[]),
                { "name": "later", "kind": "delay", "input": { "schedule": "S", "by": "2h" } },
                { "name": "limit", "kind": "set_count", "input": { "section": "schedule", "name": "S", "count": 3 } },
                { "name": "fire T", "kind": "fire", "input": { "trigger": "T" } },
                { "name": "show", "kind": "app", "input": { "do": "show" } },
                { "name": "quit", "kind": "app", "input": { "do": "quit" } },
                { "name": "note", "kind": "log", "input": { "level": "info", "text": "hello {from}" } },
                { "name": "drop Old", "kind": "delete", "input": { "section": "trigger", "name": "Old" }, "then": ["note"] },
                { "name": "chain", "kind": "count", "input": { "count": 1, "delay_ms": 100 }, "then": ["drop Old"] },
            ]),
        );
        fs::write(dir.join(FILE), serde_json::to_string_pretty(&f).unwrap()).unwrap();
        let calls: Arc<Mutex<Vec<AppDo>>> = Arc::default();
        let record = calls.clone();
        let clock = Arc::new(Mutex::new(datetime!(2027-01-30 12:00 UTC)));
        let c = clock.clone();
        let auto = Automation::new(
            dir.clone(),
            q.clone(),
            Handle::current(),
            Arc::new(|_| {}),
            Arc::new(move |what| {
                record.lock().unwrap().push(what);
                Ok(())
            }),
            Arc::new(move || *c.lock().unwrap()),
            utc,
        );
        let next = |a: &Automation| a.list().schedules[0].next_ms.unwrap();
        assert_eq!(next(&auto), to_ms(datetime!(2027-01-31 08:00 UTC)));
        auto.run("later").unwrap();
        assert_eq!(next(&auto), to_ms(datetime!(2027-01-31 10:00 UTC)), "two hours later");
        auto.run("limit").unwrap();
        let file_now: Value = serde_json::from_str(&fs::read_to_string(dir.join(FILE)).unwrap()).unwrap();
        assert_eq!(file_now["schedules"][0]["max_runs"], 3);
        auto.run("show").unwrap();
        auto.run("quit").unwrap();
        assert_eq!(*calls.lock().unwrap(), [AppDo::Show, AppDo::Quit]);
        // Fire: reads the file at once and starts A.
        auto.run("fire T").unwrap();
        for _ in 0..200 {
            if !auto.list().actions.iter().find(|a| a.name == "A").unwrap().running.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(auto.list().triggers[0].state.last_value.as_ref().unwrap()["text"], "missing");
        assert!(!auto.list().actions.iter().find(|a| a.name == "A").unwrap().running.is_empty(), "fire started A");
        // Delete through a chain: "chain" succeeds, then "drop Old" deletes the
        // trigger and itself, then "note" logs.
        auto.run("chain").unwrap();
        for _ in 0..400 {
            auto.tick();
            if auto.list().triggers.len() == 1 && auto.list().actions.iter().all(|a| a.running.is_empty()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let list = auto.list();
        assert_eq!(list.triggers.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["T"]);
        assert!(!list.actions.iter().any(|a| a.name == "drop Old"), "a delete action goes with what it deleted");
        assert!(list.actions.iter().find(|a| a.name == "chain").unwrap().then.is_empty(), "and leaves the then lists");
        assert!(list.skipped.is_empty(), "{:?}", list.skipped);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_made_up_run_never_quits_the_app() {
        let s = Setup::new(
            "made_up",
            file(json!([]), json!([{ "name": "Q", "when": { "type": "daily", "time": "08:00" }, "then": ["quit"] }]), json!([{ "name": "quit", "kind": "app", "input": { "do": "quit" } }])),
        );
        // Last ran two days ago: today's 08:00 was missed while the app was closed.
        s.auto.lock().schedule_state.get_mut("Q").unwrap().last_run_ms = Some(to_ms(datetime!(2027-01-28 08:00 UTC)));
        s.auto.reload();
        s.auto.tick();
        let last = s.action("quit").state.history.last().cloned().unwrap();
        assert_eq!(last.state, JobState::Failed);
        assert!(last.message.unwrap().contains("made up"), "skipped, not quit");
    }

    #[tokio::test]
    async fn a_jobs_1_file_is_set_aside_and_replaced_by_the_defaults() {
        let s = Setup::new("v1", json!({ "version": 1, "schedules": [], "rules": [] }));
        let old: Value = serde_json::from_str(&fs::read_to_string(s.dir.join("jobs.json.v1")).unwrap()).unwrap();
        assert_eq!(old["version"], 1);
        assert_eq!(s.file()["version"], 2);
        assert_eq!(s.auto.list().status, FileStatus::Created);
    }

    #[cfg(feature = "web-watch")]
    #[tokio::test]
    async fn test_now_and_real_checks_read_the_page() {
        use crate::job_kinds::tests::{fake_site, page_reply, PROBE_PAGE};
        let changed = PROBE_PAGE.replace("£3,577.30", "£3,612.05");
        let (port, _) = fake_site(vec![page_reply(PROBE_PAGE), page_reply(PROBE_PAGE), page_reply(&changed)]).await;
        let url = format!("http://127.0.0.1:{port}/probe");
        let mut t = trigger("gbp", json!({ "is": "above", "value": 3600 }), &["A"]);
        t["watch"]["url"] = json!(url);
        t["watch"]["name"] = json!("probe-gbp");
        let s = Setup::new("http", file(json!([t.clone()]), json!([]), json!([count("A", &[])])));

        let test = s.auto.test_trigger_named(None, t.clone()).await.unwrap();
        assert_eq!(test, TestReply { value: Some(json!({ "text": "£3,577.30", "number": 3577.3 })), error: None, holds: Some(false), changed: None });

        let def = s.trigger_def("gbp");
        s.auto.check_trigger(def.clone()).await;
        assert_eq!(s.trigger("gbp").state.held, Some(false));
        s.auto.check_trigger(def).await;
        assert_eq!(s.trigger("gbp").state.last_value.unwrap()["number"], json!(3612.05));
        assert_eq!(s.action("A").running.len(), 1, "crossed 3600");
        s.settle().await;

        let mut changes = t;
        changes["when"] = json!({ "is": "changes" });
        let test = s.auto.test_trigger_named(Some("gbp".into()), changes).await.unwrap();
        assert!(test.error.unwrap().contains("could not reach"), "the fake site has answered all its replies");
    }
}
