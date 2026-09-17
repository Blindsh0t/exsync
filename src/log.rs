//! Log module: timestamped append-only log.
//!
//! Timestamps are UTC; no local timezone offset is applied. The Unix epoch
//! (0) therefore formats as `1970-01-01 00:00:00`.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Resolve the log file path: `EXSYNC_LOG`, default
/// `$HOME/Library/Logs/exsync.log`.
fn log_path() -> PathBuf {
    if let Ok(p) = std::env::var("EXSYNC_LOG") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users".to_string());
    PathBuf::from(home).join("Library/Logs/exsync.log")
}

/// Convert days since the Unix epoch to (year, month, day) with
/// Howard Hinnant's civil-from-days arithmetic.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let zz = z + 719468;
    let era = if zz >= 0 { zz } else { zz - 146096 } / 146097;
    let doe = zz - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    if m <= 2 {
        y += 1;
    }
    (y, m, d)
}

fn format_unix_secs(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02} {hour:02}:{min:02}:{sec:02}")
}

fn timestamp_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_unix_secs(secs)
}

/// Build `<action> <entry> <file> <size> <reason>`. Callers pass single-token
/// fields; this function only joins them and never reads file contents.
pub fn action_line(action: &str, entry: &str, file: &str, size: u64, reason: &str) -> String {
    format!("{action} {entry} {file} {size} {reason}")
}

/// Append `<YYYY-MM-DD HH:MM:SS> exsync: <line>\n` to the log file,
/// creating parent directories. A write failure is swallowed: logging
/// never fails a run and never panics.
pub fn log_line(line: &str) {
    let path = log_path();
    let text = format!("{} exsync: {line}\n", timestamp_now());
    let _ = (|| -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        f.write_all(text.as_bytes())?;
        Ok(())
    })();
}

#[cfg(test)]
#[path = "log_tests.rs"]
mod tests;
