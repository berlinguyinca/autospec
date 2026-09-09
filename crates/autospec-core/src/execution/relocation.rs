//! Relocation policy (#3897): relocating a failing check is a hypothesis.
//!
//! A check fails in placement A. The reflex is to move it to placement B
//! and call it handled. But the move is a hypothesis — "the assertion is
//! satisfiable at B" — and a hypothesis that gets falsified at B is not
//! grounds for a third move to C. It is grounds for concluding the
//! assertion is the problem, not the location.
//!
//! The policy is encoded here as pure, testable primitives. A caller
//! records failures, proposes relocations, and acts on what these
//! functions allow:
//!
//! 1. **A relocation must state its hypothesis.** Moving a
//!    previously-failing check to a new placement must name the property
//!    that makes the assertion satisfiable there ([`CheckRelocation::new`]).
//!    A move with no stated property is a shuffle, not a test.
//! 2. **A falsified hypothesis is not re-run.** A check that has failed in
//!    two distinct placements has had its hypothesis falsified; it cannot
//!    be relocated a third time while it still blocks — it must first be
//!    downgraded to advisory or removed ([`RelocationLedger::relocate`]).
//! 3. **An unsatisfiable assertion is a design issue.** Where a gate
//!    asserts a property the architecture does not provide, no placement
//!    can satisfy it. The only sanctioned response is to file a design
//!    issue and run the gate advisory until the property holds
//!    ([`RelocationLedger::file_design_issue`],
//!    [`RelocationLedger::mark_property_holds`],
//!    [`RelocationLedger::restore_blocking`]).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Why a relocation request was refused.
///
/// Every variant names the state that blocks the move, so the caller can
/// act on it: state the missing property, downgrade the check, or stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelocationError {
    /// The check name is empty or whitespace.
    EmptyCheck,
    /// A source or target placement is empty or whitespace.
    EmptyPlacement,
    /// The stated satisfiability property is empty or whitespace.
    EmptyProperty,
    /// Source and target placement are identical: a check cannot be
    /// relocated to where it already is.
    SamePlacement,
    /// The source placement has no recorded failure for the check. Only a
    /// check that failed where it sits is relocated, and the record must
    /// name the failure that motivated the move.
    NotPreviouslyFailed { check: String, placement: String },
    /// The target placement is one the check has already failed in.
    /// Relocating back to a falsified placement re-runs a falsified
    /// hypothesis.
    FalsifiedPlacement { check: String, placement: String },
    /// The check has failed in at least two distinct placements and has
    /// already been relocated twice. A third relocation requires the
    /// check to be downgraded or removed first.
    RelocationExhausted {
        check: String,
        failed_placements: Vec<String>,
        relocations: u32,
    },
    /// The check has been removed from the gate. Nothing is left to
    /// relocate, downgrade, or file a design issue against.
    CheckRemoved { check: String },
    /// The property was never recorded as unprovided for the check, so
    /// nothing can hold or fail against it.
    UnknownProperty { check: String, property: String },
    /// The check asserts a property the architecture does not provide.
    /// The gate is advisory until the property holds.
    PropertyNotProvided { check: String, property: String },
}

impl fmt::Display for RelocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyCheck => write!(f, "a relocation needs a non-empty check name"),
            Self::EmptyPlacement => write!(f, "a relocation needs non-empty placements"),
            Self::EmptyProperty => write!(
                f,
                "a relocation must state the property that makes the assertion \
                 satisfiable in the new placement"
            ),
            Self::SamePlacement => {
                write!(
                    f,
                    "a relocation needs distinct source and target placements"
                )
            }
            Self::NotPreviouslyFailed { check, placement } => write!(
                f,
                "check {check} has no recorded failure in {placement}: \
                 only a failing check is relocated"
            ),
            Self::FalsifiedPlacement { check, placement } => write!(
                f,
                "check {check} already failed in {placement}: relocating back \
                 to a falsified placement re-runs a falsified hypothesis"
            ),
            Self::RelocationExhausted {
                check,
                failed_placements,
                relocations,
            } => write!(
                f,
                "check {check} has failed in {} distinct placements ({}) and \
                 been relocated {relocations} times: downgrade or remove it \
                 before a third relocation",
                failed_placements.len(),
                failed_placements.join(", ")
            ),
            Self::CheckRemoved { check } => {
                write!(f, "check {check} has been removed from the gate")
            }
            Self::UnknownProperty { check, property } => {
                write!(f, "check {check} asserts no unprovided property {property}")
            }
            Self::PropertyNotProvided { check, property } => write!(
                f,
                "check {check} asserts {property}, which the architecture does \
                 not provide: the gate stays advisory until it holds"
            ),
        }
    }
}

/// A relocation of a previously-failing check from one placement to
/// another, carrying the hypothesis the move is meant to test.
///
/// Construction is fallible on purpose: a relocation without a stated
/// satisfiability property is not representable, so it cannot be
/// accidentally proposed (#3897).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckRelocation {
    check: String,
    from: String,
    to: String,
    satisfiability: String,
}

impl CheckRelocation {
    /// Construct a relocation that states its hypothesis.
    ///
    /// `satisfiability` is the property that makes the assertion
    /// satisfiable in `to` — the claim the move is testing. It must be
    /// stated; a move without it is a shuffle, not a hypothesis.
    pub fn new(
        check: &str,
        from: &str,
        to: &str,
        satisfiability: &str,
    ) -> Result<Self, RelocationError> {
        let check = check.trim();
        let from = from.trim();
        let to = to.trim();
        let satisfiability = satisfiability.trim();
        if check.is_empty() {
            return Err(RelocationError::EmptyCheck);
        }
        if from.is_empty() || to.is_empty() {
            return Err(RelocationError::EmptyPlacement);
        }
        if from == to {
            return Err(RelocationError::SamePlacement);
        }
        if satisfiability.is_empty() {
            return Err(RelocationError::EmptyProperty);
        }
        Ok(Self {
            check: check.to_string(),
            from: from.to_string(),
            to: to.to_string(),
            satisfiability: satisfiability.to_string(),
        })
    }

    /// The check being relocated.
    pub fn check(&self) -> &str {
        &self.check
    }

    /// The placement the check failed in.
    pub fn from(&self) -> &str {
        &self.from
    }

    /// The placement the check is moving to.
    pub fn to(&self) -> &str {
        &self.to
    }

    /// The stated property that makes the assertion satisfiable in the
    /// target placement.
    pub fn satisfiability(&self) -> &str {
        &self.satisfiability
    }

    /// The durable relocation record: what moved, where it failed, where
    /// it goes, and the property that is supposed to hold there. An actor
    /// reading the record later can see the move was a test, and what the
    /// test claimed.
    pub fn line(&self) -> String {
        format!(
            "relocated {}: failed in {}; moved to {} — satisfiable there because {}",
            self.check, self.from, self.to, self.satisfiability
        )
    }
}

/// How a check stands in its gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckDisposition {
    /// Asserting and blocking: a failure holds the gate.
    Active,
    /// Downgraded: still asserting, but failures are advisory and do not
    /// hold the gate.
    Advisory,
    /// Removed from the gate entirely.
    Removed,
}

/// The relocation state of the checks a pipeline runs.
///
/// The ledger remembers, per check: the distinct placements it has failed
/// in, how many times it has been relocated, where it sits now, and how it
/// stands (active, advisory, removed). Rejections are the point — the
/// ledger exists to say no to the third move and to the move that dodges
/// the assertion.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelocationLedger {
    /// Check -> distinct failed placements, first-observed order.
    failed: BTreeMap<String, Vec<String>>,
    /// Check -> completed relocations.
    relocations: BTreeMap<String, u32>,
    /// Check -> current placement, once the ledger has seen it move.
    placement: BTreeMap<String, String>,
    /// Check -> disposition. Absent means [`CheckDisposition::Active`].
    disposition: BTreeMap<String, CheckDisposition>,
    /// Check -> properties the gate asserts that the architecture does not
    /// provide, first-filed order.
    unprovided: BTreeMap<String, Vec<String>>,
    /// Checks with a design issue on file.
    design_issues: BTreeSet<String>,
}

impl RelocationLedger {
    /// An empty ledger: no known failures, moves, or dispositions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `check` failed in `placement`.
    ///
    /// Returns whether the placement was newly recorded: failing twice in
    /// the same placement is one falsified hypothesis, not two, so the
    /// distinct-placement count stays honest.
    pub fn record_failure(
        &mut self,
        check: &str,
        placement: &str,
    ) -> Result<bool, RelocationError> {
        let check = check.trim();
        let placement = placement.trim();
        if check.is_empty() {
            return Err(RelocationError::EmptyCheck);
        }
        if placement.is_empty() {
            return Err(RelocationError::EmptyPlacement);
        }
        let failed = self.failed.entry(check.to_string()).or_default();
        let fresh = !failed.iter().any(|p| p == placement);
        if fresh {
            failed.push(placement.to_string());
        }
        Ok(fresh)
    }

    /// The distinct placements `check` has failed in, first-observed order.
    pub fn failed_placements(&self, check: &str) -> &[String] {
        self.failed.get(check).map(Vec::as_slice).unwrap_or(&[])
    }

    /// How many times `check` has been relocated.
    pub fn relocation_count(&self, check: &str) -> u32 {
        self.relocations.get(check).copied().unwrap_or(0)
    }

    /// Where `check` sits now, if the ledger has seen it move.
    pub fn current_placement(&self, check: &str) -> Option<&str> {
        self.placement.get(check).map(String::as_str)
    }

    /// How `check` stands in its gate. An unknown check is [`Active`]:
    /// blocking is the ordinary state, and the ledger only remembers a
    /// deviation from it.
    pub fn disposition(&self, check: &str) -> CheckDisposition {
        self.disposition
            .get(check)
            .copied()
            .unwrap_or(CheckDisposition::Active)
    }

    /// Whether a failure of `check` may hold the gate.
    ///
    /// Only an active check blocks. An advisory check still asserts — its
    /// findings are recorded — but they report; they do not hold
    /// (#3897: a gate that asserts what the architecture does not provide
    /// must not block).
    pub fn may_block(&self, check: &str) -> bool {
        self.disposition(check) == CheckDisposition::Active
    }

    /// Complete a relocation.
    ///
    /// Refused, and only refused, when the move would violate the
    /// relocation policy: the source has no recorded failure, the target
    /// is a placement the check has already failed in, or the check has
    /// failed in at least two distinct placements and already been
    /// relocated twice while still active. Downgrading or removing the
    /// check first is the escape — the third move is not.
    pub fn relocate(&mut self, relocation: &CheckRelocation) -> Result<(), RelocationError> {
        let check = relocation.check();
        let from = relocation.from();
        let to = relocation.to();
        if self.disposition(check) == CheckDisposition::Removed {
            return Err(RelocationError::CheckRemoved {
                check: check.to_string(),
            });
        }
        let failed = self.failed.get(check).map(Vec::as_slice).unwrap_or(&[]);
        if !failed.iter().any(|p| p == from) {
            return Err(RelocationError::NotPreviouslyFailed {
                check: check.to_string(),
                placement: from.to_string(),
            });
        }
        if failed.iter().any(|p| p == to) {
            return Err(RelocationError::FalsifiedPlacement {
                check: check.to_string(),
                placement: to.to_string(),
            });
        }
        let relocations = self.relocation_count(check);
        if self.disposition(check) == CheckDisposition::Active
            && failed.len() >= 2
            && relocations >= 2
        {
            return Err(RelocationError::RelocationExhausted {
                check: check.to_string(),
                failed_placements: failed.to_vec(),
                relocations,
            });
        }
        self.relocations.insert(check.to_string(), relocations + 1);
        self.placement.insert(check.to_string(), to.to_string());
        Ok(())
    }

    /// Downgrade `check` to advisory: it keeps asserting, its failures
    /// stop holding the gate. The escape for an exhausted check — once
    /// downgraded, it may be relocated again, because the move no longer
    /// costs a blocked pipeline.
    pub fn downgrade_to_advisory(&mut self, check: &str) -> Result<(), RelocationError> {
        let check = check.trim();
        if check.is_empty() {
            return Err(RelocationError::EmptyCheck);
        }
        if self.disposition(check) == CheckDisposition::Removed {
            return Err(RelocationError::CheckRemoved {
                check: check.to_string(),
            });
        }
        self.disposition
            .insert(check.to_string(), CheckDisposition::Advisory);
        Ok(())
    }

    /// Remove `check` from the gate. The other escape for an exhausted
    /// check: the assertion stops running at all.
    pub fn remove(&mut self, check: &str) -> Result<(), RelocationError> {
        let check = check.trim();
        if check.is_empty() {
            return Err(RelocationError::EmptyCheck);
        }
        self.disposition
            .insert(check.to_string(), CheckDisposition::Removed);
        Ok(())
    }

    /// File a design issue for `check`, which asserts `property` that the
    /// architecture does not provide.
    ///
    /// This is the only path by which a property is recorded as
    /// unprovided, and it downgrades the check to advisory in the same
    /// step: a gate that asserts what the architecture does not provide
    /// must be advisory, so the unsound state (asserting, blocking,
    /// unsatisfiable) is not representable (#3897). Returns whether the
    /// property was newly filed.
    pub fn file_design_issue(
        &mut self,
        check: &str,
        property: &str,
    ) -> Result<bool, RelocationError> {
        let check = check.trim();
        let property = property.trim();
        if check.is_empty() {
            return Err(RelocationError::EmptyCheck);
        }
        if property.is_empty() {
            return Err(RelocationError::EmptyProperty);
        }
        if self.disposition(check) == CheckDisposition::Removed {
            return Err(RelocationError::CheckRemoved {
                check: check.to_string(),
            });
        }
        let unprovided = self.unprovided.entry(check.to_string()).or_default();
        let fresh = !unprovided.iter().any(|p| p == property);
        if fresh {
            unprovided.push(property.to_string());
        }
        self.design_issues.insert(check.to_string());
        self.disposition
            .insert(check.to_string(), CheckDisposition::Advisory);
        Ok(fresh)
    }

    /// The properties `check` asserts that the architecture does not
    /// provide, first-filed order.
    pub fn unprovided_properties(&self, check: &str) -> &[String] {
        self.unprovided.get(check).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Whether a design issue is on file for `check`.
    pub fn design_issue_on_file(&self, check: &str) -> bool {
        self.design_issues.contains(check)
    }

    /// The canonical one-line brief for the design issue on file for
    /// `check`, or `None` when no issue is on file.
    pub fn design_issue_brief(&self, check: &str) -> Option<String> {
        if !self.design_issue_on_file(check) {
            return None;
        }
        let properties = self.unprovided_properties(check);
        Some(format!(
            "design issue: gate `{check}` asserts {} which the architecture does not \
             provide; the gate stays advisory until the property holds",
            if properties.is_empty() {
                "a property".to_string()
            } else {
                properties
                    .iter()
                    .map(|p| format!("`{p}`"))
                    .collect::<Vec<_>>()
                    .join(" and ")
            }
        ))
    }

    /// Record that the architecture now provides `property` for `check`.
    ///
    /// The property must be one on file as unprovided: holding is a
    /// resolution of a filed gap, not a fresh claim.
    pub fn mark_property_holds(
        &mut self,
        check: &str,
        property: &str,
    ) -> Result<(), RelocationError> {
        let check = check.trim();
        let property = property.trim();
        if check.is_empty() {
            return Err(RelocationError::EmptyCheck);
        }
        if property.is_empty() {
            return Err(RelocationError::EmptyProperty);
        }
        let unprovided =
            self.unprovided
                .get_mut(check)
                .ok_or_else(|| RelocationError::UnknownProperty {
                    check: check.to_string(),
                    property: property.to_string(),
                })?;
        let before = unprovided.len();
        unprovided.retain(|p| p != property);
        if unprovided.len() == before {
            return Err(RelocationError::UnknownProperty {
                check: check.to_string(),
                property: property.to_string(),
            });
        }
        Ok(())
    }

    /// Restore `check` to blocking.
    ///
    /// Allowed only when the check is advisory and every property it
    /// asserts is held: "advisory until the property holds" means the
    /// downgrade is lifted by the property, not by patience.
    pub fn restore_blocking(&mut self, check: &str) -> Result<(), RelocationError> {
        let check = check.trim();
        if check.is_empty() {
            return Err(RelocationError::EmptyCheck);
        }
        match self.disposition(check) {
            CheckDisposition::Active => {}
            CheckDisposition::Advisory => {
                let unprovided = self.unprovided_properties(check);
                if !unprovided.is_empty() {
                    return Err(RelocationError::PropertyNotProvided {
                        check: check.to_string(),
                        property: unprovided[0].clone(),
                    });
                }
                self.disposition
                    .insert(check.to_string(), CheckDisposition::Active);
            }
            CheckDisposition::Removed => {
                return Err(RelocationError::CheckRemoved {
                    check: check.to_string(),
                })
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reloc(
        check: &str,
        from: &str,
        to: &str,
        property: &str,
    ) -> Result<CheckRelocation, RelocationError> {
        CheckRelocation::new(check, from, to, property)
    }

    /// Fail in `a`, relocate `a` -> `b`, fail in `b`, relocate `b` -> `c`.
    /// The check has now failed in two distinct placements and been
    /// relocated twice: the third move is the one the policy exists to
    /// stop.
    fn two_failures_two_relocations(
        ledger: &mut RelocationLedger,
    ) -> Result<CheckRelocation, RelocationError> {
        ledger.record_failure("gate", "a")?;
        ledger.relocate(&reloc("gate", "a", "b", "p(a)").unwrap())?;
        ledger.record_failure("gate", "b")?;
        let third = reloc("gate", "b", "c", "p(b)");
        ledger.relocate(&third.unwrap())?;
        Ok(reloc("gate", "c", "d", "p(c)")?)
    }

    // ── 1. a relocation must state its hypothesis ────────────────────────

    #[test]
    fn a_relocation_must_state_the_property_that_makes_the_assertion_satisfiable() {
        let missing = reloc("gate", "a", "b", "");
        assert_eq!(missing, Err(RelocationError::EmptyProperty));
        let padded = reloc("gate", "a", "b", "   ");
        assert_eq!(padded, Err(RelocationError::EmptyProperty));

        let stated = reloc(
            "gate",
            "a",
            "b",
            "the patch is evaluated after the full workspace build",
        )
        .unwrap();
        assert_eq!(
            stated.satisfiability(),
            "the patch is evaluated after the full workspace build"
        );
    }

    #[test]
    fn a_relocation_rejects_malformed_moves() {
        assert_eq!(reloc("", "a", "b", "p"), Err(RelocationError::EmptyCheck));
        assert_eq!(
            reloc("gate", "  ", "b", "p"),
            Err(RelocationError::EmptyPlacement)
        );
        assert_eq!(
            reloc("gate", "a", "", "p"),
            Err(RelocationError::EmptyPlacement)
        );
        assert_eq!(
            reloc("gate", "a", "a", "p"),
            Err(RelocationError::SamePlacement)
        );
    }

    #[test]
    fn the_relocation_line_names_the_check_the_failures_and_the_property() {
        let line = reloc("gate", "a", "b", "the build runs first here")
            .unwrap()
            .line();
        for token in ["gate", "a", "b", "the build runs first here"] {
            assert!(line.contains(token), "the record names {token}: {line}");
        }
    }

    // ── 2. a falsified hypothesis is not re-run ───────────────────────────

    #[test]
    fn a_failing_check_is_relocated_with_its_stated_property() {
        let mut ledger = RelocationLedger::new();
        ledger.record_failure("gate", "a").unwrap();
        ledger
            .relocate(&reloc("gate", "a", "b", "the build runs first here").unwrap())
            .unwrap();
        assert_eq!(ledger.relocation_count("gate"), 1);
        assert_eq!(ledger.current_placement("gate"), Some("b"));
    }

    #[test]
    fn the_second_relocation_after_two_failures_is_still_a_hypothesis_test() {
        // The policy forbids the *third* relocation. With one completed
        // move, a second move is still testing a stated property —
        // allowed even while the check blocks.
        let mut ledger = RelocationLedger::new();
        ledger.record_failure("gate", "a").unwrap();
        ledger
            .relocate(&reloc("gate", "a", "b", "p(a)").unwrap())
            .unwrap();
        ledger.record_failure("gate", "b").unwrap();
        ledger
            .relocate(&reloc("gate", "b", "c", "p(b)").unwrap())
            .unwrap();
        assert_eq!(ledger.relocation_count("gate"), 2);
        assert_eq!(ledger.disposition("gate"), CheckDisposition::Active);
    }

    #[test]
    fn the_third_relocation_after_two_failed_placements_is_refused() {
        // The heart of the invariant: the check failed in `a` and `b`,
        // was relocated twice, and the hypothesis — "the assertion is
        // satisfiable somewhere" — is falsified. The third move is
        // refused; the assertion is the problem, not the location.
        let mut ledger = RelocationLedger::new();
        let third = two_failures_two_relocations(&mut ledger).unwrap();
        ledger.record_failure("gate", "c").unwrap();

        let error = ledger.relocate(&third).unwrap_err();
        match &error {
            RelocationError::RelocationExhausted {
                check,
                failed_placements,
                relocations,
            } => {
                assert_eq!(check, "gate");
                assert_eq!(
                    failed_placements,
                    &["a".to_string(), "b".to_string(), "c".to_string()]
                );
                assert_eq!(*relocations, 2);
            }
            other => panic!("expected exhaustion, got {other:?}"),
        }
        assert!(
            error.to_string().contains("downgrade or remove"),
            "{}",
            error
        );
        // The refused move left no trace: nothing moved, nothing counted.
        assert_eq!(ledger.relocation_count("gate"), 2);
        assert_eq!(ledger.current_placement("gate"), Some("c"));
    }

    #[test]
    fn an_exhausted_check_moves_after_it_is_downgraded() {
        // "Unless it is downgraded": the escape is real. Once advisory,
        // the third move no longer costs a blocked pipeline, so the
        // relocation count alone no longer vetoes it.
        let mut ledger = RelocationLedger::new();
        let third = two_failures_two_relocations(&mut ledger).unwrap();
        ledger.record_failure("gate", "c").unwrap();
        assert!(matches!(
            ledger.relocate(&third),
            Err(RelocationError::RelocationExhausted { .. })
        ));

        ledger.downgrade_to_advisory("gate").unwrap();
        ledger.relocate(&third).unwrap();
        assert_eq!(ledger.disposition("gate"), CheckDisposition::Advisory);
        assert_eq!(ledger.current_placement("gate"), Some("d"));
        assert!(!ledger.may_block("gate"));
    }

    #[test]
    fn a_downgraded_check_stays_relocatable_however_far_it_fails() {
        // The downgrade is a standing escape, not a one-move token: an
        // advisory check keeps its freedom to move while it asserts.
        let mut ledger = RelocationLedger::new();
        ledger.downgrade_to_advisory("gate").unwrap();
        for (from, to) in [("a", "b"), ("b", "c"), ("c", "d")] {
            ledger.record_failure("gate", from).unwrap();
            ledger
                .relocate(&reloc("gate", from, to, "still asserting, not blocking").unwrap())
                .unwrap();
            ledger.record_failure("gate", to).unwrap();
        }
        assert_eq!(ledger.relocation_count("gate"), 3);
    }

    #[test]
    fn a_removed_check_cannot_be_relocated_or_downgraded() {
        // "Or removed": removal resolves the problem by ending the
        // assertion. There is nothing left to move.
        let mut ledger = RelocationLedger::new();
        ledger.record_failure("gate", "a").unwrap();
        ledger.remove("gate").unwrap();
        assert_eq!(ledger.disposition("gate"), CheckDisposition::Removed);
        assert!(!ledger.may_block("gate"));

        let error = ledger
            .relocate(&reloc("gate", "a", "b", "p").unwrap())
            .unwrap_err();
        assert_eq!(
            error,
            RelocationError::CheckRemoved {
                check: "gate".into()
            }
        );
        assert_eq!(
            ledger.downgrade_to_advisory("gate"),
            Err(RelocationError::CheckRemoved {
                check: "gate".into()
            })
        );
    }

    #[test]
    fn only_a_failing_check_is_relocated() {
        // A move must be motivated by a recorded failure at the source:
        // the ledger is the record of why the check left.
        let mut ledger = RelocationLedger::new();
        let error = ledger
            .relocate(&reloc("gate", "a", "b", "p").unwrap())
            .unwrap_err();
        assert_eq!(
            error,
            RelocationError::NotPreviouslyFailed {
                check: "gate".into(),
                placement: "a".into()
            }
        );
    }

    #[test]
    fn a_relocation_back_to_a_falsified_placement_is_refused() {
        // `a` falsified the hypothesis already. Moving back to `a` is
        // not a new test; it is re-running the failed one.
        let mut ledger = RelocationLedger::new();
        ledger.record_failure("gate", "a").unwrap();
        ledger
            .relocate(&reloc("gate", "a", "b", "p(a)").unwrap())
            .unwrap();
        ledger.record_failure("gate", "b").unwrap();

        let error = ledger
            .relocate(&reloc("gate", "b", "a", "p(b)").unwrap())
            .unwrap_err();
        assert_eq!(
            error,
            RelocationError::FalsifiedPlacement {
                check: "gate".into(),
                placement: "a".into()
            }
        );
    }

    #[test]
    fn failing_twice_in_one_placement_is_one_falsified_hypothesis() {
        // The exhaustion rule counts distinct failed placements, not
        // failures: two flakes in `a` are one falsified hypothesis, and
        // the record must say so.
        let mut ledger = RelocationLedger::new();
        assert!(ledger.record_failure("gate", "a").unwrap());
        assert!(!ledger.record_failure("gate", "a").unwrap());
        assert_eq!(ledger.failed_placements("gate"), &["a".to_string()]);
        assert_eq!(ledger.relocation_count("gate"), 0);
    }

    #[test]
    fn the_ledger_tracks_failures_per_check() {
        let mut ledger = RelocationLedger::new();
        ledger.record_failure("gate", "a").unwrap();
        ledger.record_failure("lint", "x").unwrap();
        assert_eq!(ledger.failed_placements("gate"), &["a".to_string()]);
        assert_eq!(ledger.failed_placements("lint"), &["x".to_string()]);
        assert!(ledger.failed_placements("docs").is_empty());
    }

    // ── 3. an unsatisfiable assertion is a design issue ───────────────────

    #[test]
    fn a_gate_asserting_an_unprovided_property_becomes_advisory() {
        // No placement can satisfy an assertion the architecture does not
        // provide, so the sanctioned response is: file the design issue,
        // stop blocking. Filing does both in one step — the gate cannot
        // sit in the unsound state (asserting, blocking, unsatisfiable).
        let mut ledger = RelocationLedger::new();
        let fresh = ledger
            .file_design_issue("gate", "the build runs before the patch is evaluated")
            .unwrap();
        assert!(fresh);
        assert_eq!(ledger.disposition("gate"), CheckDisposition::Advisory);
        assert!(!ledger.may_block("gate"));
        assert!(ledger.design_issue_on_file("gate"));
        assert_eq!(
            ledger.unprovided_properties("gate"),
            &["the build runs before the patch is evaluated".to_string()]
        );
    }

    #[test]
    fn the_gate_stays_advisory_until_the_property_holds() {
        let mut ledger = RelocationLedger::new();
        ledger
            .file_design_issue("gate", "the build runs before the patch is evaluated")
            .unwrap();

        // Patience is not a resolution: with the property still unprovided,
        // the check does not restore itself to blocking.
        let error = ledger.restore_blocking("gate").unwrap_err();
        assert_eq!(
            error,
            RelocationError::PropertyNotProvided {
                check: "gate".into(),
                property: "the build runs before the patch is evaluated".into()
            }
        );
        assert!(!ledger.may_block("gate"));

        ledger
            .mark_property_holds("gate", "the build runs before the patch is evaluated")
            .unwrap();
        ledger.restore_blocking("gate").unwrap();
        assert_eq!(ledger.disposition("gate"), CheckDisposition::Active);
        assert!(ledger.may_block("gate"));
    }

    #[test]
    fn a_property_not_on_file_cannot_be_marked_as_holding() {
        let mut ledger = RelocationLedger::new();
        let error = ledger
            .mark_property_holds("gate", "something never filed")
            .unwrap_err();
        assert_eq!(
            error,
            RelocationError::UnknownProperty {
                check: "gate".into(),
                property: "something never filed".into()
            }
        );
    }

    #[test]
    fn every_unprovided_property_must_hold_before_blocking_returns() {
        // "The property holds" means all of them: one filed gap still
        // open keeps the gate advisory.
        let mut ledger = RelocationLedger::new();
        ledger.file_design_issue("gate", "p1").unwrap();
        ledger.file_design_issue("gate", "p2").unwrap();
        ledger.mark_property_holds("gate", "p1").unwrap();

        let error = ledger.restore_blocking("gate").unwrap_err();
        assert_eq!(
            error,
            RelocationError::PropertyNotProvided {
                check: "gate".into(),
                property: "p2".into()
            }
        );
        ledger.mark_property_holds("gate", "p2").unwrap();
        ledger.restore_blocking("gate").unwrap();
        assert!(ledger.may_block("gate"));
    }

    #[test]
    fn filing_the_same_design_issue_twice_files_it_once() {
        let mut ledger = RelocationLedger::new();
        assert!(ledger.file_design_issue("gate", "p").unwrap());
        assert!(!ledger.file_design_issue("gate", "p").unwrap());
        assert_eq!(ledger.unprovided_properties("gate"), &["p".to_string()]);
    }

    #[test]
    fn a_design_issue_cannot_be_filed_for_a_removed_check() {
        // A removed check no longer asserts anything; there is no gate to
        // make advisory.
        let mut ledger = RelocationLedger::new();
        ledger.remove("gate").unwrap();
        let error = ledger.file_design_issue("gate", "p").unwrap_err();
        assert_eq!(
            error,
            RelocationError::CheckRemoved {
                check: "gate".into()
            }
        );
        assert!(!ledger.design_issue_on_file("gate"));
    }

    #[test]
    fn the_design_issue_brief_names_the_check_and_the_property() {
        let mut ledger = RelocationLedger::new();
        assert!(ledger.design_issue_brief("gate").is_none());

        ledger
            .file_design_issue("gate", "the build runs before the patch is evaluated")
            .unwrap();
        let brief = ledger.design_issue_brief("gate").unwrap();
        assert!(brief.contains("gate"), "{brief}");
        assert!(
            brief.contains("the build runs before the patch is evaluated"),
            "{brief}"
        );
        assert!(brief.contains("advisory"), "{brief}");
        assert!(ledger.design_issue_brief("lint").is_none());
    }

    #[test]
    fn empty_names_and_properties_are_refused_everywhere() {
        let mut ledger = RelocationLedger::new();
        assert_eq!(
            ledger.record_failure("  ", "a"),
            Err(RelocationError::EmptyCheck)
        );
        assert_eq!(
            ledger.record_failure("gate", "   "),
            Err(RelocationError::EmptyPlacement)
        );
        assert_eq!(
            ledger.downgrade_to_advisory(""),
            Err(RelocationError::EmptyCheck)
        );
        assert_eq!(ledger.remove("   "), Err(RelocationError::EmptyCheck));
        assert_eq!(
            ledger.file_design_issue("gate", "  "),
            Err(RelocationError::EmptyProperty)
        );
        assert_eq!(
            ledger.mark_property_holds("gate", ""),
            Err(RelocationError::EmptyProperty)
        );
    }
}
