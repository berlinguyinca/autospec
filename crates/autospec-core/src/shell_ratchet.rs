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
//!
//! A change is a diff, not a tree. A bug fix frequently needs *more* lines
//! than the bug — a missing guard, an extra case, a corrected loop — and a
//! ceiling on the total refuses the fix for the same reason it refuses a new
//! feature, even though one grows the surface and the other repairs it
//! (#4482). So the ratchet compares the head against the base it grew from
//! (`delta`, `diff_verdict`) and refuses only the part that is new surface —
//! a shell or Bats file absent from the base. Changes inside a file that
//! already exists and already runs are admitted and recorded per file
//! (`ShellDelta::modified`), so "this file grew while being repaired" stays
//! reportable without being blocking. Porting an existing file is tracked as
//! its own scheduled work, never as the implied precondition of its bug
//! fixes.

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
    /// Per-file non-blank line counts, keyed by path relative to the measured
    /// root. The per-file record is what lets a change be told apart as
    /// growth or repair: the total alone cannot say which file grew while
    /// being fixed.
    pub per_file: BTreeMap<String, usize>,
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
            let rel = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().into_owned(),
                Err(_) => path.to_string_lossy().into_owned(),
            };
            *surface.per_file.entry(rel).or_insert(0) += n;
        }
    }
    Ok(surface)
}

/// A change to the shell surface, from a base measurement to the head it
/// grew from. This is what the ratchet decides on: a patch is a diff, and
/// only the diff tells growth from repair.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShellDelta {
    /// Files present in the head but not the base, with their non-blank line
    /// counts. Anything here is new surface, and new surface is what the
    /// moratorium refuses.
    pub new_files: BTreeMap<String, usize>,
    /// Files present in both, keyed by path, as (base lines, head lines), for
    /// the files whose count changed. Recorded so growth inside an existing
    /// file is reportable without blocking the fix.
    pub modified: BTreeMap<String, (usize, usize)>,
    /// Files present in the base but gone from the head: shell being retired.
    pub removed_files: Vec<String>,
}

impl ShellDelta {
    /// All the lines the change introduces as new surface.
    pub fn new_lines(&self) -> usize {
        self.new_files.values().sum()
    }

    /// True when the change touches only files already present on the base:
    /// repair, not growth.
    pub fn maintenance_only(&self) -> bool {
        self.new_files.is_empty()
    }
}

/// Compares a head measurement against the base it grew from.
pub fn delta(base: &ShellSurface, head: &ShellSurface) -> ShellDelta {
    let mut out = ShellDelta::default();
    for (path, lines) in &head.per_file {
        match base.per_file.get(path) {
            Some(base_lines) => {
                if base_lines != lines {
                    out.modified.insert(path.clone(), (*base_lines, *lines));
                }
            }
            None => {
                out.new_files.insert(path.clone(), *lines);
            }
        }
    }
    for path in base.per_file.keys() {
        if !head.per_file.contains_key(path) {
            out.removed_files.push(path.clone());
        }
    }
    out.removed_files.sort();
    out
}

/// The verdict of a *change* against the ceiling. Where [`verdict`] measures
/// a tree, this measures the difference from the base: new shell or Bats
/// files are refused, and changes inside files that exist on the base are
/// admitted and recorded. A moratorium on new surface must not block repair
/// of existing surface (#4482).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RatchetDiffVerdict {
    /// The change adds no shell or Bats file. Changes to existing files are
    /// recorded in `delta.modified` so the maintenance burden stays visible,
    /// but they do not block.
    Admitted {
        delta: ShellDelta,
        head_total: usize,
        ceiling: usize,
    },
    /// The change adds at least one shell or Bats file absent from the base:
    /// the growth the rule exists to stop.
    RefusedNewFiles {
        delta: ShellDelta,
        head_total: usize,
        ceiling: usize,
    },
}

impl RatchetDiffVerdict {
    /// Whether this verdict should fail a gate.
    pub fn is_regression(&self) -> bool {
        matches!(self, RatchetDiffVerdict::RefusedNewFiles { .. })
    }

    /// The hold reason, for the queue and the patch ledger. It says which of
    /// the two things the ratchet distinguishes happened — new surface, or
    /// repair of existing surface — not just "shell ratchet failed".
    pub fn hold_reason(&self) -> &'static str {
        match self {
            RatchetDiffVerdict::Admitted { .. } => "modifies existing shell",
            RatchetDiffVerdict::RefusedNewFiles { .. } => "adds a new shell file",
        }
    }

    /// The operator-facing explanation.
    pub fn message(&self) -> String {
        match self {
            RatchetDiffVerdict::Admitted {
                delta,
                head_total,
                ceiling,
            } => {
                let modified = describe_modified(&delta.modified);
                format!(
                    "shell ratchet OK: no new shell or Bats files (head {head_total} lines against a \
                     ceiling of {ceiling}); existing files modified: {modified}. Growth inside \
                     existing files is recorded, not blocked: a moratorium on new surface must not \
                     block repair of it, and porting a file is tracked as its own work, not implied \
                     by this check.{}",
                    removed_note(delta)
                )
            }
            RatchetDiffVerdict::RefusedNewFiles {
                delta,
                head_total,
                ceiling,
            } => {
                let modified = describe_modified(&delta.modified);
                let new_files: Vec<String> = delta
                    .new_files
                    .iter()
                    .map(|(p, n)| format!("{p} ({n} lines)"))
                    .collect();
                format!(
                    "shell ratchet refused: adds a new shell file ({} new line(s) in {} file(s): \
                     {}); head {head_total} lines against a ceiling of {ceiling}. The ceiling \
                     applies to new surface, where it does what it was built for. This \
                     repository's product is Rust: write it in crates/, or file a porting issue — \
                     porting is tracked as its own scheduled work, never the implied precondition \
                     of a bug fix. Existing files modified in this diff: {modified}.{}",
                    delta.new_lines(),
                    delta.new_files.len(),
                    new_files.join(", "),
                    removed_note(delta)
                )
            }
        }
    }
}

/// Renders the per-file modification record, or "none" when the change
/// touches no existing file's line count.
fn describe_modified(modified: &BTreeMap<String, (usize, usize)>) -> String {
    if modified.is_empty() {
        return "none".to_string();
    }
    modified
        .iter()
        .map(|(p, (b, h))| format!("{p} {b} -> {h}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn removed_note(delta: &ShellDelta) -> String {
    if delta.removed_files.is_empty() {
        String::new()
    } else {
        format!(" Removed: {}.", delta.removed_files.join(", "))
    }
}

/// Compares a head measurement against the base it grew from and the
/// ceiling. The ceiling accounts for new files only, where it does what it
/// was built for; growth inside an existing file never refuses on its own.
pub fn diff_verdict(
    base: &ShellSurface,
    head: &ShellSurface,
    ceiling: usize,
) -> RatchetDiffVerdict {
    let d = delta(base, head);
    let head_total = head.total_lines();
    if d.new_files.is_empty() {
        RatchetDiffVerdict::Admitted {
            delta: d,
            head_total,
            ceiling,
        }
    } else {
        RatchetDiffVerdict::RefusedNewFiles {
            delta: d,
            head_total,
            ceiling,
        }
    }
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
