//! The normalized Definition produced by `autospec define`.
//!
//! The Specification owns WHAT must be true. The Definition is its
//! machine-readable projection: stable `REQ-*` and `AC-*` identifiers with
//! provenance back to the Specification. Nothing downstream may redefine a
//! requirement; only explicit change control may (architectural invariant 1).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::ids::{CriterionId, InitiativeId, RequirementId};
use super::repository::RepositoryId;

/// Where a requirement came from in the Specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// The Specification section heading.
    pub section: String,
    /// An optional anchor within that section.
    #[serde(default)]
    pub anchor: Option<String>,
    /// The 1-based inclusive line span in the source Specification.
    #[serde(default)]
    pub lines: Option<(usize, usize)>,
}

impl Provenance {
    /// Provenance for a whole section.
    pub fn section(section: impl Into<String>) -> Self {
        Self {
            section: section.into(),
            anchor: None,
            lines: None,
        }
    }
}

/// What kind of statement a requirement is.
///
/// Non-goals and constraints are captured so that later phases cannot promote
/// an incidental implementation suggestion into an immutable requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementKind {
    /// Behaviour the system must exhibit.
    Functional,
    /// A bound on how the behaviour may be achieved.
    Constraint,
    /// A property that must hold at all times.
    Invariant,
    /// Something explicitly out of scope.
    NonGoal,
}

impl RequirementKind {
    /// The stable wire name.
    pub fn as_str(&self) -> &'static str {
        match self {
            RequirementKind::Functional => "functional",
            RequirementKind::Constraint => "constraint",
            RequirementKind::Invariant => "invariant",
            RequirementKind::NonGoal => "non_goal",
        }
    }

    /// Whether work may be planned against this kind of statement.
    pub fn is_actionable(&self) -> bool {
        !matches!(self, RequirementKind::NonGoal)
    }
}

/// One acceptance criterion under a requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceCriterion {
    /// Stable identifier.
    pub id: CriterionId,
    /// The criterion text.
    pub statement: String,
    /// Whether the criterion can be checked without human judgement.
    pub objectively_verifiable: bool,
    /// Where in the Specification it came from.
    pub provenance: Provenance,
}

/// One normalized requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requirement {
    /// Stable identifier.
    pub id: RequirementId,
    /// The requirement text.
    pub statement: String,
    /// What kind of statement this is.
    pub kind: RequirementKind,
    /// Acceptance criteria that decide whether it holds.
    #[serde(default)]
    pub acceptance: Vec<AcceptanceCriterion>,
    /// Where in the Specification it came from.
    pub provenance: Provenance,
    /// Repositories the definition suspects are involved, without prescribing how.
    #[serde(default)]
    pub candidate_repositories: Vec<RepositoryId>,
    /// Questions that must be resolved before the requirement is plannable.
    #[serde(default)]
    pub open_questions: Vec<String>,
}

impl Requirement {
    /// Whether at least one acceptance criterion is objectively verifiable.
    pub fn has_verifiable_criterion(&self) -> bool {
        self.acceptance
            .iter()
            .any(|criterion| criterion.objectively_verifiable)
    }

    /// Whether the requirement is ready to be planned against.
    pub fn is_plannable(&self) -> bool {
        self.kind.is_actionable() && self.has_verifiable_criterion() && self.open_questions.is_empty()
    }
}

/// A reason a requirement is not yet plannable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "detail")]
pub enum DefinitionGap {
    /// The requirement has no acceptance criteria at all.
    NoAcceptanceCriteria(RequirementId),
    /// Every acceptance criterion needs human judgement.
    NoObjectiveCriterion(RequirementId),
    /// The definition recorded an unresolved question.
    OpenQuestion(RequirementId, String),
}

impl DefinitionGap {
    /// The requirement the gap belongs to.
    pub fn requirement(&self) -> &RequirementId {
        match self {
            DefinitionGap::NoAcceptanceCriteria(id)
            | DefinitionGap::NoObjectiveCriterion(id)
            | DefinitionGap::OpenQuestion(id, _) => id,
        }
    }
}

/// The versioned, machine-readable Definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
    /// The Initiative this Definition belongs to.
    pub initiative: InitiativeId,
    /// Monotonic version; requirements change only by publishing a new version.
    pub version: u32,
    /// Digest of the Specification this version was normalized from.
    pub spec_digest: String,
    /// Normalized requirements, constraints, invariants, and non-goals.
    #[serde(default)]
    pub requirements: Vec<Requirement>,
    /// Ambiguities the normalizer could not resolve.
    #[serde(default)]
    pub ambiguities: Vec<String>,
}

impl Definition {
    /// An empty Definition for `initiative` at `version`.
    pub fn new(initiative: InitiativeId, version: u32, spec_digest: impl Into<String>) -> Self {
        Self {
            initiative,
            version,
            spec_digest: spec_digest.into(),
            requirements: Vec::new(),
            ambiguities: Vec::new(),
        }
    }

    /// Look up a requirement.
    pub fn requirement(&self, id: &RequirementId) -> Option<&Requirement> {
        self.requirements
            .iter()
            .find(|requirement| &requirement.id == id)
    }

    /// Every requirement identifier, including non-goals.
    pub fn requirement_ids(&self) -> BTreeSet<RequirementId> {
        self.requirements
            .iter()
            .map(|requirement| requirement.id.clone())
            .collect()
    }

    /// Requirements that work may be planned against.
    pub fn actionable_requirements(&self) -> Vec<&Requirement> {
        self.requirements
            .iter()
            .filter(|requirement| requirement.kind.is_actionable())
            .collect()
    }

    /// Statements explicitly declared out of scope.
    pub fn non_goals(&self) -> Vec<&Requirement> {
        self.requirements
            .iter()
            .filter(|requirement| requirement.kind == RequirementKind::NonGoal)
            .collect()
    }

    /// Requirements that are not yet objectively verifiable.
    pub fn gaps(&self) -> Vec<DefinitionGap> {
        let mut gaps = Vec::new();
        for requirement in self.actionable_requirements() {
            if requirement.acceptance.is_empty() {
                gaps.push(DefinitionGap::NoAcceptanceCriteria(requirement.id.clone()));
            } else if !requirement.has_verifiable_criterion() {
                gaps.push(DefinitionGap::NoObjectiveCriterion(requirement.id.clone()));
            }
            for question in &requirement.open_questions {
                gaps.push(DefinitionGap::OpenQuestion(
                    requirement.id.clone(),
                    question.clone(),
                ));
            }
        }
        gaps
    }

    /// Repositories the Definition names as candidates.
    pub fn candidate_repositories(&self) -> BTreeSet<RepositoryId> {
        self.requirements
            .iter()
            .flat_map(|requirement| requirement.candidate_repositories.iter().cloned())
            .collect()
    }

    /// Reject a Definition that cannot be planned against.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        if self.version == 0 {
            problems.push("definition versions start at 1".to_string());
        }
        if self.spec_digest.trim().is_empty() {
            problems.push("a definition must record the specification digest it normalized".to_string());
        }

        let mut seen_requirements = BTreeSet::new();
        let mut seen_criteria = BTreeSet::new();
        for requirement in &self.requirements {
            if !seen_requirements.insert(requirement.id.clone()) {
                problems.push(format!("duplicate requirement id {}", requirement.id));
            }
            if requirement.statement.trim().is_empty() {
                problems.push(format!("{} has an empty statement", requirement.id));
            }
            if requirement.kind == RequirementKind::NonGoal && !requirement.acceptance.is_empty() {
                problems.push(format!(
                    "{} is a non-goal and may not carry acceptance criteria",
                    requirement.id
                ));
            }
            for criterion in &requirement.acceptance {
                if !seen_criteria.insert(criterion.id.clone()) {
                    problems.push(format!("duplicate acceptance criterion id {}", criterion.id));
                }
            }
        }

        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }
}

/// How one Definition version differs from another.
///
/// Replanning is expected to leave this empty: plans change, requirements do
/// not (architectural invariant 10).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionChange {
    /// Requirements present only in the newer version.
    pub added: Vec<RequirementId>,
    /// Requirements present only in the older version.
    pub removed: Vec<RequirementId>,
    /// Requirements whose statement, kind, or acceptance criteria changed.
    pub modified: Vec<RequirementId>,
}

impl DefinitionChange {
    /// Compare two Definition versions.
    pub fn between(previous: &Definition, next: &Definition) -> Self {
        let previous_by_id = previous
            .requirements
            .iter()
            .map(|requirement| (requirement.id.clone(), requirement))
            .collect::<BTreeMap<_, _>>();
        let next_by_id = next
            .requirements
            .iter()
            .map(|requirement| (requirement.id.clone(), requirement))
            .collect::<BTreeMap<_, _>>();

        let mut change = Self::default();
        for (id, requirement) in &next_by_id {
            match previous_by_id.get(id) {
                None => change.added.push(id.clone()),
                Some(before) => {
                    let differs = before.statement != requirement.statement
                        || before.kind != requirement.kind
                        || before.acceptance != requirement.acceptance;
                    if differs {
                        change.modified.push(id.clone());
                    }
                }
            }
        }
        for id in previous_by_id.keys() {
            if !next_by_id.contains_key(id) {
                change.removed.push(id.clone());
            }
        }
        change
    }

    /// Whether the requirements are unchanged.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn initiative() -> InitiativeId {
        InitiativeId::parse("INIT-2026-0042").expect("valid initiative id")
    }

    fn requirement(id: &str, verifiable: bool) -> Requirement {
        Requirement {
            id: RequirementId::parse(id).expect("valid requirement id"),
            statement: format!("{id} must hold"),
            kind: RequirementKind::Functional,
            acceptance: vec![AcceptanceCriterion {
                id: CriterionId::from_sequence(id.trim_start_matches("REQ-").parse().unwrap_or(1), 3),
                statement: "a check".to_string(),
                objectively_verifiable: verifiable,
                provenance: Provenance::section("Acceptance Criteria"),
            }],
            provenance: Provenance::section("Goals"),
            candidate_repositories: Vec::new(),
            open_questions: Vec::new(),
        }
    }

    fn definition(requirements: Vec<Requirement>) -> Definition {
        let mut definition = Definition::new(initiative(), 1, "sha256:spec");
        definition.requirements = requirements;
        definition
    }

    #[test]
    fn a_definition_flags_requirements_without_an_objective_criterion() {
        let definition = definition(vec![requirement("REQ-001", true), requirement("REQ-002", false)]);

        let gaps = definition.gaps();

        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].requirement().as_str(), "REQ-002");
        assert!(matches!(gaps[0], DefinitionGap::NoObjectiveCriterion(_)));
    }

    #[test]
    fn a_requirement_without_acceptance_criteria_is_flagged_separately() {
        let mut bare = requirement("REQ-003", true);
        bare.acceptance.clear();

        let gaps = definition(vec![bare]).gaps();

        assert!(matches!(gaps[0], DefinitionGap::NoAcceptanceCriteria(_)));
    }

    #[test]
    fn open_questions_keep_a_requirement_unplannable() {
        let mut ambiguous = requirement("REQ-004", true);
        ambiguous.open_questions.push("which tenant?".to_string());

        assert!(!ambiguous.is_plannable());
        assert!(matches!(
            definition(vec![ambiguous]).gaps()[0],
            DefinitionGap::OpenQuestion(_, _)
        ));
    }

    #[test]
    fn non_goals_are_excluded_from_actionable_requirements() {
        let mut non_goal = requirement("REQ-005", true);
        non_goal.kind = RequirementKind::NonGoal;
        non_goal.acceptance.clear();
        let definition = definition(vec![requirement("REQ-001", true), non_goal]);

        assert_eq!(definition.actionable_requirements().len(), 1);
        assert_eq!(definition.non_goals().len(), 1);
        assert!(definition.gaps().is_empty());
    }

    #[test]
    fn a_non_goal_may_not_carry_acceptance_criteria() {
        let mut non_goal = requirement("REQ-006", true);
        non_goal.kind = RequirementKind::NonGoal;

        let problems = definition(vec![non_goal]).validate().expect_err("rejected");

        assert!(problems[0].contains("non-goal"), "{problems:?}");
    }

    #[test]
    fn duplicate_requirement_ids_are_rejected() {
        let problems = definition(vec![requirement("REQ-001", true), requirement("REQ-001", true)])
            .validate()
            .expect_err("duplicates are rejected");

        assert!(problems.iter().any(|problem| problem.contains("duplicate requirement id")));
    }

    #[test]
    fn a_replan_that_keeps_the_requirements_reports_no_definition_change() {
        let before = definition(vec![requirement("REQ-001", true)]);
        let after = definition(vec![requirement("REQ-001", true)]);

        assert!(DefinitionChange::between(&before, &after).is_empty());
    }

    #[test]
    fn a_changed_requirement_statement_is_reported_as_modified() {
        let before = definition(vec![requirement("REQ-001", true)]);
        let mut changed = requirement("REQ-001", true);
        changed.statement = "REQ-001 must hold differently".to_string();
        let after = definition(vec![changed, requirement("REQ-002", true)]);

        let change = DefinitionChange::between(&before, &after);

        assert_eq!(change.modified.len(), 1);
        assert_eq!(change.added.len(), 1);
        assert!(change.removed.is_empty());
        assert!(!change.is_empty());
    }
}
