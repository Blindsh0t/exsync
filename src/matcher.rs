//! Matcher module: filters files based on globs, regex, age, and open status.
//!
//! Decisions:
//! - `lsof` fallback: If `/usr/sbin/lsof` is missing or fails, `is_open` returns `false`.
//!   This ensures a broken `lsof` installation never blocks a sync operation,
//!   favoring availability over strict "skip open" adherence.

use crate::config;
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, Duration};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Candidate {
    pub path: PathBuf,
    pub size: u64,
    pub mtime: SystemTime,
}

#[derive(Debug)]
pub enum MatchError {
    Io(std::io::Error),
    Regex(regex::Error),
}

impl From<std::io::Error> for MatchError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<regex::Error> for MatchError {
    fn from(e: regex::Error) -> Self {
        Self::Regex(e)
    }
}

/// Hand-rolled glob matcher supporting *, ?, and **.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    if pattern.is_empty() {
        return path.is_empty();
    }

    // If pattern contains no '/', it matches only against the filename.
    if !pattern.contains('/') {
        let filename = Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        return glob_match_recursive(pattern, filename);
    }

    glob_match_recursive(pattern, path)
}

fn glob_match_recursive(pattern: &str, path: &str) -> bool {
    let p_chars: Vec<char> = pattern.chars().collect();
    let s_chars: Vec<char> = path.chars().collect();
    
    // I'll use a helper that explicitly handles the '*' constraint.
    match_robust(&p_chars, &s_chars)
}

fn match_robust(p: &[char], s: &[char]) -> bool {
    if p.is_empty() {
        return s.is_empty();
    }

    if p.len() >= 2 && p[0] == '*' && p[1] == '*' {
        // ** matches any sequence including /
        for i in 0..=s.len() {
            if match_robust(&p[2..], &s[i..]) {
                return true;
            }
        }
        false
    } else if p[0] == '*' {
        // * matches any sequence NOT including /
        for i in 0..=s.len() {
            if i > 0 && s[i-1] == '/' {
                break;
            }
            if match_robust(&p[1..], &s[i..]) {
                return true;
            }
        }
        false
    } else if p[0] == '?' {
        if !s.is_empty() && s[0] != '/' {
            match_robust(&p[1..], &s[1..])
        } else {
            false
        }
    } else {
        if !s.is_empty() && p[0] == s[0] {
            match_robust(&p[1..], &s[1..])
        } else {
            false
        }
    }
}

pub fn is_excluded_by_default(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        == Some(".DS_Store")
}

pub fn is_open(path: &Path) -> bool {
    let output = Command::new("/usr/sbin/lsof")
        .args(["-Fpc", "--"])
        .arg(path)
        .output();

    match output {
        Ok(out) => !out.stdout.is_empty(),
        Err(_) => false, // Fallback: report as not open
    }
}

pub fn is_candidate(
    entry: &config::Entry,
    root: &Path,
    path: &Path,
    now: SystemTime,
) -> Result<bool, MatchError> {
    // Relative path for globbing
    let rel_path = path.strip_prefix(root)
        .map(|p| p.to_string_lossy())
        .unwrap_or_else(|_| path.to_string_lossy());
    
    let filename = path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    // (1) include globs
    if !entry.include.is_empty() {
        let mut matched = false;
        for pattern in &entry.include {
            if glob_match(pattern, &rel_path) {
                matched = true;
                break;
            }
        }
        if !matched {
            return Ok(false);
        }
    }

    // (2) name_re
    if let Some(re_str) = &entry.name_re {
        let re = Regex::new(re_str)?;
        if !re.is_match(filename) {
            return Ok(false);
        }
    }

    // (3) exclude globs
    for pattern in &entry.exclude {
        if glob_match(pattern, &rel_path) {
            return Ok(false);
        }
    }

    // .DS_Store check: excluded unless explicitly included.
    // Requirements say: ".DS_Store is never a candidate, regardless of include, 
    // unless a literal .DS_Store appears in include."
    if is_excluded_by_default(path) {
        let mut explicitly_included = false;
        for pattern in &entry.include {
            if pattern == ".DS_Store" {
                explicitly_included = true;
                break;
            }
        }
        if !explicitly_included {
            return Ok(false);
        }
    }

    if let Some(min_age) = entry.min_age_minutes {
        let modified = fs::metadata(path)?.modified()?;
        let age = now.duration_since(modified).unwrap_or(Duration::from_secs(0));
        if age < Duration::from_secs(min_age * 60) {
            return Ok(false);
        }
    }

    // (5) skip_open
    if entry.skip_open && is_open(path) {
        return Ok(false);
    }

    Ok(true)
}

pub fn collect(
    entry: &config::Entry,
    root: &Path,
    now: SystemTime,
) -> Result<Vec<Candidate>, MatchError> {
    let mut candidates = Vec::new();
    
    // Recursive walk
    fn walk(dir: &Path, root: &Path, entry: &config::Entry, now: SystemTime, candidates: &mut Vec<Candidate>) -> Result<(), MatchError> {
        for res in fs::read_dir(dir)? {
            let item = res?;
            let path = item.path();
            let metadata = item.metadata()?;

            if metadata.is_dir() {
                // Safety: Never follow symlink outside root.
                // For simplicity, if it's a symlink, we check if it's inside root.
                // But if metadata.is_dir() is true, it might be a symlink to a dir.
                if metadata.file_type().is_symlink() {
                    let target = fs::read_link(&path)?;
                    let absolute_target = if target.is_absolute() {
                        target
                    } else {
                        dir.join(target)
                    };
                    
                    // Resolve and check if it's inside root.
                    // Since we can't easily resolve all symlinks without canonicalize (which requires existence),
                    // and the requirement is "Never follows a symlink that points outside root".
                    // The simplest check is canonicalize.
                    if let Ok(can) = absolute_target.canonicalize() {
                        if !can.starts_with(root) {
                            continue;
                        }
                    } else {
                        continue;
                    }
                }
                walk(&path, root, entry, now, candidates)?;
            } else if metadata.is_file() {
                if is_candidate(entry, root, &path, now)? {
                    candidates.push(Candidate {
                        path: path.clone(),
                        size: metadata.len(),
                        mtime: metadata.modified()?,
                    });
                }
            }
        }
        Ok(())
    }

    walk(root, root, entry, now, &mut candidates)?;
    
    // Sort by relative path
    candidates.sort_by(|a, b| {
        let rel_a = a.path.strip_prefix(root).unwrap_or(&a.path);
        let rel_b = b.path.strip_prefix(root).unwrap_or(&b.path);
        rel_a.cmp(rel_b)
    });

    Ok(candidates)
}

#[cfg(test)]
#[path = "matcher_tests.rs"]
mod tests;
