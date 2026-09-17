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
    let cfg_path = config_path();
    let entries = match config::load(&cfg_path) {
        Ok(e) => e,
        Err(e) => {
            log::log_line(&format!("FAIL config - 0 {}", reason_token(&e.to_string())));
            eprintln!("exsync: cannot load config '{}': {e}", cfg_path.display());
            return 2;
        }
    };
    let vol_root = volumes_root();
    let mounted = entries
        .iter()
        .filter(|e| is_mounted(&vol_root, &e.drive))
        .count();

    if mounted == 0 {
        log::log_line(&format!("OK run entries={} mounted=0", entries.len()));
        return 0;
    }

    let mut failed = false;
    for entry in &entries {
        if !is_mounted(&vol_root, &entry.drive) {
            log::log_line(&log::action_line(
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
            log::log_line(&log::action_line("FAIL", &entry.name, "-", 0, &reason));
            failed = true;
        }
    }
    log::log_line(&format!(
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
