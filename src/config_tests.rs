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

    let notes = &entries[0];
    assert_eq!(notes.name, "notes");
    assert_eq!(notes.mode, Mode::Mirror);
    assert_eq!(notes.drive, "BACKUP");
    assert_eq!(notes.dest, "backup/notes");
    assert_eq!(notes.exclude, vec![".DS_Store".to_string()]);
    assert!(notes.include.is_empty());
    assert_eq!(notes.name_re, None);
    assert_eq!(notes.min_age_minutes, None);
    assert!(!notes.skip_open);
    assert_eq!(notes.verify, Verify::Readback);
    let h = home();
    assert!(
        notes.source.starts_with(&h),
        "source {:?} should start with HOME {h}",
        notes.source
    );
    assert_eq!(notes.source, PathBuf::from(&h).join("Documents/notes"));

    let audio = &entries[1];
    assert_eq!(audio.name, "audio");
    assert_eq!(audio.mode, Mode::Move);
    assert_eq!(audio.drive, "BACKUP");
    assert_eq!(audio.dest, "backup/audio");
    assert_eq!(audio.include, vec!["*.wav".to_string()]);
    assert!(audio.exclude.is_empty());
    assert_eq!(audio.min_age_minutes, Some(5));
    assert!(audio.skip_open);
    assert_eq!(audio.verify, Verify::Readback);
    assert_eq!(audio.source, PathBuf::from(&h).join("recordings"));
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
