//! Dispatch module: config load, mount scan, per-entry dispatch.
//!
//! One invocation writes exactly one run line
//! (`OK run entries=<n> mounted=<m>`). When no configured drive is mounted,
//! that run line is the only log output; per-entry SKIP lines are emitted
//! only when at least one drive is mounted.

use std::path::{Path, PathBuf};

use crate::{cli, config, log, mirror, move_files};

fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("EXSYNC_CONFIG") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users".to_string());
    PathBuf::from(home).join("Library/Application Support/exsync/exsync.toml")
}

fn volumes_root() -> PathBuf {
    if let Ok(p) = std::env::var("EXSYNC_VOLUMES_ROOT") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    PathBuf::from("/Volumes")
}

fn is_mounted(root: &Path, drive: &str) -> bool {
    root.join(drive).is_dir()
}

/// Best-effort log append: when a log write fails, print exactly one
/// stderr line and continue the run. Success stays silent on stdout and
/// stderr, and a failing log never changes the exit code.
fn log_best_effort(line: &str) {
    if let Err(e) = log::log_line(line) {
        let path = log::log_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "<unknown>".to_string());
        eprintln!("exsync: cannot write log ({path}): {e}");
    }
}

/// A launchd user agent is not guaranteed to export `HOME`.
fn home_is_missing() -> bool {
    std::env::var("HOME").map(|h| h.is_empty()).unwrap_or(true)
}

/// Collapse an error message to a single log-safe token.
fn reason_token(msg: &str) -> String {
    if msg == "not implemented" {
        return "not-implemented".to_string();
    }
    let mut out = String::with_capacity(msg.len());
    for ch in msg.chars() {
        if ch.is_whitespace() {
            out.push('-');
        } else {
            out.push(ch);
        }
    }
    if out.is_empty() {
        "error".to_string()
    } else {
        out
    }
}

/// Run every configured entry whose drive is mounted, in config order.
/// Never writes anywhere above `<volumes-root>/<drive>`; the only
/// filesystem writes here are the log appends (mode stubs write nothing).
pub fn run(options: &cli::Options) -> i32 {
    if home_is_missing() {
        eprintln!("exsync: HOME is unset; set EXSYNC_LOG or HOME");
    }
    let cfg_path = config_path();
    let entries = match config::load(&cfg_path) {
        Ok(e) => e,
        Err(e) => {
            log_best_effort(&format!("FAIL config - 0 {}", reason_token(&e.to_string())));
            eprintln!("exsync: cannot load config '{}': {e}", cfg_path.display());
            return e.code();
        }
    };
    let vol_root = volumes_root();
    let mounted = entries
        .iter()
        .filter(|e| is_mounted(&vol_root, &e.drive))
        .count();

    if mounted == 0 {
        log_best_effort(&format!("OK run entries={} mounted=0", entries.len()));
        return 0;
    }

    let mut failed = false;
    for entry in &entries {
        if !is_mounted(&vol_root, &entry.drive) {
            log_best_effort(&log::action_line(
                "SKIP",
                &entry.name,
                "-",
                0,
                "drive-not-mounted",
            ));
            continue;
        }
        // `move_files::run` is a placeholder until task MV-01 lands.
        let result = match entry.mode {
            config::Mode::Mirror => mirror::run(entry, &vol_root, options.dry_run),
            config::Mode::Move => move_files::run(entry, &vol_root, options.dry_run),
        };
        if let Err(e) = result {
            let reason = reason_token(&e);
            log_best_effort(&log::action_line("FAIL", &entry.name, "-", 0, &reason));
            failed = true;
        }
    }
    log_best_effort(&format!(
        "OK run entries={} mounted={}",
        entries.len(),
        mounted
    ));
    if failed {
        1
    } else {
        0
    }
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
