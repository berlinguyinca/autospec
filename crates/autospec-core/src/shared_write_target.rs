//! One file many agents must write is a queue pretending to be a document
//! (issue #4238).
//!
//! The most conflicted file in the repository was `docs/invariants.md`: 23
//! conflicts. The second was a generated fixture. Neither is hard to change —
//! the table carries no logic and the fixture is emitted by a script — yet they
//! collected more conflicts than any source file, and the diagnosis on the
//! record was "parallel agents plus large PRs touching the same subsystem",
//! which describes every file in the repo and explains none of the ranking.
//!
//! The mechanism is not the subject matter, it is the layout. Every entry lives
//! in one file, so two agents adding two *unrelated* facts edit the same lines
//! and conflict on unrelated work. A file whose rows are appended by every
//! parallel agent is a shared mutable cell: the conflicts are a property of the
//! file, and no amount of care by the agents removes them. The fix moves the
//! serialisation point out of the file — one file per claim, an index nobody
//! edits by hand — and, where a shared target must survive, detects it before
//! dispatch instead of at conversion.
//!
//! Four invariants, each a primitive here:
//!
//! 1. **A claim several agents may add to has one file of its own, and the
//!    index is generated** ([`entry_path`], [`parse_table_entries`],
//!    [`render_entry`], [`render_index`]). Two agents adding two claims then
//!    touch two files; a conflict in the index is regenerated rather than
//!    hand-merged, per [`crate::conflict_resolution`].
//! 2. **Ordering the appends only works when each added entry is
//!    self-contained** ([`decide_append`], [`is_self_contained`]). An entry
//!    that continues another line, or that refers to the prose beside it,
//!    cannot be re-ordered without changing meaning, and the merge that keeps
//!    both lines produces a document that says neither. Refusal is the default.
//! 3. **A ready set whose members share a write target is decided before
//!    dispatch, and each hold names its release** ([`contended_targets`],
//!    [`ReadyPlan`], [`HeldTarget::hold_line`]). [`crate::dispatch_pipeline`]
//!    already places an issue in the first wave its surface is free; what it
//!    does not do is say *why* an issue waits. A hold that names no release is
//!    a deadlock reporting itself as idle (#4170).
//! 4. **Measure where conflicts land before assuming an architectural
//!    cause** ([`ConflictLedger`], [`file_role`], [`diagnose`]). The ranking is
//!    cheap and falsifiable: it said the top two files were a documentation
//!    table and a generated fixture — a file-layout bug, not fan-out colliding
//!    in one subsystem.
//!
//! Nothing here re-implements pairwise surface comparison. Path and directory
//! grammar (an exact file, or a `trailing /` covering everything beneath it)
//! comes from [`IssueWriteSurface`], so a directory claimed by one issue and a
//! file beneath it by another count as shared here exactly as they do at
//! filing.

use std::collections::{BTreeMap, BTreeSet};

use crate::conflict_resolution::{classify_file, FileShape};
use crate::dispatch_pipeline::IssueWriteSurface;

// ── Invariant 1: one file per claim, index generated ─────────────────────

/// One claim of an aggregate document: the row's text, and the issue that
/// decided it when the document records one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateEntry {
    /// The issue number the claim cites (`#4170`), when present.
    pub issue: Option<u64>,
    /// The claim itself, without table markup.
    pub text: String,
}

/// The per-claim path that replaces one row of `aggregate`:
/// `docs/invariants.md` + `4238` → `docs/invariants/4238.md`.
///
/// `None` for a path this module cannot split safely — absolute, containing
/// `..`, or with no file name. The layout change is opt-in per document, never
/// guessed at. A bare `invariants.md` gets sibling entries
/// (`invariants-4238.md`) rather than an invented directory.
pub fn entry_path(aggregate: &str, issue: u64) -> Option<String> {
    if aggregate.is_empty() || aggregate.starts_with('/') || aggregate.contains("..") {
        return None;
    }
    let name = aggregate.rsplit('/').next()?;
    if name.is_empty() || name.starts_with('.') {
        return None;
    }
    let (dir, stem) = match aggregate.rfind('/') {
        None => (String::new(), name.strip_suffix(".md").unwrap_or(name)),
        Some(pos) => (
            aggregate[..pos].to_string(),
            name.strip_suffix(".md").unwrap_or(name),
        ),
    };
    if stem.is_empty() {
        return None;
    }
    Some(if dir.is_empty() {
        format!("{stem}-{issue}.md")
    } else {
        format!("{dir}/{stem}/{issue}.md")
    })
}

/// Split a markdown table row into trimmed cells, dropping the empty fragments
/// the leading and trailing pipes produce.
fn row_cells(line: &str) -> Vec<&str> {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') {
        return Vec::new();
    }
    trimmed
        .split('|')
        .map(str::trim)
        .filter(|cell| !cell.is_empty())
        .collect()
}

/// Whether a row is the `|---|:--:|` separator under a table header.
fn is_separator_row(cells: &[&str]) -> bool {
    !cells.is_empty()
        && cells
            .iter()
            .all(|cell| cell.chars().all(|c| matches!(c, '-' | ':' | ' ')))
}

/// Extract the `#<digits>` issue reference from a row's cells, if any.
fn issue_reference(cells: &[&str]) -> Option<u64> {
    cells.iter().find_map(|cell| {
        let after = cell.split('#').nth(1)?;
        let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    })
}

/// Parse the table rows of an aggregate document into claims.
///
/// The rows *are* the claims; prose sections between tables stay where they
/// are, because they are read whole and are not what agents append to. A header
/// row is recognised as the table row immediately above a separator row and
/// dropped with it; anything that is not a table line is ignored.
pub fn parse_table_entries(doc: &str) -> Vec<AggregateEntry> {
    let lines: Vec<&str> = doc.lines().collect();
    let mut entries = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let cells = row_cells(line);
        if cells.is_empty() || is_separator_row(&cells) {
            continue;
        }
        let next = lines
            .get(index + 1)
            .map(|l| row_cells(l))
            .unwrap_or_default();
        if is_separator_row(&next) {
            continue; // header row
        }
        entries.push(AggregateEntry {
            issue: issue_reference(&cells),
            text: cells[0].to_string(),
        });
    }
    entries
}

/// Render the body of one per-claim file.
pub fn render_entry(entry: &AggregateEntry) -> String {
    let source = match entry.issue {
        Some(issue) => format!("issue #{issue}"),
        None => "an unnumbered claim".to_string(),
    };
    format!("# {}\n\nDecided in {source}.\n", entry.text)
}

/// Render the generated index over `entries`: one row per claim, each linking
/// to its own file under `dir`.
///
/// The first line is the generated-file header that
/// `conflict_resolution::parse_generator_header` recognises, naming `generator`,
/// so a conflict in the index resolves by regeneration: the index is a shadow of
/// the per-claim files, and hand-merging it is the corruption #3663 describes.
/// Ordering is by issue number ascending, then by text for unnumbered claims, so
/// two runs of the generator over the same entries produce identical bytes.
pub fn render_index(entries: &[AggregateEntry], dir: &str, generator: &str) -> String {
    let mut ordered: Vec<&AggregateEntry> = entries.iter().collect();
    ordered.sort_by(|a, b| match (a.issue, b.issue) {
        (Some(x), Some(y)) => x.cmp(&y).then_with(|| a.text.cmp(&b.text)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.text.cmp(&b.text),
    });
    let dir = dir.trim_matches('/');
    let mut out =
        format!("<!-- Generated by {generator} -->\n\n| Invariant | Source |\n|---|---|\n");
    for entry in ordered {
        let source = match entry.issue {
            Some(issue) => format!("[#{issue}]({dir}/{issue}.md)"),
            None => "unnumbered".to_string(),
        };
        out.push_str(&format!("| {} | {source} |\n", entry.text));
    }
    out
}

// ── Invariant 2: union-lines merge only for self-contained entries ───────

/// Why an append to a shared target may not be merged by keeping both sides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppendRefusal {
    /// The append contributed nothing: a bug in the caller, not a merge.
    Empty,
    /// An entry is not self-contained, so its position carries meaning that a
    /// line union destroys.
    NotSelfContained {
        /// The offending line, verbatim.
        line: String,
        /// What made the line depend on its neighbours.
        reason: &'static str,
    },
}

/// How a concurrent append to a shared target may be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppendMerge {
    /// Every entry stands alone: keeping both sides preserves the meaning
    /// whatever the order, so the union is safe.
    UnionLines,
    /// Refuse the union. The layout is the defect ([`entry_path`]).
    SplitFile(AppendRefusal),
}

fn count_char(haystack: &str, needle: char) -> usize {
    haystack.chars().filter(|c| *c == needle).count()
}

/// What makes `line` depend on its neighbours, for the refusal message.
fn continuation_reason(trimmed: &str) -> &'static str {
    if trimmed.is_empty() {
        "blank line"
    } else if trimmed.starts_with('|') {
        "unclosed table row"
    } else if count_char(trimmed, '`') % 2 != 0 {
        "unclosed inline code span"
    } else if ['(', '[', '{'].iter().any(|o| count_char(trimmed, *o) != 0) {
        "unbalanced brackets"
    } else {
        "continues an adjacent line"
    }
}

/// Whether one line carries its own meaning, independent of its neighbours.
///
/// A line fails when it continues another (trailing `,`, `+`, `&&`, `\`, `:`),
/// when backticks or brackets do not close on the line itself, or when a
/// markdown table row is not closed. Those forms make position part of the
/// content, which is exactly what a keep-both merge discards.
pub fn is_self_contained(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.ends_with(',')
        || trimmed.ends_with('+')
        || trimmed.ends_with("&&")
        || trimmed.ends_with('\\')
        || trimmed.ends_with(':')
    {
        return false;
    }
    if count_char(trimmed, '`') % 2 != 0 {
        return false;
    }
    for (open, close) in [('(', ')'), ('[', ']'), ('{', '}')] {
        if count_char(trimmed, open) != count_char(trimmed, close) {
            return false;
        }
    }
    !(trimmed.starts_with('|') && !trimmed.ends_with('|'))
}

/// Decide whether appended lines may be merged by keeping both sides.
///
/// The default is refusal: an empty set is a bug in the caller, and a single
/// context-dependent line makes the whole append unsafe to re-order, because the
/// merge that keeps both lines yields a document that says neither.
pub fn decide_append(lines: &[&str]) -> AppendMerge {
    if lines.is_empty() {
        return AppendMerge::SplitFile(AppendRefusal::Empty);
    }
    for line in lines {
        if is_self_contained(line) {
            continue;
        }
        return AppendMerge::SplitFile(AppendRefusal::NotSelfContained {
            line: (*line).to_string(),
            reason: continuation_reason(line.trim()),
        });
    }
    AppendMerge::UnionLines
}

// ── Invariant 3: shared write targets decided before dispatch ────────────

/// A write target more than one ready issue claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContendedTarget {
    /// The shared surface entry: an exact path, or the directory a descendant
    /// claim collided with ([`IssueWriteSurface::shared_entries`]).
    pub entry: String,
    /// Every issue whose declared surface includes `entry`, ascending.
    pub claimants: Vec<u64>,
}

impl ContendedTarget {
    /// The issue that gets the target when queue order says nothing: the
    /// lowest-numbered claimant, so the fallback does not depend on the order
    /// surfaces were supplied in.
    pub fn lowest_claimant(&self) -> u64 {
        *self
            .claimants
            .first()
            .expect("a contended target always has claimants")
    }
}

/// Group the surfaces of a ready set by shared entry.
///
/// Only entries claimed by at least `min_claimants` issues are returned, and the
/// threshold is clamped to 2: one claimant is not contention. A directory
/// claimed by one issue and a file beneath it by another appear once, keyed by
/// the broader declaration, because that is the entry the collision is about.
pub fn contended_targets(
    surfaces: &[IssueWriteSurface],
    min_claimants: usize,
) -> Vec<ContendedTarget> {
    let threshold = min_claimants.max(2);
    let mut claimants: BTreeMap<String, BTreeSet<u64>> = BTreeMap::new();
    for (index, surface) in surfaces.iter().enumerate() {
        for other in &surfaces[index + 1..] {
            for entry in surface.shared_entries(other) {
                let set = claimants.entry(entry).or_default();
                set.insert(surface.issue);
                set.insert(other.issue);
            }
        }
    }
    claimants
        .into_iter()
        .filter(|(_, set)| set.len() >= threshold)
        .map(|(entry, set)| ContendedTarget {
            entry,
            claimants: set.into_iter().collect(),
        })
        .collect()
}

/// Why a ready issue was not launched this tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldReason {
    /// Another ready issue already holds the write target it would edit.
    SharedTarget {
        /// The shared entry it waits for.
        entry: String,
        /// The issue launched ahead of it on that entry.
        waiting_on: u64,
    },
    /// The parallel budget was spent; no contention involved.
    QueueDepth,
}

/// One ready issue held back from dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldTarget {
    pub issue: u64,
    pub reason: HoldReason,
}

impl HeldTarget {
    /// The hold line, naming the cause and the action that releases it.
    ///
    /// The release for contention is a merge: the hold clears when `waiting_on`
    /// lands and the ready set is re-read, which is what distinguishes it from a
    /// deadlock that reprints the same line every run (#4170).
    pub fn hold_line(&self) -> String {
        match &self.reason {
            HoldReason::SharedTarget { entry, waiting_on } => format!(
                "HELD: #{} waits for #{waiting_on} on {entry} — re-dispatch once #{waiting_on} merges",
                self.issue
            ),
            HoldReason::QueueDepth => format!(
                "HELD: #{} waits for a free dispatch slot — re-dispatch when a running issue lands",
                self.issue
            ),
        }
    }
}

/// The dispatch decision for one ready set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyPlan {
    /// Ready issues launched now, in queue order.
    pub dispatch: Vec<u64>,
    /// Ready issues not launched, each with its reason.
    pub held: Vec<HeldTarget>,
    /// Size of the ready set the plan was computed over.
    pub ready: usize,
}

impl ReadyPlan {
    /// Plan `surfaces` (in queue order) into a launch set and a held set.
    ///
    /// The first ready issue to claim a target keeps it and later claimants wait
    /// for it; an issue is also held once `max_parallel` issues are launched, so
    /// the caller sees both reasons in one report. Queue order is preserved:
    /// this filters the frontier, never reorders it — the ordering is somebody
    /// else's decision.
    pub fn plan(surfaces: &[IssueWriteSurface], max_parallel: usize) -> Self {
        let contention = contended_targets(surfaces, 2);
        let mut dispatch: Vec<u64> = Vec::new();
        let mut held = Vec::new();
        let mut launched: BTreeSet<u64> = BTreeSet::new();
        for surface in surfaces {
            if let Some(blocker) = contention.iter().find_map(|target| {
                if !target.claimants.contains(&surface.issue) {
                    return None;
                }
                target
                    .claimants
                    .iter()
                    .find(|other| **other != surface.issue && launched.contains(other))
                    .copied()
                    .map(|waiting_on| (target.entry.clone(), waiting_on))
            }) {
                held.push(HeldTarget {
                    issue: surface.issue,
                    reason: HoldReason::SharedTarget {
                        entry: blocker.0,
                        waiting_on: blocker.1,
                    },
                });
                continue;
            }
            if dispatch.len() >= max_parallel {
                held.push(HeldTarget {
                    issue: surface.issue,
                    reason: HoldReason::QueueDepth,
                });
                continue;
            }
            launched.insert(surface.issue);
            dispatch.push(surface.issue);
        }
        Self {
            dispatch,
            held,
            ready: surfaces.len(),
        }
    }

    /// The one-line frontier report: ready, dispatchable, held.
    ///
    /// `ready` and `dispatchable` are different counts and are printed
    /// separately, as everywhere the frontier is reported (#4170).
    pub fn line(&self) -> String {
        format!(
            "{} ready ({} dispatchable, {} held)",
            self.ready,
            self.dispatch.len(),
            self.held.len()
        )
    }

    /// Whether the counts add up, for callers that would rather assert it: a
    /// frontier whose numbers do not reconcile reports a state that cannot
    /// exist.
    pub fn reconciles(&self) -> bool {
        self.ready == self.dispatch.len() + self.held.len()
    }

    /// The hold lines, one per held issue.
    pub fn hold_lines(&self) -> Vec<String> {
        self.held.iter().map(HeldTarget::hold_line).collect()
    }
}

// ── Invariant 4: measure before diagnosing ───────────────────────────────

/// A tally of observed conflicts, keyed by path.
///
/// Cheap evidence that outranks a plausible story: the ranking is computed from
/// the conflicts that actually happened, so a story that contradicts it is wrong
/// (the #4167 lesson — read the artefact whole before asserting what it lacks).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConflictLedger {
    counts: BTreeMap<String, usize>,
}

impl ConflictLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one conflict against `path`.
    pub fn observe(&mut self, path: &str) {
        *self.counts.entry(path.to_string()).or_insert(0) += 1;
    }

    /// Conflicts recorded for `path` (0 when never observed).
    pub fn count(&self, path: &str) -> usize {
        self.counts.get(path).copied().unwrap_or(0)
    }

    /// Total conflicts recorded.
    pub fn total(&self) -> usize {
        self.counts.values().sum()
    }

    /// Paths ranked by conflict count, most conflicted first, ties broken by
    /// path so the ranking is stable across runs.
    pub fn ranked(&self) -> Vec<(&str, usize)> {
        let mut ranked: Vec<(&str, usize)> = self
            .counts
            .iter()
            .map(|(path, count)| (path.as_str(), *count))
            .collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        ranked
    }

    /// The most conflicted path and its count.
    pub fn top(&self) -> Option<(&str, usize)> {
        self.ranked().first().copied()
    }

    /// The share of all conflicts held by the top file, as a whole percent
    /// (0 when nothing has been observed).
    pub fn top_share_percent(&self) -> usize {
        let total = self.total();
        match self.top() {
            Some((_, count)) if total > 0 => count * 100 / total,
            _ => 0,
        }
    }

    /// The one-line report: what is most conflicted, what share of the total
    /// that is, and across how many files.
    pub fn line(&self) -> String {
        match self.top() {
            Some((path, count)) => format!(
                "top conflicted: {} {}/{} ({}%) across {} files",
                path,
                count,
                self.total(),
                self.top_share_percent(),
                self.counts.len()
            ),
            None => "no conflicts observed".to_string(),
        }
    }
}

/// What kind of file a conflict landed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRole {
    /// A documentation table, an append-only list, a generated artefact, or a
    /// single-value golden: the file accumulates claims and carries no logic.
    AggregationPoint,
    /// Source code: a conflict here says the work itself overlaps.
    Implementation,
    /// Neither claim is supported. Additive declaration files land here on
    /// purpose: their conflicts merge by deduplication already, so they are
    /// evidence of neither a layout defect nor of overlapping work.
    Unclassified,
}

/// Extensions of files whose whole job is to hold rows.
const AGGREGATE_EXTENSIONS: [&str; 7] = ["md", "markdown", "txt", "tsv", "csv", "adoc", "lock"];

/// Extensions of source files.
const SOURCE_EXTENSIONS: [&str; 12] = [
    "rs", "py", "go", "ts", "tsx", "js", "jsx", "java", "scala", "kt", "rb", "sh",
];

/// Lowercase extension of `path`, or the empty string when it has none.
fn extension(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rfind('.') {
        Some(pos) if pos > 0 => name[pos + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// Classify a conflicted path as an aggregation point or as implementation.
///
/// The shape check from [`crate::conflict_resolution`] runs first: a generated
/// header, an append-only list, or a single-value golden is an aggregation point
/// whatever its name says. Extensions only break ties, and an unrecognised file
/// stays [`FileRole::Unclassified`] rather than being assumed into a diagnosis.
pub fn file_role(path: &str, content: &str) -> FileRole {
    match classify_file(path, content) {
        FileShape::Generated { .. } | FileShape::AppendOnlyList | FileShape::SingleValue => {
            FileRole::AggregationPoint
        }
        FileShape::AdditiveDeclarations => FileRole::Unclassified,
        FileShape::Unknown => {
            let ext = extension(path);
            if AGGREGATE_EXTENSIONS.contains(&ext.as_str()) {
                FileRole::AggregationPoint
            } else if SOURCE_EXTENSIONS.contains(&ext.as_str()) {
                FileRole::Implementation
            } else {
                FileRole::Unclassified
            }
        }
    }
}

/// What the conflict evidence says to fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentionDiagnosis {
    /// The file layout is the defect: one file per claim and a generated index
    /// ([`entry_path`], [`render_index`]).
    FileLayout,
    /// Real work overlaps in source: narrow the scope each issue declares.
    CodeComplexity,
    /// Not enough evidence to attribute the conflicts.
    InsufficientData,
}

/// Attribute the conflicts observed on one file.
///
/// Zero conflicts is [`ContentionDiagnosis::InsufficientData`], not
/// [`ContentionDiagnosis::FileLayout`]: a file nobody fought over carries no
/// evidence, and "no evidence" is the answer the ledger exists to make visible
/// instead of filling in with a theory.
pub fn diagnose(path: &str, content: &str, conflicts: usize) -> ContentionDiagnosis {
    if conflicts == 0 {
        return ContentionDiagnosis::InsufficientData;
    }
    match file_role(path, content) {
        FileRole::AggregationPoint => ContentionDiagnosis::FileLayout,
        FileRole::Implementation => ContentionDiagnosis::CodeComplexity,
        FileRole::Unclassified => ContentionDiagnosis::InsufficientData,
    }
}
