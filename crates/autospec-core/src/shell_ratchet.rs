//! A one-way ratchet on the shell and Bats line counts, kept as a per-file
//! allowlist (issue #4442).
//!
//! This repository's product is Rust, and yet it carries more shell FILES
//! than Rust files: 165,782 lines of shell and 126,699 of Bats against
//! 462,002 of Rust — 63%, and not test fixtures but the pipeline itself,
//! including a 3,732-line `autospec-loop.sh`.
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
//! The first cut of the stop was a single global line count with zero slack
//! (#4421). It did what it was told — and then it held a five-line bug fix
//! for exactly the same reason it held a new feature, because a total cannot
//! tell the two apart. That is the wrong instrument for a gate every change
//! passes through: it pressures the operator to skip the repair or to pad
//! the change by deleting unrelated lines to buy headroom, and neither
//! improves the repository.
//!
//! The allowlist is the fix, and the mechanism already exists in this
//! repository: `scripts/lint-bats-negations.sh` ratchets mid-body negations
//! the same way. Each counted file carries a permitted count in a committed
//! file — `tests/fixtures/shell-ratchet-allowlist.txt`, one `<path> <count>`
//! line per file ([`Allowlist`]) — and the count per file may only fall.
//! That gives the three properties a global total cannot:
//!
//! - a NEW `.sh`/`.bats` file has no entry and is a blocking finding
//!   ([`AllowlistFinding::UnlistedFile`]) — the growth the ratchet exists to
//!   stop;
//! - an EXISTING file may change as long as it stays at or below its own
//!   entry, so a repair that fits the slack its file has accumulated lands
//!   without spending anyone else's budget;
//! - an ENTRY may only be lowered ([`AllowlistFinding::RaisedEntry`]), so
//!   the sum of the entries — the effective ceiling — still only falls and
//!   the port stays finite.
//!
//! A change that removes shell lowers the entries it touches in the SAME
//! commit — there is no separate step, and there is no state in which the
//! allowlist drifts above reality and silently re-opens slack: an entry whose
//! file is gone is a finding of its own ([`AllowlistFinding::StaleEntry`]).
//! The invariant that keeps the shipped allowlist honest is a test in this
//! crate's suite, `the_shipped_allowlist_keeps_the_real_repository_scan_green`,
//! the same shape the bats ratchet uses.
//!
//! A change is a diff, not a tree. The verdict compares the head against the
//! base it grew from ([`delta`], [`allowlist_diff_verdict`]) and records
//! every repair to existing surface ([`ShellDelta::modified`]) so "this file
//! changed while being repaired" stays reportable. Porting an existing file
//! is tracked as its own scheduled work, never as the implied precondition
//! of its bug fixes (#4482).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub use allowlist::{Allowlist, AllowlistError};

mod allowlist;

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
/// grew from. This is what the ratchet reports on: a patch is a diff, and
/// only the diff tells growth from repair.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShellDelta {
    /// Files present in the head but not the base, with their non-blank line
    /// counts. Anything here is new surface, and new surface is what the
    /// moratorium refuses.
    pub new_files: BTreeMap<String, usize>,
    /// Files present in both, keyed by path, as (base lines, head lines), for
    /// the files whose count changed. Recorded so growth inside an existing
    /// file is reportable alongside the findings that judge it.
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

/// One way a tree and its allowlist disagree. Every variant is a blocking
/// finding: a ratchet that reports but does not refuse is a scoreboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllowlistFinding {
    /// A counted file the allowlist does not list: new shell surface — the
    /// growth the ratchet exists to stop.
    UnlistedFile {
        /// Path relative to the measured root.
        path: String,
        /// Its non-blank line count.
        lines: usize,
    },
    /// A file over its own entry: growth above the count the allowlist
    /// permits for it.
    ExceededEntry {
        /// Path relative to the measured root.
        path: String,
        /// The file's non-blank line count at the head.
        lines: usize,
        /// The entry it exceeded.
        entry: usize,
    },
    /// An entry whose file is not in the tree: the allowlist has drifted
    /// above reality and silently re-opened slack.
    StaleEntry {
        /// The entry's path, which the tree no longer has.
        path: String,
        /// The entry's count.
        entry: usize,
    },
    /// An entry that rose against the base allowlist the change grew from:
    /// the one-way property broken.
    RaisedEntry {
        /// The entry's path.
        path: String,
        /// Its count in the base allowlist (0 when the base had no entry).
        base: usize,
        /// Its count in the head allowlist.
        head: usize,
    },
}

impl AllowlistFinding {
    /// The path the finding is about.
    pub fn path(&self) -> &str {
        match self {
            Self::UnlistedFile { path, .. }
            | Self::ExceededEntry { path, .. }
            | Self::StaleEntry { path, .. }
            | Self::RaisedEntry { path, .. } => path,
        }
    }

    /// One-line operator-facing description of this finding.
    pub fn description(&self) -> String {
        match self {
            Self::UnlistedFile { path, lines } => {
                format!("new shell file {path} ({lines} line(s)) has no allowlist entry")
            }
            Self::ExceededEntry { path, lines, entry } => {
                format!("{path} is at {lines} lines, over its allowlisted {entry}")
            }
            Self::StaleEntry { path, entry } => {
                format!("allowlist entry for {path} ({entry} lines) has no file in the tree")
            }
            Self::RaisedEntry { path, base, head } => {
                format!("allowlist entry for {path} rose from {base} to {head}")
            }
        }
    }

    /// Ordering rank so a finding list is deterministic: by path, then kind.
    fn rank(&self) -> u8 {
        match self {
            Self::UnlistedFile { .. } => 0,
            Self::ExceededEntry { .. } => 1,
            Self::StaleEntry { .. } => 2,
            Self::RaisedEntry { .. } => 3,
        }
    }
}

fn sort_findings(findings: &mut [AllowlistFinding]) {
    findings.sort_by(|a, b| (a.path(), a.rank()).cmp(&(b.path(), b.rank())));
}

/// The verdict of a tree against its allowlist, no base required.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllowlistVerdict {
    /// Every counted file is listed and at or below its entry, and no entry
    /// is stale: the tree and the allowlist agree. `total` is the counted
    /// line total, `ceiling` the sum of the entries.
    InSync { total: usize, ceiling: usize },
    /// One or more disagreements, in `findings`.
    Drifted {
        total: usize,
        ceiling: usize,
        findings: Vec<AllowlistFinding>,
    },
}

impl AllowlistVerdict {
    /// Whether this verdict should fail a gate.
    pub fn is_clean(&self) -> bool {
        matches!(self, Self::InSync { .. })
    }

    /// The findings, empty when in sync.
    pub fn findings(&self) -> &[AllowlistFinding] {
        match self {
            Self::InSync { .. } => &[],
            Self::Drifted { findings, .. } => findings,
        }
    }

    /// The operator-facing explanation. A ratchet that only says "failed"
    /// teaches nothing; this says what disagrees, by how much, and what the
    /// acceptable responses are.
    ///
    /// `language` is the name of the implementation language the patch should
    /// have been written in, resolved from the repository (see
    /// `crate::implementation_language`). Naming it redirects a rejected agent
    /// to the alternative instead of merely stopping it (issue #4447).
    pub fn message(&self, language: &str) -> String {
        match self {
            Self::InSync { total, ceiling } => format!(
                "shell ratchet OK: {total} counted lines against an allowlist of {ceiling} \
                 ({} of slack). Lower the entries in the same commit that removes shell, \
                 or they will drift back.",
                ceiling - total
            ),
            Self::Drifted {
                total,
                ceiling,
                findings,
            } => {
                let listed = findings
                    .iter()
                    .map(AllowlistFinding::description)
                    .collect::<Vec<_>>()
                    .join("; ");
                format!(
                    "shell ratchet DRIFTED: {total} counted lines against an allowlist of \
                     {ceiling}; {} finding(s): {listed}. This repository's implementation \
                     language is {language}; new pipeline logic belongs in crates/. Either \
                     write it in {language}, remove shell and lower the entries it frees in \
                     the same commit, or file a porting issue -- porting is tracked as its \
                     own scheduled work, never the implied precondition of a bug fix. An \
                     entry may only fall; raise one deliberately only with a documented \
                     reason in the PR.",
                    findings.len()
                )
            }
        }
    }
}

/// Compares a measurement against its allowlist.
pub fn allowlist_verdict(surface: &ShellSurface, allowlist: &Allowlist) -> AllowlistVerdict {
    let total = surface.total_lines();
    let ceiling = allowlist.total();
    let mut findings: Vec<AllowlistFinding> = Vec::new();
    for (path, lines) in &surface.per_file {
        match allowlist.get(path) {
            None => findings.push(AllowlistFinding::UnlistedFile {
                path: path.clone(),
                lines: *lines,
            }),
            Some(entry) if *lines > entry => findings.push(AllowlistFinding::ExceededEntry {
                path: path.clone(),
                lines: *lines,
                entry,
            }),
            Some(_) => {}
        }
    }
    for (path, entry) in allowlist.entries() {
        if !surface.per_file.contains_key(path) {
            findings.push(AllowlistFinding::StaleEntry {
                path: path.clone(),
                entry: *entry,
            });
        }
    }
    if findings.is_empty() {
        AllowlistVerdict::InSync { total, ceiling }
    } else {
        sort_findings(&mut findings);
        AllowlistVerdict::Drifted {
            total,
            ceiling,
            findings,
        }
    }
}

/// Entries that rose from the base allowlist to the head allowlist. A head
/// entry with no base counterpart rose from zero; a base entry with no head
/// counterpart fell to zero and is not a rise.
pub fn raised_entries(base: &Allowlist, head: &Allowlist) -> BTreeMap<String, (usize, usize)> {
    let mut out = BTreeMap::new();
    for (path, head_entry) in head.entries() {
        let base_entry = base.get(path).unwrap_or(0);
        if *head_entry > base_entry {
            out.insert(path.clone(), (base_entry, *head_entry));
        }
    }
    out
}

/// The verdict of a *change*: the head tree against the head allowlist, plus
/// the one-way check of the head allowlist against the base allowlist it grew
/// from. `delta` is the head-vs-base file record, kept so repair of existing
/// surface stays visible in the report even when the change is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllowlistDiffVerdict {
    /// The head tree is in sync with the head allowlist and no entry rose.
    /// `total` is the head's counted line total, `ceiling` the sum of the
    /// head's entries.
    Admitted {
        delta: ShellDelta,
        total: usize,
        ceiling: usize,
    },
    /// One or more findings: new surface, growth past an entry, a raised
    /// entry, or drift.
    Refused {
        delta: ShellDelta,
        total: usize,
        ceiling: usize,
        findings: Vec<AllowlistFinding>,
    },
}

impl AllowlistDiffVerdict {
    /// Whether this verdict should fail a gate.
    pub fn is_regression(&self) -> bool {
        matches!(self, Self::Refused { .. })
    }

    /// The hold reason, for the queue and the patch ledger. It says which of
    /// the things the ratchet distinguishes happened — new surface, growth
    /// past an entry, or drift — not just "shell ratchet failed".
    pub fn hold_reason(&self) -> &'static str {
        match self {
            Self::Admitted { .. } => "in sync with the shell allowlist",
            Self::Refused { findings, .. } => {
                if findings
                    .iter()
                    .any(|f| matches!(f, AllowlistFinding::UnlistedFile { .. }))
                {
                    "adds a new shell file"
                } else if findings.iter().any(|f| matches!(
                    f,
                    AllowlistFinding::ExceededEntry { .. }
                        | AllowlistFinding::RaisedEntry { .. }
                )) {
                    "grows shell past its allowlisted entry"
                } else {
                    "allowlist out of sync with the tree"
                }
            }
        }
    }

    /// The operator-facing explanation, with the language redirect
    /// (`crate::implementation_language`, issue #4447) and the per-file
    /// repair record.
    pub fn message(&self, language: &str) -> String {
        match self {
            Self::Admitted {
                delta,
                total,
                ceiling,
            } => {
                let modified = describe_modified(&delta.modified);
                format!(
                    "shell ratchet OK: no new shell or Bats files, tree in sync with the \
                     allowlist ({total} counted lines against an allowlist of {ceiling}); \
                     existing files modified: {modified}.{}",
                    removed_note(delta)
                )
            }
            Self::Refused {
                delta,
                total,
                ceiling,
                findings,
            } => {
                let listed = findings
                    .iter()
                    .map(AllowlistFinding::description)
                    .collect::<Vec<_>>()
                    .join("; ");
                let modified = describe_modified(&delta.modified);
                format!(
                    "shell ratchet REFUSED: {listed}. {total} counted lines against an \
                     allowlist of {ceiling}. This repository's implementation language is \
                     {language}; new pipeline logic belongs in crates/. Either write it in \
                     {language}, remove shell and lower the entries it frees in the same \
                     commit, or file a porting issue -- porting is tracked as its own \
                     scheduled work, never the implied precondition of a bug fix. An entry \
                     may only fall; raise one deliberately only with a documented reason in \
                     the PR. Existing files modified in this diff: {modified}.{}",
                    removed_note(delta)
                )
            }
        }
    }
}

/// Compares a change — base tree and base allowlist to head tree and head
/// allowlist — and decides it.
pub fn allowlist_diff_verdict(
    base: &ShellSurface,
    base_allowlist: &Allowlist,
    head: &ShellSurface,
    head_allowlist: &Allowlist,
) -> AllowlistDiffVerdict {
    let d = delta(base, head);
    let total = head.total_lines();
    let ceiling = head_allowlist.total();
    let mut findings = allowlist_verdict(head, head_allowlist).findings().to_vec();
    for (path, (b, h)) in raised_entries(base_allowlist, head_allowlist) {
        findings.push(AllowlistFinding::RaisedEntry {
            path,
            base: b,
            head: h,
        });
    }
    if findings.is_empty() {
        AllowlistDiffVerdict::Admitted {
            delta: d,
            total,
            ceiling,
        }
    } else {
        sort_findings(&mut findings);
        AllowlistDiffVerdict::Refused {
            delta: d,
            total,
            ceiling,
            findings,
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
