//! Mirror module: Folder mirroring semantics.

use std::path::Path;

use crate::config::Entry;

/// Entry-point stub owned by a later task. TR-01 wires dispatch only.
pub fn run(_entry: &Entry, _vol_root: &Path, _dry_run: bool) -> Result<(), String> {
    Err("not implemented".to_string())
}
