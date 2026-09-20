//! Manifest module: Append-only move records.
//!
//! Each move entry keeps one manifest file `<dest>/.exsync-manifest.<entry>`
//! with one line per moved file: `<file>\t<size>\t<YYYY-MM-DDTHH:MM:SSZ>\n`
//! (UTC). The manifest is a record for the user, never a directory listing
//! and never file contents; recovery never depends on it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MANIFEST_PREFIX: &str = ".exsync-manifest.";

fn manifest_file(dest_dir: &Path, entry: &str) -> PathBuf {
    dest_dir.join(format!("{MANIFEST_PREFIX}{entry}"))
}

/// Convert days since the Unix epoch to (year, month, day) with
/// Howard Hinnant's civil-from-days arithmetic (same basis as log.rs).
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

/// Format a timestamp as UTC `%Y-%m-%dT%H:%M:%SZ`.
fn format_manifest_ts(ts: SystemTime) -> String {
    let secs = ts
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let (y, mo, d) = civil_from_days(days);
    format!(
        "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn write_line(file: &Path, name: &str, size: u64, timestamp: SystemTime) -> io::Result<()> {
    if let Some(parent) = file.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    use std::io::Write;
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)?;
    writeln!(f, "{name}\t{size}\t{}", format_manifest_ts(timestamp))?;
    Ok(())
}

/// Append one line `<name>\t<size>\t<timestamp>\n` to
/// `<dest_dir>/.exsync-manifest.<name>`, creating the file on first use.
///
/// This is the specified four-argument form, exercised by IT-11. Move mode
/// records per-file lines in the entry-scoped file via `append_entry`; this
/// form is otherwise unused by the binary, hence the scoped `allow`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn append(dest_dir: &Path, name: &str, size: u64, timestamp: SystemTime) -> io::Result<()> {
    write_line(&manifest_file(dest_dir, name), name, size, timestamp)
}

/// Append one line for a moved file to the entry's manifest file
/// `<dest_dir>/.exsync-manifest.<entry>`, creating it on first use.
/// `file` is the destination-relative path of the moved file.
pub fn append_entry(
    dest_dir: &Path,
    entry: &str,
    file: &str,
    size: u64,
    timestamp: SystemTime,
) -> io::Result<()> {
    write_line(&manifest_file(dest_dir, entry), file, size, timestamp)
}

/// Report whether `name` already has a line in any `.exsync-manifest.*`
/// file under `dest_dir`. Used for idempotent completion: a committed
/// destination file with no manifest line gets one appended on re-run.
pub fn last_entry(dest_dir: &Path, name: &str) -> io::Result<bool> {
    let entries = match fs::read_dir(dest_dir) {
        Ok(r) => r,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    for item in entries.flatten() {
        let fname = item.file_name().to_string_lossy().into_owned();
        if !fname.starts_with(MANIFEST_PREFIX) {
            continue;
        }
        let content = fs::read_to_string(item.path())?;
        for line in content.lines() {
            if line.split('\t').next() == Some(name) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
