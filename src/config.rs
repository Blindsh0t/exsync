//! Config module: strict TOML parsing and Entry model.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Mirror,
    Move,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verify {
    Readback,
    Full,
}

impl Default for Verify {
    fn default() -> Self {
        Verify::Readback
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub mode: Mode,
    pub drive: String,
    pub source: PathBuf,
    pub dest: String,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub name_re: Option<String>,
    pub min_age_minutes: Option<u64>,
    pub skip_open: bool,
    pub verify: Verify,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    message: String,
}

impl ConfigError {
    fn new(message: String) -> Self {
        Self { message }
    }

    pub fn code(&self) -> i32 {
        2
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug, Clone)]
enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
    StrArray(Vec<String>),
}

struct RawEntry {
    fields: HashMap<String, (Value, usize)>,
    seen: HashSet<String>,
}

impl RawEntry {
    fn new() -> Self {
        Self {
            fields: HashMap::new(),
            seen: HashSet::new(),
        }
    }
}

fn is_valid_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Strip a `#` comment that appears outside a double-quoted string.
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_quote = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_quote {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_quote = false;
            }
        } else if b == b'"' {
            in_quote = true;
        } else if b == b'#' {
            return &line[..i];
        }
    }
    line
}

/// Parse a fully quoted `"..."` string with `\"` and `\\` escapes.
fn parse_quoted(s: &str) -> Option<String> {
    let b = s.as_bytes();
    if b.is_empty() || b[0] != b'"' {
        return None;
    }
    let mut out = String::new();
    let mut i = 1;
    while i < b.len() {
        let c = b[i];
        if c == b'\\' {
            i += 1;
            if i >= b.len() {
                return None;
            }
            match b[i] {
                b'n' => out.push('\n'),
                b't' => out.push('\t'),
                b'"' => out.push('"'),
                b'\\' => out.push('\\'),
                other => {
                    out.push('\\');
                    out.push(other as char);
                }
            }
            i += 1;
        } else if c == b'"' {
            i += 1;
            // Only trailing whitespace allowed after the closing quote.
            if s[i..].trim().is_empty() {
                return Some(out);
            }
            return None;
        } else {
            // Decode one UTF-8 char starting at byte index i.
            let rest = &s[i..];
            let ch = rest.chars().next()?;
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    None
}

fn parse_string_array(s: &str, line_no: usize) -> Result<Vec<String>, ConfigError> {
    let t = s.trim();
    if !t.starts_with('[') || !t.ends_with(']') {
        return Err(ConfigError::new(format!(
            "malformed line {line_no}: expected string array"
        )));
    }
    let inner = &t[1..t.len() - 1];
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    // Split on commas outside quotes.
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    let mut escaped = false;
    for ch in inner.chars() {
        if in_quote {
            cur.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_quote = false;
            }
        } else if ch == '"' {
            in_quote = true;
            cur.push(ch);
        } else if ch == ',' {
            parts.push(cur);
            cur = String::new();
        } else {
            cur.push(ch);
        }
    }
    parts.push(cur);
    // Allow one trailing comma: drop a single trailing empty part.
    if let Some(last) = parts.last() {
        if last.trim().is_empty() {
            parts.pop();
        }
    }
    let mut out = Vec::with_capacity(parts.len());
    for part in parts {
        let p = part.trim();
        if p.is_empty() {
            return Err(ConfigError::new(format!(
                "malformed line {line_no}: empty array element"
            )));
        }
        match parse_quoted(p) {
            Some(v) => out.push(v),
            None => {
                return Err(ConfigError::new(format!(
                    "malformed line {line_no}: array elements must be quoted strings"
                )))
            }
        }
    }
    Ok(out)
}

fn parse_value(raw: &str, line_no: usize) -> Result<Value, ConfigError> {
    let t = raw.trim();
    if t.is_empty() {
        return Err(ConfigError::new(format!(
            "malformed line {line_no}: missing value after `=`"
        )));
    }
    if t.starts_with('"') {
        match parse_quoted(t) {
            Some(v) => return Ok(Value::Str(v)),
            None => {
                return Err(ConfigError::new(format!(
                    "malformed line {line_no}: bad quoted string"
                )))
            }
        }
    }
    if t.starts_with('[') {
        return parse_string_array(t, line_no).map(Value::StrArray);
    }
    if t == "true" {
        return Ok(Value::Bool(true));
    }
    if t == "false" {
        return Ok(Value::Bool(false));
    }
    if t.chars().all(|c| c.is_ascii_digit() || c == '-')
        && !t.is_empty()
        && t != "-"
    {
        match t.parse::<i64>() {
            Ok(n) => return Ok(Value::Int(n)),
            Err(_) => {
                return Err(ConfigError::new(format!(
                    "malformed line {line_no}: bad integer"
                )))
            }
        }
    }
    Err(ConfigError::new(format!(
        "malformed line {line_no}: unsupported value"
    )))
}

fn home_dir() -> String {
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() => h,
        // Fallback when HOME is unset; errors that involve tilde
        // expansion report this `/Users` fallback in their text.
        _ => "/Users".to_string(),
    }
}

fn expand_source(raw: &str) -> PathBuf {
    if raw == "~" {
        PathBuf::from(home_dir())
    } else if let Some(rest) = raw.strip_prefix("~/") {
        PathBuf::from(home_dir()).join(rest)
    } else {
        PathBuf::from(raw)
    }
}

fn field_as_string(
    raw: &RawEntry,
    entry_idx: usize,
    field: &str,
) -> Result<Option<(String, usize)>, ConfigError> {
    match raw.fields.get(field) {
        None => Ok(None),
        Some((Value::Str(s), line)) => Ok(Some((s.clone(), *line))),
        Some((_, line)) => Err(ConfigError::new(format!(
            "field '{field}' in entry {} expects a string at line {line}",
            entry_idx + 1
        ))),
    }
}

fn build_entry(raw: &RawEntry, entry_idx: usize) -> Result<Entry, ConfigError> {
    let at = |field: &str| -> String {
        format!("missing required field '{field}' in entry {}", entry_idx + 1)
    };

    let name = match field_as_string(raw, entry_idx, "name")? {
        Some((v, _)) => v,
        None => return Err(ConfigError::new(at("name"))),
    };
    let mode_raw = match field_as_string(raw, entry_idx, "mode")? {
        Some((v, _)) => v,
        None => return Err(ConfigError::new(at("mode"))),
    };
    let mode = match mode_raw.as_str() {
        "mirror" => Mode::Mirror,
        "move" => Mode::Move,
        other => {
            let line = raw.fields.get("mode").map(|(_, l)| *l).unwrap_or(0);
            return Err(ConfigError::new(format!(
                "invalid mode '{other}' in entry {} at line {line}: expected \"mirror\" or \"move\"",
                entry_idx + 1
            )));
        }
    };
    let drive = match field_as_string(raw, entry_idx, "drive")? {
        Some((v, _)) => v,
        None => return Err(ConfigError::new(at("drive"))),
    };
    let source_raw = match field_as_string(raw, entry_idx, "source")? {
        Some((v, _)) => v,
        None => return Err(ConfigError::new(at("source"))),
    };
    let dest = match field_as_string(raw, entry_idx, "dest")? {
        Some((v, _)) => v,
        None => return Err(ConfigError::new(at("dest"))),
    };

    let include = match raw.fields.get("include") {
        None => Vec::new(),
        Some((Value::StrArray(v), _)) => v.clone(),
        Some((_, line)) => {
            return Err(ConfigError::new(format!(
                "field 'include' in entry {} expects a string array at line {line}",
                entry_idx + 1
            )))
        }
    };
    let exclude = match raw.fields.get("exclude") {
        None => Vec::new(),
        Some((Value::StrArray(v), _)) => v.clone(),
        Some((_, line)) => {
            return Err(ConfigError::new(format!(
                "field 'exclude' in entry {} expects a string array at line {line}",
                entry_idx + 1
            )))
        }
    };
    let name_re = match field_as_string(raw, entry_idx, "name_re")? {
        Some((v, _)) => Some(v),
        None => None,
    };
    let min_age_minutes = match raw.fields.get("min_age_minutes") {
        None => None,
        Some((Value::Int(n), line)) => {
            if *n < 0 {
                return Err(ConfigError::new(format!(
                    "field 'min_age_minutes' in entry {} must be >= 0 at line {line}",
                    entry_idx + 1
                )));
            }
            Some(*n as u64)
        }
        Some((_, line)) => {
            return Err(ConfigError::new(format!(
                "field 'min_age_minutes' in entry {} expects an integer at line {line}",
                entry_idx + 1
            )))
        }
    };
    let skip_open = match raw.fields.get("skip_open") {
        None => false,
        Some((Value::Bool(b), _)) => *b,
        Some((_, line)) => {
            return Err(ConfigError::new(format!(
                "field 'skip_open' in entry {} expects true or false at line {line}",
                entry_idx + 1
            )))
        }
    };
    let verify = match field_as_string(raw, entry_idx, "verify")? {
        None => Verify::Readback,
        Some((v, line)) => match v.as_str() {
            "readback" => Verify::Readback,
            "full" => Verify::Full,
            other => {
                return Err(ConfigError::new(format!(
                    "invalid verify '{other}' in entry {} at line {line}: expected \"readback\" or \"full\"",
                    entry_idx + 1
                )))
            }
        },
    };

    Ok(Entry {
        name,
        mode,
        drive,
        source: expand_source(&source_raw),
        dest,
        include,
        exclude,
        name_re,
        min_age_minutes,
        skip_open,
        verify,
    })
}

/// Parse TOML-subset config text into entries.
pub fn parse(text: &str) -> Result<Vec<Entry>, ConfigError> {
    const ENTRY_KEYS: &[&str] = &[
        "name",
        "mode",
        "drive",
        "source",
        "dest",
        "include",
        "exclude",
        "name_re",
        "min_age_minutes",
        "skip_open",
        "verify",
    ];

    let mut version: Option<i64> = None;
    let mut top_seen: HashSet<String> = HashSet::new();
    let mut raws: Vec<RawEntry> = Vec::new();
    let mut current: Option<RawEntry> = None;

    for (idx, raw_line) in text.lines().enumerate() {
        let line_no = idx + 1;
        let code = strip_comment(raw_line);
        let trimmed = code.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('[') {
            let nospace: String =
                trimmed.chars().filter(|c| !c.is_whitespace()).collect();
            if nospace == "[[entry]]" {
                if version.is_none() {
                    return Err(ConfigError::new(format!(
                        "entry table [[entry]] before version at line {line_no}: expected version = 1 first"
                    )));
                }
                if let Some(done) = current.take() {
                    raws.push(done);
                }
                current = Some(RawEntry::new());
                continue;
            }
            // Any other bracket line is not part of the subset.
            return Err(ConfigError::new(format!(
                "malformed line {line_no}: expected [[entry]]"
            )));
        }

        let eq = match trimmed.find('=') {
            Some(p) => p,
            None => {
                return Err(ConfigError::new(format!(
                    "malformed line {line_no}: expected `key = value`"
                )))
            }
        };
        let key = trimmed[..eq].trim();
        let val_raw = trimmed[eq + 1..].trim();
        if !is_valid_key(key) {
            return Err(ConfigError::new(format!(
                "malformed line {line_no}: bad key"
            )));
        }
        let value = parse_value(val_raw, line_no)?;

        match current.as_mut() {
            None => {
                // Top-level table: only `version` is known.
                if key == "version" {
                    if version.is_some() || top_seen.contains("version") {
                        return Err(ConfigError::new(format!(
                            "duplicate version at line {line_no}: expected version = 1 once"
                        )));
                    }
                    match value {
                        Value::Int(n) => {
                            version = Some(n);
                            top_seen.insert(key.to_string());
                        }
                        Value::Str(s) => {
                            return Err(ConfigError::new(format!(
                                "unsupported version \"{s}\" at line {line_no}: expected version = 1"
                            )));
                        }
                        Value::Bool(b) => {
                            return Err(ConfigError::new(format!(
                                "unsupported version {b} at line {line_no}: expected version = 1"
                            )));
                        }
                        Value::StrArray(_) => {
                            return Err(ConfigError::new(format!(
                                "unsupported version array at line {line_no}: expected version = 1"
                            )));
                        }
                    }
                } else {
                    return Err(ConfigError::new(format!(
                        "unknown top-level key '{key}' at line {line_no}"
                    )));
                }
            }
            Some(entry) => {
                if key == "version" {
                    return Err(ConfigError::new(format!(
                        "duplicate version at line {line_no}: expected version = 1 once"
                    )));
                }
                if !ENTRY_KEYS.contains(&key) {
                    return Err(ConfigError::new(format!(
                        "unknown entry key '{key}' at line {line_no}"
                    )));
                }
                if entry.seen.contains(key) {
                    return Err(ConfigError::new(format!(
                        "duplicate key '{key}' at line {line_no}"
                    )));
                }
                entry.seen.insert(key.to_string());
                entry.fields.insert(key.to_string(), (value, line_no));
            }
        }
    }

    if let Some(done) = current.take() {
        raws.push(done);
    }

    match version {
        None => Err(ConfigError::new(
            "missing version (found none): expected version = 1".to_string(),
        )),
        Some(v) if v != i64::from(CONFIG_VERSION) => Err(ConfigError::new(format!(
            "unsupported version {v} (found {v}): expected version = 1"
        ))),
        Some(_) => {
            let mut out = Vec::with_capacity(raws.len());
            for (i, raw) in raws.iter().enumerate() {
                out.push(build_entry(raw, i)?);
            }
            Ok(out)
        }
    }
}

/// Load a config file. Performs no filesystem writes.
pub fn load(path: &Path) -> Result<Vec<Entry>, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(e) => Err(ConfigError::new(format!(
            "cannot read config file '{}': {e}",
            path.display()
        ))),
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
