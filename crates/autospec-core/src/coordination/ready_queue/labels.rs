pub(super) const SERIAL_LABELS: &[&str] = &[
    "reasoning:deep",
    "priority:high",
    "regression",
    "audit",
    "release",
];

/// The labels that gate readiness. Each one names a *state* an issue can
/// leave: a draft is classified, a proposal is groomed, an outstanding decision
/// is answered, a prerequisite is met. A label that gates readiness must be
/// removable — one that can never be removed is equivalent to deleting the
/// issue from the plan (#4475).
pub(super) const BLOCKING_LABELS: &[(&str, &str)] = &[
    ("needs-classify", "needs_classify"),
    ("groom:proposed", "groom_proposed"),
    ("autospec:needs-human", "autospec_needs_human"),
    (
        "autospec:blocked-prerequisite",
        "security_prerequisite_blocked",
    ),
];

/// Permanent tier and class labels. They describe *what kind* of work an issue
/// is, not a state it can leave: `tier-review` marks work that needs human
/// review at execution (how it runs, not whether it is ready) and `gate` marks
/// a phase-completion gate. Because they cannot be removed, they must never
/// appear in the readiness predicate — a permanent label there is equivalent to
/// deleting the issue from the plan (#4475).
pub(super) const TIER_CLASS_LABELS: &[&str] = &["tier-review", "gate"];

/// #4475 invariant: a label that gates dispatch must express a condition that
/// can become false. Returns the permanent (tier/class) labels present among
/// the given readiness-predicate labels. An empty result means every predicate
/// label is removable; a non-empty result is a defect — a permanent label in the
/// readiness predicate deletes the issue from the plan regardless of dependency
/// state.
pub(super) fn permanent_labels_in_predicate<'a>(predicate: &[&'a str]) -> Vec<&'a str> {
    predicate
        .iter()
        .copied()
        .filter(|label| TIER_CLASS_LABELS.contains(label))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn predicate_labels() -> Vec<&'static str> {
        BLOCKING_LABELS.iter().map(|(label, _)| *label).collect()
    }

    #[test]
    fn readiness_predicate_has_no_permanent_label() {
        // #4475: every label that gates readiness must be removable. A
        // permanent (tier/class) label in the predicate would delete the issue
        // from the plan regardless of dependency state.
        assert!(
            permanent_labels_in_predicate(&predicate_labels()).is_empty(),
            "readiness predicate contains a permanent label: {:?}",
            permanent_labels_in_predicate(&predicate_labels())
        );
    }

    #[test]
    fn a_permanent_label_in_the_predicate_is_reported() {
        // #4475: the check is what catches a regression if a tier/class label
        // is ever added to the readiness predicate.
        let predicate = ["auto-implement", "tier-review", "gate"];
        assert_eq!(
            permanent_labels_in_predicate(&predicate),
            vec!["tier-review", "gate"]
        );
    }

    #[test]
    fn removable_decision_labels_are_not_flagged() {
        // autospec:needs-human is a *decision* label (removable when answered),
        // not a permanent tier/class label, so it does not trip the check.
        let predicate = ["auto-implement", "autospec:needs-human"];
        assert!(permanent_labels_in_predicate(&predicate).is_empty());
    }
}
