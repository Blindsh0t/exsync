use crate::matcher::*;
use crate::config::{Entry, Mode, Verify};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, Duration};
use std::process::Command;

fn create_temp_dir() -> std::path::PathBuf {
    let id = std::process::id();
    let thread_id = std::thread::current().id();
    let tmp = std::env::temp_dir().join(format!("exsync-test-{:?}-{:?}", id, thread_id));
    fs::create_dir_all(&tmp).expect("failed to create temp dir");
    tmp
}

fn cleanup_temp_dir(path: PathBuf) {
    let _ = fs::remove_dir_all(path);
}

fn create_file(dir: &Path, rel_path: &str, content: &str) {
    let path = dir.join(rel_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("failed to create parent dir");
    }
    fs::write(path, content).expect("failed to write file");
}

#[test]
fn test_glob_patterns() {
    // basic *
    assert!(glob_match("*.wav", "a.wav"));
    assert!(!glob_match("*.wav", "a.txt"));
    assert!(!glob_match("*.wav", "a.txt.wav.bak"));
    
    // basic ?
    assert!(glob_match("?.wav", "a.wav"));
    assert!(!glob_match("?.wav", "aa.wav"));
    
    // ** crosses /
    assert!(glob_match("**/*.wav", "dir/a.wav"));
    assert!(glob_match("**/*.wav", "dir/subdir/a.wav"));
    assert!(glob_match("*.wav", "dir/a.wav")); // should match filename only
    
    // * does not cross /
    assert!(glob_match("dir/*.wav", "dir/a.wav"));
    assert!(!glob_match("dir/*.wav", "dir/subdir/a.wav"));
    
    // / free pattern matches filename only
    assert!(glob_match("*.wav", "dir/a.wav"));
    assert!(glob_match("*.wav", "a.wav"));
    assert!(!glob_match("*.wav", "a.txt"));
}

#[test]
fn test_ut04_include() {
    let root = create_temp_dir();
    create_file(&root, "a.wav", "1");
    create_file(&root, "a.txt", "1");
    create_file(&root, "a.txt.wav.bak", "1");

    let entry = Entry {
        name: "test".into(),
        mode: Mode::Mirror,
        drive: "disk".into(),
        source: root.clone(),
        dest: "dest".into(),
        include: vec!["*.wav".into()],
        exclude: vec![],
        name_re: None,
        min_age_minutes: None,
        skip_open: false,
        verify: Verify::Readback,
    };

    let now = SystemTime::now();
    let candidates = collect(&entry, &root, now).expect("collect failed");
    
    let paths: Vec<_> = candidates.iter().map(|c| c.path.strip_prefix(&root).unwrap().to_str().unwrap()).collect();
    assert_eq!(paths, vec!["a.wav"]);
    cleanup_temp_dir(root);
}

#[test]
fn test_ut05_exclude_wins() {
    let root = create_temp_dir();
    create_file(&root, "a.wav", "1");
    create_file(&root, "skip/x.wav", "1");
    create_file(&root, "keep/x.wav", "1");
    create_file(&root, ".DS_Store", "1");

    let entry = Entry {
        name: "test".into(),
        mode: Mode::Mirror,
        drive: "disk".into(),
        source: root.clone(),
        dest: "dest".into(),
        include: vec!["*.wav".into()],
        exclude: vec!["*.wav".into()], // exclude wins over include
        name_re: None,
        min_age_minutes: None,
        skip_open: false,
        verify: Verify::Readback,
    };

    let now = SystemTime::now();
    let candidates = collect(&entry, &root, now).expect("collect failed");
    assert!(candidates.is_empty());

    // Test .DS_Store is excluded even with include *
    let entry_all = Entry {
        name: "test".into(),
        mode: Mode::Mirror,
        drive: "disk".into(),
        source: root.clone(),
        dest: "dest".into(),
        include: vec!["*".into()],
        exclude: vec![],
        name_re: None,
        min_age_minutes: None,
        skip_open: false,
        verify: Verify::Readback,
    };
    let candidates_all = collect(&entry_all, &root, now).expect("collect failed");
    let paths: Vec<_> = candidates_all.iter().map(|c| c.path.strip_prefix(&root).unwrap().to_str().unwrap()).collect();
    
    // .DS_Store should be filtered out
    assert!(!paths.contains(&".DS_Store"));
    cleanup_temp_dir(root);
}

#[test]
fn test_ut05_exclude_paths() {
    let root = create_temp_dir();
    create_file(&root, "keep/x.wav", "1");
    create_file(&root, "skip/x.wav", "1");

    let entry = Entry {
        name: "test".into(),
        mode: Mode::Mirror,
        drive: "disk".into(),
        source: root.clone(),
        dest: "dest".into(),
        include: vec!["*.wav".into()],
        exclude: vec!["skip/*".into()],
        name_re: None,
        min_age_minutes: None,
        skip_open: false,
        verify: Verify::Readback,
    };

    let now = SystemTime::now();
    let candidates = collect(&entry, &root, now).expect("collect failed");
    let paths: Vec<_> = candidates.iter().map(|c| c.path.strip_prefix(&root).unwrap().to_str().unwrap()).collect();
    assert_eq!(paths, vec!["keep/x.wav"]);
    cleanup_temp_dir(root);
}

#[test]
fn test_it06_age_guard() {
    let root = create_temp_dir();
    let path = root.join("old.txt");
    fs::write(&path, "1").expect("failed to write");

    let entry = Entry {
        name: "test".into(),
        mode: Mode::Mirror,
        drive: "disk".into(),
        source: root.clone(),
        dest: "dest".into(),
        include: vec![],
        exclude: vec![],
        name_re: None,
        min_age_minutes: Some(5),
        skip_open: false,
        verify: Verify::Readback,
    };

    let now = SystemTime::now();
    
    // fresh file should be skipped
    let candidates = collect(&entry, &root, now).expect("collect failed");
    assert!(candidates.is_empty());

    // set mtime to 2020
    Command::new("touch").arg("-t").arg("202001010000").arg(&path).output().expect("touch failed");
    
    let candidates_old = collect(&entry, &root, now).expect("collect failed");
    assert_eq!(candidates_old.len(), 1);
    cleanup_temp_dir(root);
}

#[test]
fn test_it06_open_guard() {
    let root = create_temp_dir();
    let path = root.join("open.txt");
    fs::write(&path, "1").expect("failed to write");

    let entry = Entry {
        name: "test".into(),
        mode: Mode::Mirror,
        drive: "disk".into(),
        source: root.clone(),
        dest: "dest".into(),
        include: vec![],
        exclude: vec![],
        name_re: None,
        min_age_minutes: None,
        skip_open: true,
        verify: Verify::Readback,
    };

    let now = SystemTime::now();
    
    // Not open yet
    let candidates = collect(&entry, &root, now).expect("collect failed");
    assert_eq!(candidates.len(), 1);

    // Hold file open
    let path_str = path.to_str().unwrap();
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(format!("exec 3<\"{}\"; sleep 3", path_str))
        .spawn()
        .expect("failed to spawn shell");

    let candidates_open = collect(&entry, &root, now).expect("collect failed");
    assert!(candidates_open.is_empty(), "File should be skipped because it is open");

    child.wait().expect("wait failed");
    cleanup_temp_dir(root);
}

#[test]
fn test_name_re() {
    let root = create_temp_dir();
    create_file(&root, "a.wav", "1");
    create_file(&root, "b.txt", "1");
    create_file(&root, "c.wav", "1");

    let entry = Entry {
        name: "test".into(),
        mode: Mode::Mirror,
        drive: "disk".into(),
        source: root.clone(),
        dest: "dest".into(),
        include: vec![],
        exclude: vec![],
        name_re: Some(r"^[a-c]\.wav$".into()),
        min_age_minutes: None,
        skip_open: false,
        verify: Verify::Readback,
    };

    let now = SystemTime::now();
    let candidates = collect(&entry, &root, now).expect("collect failed");
    let paths: Vec<_> = candidates.iter().map(|c| c.path.strip_prefix(&root).unwrap().to_str().unwrap()).collect();
    assert_eq!(paths, vec!["a.wav", "c.wav"]);
    cleanup_temp_dir(root);
}
