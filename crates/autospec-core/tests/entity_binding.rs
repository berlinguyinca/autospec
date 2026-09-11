//! Regression tests for `autospec_core::entity_binding` (issue #4264).
//!
//! The tests run in the configuration the bug required: the seven
//! measurements from the session, each bound to the wrong entity. Each
//! case is reconstructed as a claim and audited, and each produces its
//! finding. On a claim whose measurement is bound to the entity the
//! conclusion is about, the audit is empty — the control case that
//! proves the checks are not false-positiving.

use autospec_core::entity_binding::{
    action_gate, audit, is_current, name_selection, untrusted_checks, Action, ActionGate,
    Attribution, BindingFinding, Claim, Discrimination, HealthCheck, NameSelection,
};

const NOW: u64 = 1_000_000;
const WINDOW: u64 = 3600; // one hour: current within it, stale beyond it

/// A clean claim: every question answered, the audit is empty. This is
/// the control case — a claim whose measurement is bound to the entity
/// the conclusion is about.
fn clean_claim() -> Claim {
    Claim {
        entity: "gateway@edge".to_string(),
        instance: Some("gateway@edge".to_string()),
        value: "served_503=12".to_string(),
        value_if_false: Some("served_503=0".to_string()),
        name: "gateway@edge".to_string(),
        selected: vec!["gateway@edge".to_string()],
        taken_at: Some(NOW - 60),
        now: NOW,
        window_secs: WINDOW,
    }
}

#[test]
fn clean_claim_has_no_findings() {
    let claim = clean_claim();
    assert_eq!(claim.attribution(), Attribution::Bound);
    assert!(audit(&claim).is_empty());
}

/// Case 1: `qwen3.8-27b-*` endpoint files — believed to describe the
/// 27b workers, actually 27b plus vision. The fleet was miscounted 6/24
/// instead of 5/20.
#[test]
fn case_1_prefix_glob_selects_two_worker_families() {
    let claim = Claim {
        entity: "qwen3.8-27b workers".to_string(),
        instance: Some("qwen3.8-27b workers".to_string()),
        value: "6 workers / 24 slots".to_string(),
        value_if_false: None,
        name: "qwen3.8-27b-*".to_string(),
        selected: vec![
            "qwen3.8-27b-01".to_string(),
            "qwen3.8-27b-02".to_string(),
            "qwen3.8-27b-vision-01".to_string(),
            "qwen3.8-27b-vision-02".to_string(),
            "qwen3.8-27b-vision-03".to_string(),
        ],
        taken_at: Some(NOW - 60),
        now: NOW,
        window_secs: WINDOW,
    };
    let findings = audit(&claim);
    assert!(
        matches!(
            findings.as_slice(),
            [BindingFinding::AmbiguousName { name, selected }]
            if name == "qwen3.8-27b-*" && selected.len() == 5
        ),
        "{findings:?}"
    );
}

/// Case 2: `$LLM/*/out/issue-*` — believed to be autospec patches,
/// actually four projects with colliding issue numbers.
#[test]
fn case_2_glob_spans_four_projects() {
    let claim = Claim {
        entity: "autospec patches".to_string(),
        instance: Some("autospec patches".to_string()),
        value: "4 patches".to_string(),
        value_if_false: None,
        name: "$LLM/*/out/issue-*".to_string(),
        selected: vec![
            "autospec/out/issue-1".to_string(),
            "iw/out/issue-1".to_string(),
            "disp/out/issue-1".to_string(),
            "orch/out/issue-1".to_string(),
        ],
        taken_at: Some(NOW - 60),
        now: NOW,
        window_secs: WINDOW,
    };
    let findings = audit(&claim);
    assert!(
        findings.iter().any(
            |f| matches!(f, BindingFinding::AmbiguousName { selected, .. } if selected.len() == 4)
        ),
        "{findings:?}"
    );
}

/// Case 3: hive gateway `served_503=0` — believed to describe the
/// gateway users hit; users hit the edge instance.
#[test]
fn case_3_measurement_bound_to_the_other_instance() {
    let claim = Claim {
        entity: "gateway@edge".to_string(),
        instance: Some("gateway@hive".to_string()),
        value: "served_503=0".to_string(),
        value_if_false: None,
        name: "gateway".to_string(),
        selected: vec!["gateway".to_string()],
        taken_at: Some(NOW - 60),
        now: NOW,
        window_secs: WINDOW,
    };
    assert_eq!(claim.attribution(), Attribution::WrongInstance);
    let findings = audit(&claim);
    assert!(
        matches!(
            findings.as_slice(),
            [BindingFinding::WrongInstance { instance, entity }]
            if instance == "gateway@hive" && entity == "gateway@edge"
        ),
        "{findings:?}"
    );
}

/// Case 4: "a gateway is running" — a different program in a different
/// repo, with the same name. This one nearly closed a correct issue.
#[test]
fn case_4_no_instance_named_for_the_component() {
    let claim = Claim {
        entity: "InferWeave gateway (InferWeave/inferweave, Rust)".to_string(),
        instance: None, // "a gateway is running" — which one?
        value: "running".to_string(),
        value_if_false: None,
        name: "gateway".to_string(),
        selected: vec![
            "InferWeave/inferweave gateway (Rust, unimplemented)".to_string(),
            "metabolomics-us/inferweave-gateway (Go, production)".to_string(),
        ],
        taken_at: Some(NOW - 60),
        now: NOW,
        window_secs: WINDOW,
    };
    let findings = audit(&claim);
    assert_eq!(findings.len(), 2, "{findings:?}");
    assert!(
        findings.contains(&BindingFinding::Unattributed),
        "{findings:?}"
    );
    assert!(
        matches!(
            &findings[1],
            BindingFinding::AmbiguousName { selected, .. } if selected.len() == 2
        ),
        "{findings:?}"
    );
}

/// Case 5: `cargo` at 0.3% CPU — believed to be the build, actually the
/// supervisor, idle by design, while its child burned 34.8%.
#[test]
fn case_5_parent_process_measured_for_the_child() {
    let claim = Claim {
        entity: "the conversion build".to_string(),
        instance: Some("cargo (supervisor)".to_string()),
        value: "cpu=0.3%".to_string(),
        value_if_false: Some("cpu=0.3%".to_string()), // idle by design, wedged or not
        name: "cargo".to_string(),
        selected: vec!["cargo (supervisor)".to_string()],
        taken_at: Some(NOW - 60),
        now: NOW,
        window_secs: WINDOW,
    };
    let findings = audit(&claim);
    assert_eq!(findings.len(), 2, "{findings:?}");
    assert!(
        matches!(
            &findings[0],
            BindingFinding::WrongInstance { instance, .. } if instance == "cargo (supervisor)"
        ),
        "{findings:?}"
    );
    assert!(
        findings.contains(&BindingFinding::Indistinguishable),
        "{findings:?}"
    );
}

/// Case 6: the agent `.out` mtime — a 264-byte banner, identical for
/// healthy and hung agents. This one nearly `scancel`ed 19
/// healthy-looking jobs.
#[test]
fn case_6_constant_signal_cannot_distinguish() {
    let claim = Claim {
        entity: "agent run 4171".to_string(),
        instance: Some("agent run 4171".to_string()),
        value: "264-byte banner".to_string(),
        value_if_false: Some("264-byte banner".to_string()),
        name: "agent .out mtime".to_string(),
        selected: vec!["agent .out mtime".to_string()],
        taken_at: Some(NOW - 60),
        now: NOW,
        window_secs: WINDOW,
    };
    let findings = audit(&claim);
    assert_eq!(
        &findings,
        &[BindingFinding::Indistinguishable],
        "{findings:?}"
    );

    // And the spec rule: the check commissioned with this value in both
    // states is rejected at design time.
    let checks = vec![HealthCheck {
        name: "agent .out mtime".to_string(),
        healthy_value: "264-byte banner".to_string(),
        broken_value: Some("264-byte banner".to_string()),
    }];
    let untrusted = untrusted_checks(&checks);
    assert_eq!(
        untrusted,
        vec![autospec_core::entity_binding::UntrustedCheck {
            name: "agent .out mtime".to_string(),
            why: Discrimination::Constant,
        }]
    );
}

/// Case 7: `awk '$1>="19:45"'` on a log — believed to be today's lines,
/// actually any day's lines: string compare, no date.
#[test]
fn case_7_measurement_with_no_date_is_any_day() {
    let claim = Claim {
        entity: "cron-regsweep.log (today's lines)".to_string(),
        instance: Some("cron-regsweep.log (today's lines)".to_string()),
        value: "1 error line".to_string(),
        value_if_false: None,
        name: "cron-regsweep.log lines since 19:45".to_string(),
        selected: vec!["cron-regsweep.log (today's lines)".to_string()],
        taken_at: None, // the awk filter has no date; the file is three days old
        now: NOW,
        window_secs: WINDOW,
    };
    let findings = audit(&claim);
    assert!(
        matches!(findings.as_slice(), [BindingFinding::Undated]),
        "{findings:?}"
    );

    // With the date restored, the same file is stale, not current.
    let dated = Claim {
        taken_at: Some(NOW - 3 * 24 * 3600),
        ..claim
    };
    let findings = audit(&dated);
    assert!(
        matches!(
            findings.as_slice(),
            [BindingFinding::Stale { age_secs, window_secs }]
                if *age_secs == 3 * 24 * 3600 && *window_secs == WINDOW
        ),
        "{findings:?}"
    );
}

/// A claim that fails all four questions at once: the audit reports all
/// four, in question order.
#[test]
fn audit_reports_every_question_that_fails() {
    let claim = Claim {
        entity: "gateway@edge".to_string(),
        instance: None,
        value: "264-byte banner".to_string(),
        value_if_false: Some("264-byte banner".to_string()),
        name: "gateway".to_string(),
        selected: vec!["gateway@edge".to_string(), "gateway@hive".to_string()],
        taken_at: Some(NOW - 2 * WINDOW),
        now: NOW,
        window_secs: WINDOW,
    };
    let findings = audit(&claim);
    assert_eq!(findings.len(), 4, "{findings:?}");
    assert!(matches!(&findings[0], BindingFinding::Unattributed));
    assert!(matches!(&findings[1], BindingFinding::Indistinguishable));
    assert!(matches!(&findings[2], BindingFinding::AmbiguousName { .. }));
    assert!(matches!(&findings[3], BindingFinding::Stale { .. }));
}

// ── Q2 at design time: the spec rule ────────────────────────────────────

#[test]
fn a_check_that_cannot_distinguish_is_rejected_at_design_time() {
    let checks = vec![
        HealthCheck {
            name: "served_503".to_string(),
            healthy_value: "served_503=0".to_string(),
            broken_value: Some("served_503>0".to_string()),
        },
        HealthCheck {
            name: "wrapper .out size".to_string(),
            healthy_value: "264 bytes".to_string(),
            broken_value: Some("264 bytes".to_string()),
        },
        HealthCheck {
            name: "crash-record query".to_string(),
            healthy_value: "no records".to_string(),
            broken_value: None, // the spec never stated the broken value
        },
    ];
    let untrusted = untrusted_checks(&checks);
    assert_eq!(untrusted.len(), 2, "{untrusted:?}");
    assert_eq!(untrusted[0].name, "wrapper .out size");
    assert_eq!(untrusted[0].why, Discrimination::Constant);
    assert_eq!(untrusted[1].name, "crash-record query");
    assert_eq!(untrusted[1].why, Discrimination::Unstated);
}

#[test]
fn a_discriminating_check_is_trusted() {
    let check = HealthCheck {
        name: "pool size".to_string(),
        healthy_value: "pool=10".to_string(),
        broken_value: Some("pool<10".to_string()),
    };
    assert_eq!(check.discrimination(), Discrimination::Discriminates);
    assert!(untrusted_checks(&[check]).is_empty());
}

// ── Q3: name selection ──────────────────────────────────────────────────

#[test]
fn a_name_is_unique_only_when_it_selects_exactly_one() {
    assert_eq!(name_selection(&[]), NameSelection::SelectsNothing);
    assert_eq!(
        name_selection(&["gateway@edge".to_string()]),
        NameSelection::Unique
    );
    assert_eq!(
        name_selection(&["a".to_string(), "b".to_string()]),
        NameSelection::Ambiguous {
            selected: vec!["a".to_string(), "b".to_string()]
        }
    );
}

#[test]
fn a_name_selecting_nothing_is_a_finding_not_an_all_clear() {
    let claim = Claim {
        selected: vec![],
        ..clean_claim()
    };
    let findings = audit(&claim);
    assert!(
        matches!(findings.as_slice(), [BindingFinding::SelectsNothing { .. }]),
        "{findings:?}"
    );
}

// ── Q4: recency ─────────────────────────────────────────────────────────

#[test]
fn currency_is_window_based_and_edge_inclusive() {
    assert!(is_current(NOW, NOW, WINDOW));
    assert!(is_current(NOW, NOW - WINDOW, WINDOW)); // exactly at the edge
    assert!(!is_current(NOW, NOW - WINDOW - 1, WINDOW));
    // A clock that rewinds is zero age, never an underflow.
    assert!(is_current(NOW, NOW + 1000, WINDOW));
}

// ── Counter-evidence is cheaper than confirmation ───────────────────────

#[test]
fn an_irreversible_action_holds_without_counter_evidence() {
    // The case 6 action: `scancel` on the 264-byte mtime signal, with
    // no second, differently-shaped measurement.
    let action = Action {
        irreversible: true,
        licensing_kind: "log mtime".to_string(),
        counter_kinds: vec![],
    };
    assert_eq!(
        action_gate(&action),
        ActionGate::NeedsCounterEvidence {
            licensing_kind: "log mtime".to_string()
        }
    );
}

#[test]
fn counter_evidence_of_the_same_kind_does_not_clear_the_gate() {
    let action = Action {
        irreversible: true,
        licensing_kind: "log read".to_string(),
        counter_kinds: vec!["log read".to_string(), "log read".to_string()],
    };
    assert_eq!(
        action_gate(&action),
        ActionGate::NeedsCounterEvidence {
            licensing_kind: "log read".to_string()
        }
    );
}

#[test]
fn a_different_kinded_measurement_clears_the_gate() {
    // The incident's corrections: the log read was checked against the
    // gateway's raw state, the process state against its child, the
    // title against the issue body.
    let action = Action {
        irreversible: true,
        licensing_kind: "log read".to_string(),
        counter_kinds: vec!["log read".to_string(), "raw payload".to_string()],
    };
    assert_eq!(
        action_gate(&action),
        ActionGate::Cleared {
            counter_kind: "raw payload".to_string()
        }
    );
}

#[test]
fn a_reversible_action_needs_no_counter_evidence() {
    let action = Action {
        irreversible: false,
        licensing_kind: "log read".to_string(),
        counter_kinds: vec![],
    };
    assert_eq!(action_gate(&action), ActionGate::Reversible);
}

// ── Rendering ───────────────────────────────────────────────────────────

#[test]
fn findings_render_with_their_question_number() {
    let claim = clean_claim();
    assert!(audit(&claim).is_empty());

    let wrong = Claim {
        instance: Some("gateway@hive".to_string()),
        ..clean_claim()
    };
    let line = audit(&wrong)[0].line();
    assert!(line.starts_with("Q1: wrong instance"), "{line}");
    assert!(line.contains("gateway@hive"), "{line}");
    assert!(line.contains("gateway@edge"), "{line}");

    let gate = ActionGate::NeedsCounterEvidence {
        licensing_kind: "log mtime".to_string(),
    };
    let line = gate.line();
    assert!(line.starts_with("hold:"), "{line}");
    assert!(line.contains("different kind"), "{line}");
}
