//! Tests for config.rs (UT-01, UT-02, UT-03 + malformed cases).

use super::*;
use std::path::PathBuf;

fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/Users".to_string())
}

// UT-01: valid version parses.
#[test]
fn ut01_valid_version_parses() {
    let text = "version = 1\n\n[[entry]]\nname = \"a\"\nmode = \"mirror\"\ndrive = \"D\"\nsource = \"~/x\"\ndest = \"d/x\"\n";
    let entries = parse(text).expect("valid config should parse");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "a");
}

// UT-01: version 2 fails with code 2.
#[test]
fn ut01_version_mismatch_fails_code2() {
    let text = "version = 2\n\n[[entry]]\nname = \"a\"\nmode = \"mirror\"\ndrive = \"D\"\nsource = \"~/x\"\ndest = \"d/x\"\n";
    let err = parse(text).expect_err("version 2 must fail");
    assert_eq!(err.code(), 2);
    let msg = format!("{err}");
    assert!(msg.contains('2'), "message names found value: {msg}");
    assert!(msg.contains("expected version = 1"), "message: {msg}");
}

// UT-01: missing version fails with code 2.
#[test]
fn ut01_missing_version_fails() {
    let text = "[[entry]]\nname = \"a\"\nmode = \"mirror\"\ndrive = \"D\"\nsource = \"~/x\"\ndest = \"d/x\"\n";
    // Entry before version is also rejected with code 2.
    let err = parse(text).expect_err("missing version must fail");
    assert_eq!(err.code(), 2);
    let empty: super::ConfigError = parse("").expect_err("empty must fail");
    assert_eq!(empty.code(), 2);
    assert!(format!("{empty}").contains("expected version = 1"));
}

// UT-02: unknown key fails and names the key.
#[test]
fn ut02_unknown_top_key_names_key() {
    let text = "version = 1\nbogus = \"x\"\n";
    let err = parse(text).expect_err("unknown top key must fail");
    assert_eq!(err.code(), 2);
    assert!(format!("{err}").contains("bogus"));
}

// UT-02: unknown entry key fails and names the key.
#[test]
fn ut02_unknown_entry_key_names_key() {
    let text = "version = 1\n\n[[entry]]\nname = \"a\"\nmode = \"mirror\"\ndrive = \"D\"\nsource = \"~/x\"\ndest = \"d/x\"\nnope = 1\n";
    let err = parse(text).expect_err("unknown entry key must fail");
    assert_eq!(err.code(), 2);
    assert!(format!("{err}").contains("nope"));
}

// UT-03: fixture entries parse with every field, tilde expanded under $HOME.
#[test]
fn ut03_fixture_entries_parse() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let path = PathBuf::from(manifest_dir).join("test-fixtures/cfg.toml");
    let entries = load(&path).expect("fixture should parse");
    assert_eq!(entries.len(), 2);

    let dot = &entries[0];
    assert_eq!(dot.name, "dotfiles");
    assert_eq!(dot.mode, Mode::Mirror);
    assert_eq!(dot.drive, "KINGSTON");
    assert_eq!(dot.dest, "05_Programming/dotfiles");
    assert_eq!(dot.exclude, vec![".DS_Store".to_string()]);
    assert!(dot.include.is_empty());
    assert_eq!(dot.name_re, None);
    assert_eq!(dot.min_age_minutes, None);
    assert!(!dot.skip_open);
    assert_eq!(dot.verify, Verify::Readback);
    let h = home();
    assert!(
        dot.source.starts_with(&h),
        "source {:?} should start with HOME {h}",
        dot.source
    );
    assert_eq!(
        dot.source,
        PathBuf::from(&h).join("Documents/Programming/dotfiles")
    );

    let handy = &entries[1];
    assert_eq!(handy.name, "handy-audio");
    assert_eq!(handy.mode, Mode::Move);
    assert_eq!(handy.drive, "KINGSTON");
    assert_eq!(
        handy.dest,
        "05_Programming/AI/dictation/handy-voice-recordings"
    );
    assert_eq!(handy.include, vec!["*.wav".to_string()]);
    assert!(handy.exclude.is_empty());
    assert_eq!(handy.min_age_minutes, Some(5));
    assert!(handy.skip_open);
    assert_eq!(handy.verify, Verify::Readback);
    assert_eq!(
        handy.source,
        PathBuf::from(&h).join("Library/Application Support/com.pais.handy/recordings")
    );
}

#[test]
fn malformed_bad_quote_is_error() {
    let text = "version = 1\n\n[[entry]]\nname = \"oops\nmode = \"mirror\"\ndrive = \"D\"\nsource = \"~/x\"\ndest = \"d/x\"\n";
    let err = parse(text).expect_err("bad quote must be error, not panic");
    assert_eq!(err.code(), 2);
}

#[test]
fn malformed_missing_value_is_error() {
    let text = "version = 1\n\n[[entry]]\nname =\nmode = \"mirror\"\ndrive = \"D\"\nsource = \"~/x\"\ndest = \"d/x\"\n";
    let err = parse(text).expect_err("`key =` must be error, not panic");
    assert_eq!(err.code(), 2);
}

#[test]
fn malformed_unknown_mode_is_error() {
    let text = "version = 1\n\n[[entry]]\nname = \"a\"\nmode = \"warp\"\ndrive = \"D\"\nsource = \"~/x\"\ndest = \"d/x\"\n";
    let err = parse(text).expect_err("unknown mode must be error, not panic");
    assert_eq!(err.code(), 2);
}

#[test]
fn malformed_missing_drive_is_error() {
    let text = "version = 1\n\n[[entry]]\nname = \"a\"\nmode = \"mirror\"\nsource = \"~/x\"\ndest = \"d/x\"\n";
    let err = parse(text).expect_err("missing drive must be error, not panic");
    assert_eq!(err.code(), 2);
    assert!(format!("{err}").contains("drive"));
}

#[test]
fn missing_file_names_path() {
    let err =
        load(Path::new("/tmp/exsync-test-missing-cfg-12345.toml")).expect_err("must fail");
    assert_eq!(err.code(), 2);
    assert!(format!("{err}").contains("/tmp/exsync-test-missing-cfg-12345.toml"));
}
