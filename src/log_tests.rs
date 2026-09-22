use super::action_line;
use super::format_unix_secs;
use super::log_line;

#[test]
fn timestamp_epoch_zero_is_utc_midnight() {
    assert_eq!(format_unix_secs(0), "1970-01-01 00:00:00");
}

#[test]
fn timestamp_format_is_19_chars() {
    let ts = format_unix_secs(0);
    assert_eq!(ts.len(), 19);
    let b = ts.as_bytes();
    assert_eq!(b[4], b'-');
    assert_eq!(b[7], b'-');
    assert_eq!(b[10], b' ');
    assert_eq!(b[13], b':');
    assert_eq!(b[16], b':');
}

#[test]
fn action_line_has_five_fields() {
    let line = action_line("OK", "entry", "file", 123, "reason");
    assert_eq!(line, "OK entry file 123 reason");
    assert_eq!(line.split(' ').count(), 5);
}

#[test]
fn log_line_returns_err_when_parent_uncreatable() {
    // A path segment that is an existing regular file: the parent directory
    // can never be created, so the write must fail with `Err`, not panic.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let blocker =
        std::env::temp_dir().join(format!("exsync-log-blocker-{}-{nanos}", std::process::id()));
    std::fs::write(&blocker, b"blocker").unwrap();
    let bad = blocker.join("child/exsync.log");
    let saved = std::env::var("EXSYNC_LOG").ok();
    std::env::set_var("EXSYNC_LOG", &bad);
    let result = log_line("uncreatable-parent");
    match saved {
        Some(v) => std::env::set_var("EXSYNC_LOG", v),
        None => std::env::remove_var("EXSYNC_LOG"),
    }
    let _ = std::fs::remove_file(&blocker);
    assert!(result.is_err(), "log_line must return Err, not panic");
}
