//! Gate promotion requires a passing run (#3863).
//!
//! A dependency inside a CI workflow is a gate, and adding one is a decision
//! with consequences nobody in the diff can see. From the moment the edge
//! exists, the depending job stops whenever the job it now waits on fails. In
//! InferWeave #231 that edge was added four times and removed three times, and
//! every one of the seven changes was argued from first principles rather than
//! settled by looking: the job the patch promoted had never run successfully on
//! `main`, so the patch would have made every downstream job red on a gate that
//! could not pass.
//!
//! Two invariants, both primitives here:
//!
//! 1. **A job may be made blocking only if it has passed and the patch says
//!    which pass** ([`review_gate_promotions`]). The job must have completed
//!    successfully on at least one real run of the target branch, *and* the
//!    patch must cite that run ([`SuccessfulRun`], [`GateEvidence::citations`]).
//!    Both halves are required: an unobserved job is not a gate, and an
//!    observed-but-uncited pass is not reviewable evidence.
//! 2. **A dependency that cannot be attributed is held, not waved through**
//!    ([`HoldKind::JobUnresolved`], [`HoldKind::DependencyUnresolved`]). The
//!    risk is asymmetric: a false hold costs one line of evidence, a false pass
//!    costs every downstream job on every run afterwards.
//!
//! A patch that *removes* a dependency is never held ([`GatePromotionReview::
//! releases`]): loosening a gate cannot turn a green branch red. An agent that
//! cannot observe a run cannot promote the gate either; it can propose the
//! promotion in the PR body and leave the change out.
//!
//! Two things are deliberately out of scope. The *reverted-decision* half -- a
//! patch removing an edge whose promotion was itself never evidenced, which
//! needs the history of the line rather than the diff -- is a separate issue.
//! And attribution is textual, not parsed YAML: the owning job comes from the
//! indentation key stack visible inside one hunk, so an edge whose job key lives
//! in another hunk, or a flow-style `jobs: { … }` mapping, resolves to
//! [`UNRESOLVED_JOB`] and is held instead of waved through.
//!
//! Everything here is pure and testable: no I/O, no `gh`, no subprocess. The
//! caller supplies the patch text and the runs it was able to observe; this
//! module decides.

use std::collections::BTreeSet;

use crate::lint::diff::{parse_unified_diff, DiffLine, DiffLineKind, UnifiedDiff};

/// The hold reason recorded for every held promotion (issue #3863).
pub const HOLD_REASON: &str = "GATE_PROMOTED_WITHOUT_EVIDENCE";

/// Path prefix treated as CI workflow configuration unless the policy says
/// otherwise.
pub const DEFAULT_WORKFLOW_PREFIX: &str = ".github/workflows/";

/// Job recorded when the patch does not show the enclosing job key. Held rather
/// than attributed to a guess.
pub const UNRESOLVED_JOB: &str = "<unresolved job>";

/// Dependency recorded when the `needs:` value is computed (`${{ … }}`) or
/// otherwise unreadable. Held rather than treated as "no new gate".
pub const UNRESOLVED_DEPENDENCY: &str = "<unresolved dependency>";

/// Keys that contain jobs or job sections but are never themselves a job.
const CONTAINER_KEYS: &[&str] = &[
    "jobs",
    "on",
    "name",
    "run-name",
    "env",
    "permissions",
    "concurrency",
    "defaults",
    "strategy",
    "matrix",
    "services",
    "steps",
    "outputs",
    "with",
];

/// Which side of a hunk a line belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    /// Context plus removed lines: the file as it was.
    Before,
    /// Context plus added lines: the file as the patch leaves it.
    After,
}

impl View {
    fn includes(self, kind: DiffLineKind) -> bool {
        match self {
            Self::Before => matches!(kind, DiffLineKind::Context | DiffLineKind::Removed),
            Self::After => matches!(kind, DiffLineKind::Context | DiffLineKind::Added),
        }
    }
}

/// One `job -> needs` edge inside one workflow file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct JobDependency {
    /// The workflow file the edge lives in, repo-relative.
    pub workflow: String,
    /// The job that depends on another job, or [`UNRESOLVED_JOB`].
    pub job: String,
    /// The job now waited on, or [`UNRESOLVED_DEPENDENCY`].
    pub needs: String,
}

impl JobDependency {
    pub fn new(workflow: &str, job: &str, needs: &str) -> Self {
        Self {
            workflow: workflow.to_string(),
            job: job.to_string(),
            needs: needs.to_string(),
        }
    }

    pub fn attributed(&self) -> bool {
        self.job != UNRESOLVED_JOB && self.needs != UNRESOLVED_DEPENDENCY
    }
}

/// Which job edges a patch introduces and which it drops.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DependencyChanges {
    /// Edges present after the patch and absent before it.
    pub added: Vec<JobDependency>,
    /// Edges present before the patch and absent after it.
    pub removed: Vec<JobDependency>,
    /// Workflow files the patch touched, in patch order.
    pub workflows: Vec<String>,
}

impl DependencyChanges {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// What the caller observed about one job's runs. Supplied by the caller because
/// this module never calls out to a CI provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuccessfulRun {
    /// The job that completed.
    pub job: String,
    /// The run's identity as it appears in a citation: a run id, or a URL
    /// ending in one.
    pub run_ref: String,
    /// The branch the run executed on.
    pub branch: String,
    /// When set, the run is scoped to one workflow file; job names that repeat
    /// across workflows stay distinguishable.
    pub workflow: Option<String>,
}

impl SuccessfulRun {
    pub fn new(job: &str, run_ref: &str, branch: &str) -> Self {
        Self {
            job: job.to_string(),
            run_ref: run_ref.to_string(),
            branch: branch.to_string(),
            workflow: None,
        }
    }

    /// Scope the observation to one workflow file.
    pub fn in_workflow(mut self, workflow: &str) -> Self {
        self.workflow = Some(workflow.to_string());
        self
    }

    /// Whether a citation string names this run. The match is literal —
    /// `run 4692135711`, a bare id, or a URL containing it all cite it; "the
    /// reproducible build passes now" cites nothing.
    pub fn cited_by(&self, citations: &[String]) -> bool {
        if self.run_ref.is_empty() {
            return false;
        }
        let needle = self.run_ref.to_ascii_lowercase();
        citations
            .iter()
            .any(|citation| citation.to_ascii_lowercase().contains(&needle))
    }
}

/// The evidence a promotion may be cleared with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GateEvidence {
    /// Runs the caller observed to complete successfully. Empty means the
    /// caller could not observe runs, which is not the same as runs having
    /// passed.
    pub successful_runs: Vec<SuccessfulRun>,
    /// Run references the patch cites — a PR-body line, a commit message, or a
    /// comment added by the patch itself.
    pub citations: Vec<String>,
}

impl GateEvidence {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_run(mut self, run: SuccessfulRun) -> Self {
        self.successful_runs.push(run);
        self
    }

    pub fn citing(mut self, citation: &str) -> Self {
        if !citation.trim().is_empty() {
            self.citations.push(citation.to_string());
        }
        self
    }

    /// Successful runs of `needs` on the target branch, workflow-scoped.
    fn passing_runs(
        &self,
        needs: &str,
        workflow: &str,
        target_branch: &str,
    ) -> Vec<&SuccessfulRun> {
        self.successful_runs
            .iter()
            .filter(|run| {
                run.job == needs
                    && run.branch == target_branch
                    && run
                        .workflow
                        .as_deref()
                        .is_none_or(|scoped| scoped == workflow)
            })
            .collect()
    }
}

/// Policy for a review run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatePromotionPolicy {
    /// The branch the promoted gate must have passed on (usually `main`).
    pub target_branch: String,
    /// Path prefixes treated as CI workflow configuration.
    pub workflow_prefixes: Vec<String>,
}

impl Default for GatePromotionPolicy {
    fn default() -> Self {
        Self {
            target_branch: "main".to_string(),
            workflow_prefixes: vec![DEFAULT_WORKFLOW_PREFIX.to_string()],
        }
    }
}

impl GatePromotionPolicy {
    /// Whether `path` is CI workflow configuration under this policy.
    pub fn covers(&self, path: &str) -> bool {
        let path = path.replace('\\', "/");
        if !(path.ends_with(".yml") || path.ends_with(".yaml")) {
            return false;
        }
        self.workflow_prefixes
            .iter()
            .any(|prefix| path.contains(prefix.as_str()))
    }
}

/// Why a promotion is held, beside the single hold reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldKind {
    /// The job has no successful run on the target branch to cite.
    NoSuccessfulRun,
    /// Passing runs exist; the patch cites none of them.
    NotCited,
    /// The patch does not show which job gains the dependency.
    JobUnresolved,
    /// The `needs:` value is computed and cannot be resolved to a job.
    DependencyUnresolved,
    /// The patch is not a readable unified diff, so no dependency change in it
    /// can be checked.
    PatchUnreadable,
}

impl HoldKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoSuccessfulRun => "NO_SUCCESSFUL_RUN",
            Self::NotCited => "NOT_CITED",
            Self::JobUnresolved => "JOB_UNRESOLVED",
            Self::DependencyUnresolved => "DEPENDENCY_UNRESOLVED",
            Self::PatchUnreadable => "PATCH_UNREADABLE",
        }
    }
}

/// One held promotion: the edge, and what would clear it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateHold {
    /// Always [`HOLD_REASON`]; kept as a field so a caller can match on it
    /// without importing the constant.
    pub code: &'static str,
    /// The sub-reason, for log triage.
    pub kind: HoldKind,
    pub workflow: String,
    pub job: String,
    pub needs: String,
    /// Names the job and states the evidence that would clear the hold.
    pub message: String,
}

impl GateHold {
    pub fn line(&self) -> String {
        self.message.clone()
    }
}

/// The outcome of one review: what was promoted, what was released, what is
/// held.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GatePromotionReview {
    /// Every net-new dependency, held or not.
    pub promotions: Vec<JobDependency>,
    /// Every net-removed dependency. Never held: loosening a gate cannot break
    /// a green branch.
    pub releases: Vec<JobDependency>,
    /// Held promotions, in the order the patch introduces them.
    pub holds: Vec<GateHold>,
    /// Set when the patch could not be parsed at all: the hold is fail-closed,
    /// and this says why, so a caller can ask for a readable patch instead of
    /// for evidence.
    pub patch_parse_error: Option<String>,
}

impl GatePromotionReview {
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

/// Extract the net job-edge changes from a unified diff of CI workflow files.
///
/// Edges are computed per hunk, on the before-view (context + removed) and the
/// after-view (context + added) of that hunk, so a reordered `needs:` list
/// produces no change and a pure removal is never reported as an addition.
pub fn dependency_changes(
    diff_source: &str,
    policy: &GatePromotionPolicy,
) -> Result<DependencyChanges, String> {
    Ok(changes_from_diff(&parse_unified_diff(diff_source)?, policy))
}

fn changes_from_diff(diff: &UnifiedDiff, policy: &GatePromotionPolicy) -> DependencyChanges {
    let mut changes = DependencyChanges::default();
    for file in &diff.files {
        if !policy.covers(&file.path) {
            continue;
        }
        changes.workflows.push(file.path.clone());
        let mut before: BTreeSet<(String, String)> = BTreeSet::new();
        let mut after: BTreeSet<(String, String)> = BTreeSet::new();
        for hunk in &file.hunks {
            // Each hunk is read on its own: the enclosing job key has to be in
            // the hunk (as context or as an added line) for the edge to be
            // attributable.
            before.extend(dependencies_in(&hunk.lines, View::Before));
            after.extend(dependencies_in(&hunk.lines, View::After));
        }
        for (job, needs) in after.difference(&before) {
            changes
                .added
                .push(JobDependency::new(&file.path, job, needs));
        }
        for (job, needs) in before.difference(&after) {
            changes
                .removed
                .push(JobDependency::new(&file.path, job, needs));
        }
    }
    changes
}

/// Hold every patch that promotes a CI gate to blocking without citing a
/// passing run of that gate on the target branch.
pub fn review_gate_promotions(
    diff_source: &str,
    policy: &GatePromotionPolicy,
    evidence: &GateEvidence,
) -> GatePromotionReview {
    let diff = match parse_unified_diff(diff_source) {
        Ok(diff) => diff,
        Err(error) => return unreadable_review(error),
    };
    if diff.files.is_empty() && !diff_source.trim().is_empty() {
        // Prose, a truncated patch, a patch fed in the wrong format: nothing in
        // it could be read, so nothing in it can be cleared either.
        return unreadable_review("the patch lists no `diff --git` file entries".to_string());
    }
    let changes = changes_from_diff(&diff, policy);
    let mut holds: Vec<GateHold> = Vec::new();
    for promotion in &changes.added {
        if let Some(kind) = classify_unattributed(promotion) {
            holds.push(build_hold(kind, promotion, &[], policy, evidence));
            continue;
        }
        let passing =
            evidence.passing_runs(&promotion.needs, &promotion.workflow, &policy.target_branch);
        if passing.is_empty() {
            holds.push(build_hold(
                HoldKind::NoSuccessfulRun,
                promotion,
                &[],
                policy,
                evidence,
            ));
        } else if passing.iter().any(|run| run.cited_by(&evidence.citations)) {
            // The gate has passed on the target branch and the patch says which
            // run: the promotion stands.
        } else {
            let refs: Vec<String> = passing.iter().map(|run| run.run_ref.clone()).collect();
            holds.push(build_hold(
                HoldKind::NotCited,
                promotion,
                &refs,
                policy,
                evidence,
            ));
        }
    }
    GatePromotionReview {
        promotions: changes.added,
        releases: changes.removed,
        holds,
        patch_parse_error: None,
    }
}

/// A patch that cannot be read cannot be cleared either.
fn unreadable_review(error: String) -> GatePromotionReview {
    GatePromotionReview {
        promotions: Vec::new(),
        releases: Vec::new(),
        holds: vec![GateHold {
            code: HOLD_REASON,
            kind: HoldKind::PatchUnreadable,
            workflow: String::new(),
            job: String::new(),
            needs: String::new(),
            message: format!(
                "{HOLD_REASON} [{}]: the patch could not be read as a unified diff ({error}), so a workflow dependency change inside it cannot be checked. To clear: resubmit it as a `git diff` over the workflow file; a patch that cannot be read is held, not passed.",
                HoldKind::PatchUnreadable.as_str()
            ),
        }],
        patch_parse_error: Some(error),
    }
}

fn classify_unattributed(promotion: &JobDependency) -> Option<HoldKind> {
    if promotion.needs == UNRESOLVED_DEPENDENCY {
        return Some(HoldKind::DependencyUnresolved);
    }
    if promotion.job == UNRESOLVED_JOB {
        return Some(HoldKind::JobUnresolved);
    }
    None
}

fn build_hold(
    kind: HoldKind,
    promotion: &JobDependency,
    passing_run_refs: &[String],
    policy: &GatePromotionPolicy,
    evidence: &GateEvidence,
) -> GateHold {
    let workflow = &promotion.workflow;
    let job = &promotion.job;
    let needs = &promotion.needs;
    let branch = &policy.target_branch;
    let detail = match kind {
        HoldKind::NoSuccessfulRun => {
            let observed_elsewhere = evidence
                .successful_runs
                .iter()
                .filter(|run| {
                    run.job == *needs
                        && run.workflow
                            .as_deref()
                            .is_none_or(|scoped| scoped == *workflow)
                })
                .count();
            if observed_elsewhere == 0 {
                format!(
                    "`{needs}` has no successful run on `{branch}` to cite, so making `{job}` wait on it would block every downstream job on a gate that has never passed"
                )
            } else {
                format!(
                    "`{needs}` has {observed_elsewhere} successful run(s) on other branches and none on `{branch}`, so making `{job}` wait on it would block every downstream job on a gate that has never passed here"
                )
            }
        }
        HoldKind::NotCited => {
            let mut detail = format!(
                "`{needs}` has a successful run on `{branch}` but the patch cites none of them (passing: {})",
                passing_run_refs
                    .iter()
                    .map(|run| format!("`{run}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            if let Some(cited) = evidence
                .citations
                .iter()
                .find(|citation| !citation.trim().is_empty())
                .cloned()
            {
                detail.push_str(&format!(
                    "; the patch cites `{cited}`, which is not one of them"
                ));
            }
            detail
        }
        HoldKind::JobUnresolved => format!(
            "the job gaining the dependency on `{needs}` is not visible in the patch context, so the promotion cannot be attributed to a job or checked against its runs"
        ),
        HoldKind::DependencyUnresolved => format!(
            "the new dependency of `{job}` is computed (`{needs}`) and cannot be resolved to a job whose runs can be observed"
        ),
        // Constructed by `unreadable_review`, never through here.
        HoldKind::PatchUnreadable => String::from("the patch could not be read as a unified diff"),
    };
    let subject = if job == UNRESOLVED_JOB {
        format!("`{workflow}` adds a blocking dependency")
    } else {
        format!("`{workflow}` makes `{job}` wait on `{needs}`")
    };
    let clearance = match kind {
        HoldKind::JobUnresolved => format!(
            "To clear: resubmit the change so the hunk carries the enclosing job key line and the promoted job is attributable, then cite a successful run of `{needs}` on `{branch}` (run id or URL, e.g. `run 4692135711`)."
        ),
        HoldKind::DependencyUnresolved => format!(
            "To clear: name the concrete job in `needs:` so its runs can be observed, then cite a successful run of that job on `{branch}` (run id or URL, e.g. `run 4692135711`)."
        ),
        _ => format!(
            "To clear: cite a successful run of `{needs}` on `{branch}` (run id or URL, e.g. `run 4692135711`) in the PR description or in a comment the patch adds."
        ),
    };
    let proposal = if matches!(
        kind,
        HoldKind::JobUnresolved | HoldKind::DependencyUnresolved
    ) {
        String::new()
    } else {
        " An argument that the job will pass now is not evidence; if the run cannot be observed, propose the promotion in the PR body and leave the dependency out.".to_string()
    };
    GateHold {
        code: HOLD_REASON,
        kind,
        workflow: workflow.clone(),
        job: job.clone(),
        needs: needs.clone(),
        message: format!(
            "{HOLD_REASON} [{}]: {subject} — {detail}. {clearance}{proposal}",
            kind.as_str()
        ),
    }
}

/// One diff line reduced to what the workflow reader needs.
struct ObservedLine {
    indent: usize,
    trimmed: String,
}

fn observed_lines(lines: &[DiffLine], view: View) -> Vec<ObservedLine> {
    lines
        .iter()
        .filter(|line| view.includes(line.kind))
        .map(|line| ObservedLine {
            indent: indent_of(&line.content),
            trimmed: strip_trailing_comment(line.content.trim()).to_string(),
        })
        .collect()
}

/// `job -> needs` edges visible in one view of one hunk.
fn dependencies_in(lines: &[DiffLine], view: View) -> BTreeSet<(String, String)> {
    let lines = observed_lines(lines, view);
    let mut edges = BTreeSet::new();
    // Enclosing mapping keys, shallowest first, so the innermost key below a
    // `needs:` line is the job that owns it.
    let mut keys: Vec<(usize, String)> = Vec::new();
    // Indent of a `needs:` key whose dependencies follow as a block sequence.
    let mut block_needs: Option<usize> = None;

    for line in lines.iter().filter(|line| !line.trimmed.is_empty()) {
        // A line at column N closes every key opened at column N or deeper,
        // whatever kind of line it is: `needs:` four columns under `jobs:` must
        // inherit the job, never the `permissions:` block above it.
        while keys.last().is_some_and(|(depth, _)| *depth >= line.indent) {
            keys.pop();
        }

        if let Some(needs_indent) = block_needs {
            // `- job` at the key's own column is the compact block form.
            if line.indent >= needs_indent && line.trimmed.starts_with("- ") {
                if let Some(dep) = sequence_item(&line.trimmed) {
                    edges.insert((owner(&keys), dep));
                }
                continue;
            }
            if line.indent > needs_indent {
                continue;
            }
            block_needs = None;
        }

        if line.trimmed.starts_with('#') {
            continue;
        }

        if let Some(value) = line.trimmed.strip_prefix("needs:") {
            let value = value.trim();
            if value.is_empty() {
                block_needs = Some(line.indent);
            } else {
                for dep in needs_values(value) {
                    edges.insert((owner(&keys), dep));
                }
            }
            continue;
        }

        if let Some(name) = key_line(&line.trimmed) {
            keys.push((line.indent, name));
        }
    }
    edges
}

/// The job a `needs:` line belongs to: the innermost enclosing key that is not
/// a container key. Unresolvable is reported as such, never guessed.
fn owner(keys: &[(usize, String)]) -> String {
    match keys.last() {
        Some((_, name)) if !CONTAINER_KEYS.contains(&name.as_str()) => name.clone(),
        _ => UNRESOLVED_JOB.to_string(),
    }
}

/// A mapping key with no value on the line (`publish:`), unquoted or quoted.
fn key_line(trimmed: &str) -> Option<String> {
    if trimmed.starts_with('-') || trimmed.starts_with('#') || !trimmed.ends_with(':') {
        return None;
    }
    let name = &trimmed[..trimmed.len() - 1];
    if name.is_empty() || name.contains(':') {
        return None;
    }
    Some(unquote(name.trim()))
}

/// Dependencies named by `needs: job` or `needs: [a, b]`.
fn needs_values(value: &str) -> Vec<String> {
    let value = value.trim();
    if value.starts_with("${{") || value.contains("${{") {
        return vec![UNRESOLVED_DEPENDENCY.to_string()];
    }
    let inner = value
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(value);
    if inner.trim().is_empty() {
        return Vec::new();
    }
    inner
        .split(',')
        .map(|entry| unquote(entry.trim()))
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            if entry.contains("${{") || !is_job_name(&entry) {
                UNRESOLVED_DEPENDENCY.to_string()
            } else {
                entry
            }
        })
        .collect()
}

/// A block-sequence item (`- job`) under a `needs:` key.
fn sequence_item(trimmed: &str) -> Option<String> {
    let entry = unquote(trimmed.strip_prefix("- ")?.trim());
    if entry.is_empty() {
        return None;
    }
    Some(if entry.contains("${{") || !is_job_name(&entry) {
        UNRESOLVED_DEPENDENCY.to_string()
    } else {
        entry
    })
}

/// Job ids are plain identifiers: no spaces, no YAML punctuation.
fn is_job_name(value: &str) -> bool {
    value.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | '/')
    })
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    for quote in ['"', '\''] {
        if value.len() >= 2 && value.starts_with(quote) && value.ends_with(quote) {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

/// Leading indentation, with a tab counted as two columns.
fn indent_of(content: &str) -> usize {
    let mut indent = 0usize;
    for character in content.chars() {
        match character {
            ' ' => indent += 1,
            '\t' => indent += 2,
            _ => break,
        }
    }
    indent
}

/// Strip a trailing `# comment`, respecting quotes.
fn strip_trailing_comment(value: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    let mut previous_whitespace = true;
    for (offset, character) in value.char_indices() {
        match character {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '#' if !in_single && !in_double && previous_whitespace => {
                return value[..offset].trim_end()
            }
            other => previous_whitespace = other.is_whitespace(),
        }
    }
    value
}
