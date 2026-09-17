use super::action_line;
use super::format_unix_secs;

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
