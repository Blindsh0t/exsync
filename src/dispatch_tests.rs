use std::sync::Mutex;

use crate::cli;

static TEST_MUTEX: Mutex<()> = Mutex::new(());

struct SavedEnv {
    config: Option<String>,
    volumes: Option<String>,
    log: Option<String>,
}

fn save_env() -> SavedEnv {
    SavedEnv {
        config: std::env::var("EXSYNC_CONFIG").ok(),
        volumes: std::env::var("EXSYNC_VOLUMES_ROOT").ok(),
        log: std::env::var("EXSYNC_LOG").ok(),
    }
}

fn restore_env(saved: SavedEnv) {
    match saved.config {
        Some(v) => std::env::set_var("EXSYNC_CONFIG", v),
        None => std::env::remove_var("EXSYNC_CONFIG"),
    }
    match saved.volumes {
        Some(v) => std::env::set_var("EXSYNC_VOLUMES_ROOT", v),
        None => std::env::remove_var("EXSYNC_VOLUMES_ROOT"),
    }
    match saved.log {
        Some(v) => std::env::set_var("EXSYNC_LOG", v),
        None => std::env::remove_var("EXSYNC_LOG"),
    }
}

fn unique_base(tag: &str) -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "exsync-dispatch-{tag}-{}-{nanos}-{n}",
        std::process::id()
    ))
}

fn write_cfg(dir: &std::path::Path, drive: &str) -> std::path::PathBuf {
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    let cfg = dir.join("cfg.toml");
    let text = format!(
        "version = 1\n\n[[entry]]\nname = \"t\"\nmode = \"mirror\"\ndrive = \"{drive}\"\nsource = \"{}\"\ndest = \"x\"\n",
        src.display()
    );
    std::fs::write(&cfg, text).unwrap();
    cfg
}

#[test]
fn no_drive_mounted_returns_zero_with_single_run_line() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let saved = save_env();
    let base = unique_base("unmounted");
    std::fs::create_dir_all(base.join("vols/OTHER")).unwrap();
    let cfg = write_cfg(&base, "KINGSTON");
    let log = base.join("exsync.log");
    std::env::set_var("EXSYNC_CONFIG", &cfg);
    std::env::set_var("EXSYNC_VOLUMES_ROOT", base.join("vols"));
    std::env::set_var("EXSYNC_LOG", &log);

    let code = super::run(&cli::Options { dry_run: false });

    let content = std::fs::read_to_string(&log).unwrap_or_default();
    restore_env(saved);
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(code, 0);
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("run entries="));
}

#[test]
fn mounted_mirror_entry_runs_and_returns_zero() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let saved = save_env();
    let base = unique_base("mounted");
    std::fs::create_dir_all(base.join("vols/KINGSTON")).unwrap();
    let cfg = write_cfg(&base, "KINGSTON");
    let log = base.join("exsync.log");
    std::env::set_var("EXSYNC_CONFIG", &cfg);
    std::env::set_var("EXSYNC_VOLUMES_ROOT", base.join("vols"));
    std::env::set_var("EXSYNC_LOG", &log);

    let code = super::run(&cli::Options { dry_run: false });

    let content = std::fs::read_to_string(&log).unwrap_or_default();
    restore_env(saved);
    let marker = base.join("vols/KINGSTON/x/.exsync-managed");
    let marker_exists = marker.is_file();
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(code, 0);
    assert!(content.contains("OK run entries=1 mounted=1"));
    assert!(marker_exists, "mirror must write the marker for an empty source");
}

#[test]
fn malformed_config_returns_two() {
    let _lock = TEST_MUTEX.lock().unwrap();
    let saved = save_env();
    let base = unique_base("badcfg");
    std::fs::create_dir_all(base.join("vols")).unwrap();
    let cfg = base.join("cfg.toml");
    std::fs::write(&cfg, "version = 2\n").unwrap();
    let log = base.join("exsync.log");
    std::env::set_var("EXSYNC_CONFIG", &cfg);
    std::env::set_var("EXSYNC_VOLUMES_ROOT", base.join("vols"));
    std::env::set_var("EXSYNC_LOG", &log);

    let code = super::run(&cli::Options { dry_run: false });

    restore_env(saved);
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(code, 2);
}
