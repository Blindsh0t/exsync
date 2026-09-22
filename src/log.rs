//! Log module: timestamped append-only log.
//!
//! Timestamps are UTC; no local timezone offset is applied. The Unix epoch
//! (0) therefore formats as `1970-01-01 00:00:00`.

use std::io;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Resolve the log file path: `EXSYNC_LOG` wins when set and non-empty;
/// otherwise `$HOME/Library/Logs/exsync.log`. A missing or empty `HOME`
/// with no `EXSYNC_LOG` is an error naming `HOME`, never a silent fallback:
/// a launchd user agent is not guaranteed to export `HOME`, and falling back
/// to `/Users/Library/...` would hide the run.
pub fn log_path() -> io::Result<PathBuf> {
    if let Ok(p) = std::env::var("EXSYNC_LOG") {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() => Ok(PathBuf::from(h).join("Library/Logs/exsync.log")),
        _ => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "HOME is unset or empty; set EXSYNC_LOG or HOME",
        )),
    }
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
/// creating parent directories. Best-effort: on any failure one line is
/// written to stderr in the form
/// `exsync: log write failed (<path>): <error>` and the error is returned.
/// Callers must ignore it, so a failing log never changes an exit code and
/// never panics.
pub fn log_line(line: &str) -> io::Result<()> {
    let path = match log_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("exsync: log write failed (<unknown>): {e}");
            return Err(e);
        }
    };
    let text = format!("{} exsync: {line}\n", timestamp_now());
    let result = (|| -> std::io::Result<()> {
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
    if let Err(ref e) = result {
        eprintln!("exsync: log write failed ({}): {e}", path.display());
    }
    result
}

#[cfg(test)]
#[path = "log_tests.rs"]
mod tests;
