#[derive(Debug, PartialEq)]
pub struct Options {
    pub dry_run: bool,
}

pub const USAGE: &str = "exsync [--dry-run] | Env: EXSYNC_CONFIG, EXSYNC_LOG, EXSYNC_VOLUMES_ROOT";

pub fn parse_args<I: Iterator<Item = String>>(mut args: I) -> Result<Options, i32> {
    let first = args.next();
    if first.is_none() {
        return Ok(Options { dry_run: false });
    }

    let first_val = first.unwrap();
    if first_val == "--dry-run" {
        if args.next().is_none() {
            return Ok(Options { dry_run: true });
        }
    }

    Err(2)
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
