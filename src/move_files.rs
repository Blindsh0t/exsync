//! MoveFiles module: verified move of candidate files to the drive.
//!
//! Each file travels S0 -> S1 -> S2 -> S3: copy to a temp name, verify,
//! rename into place (S2, source still present), then unlink the source
//! (S3). The source is never unlinked before its verified, committed
//! destination exists, so a crash at any point leaves at least one copy.

use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use crate::{config, copy, log, manifest, matcher, notify};

const NEW_MARKER: &str = ".exsync-managed";
const LEGACY_MARKER: &str = ".external-sync-managed";
const DS_STORE: &str = ".DS_Store";
const TMP_PREFIX: &str = ".exsync-tmp.";
const MANIFEST_PREFIX: &str = ".exsync-manifest.";

/// Lexically normalize a path, resolving `.` and `..` without touching the
/// filesystem (so it works for not-yet-existing destinations).
fn clean_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
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

/// Collapse an error message to a single log-safe token.
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

fn file_name_of(p: &Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string()
}

/// Runtime bookkeeping names that are never moved, completed, or counted
/// when judging whether a destination is empty.
fn is_special_name(name: &str) -> bool {
    name == NEW_MARKER
        || name == LEGACY_MARKER
        || name == DS_STORE
        || name.starts_with(MANIFEST_PREFIX)
        || name.starts_with(TMP_PREFIX)
}

fn has_manifest(dest: &Path) -> bool {
    match std::fs::read_dir(dest) {
        Ok(rd) => rd.flatten().any(|item| {
            item.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(MANIFEST_PREFIX))
        }),
        Err(_) => false,
    }
}

/// Chunked byte equality; both files are read fully, nothing is loaded
/// whole into memory.
fn files_equal(a: &Path, b: &Path) -> std::io::Result<bool> {
    use std::io::Read;
    let mut fa = std::fs::File::open(a)?;
    let mut fb = std::fs::File::open(b)?;
    let mut ba = [0u8; 65536];
    let mut bb = [0u8; 65536];
    loop {
        let na = {
            let mut got = 0;
            while got < ba.len() {
                match fa.read(&mut ba[got..])? {
                    0 => break,
                    n => got += n,
                }
            }
            got
        };
        let nb = {
            let mut got = 0;
            while got < bb.len() {
                match fb.read(&mut bb[got..])? {
                    0 => break,
                    n => got += n,
                }
            }
            got
        };
        if na != nb {
            return Ok(false);
        }
        if na == 0 {
            return Ok(true);
        }
        if ba[..na] != bb[..nb] {
            return Ok(false);
        }
    }
}

/// Startup recovery (S1 crash): delete every `.exsync-tmp.*` file in the
/// destination tree. Nothing else is ever deleted here.
fn remove_tmp_tree(dest: &Path) {
    let mut stack = vec![dest.to_path_buf()];
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
            if meta.file_type().is_symlink() {
                if file_name_of(&path).starts_with(TMP_PREFIX) {
                    let _ = std::fs::remove_file(&path);
                }
                continue;
            }
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() && file_name_of(&path).starts_with(TMP_PREFIX) {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

/// Regular files under `dest` (no symlink following) with their
/// destination-relative paths, excluding runtime bookkeeping names.
fn dest_regular_files(dest: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut stack = vec![dest.to_path_buf()];
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
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                if is_special_name(&file_name_of(&path)) {
                    continue;
                }
                if let Ok(rel) = path.strip_prefix(dest).map(|r| r.to_path_buf()) {
                    out.push((path, rel.to_string_lossy().into_owned()));
                }
            }
        }
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// Read a file fully to prove it is committed and readable. Returns its
/// metadata length.
fn verify_readable(path: &Path) -> std::io::Result<u64> {
    use std::io::Read;
    let len = std::fs::metadata(path)?.len();
    let mut f = std::fs::File::open(path)?;
    let mut buf = [0u8; 65536];
    loop {
        match f.read(&mut buf)? {
            0 => break,
            _ => {}
        }
    }
    Ok(len)
}

fn ensure_marker(dest: &Path) {
    let marker = dest.join(NEW_MARKER);
    let needs = match std::fs::read(&marker) {
        Ok(bytes) => bytes != b"exsync-managed\n",
        Err(_) => true,
    };
    if needs {
        let _ = std::fs::write(&marker, "exsync-managed\n");
    }
}

/// Unlink the source after its destination committed (S2 -> S3). A source
/// that already vanished counts as done: the committed destination holds
/// the only copy and the manifest still needs its line.
fn unlink_source(src: &Path) -> Result<bool, String> {
    match std::fs::remove_file(src) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(reason_token(&e.to_string())),
    }
}

/// Entry-point for move mode.
pub fn run(entry: &config::Entry, vol_root: &Path, dry_run: bool) -> Result<(), String> {
    let dest = vol_root.join(&entry.drive).join(&entry.dest);

    // Never write above the volumes root: strict lexical descendant.
    let clean_root = clean_path(vol_root);
    let clean_dest = clean_path(&dest);
    if clean_dest == clean_root || !clean_dest.starts_with(&clean_root) {
        notify::notify(
            "Exsync failed",
            &format!("{}: destination escapes volumes root", entry.name),
            dry_run,
        );
        return Err(format!(
            "destination escapes volumes root: {}",
            dest.display()
        ));
    }

    // Source must exist and be a directory; move never creates it.
    match std::fs::symlink_metadata(&entry.source) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
        _ => {
            notify::notify(
                "Exsync failed",
                &format!("{}: source missing", entry.name),
                dry_run,
            );
            return Err(format!("source missing: {}", entry.source.display()));
        }
    }

    // Unmanaged-destination guard: non-empty without a marker (or one of our
    // manifests) is refused without touching a file. Emptiness ignores
    // markers, .DS_Store, manifests, and leftover temp files.
    if std::fs::symlink_metadata(&dest).is_ok() {
        let meta = std::fs::symlink_metadata(&dest)
            .map_err(|e| format!("cannot stat destination: {e}"))?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            let _ = log::log_line(&log::action_line(
                "REFUSED",
                &entry.name,
                "-",
                0,
                "unmanaged-destination",
            ));
            notify::notify(
                "Exsync refused",
                &format!("{}: unmanaged-destination", entry.name),
                dry_run,
            );
            return Err("unmanaged-destination".to_string());
        }
        let managed = dest.join(NEW_MARKER).exists()
            || dest.join(LEGACY_MARKER).exists()
            || has_manifest(&dest);
        let non_empty = match std::fs::read_dir(&dest) {
            Ok(rd) => rd.flatten().any(|item| {
                !item
                    .file_name()
                    .to_str()
                    .is_some_and(is_special_name)
            }),
            Err(e) => return Err(format!("cannot read destination: {e}")),
        };
        if non_empty && !managed {
            let _ = log::log_line(&log::action_line(
                "REFUSED",
                &entry.name,
                "-",
                0,
                "unmanaged-destination",
            ));
            notify::notify(
                "Exsync refused",
                &format!("{}: unmanaged-destination", entry.name),
                dry_run,
            );
            return Err("unmanaged-destination".to_string());
        }
    }

    let now = SystemTime::now();
    let candidates = matcher::collect(entry, &entry.source, now).map_err(|e| {
        let reason = reason_token(&e.to_string());
        notify::notify(
            "Exsync failed",
            &format!("{}: {reason}", entry.name),
            dry_run,
        );
        format!("cannot list source: {reason}")
    })?;

    if dry_run {
        for c in &candidates {
            let rel = c
                .path
                .strip_prefix(&entry.source)
                .map(|r| r.to_string_lossy().into_owned())
                .unwrap_or_else(|_| file_name_of(&c.path));
            println!("PLAN MOVE {rel}");
        }
        return Ok(());
    }

    let dest_is_dir = std::fs::symlink_metadata(&dest)
        .map(|m| m.is_dir() && !m.file_type().is_symlink())
        .unwrap_or(false);
    if candidates.is_empty() && !dest_is_dir {
        // Empty source, no destination: genuinely nothing to do.
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

    // The guard above proved this destination is ours (managed) or empty,
    // so mark it before any copy: a kill later in the run must still find
    // a managed destination on re-run instead of a REFUSED one.
    ensure_marker(&dest);

    // Startup recovery first: drop S1 temp files, delete nothing else ever.
    remove_tmp_tree(&dest);

    let mut failed = false;
    let mut moved_any = false;

    // Idempotent completion: a committed destination file whose source is
    // gone (S3 reached but the manifest line lost) is verified and recorded.
    // Files with a live source are left for the candidate loop so each move
    // appends exactly one line.
    for (dst_path, rel_str) in dest_regular_files(&dest) {
        if manifest::last_entry(&dest, &rel_str).unwrap_or(true) {
            continue;
        }
        if std::fs::symlink_metadata(entry.source.join(&rel_str)).is_ok() {
            continue;
        }
        match verify_readable(&dst_path) {
            Ok(size) => {
                if let Err(e) = manifest::append_entry(
                    &dest,
                    &entry.name,
                    &rel_str,
                    size,
                    SystemTime::now(),
                ) {
                    let reason = reason_token(&e.to_string());
                    let _ = log::log_line(&log::action_line(
                        "FAIL",
                        &entry.name,
                        &rel_str,
                        size,
                        &reason,
                    ));
                    notify::notify(
                        "Exsync failed",
                        &format!("{}: {rel_str} {reason}", entry.name),
                        dry_run,
                    );
                    failed = true;
                } else {
                    moved_any = true;
                }
            }
            Err(e) => {
                let reason = reason_token(&e.to_string());
                let size = std::fs::metadata(&dst_path).map(|m| m.len()).unwrap_or(0);
                let _ = log::log_line(&log::action_line(
                    "FAIL",
                    &entry.name,
                    &rel_str,
                    size,
                    &reason,
                ));
                notify::notify(
                    "Exsync failed",
                    &format!("{}: {rel_str} {reason}", entry.name),
                    dry_run,
                );
                failed = true;
            }
        }
    }

    for c in &candidates {
        let rel = match c.path.strip_prefix(&entry.source) {
            Ok(r) => r.to_path_buf(),
            Err(_) => continue,
        };
        let rel_str = rel.to_string_lossy().into_owned();
        let dst_file = dest.join(&rel);

        // Destination already holds this name.
        if std::fs::symlink_metadata(&dst_file).is_ok() {
            let identical = match copy::same_size_mtime(&c.path, &dst_file) {
                Ok(true) => files_equal(&c.path, &dst_file).unwrap_or(false),
                _ => false,
            };
            if identical {
                // Verified identical: finish the move without a second copy.
                let _ = log::log_line(&log::action_line(
                    "DUP",
                    &entry.name,
                    &rel_str,
                    c.size,
                    "identical",
                ));
                match unlink_source(&c.path) {
                    Ok(_) => {
                        if let Err(e) = manifest::append_entry(
                            &dest,
                            &entry.name,
                            &rel_str,
                            c.size,
                            SystemTime::now(),
                        ) {
                            let reason = reason_token(&e.to_string());
                            let _ = log::log_line(&log::action_line(
                                "FAIL",
                                &entry.name,
                                &rel_str,
                                c.size,
                                &reason,
                            ));
                            notify::notify(
                                "Exsync failed",
                                &format!("{}: {rel_str} {reason}", entry.name),
                                dry_run,
                            );
                            failed = true;
                        } else {
                            moved_any = true;
                        }
                    }
                    Err(reason) => {
                        let _ = log::log_line(&log::action_line(
                            "FAIL",
                            &entry.name,
                            &rel_str,
                            c.size,
                            &reason,
                        ));
                        notify::notify(
                            "Exsync failed",
                            &format!("{}: {rel_str} {reason}", entry.name),
                            dry_run,
                        );
                        failed = true;
                    }
                }
                continue;
            }
            // Same name, different content: collision. Source stays intact.
            let _ = log::log_line(&log::action_line(
                "FAIL",
                &entry.name,
                &rel_str,
                c.size,
                "collision",
            ));
            notify::notify(
                "Exsync collision",
                &format!("{}: {rel_str} collision", entry.name),
                dry_run,
            );
            failed = true;
            continue;
        }

        let parent = dst_file
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| dest.clone());
        if let Err(e) = std::fs::create_dir_all(&parent) {
            let reason = reason_token(&e.to_string());
            let _ = log::log_line(&log::action_line(
                "FAIL",
                &entry.name,
                &rel_str,
                c.size,
                &reason,
            ));
            notify::notify(
                "Exsync failed",
                &format!("{}: {rel_str} {reason}", entry.name),
                dry_run,
            );
            failed = true;
            continue;
        }
        // S0 -> S2: the source is still present when the commit lands.
        match copy::copy_verified(&c.path, &parent, entry.verify.clone()) {
            Ok(_) => {}
            Err(copy::CopyError::SourceMissing) => continue,
            Err(e) => {
                let reason = reason_token(&e.to_string());
                let _ = log::log_line(&log::action_line(
                    "FAIL",
                    &entry.name,
                    &rel_str,
                    c.size,
                    &reason,
                ));
                notify::notify(
                    "Exsync failed",
                    &format!("{}: {rel_str} {reason}", entry.name),
                    dry_run,
                );
                failed = true;
                continue;
            }
        }

        // Crash hook (test builds only): die in S2 with both copies present.
        #[cfg(feature = "kill-hook")]
        {
            if let Ok(hook) = std::env::var("EXSYNC_KILL_AFTER") {
                if !hook.is_empty() && file_name_of(&dst_file) == hook {
                    std::process::exit(99);
                }
            }
        }

        // S2 -> S3: unlink only after the verified commit.
        match unlink_source(&c.path) {
            Ok(_) => {
                if let Err(e) =
                    manifest::append_entry(&dest, &entry.name, &rel_str, c.size, SystemTime::now())
                {
                    let reason = reason_token(&e.to_string());
                    let _ = log::log_line(&log::action_line(
                        "FAIL",
                        &entry.name,
                        &rel_str,
                        c.size,
                        &reason,
                    ));
                    notify::notify(
                        "Exsync failed",
                        &format!("{}: {rel_str} {reason}", entry.name),
                        dry_run,
                    );
                    failed = true;
                } else {
                    moved_any = true;
                }
            }
            Err(reason) => {
                let _ = log::log_line(&log::action_line(
                    "FAIL",
                    &entry.name,
                    &rel_str,
                    c.size,
                    &reason,
                ));
                notify::notify(
                    "Exsync failed",
                    &format!("{}: {rel_str} {reason}", entry.name),
                    dry_run,
                );
                failed = true;
            }
        }
    }

    if moved_any || has_manifest(&dest) {
        ensure_marker(&dest);
    }

    if failed {
        return Err("file failures".to_string());
    }
    Ok(())
}
