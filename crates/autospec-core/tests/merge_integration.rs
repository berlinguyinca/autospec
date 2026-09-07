//! Post-merge integration gate (#3569): a green merge is not integration.
//!
//! Covers the acceptance criteria: clean batch, semantically conflicting
//! batch, a project with no smoke test, concept collisions, and unrecorded
//! shared-concept constraints.

use autospec_core::integration::{
    detect_concept_collisions, evaluate_post_merge_smoke, find_unrecorded_constraints,
    normalize_concept, scan_added_lines, verify_trunk_after_merges, AddedLine, ConceptCollision,
    GateOutcome, IssueConcepts, PostMergeSmokeReport, SmokeCoverage, SmokeOutcome,
    TrunkGateVerdict, TrunkMerge,
};

fn merge(id: &str, issue: u64, branch: &str) -> TrunkMerge {
    TrunkMerge {
        id: id.to_string(),
        issue,
        branch: branch.to_string(),
    }
}

// --- 1 + 2. trunk gate after each merge -------------------------------------

#[test]
fn clean_batch_verifies_trunk_after_every_merge() {
    let merges = vec![
        merge("PR#4101", 3560, "feat/model-id-validation"),
        merge("PR#4102", 3561, "feat/model-id-path"),
    ];
    let results = vec![GateOutcome::Pass, GateOutcome::Pass];

    let verdict = verify_trunk_after_merges(&merges, &results).expect("clean batch verifies");

    assert_eq!(verdict, TrunkGateVerdict::Verified { merges_verified: 2 });
    assert!(verdict
        .summary()
        .contains("trunk verified after 2 merge(s)"));
}

#[test]
fn semantically_conflicting_batch_names_the_merge_that_broke_the_trunk() {
    // Mirror of the incident: merge one adds a validation rule, merge two
    // requires the same field to equal a filesystem path. Both sides green
    // in isolation; only the trunk after the second merge is unusable.
    let merges = vec![
        merge("PR#4101", 3560, "feat/model-id-validation"),
        merge("PR#4102", 3561, "feat/model-id-path"),
    ];
    let results = vec![
        GateOutcome::Pass,
        GateOutcome::Fail {
            findings: vec![
                "every worker registration returns 422: model_id fails ^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$ when it is a path".to_string(),
            ],
        },
    ];

    let verdict = verify_trunk_after_merges(&merges, &results).expect("verdict is produced");

    match verdict {
        TrunkGateVerdict::BrokenByMerge {
            ref merge,
            merge_index,
            ref findings,
        } => {
            assert_eq!(merge.id, "PR#4102");
            assert_eq!(merge.issue, 3561);
            assert_eq!(merge.branch, "feat/model-id-path");
            assert_eq!(merge_index, 1);
            assert_eq!(findings.len(), 1);
        }
        other => panic!("expected BrokenByMerge, got {other:?}"),
    }
    assert!(verdict.summary().contains("PR#4102"));
    assert!(verdict.summary().contains("feat/model-id-path"));
}

#[test]
fn verifying_the_batch_once_is_rejected() {
    // The gap this exists to close: one gate run for the whole batch.
    let merges = vec![merge("PR#4101", 3560, "a"), merge("PR#4102", 3561, "b")];
    let results = vec![GateOutcome::Pass];

    let error = verify_trunk_after_merges(&merges, &results).unwrap_err();
    assert!(error.contains("after each merge"), "{error}");
    assert!(error.contains("2 merge(s), 1 result(s)"), "{error}");
}

#[test]
fn an_empty_batch_is_trivially_verified() {
    let verdict = verify_trunk_after_merges(&[], &[]).expect("empty batch verifies");
    assert_eq!(verdict, TrunkGateVerdict::Verified { merges_verified: 0 });
}

// --- 2. smoke test on the trunk post-merge ----------------------------------

#[test]
fn a_project_without_a_smoke_test_reports_a_gap_instead_of_passing_silently() {
    let coverage = SmokeCoverage::Gap {
        reason: "no tests/e2e or smoke target found".to_string(),
    };

    let report = evaluate_post_merge_smoke(&coverage, None).expect("gap is reportable");

    assert!(!report.covered);
    assert!(report.outcome.is_none());
    assert_eq!(
        report.gap.as_deref(),
        Some("no tests/e2e or smoke target found")
    );
    // The gap must surface as a finding; a `None` finding would be the
    // silence this check exists to prevent.
    let finding = report.finding().expect("gap produces a finding");
    assert!(finding.contains("no end-to-end smoke test"), "{finding}");
}

#[test]
fn a_project_with_a_smoke_test_must_run_it_on_the_trunk() {
    let coverage = SmokeCoverage::Present {
        command: "cargo run --bin worker -- register smoke-worker".to_string(),
    };

    let error = evaluate_post_merge_smoke(&coverage, None).unwrap_err();
    assert!(error.contains("not run on the trunk"), "{error}");

    let report =
        evaluate_post_merge_smoke(&coverage, Some(SmokeOutcome::Pass)).expect("pass recorded");
    assert!(report.covered);
    assert!(report.gap.is_none());
    assert!(report.finding().is_none());
}

#[test]
fn a_failing_trunk_smoke_is_a_finding() {
    let coverage = SmokeCoverage::Present {
        command: "make smoke".to_string(),
    };
    let report = evaluate_post_merge_smoke(
        &coverage,
        Some(SmokeOutcome::Fail {
            findings: vec!["worker registration returned 400".to_string()],
        }),
    )
    .expect("failure is reportable");

    let finding = report.finding().expect("failing smoke produces a finding");
    assert!(
        finding.contains("worker registration returned 400"),
        "{finding}"
    );
}

#[test]
fn a_smoke_outcome_without_coverage_is_rejected() {
    let coverage = SmokeCoverage::Gap {
        reason: "none".to_string(),
    };
    let error = evaluate_post_merge_smoke(&coverage, Some(SmokeOutcome::Pass)).unwrap_err();
    assert!(error.contains("no end-to-end smoke test"), "{error}");
}

// --- 3. collisions on concepts, not just files ------------------------------

#[test]
fn concept_collision_flags_two_issues_constraining_the_same_field() {
    // Barely overlapping textually, colliding on the shared noun
    // "what identifies a model".
    let issues = vec![
        IssueConcepts {
            issue: 3560,
            concepts: vec!["Model_ID".to_string()],
        },
        IssueConcepts {
            issue: 3561,
            concepts: vec!["`model_id`".to_string(), "worker_name".to_string()],
        },
    ];

    let collisions = detect_concept_collisions(&issues);

    assert_eq!(collisions.len(), 1);
    let collision = &collisions[0];
    assert_eq!(collision.concept, "model_id");
    assert_eq!(collision.issues, vec![3560, 3561]);
    assert!(
        collision.directive().contains("#3560 and #3561"),
        "{}",
        collision.directive()
    );
    assert!(
        collision.directive().contains("sequence them"),
        "{}",
        collision.directive()
    );
}

#[test]
fn concept_collisions_ignore_distinct_concepts() {
    let issues = vec![
        IssueConcepts {
            issue: 1,
            concepts: vec!["model_id".to_string()],
        },
        IssueConcepts {
            issue: 2,
            concepts: vec!["worker_name".to_string()],
        },
    ];

    assert!(detect_concept_collisions(&issues).is_empty());
}

#[test]
fn concept_collisions_include_every_issue_sharing_a_concept() {
    let issues = (1..=3)
        .map(|issue| IssueConcepts {
            issue,
            concepts: vec!["model_id".to_string()],
        })
        .collect::<Vec<_>>();

    let collisions = detect_concept_collisions(&issues);
    assert_eq!(
        collisions,
        vec![ConceptCollision {
            concept: "model_id".to_string(),
            issues: vec![1, 2, 3],
        }]
    );
}

#[test]
fn normalize_concept_collapses_casing_and_backticks() {
    assert_eq!(normalize_concept("  Model_ID "), "model_id");
    assert_eq!(normalize_concept("`model_id`"), "model_id");
    assert_eq!(normalize_concept(""), "");
}

// --- 4. constraints on shared concepts must be recorded ---------------------

#[test]
fn unrecorded_constraint_on_a_shared_concept_is_prompted() {
    let added = vec![
        AddedLine {
            path: "src/registry.rs".to_string(),
            line: 41,
            text: "let re = Regex::new(r\"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$\").unwrap(); if !re.is_match(&model_id) { return Err(422); }".to_string(),
        },
        AddedLine {
            path: "src/registry.rs".to_string(),
            line: 58,
            text: "assert_eq!(model_id, requested_path.to_string_lossy().to_string());".to_string(),
        },
        AddedLine {
            path: "src/registry.rs".to_string(),
            line: 70,
            text: "log::info!(\"registering {worker_name}\");".to_string(),
        },
    ];

    let constraints = scan_added_lines(&added, &["model_id".to_string()]);

    // Only the two constraint lines on the shared concept are detected.
    assert_eq!(constraints.len(), 2);
    assert_eq!(
        constraints[0].kind,
        autospec_core::integration::ConstraintKind::Validation
    );
    assert_eq!(constraints[0].line, 41);
    assert_eq!(
        constraints[1].kind,
        autospec_core::integration::ConstraintKind::Equality
    );

    let unrecorded = find_unrecorded_constraints(&constraints, None);
    assert_eq!(unrecorded.len(), 2);
    for finding in &unrecorded {
        assert!(
            finding.directive.contains("CONTRACT.md"),
            "{}",
            finding.directive
        );
        assert!(
            finding.directive.contains("model_id"),
            "{}",
            finding.directive
        );
    }
}

#[test]
fn constraint_recorded_in_the_contract_is_not_flagged() {
    let added = vec![AddedLine {
        path: "src/registry.rs".to_string(),
        line: 41,
        text: "if !MODEL_ID_PATTERN.is_match(&model_id) { return 422; }".to_string(),
    }];
    let contract = "R7: model_id must match ^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$ and is the canonical model identifier.\n";

    let constraints = scan_added_lines(&added, &["model_id".to_string()]);
    assert_eq!(constraints.len(), 1);

    assert!(
        find_unrecorded_constraints(&constraints, Some(contract)).is_empty(),
        "a constraint on a concept the contract already defines is recorded"
    );
}

#[test]
fn scan_ignores_concepts_the_issue_does_not_constrain() {
    let added = vec![AddedLine {
        path: "src/registry.rs".to_string(),
        line: 12,
        text: "assert_eq!(worker_name, expected);".to_string(),
    }];

    // The shared concept for this batch is model_id; worker_name is out of
    // scope for the scan.
    assert!(scan_added_lines(&added, &["model_id".to_string()]).is_empty());
}

#[test]
fn smoke_reports_are_comparable_and_cloneable() {
    let coverage = SmokeCoverage::Gap {
        reason: "none".to_string(),
    };
    let report = evaluate_post_merge_smoke(&coverage, None).expect("gap report");
    let cloned: &PostMergeSmokeReport = &report.clone();
    assert_eq!(cloned, &report);
}
