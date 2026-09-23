//! Mirror module: exact copy of a source folder to a drive destination.
//!
//! Mirror never writes to the source and never preserves unix metadata:
//! content only. A source file is never deleted (net-zero loss holds
//! trivially); destination files absent from the source are deleted only
//! after the copy pass completes.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use crate::{config, copy, log, matcher, notify};

const NEW_MARKER: &str = ".exsync-managed";
const LEGACY_MARKER: &str = ".external-sync-managed";
const DS_STORE: &str = ".DS_Store";

/// Lexically normalize a path, resolving `.` and `..` without touching the
/// filesystem (so it works for not-yet-existing destinations).
fn clean_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    // Leading `..` above root: keep it so the descendant
                    // check fails closed.
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

fn reason_token(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    for ch in msg.chars() {
        if ch.is_whitespace() {
            out.push('-');
        } else {
            out.push(ch);
        }
    }
    if out.is_empty() {
        "error".to_string()
    } else {
        out
    }
}

fn is_marker_name(name: &str) -> bool {
    name == NEW_MARKER || name == LEGACY_MARKER
}

fn is_skipped_copy_name(name: &str) -> bool {
    // Both marker names plus .DS_Store are excluded from copy and delete.
    // `.git` itself is copied; only the delete pass protects it.
    is_marker_name(name) || name == DS_STORE
}

fn file_name_of(p: &Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string()
}

/// True when `rel` is `.git` or lives inside it.
fn is_git_rel(rel: &Path) -> bool {
    rel.components().next()
        == Some(Component::Normal(std::ffi::OsStr::new(".git")))
}

/// Collect relative paths of all files under `root` (no filtering, no
/// symlink following). Symlinked directories are not descended into;
/// symlinks themselves are recorded as files.
fn all_source_files(root: &Path) -> HashSet<PathBuf> {
    let mut set = HashSet::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for item in entries.flatten() {
            let path = item.path();
            let meta = match std::fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let ft = meta.file_type();
            if ft.is_symlink() {
                if let Ok(rel) = path.strip_prefix(root).map(|r| r.to_path_buf()) {
                    set.insert(rel);
                }
                continue;
            }
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                if let Ok(rel) = path.strip_prefix(root).map(|r| r.to_path_buf()) {
                    set.insert(rel);
                }
            }
        }
    }
    set
}

struct DestEntry {
    path: PathBuf,
    rel: PathBuf,
    is_dir: bool,
    is_symlink: bool,
}

/// Walk the destination without following symlinks. `.git` subtrees are
/// pruned here so callers never see inside them.
fn walk_dest(dest: &Path) -> (Vec<DestEntry>, Vec<PathBuf>) {
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut stack = vec![dest.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for item in entries.flatten() {
            let path = item.path();
            let rel = match path.strip_prefix(dest).map(|r| r.to_path_buf()) {
                Ok(r) => r,
                Err(_) => continue,
            };
            if is_git_rel(&rel) {
                continue;
            }
            let meta = match std::fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let ft = meta.file_type();
            if ft.is_symlink() {
                files.push(DestEntry {
                    path,
                    rel,
                    is_dir: false,
                    is_symlink: true,
                });
            } else if meta.is_dir() {
                dirs.push(path.clone());
                stack.push(path);
            } else if meta.is_file() {
                files.push(DestEntry {
                    path,
                    rel,
                    is_dir: false,
                    is_symlink: false,
                });
            }
        }
    }
    // Deepest directories first so empty parents can be removed bottom-up.
    dirs.sort_by(|a, b| b.components().count().cmp(&a.components().count()));
    (files, dirs)
}

/// Entry-point for mirror mode.
pub fn run(entry: &config::Entry, vol_root: &Path, dry_run: bool) -> Result<(), String> {
    let dest = vol_root.join(&entry.drive).join(&entry.dest);

    // Never write above the volumes root: the destination must be a strict
    // descendant of the volumes root (lexical check, no IO).
    let clean_root = clean_path(vol_root);
    let clean_dest = clean_path(&dest);
    if clean_dest == clean_root || !clean_dest.starts_with(&clean_root) {
        notify::notify(
            "Exsync failed",
            &format!("{}: destination escapes volumes root", entry.name),
            dry_run,
        );
        return Err(format!("destination escapes volumes root: {}", dest.display()));
    }

    // Source must exist and be a directory. Mirror never writes to it.
    match std::fs::symlink_metadata(&entry.source) {
        Ok(m) if m.file_type().is_symlink() || m.is_file() => {
            notify::notify(
                "Exsync failed",
                &format!("{}: source missing", entry.name),
                dry_run,
            );
            return Err(format!("source missing: {}", entry.source.display()));
        }
        Ok(m) if m.is_dir() => {}
        _ => {
            notify::notify(
                "Exsync failed",
                &format!("{}: source missing", entry.name),
                dry_run,
            );
            return Err(format!("source missing: {}", entry.source.display()));
        }
    }

    // Guard (FR-10): a non-empty destination without either marker is
    // refused without touching a single file. Emptiness ignores both marker
    // names and `.DS_Store`, and symlinks are not followed.
    if std::fs::symlink_metadata(&dest).is_ok() {
        let meta = std::fs::symlink_metadata(&dest)
            .map_err(|e| format!("cannot stat destination: {e}"))?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            let line = log::action_line(
                "REFUSED",
                &entry.name,
                "-",
                0,
                "unmanaged-destination",
            );
            let _ = log::log_line(&line);
            notify::notify(
                "Exsync refused",
                &format!("{}: unmanaged-destination", entry.name),
                dry_run,
            );
            return Err("unmanaged-destination".to_string());
        }
        let has_marker = dest.join(NEW_MARKER).exists() || dest.join(LEGACY_MARKER).exists();
        let entries = std::fs::read_dir(&dest)
            .map_err(|e| format!("cannot read destination: {e}"))?;
        let mut non_empty = false;
        for item in entries.flatten() {
            let name = item.file_name();
            let name = name.to_string_lossy();
            if is_marker_name(&name) || name == DS_STORE {
                continue;
            }
            non_empty = true;
            break;
        }
        if non_empty && !has_marker {
            let line = log::action_line(
                "REFUSED",
                &entry.name,
                "-",
                0,
                "unmanaged-destination",
            );
            let _ = log::log_line(&line);
            notify::notify(
                "Exsync refused",
                &format!("{}: unmanaged-destination", entry.name),
                dry_run,
            );
            return Err("unmanaged-destination".to_string());
        }
    }

    let now = SystemTime::now();
    let candidates =
        matcher::collect(entry, &entry.source, now).map_err(|e| {
            let reason = reason_token(&e.to_string());
            notify::notify(
                "Exsync failed",
                &format!("{}: {reason}", entry.name),
                dry_run,
            );
            format!("cannot list source: {reason}")
        })?;

    // Filter markers and .DS_Store out of the copy set (matcher already
    // drops .DS_Store by default; this keeps the mirror guarantee even if
    // a config explicitly includes it).
    let mut wanted: Vec<(PathBuf, PathBuf, u64)> = Vec::new();
    for c in &candidates {
        let rel = match c.path.strip_prefix(&entry.source) {
            Ok(r) => r.to_path_buf(),
            Err(_) => continue,
        };
        let name = file_name_of(&c.path);
        if is_skipped_copy_name(&name) {
            continue;
        }
        // Never copy the marker files themselves even if the source holds
        // files with those names at any depth.
        if rel
            .components()
            .any(|comp| matches!(comp, Component::Normal(n) if is_marker_name(&n.to_string_lossy())))
        {
            continue;
        }
        wanted.push((c.path.clone(), rel, c.size));
    }
    wanted.sort_by(|a, b| a.1.cmp(&b.1));

    let source_set = all_source_files(&entry.source);

    if dry_run {
        for (_, rel, _) in &wanted {
            println!("PLAN MIRROR {}", rel.display());
        }
        // Deletes that would happen.
        if std::fs::symlink_metadata(&dest).map(|m| m.is_dir()).unwrap_or(false) {
            let (files, _) = walk_dest(&dest);
            let mut doomed: Vec<&PathBuf> = Vec::new();
            for f in &files {
                let name = file_name_of(&f.path);
                if is_skipped_copy_name(&name) {
                    continue;
                }
                if !source_set.contains(&f.rel) {
                    doomed.push(&f.rel);
                }
            }
            doomed.sort();
            for rel in doomed {
                println!("PLAN MIRROR {}", rel.display());
            }
        }
        return Ok(());
    }

    if let Err(e) = std::fs::create_dir_all(&dest) {
        let reason = reason_token(&e.to_string());
        notify::notify(
            "Exsync failed",
            &format!("{}: {reason}", entry.name),
            dry_run,
        );
        return Err(format!("cannot create destination: {e}"));
    }

    let mut failed = false;
    let mut failed_rels: HashSet<PathBuf> = HashSet::new();

    // Copy pass.
    for (src_path, rel, size) in &wanted {
        let rel_str = rel.to_string_lossy().into_owned();
        let dst_file = dest.join(rel);
        if std::fs::symlink_metadata(&dst_file).is_ok() {
            match copy::same_size_mtime(src_path, &dst_file) {
                Ok(true) => {
                    let _ = log::log_line(&log::action_line(
                        "DUP",
                        &entry.name,
                        &rel_str,
                        *size,
                        "identical",
                    ));
                    continue;
                }
                Ok(false) => {}
                Err(_) => {}
            }
        }
        let parent = dst_file.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| dest.clone());
        if let Err(e) = std::fs::create_dir_all(&parent) {
            let reason = reason_token(&e.to_string());
            let _ = log::log_line(&log::action_line("FAIL", &entry.name, &rel_str, *size, &reason));
            failed = true;
            failed_rels.insert(rel.clone());
            continue;
        }
        match copy::copy_verified(src_path, &parent, entry.verify.clone()) {
            Ok(_) => {}
            Err(copy::CopyError::SourceMissing) => {
                // File disappeared between listing and copy: skip, no error.
            }
            Err(e) => {
                let reason = reason_token(&e.to_string());
                let _ = log::log_line(&log::action_line("FAIL", &entry.name, &rel_str, *size, &reason));
                failed = true;
                failed_rels.insert(rel.clone());
            }
        }
    }

    // Delete pass: only after the copy pass completes. Never delete `.git`
    // or anything inside it, either marker, or `.DS_Store`. A failed file
    // is never deleted from the destination.
    {
        let (files, dirs) = walk_dest(&dest);
        for f in &files {
            if f.is_dir {
                continue;
            }
            let name = file_name_of(&f.path);
            if is_skipped_copy_name(&name) {
                continue;
            }
            if failed_rels.contains(&f.rel) {
                continue;
            }
            if source_set.contains(&f.rel) {
                continue;
            }
            // No counterpart in the source: delete.
            let size = std::fs::symlink_metadata(&f.path).map(|m| m.len()).unwrap_or(0);
            let rel_str = f.rel.to_string_lossy().into_owned();
            if f.is_symlink {
                if let Err(e) = std::fs::remove_file(&f.path) {
                    let reason = reason_token(&e.to_string());
                    let _ = log::log_line(&log::action_line("FAIL", &entry.name, &rel_str, size, &reason));
                    failed = true;
                }
            } else if let Err(e) = std::fs::remove_file(&f.path) {
                let reason = reason_token(&e.to_string());
                let _ = log::log_line(&log::action_line("FAIL", &entry.name, &rel_str, size, &reason));
                failed = true;
            }
        }
        // Remove directories left empty, never the destination root itself.
        for d in &dirs {
            if d == &dest {
                continue;
            }
            let _ = std::fs::remove_dir(d);
        }
    }

    if failed {
        notify::notify(
            "Exsync failed",
            &format!("{}: file failures", entry.name),
            dry_run,
        );
        return Err("file failures".to_string());
    }

    // Marker after a successful pass. Leave an existing marker untouched so
    // a re-run over an unchanged tree changes nothing (NFR-06).
    let marker_path = dest.join(NEW_MARKER);
    let needs_marker = match std::fs::read(&marker_path) {
        Ok(bytes) => bytes != b"exsync-managed\n",
        Err(_) => true,
    };
    if needs_marker {
        if let Err(e) = std::fs::write(&marker_path, "exsync-managed\n") {
            let reason = reason_token(&e.to_string());
            notify::notify(
                "Exsync failed",
                &format!("{}: {reason}", entry.name),
                dry_run,
            );
            return Err(format!("cannot write marker: {e}"));
        }
    }

    Ok(())
}
