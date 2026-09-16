use super::{copy_verified, same_size_mtime, set_corrupt_temp, CopyError};
use crate::config::Verify;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_LOCK: Mutex<()> = Mutex::new(());
static CTR: AtomicU64 = AtomicU64::new(0);

fn lock_tests() -> std::sync::MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn fresh_dir(tag: &str) -> PathBuf {
    let n = CTR.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "exsync-cp-{}-{}-{}-{}",
        tag,
        std::process::id(),
        nanos,
        n
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn no_tmp_files(dir: &std::path::Path) -> bool {
    match std::fs::read_dir(dir) {
        Ok(it) => it.flatten().all(|e| {
            !e.file_name()
                .to_string_lossy()
                .starts_with(".exsync-tmp")
        }),
        Err(_) => true,
    }
}

#[test]
fn ut09_readback_mismatch_fails_closed() {
    let _g = lock_tests();
    let base = fresh_dir("ut09");
    let src = base.join("src.dat");
    let dst = base.join("dst");
    std::fs::create_dir_all(&dst).unwrap();
    let content = vec![0xABu8; 64 * 1024];
    std::fs::write(&src, &content).unwrap();

    set_corrupt_temp(true);
    let r = copy_verified(&src, &dst, Verify::Readback);
    set_corrupt_temp(false);

    match r {
        Err(CopyError::VerifyMismatch) => {}
        other => panic!("expected VerifyMismatch, got {other:?}"),
    }
    assert!(no_tmp_files(&dst), "temp file leaked");
    assert!(
        !dst.join("src.dat").exists(),
        "final file must not exist after mismatch"
    );
    assert_eq!(
        std::fs::read(&src).unwrap(),
        content,
        "source must be unchanged"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn it13_copy_commits_cleanly() {
    let _g = lock_tests();
    set_corrupt_temp(false);
    let base = fresh_dir("it13ok");
    let src = base.join("hello.bin");
    let dst = base.join("dst");
    std::fs::create_dir_all(&dst).unwrap();
    let content: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&src, &content).unwrap();

    let n = copy_verified(&src, &dst, Verify::Readback).unwrap();
    assert_eq!(n, content.len() as u64);
    assert_eq!(std::fs::read(dst.join("hello.bin")).unwrap(), content);
    assert!(same_size_mtime(&src, &dst.join("hello.bin")).unwrap());
    assert!(no_tmp_files(&dst), "temp file leaked");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn it13_unwritable_dest_fails_closed() {
    let _g = lock_tests();
    set_corrupt_temp(false);
    use std::os::unix::fs::PermissionsExt;
    let base = fresh_dir("it13neg");
    let src = base.join("data.bin");
    let dst = base.join("dst");
    std::fs::create_dir_all(&dst).unwrap();
    let content = b"do not lose me".to_vec();
    std::fs::write(&src, &content).unwrap();

    std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(0o500)).unwrap();
    let r = copy_verified(&src, &dst, Verify::Readback);
    // Restore before any assertion that could panic, so cleanup can proceed.
    std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(0o700)).unwrap();

    match r {
        Err(CopyError::DestUnwritable(_)) => {}
        other => panic!("expected DestUnwritable, got {other:?}"),
    }
    assert!(
        !dst.join("data.bin").exists(),
        "no final file on unwritable dest"
    );
    assert!(no_tmp_files(&dst));
    assert_eq!(std::fs::read(&src).unwrap(), content);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ut06_same_size_mtime() {
    let _g = lock_tests();
    let base = fresh_dir("ut06");
    let a = base.join("a.dat");
    let b = base.join("b.dat");
    std::fs::write(&a, b"same content").unwrap();
    std::fs::write(&b, b"same content").unwrap();
    // Pin both mtimes equal with touch, then verify true.
    let st = std::process::Command::new("/usr/bin/touch")
        .args(["-t", "202001010000.00"])
        .arg(&a)
        .arg(&b)
        .status()
        .unwrap();
    assert!(st.success());
    assert!(
        same_size_mtime(&a, &b).unwrap(),
        "equal size+mtime should compare true"
    );
    // Bump one mtime 10 seconds later.
    let st = std::process::Command::new("/usr/bin/touch")
        .args(["-t", "202001010000.10"])
        .arg(&b)
        .status()
        .unwrap();
    assert!(st.success());
    assert!(
        !same_size_mtime(&a, &b).unwrap(),
        "differing mtime should compare false"
    );
    let _ = std::fs::remove_dir_all(&base);
}
