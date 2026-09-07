//! Post-merge integration gate.
//!
//! A green merge is not integration (#3569). Every branch is verified against
//! the trunk *as it was at dispatch*; nothing verifies the trunk *after* the
//! merges land. Two merges can each be correct in isolation and together
//! contradictory: one validates a field as `^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$`,
//! the other requires that same field to equal a filesystem path. No textual
//! conflict, no new test failure — only running the system reveals it.
//!
//! This module encodes the missing pipeline steps, cheapest first:
//!
//! 1. Re-run the acceptance gate on the trunk **after each merge** in a batch
//!    ([`verify_trunk_after_merges`]) so a failure names the merge that
//!    introduced it rather than the batch.
//! 2. Run the end-to-end smoke test on the trunk post-merge
//!    ([`evaluate_post_merge_smoke`]); a project *without* one is reported as
//!    a gap, not a silence.
//! 3. Detect collisions on **concepts**, not just files
//!    ([`detect_concept_collisions`]): two issues that constrain the same
//!    field, type, or identifier are sequenced or one defines the concept and
//!    the other builds on it.
//! 4. Surface constraints on shared concepts that were never recorded where
//!    the next implementer reads them ([`scan_added_lines`] +
//!    [`find_unrecorded_constraints`]). A constraint that lives only inside
//!    one function is invisible to everyone else.
//!
//! The core never shells out; callers supply the gate results, the smoke
//! outcome, and the issue/diff evidence.

use std::collections::{BTreeMap, BTreeSet};

// ---------------------------------------------------------------------------
// 1. Verify the trunk after each merge, not only the branch before it
// ---------------------------------------------------------------------------

/// One merge landing on trunk, in landing order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrunkMerge {
    /// Stable identifier for the merge, e.g. the PR reference (`PR#4102`).
    pub id: String,
    pub issue: u64,
    pub branch: String,
}

/// The acceptance-gate result of running the gate on the trunk after a merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateOutcome {
    Pass,
    Fail { findings: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrunkGateVerdict {
    /// The acceptance gate passed on the trunk after every merge in the batch.
    Verified { merges_verified: usize },
    /// The trunk gate failed; the merge that introduced the failure is named.
    BrokenByMerge {
        merge: TrunkMerge,
        /// Zero-based position of the failing merge in the batch.
        merge_index: usize,
        findings: Vec<String>,
    },
}

impl TrunkGateVerdict {
    /// One-line, result-first summary for monitor logs.
    pub fn summary(&self) -> String {
        match self {
            Self::Verified { merges_verified } => {
                format!("trunk verified after {merges_verified} merge(s)")
            }
            Self::BrokenByMerge {
                merge,
                merge_index,
                findings,
            } => format!(
                "trunk broken after merge {} (issue #{}, branch {}) at batch position {}: {}",
                merge.id,
                merge.issue,
                merge.branch,
                merge_index,
                findings.join("; ")
            ),
        }
    }
}

/// Verify a batch by re-running the acceptance gate on the trunk after each
/// merge, in landing order.
///
/// `gate_results` must contain exactly one outcome per merge — verifying the
/// batch once, or skipping a merge, is the gap this exists to close and is
/// rejected with an error rather than silently accepted. Evaluation stops at
/// the first failing merge so the failure names that merge.
pub fn verify_trunk_after_merges(
    merges: &[TrunkMerge],
    gate_results: &[GateOutcome],
) -> Result<TrunkGateVerdict, String> {
    if gate_results.len() != merges.len() {
        return Err(format!(
            "expected one post-merge trunk gate result per merge ({} merge(s), {} result(s)); the acceptance gate must be re-run on the trunk after each merge, not per batch",
            merges.len(),
            gate_results.len()
        ));
    }
    for (index, (merge, outcome)) in merges.iter().zip(gate_results.iter()).enumerate() {
        if let GateOutcome::Fail { findings } = outcome {
            return Ok(TrunkGateVerdict::BrokenByMerge {
                merge: merge.clone(),
                merge_index: index,
                findings: findings.clone(),
            });
        }
    }
    Ok(TrunkGateVerdict::Verified {
        merges_verified: merges.len(),
    })
}

// ---------------------------------------------------------------------------
// 2. An end-to-end smoke test that exercises real components
// ---------------------------------------------------------------------------

/// Whether the project has an end-to-end smoke test that starts the real
/// binary and exercises its primary path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmokeCoverage {
    Present {
        command: String,
    },
    /// "This project has no end-to-end test" is a finding, not a silence.
    Gap {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmokeOutcome {
    Pass,
    Fail { findings: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostMergeSmokeReport {
    /// True when the project has a smoke test and it was run on the trunk.
    pub covered: bool,
    /// The trunk smoke-test outcome; `None` only when coverage is a gap.
    pub outcome: Option<SmokeOutcome>,
    /// `Some(reason)` when the project has no end-to-end smoke test. The
    /// caller must surface this to the operator — a gap is a finding.
    pub gap: Option<String>,
}

impl PostMergeSmokeReport {
    /// `None` when the trunk is covered by a passing smoke test; `Some`
    /// otherwise (gap or failing smoke), so callers cannot pass silently.
    pub fn finding(&self) -> Option<String> {
        if let Some(reason) = &self.gap {
            return Some(format!(
                "this project has no end-to-end smoke test: {reason}"
            ));
        }
        match &self.outcome {
            Some(SmokeOutcome::Pass) | None => None,
            Some(SmokeOutcome::Fail { findings }) => {
                Some(format!("trunk smoke test failed: {}", findings.join("; ")))
            }
        }
    }
}

/// Combine post-merge smoke coverage with the outcome of running it on the
/// trunk.
///
/// A project *with* a smoke test must have it run on the trunk post-merge —
/// missing the outcome is an error. A project *without* one is reported as a
/// gap (with the reason the caller supplied) rather than passing silently.
pub fn evaluate_post_merge_smoke(
    coverage: &SmokeCoverage,
    outcome: Option<SmokeOutcome>,
) -> Result<PostMergeSmokeReport, String> {
    match coverage {
        SmokeCoverage::Present { command } => {
            let outcome = outcome.ok_or_else(|| {
                format!(
                    "smoke test {command:?} exists but was not run on the trunk post-merge; a green merge is not integration"
                )
            })?;
            Ok(PostMergeSmokeReport {
                covered: true,
                outcome: Some(outcome),
                gap: None,
            })
        }
        SmokeCoverage::Gap { reason } => {
            if outcome.is_some() {
                return Err(
                    "a smoke outcome was recorded but the project has no end-to-end smoke test"
                        .to_string(),
                );
            }
            Ok(PostMergeSmokeReport {
                covered: false,
                outcome: None,
                gap: Some(reason.clone()),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// 3. Detect collisions on concepts, not just files
// ---------------------------------------------------------------------------

/// One issue's declared constraints on shared concepts — field names, type
/// names, identifier formats — the shared nouns both sides might silently
/// assume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueConcepts {
    pub issue: u64,
    pub concepts: Vec<String>,
}

/// Canonicalize a concept name so `Model_ID`, `model_id`, and `` `model_id` ``
/// are the same concept.
pub fn normalize_concept(raw: &str) -> String {
    raw.trim().trim_matches('`').to_lowercase()
}

/// Two (or more) issues in a batch constrain the same shared concept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConceptCollision {
    pub concept: String,
    pub issues: Vec<u64>,
}

impl ConceptCollision {
    /// Sequencing directive injected into the batch planner.
    pub fn directive(&self) -> String {
        let issues = self
            .issues
            .iter()
            .map(|issue| format!("#{issue}"))
            .collect::<Vec<_>>()
            .join(" and ");
        format!(
            "issues {issues} both constrain the shared concept `{}`; sequence them, or have one define the concept and the other build on it",
            self.concept
        )
    }
}

/// Flag every shared concept constrained by more than one issue in the batch.
///
/// File-overlap prediction misses these: the issues can barely overlap
/// textually and still collide on a shared noun neither one defined.
pub fn detect_concept_collisions(issues: &[IssueConcepts]) -> Vec<ConceptCollision> {
    let mut by_concept: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for issue in issues {
        let mut seen = BTreeSet::new();
        for raw in &issue.concepts {
            let concept = normalize_concept(raw);
            if concept.is_empty() || !seen.insert(concept.clone()) {
                continue;
            }
            by_concept.entry(concept).or_default().push(issue.issue);
        }
    }
    by_concept
        .into_iter()
        .filter(|(_, issues)| issues.len() >= 2)
        .map(|(concept, issues)| ConceptCollision { concept, issues })
        .collect()
}

// ---------------------------------------------------------------------------
// 4. Make the shared concept explicit: record constraints where they are read
// ---------------------------------------------------------------------------

/// The shape of a constraint an implementer introduces on a shared concept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintKind {
    /// A validation rule (regex, matcher, validator call).
    Validation,
    /// The concept must equal some other value (e.g. a filesystem path).
    Equality,
    /// An identifier/format requirement.
    IdentifierFormat,
}

impl ConstraintKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::Equality => "equality",
            Self::IdentifierFormat => "identifier format",
        }
    }
}

/// An added diff line that constrains a shared concept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConceptConstraint {
    pub concept: String,
    pub kind: ConstraintKind,
    pub path: String,
    pub line: usize,
    /// The source line the constraint was detected in.
    pub evidence: String,
}

/// A constraint on a shared concept that is not recorded in the shared
/// contract — invisible to the next implementer, who will contradict it in
/// good faith.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnrecordedConstraint {
    pub constraint: ConceptConstraint,
    /// Retry-prompt directive for the implementer.
    pub directive: String,
}

/// One added diff line, in the form the caller extracted it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedLine {
    pub path: String,
    pub line: usize,
    pub text: String,
}

/// True when `concept` (already normalized) appears in `line` as an
/// identifier token.
fn concept_on_line(line: &str, concept: &str) -> bool {
    line.split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|token| token.eq_ignore_ascii_case(concept))
}

fn classify_constraint(text: &str) -> Option<ConstraintKind> {
    let lower = text.to_lowercase();
    let has_regex_literal = text.contains("\"^") || text.contains("r\"^") || text.contains("'^");
    if has_regex_literal
        || lower.contains("regex")
        || lower.contains("re.compile")
        || lower.contains("is_match")
        || lower.contains("matches(")
        || lower.contains("validate")
    {
        return Some(ConstraintKind::Validation);
    }
    if text.contains("==") || lower.contains(".equals(") || lower.contains("assert_eq") {
        return Some(ConstraintKind::Equality);
    }
    if lower.contains("format") || lower.contains("pattern") {
        return Some(ConstraintKind::IdentifierFormat);
    }
    None
}

/// Scan added diff lines for constraints on the batch's shared concepts.
///
/// `shared_concepts` are the concept names (any casing; backticks allowed)
/// other issues in the batch or the contract define. Detection is a
/// deliberately conservative heuristic: a line must name the concept *and*
/// carry a constraint indicator (regex literal, validator call, equality
/// operator, format/pattern keyword).
pub fn scan_added_lines(
    added_lines: &[AddedLine],
    shared_concepts: &[String],
) -> Vec<ConceptConstraint> {
    let mut constraints = Vec::new();
    for added in added_lines {
        for raw in shared_concepts {
            let concept = normalize_concept(raw);
            if concept.is_empty() || !concept_on_line(&added.text, &concept) {
                continue;
            }
            if let Some(kind) = classify_constraint(&added.text) {
                constraints.push(ConceptConstraint {
                    concept,
                    kind,
                    path: added.path.clone(),
                    line: added.line,
                    evidence: added.text.clone(),
                });
            }
        }
    }
    constraints
}

/// Report the constraints that are not recorded in the shared contract.
///
/// `contract_text` is the body of the file other implementers read
/// (e.g. `CONTRACT.md` with its numbered requirements). A constraint is
/// recorded when its concept appears in that text; with no contract supplied,
/// every constraint is unrecorded — there is nowhere the next implementer
/// would see it.
pub fn find_unrecorded_constraints(
    constraints: &[ConceptConstraint],
    contract_text: Option<&str>,
) -> Vec<UnrecordedConstraint> {
    constraints
        .iter()
        .filter(|constraint| match contract_text {
            Some(contract) => !contract.lines().any(|line| {
                concept_on_line(line, &constraint.concept)
            }),
            None => true,
        })
        .map(|constraint| UnrecordedConstraint {
            directive: format!(
                "record the {} constraint on `{}` introduced at {}:{} in the shared contract (e.g. CONTRACT.md) where other implementers read requirements; a constraint that lives only inside one function is invisible to the next implementer",
                constraint.kind.as_str(),
                constraint.concept,
                constraint.path,
                constraint.line
            ),
            constraint: constraint.clone(),
        })
        .collect()
}
