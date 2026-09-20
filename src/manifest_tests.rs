//! IT-11: manifest lines are name, size, timestamp; never a listing.

use super::*;
use std::time::Duration;

fn unique_dir(tag: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "exsync-manifest-{tag}-{}-{nanos}-{n}",
        std::process::id()
    ))
}

/// Check `s` parses as `%Y-%m-%dT%H:%M:%SZ` with sane field ranges.
fn assert_valid_ts(s: &str) {
    let b = s.as_bytes();
    assert_eq!(s.len(), 20, "timestamp length: {s}");
    assert_eq!((b[4], b[7], b[10], b[13], b[16], b[19]), (b'-', b'-', b'T', b':', b':', b'Z'));
    for (i, c) in s.chars().enumerate() {
        if [4, 7, 10, 13, 16, 19].contains(&i) {
            continue;
        }
        assert!(c.is_ascii_digit(), "digit at {i}: {s}");
    }
    let num = |a: usize, b: usize| s[a..b].parse::<u32>().unwrap();
    assert!((1..=12).contains(&num(5, 7)), "month: {s}");
    assert!((1..=31).contains(&num(8, 10)), "day: {s}");
    assert!((0..=23).contains(&num(11, 13)), "hour: {s}");
    assert!((0..=59).contains(&num(14, 16)), "minute: {s}");
    assert!((0..=59).contains(&num(17, 19)), "second: {s}");
}

#[test]
fn it11_manifest_lines_hold_name_size_timestamp_only() {
    let dir = unique_dir("it11");
    // A data file with distinctive contents: the manifest must never
    // absorb file contents or degenerate into a directory listing.
    std::fs::create_dir_all(&dir).unwrap();
    let distinctive = "S3CR3T-payload-never-in-manifest-७";
    std::fs::write(dir.join("cargo.wav"), distinctive).unwrap();

    let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_757_000_000);
    append(&dir, "meter", 5, t0).unwrap();
    append(&dir, "meter", 7, t0 + Duration::from_secs(60)).unwrap();

    let content = std::fs::read_to_string(dir.join(".exsync-manifest.meter")).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2, "two appends, two lines: {content:?}");
    for line in &lines {
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(fields.len(), 3, "exactly three tab fields: {line:?}");
        fields[1].parse::<u64>().expect("size parses as u64");
        assert_valid_ts(fields[2]);
    }
    assert!(
        !content.contains(distinctive),
        "no file contents in manifest"
    );
    // Not a directory listing: the manifest file itself lives in the dir
    // but is never listed as a line, and only appended names appear.
    for line in &lines {
        assert!(
            !line.contains(".exsync-manifest"),
            "manifest never lists itself: {line:?}"
        );
    }

    assert!(last_entry(&dir, "meter").unwrap());
    assert!(!last_entry(&dir, "absent").unwrap());

    let _ = std::fs::remove_dir_all(&dir);
}
