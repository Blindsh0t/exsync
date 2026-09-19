//! Mirror integration tests: IT-02/03/04/05/12/16 plus legacy marker.
//!
//! Each test spawns the real binary via `env!("CARGO_BIN_EXE_exsync")`
//! with its own temp dirs and `EXSYNC_*` env overrides, so the machine's
//! real `/Volumes` and `~/Library/Logs/exsync.log` are never used.

use std::collections::BTreeMap;
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
        "exsync-mirror-{tag}-{}-{nanos}-{n}",
        std::process::id()
    ))
}

fn write_file(path: &Path, contents: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn write_config(path: &Path, name: &str, drive: &str, source: &Path, dest: &str) {
    let text = format!(
        "version = 1\n\n[[entry]]\nname = \"{name}\"\nmode = \"mirror\"\ndrive = \"{drive}\"\nsource = \"{}\"\ndest = \"{dest}\"\n",
        source.display()
    );
    std::fs::write(path, text).unwrap();
}

fn run_exsync(cfg: &Path, vols: &Path, log: &Path, extra: &[&str]) -> Output {
    let bin = env!("CARGO_BIN_EXE_exsync");
    let mut cmd = Command::new(bin);
    cmd.env("EXSYNC_CONFIG", cfg);
    cmd.env("EXSYNC_VOLUMES_ROOT", vols);
    cmd.env("EXSYNC_LOG", log);
    for a in extra {
        cmd.arg(a);
    }
    cmd.output().expect("spawn exsync")
}

/// Recursive file listing: rel -> contents. Markers included; used to prove
/// dry-run writes nothing.
fn snapshot_files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut map = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = match std::fs::read_dir(&d) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for item in entries.flatten() {
            let p = item.path();
            let meta = match std::fs::symlink_metadata(&p) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(p);
            } else if meta.is_file() {
                let rel = p
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                let bytes = std::fs::read(&p).unwrap_or_default();
                map.insert(rel, bytes);
            }
        }
    }
    map
}

/// Recursive listing plus mtimes and sizes, for idempotence comparison.
fn snapshot_mtimes(dir: &Path) -> BTreeMap<String, (u64, SystemTime)> {
    let mut map = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = match std::fs::read_dir(&d) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for item in entries.flatten() {
            let p = item.path();
            let meta = match std::fs::symlink_metadata(&p) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(p);
            } else if meta.is_file() {
                let rel = p
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                map.insert(rel, (meta.len(), mtime));
            }
        }
    }
    map
}

fn setup_source(base: &Path) -> PathBuf {
    let src = base.join("src");
    write_file(&src.join("hello.txt"), b"hello");
    write_file(&src.join("a/data.txt"), b"data");
    write_file(&src.join("a/.DS_Store"), b"ds");
    write_file(&src.join(".git/HEAD"), b"ref: refs/heads/main\n");
    src
}

#[test]
fn it03_first_mirror_copies_git_skips_ds_writes_marker() {
    let base = unique_base("it03");
    let src = setup_source(&base);
    let vols = base.join("vols");
    std::fs::create_dir_all(vols.join("KINGSTON")).unwrap();
    let cfg = base.join("cfg.toml");
    write_config(&cfg, "dotfiles", "KINGSTON", &src, "data");
    let log = base.join("exsync.log");

    let out = run_exsync(&cfg, &vols, &log, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let dest = vols.join("KINGSTON/data");
    assert_eq!(
        std::fs::read(dest.join("hello.txt")).unwrap(),
        b"hello"
    );
    assert_eq!(
        std::fs::read(dest.join("a/data.txt")).unwrap(),
        b"data"
    );
    assert!(
        std::fs::read(dest.join(".git/HEAD")).unwrap() == b"ref: refs/heads/main\n",
        ".git/HEAD present"
    );
    assert!(
        !dest.join("a/.DS_Store").exists(),
        "a/.DS_Store absent"
    );
    assert!(dest.join(".exsync-managed").is_file(), "marker present");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn it04_second_run_changes_nothing() {
    let base = unique_base("it04");
    let src = setup_source(&base);
    let vols = base.join("vols");
    std::fs::create_dir_all(vols.join("KINGSTON")).unwrap();
    let cfg = base.join("cfg.toml");
    write_config(&cfg, "dotfiles", "KINGSTON", &src, "data");
    let log = base.join("exsync.log");

    let first = run_exsync(&cfg, &vols, &log, &[]);
    assert_eq!(first.status.code(), Some(0));
    let dest = vols.join("KINGSTON/data");
    let before = snapshot_mtimes(&dest);
    assert!(dest.join(".exsync-managed").is_file());
    std::thread::sleep(std::time::Duration::from_millis(50));

    let second = run_exsync(&cfg, &vols, &log, &[]);
    assert_eq!(second.status.code(), Some(0));
    assert!(dest.join(".exsync-managed").is_file(), "marker survives");
    let after = snapshot_mtimes(&dest);
    assert_eq!(before, after, "no file modified on second run");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn it05_unmanaged_destination_refused_untouched() {
    let base = unique_base("it05");
    let src = setup_source(&base);
    let vols = base.join("vols");
    let dest = vols.join("KINGSTON/data");
    std::fs::create_dir_all(&dest).unwrap();
    write_file(&dest.join("sentinel.txt"), b"keep");
    let cfg = base.join("cfg.toml");
    write_config(&cfg, "dotfiles", "KINGSTON", &src, "data");
    let log = base.join("exsync.log");

    let out = run_exsync(&cfg, &vols, &log, &[]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        std::fs::read(dest.join("sentinel.txt")).unwrap(),
        b"keep",
        "sentinel unchanged"
    );
    assert!(
        !dest.join("hello.txt").exists(),
        "nothing copied into refused dest"
    );
    let content = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(content.contains("REFUSED"), "log contains REFUSED: {content}");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn it02_configured_drive_runs_and_logs_outcome() {
    let base = unique_base("it02");
    let src = setup_source(&base);
    let vols = base.join("vols");
    std::fs::create_dir_all(vols.join("KINGSTON")).unwrap();
    let cfg = base.join("cfg.toml");
    write_config(&cfg, "dotfiles", "KINGSTON", &src, "data");
    let log = base.join("exsync.log");

    let out = run_exsync(&cfg, &vols, &log, &[]);
    assert_eq!(out.status.code(), Some(0));
    assert!(vols.join("KINGSTON/data/hello.txt").is_file());
    let content = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        content.contains("run entries="),
        "log shows run outcome: {content}"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn it12_success_is_silent() {
    let base = unique_base("it12");
    let src = setup_source(&base);
    let vols = base.join("vols");
    std::fs::create_dir_all(vols.join("KINGSTON")).unwrap();
    let cfg = base.join("cfg.toml");
    write_config(&cfg, "dotfiles", "KINGSTON", &src, "data");
    let log = base.join("exsync.log");

    let out = run_exsync(&cfg, &vols, &log, &[]);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        out.stdout.is_empty(),
        "stdout empty: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        out.stderr.is_empty(),
        "stderr empty: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn it16_dry_run_prints_plan_and_writes_nothing() {
    let base = unique_base("it16");
    let src = setup_source(&base);
    // Managed dest with an extra file (delete candidate) and without the
    // new source file... first seed a real run, then add changes.
    let vols = base.join("vols");
    std::fs::create_dir_all(vols.join("KINGSTON")).unwrap();
    let cfg = base.join("cfg.toml");
    write_config(&cfg, "dotfiles", "KINGSTON", &src, "data");
    let log = base.join("exsync.log");

    let seed = run_exsync(&cfg, &vols, &log, &[]);
    assert_eq!(seed.status.code(), Some(0));
    let dest = vols.join("KINGSTON/data");
    // New source file (copy candidate) and extra dest file (delete candidate).
    write_file(&src.join("new.txt"), b"new");
    write_file(&dest.join("extra.txt"), b"extra");
    let before = snapshot_files(&dest);
    assert!(before.contains_key(".exsync-managed"));

    let out = run_exsync(&cfg, &vols, &log, &["--dry-run"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        stdout.contains("PLAN MIRROR"),
        "dry-run prints PLAN MIRROR: {stdout}"
    );
    let after = snapshot_files(&dest);
    assert_eq!(before, after, "destination identical before and after");
    assert!(
        !dest.join("new.txt").exists(),
        "dry-run copies nothing"
    );
    assert!(dest.join("extra.txt").is_file(), "dry-run deletes nothing");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn legacy_marker_counts_as_managed() {
    let base = unique_base("legacy");
    let src = setup_source(&base);
    let vols = base.join("vols");
    let dest = vols.join("KINGSTON/data");
    std::fs::create_dir_all(&dest).unwrap();
    write_file(&dest.join(".external-sync-managed"), b"legacy");
    let cfg = base.join("cfg.toml");
    write_config(&cfg, "dotfiles", "KINGSTON", &src, "data");
    let log = base.join("exsync.log");

    let out = run_exsync(&cfg, &vols, &log, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout={} stderr={} log={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
        std::fs::read_to_string(&log).unwrap_or_default()
    );
    assert_eq!(
        std::fs::read(dest.join("hello.txt")).unwrap(),
        b"hello"
    );

    let _ = std::fs::remove_dir_all(&base);
}
