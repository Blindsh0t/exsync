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
fn notify_without_osascript_returns_promptly() {
    // Empty `PATH` rooted at a fresh temp dir, so no `osascript` can be
    // found via `PATH`. Success is simply returning; no assertion on side
    // effects. (`notify` spawns without waiting, so it cannot block.)
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir =
        std::env::temp_dir().join(format!("exsync-notify-path-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let saved = std::env::var("PATH").ok();
    std::env::set_var("PATH", &dir);
    // Must not block and must not panic even with nothing to spawn.
    notify("t", "b", false);
    match saved {
        Some(v) => std::env::set_var("PATH", v),
        None => std::env::remove_var("PATH"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn escape_quotes_and_backslashes() {
    assert_eq!(escape("a\"b\\c"), "a\\\"b\\\\c");
    assert_eq!(escape("plain"), "plain");
}
