//! Tests for `autospec_core::rule_reach` (issue #4576).
//!
//! A rule that routes work must state the capability it assumes, and the
//! no-hand-patch rule has a scope: in-reach defects are never hand-patched,
//! out-of-reach defects get a guarded hand stopgap plus a companion issue
//! that is labelled unreachable and stays open.

use autospec_core::rule_reach::{
    assumption_findings, audit, handling_findings, stopgap_findings, Disposition, Reach, Rule,
    Stopgap,
};

/// The no-hand-patch rule, parameterised by what it states about its
/// capability.
fn no_hand_patch_rule(states_reach: bool, states_escape_hatch: bool) -> Rule {
    Rule {
        name: "no hand-patching",
        states_reach,
        states_escape_hatch,
    }
}

/// A hand stopgap that satisfies every requirement of the permitted escape
/// hatch.
fn well_formed_stopgap() -> Stopgap {
    Stopgap {
        minimal: true,
        guarded: true,
        reverted_on_failure: true,
        companion_issue: true,
        issue_labelled_unreachable: true,
        issue_closed_on_stopgap: false,
    }
}

// Invariant 1: the rule must state its capability assumption.

#[test]
fn a_rule_that_states_reach_and_escape_hatch_is_clean() {
    let rule = no_hand_patch_rule(true, true);
    assert!(assumption_findings(&rule).is_empty());
}

#[test]
fn a_rule_that_states_no_reach_is_an_unstated_capability() {
    // The #4576 rule: "no hand-patching" assumed every defect is reachable
    // and never scoped itself.
    let rule = no_hand_patch_rule(false, false);
    let findings = assumption_findings(&rule);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("UNSTATED_CAPABILITY"));
}

#[test]
fn a_rule_that_states_reach_but_no_escape_hatch_is_flagged() {
    let rule = no_hand_patch_rule(true, false);
    let findings = assumption_findings(&rule);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("NO_ESCAPE_HATCH"));
}

// Invariant 2: the no-hand-patch rule has a scope.

#[test]
fn an_in_reach_defect_dispatched_to_an_agent_is_clean() {
    assert!(handling_findings(&Reach::InReach, &Disposition::Dispatched).is_empty());
}

#[test]
fn an_in_reach_defect_that_is_hand_patched_is_a_finding() {
    let findings = handling_findings(
        &Reach::InReach,
        &Disposition::HandFix(well_formed_stopgap()),
    );
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("HAND_PATCH_IN_REACH"));
}

#[test]
fn an_out_of_reach_defect_routed_to_an_agent_is_the_incident() {
    // "File an issue and an agent will fix it" for a cluster script no agent
    // checks out: the issue is filed, well evidenced, and inert.
    let findings = handling_findings(&Reach::OutOfReach, &Disposition::Dispatched);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("UNREACHABLE_DISPATCHED"));
}

// Invariant 3: a hand stopgap is well-formed only in a specific shape.

#[test]
fn a_well_formed_stopgap_for_an_out_of_reach_defect_is_clean() {
    let findings = handling_findings(
        &Reach::OutOfReach,
        &Disposition::HandFix(well_formed_stopgap()),
    );
    assert!(findings.is_empty());
}

#[test]
fn each_stopgap_requirement_is_individually_required() {
    let not_minimal = Stopgap {
        minimal: false,
        ..well_formed_stopgap()
    };
    assert!(stopgap_findings(&not_minimal)
        .iter()
        .any(|f| f.starts_with("STOPGAP_NOT_MINIMAL")));

    let not_guarded = Stopgap {
        guarded: false,
        ..well_formed_stopgap()
    };
    assert!(stopgap_findings(&not_guarded)
        .iter()
        .any(|f| f.starts_with("STOPGAP_NOT_GUARDED")));

    let not_reverted = Stopgap {
        reverted_on_failure: false,
        ..well_formed_stopgap()
    };
    assert!(stopgap_findings(&not_reverted)
        .iter()
        .any(|f| f.starts_with("STOPGAP_NOT_REVERTED_ON_FAILURE")));

    let no_issue = Stopgap {
        companion_issue: false,
        ..well_formed_stopgap()
    };
    let findings = stopgap_findings(&no_issue);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("STOPGAP_WITHOUT_ISSUE"));
}

#[test]
fn an_unlabelled_companion_issue_can_be_queued_for_an_agent() {
    let sg = Stopgap {
        issue_labelled_unreachable: false,
        ..well_formed_stopgap()
    };
    let findings = stopgap_findings(&sg);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("STOPGAP_ISSUE_UNLABELLED"));
}

// Invariant 4: the stopgap is not the fix.

#[test]
fn a_companion_issue_that_closes_on_the_stopgap_is_read_as_the_fix() {
    let sg = Stopgap {
        issue_closed_on_stopgap: true,
        ..well_formed_stopgap()
    };
    let findings = stopgap_findings(&sg);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("STOPGAP_READ_AS_FIX"));
}

// The combined audit: the incident end-to-end.

#[test]
fn the_4576_incident_is_flagged_on_the_rule_and_the_handling() {
    // The rule stated no reach; the defect is out of reach; it was routed to
    // an agent that cannot act on it.
    let rule = no_hand_patch_rule(false, false);
    let findings = audit(&rule, &Reach::OutOfReach, &Disposition::Dispatched);
    assert!(findings
        .iter()
        .any(|f| f.starts_with("UNSTATED_CAPABILITY")));
    assert!(findings
        .iter()
        .any(|f| f.starts_with("UNREACHABLE_DISPATCHED")));
}

#[test]
fn a_scoped_rule_handling_an_out_of_reach_defect_with_a_stopgap_is_clean() {
    let rule = no_hand_patch_rule(true, true);
    let findings = audit(
        &rule,
        &Reach::OutOfReach,
        &Disposition::HandFix(well_formed_stopgap()),
    );
    assert!(findings.is_empty());
}

#[test]
fn a_scoped_rule_handling_an_in_reach_defect_by_dispatch_is_clean() {
    let rule = no_hand_patch_rule(true, true);
    let findings = audit(&rule, &Reach::InReach, &Disposition::Dispatched);
    assert!(findings.is_empty());
}
