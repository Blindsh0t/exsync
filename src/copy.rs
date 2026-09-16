//! Copy module: Verified streaming copy core.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process;

use crate::config::Verify;
use crate::hash::Fnv1a;

#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};

const CHUNK: usize = 1 << 20;

#[derive(Debug)]
pub enum CopyError {
    SourceMissing,
    DestUnwritable(String),
    VerifyMismatch,
    Io(String),
}

impl std::fmt::Display for CopyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SourceMissing => write!(f, "source missing"),
            Self::DestUnwritable(e) => write!(f, "destination unwritable: {e}"),
            Self::VerifyMismatch => write!(f, "verify mismatch"),
            Self::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for CopyError {}

#[cfg(test)]
static CORRUPT_TEMP: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
pub fn set_corrupt_temp(v: bool) {
    CORRUPT_TEMP.store(v, Ordering::Relaxed);
}

fn remove_quiet(p: &Path) {
    let _ = std::fs::remove_file(p);
}

fn temp_path(dst_dir: &Path, file_name: &str) -> PathBuf {
    dst_dir.join(format!(".exsync-tmp.{}.{}", file_name, process::id()))
}

fn dir_writable_bits(dst_dir: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(dst_dir) {
        Ok(md) => md.permissions().mode() & 0o222 != 0,
        // Missing dir or stat failure: let the create step fail and map it.
        Err(_) => true,
    }
}

/// One streaming attempt: copy src to tmp, fsync both handles, readback-verify
/// the temp. Returns the (bytes, hash) on success. Mismatches are reported as
/// `CopyError::VerifyMismatch` with the temp already removed; the caller
/// decides whether to retry.
fn attempt_once(
    src: &Path,
    tmp: &Path,
    dst_dir: &Path,
    wbuf: &mut [u8],
    rbuf: &mut [u8],
) -> Result<(u64, u64), CopyError> {
    let mut reader = match File::open(src) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(CopyError::SourceMissing)
        }
        Err(e) => {
            remove_quiet(tmp);
            return Err(CopyError::Io(e.to_string()));
        }
    };

    if !dir_writable_bits(dst_dir) {
        return Err(CopyError::DestUnwritable(format!(
            "directory not writable: {}",
            dst_dir.display()
        )));
    }
    let mut out = match File::create(tmp) {
        Ok(f) => f,
        Err(e) => return Err(CopyError::DestUnwritable(e.to_string())),
    };

    let mut hasher = Fnv1a::new();
    let mut total: u64 = 0;
    loop {
        let n = match reader.read(&mut wbuf[..]) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                drop(out);
                remove_quiet(tmp);
                return Err(CopyError::Io(e.to_string()));
            }
        };
        hasher.write(&wbuf[..n]);
        if let Err(e) = out.write_all(&wbuf[..n]) {
            drop(out);
            remove_quiet(tmp);
            return Err(CopyError::Io(e.to_string()));
        }
        total += n as u64;
    }

    #[cfg(test)]
    {
        if CORRUPT_TEMP.load(Ordering::Relaxed) && total > 0 {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(tmp)
            {
                use std::io::Seek;
                if f.seek(std::io::SeekFrom::End(-1)).is_ok() {
                    let mut b = [0u8; 1];
                    if f.read_exact(&mut b).is_ok() {
                        let _ = f.seek(std::io::SeekFrom::End(-1));
                        b[0] ^= 0xff;
                        let _ = f.write_all(&b);
                    }
                }
            }
        }
    }

    if let Err(e) = out.sync_all() {
        drop(out);
        remove_quiet(tmp);
        return Err(CopyError::Io(e.to_string()));
    }
    drop(out);

    if let Err(e) = File::open(dst_dir).and_then(|d| d.sync_all()) {
        remove_quiet(tmp);
        return Err(CopyError::Io(e.to_string()));
    }

    let recorded = hasher.finish();
    let mut back = match File::open(tmp) {
        Ok(f) => f,
        Err(e) => {
            remove_quiet(tmp);
            return Err(CopyError::Io(e.to_string()));
        }
    };
    let mut check = Fnv1a::new();
    loop {
        match back.read(&mut rbuf[..]) {
            Ok(0) => break,
            Ok(n) => check.write(&rbuf[..n]),
            Err(e) => {
                drop(back);
                remove_quiet(tmp);
                return Err(CopyError::Io(e.to_string()));
            }
        }
    }
    drop(back);
    if check.finish() != recorded {
        remove_quiet(tmp);
        return Err(CopyError::VerifyMismatch);
    }
    Ok((total, recorded))
}

pub fn copy_verified(
    src: &Path,
    dst_dir: &Path,
    verify: Verify,
) -> Result<u64, CopyError> {
    // Step 1: stat the source first; a missing source never touches dst.
    // Capture the source mtime so the commit can preserve it (dup avoidance
    // matches on size+mtime, so a fresh copy must compare equal).
    let src_mtime = match std::fs::metadata(src) {
        Ok(md) => match md.modified() {
            Ok(t) => t,
            Err(e) => return Err(CopyError::Io(e.to_string())),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(CopyError::SourceMissing)
        }
        Err(_) => return Err(CopyError::SourceMissing),
    };
    let file_name = match src.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => return Err(CopyError::SourceMissing),
    };

    // Step 2: temp path in the destination directory.
    let tmp = temp_path(dst_dir, &file_name);
    let final_path = dst_dir.join(&file_name);

    // Bounded memory: exactly two 1 MiB buffers per call, reused across the
    // initial attempt, the single retry, and all verify passes.
    let mut wbuf = vec![0u8; CHUNK];
    let mut rbuf = vec![0u8; CHUNK];

    // Steps 3-5: stream + fsync + readback verify, with one full retry on
    // mismatch only.
    let mut last: Option<(u64, u64)> = None;
    for attempt in 0..2 {
        match attempt_once(src, &tmp, dst_dir, &mut wbuf, &mut rbuf) {
            Ok(v) => {
                last = Some(v);
                break;
            }
            Err(CopyError::VerifyMismatch) => {
                remove_quiet(&tmp);
                if attempt == 1 {
                    return Err(CopyError::VerifyMismatch);
                }
                continue;
            }
            Err(e) => {
                remove_quiet(&tmp);
                return Err(e);
            }
        }
    }
    let (total, recorded) = match last {
        Some(v) => v,
        None => return Err(CopyError::VerifyMismatch),
    };

    // Step 6: commit, then preserve the source mtime on the committed file
    // so a size+mtime dup comparison matches.
    if let Err(e) = std::fs::rename(&tmp, &final_path) {
        remove_quiet(&tmp);
        return Err(CopyError::Io(e.to_string()));
    }
    if let Err(e) = File::options().read(true).open(&final_path).and_then(|f| f.set_modified(src_mtime)) {
        remove_quiet(&final_path);
        return Err(CopyError::Io(e.to_string()));
    }

    // Step 7: full verify reads the committed file once more.
    if verify == Verify::Full {
        let mut f = match File::open(&final_path) {
            Ok(f) => f,
            Err(e) => {
                remove_quiet(&final_path);
                return Err(CopyError::Io(e.to_string()));
            }
        };
        let mut h = Fnv1a::new();
        loop {
            match f.read(&mut rbuf[..]) {
                Ok(0) => break,
                Ok(n) => h.write(&rbuf[..n]),
                Err(e) => {
                    drop(f);
                    remove_quiet(&final_path);
                    return Err(CopyError::Io(e.to_string()));
                }
            }
        }
        drop(f);
        if h.finish() != recorded {
            remove_quiet(&final_path);
            return Err(CopyError::VerifyMismatch);
        }
    }

    Ok(total)
}

pub fn same_size_mtime(a: &Path, b: &Path) -> std::io::Result<bool> {
    let ma = std::fs::metadata(a)?;
    let mb = std::fs::metadata(b)?;
    if ma.len() != mb.len() {
        return Ok(false);
    }
    Ok(ma.modified()? == mb.modified()?)
}

#[cfg(test)]
#[path = "copy_tests.rs"]
mod tests;
