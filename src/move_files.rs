//! MoveFiles module: verified move of candidate files to the destination drive.

use std::path::Path;

use crate::config;

/// Move every candidate of `entry`. Implemented by task MV-01.
pub fn run(_entry: &config::Entry, _vol_root: &Path, _dry_run: bool) -> Result<(), String> {
    Err("not implemented".to_string())
}
