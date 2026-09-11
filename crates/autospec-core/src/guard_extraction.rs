//! Sibling-guard extraction (issue #4289).
//!
//! One system, four implementations, one rule: *"an output patch must not be
//! a stale patch from the previous round; it must be converted against the
//! current tip."* The rule was re-earned as a guard in each implementation
//! instead of being extracted once:
//!
//! | Guard | Where it reached |
//! |---|---|
//! | `timeout -k` for wedged runners | one of 16 runners, not the other 15 — every wedge was a debugging session |
//! | "empty stage = no patch" | `convpass`, not `iwconv` — an empty stage in the second system produced a bogus conversion (a PR with an empty diff and a merge commit) |
//! | "stage the spec" | the autospec side, not the InferWeave side — `git add` silently failed |
//! | "resolve the base ref to a full SHA before using it" | `convpass`, not `iwconv` — extracted at the fourth copy |
//!
//! Cost: three debugging sessions, one bogus conversion, two stranded
//! patches. The extraction cost is a one-time constant — ~20 minutes,
//! whether done at the first copy or the fourth. The copy cost is a
//! debugging session, paid per copy, per system. Deferring extraction to the
//! fourth copy multiplied the copy cost without reducing the extraction cost
//! at all.
//!
//! Invariants:
//!
//! 1. **Extraction is due at the second copy, not the fourth.** The moment
//!    the same guard is written into a second component, stop and extract.
//!    [`extraction_due`] flips at copy 2; [`CopyLedger::add_copy`] reports
//!    [`CopyEvent::ExtractionDue`] at the second and
//!    [`CopyEvent::Overdue`] from the third on.
//! 2. **Deferring extraction multiplies the copy cost, never the extraction
//!    cost.** The extraction cost is a one-time constant; the copy cost is
//!    paid per copy, per system. [`CostModel::deferral_cost`] is
//!    `(copies − 2) × copy_cost` — the extraction cost cancels out of the
//!    deferral delta because it is constant.
//! 3. **Siblings are detectable before they bite.** Two files whose names
//!    differ only in a domain prefix (convpass / iwconv, autospec-issue /
//!    iw-issue) and share more than a small number of distinctive strings
//!    are siblings: a fix in one is due in the other. [`name_candidates`]
//!    is the coarse candidate test (a shared word stem of
//!    [`NAME_STEM_MIN_LEN`] chars); [`sibling_pair`] requires the content
//!    half as well — a name match alone is a candidate, not a finding.
//! 4. **A guard that cites an issue number is expected in every sibling.** A
//!    guard comment citing an issue number, present in only one file of a
//!    sibling family, is a gap, with the issue and the missing sibling(s)
//!    named. [`guard_citations`], [`guard_coverage`]. This is the check that
//!    would have caught the incident: #4170 cited in `convpass`, absent from
//!    `iwconv`.
//! 5. **Grep the tree before writing a one-file fix.** A change touching
//!    exactly one file while the tree names a sibling candidate is a smell;
//!    [`fix_scope`] names the candidate(s) to grep.
//!
//! [`audit`] is the runtime half of this contract; everything here is pure.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// The copy at which extraction becomes due. Not the fourth.
pub const SECOND_COPY: usize = 2;

/// Minimum shared word-stem length (chars, case-insensitive, extension
/// stripped) for two file names to be sibling *candidates*. `convpass` /
/// `iwconv` share `conv` (4); `autospecissue` / `iwissue` share `issue` (5).
pub const NAME_STEM_MIN_LEN: usize = 4;

/// Minimum length of a distinctive token (see [`is_distinctive`]).
pub const DISTINCTIVE_MIN_LEN: usize = 6;

/// Two sibling files must share *more* than this many distinctive strings
/// for [`sibling_pair`] to fire — "more than a small number".
pub const SIBLING_SHARED_THRESHOLD: usize = 5;

/// The copy count at which extraction is due.
///
/// 0 and 1 are the original implementation(s) of a rule that has not yet
/// been duplicated; 2 is the moment to extract; anything past that is
/// deferral.
pub fn extraction_due(copies: usize) -> bool {
    copies >= SECOND_COPY
}

/// The cost of a duplicated guard, in arbitrary units (minutes, sessions).
///
/// `extraction_cost` is a one-time constant: extracting the guard into a
/// shared component takes ~20 minutes whether the second copy has just been
/// written or the fourth. `copy_cost` is paid per copy, per system: the
/// debugging session each independently-maintained copy eventually demands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostModel {
    /// One-time cost of extracting the guard into a shared implementation.
    pub extraction_cost: u64,
    /// Per-copy cost: the debugging session a copy eventually demands.
    pub copy_cost: u64,
}

impl CostModel {
    pub const fn new(extraction_cost: u64, copy_cost: u64) -> Self {
        Self {
            extraction_cost,
            copy_cost,
        }
    }

    /// Total cost of writing `copies` copies and then extracting: the
    /// one-time extraction plus one copy cost per copy.
    pub fn total_cost(&self, copies: usize) -> u64 {
        self.extraction_cost + (copies as u64).saturating_mul(self.copy_cost)
    }

    /// The extra cost paid by deferring extraction to the `copies`-th copy
    /// instead of extracting at [`SECOND_COPY`]: `(copies − 2) × copy_cost`.
    ///
    /// The extraction cost does not appear here — it is constant, so it
    /// cancels out of the deferral delta. This is the arithmetic the
    /// incident got wrong: deferring from copy 2 to copy 4 added two copy
    /// costs and saved nothing.
    pub fn deferral_cost(&self, copies: usize) -> u64 {
        (copies.saturating_sub(SECOND_COPY) as u64).saturating_mul(self.copy_cost)
    }
}

/// What recording a copy of a guard into a new component means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CopyEvent {
    /// First implementation of the rule — the original, not a copy. No
    /// extraction pressure.
    Original,
    /// Second implementation — the moment extraction is due. Stop and
    /// extract.
    ExtractionDue,
    /// Third implementation or later — extraction was already due and was
    /// not done. Each of these is an unpaid deferral.
    Overdue {
        /// Total copies after this one (≥ 3).
        copies: usize,
    },
}

/// A per-guard ledger of the components that carry copies of it.
///
/// The caller persists this (as JSON) between runs, one ledger per
/// duplicated guard, the same way the stored-output `BlockerLedger` is
/// persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CopyLedger {
    components: Vec<String>,
}

impl CopyLedger {
    pub fn new() -> Self {
        Self {
            components: Vec::new(),
        }
    }

    /// Record that `component` now carries a copy of the guard, and report
    /// what that copy means.
    pub fn add_copy(&mut self, component: &str) -> CopyEvent {
        self.components.push(component.to_string());
        match self.components.len() {
            1 => CopyEvent::Original,
            2 => CopyEvent::ExtractionDue,
            n => CopyEvent::Overdue { copies: n },
        }
    }

    /// The components carrying copies, in the order they were recorded.
    pub fn components(&self) -> &[String] {
        &self.components
    }

    pub fn len(&self) -> usize {
        self.components.len()
    }

    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }

    /// Whether extraction is due: at least [`SECOND_COPY`] copies exist.
    pub fn due(&self) -> bool {
        extraction_due(self.components.len())
    }

    /// The extra cost this ledger has paid in deferrals so far.
    pub fn deferral_cost(&self, model: &CostModel) -> u64 {
        model.deferral_cost(self.components.len())
    }

    /// The report line for this ledger: `OK:` at 0–1 copies, `WARN:` from
    /// the second on, naming the deferral cost.
    pub fn line(&self, model: &CostModel) -> String {
        match self.components.len() {
            n @ 0..=1 => format!(
                "OK: guard has {n} implementation(s); no extraction pressure"
            ),
            n => format!(
                "WARN: guard has {n} copies [{}]; extraction was due at copy {second} and {extra} copy(s) were deferred at {each} each (total {total})",
                self.components.join(", "),
                second = SECOND_COPY,
                extra = n - SECOND_COPY,
                each = model.copy_cost,
                total = model.deferral_cost(n)
            ),
        }
    }
}

/// Strip the directory and the trailing extension from a file name.
///
/// `convpass.sh` → `convpass`, `iwconv` → `iwconv`, `lib/iw-issue.sh` →
/// `iw-issue`.
pub fn stem(name: &str) -> &str {
    let base = name.rsplit('/').next().unwrap_or(name);
    match base.rfind('.') {
        Some(i) if i > 0 => &base[..i],
        _ => base,
    }
}

/// Length of the longest common substring of two strings (char-based).
///
/// This is the name half of the sibling heuristic: a shared word stem of
/// [`NAME_STEM_MIN_LEN`] chars or more is a candidate.
pub fn longest_common_substring_len(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    let mut prev = vec![0usize; b.len() + 1];
    let mut best = 0;
    for &ca in &a {
        let mut curr = vec![0usize; b.len() + 1];
        for (j, &cb) in b.iter().enumerate() {
            if ca == cb {
                curr[j + 1] = prev[j] + 1;
                if curr[j + 1] > best {
                    best = curr[j + 1];
                }
            }
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    best
}

/// Whether two file names are sibling *candidates*: their extension-stripped
/// stems share a word stem of at least [`NAME_STEM_MIN_LEN`] chars
/// (case-insensitive).
///
/// `convpass` / `iwconv` share `conv`; `autospecissue` / `iwissue` share
/// `issue`; `conflictresolve` / `convpass` share only `con` (3) and are not
/// candidates. This is a candidate generator, not a finding: a false name
/// candidate is not a false sibling, because [`sibling_pair`] requires the
/// content half as well.
pub fn name_candidates(a: &str, b: &str) -> bool {
    let a: String = stem(a).to_ascii_lowercase();
    let b: String = stem(b).to_ascii_lowercase();
    if a.is_empty() || b.is_empty() {
        return false;
    }
    longest_common_substring_len(&a, &b) >= NAME_STEM_MIN_LEN
}

/// Whether a token is *distinctive*: long enough and carrying at least one
/// of a digit or one of `- _ . / # $ : =`.
///
/// Distinctive tokens are the ones that identify a system: `PATCH_DIR`,
/// `--check`, `rev-parse`, `origin/main`, `out/issue-`. Boilerplate is not
/// distinctive: `git`, `echo`, `pipefail`, `timeout` are short or purely
/// alphabetic, and shebangs are excluded outright (they are shared by every
/// shell script and identify nothing).
pub fn is_distinctive(token: &str) -> bool {
    token.len() >= DISTINCTIVE_MIN_LEN
        && !token.starts_with("#!")
        && token.chars().any(|c| {
            c.is_ascii_digit() || matches!(c, '-' | '_' | '.' | '/' | '#' | '$' | ':' | '=')
        })
}

/// The distinctive tokens of a file's content, sorted and deduplicated.
///
/// Tokens are whitespace-delimited; surrounding quote/bracket punctuation
/// (`" ' ` ( ) , ;`) is trimmed so that `"$STAGE_DIR"` and `'${STAGE_DIR}'`
/// compare as the same string.
pub fn distinctive_tokens(content: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    for raw in content.split_whitespace() {
        let token =
            raw.trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '(' | ')' | ',' | ';'));
        if is_distinctive(token) {
            seen.insert(token.to_string());
        }
    }
    seen.into_iter().collect()
}

/// The distinctive strings shared by two file contents, sorted.
///
/// This is the content half of the sibling heuristic: siblings
/// re-implement the same system and therefore carry the same identifiers,
/// flags and path shapes.
pub fn distinctive_shared(a: &str, b: &str) -> Vec<String> {
    let a = distinctive_tokens(a);
    let b = distinctive_tokens(b);
    let set_b: BTreeSet<&String> = b.iter().collect();
    a.into_iter().filter(|t| set_b.contains(t)).collect()
}

/// A confirmed sibling pair: name candidates that share more than a small
/// number of distinctive strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiblingPair {
    pub a: String,
    pub b: String,
    /// The shared distinctive strings, sorted.
    pub shared: Vec<String>,
}

impl SiblingPair {
    /// The report line: a `WARN:` naming both files and the shared strings
    /// (first five, then `+k more`).
    pub fn line(&self) -> String {
        let shown = self
            .shared
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let extra = self.shared.len().saturating_sub(5);
        let more = if extra > 0 {
            format!(" +{extra} more")
        } else {
            String::new()
        };
        format!(
            "WARN: sibling candidates '{}' and '{}' share {} distinctive strings ({show}{more}); a fix in one is due in the other",
            self.a,
            self.b,
            self.shared.len(),
            show = shown
        )
    }
}

/// Whether two named files with the given contents are confirmed siblings:
/// name candidates **and** sharing more than [`SIBLING_SHARED_THRESHOLD`]
/// distinctive strings.
///
/// Returns [`None`] when either half fails — a name match alone is a
/// candidate, not a finding.
pub fn sibling_pair(
    name_a: &str,
    name_b: &str,
    content_a: &str,
    content_b: &str,
) -> Option<SiblingPair> {
    if !name_candidates(name_a, name_b) {
        return None;
    }
    let shared = distinctive_shared(content_a, content_b);
    if shared.len() <= SIBLING_SHARED_THRESHOLD {
        return None;
    }
    Some(SiblingPair {
        a: name_a.to_string(),
        b: name_b.to_string(),
        shared,
    })
}

/// The issue numbers cited by guard comments in a file's content, sorted
/// and deduplicated.
///
/// A citation is a `#` immediately followed by 3–5 digits (never 6+ — a
/// date or id, not an issue ref), on a line whose first non-whitespace
/// character is `#` (a comment line; shebangs excluded). Line-level comment
/// parsing: a `#NNNN` inside a string literal on a non-comment line is not
/// a citation.
pub fn guard_citations(content: &str) -> Vec<u32> {
    let mut seen = BTreeSet::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with('#') || trimmed.starts_with("#!") {
            continue;
        }
        for issue in issue_refs(trimmed) {
            seen.insert(issue);
        }
    }
    seen.into_iter().collect()
}

/// Issue references in a string: each `#` followed by exactly 3–5 digits
/// (the digit run must not continue past the 5th digit).
fn issue_refs(s: &str) -> Vec<u32> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' {
            let start = i + 1;
            let digits = bytes[start..]
                .iter()
                .take_while(|&&c| c.is_ascii_digit())
                .count();
            if (3..=5).contains(&digits)
                && bytes
                    .get(start + digits)
                    .is_none_or(|&c| !c.is_ascii_digit())
            {
                out.push(s[start..start + digits].parse().expect("digit run parses"));
                i = start + digits;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// A guard that is present in some files of a sibling family but missing
/// from the rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardGap {
    /// The issue number the guard comment cites.
    pub issue: u32,
    /// Files (in the given order) whose guard comments cite the issue.
    pub present_in: Vec<String>,
    /// Files (in the given order) that are missing the guard.
    pub missing_from: Vec<String>,
}

impl GuardGap {
    /// The report line: a `WARN:` naming the issue and every missing
    /// sibling.
    pub fn line(&self) -> String {
        format!(
            "WARN: guard for #{} present in '{}' but missing from '{}'",
            self.issue,
            self.present_in.join("', '"),
            self.missing_from.join("', '")
        )
    }
}

/// Guard coverage over a sibling family.
///
/// `files` is the set of files implementing the same system (the sibling
/// family — use [`sibling_pair`] / [`name_candidates`] to find it). For
/// every issue number cited by a guard comment in at least one file, the
/// files missing that citation form a [`GuardGap`]. A guard cited in every
/// file of the family produces no gap.
pub fn guard_coverage(files: &[(String, String)]) -> Vec<GuardGap> {
    let mut by_issue: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for (name, content) in files {
        for issue in guard_citations(content) {
            by_issue.entry(issue).or_default().push(name.clone());
        }
    }
    let mut gaps = Vec::new();
    for (issue, present) in &by_issue {
        if present.len() < files.len() {
            let missing_from: Vec<String> = files
                .iter()
                .map(|(n, _)| n.clone())
                .filter(|n| !present.contains(n))
                .collect();
            gaps.push(GuardGap {
                issue: *issue,
                present_in: present.clone(),
                missing_from,
            });
        }
    }
    gaps
}

/// What a change touching a set of files implies about its scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FixScope {
    /// The change touches more than one file (or none): the author has
    /// already searched the tree, or the fix is inherently broad. No
    /// single-file-fix smell.
    Broad { files: Vec<String> },
    /// The change touches exactly one file.
    SingleFile {
        file: String,
        /// Files in the tree (sorted, excluding the changed file) whose
        /// names are sibling candidates of the changed file — the files to
        /// grep before the fix is written.
        siblings: Vec<String>,
    },
}

impl FixScope {
    /// The report line: `OK:` when there is no smell, `WARN:` naming the
    /// sibling candidate(s) to grep.
    pub fn line(&self) -> String {
        match self {
            FixScope::Broad { files } if files.is_empty() => {
                "OK: no changed files".to_string()
            }
            FixScope::Broad { files } => format!(
                "OK: change touches {} file(s); no single-file fix smell",
                files.len()
            ),
            FixScope::SingleFile { file, siblings } if siblings.is_empty() => format!(
                "OK: change touches only '{file}'; no sibling candidate in the tree"
            ),
            FixScope::SingleFile { file, siblings } => format!(
                "WARN: change touches only '{file}' but the tree names sibling candidate(s) '{}'; grep the tree before writing a one-file fix",
                siblings.join("', '")
            ),
        }
    }
}

/// The fix-scope verdict for a change.
///
/// `changed` is the set of files the change touches (duplicates are
/// collapsed); `tree` is the full set of file names in the repository (for
/// example `git ls-files`). A change touching exactly one file while the
/// tree names a sibling candidate is the smell: "grep the tree before
/// writing a one-file fix."
pub fn fix_scope(changed: &[String], tree: &[String]) -> FixScope {
    let changed: BTreeSet<&String> = changed.iter().collect();
    if changed.len() != 1 {
        return FixScope::Broad {
            files: changed.into_iter().cloned().collect(),
        };
    }
    let file = changed
        .into_iter()
        .next()
        .expect("exactly one file")
        .clone();
    let siblings: Vec<String> = tree
        .iter()
        .filter(|t| *t != &file && name_candidates(&file, t))
        .cloned()
        .collect();
    FixScope::SingleFile { file, siblings }
}

/// The audit over a sibling family: the ledger verdict, the confirmed
/// sibling pairs, and the guard gaps.
///
/// `files` is the sibling family (names + contents), `ledger` the copies of
/// the guard under review, `model` the cost model. Returns the report lines
/// — `OK:` where the invariants hold, `WARN:` where one of them does not.
pub fn audit(files: &[(String, String)], ledger: &CopyLedger, model: &CostModel) -> Vec<String> {
    let mut lines = vec![ledger.line(model)];
    for i in 0..files.len() {
        for j in (i + 1)..files.len() {
            if let Some(pair) = sibling_pair(&files[i].0, &files[j].0, &files[i].1, &files[j].1) {
                lines.push(pair.line());
            }
        }
    }
    for gap in guard_coverage(files) {
        lines.push(gap.line());
    }
    lines
}
