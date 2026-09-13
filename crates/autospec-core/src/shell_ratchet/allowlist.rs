//! The per-file shell allowlist (issue #4442).
//!
//! One `<path> <count>` line per counted shell file, in a committed file
//! (shipped at `tests/fixtures/shell-ratchet-allowlist.txt`). The count is
//! the file's permitted non-blank line count, and it may only fall. The
//! mechanics mirror `scripts/lint-bats-negations.sh`: a file that exceeds
//! its own entry is a blocking finding, a counted file with no entry is a
//! blocking finding, and the sum of the entries — the effective ceiling —
//! still only falls, so the port of the shell surface stays finite.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::ShellSurface;

/// A failure to read or parse an allowlist.
#[derive(Debug)]
pub enum AllowlistError {
    /// A line that is neither blank, a `#` comment, nor `<path> <count>`.
    MalformedLine {
        /// 1-based line number in the allowlist text.
        line: usize,
        /// The offending line, verbatim.
        text: String,
    },
    /// A filesystem failure while loading or saving.
    Io(std::io::Error),
}

impl std::fmt::Display for AllowlistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedLine { line, text } => {
                write!(f, "allowlist line {line} is not `<path> <count>`: {text:?}")
            }
            Self::Io(e) => write!(f, "allowlist I/O error: {e}"),
        }
    }
}

impl std::error::Error for AllowlistError {}

impl From<std::io::Error> for AllowlistError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Permitted non-blank line count for each counted shell file.
///
/// Text format, one line per file (blank lines and `#` comments allowed):
///
/// ```text
/// scripts/foo.sh 120
/// tests/bar.bats 45
/// ```
///
/// [`Allowlist::render`] is the inverse of [`Allowlist::parse`]: it emits
/// every entry sorted by path with no comments, so a rendered file
/// round-trips byte-for-byte. A reseed therefore rewrites the whole file;
/// hand comments do not survive it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Allowlist {
    entries: BTreeMap<String, usize>,
}

impl Allowlist {
    /// The permitted count for `path`, if the allowlist lists it.
    pub fn get(&self, path: &str) -> Option<usize> {
        self.entries.get(path).copied()
    }

    /// How many files the allowlist lists.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the allowlist lists no files.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entries, keyed by path, in sorted order.
    pub fn entries(&self) -> &BTreeMap<String, usize> {
        &self.entries
    }

    /// The sum of every entry: the effective ceiling on counted lines.
    pub fn total(&self) -> usize {
        self.entries.values().sum()
    }

    /// Seed a fresh allowlist from a measurement: every counted file gets
    /// its current count. The seed is the one moment entries may be set
    /// from the tree; from then on they only fall.
    pub fn seed(surface: &ShellSurface) -> Self {
        let entries = surface
            .per_file
            .iter()
            .map(|(path, lines)| (path.clone(), *lines))
            .collect();
        Self { entries }
    }

    /// Parse allowlist text.
    pub fn parse(text: &str) -> Result<Self, AllowlistError> {
        let mut entries = BTreeMap::new();
        for (idx, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            // The count is the last whitespace-separated token; the path is
            // everything before it, so paths with spaces survive.
            let (path_part, count_part) = match trimmed.rsplit_once(char::is_whitespace) {
                Some(p) => p,
                None => {
                    return Err(AllowlistError::MalformedLine {
                        line: idx + 1,
                        text: line.to_string(),
                    })
                }
            };
            let count: usize = count_part
                .parse()
                .map_err(|_| AllowlistError::MalformedLine {
                    line: idx + 1,
                    text: line.to_string(),
                })?;
            let path = path_part.trim();
            if path.is_empty() {
                return Err(AllowlistError::MalformedLine {
                    line: idx + 1,
                    text: line.to_string(),
                });
            }
            entries.insert(path.to_string(), count);
        }
        Ok(Self { entries })
    }

    /// Load the allowlist from a file.
    pub fn load(path: &Path) -> Result<Self, AllowlistError> {
        Self::parse(&fs::read_to_string(path)?)
    }

    /// Render every entry sorted by path, one `<path> <count>` per line.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for (path, count) in &self.entries {
            out.push_str(path);
            out.push(' ');
            out.push_str(&count.to_string());
            out.push('\n');
        }
        out
    }

    /// Write the rendered allowlist to a file, creating parent directories.
    pub fn save(&self, path: &Path) -> Result<(), AllowlistError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, self.render())?;
        Ok(())
    }
}
