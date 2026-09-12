//! A patch's own trunk invalidates the patch (issue #4151).
//!
//! 53 of 141 held patches (38%) die on merge conflicts — the single largest
//! cause of held work. The three other explanations were ruled out first:
//!
//! | hypothesis | verdict |
//! |---|---|
//! | the patches are stale | disproven — conflict-held patches are *newer* (median 0.7 d) than the ones that converted cleanly (1.4 d) |
//! | `git apply --3way` has no usable ancestor | mostly disproven — the base blobs are present in the object store |
//! | pure module-declaration manifests serialise every feature | real, but a minority — 7 of 53 (13%) conflict exclusively on manifest files |
//! | the patches collide with work that merged while they were in flight | **confirmed, at 100%** — all 90 conflicting-file mentions were in files `main` changed in the same 24 h window |
//!
//! Not one conflict was in a file `main` had left alone. What this is:
//! **the fleet's own merge throughput invalidates the fleet's in-flight
//! patches.** 22 agents produce patches against a base; ~160 commits/day
//! land; every patch touching a hot file has a narrow window to convert
//! before its context is rewritten. Merging faster does not drain the
//! backlog — past a point it *creates* held work.
//!
//! Five invariants, each a primitive here:
//!
//! 1. **Record the base commit with the patch** ([`InFlightPatch::new`]).
//!    The durable fix — the sidecar that records `base_sha` next to the
//!    patch — already exists as [`crate::rebaseline::PatchMeta`]
//!    (issue #3708). What is missing is the converter-side record, which
//!    refuses a patch that names no base rather than ordering it
//!    anyway: without the base nothing downstream can compute how far
//!    `main` has moved since the patch was written.
//! 2. **A hot file is a contended resource at dispatch time**
//!    ([`dispatch_batches`]). Concurrently assigning several issues that
//!    all declare the same surface guarantees that at most one converts;
//!    issues whose declared paths overlap are serialised, not fanned out.
//! 3. **Convert in contention order, not arrival order**
//!    ([`contention_order`]). A patch touching a file with high churn
//!    since its base converts first, while its context still exists.
//! 4. **A pure declaration manifest is not a merge point**
//!    ([`declaration_manifest_findings`]). For a file whose every
//!    non-comment line is an independent `mod`/`use` declaration, a union
//!    merge is correct — both sides' declarations must exist — and the
//!    content check is the guard that keeps union safe.
//!    [`crate::conflict_resolution`] supplies the strategy
//!    (`KeepBothDeduplicated`); this module supplies the precondition, and
//!    it reads the *content*, not the path, so a `fn` or an inline
//!    `mod x { }` body in a file the path calls `lib.rs` refuses union.
//! 5. **Report the conflicting file's recent churn in the HELD line**
//!    ([`held_conflicts_line`]). `HELD: conflicts in X` invites the reader
//!    to study the patch; `HELD: conflicts in X (main changed X 25 times
//!    in 24h)` names the real cause and stops a person debugging a patch
//!    that was simply overtaken. [`overlap_report`] is the measurement
//!    behind the line: every conflict is attributed to churn before it is
//!    read as a patch defect.
//!
//! Everything here is pure: no I/O, no clock, no subprocess. The caller
//! measures the trunk's own history (the commits since each patch's base
//! and the files each touched) and supplies it.

use std::collections::{BTreeMap, BTreeSet};

/// One in-flight patch, as the conversion pass sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InFlightPatch {
    /// The issue the patch was produced for.
    pub issue: u32,
    /// The commit the patch was written against, recorded at write time
    /// (invariant 1). The sidecar that records it durably is
    /// [`crate::rebaseline::PatchMeta`]; this field is what the ordering
    /// and prioritisation here are computed from.
    pub base_sha: String,
    /// The files the patch touches.
    pub files: Vec<String>,
}

impl InFlightPatch {
    /// Builds the record and enforces invariant 1: a patch with no
    /// recorded base is refused, never defaulted. Without the base
    /// nothing downstream can compute how far `main` has moved since the
    /// patch was written, so the patch cannot be ordered, prioritised, or
    /// rebased — and ordering it anyway is exactly what this module exists
    /// to stop.
    pub fn new(
        issue: u32,
        base_sha: &str,
        files: impl IntoIterator<Item = impl Into<String>>,
    ) -> Option<Self> {
        let base = base_sha.trim();
        if base.is_empty() {
            return None;
        }
        Some(Self {
            issue,
            base_sha: base.to_string(),
            files: files.into_iter().map(Into::into).collect(),
        })
    }
}

/// One commit to `main` since a patch's base, with the files it touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrunkCommit {
    /// The commit id.
    pub sha: String,
    /// The files the commit touched.
    pub files: Vec<String>,
}

/// The commits to `main` since a patch's base — the measurement invariants
/// 2 through 5 are computed from. The caller supplies it from the trunk's
/// own history; the module never guesses it, and an empty movement is not
/// "the trunk did not move", it is "the movement was not measured".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrunkMovement {
    /// The commits in the window, in trunk order.
    pub commits: Vec<TrunkCommit>,
}

impl TrunkMovement {
    /// How many commits in the movement touched `file`.
    pub fn commits_touching(&self, file: &str) -> u32 {
        self.commits
            .iter()
            .filter(|c| c.files.iter().any(|f| f == file))
            .count() as u32
    }

    /// How many commits in the movement touched at least one of `files` —
    /// the union, not a sum: a commit that touched two of the patch's
    /// files invalidates the patch once, not twice.
    pub fn commits_touching_any(&self, files: &[String]) -> u32 {
        self.commits
            .iter()
            .filter(|c| c.files.iter().any(|f| files.iter().any(|p| p == f)))
            .count() as u32
    }

    /// The file with the most churn among `files` — the one whose
    /// rewriting the patch is racing. Ties break lexicographically so the
    /// answer is deterministic. `None` when nothing in `files` has churn:
    /// no file is driving the contention, and naming one would be a guess.
    pub fn hottest<'a>(&self, files: &'a [String]) -> Option<&'a str> {
        let mut best: Option<(&str, u32)> = None;
        for file in files {
            let count = self.commits_touching(file);
            if count == 0 {
                continue;
            }
            match best {
                Some((_, n)) if count < n => {}
                Some((name, n)) if count == n && name < file.as_str() => {}
                _ => best = Some((file.as_str(), count)),
            }
        }
        best.map(|(f, _)| f)
    }
}

/// One slot in the contention order (invariant 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentionSlot {
    /// The patch's position in the arrival (input) list.
    pub index: usize,
    /// The issue the patch was produced for.
    pub issue: u32,
    /// Commits to `main` since the patch's base that touched one of the
    /// patch's files: the patch's context has been rewritten this many
    /// times, and the window to convert before it is invalidated narrows
    /// with each.
    pub contention: u32,
    /// The file that drives the contention, when one has churn.
    pub hot_file: Option<String>,
}

/// The conversion order for one pass: highest contention first, so a patch
/// whose context is being rewritten converts while it still exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentionPlan {
    /// The patches in conversion order.
    pub slots: Vec<ContentionSlot>,
}

impl ContentionPlan {
    /// The order as a line: `contention order: #4198 (25 commits to main
    /// since base, crates/autospec-core/src/lib.rs); #4203 (0 commits to
    /// main since base)`. Every entry names its driver, so the ordering is
    /// falsifiable — a reader can check the churn the order claims.
    pub fn line(&self) -> String {
        let mut parts = Vec::with_capacity(self.slots.len());
        for slot in &self.slots {
            match &slot.hot_file {
                Some(file) => parts.push(format!(
                    "#{} ({} commits to main since base, {})",
                    slot.issue, slot.contention, file
                )),
                None => parts.push(format!(
                    "#{} ({} commits to main since base)",
                    slot.issue, slot.contention
                )),
            }
        }
        format!("contention order: {}", parts.join("; "))
    }
}

/// Convert in contention order, not arrival order (invariant 3).
///
/// The key is the number of commits to `main` since the patch's base that
/// touched one of the patch's files — the union over the patch's files, so
/// a commit that touched two of them counts once. Highest contention
/// first; ties keep arrival order, so the pass is deterministic. A patch
/// whose files `main` never touched since its base converts last: its
/// context still exists and it can wait.
pub fn contention_order(patches: &[InFlightPatch], trunk: &TrunkMovement) -> ContentionPlan {
    let mut slots: Vec<ContentionSlot> = patches
        .iter()
        .enumerate()
        .map(|(index, patch)| ContentionSlot {
            index,
            issue: patch.issue,
            contention: trunk.commits_touching_any(&patch.files),
            hot_file: trunk.hottest(&patch.files).map(str::to_string),
        })
        .collect();
    slots.sort_by_key(|slot| std::cmp::Reverse(slot.contention));
    // `sort_by_key` is stable, so equal contention keeps arrival order.
    ContentionPlan { slots }
}

/// An issue as the dispatcher sees it: its number and the files it is
/// likely to touch (its declared surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredIssue {
    /// The issue number.
    pub issue: u32,
    /// The files the issue is likely to touch.
    pub paths: Vec<String>,
}

/// A set of issues that must be serialised because their declared paths
/// overlap: they are all racing the same surface, and at most one of them
/// converts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchBatch {
    /// The issues in the batch, in input order.
    pub issues: Vec<u32>,
    /// The files at least two members of the batch declare — the shared
    /// surface the serialisation protects. Empty when the batch is a
    /// singleton.
    pub shared_paths: Vec<String>,
}

/// The dispatch plan: which issues run concurrently and which are
/// serialised (invariant 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchPlan {
    /// The batches. Batches run concurrently; issues inside a batch run
    /// one at a time.
    pub batches: Vec<DispatchBatch>,
}

impl DispatchPlan {
    /// The plan as a line: `dispatch: 2 serialized (shared surface:
    /// crates/autospec-core/src/execution/mod.rs), 1 fanned out of 3`.
    /// A plan with no overlap says so plainly, so a serialised fleet is
    /// not read as a fanned-out one.
    pub fn line(&self) -> String {
        let total = self.batches.iter().map(|b| b.issues.len()).sum::<usize>();
        let non_singleton: Vec<&DispatchBatch> =
            self.batches.iter().filter(|b| b.issues.len() > 1).collect();
        if non_singleton.is_empty() {
            return format!("dispatch: {total} fanned out (no overlapping declared paths)");
        }
        let serialized: usize = non_singleton.iter().map(|b| b.issues.len()).sum();
        let fanned = total - serialized;
        let mut shared: BTreeSet<&str> = BTreeSet::new();
        for batch in &non_singleton {
            for path in &batch.shared_paths {
                shared.insert(path.as_str());
            }
        }
        format!(
            "dispatch: {serialized} serialized (shared surface: {}), {fanned} fanned out of {total}",
            shared.iter().copied().collect::<Vec<_>>().join(", ")
        )
    }
}

/// Serialise the issues whose declared paths overlap; fan out the rest
/// (invariant 2).
///
/// The batches are the connected components of the overlap graph: A and B
/// share one file and B and C share another, so A, B and C are one batch
/// even though A and C share nothing — the contention propagates through
/// B. A batch of one has no overlap and may run alongside every other
/// batch.
pub fn dispatch_batches(issues: &[DeclaredIssue]) -> DispatchPlan {
    // path -> the indices of the issues that declare it.
    let mut by_path: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, issue) in issues.iter().enumerate() {
        for path in &issue.paths {
            by_path.entry(path.as_str()).or_default().push(index);
        }
    }

    // Union-find over the indices: issues that share a path are linked.
    let mut parent: Vec<usize> = (0..issues.len()).collect();
    for indices in by_path.values() {
        if indices.len() > 1 {
            let first = union_find_root(&mut parent, indices[0]);
            for &index in &indices[1..] {
                let root = union_find_root(&mut parent, index);
                parent[root] = first;
            }
        }
    }

    let mut components: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for index in 0..issues.len() {
        components
            .entry(union_find_root(&mut parent, index))
            .or_default()
            .push(index);
    }

    let mut batches = components
        .into_values()
        .map(|indices| {
            // The shared surface: the paths at least two members declare.
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
            for &index in &indices {
                for path in &issues[index].paths {
                    *counts.entry(path.as_str()).or_insert(0) += 1;
                }
            }
            let shared_paths = counts
                .into_iter()
                .filter(|(_, n)| *n > 1)
                .map(|(path, _)| path.to_string())
                .collect();
            DispatchBatch {
                issues: indices.iter().map(|&i| issues[i].issue).collect(),
                shared_paths,
            }
        })
        .collect::<Vec<_>>();
    batches.sort_by_key(|batch| batch.issues[0]);
    DispatchPlan { batches }
}

/// The root of `x` in a union-find forest, with path compression.
fn union_find_root(parent: &mut [usize], mut x: usize) -> usize {
    let mut root = x;
    while parent[root] != root {
        root = parent[root];
    }
    while parent[x] != root {
        let next = parent[x];
        parent[x] = root;
        x = next;
    }
    root
}

/// The lines of `content` that are not a `mod`/`use` declaration, a
/// comment, or blank (invariant 4).
///
/// Empty when the file is a **pure declaration manifest**: the shape for
/// which a union merge is correct, because both sides' declarations must
/// exist and the union is the set. This is the guard union needs —
/// [`crate::conflict_resolution`] keeps both sides of a `mod.rs`/`lib.rs`
/// conflict because the *path* says it is a manifest, and a crate whose
/// `lib.rs` grows a `fn` or an inline `mod x { }` body would silently
/// receive a union of things that are not a set. Reading the *content*
/// keeps union safe for any file shape and refuses it the moment a
/// declaration stops being independent.
///
/// Grammar, deliberately narrow (issue #4151):
///
/// - a blank line;
/// - a `//` comment line (including `///` and `//!`);
/// - `mod ident;`, `pub mod ident;`, `pub(crate) mod ident;` — an
///   *independent* module declaration: a `;`, never a `{ }` body;
/// - `use …;`, `pub use …;` — a use tree (`{`, `}`, `*`, `as` allowed).
///
/// Nothing else qualifies — an attribute line, a `fn`, a `const`, an
/// inline module body. The refusal is the safe direction: a manifest that
/// is not pure is resolved by a human, and the finding names the line
/// that makes it so.
pub fn declaration_manifest_findings(content: &str) -> Vec<String> {
    let mut findings = Vec::new();
    for (n, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if !is_declaration(trimmed) {
            findings.push(format!(
                "line {}: not a declaration, comment, or blank: {trimmed}",
                n + 1
            ));
        }
    }
    findings
}

/// Whether `content` is a pure declaration manifest: nothing but
/// `mod`/`use` declarations, comments, and blanks.
pub fn is_declaration_manifest(content: &str) -> bool {
    declaration_manifest_findings(content).is_empty()
}

fn strip_visibility(line: &str) -> Option<&str> {
    if let Some(rest) = line.strip_prefix("pub ") {
        return Some(rest);
    }
    let rest = line.strip_prefix("pub(")?;
    let close = rest.find(')')?;
    let scope = &rest[..close];
    if scope.contains('(') {
        return None;
    }
    if !(scope == "crate" || scope == "super" || scope == "self" || scope.starts_with("in ")) {
        return None;
    }
    Some(rest[close + 1..].trim_start())
}

fn is_declaration(line: &str) -> bool {
    // `None` is "no visibility prefix" (or a malformed one, which then
    // fails the `mod`/`use` check below): the line is the body itself.
    let body = match strip_visibility(line) {
        Some(body) => body,
        None => line,
    };
    if let Some(rest) = body.strip_prefix("mod ") {
        match rest.strip_suffix(';') {
            Some(ident) => is_ident(ident),
            None => false,
        }
    } else if let Some(rest) = body.strip_prefix("use ") {
        is_use_tree(rest)
    } else {
        false
    }
}

fn is_ident(ident: &str) -> bool {
    let mut chars = ident.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn is_use_tree(tree: &str) -> bool {
    let body = match tree.strip_suffix(';') {
        Some(body) if !body.is_empty() => body,
        _ => return false,
    };
    body
        .bytes()
        .all(|b| matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b':' | b'{' | b'}' | b',' | b'*' | b' '))
}

/// The HELD line for a conflict hold, with the conflicting file's recent
/// churn named (invariant 5).
///
/// `HELD: conflicts in X` invites the reader to study the patch;
/// `HELD: conflicts in X (main changed X 25 times in 24h)` names the real
/// cause — the patch was overtaken — and stops a person debugging a patch
/// that was simply rewritten around. `churn_in_window` is the number of
/// commits to `main` touching `file` in the window, measured by the
/// caller. `None` is not `0`: a missing measurement is named as such,
/// never rendered as "main did not touch it".
pub fn held_conflicts_line(file: &str, churn_in_window: Option<u32>, window: &str) -> String {
    match churn_in_window {
        None => format!("HELD: conflicts in {file} (churn not recorded)"),
        Some(0) => format!(
            "HELD: conflicts in {file} (main did not touch it in {window}; the patch is the suspect)"
        ),
        Some(n) => format!(
            "HELD: conflicts in {file} (main changed {file} {n} times in {window})"
        ),
    }
}

/// Where the conflicts were, against what `main` changed in the window —
/// the measurement that confirmed hypothesis 4 at 100%.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlapReport {
    /// Conflicting-file mentions: one per patch-file conflict, so the same
    /// file appears once per patch that conflicted in it.
    pub mentions: usize,
    /// Mentions in a file `main` changed in the window: the pattern.
    pub attributed: usize,
    /// Mentions in a file `main` did not touch: the out-of-pattern case.
    /// A conflict the churn does not explain is a patch defect, not
    /// overtaking — and it is named, never counted into silence.
    pub unexplained: Vec<String>,
}

/// Attribute every conflict mention to the window's churn
/// ([`OverlapReport`]).
pub fn overlap_report(
    conflicting_mentions: &[String],
    main_touched_files: &[String],
) -> OverlapReport {
    let touched: BTreeSet<&str> = main_touched_files.iter().map(String::as_str).collect();
    let mut attributed = 0usize;
    let mut unexplained = Vec::new();
    for mention in conflicting_mentions {
        if touched.contains(mention.as_str()) {
            attributed += 1;
        } else if !unexplained.contains(mention) {
            unexplained.push(mention.clone());
        }
    }
    OverlapReport {
        mentions: conflicting_mentions.len(),
        attributed,
        unexplained,
    }
}

impl OverlapReport {
    /// The attribution as a line: `90 of 90 conflicting-file mentions in
    /// files main changed (100%)` — and when the pattern has a hole, the
    /// files that form it: `… (97%); unexplained: a.rs, b.rs`. An empty
    /// report is not "100% of nothing": it says there was nothing to
    /// attribute.
    pub fn line(&self) -> String {
        if self.mentions == 0 {
            return "no conflicting-file mentions".to_string();
        }
        let pct = self.attributed * 100 / self.mentions;
        let mut line = format!(
            "{} of {} conflicting-file mentions in files main changed ({pct}%)",
            self.attributed, self.mentions
        );
        if !self.unexplained.is_empty() {
            line.push_str(&format!("; unexplained: {}", self.unexplained.join(", ")));
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIB: &str = "crates/autospec-core/src/lib.rs";

    /// A trunk of 4 commits: one touches `a.rs`, one touches `a.rs` and
    /// `b.rs`, one touches `c.rs`, one touches `b.rs`.
    fn trunk() -> TrunkMovement {
        TrunkMovement {
            commits: vec![
                TrunkCommit {
                    sha: "c1".to_string(),
                    files: vec!["a.rs".to_string()],
                },
                TrunkCommit {
                    sha: "c2".to_string(),
                    files: vec!["a.rs".to_string(), "b.rs".to_string()],
                },
                TrunkCommit {
                    sha: "c3".to_string(),
                    files: vec!["c.rs".to_string()],
                },
                TrunkCommit {
                    sha: "c4".to_string(),
                    files: vec!["b.rs".to_string()],
                },
            ],
        }
    }

    #[test]
    fn a_patch_without_a_recorded_base_is_refused() {
        assert_eq!(InFlightPatch::new(1, "", vec!["a.rs".to_string()]), None);
        assert_eq!(InFlightPatch::new(1, "   ", vec!["a.rs".to_string()]), None);
        assert_eq!(
            InFlightPatch::new(1, "a1b2", vec!["a.rs".to_string()]),
            Some(InFlightPatch {
                issue: 1,
                base_sha: "a1b2".to_string(),
                files: vec!["a.rs".to_string()],
            })
        );
    }

    #[test]
    fn a_commit_touching_two_files_of_a_patch_counts_once() {
        let trunk = trunk();
        // c1 and c2 touch a.rs; c2 and c4 touch b.rs: the union over
        // {a.rs, b.rs} is 3 commits, not 2 + 2.
        assert_eq!(
            trunk.commits_touching_any(&["a.rs".to_string(), "b.rs".to_string()]),
            3
        );
        assert_eq!(trunk.commits_touching("a.rs"), 2);
        assert_eq!(trunk.commits_touching("b.rs"), 2);
    }

    #[test]
    fn the_hottest_file_breaks_ties_lexicographically() {
        let trunk = trunk();
        assert_eq!(
            trunk.hottest(&["b.rs".to_string(), "a.rs".to_string()]),
            Some("a.rs")
        );
        assert_eq!(trunk.hottest(&["c.rs".to_string()]), Some("c.rs"));
        assert_eq!(trunk.hottest(&["d.rs".to_string()]), None);
        assert_eq!(trunk.hottest(&[]), None);
    }

    #[test]
    fn the_order_is_contention_first_and_ties_keep_arrival() {
        let trunk = trunk();
        let patches = vec![
            InFlightPatch::new(11, "base", vec!["c.rs".to_string()]).unwrap(), // 1
            InFlightPatch::new(12, "base", vec!["a.rs".to_string()]).unwrap(), // 2
            InFlightPatch::new(13, "base", vec!["b.rs".to_string()]).unwrap(), // 2
            InFlightPatch::new(14, "base", vec!["d.rs".to_string()]).unwrap(), // 0
        ];
        let plan = contention_order(&patches, &trunk);
        assert_eq!(
            plan.slots.iter().map(|s| s.issue).collect::<Vec<_>>(),
            vec![12, 13, 11, 14]
        );
        assert_eq!(
            plan.slots.iter().map(|s| s.contention).collect::<Vec<_>>(),
            vec![2, 2, 1, 0]
        );
        assert_eq!(plan.slots[0].hot_file.as_deref(), Some("a.rs"));
        assert_eq!(plan.slots[3].hot_file, None);
    }

    #[test]
    fn the_contention_line_names_every_driver() {
        let trunk = trunk();
        let patches = vec![
            InFlightPatch::new(12, "base", vec!["a.rs".to_string()]).unwrap(),
            InFlightPatch::new(14, "base", vec!["d.rs".to_string()]).unwrap(),
        ];
        let line = contention_order(&patches, &trunk).line();
        assert_eq!(
            line,
            "contention order: #12 (2 commits to main since base, a.rs); #14 (0 commits to main since base)"
        );
    }

    #[test]
    fn overlapping_issues_are_one_batch_and_the_shared_surface_is_named() {
        let issues = vec![
            DeclaredIssue {
                issue: 1,
                paths: vec!["a.rs".to_string()],
            },
            DeclaredIssue {
                issue: 2,
                paths: vec!["a.rs".to_string(), "b.rs".to_string()],
            },
            DeclaredIssue {
                issue: 3,
                paths: vec!["c.rs".to_string()],
            },
        ];
        let plan = dispatch_batches(&issues);
        assert_eq!(
            plan.batches,
            vec![
                DispatchBatch {
                    issues: vec![1, 2],
                    shared_paths: vec!["a.rs".to_string()],
                },
                DispatchBatch {
                    issues: vec![3],
                    shared_paths: vec![]
                },
            ]
        );
        assert_eq!(
            plan.line(),
            "dispatch: 2 serialized (shared surface: a.rs), 1 fanned out of 3"
        );
    }

    #[test]
    fn contention_propagates_through_a_shared_issue() {
        // 1 and 2 share a.rs; 2 and 3 share b.rs: one batch, and both
        // shared files are named.
        let issues = vec![
            DeclaredIssue {
                issue: 1,
                paths: vec!["a.rs".to_string()],
            },
            DeclaredIssue {
                issue: 2,
                paths: vec!["a.rs".to_string(), "b.rs".to_string()],
            },
            DeclaredIssue {
                issue: 3,
                paths: vec!["b.rs".to_string()],
            },
        ];
        let plan = dispatch_batches(&issues);
        assert_eq!(plan.batches.len(), 1);
        assert_eq!(plan.batches[0].issues, vec![1, 2, 3]);
        assert_eq!(
            plan.batches[0].shared_paths,
            vec!["a.rs".to_string(), "b.rs".to_string()]
        );
    }

    #[test]
    fn disjoint_issues_all_fan_out() {
        let issues = vec![
            DeclaredIssue {
                issue: 1,
                paths: vec!["a.rs".to_string()],
            },
            DeclaredIssue {
                issue: 2,
                paths: vec!["b.rs".to_string()],
            },
        ];
        let plan = dispatch_batches(&issues);
        assert_eq!(
            plan.line(),
            "dispatch: 2 fanned out (no overlapping declared paths)"
        );
    }

    #[test]
    fn a_pure_manifest_has_no_findings() {
        let content = "// top-level modules\n\npub mod evaluation;\npub(crate) mod execution;\nmod hidden;\n\n/// A documented one.\npub use evaluation::score;\nuse crate::evaluation::{Grade, Score};\n";
        assert!(
            is_declaration_manifest(content),
            "{:?}",
            declaration_manifest_findings(content)
        );
        assert!(declaration_manifest_findings(content).is_empty());
    }

    #[test]
    fn a_manifest_with_a_body_is_not_pure() {
        let content = "pub mod evaluation;\nfn helper() {}\nmod inline { pub fn f() {}\n}\nconst N: u32 = 1;\n#[cfg(test)] mod tests;\n";
        let findings = declaration_manifest_findings(content);
        // Lines 2-6: the fn, the inline body's opening line, its closing
        // brace, the const, and the attribute line.
        assert_eq!(findings.len(), 5, "{findings:?}");
        assert!(findings[0].starts_with("line 2:"), "{:?}", findings[0]);
        assert!(findings[0].contains("fn helper"), "{:?}", findings[0]);
        // The inline body's `pub fn f()` is a finding too: a `fn` inside
        // the body is not a declaration.
        assert!(
            findings.iter().any(|f| f.contains("pub fn f")),
            "{findings:?}"
        );
    }

    #[test]
    fn an_attribute_line_is_not_a_declaration() {
        let content = "#![allow(unused)]\npub mod evaluation;\n";
        let findings = declaration_manifest_findings(content);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(
            findings[0].contains("#![allow(unused)]"),
            "{:?}",
            findings[0]
        );
    }

    #[test]
    fn the_held_line_names_the_churn() {
        assert_eq!(
            held_conflicts_line(LIB, Some(25), "24h"),
            "HELD: conflicts in crates/autospec-core/src/lib.rs (main changed crates/autospec-core/src/lib.rs 25 times in 24h)"
        );
    }

    #[test]
    fn zero_churn_and_missing_churn_are_distinct() {
        assert_eq!(
            held_conflicts_line(LIB, Some(0), "24h"),
            "HELD: conflicts in crates/autospec-core/src/lib.rs (main did not touch it in 24h; the patch is the suspect)"
        );
        assert_eq!(
            held_conflicts_line(LIB, None, "24h"),
            "HELD: conflicts in crates/autospec-core/src/lib.rs (churn not recorded)"
        );
    }

    #[test]
    fn the_report_counts_mentions_not_files() {
        // Two patches both conflicted in a.rs: two mentions, one file.
        let mentions = vec!["a.rs".to_string(), "a.rs".to_string(), "b.rs".to_string()];
        let report = overlap_report(&mentions, &["a.rs".to_string(), "b.rs".to_string()]);
        assert_eq!(report.mentions, 3);
        assert_eq!(report.attributed, 3);
        assert!(report.unexplained.is_empty());
        assert_eq!(
            report.line(),
            "3 of 3 conflicting-file mentions in files main changed (100%)"
        );
    }

    #[test]
    fn an_unexplained_conflict_is_named() {
        let mentions = vec!["a.rs".to_string(), "quiet.rs".to_string()];
        let report = overlap_report(&mentions, &["a.rs".to_string()]);
        assert_eq!(report.attributed, 1);
        assert_eq!(report.unexplained, vec!["quiet.rs".to_string()]);
        assert_eq!(
            report.line(),
            "1 of 2 conflicting-file mentions in files main changed (50%); unexplained: quiet.rs"
        );
    }

    #[test]
    fn an_empty_report_is_not_a_pattern() {
        let report = overlap_report(&[], &["a.rs".to_string()]);
        assert_eq!(report.line(), "no conflicting-file mentions");
    }
}
