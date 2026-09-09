//! A fix is verified against the surface the defect lived on, not the diff (#3905).
//!
//! Three same-day repairs, each verified against its own subject while the
//! defect lived on a wider surface:
//!
//! - a registration fix was checked on the one failing target and missed a
//!   ninth count site in the other crate, breaking `main` again and falsely
//!   holding every subsequent patch (#3891 → #3902);
//! - a new guard script was exercised positive and negative, but it had been
//!   wired into a script belonging to a different repository — a check that
//!   ran nowhere, which reads as coverage and provides none (#3903 → #3904);
//! - a regenerated artefact passed the full Rust gate set but not the web
//!   image build, whose lockfile pinned the artefact's integrity hash.
//!
//! The pattern: the failing test tells you what to run, and running exactly
//! that answers a narrower question than the one being asked. The size of the
//! diff is taken as a proxy for blast radius, and it is a bad one — the count
//! bump was four literals and cost a cascade.
//!
//! This module makes the invariants from #3905 mechanical. Everything here is
//! pure and testable: no I/O, no `gh`, no subprocess. The caller supplies the
//! patch's title, body, and diff text; this module decides which holds fire
//! and what would clear them.
//!
//! Body grammar (exact keys, first occurrence wins):
//!
//! ```text
//! Fix surface: <the surface the defect lived on>
//! Fix gates: <gate 1, gate 2, ...>
//! Cascade: <no | description of the held verdicts>
//! Re-ran held verdicts: <verdict 1, verdict 2, ...>
//! Invocation read-back:
//!   <lines read back from the parsed configuration, to a blank line>
//! ```
//!
//! Invariants, one hold kind each:
//!
//! 1. **A fix records the surface the defect lived on, and the gates that
//!    cover that surface.** A fix-labeled patch (title prefix `fix`,
//!    [`is_fix_label`]) must record both `Fix surface:` and `Fix gates:`.
//!    One entry per gate the surface has; an entry of exactly `n/a` covers
//!    nothing. ([`HoldKind::SurfaceUnrecorded`], [`HoldKind::GatesUnrecorded`])
//! 2. **A fix that adds a control must demonstrate the control runs.** A
//!    patch that adds a check file — a new file under `scripts/`, or a new
//!    `.bats` suite anywhere — must show the invocation: a line (added or
//!    context) in another file of the same patch that references the check
//!    (self-registration — a context line shows the file's current content,
//!    so it is proof of an invocation that already exists), or an
//!    `Invocation read-back:` block naming the check (read back from the
//!    parsed configuration, not the diff). Rust
//!    integration tests under `crates/*/tests/` are discovered by the
//!    compiler and are not held. ([`HoldKind::CheckUninvoked`])
//! 3. **Where the defect was a cascade, re-run the things it held.** A base
//!    fix invalidates every verdict computed against the broken base; those
//!    verdicts are part of the blast radius. A patch that declares
//!    `Cascade:` must list the re-run verdicts in
//!    `Re-ran held verdicts:`. ([`HoldKind::HeldVerdictsUnrerun`])
//!
//! A patch that cannot be read is held, not waved through
//! ([`HoldKind::PatchUnreadable`]): the check-addition half cannot be
//! verified from unreadable diff text. The body invariants still evaluate —
//! they do not depend on the diff — so an unreadable fix-labeled patch
//! carries both its body holds and the fail-closed hold.
//!
//! This module's own registration follows invariant 2: it is wired into the
//! crate via `pub mod fix_surface;` in `lib.rs` (self-registration in the
//! same patch) and is invoked by `cargo test` through
//! `tests/fix_surface.rs`, which cargo discovers on its own.

use crate::lint::diff::{parse_unified_diff, DiffLine, DiffLineKind};

/// The hold reason recorded for every hold in this module (issue #3905).
pub const HOLD_REASON: &str = "FIX_VERIFIED_ON_DIFF_ONLY";

/// Default title prefix that labels a patch as a fix.
pub const DEFAULT_FIX_PREFIX: &str = "fix";

/// Default path prefix treated as check-script territory.
pub const DEFAULT_CHECK_PREFIX: &str = "scripts/";

/// Default suffix treated as a test-suite registration surface.
pub const DEFAULT_CHECK_SUFFIX: &str = ".bats";

/// Body field keys. Exact, case-sensitive; first occurrence wins.
pub const SURFACE_KEY: &str = "Fix surface:";
pub const GATES_KEY: &str = "Fix gates:";
pub const CASCADE_KEY: &str = "Cascade:";
pub const RE_RAN_KEY: &str = "Re-ran held verdicts:";
pub const READ_BACK_KEY: &str = "Invocation read-back:";

/// A list entry that declares the list to be empty; it covers nothing.
const NA_ENTRY: &str = "n/a";

/// The text the caller saw on the patch: its title and its body (a PR body
/// or a commit message).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PatchRecord {
    pub title: String,
    pub body: String,
}

impl PatchRecord {
    pub fn new(title: &str, body: &str) -> Self {
        Self {
            title: title.to_string(),
            body: body.to_string(),
        }
    }

    /// A titled patch with no body — the strongest version of the failure:
    /// nothing to read back.
    pub fn titled(title: &str) -> Self {
        Self {
            title: title.to_string(),
            body: String::new(),
        }
    }
}

/// Policy for a review run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixSurfacePolicy {
    /// Title prefixes that label a patch as a fix (compared case-insensitively,
    /// followed by an optional `(scope)`, an optional `!`, and a `:`).
    pub fix_title_prefixes: Vec<String>,
    /// Path prefixes whose newly added files are check scripts.
    pub check_path_prefixes: Vec<String>,
    /// Path suffixes whose newly added files are check suites.
    pub check_path_suffixes: Vec<String>,
}

impl Default for FixSurfacePolicy {
    fn default() -> Self {
        Self {
            fix_title_prefixes: vec![DEFAULT_FIX_PREFIX.to_string()],
            check_path_prefixes: vec![DEFAULT_CHECK_PREFIX.to_string()],
            check_path_suffixes: vec![DEFAULT_CHECK_SUFFIX.to_string()],
        }
    }
}

impl FixSurfacePolicy {
    /// Whether `title` labels the patch as a fix: `fix:`, `fix(core):`,
    /// `fix!:`, case-insensitive. `fixup:` and `fixed:` do not.
    pub fn labeled_as_fix(&self, title: &str) -> bool {
        is_fix_label(title, self)
    }

    /// Whether a newly added file at `path` is a check whose invocation must
    /// be demonstrated.
    pub fn covers_check(&self, path: &str) -> bool {
        let path = path.replace('\\', "/");
        self.check_path_prefixes
            .iter()
            .any(|prefix| path.starts_with(prefix.as_str()))
            || self
                .check_path_suffixes
                .iter()
                .any(|suffix| path.ends_with(suffix.as_str()))
    }
}

/// Whether `title` carries a fix label under `policy` (`fix:`, `fix(scope):`,
/// `fix!:`, case-insensitive).
pub fn is_fix_label(title: &str, policy: &FixSurfacePolicy) -> bool {
    let title = title.trim().to_ascii_lowercase();
    policy.fix_title_prefixes.iter().any(|prefix| {
        let Some(rest) = title.strip_prefix(&prefix.to_ascii_lowercase()) else {
            return false;
        };
        let mut rest = rest;
        if let Some(inner) = rest.strip_prefix('(') {
            let Some(close) = inner.find(')') else {
                return false;
            };
            rest = &inner[close + 1..];
        }
        if let Some(banged) = rest.strip_prefix('!') {
            rest = banged;
        }
        rest.starts_with(':')
    })
}

/// Why a patch is held, beside the single hold reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldKind {
    /// A fix-labeled patch does not record the surface the defect lived on.
    SurfaceUnrecorded,
    /// A fix-labeled patch does not record the gates that cover that surface.
    GatesUnrecorded,
    /// A patch declares a cascade but does not record re-running the verdicts
    /// the broken base held.
    HeldVerdictsUnrerun,
    /// A newly added check has no added registration line and no parsed-config
    /// read-back naming it: it would run nowhere.
    CheckUninvoked,
    /// The patch is not readable diff text, so the check additions in it
    /// cannot be verified.
    PatchUnreadable,
}

impl HoldKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SurfaceUnrecorded => "SURFACE_UNRECORDED",
            Self::GatesUnrecorded => "GATES_UNRECORDED",
            Self::HeldVerdictsUnrerun => "HELD_VERDICTS_UNRERUN",
            Self::CheckUninvoked => "CHECK_UNINVOKED",
            Self::PatchUnreadable => "PATCH_UNREADABLE",
        }
    }
}

/// One held patch aspect: the sub-reason and the line that would clear it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixSurfaceHold {
    /// Always [`HOLD_REASON`]; kept as a field so a caller can match on it
    /// without importing the constant.
    pub code: &'static str,
    pub kind: HoldKind,
    /// Names the missing evidence and states what would clear the hold.
    pub message: String,
}

impl FixSurfaceHold {
    pub fn line(&self) -> String {
        self.message.clone()
    }
}

/// One newly added check file, and the two ways its invocation can be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckAddition {
    /// The added file's path, repo-relative.
    pub path: String,
    /// A line (added or context) in another file of the same patch references
    /// the check.
    pub self_registered: bool,
    /// The body's `Invocation read-back:` block references the check.
    pub read_back: bool,
}

impl CheckAddition {
    /// The control runs: either half of the evidence is sufficient.
    pub fn invoked(&self) -> bool {
        self.self_registered || self.read_back
    }
}

/// The outcome of one review.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FixSurfaceReview {
    /// Whether the title labels the patch as a fix.
    pub labeled_fix: bool,
    /// The recorded surface, if any.
    pub surface: Option<String>,
    /// The recorded gates, in body order (`n/a` entries removed).
    pub gates: Vec<String>,
    /// Whether the body declares the defect to have been a cascade.
    pub cascade_declared: bool,
    /// The recorded re-run verdicts, in body order (`n/a` entries removed).
    pub re_ran_held_verdicts: Vec<String>,
    /// Every newly added check file, in patch order, held or not.
    pub check_additions: Vec<CheckAddition>,
    /// Holds, in deterministic order: body holds, then per-check holds, then
    /// the fail-closed unreadable hold.
    pub holds: Vec<FixSurfaceHold>,
    /// Set when the check-addition half was held fail-closed (no readable file
    /// entries, or a parse error): the hold is fail-closed, and this says why.
    pub diff_parse_error: Option<String>,
}

impl FixSurfaceReview {
    pub fn held(&self) -> bool {
        !self.holds.is_empty()
    }

    pub fn clear(&self) -> bool {
        self.holds.is_empty()
    }

    /// One line per hold, for a PR comment or a monitor log.
    pub fn hold_lines(&self) -> String {
        self.holds
            .iter()
            .map(|hold| hold.message.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Hold every fix-labeled patch that does not record the defect's surface and
/// its covering gates, every newly added check whose invocation cannot be
/// shown, and every declared cascade whose held verdicts were not re-run.
///
/// The diff text is parsed with the shared unified-diff model; the title and
/// body are plain text. Nothing here calls out to a repository.
pub fn review_fix_surface(
    record: &PatchRecord,
    diff_source: &str,
    policy: &FixSurfacePolicy,
) -> FixSurfaceReview {
    let labeled_fix = policy.labeled_as_fix(&record.title);
    let surface = body_field(&record.body, SURFACE_KEY);
    let gates = body_field(&record.body, GATES_KEY)
        .as_deref()
        .map(split_entries)
        .unwrap_or_default();
    let cascade_value = body_field(&record.body, CASCADE_KEY).unwrap_or_default();
    let cascade_declared = !cascade_value.is_empty() && !cascade_value.eq_ignore_ascii_case("no");
    let re_ran_held_verdicts = body_field(&record.body, RE_RAN_KEY)
        .as_deref()
        .map(split_entries)
        .unwrap_or_default();
    let read_back = read_back_block(&record.body);

    let mut review = FixSurfaceReview {
        labeled_fix,
        surface,
        gates,
        cascade_declared,
        re_ran_held_verdicts,
        check_additions: Vec::new(),
        holds: Vec::new(),
        diff_parse_error: None,
    };
    if review.labeled_fix {
        if review.surface.is_none() {
            review.holds.push(hold(
                HoldKind::SurfaceUnrecorded,
                "labeled fix records no `Fix surface:` — the gate set is chosen from the surface the defect lived on, not from the diff; record the surface to clear".to_string(),
            ));
        }
        if review.gates.is_empty() {
            review.holds.push(hold(
                HoldKind::GatesUnrecorded,
                "labeled fix records no `Fix gates:` — list every gate that covers the recorded surface; one failing target is not the set".to_string(),
            ));
        }
        if cascade_declared && review.re_ran_held_verdicts.is_empty() {
            review.holds.push(hold(
                HoldKind::HeldVerdictsUnrerun,
                "labeled fix declares a cascade but records no `Re-ran held verdicts:` — every verdict computed against the broken base is part of the blast radius".to_string(),
            ));
        }
    }

    let diff = parse_unified_diff(diff_source);
    match &diff {
        Ok(diff) if diff.files.is_empty() && !diff_source.trim().is_empty() => {
            // Prose, a truncated patch, a patch fed in the wrong format:
            // nothing in it could be read, so its check additions cannot be
            // cleared either.
            review.holds.push(hold(
                HoldKind::PatchUnreadable,
                "patch text lists no `diff --git` file entries; the check additions in it cannot be verified and are held fail-closed".to_string(),
            ));
            review.diff_parse_error =
                Some("the patch lists no `diff --git` file entries".to_string());
            return review;
        }
        Ok(diff) => {
            for file in &diff.files {
                if !file.is_new || file.is_binary || !policy.covers_check(&file.path) {
                    continue;
                }
                let name = file_name(&file.path);
                let references = |line: &DiffLine| {
                    line.content.contains(&file.path) || line.content.contains(name)
                };
                // A context line shows the file's current content, so it is
                // proof of an invocation that already exists; a removed line
                // is the opposite and never counts.
                let self_registered = diff.files.iter().any(|other| {
                    other.path != file.path
                        && other.hunks.iter().any(|hunk| {
                            hunk.lines
                                .iter()
                                .filter(|line| line.kind != DiffLineKind::Removed)
                                .any(references)
                        })
                });
                let read_back = read_back
                    .iter()
                    .any(|line| line.contains(&file.path) || line.contains(name));
                let addition = CheckAddition {
                    path: file.path.clone(),
                    self_registered,
                    read_back,
                };
                if !addition.invoked() {
                    review.holds.push(hold(
                        HoldKind::CheckUninvoked,
                        format!(
                            "added check `{}` has no invocation: no line in another file of the patch registers it and no `Invocation read-back:` block names it — a check that runs nowhere reads as coverage and provides none",
                            file.path
                        ),
                    ));
                }
                review.check_additions.push(addition);
            }
        }
        Err(error) => {
            // A malformed hunk header: the diff model refused to guess. The
            // check-addition half is held fail-closed for the same reason.
            review.diff_parse_error = Some(error.clone());
            review.holds.push(hold(
                HoldKind::PatchUnreadable,
                "patch text is not a readable unified diff; the check additions in it cannot be verified and are held fail-closed".to_string(),
            ));
        }
    }

    review
}

fn hold(kind: HoldKind, message: String) -> FixSurfaceHold {
    FixSurfaceHold {
        code: HOLD_REASON,
        kind,
        message,
    }
}

/// The first body line that starts with `key`, with the key and surrounding
/// whitespace removed. A line with the key and an empty value is
/// unrecorded, not an empty-string record.
fn body_field(body: &str, key: &str) -> Option<String> {
    body.lines()
        .filter_map(|line| line.strip_prefix(key).map(|rest| rest.trim().to_string()))
        .find(|value| !value.is_empty())
}

/// One comma-separated list value into its entries, trimmed, with `n/a`
/// entries removed: `n/a` declares the list empty, it is not an entry.
fn split_entries(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|entry| entry.trim().to_string())
        .filter(|entry| !entry.is_empty() && !entry.eq_ignore_ascii_case(NA_ENTRY))
        .collect()
}

/// The lines of the first `Invocation read-back:` block: everything after the
/// key line up to the next blank line, trimmed. Reading back from the parsed
/// configuration — not the diff — is what makes a wired-in check verifiable
/// after the fact (#3904).
fn read_back_block(body: &str) -> Vec<String> {
    let mut block: Vec<String> = Vec::new();
    let mut in_block = false;
    for line in body.lines() {
        if !in_block {
            if line.starts_with(READ_BACK_KEY) {
                in_block = true;
            }
        } else if line.trim().is_empty() {
            break;
        } else {
            block.push(line.trim().to_string());
        }
    }
    block
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
