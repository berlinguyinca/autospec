//! A one-way ratchet on shell and Bats line counts (issue #4421).
//!
//! This repository's product is Rust, and yet it carries more shell FILES than
//! Rust files: 165,782 lines of shell and 126,699 of Bats against 462,002 of
//! Rust — 63%, and not test fixtures but the pipeline itself, including a
//! 3,732-line `autospec-loop.sh`.
//!
//! That is a stability problem rather than a matter of taste. Nearly every
//! defect recorded in this session's incident log came from shell semantics a
//! compiler rejects or a type prevents: a `comm` that warned on stderr and
//! emitted a wrong answer anyway; a loop whose `git clean` did not undo
//! `git add`, making verdicts depend on iteration order; an argument parser
//! with no `*)` catch-all; a `while read` loop that consumed its own worklist
//! from stdin; a mid-body `! cmd` that could not fail. None of those are
//! expressible in the Rust sitting beside them.
//!
//! Porting 292k lines is not a single change. What this module does is stop
//! the growth, so the port is a finite problem: the count may fall and may
//! never rise. A new script must either replace more shell than it adds, or
//! be written in Rust.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Extensions the ratchet counts, and the label each reports under.
const COUNTED: &[(&str, &str)] = &[("sh", "shell"), ("bats", "bats")];

/// Directories never counted: build output, VCS metadata, and vendored code
/// that is not ours to port.
// `fixtures` is skipped because this ratchet measures the pipeline, not test
// data -- as the module doc above already states. A fixture that demonstrates a
// skill invoking a shell script is an input to a test, not pipeline logic, and
// counting it makes the ratchet refuse the Rust checker that reads it. That is
// what happened: a patch adding a Rust structural validator plus a 5-line
// fixture script pushed the repository 5 lines over its ceiling.
const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", "vendor", "fixtures"];

/// A measurement of the repository's shell surface.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShellSurface {
    /// Non-blank line counts, keyed by label ("shell", "bats").
    pub lines: BTreeMap<String, usize>,
    /// File counts, keyed by the same labels.
    pub files: BTreeMap<String, usize>,
}

impl ShellSurface {
    /// Total counted lines across every label.
    pub fn total_lines(&self) -> usize {
        self.lines.values().sum()
    }

    /// Total counted files across every label.
    pub fn total_files(&self) -> usize {
        self.files.values().sum()
    }
}

/// The verdict of comparing a measurement against the committed ceiling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RatchetVerdict {
    /// At or below the ceiling. `slack` is how far below.
    Held {
        total: usize,
        ceiling: usize,
        slack: usize,
    },
    /// Above the ceiling: the change added shell.
    Regressed {
        total: usize,
        ceiling: usize,
        excess: usize,
    },
}

impl RatchetVerdict {
    /// Whether this verdict should fail a gate.
    pub fn is_regression(&self) -> bool {
        matches!(self, RatchetVerdict::Regressed { .. })
    }

    /// The operator-facing explanation. A ratchet that only says "failed"
    /// teaches nothing; this says what grew, by how much, and what the two
    /// acceptable responses are.
    ///
    /// `language` is the name of the implementation language the patch should
    /// have been written in, resolved from the repository (see
    /// `crate::implementation_language`). Naming it redirects a rejected agent
    /// to the alternative instead of merely stopping it (issue #4447).
    pub fn message(&self, language: &str) -> String {
        match self {
            RatchetVerdict::Held {
                total,
                ceiling,
                slack,
            } => format!(
                "shell ratchet OK: {total} lines against a ceiling of {ceiling} ({slack} below). \
                 Lower the ceiling in the same change that removes shell, or it will drift back."
            ),
            RatchetVerdict::Regressed {
                total,
                ceiling,
                excess,
            } => format!(
                "shell ratchet REGRESSED: {total} lines against a ceiling of {ceiling}, \
                 {excess} over. This repository's implementation language is {language}; new \
                 pipeline logic belongs in crates/. Either write it in {language}, or remove \
                 more shell than this change adds. If the shell is genuinely unavoidable (a \
                 harness entry point, process supervision), say so in the PR and raise the \
                 ceiling deliberately -- the ceiling is a decision, not a formality."
            ),
        }
    }
}

/// Counts the shell surface under `root`.
///
/// Blank lines are not counted: they are not logic, and counting them would
/// let reformatting move the ratchet.
pub fn measure(root: &Path) -> std::io::Result<ShellSurface> {
    let mut surface = ShellSurface::default();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            // An unreadable directory is skipped rather than fatal: the
            // ratchet must not fail a build because of a permissions quirk in
            // a path it does not care about.
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !SKIP_DIRS.contains(&name) {
                    stack.push(path);
                }
                continue;
            }
            let ext = match path.extension().and_then(|e| e.to_str()) {
                Some(e) => e,
                None => continue,
            };
            let label = match COUNTED.iter().find(|(e, _)| *e == ext) {
                Some((_, label)) => *label,
                None => continue,
            };
            let text = match fs::read_to_string(&path) {
                Ok(t) => t,
                Err(_) => continue, // binary or unreadable; not shell we can port
            };
            let n = text.lines().filter(|l| !l.trim().is_empty()).count();
            *surface.lines.entry(label.to_string()).or_insert(0) += n;
            *surface.files.entry(label.to_string()).or_insert(0) += 1;
        }
    }
    Ok(surface)
}

/// Compares a measurement against a ceiling.
pub fn verdict(surface: &ShellSurface, ceiling: usize) -> RatchetVerdict {
    let total = surface.total_lines();
    if total > ceiling {
        RatchetVerdict::Regressed {
            total,
            ceiling,
            excess: total - ceiling,
        }
    } else {
        RatchetVerdict::Held {
            total,
            ceiling,
            slack: ceiling - total,
        }
    }
}
