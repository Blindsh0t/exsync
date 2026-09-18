//! Notify module: macOS system notifications.
//!
//! One notification per error, silent success otherwise. In `dry_run` mode
//! nothing is spawned; the notification is printed to stdout instead.

use std::process::Command;

/// Escape `"` and `\` for embedding in an AppleScript string.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch == '\\' || ch == '"' {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Send a macOS notification. With `dry_run`, print
/// `NOTIFY <title>: <body>` to stdout and run nothing. Otherwise run
/// `/usr/bin/osascript` with stdout and stderr discarded; a missing
/// `osascript` binary is not an error.
pub fn notify(title: &str, body: &str, dry_run: bool) {
    if dry_run {
        println!("NOTIFY {title}: {body}");
        return;
    }
    let title = escape(title);
    let body = escape(body);
    let script = format!("display notification \"{body}\" with title \"{title}\"");
    match Command::new("/usr/bin/osascript")
        .args(["-e", script.as_str()])
        .output()
    {
        Ok(_) => {}
        Err(_) => {}
    }
}

#[cfg(test)]
#[path = "notify_tests.rs"]
mod tests;
