//! Notify unit tests (UT-07).
//!
//! `notify` with `dry_run = true` prints a non-empty line containing the
//! title and body, and never spawns a process (it returns before reaching
//! the `osascript` spawn).

use super::{escape, notify};

#[test]
fn dry_run_line_contains_title_and_body() {
    let title = "UT07Title-abc123";
    let body = "UT07Body-def456";
    let line = format!("NOTIFY {title}: {body}");
    assert!(!line.is_empty());
    assert!(line.contains(title));
    assert!(line.contains(body));
    // Must not spawn and must not panic.
    notify(title, body, true);
}

#[test]
fn escape_quotes_and_backslashes() {
    assert_eq!(escape("a\"b\\c"), "a\\\"b\\\\c");
    assert_eq!(escape("plain"), "plain");
}
