//! Verdict validity: a recorded verdict is a statement about a tree, not a
//! standing property of a patch (issue #4031).
//!
//! The runner grades a patch and records a verdict per patch; the converter
//! short-circuits on the recorded failing verdict instead of re-running the
//! suite. A verdict is only valid while the conditions it was graded under
//! hold. In the failpoint incident (#4027), a recorded failing verdict
//! outlived the fix of the systemic cause that produced it: the cause was
//! corrected on main, but the converter kept discarding patches on a verdict
//! that had been graded against a tree in which the tests still failed. The
//! shipped workaround trusted only verdicts recorded *after* the fix, by
//! wall clock — a date, not a condition.
//!
//! The general rule replaces the date with the conditions themselves:
//!
//! 1. **A verdict records its validity conditions.** Every recorded verdict
//!    names the commit of the tree it was graded against and the hash of
//!    the failing baseline it used ([`RecordedVerdict`], [`baseline_hash`]).
//! 2. **A verdict whose commit does not match the current tree is stale.**
//!    Staleness is a routing decision, not a trust decision: the only thing
//!    a stale verdict may do is mark the patch for re-verification. It must
//!    never be the reason a patch is discarded or retired ([`route`]).
//! 3. **A verdict whose failing baseline has moved is stale.** A verdict
//!    that named a failing test which has since been fixed on main carries
//!    a different baseline hash than the current tree; the patch goes back
//!    to the gate instead of being dropped ([`StaleReason::BaselineDrift`],
//!    [`fixed_causes`]).
//! 4. **Unverifiable is fail-closed.** When the commit cannot be read — the
//!    verdict recorded none, or the current tree's commit cannot be read —
//!    discard and retire refuse to operate on the patch and report why the
//!    verdict cannot be verified ([`guard_destructive`]).
//!
//! Nothing here uses wall-clock time as a trust condition.
//! [`RecordedVerdict::recorded_at`] is kept for audit only.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Deterministic hash of a failing baseline: lowercase hex SHA-256 over the
/// test names in sorted order, one per line. [`BTreeSet`] iterates in sorted
/// order, so the hash is stable across runs and platforms for one set.
pub fn baseline_hash(baseline: &BTreeSet<String>) -> String {
    let mut hasher = Sha256::new();
    for name in baseline {
        hasher.update(name.as_bytes());
        hasher.update(b"\n");
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A verdict the runner recorded for one patch, as it must exist on disk:
/// the decision plus the two validity conditions it was graded under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedVerdict {
    /// The patch this verdict was recorded for.
    pub patch_identity: String,
    /// The decision token, e.g. `new-test-failures` or `pass`.
    pub verdict: String,
    /// The failing tests the verdict named, at grading time.
    pub failing_tests: BTreeSet<String>,
    /// The commit of the tree the verdict was graded against. `None` means
    /// the verdict was recorded without it — the verdict is then
    /// unverifiable, never trusted.
    pub tree_commit: Option<String>,
    /// [`baseline_hash`] of the failing baseline used at grading time.
    pub baseline_hash: Option<String>,
    /// Unix epoch seconds. Audit only — never a trust condition.
    pub recorded_at: i64,
}

/// Encode a recorded verdict for persistence.
pub fn encode(verdict: &RecordedVerdict) -> String {
    serde_json::to_string(verdict).expect("RecordedVerdict is serializable")
}

/// Decode a recorded verdict. Fields the record omits come back `None`, and
/// a verdict that recorded no `tree_commit` is unverifiable by construction
/// ([`verify`]).
pub fn decode(text: &str) -> Result<RecordedVerdict, String> {
    serde_json::from_str(text).map_err(|error| format!("invalid recorded verdict: {error}"))
}

/// The state of the tree a recorded verdict is being checked against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CurrentTree {
    /// The current tree's commit. `None` means it could not be read — the
    /// verdict is then unverifiable, fail-closed.
    pub commit: Option<String>,
    /// The failing tests on the current tree (the failing baseline).
    pub failing_baseline: BTreeSet<String>,
}

/// Why a verdict no longer holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StaleReason {
    /// The verdict was graded against a different commit than the current
    /// tree.
    TreeCommitDrift {
        /// The commit the verdict was graded against.
        recorded: String,
        /// The current tree's commit.
        current: String,
    },
    /// The failing baseline moved since grading: a failing test was fixed
    /// on main, or a new test started failing. `changed` names the tests
    /// whose failing status differs between the recorded baseline and the
    /// current one (symmetric difference), sorted.
    BaselineDrift {
        /// The baseline hash the verdict recorded.
        recorded: String,
        /// The current failing baseline's hash.
        current: String,
        /// Test names whose failing status changed, sorted.
        changed: Vec<String>,
    },
}

impl StaleReason {
    /// The report line.
    pub fn message(&self) -> String {
        match self {
            StaleReason::TreeCommitDrift { recorded, current } => {
                format!("stale: graded at commit {recorded}, tree is now at {current}")
            }
            StaleReason::BaselineDrift {
                recorded,
                current,
                changed,
            } => {
                let changed = changed.join(", ");
                format!(
                    "stale: failing baseline moved (recorded {recorded}, now {current}); \
                     tests whose failing status changed: {changed}"
                )
            }
        }
    }
}

/// The validity of a recorded verdict against the current tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VerdictValidity {
    /// Both conditions hold: the verdict was graded against the current
    /// commit with the current failing baseline.
    Fresh,
    /// One or more conditions no longer hold. A stale verdict may only mark
    /// the patch for re-verification.
    Stale(Vec<StaleReason>),
    /// The validity conditions cannot be checked at all: the verdict
    /// recorded no commit, or the current commit could not be read.
    Unverifiable(String),
}

impl VerdictValidity {
    /// Whether the verdict may be trusted for its recorded decision.
    pub fn is_trustworthy(&self) -> bool {
        matches!(self, VerdictValidity::Fresh)
    }
}

/// Check a recorded verdict against the current tree.
///
/// Order matters and is fixed: an unreadable commit is [`VerdictValidity::Unverifiable`]
/// and stops the check — drift cannot even be computed against a tree whose
/// commit is unknown. Otherwise every condition that no longer holds is
/// reported; a verdict is stale for *all* of its broken conditions, not
/// just the first.
pub fn verify(verdict: &RecordedVerdict, current: &CurrentTree) -> VerdictValidity {
    // Presence first, comparison second: a condition that was never recorded
    // (or cannot be read) makes the verdict unverifiable, full stop — and the
    // check stops there, because no drift claim could be backed.
    let recorded_commit =
        match verdict.tree_commit.as_deref() {
            Some(commit) if !commit.trim().is_empty() => commit.to_string(),
            _ => return VerdictValidity::Unverifiable(
                "verdict recorded no tree commit; it cannot be verified against the current tree"
                    .to_string(),
            ),
        };
    let current_commit = match current.commit.as_deref() {
        Some(commit) if !commit.trim().is_empty() => commit.to_string(),
        _ => {
            return VerdictValidity::Unverifiable(
                "the current tree's commit could not be read; the verdict cannot be verified"
                    .to_string(),
            )
        }
    };
    let recorded_hash = match verdict.baseline_hash.as_deref() {
        Some(hash) if !hash.trim().is_empty() => hash.to_string(),
        _ => {
            return VerdictValidity::Unverifiable(
                "verdict recorded no failing-baseline hash; it cannot be verified against the \
                 current tree"
                    .to_string(),
            )
        }
    };

    let mut reasons = Vec::new();
    if recorded_commit != current_commit {
        reasons.push(StaleReason::TreeCommitDrift {
            recorded: recorded_commit,
            current: current_commit,
        });
    }
    let current_hash = baseline_hash(&current.failing_baseline);
    if recorded_hash != current_hash {
        let mut changed: Vec<String> = Vec::new();
        for test in verdict
            .failing_tests
            .iter()
            .chain(current.failing_baseline.iter())
        {
            let was_failing = verdict.failing_tests.contains(test);
            let is_failing = current.failing_baseline.contains(test);
            if was_failing != is_failing {
                changed.push(test.clone());
            }
        }
        changed.sort();
        changed.dedup();
        reasons.push(StaleReason::BaselineDrift {
            recorded: recorded_hash,
            current: current_hash,
            changed,
        });
    }

    if reasons.is_empty() {
        VerdictValidity::Fresh
    } else {
        VerdictValidity::Stale(reasons)
    }
}

/// What the converter does with a patch whose verdict has been checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PatchRoute {
    /// The verdict is fresh and may be honored as recorded.
    Trust,
    /// The verdict is stale or unverifiable: the patch goes back to the
    /// gate for re-verification. Never discard, never retire.
    Reverify(String),
}

/// Route a patch from its checked verdict.
///
/// The only trustworthy path is [`PatchRoute::Trust`]. Stale and
/// unverifiable verdicts both route to [`PatchRoute::Reverify`]: staleness
/// can only mark the patch for re-verification, and an unverifiable verdict
/// is no verdict at all.
pub fn route(verdict: &RecordedVerdict, current: &CurrentTree) -> PatchRoute {
    match verify(verdict, current) {
        VerdictValidity::Fresh => PatchRoute::Trust,
        VerdictValidity::Stale(reasons) => PatchRoute::Reverify(
            reasons
                .iter()
                .map(StaleReason::message)
                .collect::<Vec<_>>()
                .join("; "),
        ),
        VerdictValidity::Unverifiable(reason) => {
            PatchRoute::Reverify(format!("unverifiable: {reason}"))
        }
    }
}

/// The failing tests the verdict named that no longer fail on the current
/// tree — the systemic causes that were fixed on main since grading.
///
/// A non-empty result means the verdict's failing cause is gone from the
/// tree it is being checked against: the patch must be re-verified against
/// the new tree, and the pre-fix verdict is not trusted.
pub fn fixed_causes(verdict: &RecordedVerdict, current: &CurrentTree) -> Vec<String> {
    verdict
        .failing_tests
        .iter()
        .filter(|test| !current.failing_baseline.contains(*test))
        .cloned()
        .collect()
}

/// Guard a destructive action (`"discard"` or `"retire"`) on a patch whose
/// recorded verdict has been checked.
///
/// The action proceeds only on a fresh verdict. A stale verdict refuses the
/// action and names every broken condition (the patch goes back to the
/// gate instead). An unverifiable verdict refuses the action and reports
/// why the verdict cannot be verified. Either way the patch is not touched.
pub fn guard_destructive(
    action: &str,
    verdict: &RecordedVerdict,
    current: &CurrentTree,
) -> Result<(), String> {
    match verify(verdict, current) {
        VerdictValidity::Fresh => Ok(()),
        VerdictValidity::Stale(reasons) => Err(format!(
            "refuse to {action} patch {}: verdict is stale; {}",
            verdict.patch_identity,
            reasons
                .iter()
                .map(StaleReason::message)
                .collect::<Vec<_>>()
                .join("; ")
        )),
        VerdictValidity::Unverifiable(reason) => Err(format!(
            "refuse to {action} patch {}: cannot verify the verdict — {reason}",
            verdict.patch_identity
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    fn verdict(commit: Option<&str>, baseline: &BTreeSet<String>) -> RecordedVerdict {
        RecordedVerdict {
            patch_identity: "issue-42/0001-fix.patch".to_string(),
            verdict: "new-test-failures".to_string(),
            failing_tests: set(&["crate_a::tests::broken"]),
            tree_commit: commit.map(|commit| commit.to_string()),
            baseline_hash: Some(baseline_hash(baseline)),
            recorded_at: 1_700_000_000,
        }
    }

    fn tree(commit: Option<&str>, baseline: &BTreeSet<String>) -> CurrentTree {
        CurrentTree {
            commit: commit.map(|commit| commit.to_string()),
            failing_baseline: baseline.clone(),
        }
    }

    #[test]
    fn baseline_hash_is_deterministic_and_content_sensitive() {
        let a = set(&["b::test", "a::test"]);
        let same = set(&["a::test", "b::test"]);
        let different = set(&["a::test", "b::other"]);
        assert_eq!(baseline_hash(&a), baseline_hash(&same));
        assert_ne!(baseline_hash(&a), baseline_hash(&different));
        assert_ne!(baseline_hash(&a), baseline_hash(&BTreeSet::new()));
        assert_eq!(baseline_hash(&a).len(), 64);
    }

    #[test]
    fn encode_decode_round_trip_preserves_validity_fields() {
        let baseline = set(&["crate_a::tests::broken"]);
        let original = verdict(Some("abc123"), &baseline);
        let decoded = decode(&encode(&original)).expect("decodes");
        assert_eq!(decoded, original);
        assert_eq!(decoded.tree_commit.as_deref(), Some("abc123"));
        assert_eq!(
            decoded.baseline_hash.as_deref(),
            Some(baseline_hash(&baseline).as_str())
        );
    }

    #[test]
    fn fresh_verdict_is_trustworthy_and_trusted() {
        let baseline = set(&["crate_a::tests::broken"]);
        let v = verdict(Some("abc123"), &baseline);
        let current = tree(Some("abc123"), &baseline);
        assert_eq!(verify(&v, &current), VerdictValidity::Fresh);
        assert!(verify(&v, &current).is_trustworthy());
        assert_eq!(route(&v, &current), PatchRoute::Trust);
    }

    #[test]
    fn commit_drift_is_stale_and_routes_to_reverify_not_discard() {
        let baseline = set(&["crate_a::tests::broken"]);
        let v = verdict(Some("abc123"), &baseline);
        let current = tree(Some("def456"), &baseline);
        match verify(&v, &current) {
            VerdictValidity::Stale(reasons) => {
                assert_eq!(
                    reasons,
                    vec![StaleReason::TreeCommitDrift {
                        recorded: "abc123".to_string(),
                        current: "def456".to_string(),
                    }]
                );
            }
            other => panic!("expected stale, got {other:?}"),
        }
        match route(&v, &current) {
            PatchRoute::Reverify(reason) => {
                assert!(reason.contains("abc123") && reason.contains("def456"));
            }
            other => panic!("a stale verdict must route to re-verification, got {other:?}"),
        }
        assert!(
            guard_destructive("discard", &v, &current).is_err(),
            "a stale verdict must not authorize discarding the patch"
        );
    }

    #[test]
    fn a_failing_test_fixed_on_main_is_stale_and_named() {
        let recorded_baseline = set(&["crate_a::tests::broken", "crate_b::tests::still_failing"]);
        let mut v = verdict(Some("abc123"), &recorded_baseline);
        v.failing_tests = recorded_baseline.clone();
        // main moved: the broken test was fixed, the other still fails.
        let current_baseline = set(&["crate_b::tests::still_failing"]);
        let current = tree(Some("abc123"), &current_baseline);
        match verify(&v, &current) {
            VerdictValidity::Stale(reasons) => match &reasons[0] {
                StaleReason::BaselineDrift { changed, .. } => {
                    assert_eq!(changed, &["crate_a::tests::broken".to_string()]);
                }
                other => panic!("expected baseline drift, got {other:?}"),
            },
            other => panic!("expected stale, got {other:?}"),
        }
        assert_eq!(
            fixed_causes(&v, &current),
            vec!["crate_a::tests::broken".to_string()],
            "the fixed failing test is the systemic cause that was fixed on main"
        );
        match route(&v, &current) {
            PatchRoute::Reverify(reason) => {
                assert!(
                    reason.contains("crate_a::tests::broken"),
                    "the report names the changed test: {reason}"
                );
            }
            other => panic!("the patch must go back to the gate, got {other:?}"),
        }
        assert!(
            guard_destructive("retire", &v, &current).is_err(),
            "a verdict whose named cause was fixed must not authorize retiring the patch"
        );
    }

    #[test]
    fn both_drifts_are_reported_together() {
        let recorded_baseline = set(&["t::a"]);
        let v = verdict(Some("abc123"), &recorded_baseline);
        let current = tree(Some("def456"), &set(&["t::b"]));
        match verify(&v, &current) {
            VerdictValidity::Stale(reasons) => {
                assert_eq!(reasons.len(), 2);
                assert!(matches!(reasons[0], StaleReason::TreeCommitDrift { .. }));
                assert!(matches!(reasons[1], StaleReason::BaselineDrift { .. }));
            }
            other => panic!("expected both drifts, got {other:?}"),
        }
    }

    #[test]
    fn a_verdict_without_a_commit_is_unverifiable_and_refuses_discard() {
        let baseline = set(&["crate_a::tests::broken"]);
        let v = verdict(None, &baseline);
        let current = tree(Some("abc123"), &baseline);
        match verify(&v, &current) {
            VerdictValidity::Unverifiable(reason) => {
                assert!(reason.contains("no tree commit"), "reason: {reason}");
            }
            other => panic!("expected unverifiable, got {other:?}"),
        }
        let error = guard_destructive("discard", &v, &current).expect_err("must refuse");
        assert!(error.contains("refuse to discard"), "{error}");
        assert!(error.contains("cannot verify"), "{error}");
        match route(&v, &current) {
            PatchRoute::Reverify(reason) => assert!(reason.contains("unverifiable"), "{reason}"),
            other => panic!("unverifiable routes to re-verification, got {other:?}"),
        }
    }

    #[test]
    fn an_unreadable_current_commit_refuses_retire_and_says_why() {
        let baseline = set(&["crate_a::tests::broken"]);
        let v = verdict(Some("abc123"), &baseline);
        let current = tree(None, &baseline);
        match verify(&v, &current) {
            VerdictValidity::Unverifiable(reason) => {
                assert!(reason.contains("could not be read"), "reason: {reason}");
            }
            other => panic!("expected unverifiable, got {other:?}"),
        }
        let error = guard_destructive("retire", &v, &current).expect_err("must refuse");
        assert!(error.contains("refuse to retire"), "{error}");
        assert!(error.contains("could not be read"), "{error}");
    }

    #[test]
    fn a_verdict_without_a_baseline_hash_is_unverifiable() {
        let baseline = set(&["crate_a::tests::broken"]);
        let mut v = verdict(Some("abc123"), &baseline);
        v.baseline_hash = None;
        let current = tree(Some("abc123"), &baseline);
        assert!(
            matches!(verify(&v, &current), VerdictValidity::Unverifiable(_)),
            "a verdict that recorded no baseline hash cannot be verified"
        );
        assert!(guard_destructive("discard", &v, &current).is_err());
    }

    #[test]
    fn a_pre_fix_verdict_is_not_trusted_after_the_cause_is_fixed() {
        // The systemic cause (a failpoint) made one test fail. The verdict
        // was recorded while it failed; main then fixed the cause.
        let pre_fix = set(&["executor_bridge::tests::failpoint"]);
        let mut v = verdict(Some("pre-fix-commit"), &pre_fix);
        v.failing_tests = pre_fix.clone();
        let post_fix = CurrentTree {
            commit: Some("post-fix-commit".to_string()),
            failing_baseline: BTreeSet::new(),
        };
        assert!(!verify(&v, &post_fix).is_trustworthy());
        assert_eq!(
            fixed_causes(&v, &post_fix),
            vec!["executor_bridge::tests::failpoint".to_string()]
        );
        assert!(
            matches!(route(&v, &post_fix), PatchRoute::Reverify(_)),
            "the patch is re-verified against the new tree"
        );
    }

    #[test]
    fn decoded_verdict_missing_commit_field_is_unverifiable() {
        let text = r#"{"patch_identity":"issue-42/0001","verdict":"new-test-failures","failing_tests":[],"recorded_at":1}"#;
        let v = decode(text).expect("decodes — omitted fields come back None");
        assert!(v.tree_commit.is_none() && v.baseline_hash.is_none());
        assert!(
            matches!(
                verify(&v, &CurrentTree::default()),
                VerdictValidity::Unverifiable(_)
            ),
            "legacy records without validity fields fail closed"
        );
    }
}
