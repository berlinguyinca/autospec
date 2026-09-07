//! Spec repair: turn an unusable issue into a proposal instead of a dead-end refusal.
//!
//! When the pipeline judges an issue unusable — the mechanical case is an issue whose
//! acceptance criteria are all already true — refusing to dispatch is correct, but it
//! is a dead end: a human has to notice, work out why, and rewrite the issue. This
//! module turns the analysis the agent already did into a concrete, five-part repair
//! proposal posted as a **comment**, never as an edit to the issue body.
//!
//! Authority vs capability (#3531): a model can *propose* a specification; deciding
//! that the proposal *is* the requirement is an authorization gate. The proposal
//! comment says so explicitly, and the [`IssueRepairTracker`] trait has no
//! body-mutation method at all, so the repair path cannot silently overwrite the
//! issue body — a misreading must never look like a specification.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::coordination::RemoteIssue;
use crate::error::AutospecError;

/// Label applied while an issue is waiting for a maintainer to answer a repair
/// proposal. Removed when a maintainer reply approves the proposal.
pub const NEEDS_SPEC_CLARIFICATION_LABEL: &str = "needs-spec-clarification";

/// Line a maintainer posts to adopt a repair proposal as the requirement of record.
pub const APPROVAL_MARKER: &str = "spec-repair: approved";

/// Line a maintainer posts to decline a repair proposal (the issue then needs a
/// human rewrite; the label stays on).
pub const REJECTION_MARKER: &str = "spec-repair: rejected";

/// HTML comment marking a posted repair-proposal comment.
pub const PROPOSAL_MARKER: &str = "<!-- autospec:spec-repair-proposal -->";

/// HTML comment marking the escalation notice posted when a second consecutive
/// repair proposal would otherwise loop.
pub const ESCALATION_MARKER: &str = "<!-- autospec:spec-repair-escalation -->";

/// Heading under which a low-stakes assumption is recorded in a PR body so review
/// can see it.
pub const ASSUMPTION_HEADING: &str = "## Stated assumption (reviewable)";

/// Number of consecutive no-output runs after which the pipeline must trigger spec
/// review instead of retrying: an issue that stalls repeatedly without producing
/// output is evidence of a spec problem, not only a model problem.
pub const SPEC_REVIEW_STALL_THRESHOLD: u32 = 2;

/// Number of repair proposals on one issue after which the repair loop escalates to
/// a human instead of proposing again. Two proposals may happen; a third attempt
/// must not.
pub const ESCALATION_AFTER_CONSECUTIVE_PROPOSALS: usize = 2;

/// Whether an issue's acceptance-criteria checkboxes are open, satisfied, or mixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CriteriaState {
    /// No `## Acceptance criteria` checkbox section at all.
    NoCriteria,
    /// Every checkbox is unchecked — work remains, dispatch normally.
    OpenCriteria,
    /// Every checkbox is checked: the issue reads as already done. Dispatching it is
    /// the dead end this module repairs.
    AllSatisfied,
    /// Some checked, some unchecked — a partially landed issue, not this module's case.
    Mixed,
}

impl CriteriaState {
    /// Machine-readable name for diagnostics and ledgers.
    pub fn id(self) -> &'static str {
        match self {
            Self::NoCriteria => "no_criteria",
            Self::OpenCriteria => "open_criteria",
            Self::AllSatisfied => "all_satisfied",
            Self::Mixed => "mixed",
        }
    }
}

/// Extract the body of the acceptance-criteria section, mirroring the shell
/// `extract_section` helper: the heading must be an exact line, and content ends at
/// the next `## ` heading.
fn acceptance_criteria_section(body: &str) -> Vec<&str> {
    let heading = body
        .lines()
        .position(|line| line == "## Acceptance criteria")
        .or_else(|| {
            body.lines()
                .position(|line| line == "## Acceptance Criteria")
        });
    let Some(start) = heading else {
        return Vec::new();
    };
    let rest = &body.lines().collect::<Vec<_>>()[start + 1..];
    let end = rest
        .iter()
        .position(|line| line.starts_with("## "))
        .unwrap_or(rest.len());
    rest[..end].to_vec()
}

fn is_checkbox(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("- [ ]")
        || trimmed.starts_with("- [x]")
        || trimmed.starts_with("- [X]")
        || trimmed.starts_with("* [ ]")
        || trimmed.starts_with("* [x]")
        || trimmed.starts_with("* [X]")
}

fn is_checked_checkbox(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("- [x]")
        || trimmed.starts_with("- [X]")
        || trimmed.starts_with("* [x]")
        || trimmed.starts_with("* [X]")
}

/// Classify the acceptance-criteria checkboxes of an issue body.
pub fn acceptance_criteria_state(body: &str) -> CriteriaState {
    let section = acceptance_criteria_section(body);
    let checkboxes = section
        .iter()
        .filter(|line| is_checkbox(line))
        .collect::<Vec<_>>();
    if checkboxes.is_empty() {
        return CriteriaState::NoCriteria;
    }
    let checked = checkboxes
        .iter()
        .filter(|line| is_checked_checkbox(line))
        .count();
    if checked == 0 {
        CriteriaState::OpenCriteria
    } else if checked == checkboxes.len() {
        CriteriaState::AllSatisfied
    } else {
        CriteriaState::Mixed
    }
}

/// The six failure shapes an unusable spec can take. Each needs a different
/// proposal, which is why classification comes first.
///
/// Serialized ids are the snake_case forms returned by [`Self::id`], so proposal
/// documents and ledger lines share one vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecDefectShape {
    /// Every criterion is already true; the agent verified and correctly stopped.
    /// Repair: rewrite criteria to assert the new behaviour, each shown failing today.
    NoFalsifiableCriterion,
    /// Criteria referenced an artifact that was never committed, so the check ran
    /// 0/0 and read as a pass. Repair: name the missing artifact and propose it as
    /// a dependency or a prior issue.
    MissingPrecondition,
    /// Two readings imply materially different work. Repair: state both readings,
    /// recommend one, ask which.
    Ambiguous,
    /// One issue covering several independent deliverables, stalled repeatedly.
    /// Repair: propose a split with the boundary named, hand to the splitter.
    OverScoped,
    /// Criteria with no mechanical check ("should be fast"). Repair: propose a
    /// measurable restatement with a threshold and the command.
    Unverifiable,
    /// Criteria conflict with each other or with the codebase. Repair: name the
    /// conflict and both sides; ask which governs.
    Contradictory,
}

impl SpecDefectShape {
    pub fn all() -> &'static [Self] {
        &[
            Self::NoFalsifiableCriterion,
            Self::MissingPrecondition,
            Self::Ambiguous,
            Self::OverScoped,
            Self::Unverifiable,
            Self::Contradictory,
        ]
    }

    /// Machine-readable id used in ledgers (the queryable defect classification).
    pub fn id(self) -> &'static str {
        match self {
            Self::NoFalsifiableCriterion => "no_falsifiable_criterion",
            Self::MissingPrecondition => "missing_precondition",
            Self::Ambiguous => "ambiguous",
            Self::OverScoped => "over_scoped",
            Self::Unverifiable => "unverifiable",
            Self::Contradictory => "contradictory",
        }
    }

    /// One-sentence classification for the comment's defect section.
    pub fn headline(self) -> &'static str {
        match self {
            Self::NoFalsifiableCriterion => {
                "every acceptance criterion is already true, so the issue has nothing left to verify"
            }
            Self::MissingPrecondition => {
                "a criterion depends on an artifact that does not exist, so its check passes vacuously"
            }
            Self::Ambiguous => "the issue admits two readings that imply materially different work",
            Self::OverScoped => {
                "one issue bundles several independent deliverables and stalls without output"
            }
            Self::Unverifiable => {
                "at least one criterion has no mechanical check, so it cannot be falsified"
            }
            Self::Contradictory => {
                "the acceptance criteria conflict with each other or with the codebase"
            }
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::all().iter().copied().find(|shape| shape.id() == id)
    }
}

/// One proposed acceptance criterion together with the command that checks it and
/// the exit status observed today, so the reader can see it fails right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedCriterion {
    /// The criterion, rewritten to be falsifiable.
    pub requirement: String,
    /// The command whose result decides the criterion.
    pub check_command: String,
    /// Exit status the check command produced on the current tree (non-zero = fails
    /// today, which is what a repair proposal must show).
    pub current_exit_status: i32,
}

impl ProposedCriterion {
    pub fn fails_today(&self) -> bool {
        self.current_exit_status != 0
    }
}

/// The agent-authored content of a repair proposal (everything except the issue
/// number, which the pipeline supplies).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SpecRepairProposalInput {
    pub shape: SpecDefectShape,
    /// The best available interpretation, stated plainly — the reading the agent
    /// would actually act on, not a hedge.
    pub reading: String,
    /// Proposed criteria, each with its check command and current exit status.
    pub criteria: Vec<ProposedCriterion>,
    /// The specific question whose answer unblocks the issue, with the plausible
    /// answers named so it can be settled in one word.
    pub question: String,
    /// What was checked to reach this conclusion, so the reader can see it is not a
    /// guess.
    pub checked: Vec<String>,
}

impl SpecRepairProposalInput {
    /// Validate the five required parts. Every proposed criterion must carry a check
    /// command and fail today — that is what lets the reader see the gap.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        if self.reading.trim().is_empty() {
            errors.push(
                "reading is empty: state the interpretation the agent would act on".to_string(),
            );
        }
        if self.criteria.is_empty() {
            errors
                .push("criteria are empty: propose at least one falsifiable criterion".to_string());
        }
        for (index, criterion) in self.criteria.iter().enumerate() {
            let label = format!("criterion {}", index + 1);
            if criterion.requirement.trim().is_empty() {
                errors.push(format!("{label} has an empty requirement"));
            }
            if criterion.check_command.trim().is_empty() {
                errors.push(format!(
                    "{label} has no check command: name the command that decides it"
                ));
            }
            if !criterion.fails_today() {
                errors.push(format!(
                    "{label} passes today (exit 0): a repair criterion must fail on the current tree"
                ));
            }
        }
        if self.question.trim().is_empty() {
            errors.push(
                "question is empty: ask the one question whose answer unblocks the issue"
                    .to_string(),
            );
        } else {
            let question = self.question.trim();
            if !question.ends_with('?') {
                errors.push("question must end with '?'".to_string());
            }
            if question.to_ascii_lowercase().contains("please clarify") {
                errors.push(
                    "question must not be a bare 'please clarify': name the plausible answers"
                        .to_string(),
                );
            }
        }
        if self.checked.is_empty() {
            errors.push(
                "checked is empty: record what was examined so the reader can see it is not a guess"
                    .to_string(),
            );
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub fn into_proposal(self, issue_number: u64) -> SpecRepairProposal {
        SpecRepairProposal {
            issue_number,
            shape: self.shape,
            reading: self.reading,
            criteria: self.criteria,
            question: self.question,
            checked: self.checked,
        }
    }
}

/// A complete repair proposal bound to one issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecRepairProposal {
    pub issue_number: u64,
    pub shape: SpecDefectShape,
    pub reading: String,
    pub criteria: Vec<ProposedCriterion>,
    pub question: String,
    pub checked: Vec<String>,
}

impl SpecRepairProposal {
    /// Render the one comment that carries all five required parts.
    ///
    /// The banner states that the comment is a proposal, not a decision, and that the
    /// issue body was not modified — adoption is a maintainer authority act, done by
    /// editing the body or by replying with a recognized approval.
    pub fn render_comment(&self) -> String {
        let mut comment = String::new();
        comment.push_str(PROPOSAL_MARKER);
        comment.push_str("\n## Spec repair proposal (a proposal, not a decision)\n\n");
        comment.push_str(
            "An agent found this issue unusable as specified and proposes the repair below. \
             **This comment is a proposal. It is not a decision, and the issue body was not \
             modified.** Adopting it as the requirement of record is a maintainer act: reply \
             with a line reading `spec-repair: approved` to adopt it (re-dispatch then proceeds \
             without a body rewrite), or `spec-repair: rejected: <reason>` to decline.\n\n",
        );
        comment.push_str(&format!(
            "### 1. The defect\n\n- classification: `{}` — {}\n\n",
            self.shape.id(),
            self.shape.headline()
        ));
        comment.push_str("### 2. What we think it means\n\n");
        comment.push_str(self.reading.trim());
        comment.push_str("\n\n### 3. Proposed acceptance criteria\n\n");
        comment.push_str(
            "Each criterion is rewritten to be falsifiable and shown failing today, with the \
             command that checks it and its current exit status:\n\n",
        );
        for criterion in &self.criteria {
            comment.push_str(&format!("- `{}`\n", criterion.requirement.trim()));
            comment.push_str(&format!(
                "  - check: `{}`\n",
                criterion.check_command.trim()
            ));
            if criterion.fails_today() {
                comment.push_str(&format!(
                    "  - current exit status: {} (fails today)\n",
                    criterion.current_exit_status
                ));
            } else {
                comment.push_str(
                    "  - current exit status: 0 (passes today — not a repair criterion)\n",
                );
            }
        }
        comment.push_str("\n### 4. The question\n\n");
        comment.push_str(self.question.trim());
        comment.push_str("\n\n### 5. What was checked\n\n");
        for item in &self.checked {
            comment.push_str(&format!("- {}\n", item.trim()));
        }
        comment
    }
}

/// High-stakes subjects where a wrong reading is expensive or hard to reverse. A
/// low-stakes ambiguity may proceed under a stated assumption; these must always
/// block and ask.
const HIGH_STAKES_MARKERS: &[&str] = &[
    "security",
    "credential",
    "secret",
    "authentication",
    "authorization",
    "permission",
    "migration",
    "public interface",
    "public api",
    "api contract",
    "breaking change",
    "destructive",
    "irreversible",
    "data loss",
];

/// Whether an ambiguity is low-stakes (reversible — proceed under a stated
/// assumption) or high-stakes (always block and ask).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmbiguityStakes {
    Low,
    High,
}

impl AmbiguityStakes {
    pub fn id(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::High => "high",
        }
    }
}

fn contains_word(haystack_lower: &str, needle_lower: &str) -> bool {
    let Some(start) = haystack_lower.find(needle_lower) else {
        return false;
    };
    let before_ok = start == 0
        || !haystack_lower[..start]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric);
    let after = start + needle_lower.len();
    let after_ok = after >= haystack_lower.len()
        || !haystack_lower[after..]
            .chars()
            .next()
            .is_some_and(char::is_alphanumeric);
    before_ok && after_ok
}

/// Classify the stakes of an ambiguity from the subject under debate plus any
/// surrounding context. The test is not "am I uncertain" but "would being wrong here
/// be cheap to undo": security boundaries, data migrations, public interfaces, and
/// destructive actions are never cheap to undo.
pub fn classify_ambiguity_stakes(subject: &str) -> AmbiguityStakes {
    let lowered = subject.to_ascii_lowercase();
    for marker in HIGH_STAKES_MARKERS {
        let hit = if marker.contains(' ') {
            lowered.contains(marker)
        } else {
            contains_word(&lowered, marker)
        };
        if hit {
            return AmbiguityStakes::High;
        }
    }
    AmbiguityStakes::Low
}

/// A low-stakes assumption an agent proceeds under, recorded in the PR body so
/// review can see and overturn it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatedAssumption {
    /// The assumption, stated plainly enough that a reviewer can reject it in one word.
    pub statement: String,
    /// The other readings that were considered.
    pub alternatives_considered: Vec<String>,
    /// How to undo work built on the assumption if it is wrong.
    pub rollback: String,
}

/// Render the PR-body section that records a stated assumption.
///
/// Returns `Err` for a high-stakes subject: there the ambiguity must block and ask,
/// because being wrong would be expensive or hard to reverse.
pub fn render_pr_assumption_section(
    assumption: &StatedAssumption,
    stakes: AmbiguityStakes,
) -> Result<String, String> {
    if stakes == AmbiguityStakes::High {
        return Err(
            "high-stakes ambiguity (security boundary, data migration, public interface, or \
             destructive action): it must block and ask, not proceed under an assumption"
                .to_string(),
        );
    }
    if assumption.statement.trim().is_empty() {
        return Err("assumption statement is empty".to_string());
    }
    let mut section = String::new();
    section.push_str(ASSUMPTION_HEADING);
    section.push_str(
        "\n\nProceeding under an explicitly stated assumption because being wrong here is \
         cheap to undo. Reviewers: reject this assumption in a one-word reply if it is wrong.\n\n",
    );
    section.push_str(&format!(
        "- **Assumption:** {}\n",
        assumption.statement.trim()
    ));
    if assumption.alternatives_considered.is_empty() {
        section.push_str("- **Alternatives considered:** none — no other reading was identified\n");
    } else {
        section.push_str("- **Alternatives considered:**\n");
        for alternative in &assumption.alternatives_considered {
            section.push_str(&format!("  - {}\n", alternative.trim()));
        }
    }
    section.push_str(&format!(
        "- **If wrong, undo by:** {}\n",
        if assumption.rollback.trim().is_empty() {
            "revert the PR; no irreversible effect is in scope"
        } else {
            assumption.rollback.trim()
        }
    ));
    Ok(section)
}

/// How a maintainer responded to a repair proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaintainerReply {
    /// A line reading `spec-repair: approved` — adopt the proposal as the
    /// requirement of record and allow re-dispatch without a body rewrite.
    Approval,
    /// A line reading `spec-repair: rejected: <reason>` — a human will rewrite the
    /// issue; the proposal is dead.
    Rejection { reason: Option<String> },
    /// Anything else — still waiting.
    Other,
}

/// Classify a comment body as a proposal approval, a rejection, or neither.
/// Only a marker at the start of a line counts, so quoting the banner inside
/// another comment cannot forge a decision.
pub fn classify_maintainer_reply(body: &str) -> MaintainerReply {
    for line in body.lines() {
        let trimmed = line.trim().to_ascii_lowercase();
        if let Some(rest) = trimmed.strip_prefix("spec-repair:") {
            let rest = rest.trim_start();
            if rest.starts_with("approved") {
                return MaintainerReply::Approval;
            }
            if let Some(reason) = rest.strip_prefix("rejected") {
                let reason = reason.trim_start_matches([':', ' ']).trim();
                return MaintainerReply::Rejection {
                    reason: (!reason.is_empty()).then(|| reason.to_string()),
                };
            }
        }
    }
    MaintainerReply::Other
}

/// One recorded spec-repair event: the defect classification plus the issue's
/// origin (which template, which command, which author produced it). One badly
/// written issue is noise; the same defect recurring from one template is a
/// systemic problem, fixable at the source — but only if the recurrence is
/// recorded in a queryable form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecRepairEvent {
    /// Unix seconds at recording time.
    pub recorded_at: u64,
    pub repo: String,
    pub issue: u64,
    /// [`SpecDefectShape::id`] of the classified defect.
    pub shape: String,
    /// The issue template (or generator) the issue originated from, if known.
    pub origin_template: String,
    /// The command that created the issue, if known.
    pub origin_command: String,
    /// The author of the issue.
    pub origin_author: String,
}

/// Append one event as a JSON line. The ledger is append-only JSON Lines so it
/// stays queryable with plain text tools on any platform.
pub fn append_repair_event(path: &Path, event: &SpecRepairEvent) -> std::io::Result<()> {
    let line =
        serde_json::to_string(event).map_err(|error| std::io::Error::other(error.to_string()))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{line}")
}

/// Load every recorded event. A missing ledger is an empty history.
pub fn load_repair_events(path: &Path) -> std::io::Result<Vec<SpecRepairEvent>> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut events = Vec::new();
    for line in contents.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let event = serde_json::from_str(line).map_err(|error| {
            std::io::Error::other(format!("corrupt repair ledger line: {error}"))
        })?;
        events.push(event);
    }
    Ok(events)
}

/// How many repair proposals have already been posted for one issue. Two may
/// happen; a third attempt escalates instead of looping.
pub fn proposal_count(events: &[SpecRepairEvent], repo: &str, issue: u64) -> usize {
    events
        .iter()
        .filter(|event| event.repo == repo && event.issue == issue)
        .count()
}

/// Whether a repair attempt with this many prior proposals must escalate to a
/// human rather than loop — a repair loop that can propose to itself indefinitely
/// is a way to spend an entire budget converging on nothing.
pub fn should_escalate_to_human(prior_proposals: usize) -> bool {
    prior_proposals >= ESCALATION_AFTER_CONSECUTIVE_PROPOSALS
}

/// Whether `consecutive_no_output_runs` stalled dispatches on one issue must
/// trigger spec review instead of another retry.
pub fn should_trigger_spec_review(consecutive_no_output_runs: u32) -> bool {
    consecutive_no_output_runs >= SPEC_REVIEW_STALL_THRESHOLD
}

/// Defect counts by classification — the systemic view: one shape recurring from
/// one origin template is a source-level problem, not one issue at a time.
pub fn summarize_by_shape(events: &[SpecRepairEvent]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for event in events {
        *counts.entry(event.shape.clone()).or_insert(0) += 1;
    }
    counts
}

/// Defect counts by the issue's origin template — where to fix the generator.
pub fn summarize_by_origin_template(events: &[SpecRepairEvent]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for event in events {
        let origin = if event.origin_template.is_empty() {
            "(unknown)"
        } else {
            &event.origin_template
        };
        *counts.entry(origin.to_string()).or_insert(0) += 1;
    }
    counts
}

/// A comment on the issue being repaired. Serializable so platform adapters can
/// parse it straight out of tracker JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueCommentSnapshot {
    pub author: String,
    pub body: String,
}

/// The issue-tracker surface the repair loop needs.
///
/// There is deliberately **no method to write an issue body**. The repair path can
/// read, comment, and label — nothing else — so the "the proposal is a comment and
/// the body is never modified" rule is enforced by the interface itself, on every
/// platform adapter, rather than by a convention reviewers must remember.
pub trait IssueRepairTracker {
    fn read_issue(&self, repo: &str, number: u64) -> std::io::Result<RemoteIssue>;
    fn post_comment(&self, repo: &str, number: u64, body: &str) -> std::io::Result<()>;
    fn add_label(&self, repo: &str, number: u64, label: &str) -> std::io::Result<()>;
    fn remove_label(&self, repo: &str, number: u64, label: &str) -> std::io::Result<()>;
    fn list_comments(&self, repo: &str, number: u64) -> std::io::Result<Vec<IssueCommentSnapshot>>;
}

/// Outcome of a repair attempt on an unusable issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposeOutcome {
    /// The five-part proposal was posted and the label applied.
    Posted { comment: String },
    /// A proposal is already on the issue and no maintainer has answered: do not
    /// post a second one. The label is ensured.
    AlreadyProposed,
    /// This issue already carries two repair proposals: a third would be the
    /// pipeline proposing to itself. An escalation notice is posted (once) and the
    /// loop hands the issue to a human.
    EscalatedToHuman,
}

/// Outcome of checking an issue for a maintainer's answer to a repair proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// No proposal comment exists on the issue.
    NoProposal,
    /// A proposal exists but no maintainer reply after it.
    AwaitingMaintainer,
    /// A maintainer approved: the label is removed and re-dispatch may proceed on
    /// the proposal as the requirement of record (recorded in the comment, not a
    /// body rewrite).
    Approved { label_removed: bool },
    /// A maintainer rejected: a human will rewrite the issue; the label stays on.
    Rejected,
}

fn has_comment_with_marker(comments: &[IssueCommentSnapshot], marker: &str) -> bool {
    comments.iter().any(|comment| comment.body.contains(marker))
}

fn ensure_label(
    tracker: &dyn IssueRepairTracker,
    repo: &str,
    issue: &RemoteIssue,
) -> std::io::Result<()> {
    if !issue
        .labels
        .iter()
        .any(|existing| existing == NEEDS_SPEC_CLARIFICATION_LABEL)
    {
        tracker.add_label(repo, issue.number, NEEDS_SPEC_CLARIFICATION_LABEL)?;
    }
    Ok(())
}

fn escalation_comment(issue_number: u64, proposal_count: usize) -> String {
    format!(
        "{ESCALATION_MARKER}\n## Escalating to a human\n\nIssue #{issue_number} has received \
         {proposal_count} spec-repair proposals and is still unusable. A repair loop that \
         keeps proposing to itself only spends budget converging on nothing, so no further \
         automatic proposals will be posted: a maintainer must rewrite the issue (or close \
         it). The earlier proposals remain above as candidate readings, and their defect \
         classifications are recorded in the repair ledger.\n"
    )
}

/// Run one repair attempt on an issue the pipeline judged unusable.
///
/// `judged_unusable` records that the caller judged the issue unusable for a reason
/// the mechanical criteria check cannot see (ambiguity, contradiction, …). Without
/// it the issue must at least be in the mechanical dead-end state — every acceptance
/// criterion already true — or the attempt is refused: a repair loop is for unusable
/// issues, not a way to comment on workable ones.
pub fn propose_spec_repair(
    tracker: &dyn IssueRepairTracker,
    repo: &str,
    issue_number: u64,
    input: SpecRepairProposalInput,
    prior_proposals: usize,
    judged_unusable: bool,
) -> Result<ProposeOutcome, AutospecError> {
    let issue = tracker
        .read_issue(repo, issue_number)
        .map_err(|error| AutospecError::io("read issue", repo, error))?;
    let state = acceptance_criteria_state(&issue.body);
    if state != CriteriaState::AllSatisfied && !judged_unusable {
        return Err(AutospecError::validation(format!(
            "issue #{issue_number} is not in the mechanical dead-end state (acceptance criteria: \
             {}); pass judged-unusable only after a close read shows the issue unusable for \
             another reason",
            state.id()
        )));
    }
    if let Err(errors) = input.validate() {
        return Err(AutospecError::validation(errors.join("; ")));
    }
    let comments = tracker
        .list_comments(repo, issue_number)
        .map_err(|error| AutospecError::io("list issue comments", repo, error))?;
    if should_escalate_to_human(prior_proposals) {
        if !has_comment_with_marker(&comments, ESCALATION_MARKER) {
            tracker
                .post_comment(
                    repo,
                    issue_number,
                    &escalation_comment(issue_number, prior_proposals),
                )
                .map_err(|error| AutospecError::io("post escalation comment", repo, error))?;
        }
        ensure_label(tracker, repo, &issue)?;
        return Ok(ProposeOutcome::EscalatedToHuman);
    }
    if has_comment_with_marker(&comments, PROPOSAL_MARKER) {
        ensure_label(tracker, repo, &issue)?;
        return Ok(ProposeOutcome::AlreadyProposed);
    }
    let proposal = input.into_proposal(issue_number);
    let comment = proposal.render_comment();
    tracker
        .post_comment(repo, issue_number, &comment)
        .map_err(|error| AutospecError::io("post repair proposal", repo, error))?;
    ensure_label(tracker, repo, &issue)?;
    Ok(ProposeOutcome::Posted { comment })
}

/// Check an issue for a maintainer's answer to its repair proposal and, on
/// approval, remove the clarification label so re-dispatch can proceed.
pub fn check_spec_repair(
    tracker: &dyn IssueRepairTracker,
    repo: &str,
    issue_number: u64,
) -> Result<CheckOutcome, AutospecError> {
    let issue = tracker
        .read_issue(repo, issue_number)
        .map_err(|error| AutospecError::io("read issue", repo, error))?;
    let comments = tracker
        .list_comments(repo, issue_number)
        .map_err(|error| AutospecError::io("list issue comments", repo, error))?;
    let Some(proposal_index) = comments
        .iter()
        .rposition(|comment| comment.body.contains(PROPOSAL_MARKER))
    else {
        return Ok(CheckOutcome::NoProposal);
    };
    let mut outcome = CheckOutcome::AwaitingMaintainer;
    for comment in &comments[proposal_index + 1..] {
        match classify_maintainer_reply(&comment.body) {
            MaintainerReply::Approval => {
                outcome = CheckOutcome::Approved {
                    label_removed: false,
                };
                break;
            }
            MaintainerReply::Rejection { .. } => {
                outcome = CheckOutcome::Rejected;
                break;
            }
            MaintainerReply::Other => {}
        }
    }
    if let CheckOutcome::Approved { .. } = outcome {
        let label_removed = issue
            .labels
            .iter()
            .any(|label| label == NEEDS_SPEC_CLARIFICATION_LABEL);
        if label_removed {
            tracker
                .remove_label(repo, issue_number, NEEDS_SPEC_CLARIFICATION_LABEL)
                .map_err(|error| {
                    AutospecError::io("remove needs-spec-clarification label", repo, error)
                })?;
        }
        outcome = CheckOutcome::Approved { label_removed };
    }
    Ok(outcome)
}
