//! Hardware-Watch: values from this computer for a Trigger to watch (see
//! `src/automation.rs`). No crates beyond the ones already compiled: on
//! Linux the readings come from `/proc`, `statvfs` (the `libc` crate) and
//! file metadata, and graphics card memory from `nvidia-smi`. Elsewhere the
//! ones that need Linux say so instead of guessing.
//!
//! Each reading is `{"text": ..., "number": ...}` like a web watch read as a
//! number, so "is above" and "is below" work on it; a file watch has text
//! only (its size or time is not a number to compare), so "changes" notices
//! any change of contents or permissions.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde::Deserialize;
use serde_json::{json, Value};

/// Shortest interval between two checks of a hardware watch.
pub const MIN_EVERY_SECONDS: u64 = 5;
/// How long the CPU is sampled for.
const CPU_SAMPLE: Duration = Duration::from_secs(1);
/// Longest `nvidia-smi` may take.
const NVIDIA_SMI_WAIT: Duration = Duration::from_secs(10);
const MAX_PATH: usize = 4096;

/// Which disk figure is the number.
#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiskMeasure {
    /// Per cent of the disk in use.
    #[default]
    UsedPercent,
    /// Gigabytes free (1 GB = 1,000,000,000 bytes, as disks are sold).
    FreeGb,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DiskWatch {
    /// Any folder on the disk, for example "/" or "~".
    pub path: String,
    #[serde(default)]
    pub measure: DiskMeasure,
}

/// What about a file counts.
#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileChange {
    /// Its size or modification time (a file appearing or disappearing
    /// counts too).
    #[default]
    Contents,
    /// Its permissions and owner.
    Permissions,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FileWatch {
    /// A file or folder; ~ is the home folder.
    pub path: String,
    #[serde(default)]
    pub watch_for: FileChange,
}

fn check_path(path: &str) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("the path is empty".into());
    }
    if path.len() > MAX_PATH || path.chars().any(char::is_control) {
        return Err("the path is not a usable path".into());
    }
    Ok(())
}

impl DiskWatch {
    pub fn check(&self) -> Result<(), String> {
        check_path(&self.path)
    }
}

impl FileWatch {
    pub fn check(&self) -> Result<(), String> {
        check_path(&self.path)
    }
}

/// `~` and `~/...` as the home folder.
fn expand(path: &str) -> PathBuf {
    let path = path.trim();
    match (path.strip_prefix('~'), dirs::home_dir()) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => home.join(rest.trim_start_matches('/')),
        _ => PathBuf::from(path),
    }
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn gb(bytes: f64) -> f64 {
    round1(bytes / 1e9)
}

// ---- CPU ----------------------------------------------------------------------

/// Idle and total time from the first line of /proc/stat.
#[cfg(target_os = "linux")]
fn cpu_times() -> Result<(u64, u64), String> {
    let text = std::fs::read_to_string("/proc/stat").map_err(|e| format!("could not read /proc/stat: {e}"))?;
    parse_cpu_line(text.lines().next().unwrap_or_default())
}

fn parse_cpu_line(line: &str) -> Result<(u64, u64), String> {
    let numbers: Vec<u64> = line.split_whitespace().skip(1).filter_map(|n| n.parse().ok()).collect();
    if numbers.len() < 4 {
        return Err("/proc/stat has no cpu line".into());
    }
    // user nice system idle iowait irq softirq steal: idle and iowait are idle.
    let idle = numbers[3] + numbers.get(4).copied().unwrap_or(0);
    let total = numbers.iter().take(8).sum();
    Ok((idle, total))
}

/// Per cent of the CPU in use, all cores together, over one second.
pub async fn cpu() -> Result<Value, String> {
    #[cfg(target_os = "linux")]
    {
        let (idle1, total1) = cpu_times()?;
        tokio::time::sleep(CPU_SAMPLE).await;
        let (idle2, total2) = cpu_times()?;
        let total = total2.saturating_sub(total1).max(1) as f64;
        let busy = round1(100.0 * (1.0 - idle2.saturating_sub(idle1) as f64 / total)).clamp(0.0, 100.0);
        Ok(json!({ "text": format!("{busy}%"), "number": busy }))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = CPU_SAMPLE;
        Err("CPU load is read on Linux only so far".into())
    }
}

// ---- Memory -------------------------------------------------------------------

fn parse_meminfo(text: &str) -> Result<(f64, f64), String> {
    let field = |name: &str| {
        text.lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|n| n.parse::<f64>().ok())
            .map(|kib| kib * 1024.0)
    };
    match (field("MemTotal:"), field("MemAvailable:")) {
        (Some(total), Some(available)) if total > 0.0 => Ok((total, available)),
        _ => Err("/proc/meminfo has no MemTotal and MemAvailable".into()),
    }
}

/// Per cent of memory (RAM) in use: what is not available to programs.
pub fn memory() -> Result<Value, String> {
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/meminfo").map_err(|e| format!("could not read /proc/meminfo: {e}"))?;
        let (total, available) = parse_meminfo(&text)?;
        let used = total - available;
        let percent = round1(100.0 * used / total);
        Ok(json!({ "text": format!("{} of {} GB used ({percent}%)", gb(used), gb(total)), "number": percent }))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = parse_meminfo;
        Err("memory use is read on Linux only so far".into())
    }
}

// ---- Disk ---------------------------------------------------------------------

/// Used per cent (as `df` counts it: of the space ordinary users can have)
/// or gigabytes free, for the disk holding `path`.
pub fn disk(w: &DiskWatch) -> Result<Value, String> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let path = expand(&w.path);
        let c = CString::new(path.as_os_str().as_bytes()).map_err(|_| "the path contains a NUL byte".to_string())?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c` is a valid NUL-terminated string and `st` a writable
        // statvfs for the call to fill in.
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return Err(format!("could not read the disk of {}: {}", path.display(), std::io::Error::last_os_error()));
        }
        let unit = st.f_frsize as f64;
        let used = (st.f_blocks as f64 - st.f_bfree as f64) * unit;
        let free = st.f_bavail as f64 * unit;
        let percent = if used + free > 0.0 { round1(100.0 * used / (used + free)) } else { 0.0 };
        let text = format!("{percent}% used, {} GB free", gb(free));
        Ok(match w.measure {
            DiskMeasure::UsedPercent => json!({ "text": text, "number": percent }),
            DiskMeasure::FreeGb => json!({ "text": text, "number": gb(free) }),
        })
    }
    #[cfg(not(unix))]
    {
        let _ = (w, expand("~"));
        Err("disk space is read on Linux only so far".into())
    }
}

// ---- Graphics card memory -----------------------------------------------------

fn parse_nvidia_smi(text: &str) -> Result<(f64, f64), String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).ok_or("nvidia-smi printed nothing")?;
    let mut parts = line.split(',').map(|p| p.trim().parse::<f64>());
    match (parts.next(), parts.next()) {
        (Some(Ok(used)), Some(Ok(total))) if total > 0.0 => Ok((used, total)),
        _ => Err(format!("nvidia-smi printed something unexpected: {line}")),
    }
}

/// Per cent of the first NVIDIA graphics card's memory in use, from
/// `nvidia-smi` (in MiB).
pub async fn vram() -> Result<Value, String> {
    let run = tokio::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.used,memory.total", "--format=csv,noheader,nounits"])
        .kill_on_drop(true)
        .output();
    let out = match tokio::time::timeout(NVIDIA_SMI_WAIT, run).await {
        Err(_) => return Err(format!("nvidia-smi did not answer within {} seconds", NVIDIA_SMI_WAIT.as_secs())),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err("nvidia-smi is not installed (it reads NVIDIA graphics card memory)".into())
        }
        Ok(Err(e)) => return Err(format!("nvidia-smi could not be run: {e}")),
        Ok(Ok(out)) => out,
    };
    if !out.status.success() {
        return Err(format!("nvidia-smi failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let (used, total) = parse_nvidia_smi(&String::from_utf8_lossy(&out.stdout))?;
    let percent = round1(100.0 * used / total);
    let mib = 1024.0 * 1024.0;
    Ok(json!({ "text": format!("{} of {} GB used ({percent}%)", gb(used * mib), gb(total * mib)), "number": percent }))
}

// ---- A file -------------------------------------------------------------------

fn local_time(at: SystemTime) -> String {
    let at = time::OffsetDateTime::from(at);
    let at = at.to_offset(crate::automation::system_offset(at));
    format!(
        "{}-{:02}-{:02} {:02}:{:02}:{:02}",
        at.year(),
        at.month() as u8,
        at.day(),
        at.hour(),
        at.minute(),
        at.second()
    )
}

#[cfg(unix)]
fn mode_text(mode: u32) -> String {
    let bits = ['r', 'w', 'x'];
    let letters: String = (0..9).map(|i| if mode & (0o400 >> i) != 0 { bits[i % 3] } else { '-' }).collect();
    format!("{letters} ({:03o})", mode & 0o777)
}

/// What the file is like now, as text: "missing", its size and
/// modification time, or its permissions and owner.
pub fn file(w: &FileWatch) -> Result<Value, String> {
    let path = expand(&w.path);
    let meta = match std::fs::metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(json!({ "text": "missing" })),
        Err(e) => return Err(format!("could not read {}: {e}", path.display())),
    };
    let text = match w.watch_for {
        FileChange::Contents => {
            let modified = meta.modified().map(local_time).unwrap_or_else(|_| "unknown".into());
            if meta.is_dir() {
                format!("folder, modified {modified}")
            } else {
                format!("{} bytes, modified {modified}", meta.len())
            }
        }
        FileChange::Permissions => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                format!("{}, owner {}, group {}", mode_text(meta.mode()), meta.uid(), meta.gid())
            }
            #[cfg(not(unix))]
            {
                if meta.permissions().readonly() { "read-only".to_string() } else { "writable".to_string() }
            }
        }
    };
    Ok(json!({ "text": text }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_and_nvidia_smi_text_is_parsed() {
        assert_eq!(parse_cpu_line("cpu  100 0 50 800 50 0 0 0 0 0"), Ok((850, 1000)));
        assert!(parse_cpu_line("intr 1 2").is_err());
        let meminfo = "MemTotal:       65536000 kB\nMemFree:  1 kB\nMemAvailable:   49152000 kB\n";
        let (total, available) = parse_meminfo(meminfo).unwrap();
        assert_eq!((total / 1024.0, available / 1024.0), (65536000.0, 49152000.0));
        assert!(parse_meminfo("MemFree: 1 kB").is_err());
        assert_eq!(parse_nvidia_smi("3120, 12288\n"), Ok((3120.0, 12288.0)));
        assert!(parse_nvidia_smi("[N/A], [N/A]").is_err());
        #[cfg(unix)]
        assert_eq!(mode_text(0o100644), "rw-r--r-- (644)");
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn this_computer_can_be_read() {
        let cpu = cpu().await.unwrap();
        assert!((0.0..=100.0).contains(&cpu["number"].as_f64().unwrap()), "{cpu}");
        let mem = memory().unwrap();
        assert!(mem["text"].as_str().unwrap().contains("GB used"), "{mem}");
        let root = disk(&DiskWatch { path: "/".into(), measure: DiskMeasure::UsedPercent }).unwrap();
        assert!((0.0..=100.0).contains(&root["number"].as_f64().unwrap()), "{root}");
        let free = disk(&DiskWatch { path: "~".into(), measure: DiskMeasure::FreeGb }).unwrap();
        assert!(free["number"].as_f64().unwrap() >= 0.0, "{free}");
        assert!(disk(&DiskWatch { path: "/no/such/folder".into(), measure: DiskMeasure::UsedPercent }).is_err());
        // nvidia-smi may be missing; either way the answer is a reading or a sentence.
        match vram().await {
            Ok(v) => assert!(v["number"].as_f64().is_some(), "{v}"),
            Err(e) => assert!(!e.is_empty()),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_file_watch_sees_contents_and_permissions_change() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("rust_scaffold_hw_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("watched.txt");
        let contents = FileWatch { path: path.display().to_string(), watch_for: FileChange::Contents };
        let perms = FileWatch { path: path.display().to_string(), watch_for: FileChange::Permissions };
        assert_eq!(file(&contents).unwrap()["text"], "missing");
        std::fs::write(&path, "one").unwrap();
        let first = file(&contents).unwrap();
        assert!(first["text"].as_str().unwrap().starts_with("3 bytes, modified "), "{first}");
        assert!(first.get("number").is_none(), "no number: a change of time alone must count");
        std::fs::write(&path, "three").unwrap();
        assert_ne!(file(&contents).unwrap(), first);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let before = file(&perms).unwrap();
        assert!(before["text"].as_str().unwrap().starts_with("rw-r--r-- (644), owner "), "{before}");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(file(&perms).unwrap()["text"].as_str().unwrap().starts_with("rw------- (600)"));
        assert!(FileWatch { path: " ".into(), watch_for: FileChange::Contents }.check().is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
