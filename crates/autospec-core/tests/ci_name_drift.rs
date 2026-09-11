//! CI job name/step drift (issue #4197).
//!
//! The incident: a conversion gate was built to "match CI" by reading a CI
//! job's name to learn what it runs. The name enumerated three of the job's
//! four steps and omitted `test`; the gate derived from the name silently
//! skipped the test step (68 tests never ran) and nothing warned.
//!
//! The regression tests run in the configuration the bug required: a job
//! whose enumerating name omits a step. On a job whose name matches its
//! steps (the control), every check agrees and the bug is invisible — which
//! is exactly why a matching-name control cannot see the defect.

use autospec_core::ci_name_drift::{
    audit, derive_name, gate_matches_job, name_enumeration, name_is_stale, name_matches_steps,
    CiJob, CiStep, GateDrift, NameDrift,
};

/// The incident configuration: a job whose name enumerates three of its four
/// steps and omits `test`. The name is a label; the steps are the contract.
fn incident_job() -> CiJob {
    CiJob {
        id: "baseline".into(),
        name: "Next.js baseline (lint / typecheck / build)".into(),
        steps: vec![
            CiStep {
                name: "Lint".into(),
                run: "npm run lint".into(),
            },
            CiStep {
                name: "Typecheck".into(),
                run: "npm run typecheck".into(),
            },
            CiStep {
                name: "Test".into(),
                run: "npm test".into(),
            },
            CiStep {
                name: "Build".into(),
                run: "npm run build".into(),
            },
        ],
    }
}

/// The gate the author built by reading the name: it omits the test step.
fn incident_gate() -> Vec<String> {
    vec![
        "npm run lint".into(),
        "npm run typecheck".into(),
        "npm run build".into(),
    ]
}

/// The correct gate, generated from the steps (invariants 1/2), not the name.
fn full_gate() -> Vec<String> {
    vec![
        "npm run lint".into(),
        "npm run typecheck".into(),
        "npm test".into(),
        "npm run build".into(),
    ]
}

/// The control: a job whose enumerating name matches its steps. Every check
/// agrees here and the incident bug is invisible — the control proves the
/// checks are not false-positiving.
fn control_job() -> CiJob {
    CiJob {
        id: "baseline".into(),
        name: "Next.js baseline (lint / typecheck / test / build)".into(),
        steps: vec![
            CiStep {
                name: "Lint".into(),
                run: "npm run lint".into(),
            },
            CiStep {
                name: "Typecheck".into(),
                run: "npm run typecheck".into(),
            },
            CiStep {
                name: "Test".into(),
                run: "npm test".into(),
            },
            CiStep {
                name: "Build".into(),
                run: "npm run build".into(),
            },
        ],
    }
}

// --- Invariant 1: the steps are the contract; the name is a label ---

#[test]
fn command_list_is_the_run_steps_in_order() {
    // The contract includes the test step even though the name omits it: the
    // name is never consulted, so a command list derived from the steps is
    // complete where one derived from the name is not.
    assert_eq!(
        incident_job().command_list(),
        vec![
            "npm run lint",
            "npm run typecheck",
            "npm test",
            "npm run build"
        ]
    );
}

#[test]
fn command_list_includes_the_step_the_name_omits() {
    let job = incident_job();
    // The name says (lint / typecheck / build); the contract says there is
    // also a test step. The contract is what a gate is built from.
    assert!(job.command_list().contains(&"npm test".to_string()));
    assert!(!job.name.contains("test"));
}

// --- Invariant 2: the local gate must equal the job's step list ---

#[test]
fn gate_generated_from_steps_is_in_sync() {
    assert_eq!(
        gate_matches_job(&full_gate(), &incident_job()),
        GateDrift::InSync
    );
}

#[test]
fn gate_derived_from_the_name_is_out_of_sync_and_names_the_skip() {
    // THE regression: the author built the gate from the name, so it omits
    // the test step. This is the check that would have caught the incident.
    assert_eq!(
        gate_matches_job(&incident_gate(), &incident_job()),
        GateDrift::OutOfSync {
            missing_from_gate: vec!["npm test".to_string()],
            extra_in_gate: vec![],
        }
    );
}

#[test]
fn gate_out_of_sync_line_is_a_warn_naming_the_skipped_step() {
    let job = incident_job();
    let line = gate_matches_job(&incident_gate(), &job).line(&job);
    assert!(line.starts_with("WARN:"), "line: {line}");
    assert!(line.contains("npm test"), "line: {line}");
    assert!(line.contains("baseline"), "line: {line}");
}

#[test]
fn gate_with_an_extra_command_reports_it() {
    let gate = vec![
        "npm run lint".to_string(),
        "npm run typecheck".to_string(),
        "npm test".to_string(),
        "npm run build".to_string(),
        "npm run extra".to_string(),
    ];
    assert_eq!(
        gate_matches_job(&gate, &incident_job()),
        GateDrift::OutOfSync {
            missing_from_gate: vec![],
            extra_in_gate: vec!["npm run extra".to_string()],
        }
    );
}

#[test]
fn gate_match_accounts_for_multiplicity() {
    // Two steps run the same command; a gate with only one copy is missing
    // the other.
    let job = CiJob {
        id: "dup".into(),
        name: "dup (a / a)".into(),
        steps: vec![
            CiStep {
                name: "A".into(),
                run: "npm run a".into(),
            },
            CiStep {
                name: "A again".into(),
                run: "npm run a".into(),
            },
        ],
    };
    let gate = vec!["npm run a".to_string()];
    assert_eq!(
        gate_matches_job(&gate, &job),
        GateDrift::OutOfSync {
            missing_from_gate: vec!["npm run a".to_string()],
            extra_in_gate: vec![],
        }
    );
}

#[test]
fn control_gate_is_in_sync_on_the_matching_job() {
    assert_eq!(
        gate_matches_job(&full_gate(), &control_job()),
        GateDrift::InSync
    );
}

// --- Invariant 3: an enumerating name is tested against the steps ---

#[test]
fn name_enumeration_parses_the_parenthetical_list() {
    assert_eq!(
        name_enumeration("Next.js baseline (lint / typecheck / build)"),
        Some(vec![
            "lint".to_string(),
            "typecheck".to_string(),
            "build".to_string()
        ])
    );
}

#[test]
fn name_enumeration_is_none_when_there_is_no_group() {
    assert_eq!(name_enumeration("Next.js baseline"), None);
}

#[test]
fn name_enumeration_handles_a_single_token() {
    assert_eq!(
        name_enumeration("baseline (build)"),
        Some(vec!["build".to_string()])
    );
}

#[test]
fn incident_name_omits_the_test_step() {
    // The lint that would have caught the incident: the name enumerates three
    // steps, the job has four, and the omission is named.
    assert_eq!(
        name_matches_steps(&incident_job()),
        NameDrift::OutOfSync {
            named: vec![
                "lint".to_string(),
                "typecheck".to_string(),
                "build".to_string()
            ],
            steps: vec![
                "lint".to_string(),
                "typecheck".to_string(),
                "test".to_string(),
                "build".to_string()
            ],
            missing: vec!["test".to_string()],
            extra: vec![],
        }
    );
}

#[test]
fn incident_name_drift_line_is_a_warn_naming_the_omitted_step() {
    let job = incident_job();
    let line = name_matches_steps(&job).line(&job);
    assert!(line.starts_with("WARN:"), "line: {line}");
    assert!(line.contains("test"), "line: {line}");
    assert!(line.contains("baseline"), "line: {line}");
}

#[test]
fn control_name_is_in_sync() {
    assert_eq!(name_matches_steps(&control_job()), NameDrift::InSync);
}

#[test]
fn non_enumerating_name_is_not_checked() {
    let job = CiJob {
        id: "plain".into(),
        name: "Next.js baseline".into(),
        steps: vec![CiStep {
            name: "Test".into(),
            run: "npm test".into(),
        }],
    };
    assert_eq!(name_matches_steps(&job), NameDrift::NotEnumerating);
    assert!(name_matches_steps(&job).line(&job).starts_with("OK:"));
}

#[test]
fn name_match_is_case_insensitive() {
    let job = CiJob {
        id: "case".into(),
        name: "baseline (Lint / Typecheck / Test / Build)".into(),
        steps: vec![
            CiStep {
                name: "lint".into(),
                run: "npm run lint".into(),
            },
            CiStep {
                name: "typecheck".into(),
                run: "npm run typecheck".into(),
            },
            CiStep {
                name: "test".into(),
                run: "npm test".into(),
            },
            CiStep {
                name: "build".into(),
                run: "npm run build".into(),
            },
        ],
    };
    assert_eq!(name_matches_steps(&job), NameDrift::InSync);
}

#[test]
fn name_listing_a_token_with_no_step_is_reported_extra() {
    let job = CiJob {
        id: "extra".into(),
        name: "baseline (lint / typecheck / build / coverage)".into(),
        steps: vec![
            CiStep {
                name: "Lint".into(),
                run: "npm run lint".into(),
            },
            CiStep {
                name: "Typecheck".into(),
                run: "npm run typecheck".into(),
            },
            CiStep {
                name: "Build".into(),
                run: "npm run build".into(),
            },
        ],
    };
    assert_eq!(
        name_matches_steps(&job),
        NameDrift::OutOfSync {
            named: vec![
                "lint".to_string(),
                "typecheck".to_string(),
                "build".to_string(),
                "coverage".to_string()
            ],
            steps: vec![
                "lint".to_string(),
                "typecheck".to_string(),
                "build".to_string()
            ],
            missing: vec![],
            extra: vec!["coverage".to_string()],
        }
    );
}

#[test]
fn unnamed_steps_are_out_of_scope_for_name_matching() {
    // An unnamed step cannot be enumerated by name; it is bound by the gate
    // check instead. The name enumerating only the named steps is in sync.
    let job = CiJob {
        id: "unnamed".into(),
        name: "baseline (lint)".into(),
        steps: vec![
            CiStep {
                name: "Lint".into(),
                run: "npm run lint".into(),
            },
            CiStep {
                name: "".into(),
                run: "npm test".into(),
            },
        ],
    };
    assert_eq!(name_matches_steps(&job), NameDrift::InSync);
    // ...but the gate check still sees the unnamed test step.
    let gate = vec!["npm run lint".to_string()];
    assert_eq!(
        gate_matches_job(&gate, &job),
        GateDrift::OutOfSync {
            missing_from_gate: vec!["npm test".to_string()],
            extra_in_gate: vec![],
        }
    );
}

// --- Invariant 4: re-derive the name; do not hand-edit it ---

#[test]
fn derive_name_regenerates_the_group_from_the_steps() {
    assert_eq!(
        derive_name(&incident_job()),
        "Next.js baseline (Lint / Typecheck / Test / Build)"
    );
}

#[test]
fn incident_name_is_stale() {
    assert!(name_is_stale(&incident_job()));
}

#[test]
fn control_name_is_not_stale() {
    // Case-insensitive: the control's lowercase name matches its steps.
    assert!(!name_is_stale(&control_job()));
}

#[test]
fn non_enumerating_name_is_not_stale() {
    let job = CiJob {
        id: "plain".into(),
        name: "Next.js baseline".into(),
        steps: vec![CiStep {
            name: "Test".into(),
            run: "npm test".into(),
        }],
    };
    assert!(!name_is_stale(&job));
}

#[test]
fn re_derived_name_is_in_sync_and_fresh() {
    // The fix: re-derive the name from the steps, and the drift disappears.
    let mut fixed = incident_job();
    fixed.name = derive_name(&fixed);
    assert_eq!(name_matches_steps(&fixed), NameDrift::InSync);
    assert!(!name_is_stale(&fixed));
    assert_eq!(gate_matches_job(&full_gate(), &fixed), GateDrift::InSync);
}

#[test]
fn derive_name_preserves_prefix_and_omits_group_when_no_steps() {
    let job = CiJob {
        id: "empty".into(),
        name: "baseline (stale)".into(),
        steps: vec![],
    };
    assert_eq!(derive_name(&job), "baseline");
}

// --- audit: the runtime half (the lint a gate-definition change triggers) ---

#[test]
fn audit_reports_all_three_lines_for_the_incident() {
    let job = incident_job();
    let lines = audit(&incident_gate(), &job);
    // 1) the gate skips the test step (invariant 2)
    // 2) the name omits the test step (invariant 3)
    // 3) the name is stale and must be re-derived (invariant 4)
    assert_eq!(lines.len(), 3, "lines: {lines:?}");
    assert!(lines[0].starts_with("WARN: gate"), "line: {}", lines[0]);
    assert!(lines[0].contains("npm test"), "line: {}", lines[0]);
    assert!(lines[1].starts_with("WARN: name"), "line: {}", lines[1]);
    assert!(lines[1].contains("test"), "line: {}", lines[1]);
    assert!(lines[2].starts_with("WARN: name"), "line: {}", lines[2]);
    assert!(lines[2].contains("re-derive"), "line: {}", lines[2]);
}

#[test]
fn audit_is_all_ok_on_the_control() {
    let job = control_job();
    let lines = audit(&full_gate(), &job);
    assert_eq!(lines.len(), 2, "lines: {lines:?}");
    assert!(
        lines.iter().all(|l| l.starts_with("OK:")),
        "lines: {lines:?}"
    );
}

#[test]
fn audit_is_all_ok_when_gate_and_name_both_match() {
    let job = incident_job();
    // The correct gate plus a name that has been re-derived: only OK lines,
    // no stale warning.
    let mut fixed = job;
    fixed.name = derive_name(&fixed);
    let lines = audit(&full_gate(), &fixed);
    assert!(
        lines.iter().all(|l| l.starts_with("OK:")),
        "lines: {lines:?}"
    );
}
