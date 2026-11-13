// Build script. Runs Tauri's own build step, then records build details for
// the Help page (build time, Rust and Cargo versions, profile, the crates the
// app depends on directly and the total crate count). Uses no extra crates:
// Cargo.lock is read with plain line parsing.

use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    tauri_build::build();

    // Re-run (and so refresh the build time) whenever anything that goes into
    // the binary changes. A new Rust toolchain also re-runs build scripts.
    for path in ["build.rs", "Cargo.toml", "Cargo.lock", "src", "../ui"] {
        println!("cargo:rerun-if-changed={path}");
    }

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=BUILD_UNIX_TIME={secs}");
    println!("cargo:rustc-env=BUILD_RUSTC={}", tool_version("RUSTC", "rustc"));
    println!("cargo:rustc-env=BUILD_CARGO={}", tool_version("CARGO", "cargo"));
    println!(
        "cargo:rustc-env=BUILD_PROFILE={}",
        env::var("PROFILE").unwrap_or_else(|_| "unknown".into())
    );

    let lock = fs::read_to_string(Path::new("Cargo.lock")).unwrap_or_default();
    let (direct, total) = crates_from_lock(&lock, &env::var("CARGO_PKG_NAME").unwrap_or_default());
    println!("cargo:rustc-env=BUILD_CRATES={}", direct.join(";"));
    println!("cargo:rustc-env=BUILD_CRATE_COUNT={total}");
}

/// First line of `<tool> -V`, for example "rustc 1.100.0 (0ef1bc386 2026-11-09)".
fn tool_version(var: &str, fallback: &str) -> String {
    let tool = env::var(var).unwrap_or_else(|_| fallback.into());
    Command::new(tool)
        .arg("-V")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.lines().next().map(str::trim).map(String::from))
        .unwrap_or_else(|| "unknown".into())
}

/// Returns ("name version" for each direct dependency of `root`, number of
/// packages in the lock file other than `root`).
fn crates_from_lock(lock: &str, root: &str) -> (Vec<String>, usize) {
    // Each package: (name, version, dependency entries).
    let mut packages: Vec<(String, String, Vec<String>)> = Vec::new();
    let mut in_deps = false;
    for line in lock.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            packages.push((String::new(), String::new(), Vec::new()));
            in_deps = false;
        } else if let Some(p) = packages.last_mut() {
            if let Some(v) = line.strip_prefix("name = ") {
                p.0 = v.trim_matches('"').to_string();
            } else if let Some(v) = line.strip_prefix("version = ") {
                p.1 = v.trim_matches('"').to_string();
            } else if line == "dependencies = [" {
                in_deps = true;
            } else if in_deps && line == "]" {
                in_deps = false;
            } else if in_deps {
                p.2.push(line.trim_end_matches(',').trim_matches('"').to_string());
            }
        }
    }

    let total = packages.iter().filter(|p| p.0 != root).count();
    let Some(root_pkg) = packages.iter().find(|p| p.0 == root) else {
        return (Vec::new(), total);
    };
    // An entry is "name" when only one version is locked, otherwise
    // "name version" (optionally followed by a source in brackets).
    let direct = root_pkg
        .2
        .iter()
        .map(|entry| {
            let mut parts = entry.split_whitespace();
            let name = parts.next().unwrap_or_default();
            let version = parts.next().map(String::from).or_else(|| {
                packages.iter().find(|p| p.0 == name).map(|p| p.1.clone())
            });
            format!("{name} {}", version.unwrap_or_else(|| "?".into()))
        })
        .collect();
    (direct, total)
}
