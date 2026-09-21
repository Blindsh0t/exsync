//! Move-mode integration tests: IT-07/08/09/10, collision, guard, empty.
//!
//! Each test spawns the real binary via `env!("CARGO_BIN_EXE_exsync")`
//! with its own temp dirs and `EXSYNC_*` env overrides, so the machine's
//! real `/Volumes` and `~/Library/Logs/exsync.log` are never used.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::SystemTime;

fn unique_base(tag: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "exsync-move-{tag}-{}-{nanos}-{n}",
        std::process::id()
    ))
}

fn write_file(path: &Path, contents: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn write_move_config(path: &Path, name: &str, drive: &str, source: &Path, dest: &str) {
    let text = format!(
        "version = 1\n\n[[entry]]\nname = \"{name}\"\nmode = \"move\"\ndrive = \"{drive}\"\nsource = \"{}\"\ndest = \"{dest}\"\n",
        source.display()
    );
    std::fs::write(path, text).unwrap();
}

fn run_exsync(
    cfg: &Path,
    vols: &Path,
    log: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
) -> Output {
    let bin = env!("CARGO_BIN_EXE_exsync");
    let mut cmd = Command::new(bin);
    cmd.env("EXSYNC_CONFIG", cfg);
    cmd.env("EXSYNC_VOLUMES_ROOT", vols);
    cmd.env("EXSYNC_LOG", log);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    for a in args {
        cmd.arg(a);
    }
    cmd.output().expect("spawn exsync")
}

fn run_plain(cfg: &Path, vols: &Path, log: &Path) -> Output {
    run_exsync(cfg, vols, log, &[], &[])
}

/// Every `.exsync-manifest.*` line in `dest`.
fn manifest_lines(dest: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let rd = match std::fs::read_dir(dest) {
        Ok(r) => r,
        Err(_) => return out,
    };
    for item in rd.flatten() {
        let name = item.file_name().to_string_lossy().into_owned();
        if !name.starts_with(".exsync-manifest.") {
            continue;
        }
        let content = std::fs::read_to_string(item.path()).unwrap();
        out.extend(content.lines().map(|l| l.to_string()));
    }
    out.sort();
    out
}

fn has_manifest_file(dest: &Path) -> bool {
    match std::fs::read_dir(dest) {
        Ok(rd) => rd.flatten().any(|item| {
            item.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(".exsync-manifest."))
        }),
        Err(_) => false,
    }
}

fn status(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

#[test]
fn it07_move_basic_copies_verifies_unlinks_and_records() {
    let base = unique_base("it07");
    let src = base.join("src");
    write_file(&src.join("a.wav"), b"audio-bytes");
    write_file(&src.join("sub/b.wav"), b"more-audio");
    let vols = base.join("vols");
    std::fs::create_dir_all(vols.join("KINGSTON")).unwrap();
    let cfg = base.join("cfg.toml");
    write_move_config(&cfg, "handy", "KINGSTON", &src, "audio");
    let log = base.join("exsync.log");

    let out = run_plain(&cfg, &vols, &log);
    assert_eq!(
        status(&out),
        0,
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let dest = vols.join("KINGSTON/audio");
    assert_eq!(std::fs::read(dest.join("a.wav")).unwrap(), b"audio-bytes");
    assert_eq!(std::fs::read(dest.join("sub/b.wav")).unwrap(), b"more-audio");
    assert!(!src.join("a.wav").exists(), "source unlinked after commit");
    assert!(!src.join("sub/b.wav").exists());
    let lines = manifest_lines(&dest);
    assert_eq!(lines.len(), 2, "one line per file: {lines:?}");
    for line in &lines {
        assert_eq!(line.split('\t').count(), 3, "three fields: {line:?}");
    }

    // Idempotent re-run changes nothing.
    let again = run_plain(&cfg, &vols, &log);
    assert_eq!(status(&again), 0);
    assert_eq!(manifest_lines(&dest).len(), 2, "no duplicate lines");

    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(feature = "kill-hook")]
#[test]
fn it08_kill_mid_copy_resumes_on_rerun() {
    let base = unique_base("it08");
    let src = base.join("src");
    write_file(&src.join("a_first.wav"), b"first-bytes");
    write_file(&src.join("b_second.wav"), b"second-bytes");
    let vols = base.join("vols");
    std::fs::create_dir_all(vols.join("KINGSTON")).unwrap();
    let cfg = base.join("cfg.toml");
    write_move_config(&cfg, "handy", "KINGSTON", &src, "audio");
    let log = base.join("exsync.log");
    let dest = vols.join("KINGSTON/audio");

    // Crash in S2: committed destination, source still present.
    let killed = run_exsync(
        &cfg,
        &vols,
        &log,
        &[],
        &[("EXSYNC_KILL_AFTER", "a_first.wav")],
    );
    assert_eq!(
        status(&killed),
        99,
        "stdout={} stderr={}",
        String::from_utf8_lossy(&killed.stdout),
        String::from_utf8_lossy(&killed.stderr)
    );
    assert!(src.join("a_first.wav").is_file(), "source survives the kill");
    assert_eq!(
        std::fs::read(dest.join("a_first.wav")).unwrap(),
        b"first-bytes",
        "destination committed before the kill"
    );
    assert!(src.join("b_second.wav").is_file(), "later file untouched");

    // Re-run without the hook finishes both files with one line each.
    let done = run_plain(&cfg, &vols, &log);
    assert_eq!(
        status(&done),
        0,
        "stdout={} stderr={} log={}",
        String::from_utf8_lossy(&done.stdout),
        String::from_utf8_lossy(&done.stderr),
        std::fs::read_to_string(&log).unwrap_or_default()
    );
    assert!(!src.join("a_first.wav").exists());
    assert!(!src.join("b_second.wav").exists());
    assert_eq!(
        std::fs::read(dest.join("a_first.wav")).unwrap(),
        b"first-bytes"
    );
    assert_eq!(
        std::fs::read(dest.join("b_second.wav")).unwrap(),
        b"second-bytes"
    );
    let lines = manifest_lines(&dest);
    assert_eq!(lines.len(), 2, "exactly one line per file: {lines:?}");

    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(not(feature = "kill-hook"))]
#[test]
#[ignore]
fn it08_kill_mid_copy_resumes_on_rerun() {}

#[test]
fn it09_stale_temp_deleted_then_move_completes() {
    let base = unique_base("it09");
    let src = base.join("src");
    write_file(&src.join("real.wav"), b"real-bytes");
    let vols = base.join("vols");
    let dest = vols.join("KINGSTON/audio");
    std::fs::create_dir_all(&dest).unwrap();
    // Leftover S1 temp from a crashed run; must die on startup.
    write_file(&dest.join(".exsync-tmp.real.wav.99999"), b"partial-junk");
    let cfg = base.join("cfg.toml");
    write_move_config(&cfg, "handy", "KINGSTON", &src, "audio");
    let log = base.join("exsync.log");

    let out = run_plain(&cfg, &vols, &log);
    assert_eq!(
        status(&out),
        0,
        "stdout={} stderr={} log={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
        std::fs::read_to_string(&log).unwrap_or_default()
    );
    assert!(
        !dest.join(".exsync-tmp.real.wav.99999").exists(),
        "stale temp deleted"
    );
    assert_eq!(
        std::fs::read(dest.join("real.wav")).unwrap(),
        b"real-bytes",
        "real file moved from the intact source"
    );
    assert!(!src.join("real.wav").exists());
    assert_eq!(manifest_lines(&dest).len(), 1);

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn it10_dup_move_deletes_source_without_second_copy() {
    use std::os::unix::fs::MetadataExt;

    let base = unique_base("it10");
    let src = base.join("src");
    write_file(&src.join("same.wav"), b"identical-bytes");
    let vols = base.join("vols");
    let dest = vols.join("KINGSTON/audio");
    std::fs::create_dir_all(&dest).unwrap();
    write_file(&dest.join(".exsync-managed"), b"exsync-managed\n");
    write_file(&dest.join("same.wav"), b"identical-bytes");
    // Identical size and mtime: the committed copy must be reused as-is.
    let t = std::fs::metadata(src.join("same.wav"))
        .unwrap()
        .modified()
        .unwrap();
    std::fs::File::options()
        .read(true)
        .open(dest.join("same.wav"))
        .unwrap()
        .set_modified(t)
        .unwrap();
    let before_meta = std::fs::metadata(dest.join("same.wav")).unwrap();
    let (before_mtime, before_ino) = (
        before_meta.modified().unwrap(),
        before_meta.ino(),
    );

    let cfg = base.join("cfg.toml");
    write_move_config(&cfg, "handy", "KINGSTON", &src, "audio");
    let log = base.join("exsync.log");

    let out = run_plain(&cfg, &vols, &log);
    assert_eq!(
        status(&out),
        0,
        "stdout={} stderr={} log={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
        std::fs::read_to_string(&log).unwrap_or_default()
    );
    assert!(!src.join("same.wav").exists(), "source deleted");
    let after_meta = std::fs::metadata(dest.join("same.wav")).unwrap();
    assert_eq!(after_meta.modified().unwrap(), before_mtime, "mtime kept");
    assert_eq!(after_meta.ino(), before_ino, "no second copy written");
    assert_eq!(
        std::fs::read(dest.join("same.wav")).unwrap(),
        b"identical-bytes"
    );
    assert_eq!(manifest_lines(&dest).len(), 1, "one manifest line");
    let content = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(content.contains("DUP"), "log shows DUP: {content}");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn collision_keeps_source_and_fails() {
    let base = unique_base("collision");
    let src = base.join("src");
    write_file(&src.join("clash.wav"), b"new-bytes-here");
    let vols = base.join("vols");
    let dest = vols.join("KINGSTON/audio");
    std::fs::create_dir_all(&dest).unwrap();
    write_file(&dest.join(".exsync-managed"), b"exsync-managed\n");
    write_file(&dest.join("clash.wav"), b"wholly-different");
    let cfg = base.join("cfg.toml");
    write_move_config(&cfg, "handy", "KINGSTON", &src, "audio");
    let log = base.join("exsync.log");

    let out = run_plain(&cfg, &vols, &log);
    assert_eq!(status(&out), 1, "collision exits nonzero");
    assert_eq!(
        std::fs::read(src.join("clash.wav")).unwrap(),
        b"new-bytes-here",
        "source intact"
    );
    assert_eq!(
        std::fs::read(dest.join("clash.wav")).unwrap(),
        b"wholly-different",
        "destination untouched"
    );
    let content = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(content.contains("collision"), "log names collision: {content}");
    assert!(!has_manifest_file(&dest), "no manifest on pure collision");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn unmanaged_destination_refused_source_untouched() {
    let base = unique_base("guard");
    let src = base.join("src");
    write_file(&src.join("f.wav"), b"bytes");
    let vols = base.join("vols");
    let dest = vols.join("KINGSTON/audio");
    std::fs::create_dir_all(&dest).unwrap();
    write_file(&dest.join("sentinel.txt"), b"keep");
    let cfg = base.join("cfg.toml");
    write_move_config(&cfg, "handy", "KINGSTON", &src, "audio");
    let log = base.join("exsync.log");

    let out = run_plain(&cfg, &vols, &log);
    assert_eq!(status(&out), 1);
    assert_eq!(
        std::fs::read(dest.join("sentinel.txt")).unwrap(),
        b"keep",
        "sentinel unchanged"
    );
    assert!(!dest.join("f.wav").exists(), "nothing moved");
    assert!(src.join("f.wav").is_file(), "source untouched");
    let content = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(content.contains("REFUSED"), "log contains REFUSED: {content}");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn empty_source_moves_nothing_and_writes_no_manifest() {
    let base = unique_base("empty");
    let src = base.join("src");
    std::fs::create_dir_all(&src).unwrap();
    let vols = base.join("vols");
    std::fs::create_dir_all(vols.join("KINGSTON")).unwrap();
    let cfg = base.join("cfg.toml");
    write_move_config(&cfg, "handy", "KINGSTON", &src, "audio");
    let log = base.join("exsync.log");

    let out = run_plain(&cfg, &vols, &log);
    assert_eq!(
        status(&out),
        0,
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let dest = vols.join("KINGSTON/audio");
    assert!(!has_manifest_file(&dest), "no manifest file created");

    let _ = std::fs::remove_dir_all(&base);
}
