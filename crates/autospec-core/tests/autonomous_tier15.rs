use autospec_core::autonomous::tier15::{
    observe_tier15, Tier15Classification, Tier15Decision, Tier15HoldReason, Tier15Input,
    Tier15QuarantineReason, Tier15Route, Tier15RouteReason, Tier15SkipReason,
};
use autospec_core::coordination::RemoteIssue;
use std::fs;
use std::path::Path;

fn open(number: u64, title: &str, body: &str, labels: &[&str]) -> RemoteIssue {
    RemoteIssue::open(
        number,
        title,
        body,
        labels.iter().map(|label| (*label).to_string()).collect(),
        "autospec",
    )
}

fn closed(number: u64, title: &str, body: &str) -> RemoteIssue {
    RemoteIssue::closed(number, title, body, Vec::new(), "autospec")
}

#[test]
fn observer_records_each_closed_selection_outcome_without_mutating_issues() {
    let duplicate_title = "Duplicate closed finding";
    let duplicate_body = "fix: preserve this exact already-closed finding with a bounded scope";
    let observation = observe_tier15(Tier15Input::new(
        vec![
            open(
                10,
                "Eligible",
                "fix: retain a native read-only Tier 1.5 observer in `src/tier15.rs`",
                &[],
            ),
            open(
                11,
                "Excluded",
                "fix: excluded issue body is still actionable",
                &["no-auto"],
            ),
            open(12, duplicate_title, duplicate_body, &[]),
            open(
                13,
                "Already groomed",
                "fix: do not re-groom this candidate",
                &["groom:proposed"],
            ),
            open(14, "Thin", "fix", &[]),
            open(
                15,
                "Ambiguous",
                "This body is long enough but has no concrete intent at all.",
                &[],
            ),
            open(
                16,
                "Blocked dependency",
                "fix: wait for the prerequisite\n\n## Dependencies\n\nDepends on #99\n",
                &[],
            ),
            open(
                17,
                "Security quarantine",
                "fix: preserve quarantine",
                &["security:quarantined"],
            ),
            open(
                18,
                "Epic",
                "fix: split this epic into bounded children",
                &["epic"],
            ),
            open(
                19,
                "Template",
                "## Goal\n\nDeliver this structured candidate with a bounded implementation scope.",
                &["needs-autospec-template"],
            ),
            open(
                20,
                "Classify",
                "fix: retain a complete native observation for `src/tier15.rs`",
                &["needs-classify"],
            ),
        ],
        vec![closed(90, duplicate_title, duplicate_body)],
        20,
    ))
    .expect("complete evidence produces a closed observation");

    assert_eq!(
        observation.decisions(),
        &[
            Tier15Decision::Produced {
                number: 10,
                classification: Tier15Classification::Unlabeled,
            },
            Tier15Decision::Skipped {
                number: 11,
                classification: Tier15Classification::Unlabeled,
                reason: Tier15SkipReason::ExcludedLabel,
            },
            Tier15Decision::Skipped {
                number: 12,
                classification: Tier15Classification::Unlabeled,
                reason: Tier15SkipReason::ClosedFingerprint,
            },
            Tier15Decision::Skipped {
                number: 13,
                classification: Tier15Classification::Unlabeled,
                reason: Tier15SkipReason::AlreadyGroomed,
            },
            Tier15Decision::Held {
                number: 14,
                classification: Tier15Classification::Unlabeled,
                reason: Tier15HoldReason::ThinIntent,
            },
            Tier15Decision::Held {
                number: 15,
                classification: Tier15Classification::Unlabeled,
                reason: Tier15HoldReason::AmbiguousIntent,
            },
            Tier15Decision::Held {
                number: 16,
                classification: Tier15Classification::Unlabeled,
                reason: Tier15HoldReason::Dependency,
            },
            Tier15Decision::Quarantined {
                number: 17,
                classification: Tier15Classification::Unlabeled,
                reason: Tier15QuarantineReason::ExistingSecurity,
            },
            Tier15Decision::Routed {
                number: 18,
                classification: Tier15Classification::Unlabeled,
                route: Tier15Route::Split,
                reason: Tier15RouteReason::Epic,
            },
            Tier15Decision::Routed {
                number: 19,
                classification: Tier15Classification::NeedsTemplate,
                route: Tier15Route::Template,
                reason: Tier15RouteReason::TemplateRequired,
            },
            Tier15Decision::Produced {
                number: 20,
                classification: Tier15Classification::NeedsClassify,
            },
        ]
    );
    assert_eq!(observation.produced_count(), 2);
    assert!(observation.evidence_json().contains("closed_fingerprint"));
    assert!(observation.evidence_json().contains("existing_security"));
}

#[test]
fn observer_sorts_identical_duplicate_open_issues_and_rejects_conflicting_payloads() {
    let issue = open(
        42,
        "Duplicate",
        "fix: choose one deterministic observation for `src/tier15.rs`",
        &[],
    );
    let identical = observe_tier15(Tier15Input::new(vec![issue.clone(), issue], Vec::new(), 1))
        .expect("identical snapshots are idempotent");
    assert_eq!(identical.decisions().len(), 1);

    let conflict = observe_tier15(Tier15Input::new(
        vec![
            open(42, "Duplicate", "fix: first payload has enough detail", &[]),
            open(
                42,
                "Duplicate",
                "fix: changed payload must fail closed",
                &[],
            ),
        ],
        Vec::new(),
        1,
    ));
    assert!(conflict
        .expect_err("same open number with different payload must fail closed")
        .contains("conflicting open issue 42"));
}

#[test]
fn observer_marks_unselected_candidates_as_budget_exhausted() {
    let observation = observe_tier15(Tier15Input::new(
        vec![
            open(
                2,
                "Second",
                "fix: second candidate has a bounded scope",
                &[],
            ),
            open(1, "First", "fix: first candidate has a bounded scope", &[]),
        ],
        Vec::new(),
        1,
    ))
    .expect("complete evidence produces an observation");

    assert_eq!(
        observation.decisions(),
        &[
            Tier15Decision::Produced {
                number: 1,
                classification: Tier15Classification::Unlabeled,
            },
            Tier15Decision::Skipped {
                number: 2,
                classification: Tier15Classification::Unlabeled,
                reason: Tier15SkipReason::BudgetExhausted,
            },
        ]
    );
}

#[test]
fn observer_produces_explicit_fix_body_despite_typed_title() {
    let observation = observe_tier15(Tier15Input::new(
        vec![open(
            1,
            "fix: retain body-intent precedence",
            "fix: preserve a bounded eligibility decision in `src/tier15.rs`.",
            &[],
        )],
        Vec::new(),
        1,
    ))
    .expect("complete evidence produces an observation");

    assert_eq!(
        observation.decisions(),
        &[Tier15Decision::Produced {
            number: 1,
            classification: Tier15Classification::Unlabeled,
        }]
    );
}

#[test]
fn observer_produces_explicit_fix_body_despite_checkbox_structure() {
    let observation = observe_tier15(Tier15Input::new(
        vec![open(
            1,
            "Retain body-intent precedence",
            "fix: preserve a bounded eligibility decision in `src/tier15.rs`.\n\n- [ ] Keep the outcome stable.",
            &[],
        )],
        Vec::new(),
        1,
    ))
    .expect("complete evidence produces an observation");

    assert_eq!(
        observation.decisions(),
        &[Tier15Decision::Produced {
            number: 1,
            classification: Tier15Classification::Unlabeled,
        }]
    );
}

#[test]
fn observer_produces_explicit_fix_body_despite_goal_heading() {
    let observation = observe_tier15(Tier15Input::new(
        vec![open(
            1,
            "Retain body-intent precedence",
            "fix: preserve a bounded eligibility decision in `src/tier15.rs`.\n\n## Goal\n\nKeep the outcome stable.",
            &[],
        )],
        Vec::new(),
        1,
    ))
    .expect("complete evidence produces an observation");

    assert_eq!(
        observation.decisions(),
        &[Tier15Decision::Produced {
            number: 1,
            classification: Tier15Classification::Unlabeled,
        }]
    );
}

#[test]
fn observer_rejects_an_invalid_closed_snapshot_and_distinguishes_satisfied_dependencies() {
    let invalid = observe_tier15(Tier15Input::new(
        Vec::new(),
        vec![open(90, "Not closed", "fix: invalid evidence", &[])],
        1,
    ));
    assert!(invalid
        .expect_err("a closed snapshot may not contain an open issue")
        .contains("closed snapshot contains open issue 90"));

    let observation = observe_tier15(Tier15Input::new(
        vec![
            open(
                1,
                "Satisfied dependency",
                "fix: admit after evidence closes\n\n## Dependencies\n\nDepends on #90\n",
                &[],
            ),
            open(
                2,
                "Unresolved dependency",
                "fix: retain hold until evidence closes\n\n## Dependencies\n\nDepends on #91\n",
                &[],
            ),
        ],
        vec![closed(90, "Completed dependency", "done")],
        2,
    ))
    .expect("complete dependency evidence produces an observation");

    assert_eq!(
        observation.decisions(),
        &[
            Tier15Decision::Produced {
                number: 1,
                classification: Tier15Classification::Unlabeled,
            },
            Tier15Decision::Held {
                number: 2,
                classification: Tier15Classification::Unlabeled,
                reason: Tier15HoldReason::Dependency,
            },
        ]
    );
}

#[test]
fn observer_source_has_no_process_or_mutation_authority() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for source in [
        root.join("src/autonomous/tier15/model.rs"),
        root.join("src/autonomous/tier15/observer.rs"),
    ] {
        let contents = fs::read_to_string(&source).expect("read Tier 1.5 observer source");
        for forbidden in [
            "Command",
            "std::process",
            "WaterfallStore",
            "queue::",
            "claim::",
            "gh ",
            "autonomous-promote-open-issues",
        ] {
            assert!(
                !contents.contains(forbidden),
                "{} retains prohibited authority: {forbidden}",
                source.display()
            );
        }
    }
}

#[test]
fn observer_preserves_skip_guards_before_epic_routing() {
    let observation = observe_tier15(Tier15Input::new(
        vec![
            open(
                1,
                "Excluded epic",
                "fix: this epic must not bypass the no-auto guard",
                &["epic", "no-auto"],
            ),
            open(
                2,
                "Groomed epic",
                "fix: this epic must not be routed a second time",
                &["epic", "groom:proposed"],
            ),
        ],
        Vec::new(),
        2,
    ))
    .expect("complete evidence produces an observation");

    assert_eq!(
        observation.decisions(),
        &[
            Tier15Decision::Skipped {
                number: 1,
                classification: Tier15Classification::Unlabeled,
                reason: Tier15SkipReason::ExcludedLabel,
            },
            Tier15Decision::Skipped {
                number: 2,
                classification: Tier15Classification::Unlabeled,
                reason: Tier15SkipReason::AlreadyGroomed,
            },
        ]
    );
}

#[test]
fn observer_allows_a_dependency_present_in_the_open_snapshot() {
    let observation = observe_tier15(Tier15Input::new(
        vec![
            open(
                1,
                "Dependent issue",
                "fix: preserve an open dependency\n\n## Dependencies\n\nDepends on #2\n",
                &[],
            ),
            open(
                2,
                "Dependency",
                "fix: provide the observed dependency with enough implementation detail",
                &[],
            ),
        ],
        Vec::new(),
        2,
    ))
    .expect("an open dependency is complete existence evidence");

    assert_eq!(
        observation.decisions(),
        &[
            Tier15Decision::Produced {
                number: 1,
                classification: Tier15Classification::Unlabeled,
            },
            Tier15Decision::Produced {
                number: 2,
                classification: Tier15Classification::Unlabeled,
            },
        ]
    );
}

#[test]
fn observer_matches_legacy_label_checks_case_insensitively() {
    let observation = observe_tier15(Tier15Input::new(
        vec![open(
            1,
            "Case insensitive labels",
            "fix: an excluded epic must remain excluded",
            &["EPIC", "NO-AUTO"],
        )],
        Vec::new(),
        1,
    ))
    .expect("complete evidence produces an observation");

    assert_eq!(
        observation.decisions(),
        &[Tier15Decision::Skipped {
            number: 1,
            classification: Tier15Classification::Unlabeled,
            reason: Tier15SkipReason::ExcludedLabel,
        }]
    );
}

#[test]
fn observer_recognizes_every_legacy_structured_intent_shape() {
    let observation = observe_tier15(Tier15Input::new(
        vec![
            open(
                1,
                "Star checkbox",
                "* [ ] Preserve the complete Tier 1.5 observer behavior for review.",
                &[],
            ),
            open(
                2,
                "Nested heading",
                "### Objective\n\nPreserve the complete Tier 1.5 observer behavior for review.",
                &[],
            ),
        ],
        Vec::new(),
        2,
    ))
    .expect("complete evidence produces an observation");

    assert_eq!(
        observation.decisions(),
        &[
            Tier15Decision::Routed {
                number: 1,
                classification: Tier15Classification::Unlabeled,
                route: Tier15Route::Template,
                reason: Tier15RouteReason::StructuredIntent,
            },
            Tier15Decision::Routed {
                number: 2,
                classification: Tier15Classification::Unlabeled,
                route: Tier15Route::Template,
                reason: Tier15RouteReason::StructuredIntent,
            },
        ]
    );
}
