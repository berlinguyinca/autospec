//! Issue #3324 (Pi + Qwen3.8 M9): planner, builder and reviewer run as
//! separate sessions with distinct ids; modifying sessions get independent
//! worktrees; read-only lanes run in parallel and must finish with zero
//! filesystem edits; handoffs between sessions are structured artifacts; the
//! reviewer's output validates as `approve|changes_required|uncertain`.

use autospec_core::aar::{
    fold_events, parse_pi_event, AgentRole, IsolationViolation, ObservedEdits, ReviewVerdict,
    RolePolicy, SessionArtifact, SessionGrant, SessionIsolation,
};

fn grant(session_id: &str, worktree: &str, role: AgentRole) -> SessionGrant {
    SessionGrant {
        session_id: session_id.to_string(),
        worktree: worktree.to_string(),
        role,
        policy: RolePolicy::for_role(role),
    }
}

// --- AC1: three distinct session ids for planner, builder, reviewer --------

#[test]
fn planner_builder_reviewer_run_as_three_distinct_sessions() {
    let mut iso = SessionIsolation::new();
    iso.grant(grant("planner-s1", "/work/main", AgentRole::Planner))
        .expect("planner grant");
    iso.grant(grant("builder-s2", "/work/feat", AgentRole::Implementer))
        .expect("builder grant");
    iso.grant(grant("reviewer-s3", "/work/feat", AgentRole::Reviewer))
        .expect("reviewer grant");

    let sessions = ["planner-s1", "builder-s2", "reviewer-s3"];
    assert_eq!(iso.session_count(), 3);
    for session in &sessions {
        assert!(iso.is_active(session));
    }

    // Re-using a session id is a violation, not a re-grant.
    let err = iso
        .grant(grant("planner-s1", "/work/main", AgentRole::Planner))
        .unwrap_err();
    assert!(
        matches!(
            err,
            IsolationViolation::DuplicateSession { ref session_id }
                if session_id == "planner-s1"
        ),
        "duplicate session id must fail closed, got {err}"
    );
    assert_eq!(iso.session_count(), 3);
}

// --- AC2: read-only lanes share a worktree and finish with zero edits ------

#[test]
fn read_only_sessions_share_a_worktree_in_parallel() {
    let mut iso = SessionIsolation::new();
    iso.grant(grant("planner-s1", "/work/main", AgentRole::Planner))
        .expect("planner grant");
    iso.grant(grant("scout-s2", "/work/main", AgentRole::Explorer))
        .expect("scout grant");
    iso.grant(grant("tester-s3", "/work/main", AgentRole::Tester))
        .expect("test grant");
    iso.grant(grant("reviewer-s4", "/work/main", AgentRole::Reviewer))
        .expect("reviewer grant");

    // Holders are reported in session-id order (a deterministic registry
    // order), not grant order.
    assert_eq!(
        iso.holders("/work/main"),
        vec![
            "planner-s1".to_string(),
            "reviewer-s4".to_string(),
            "scout-s2".to_string(),
            "tester-s3".to_string()
        ]
    );
    assert_eq!(iso.writer("/work/main"), None);
    assert_eq!(iso.session_count(), 4);
}

#[test]
fn read_only_session_reporting_edits_fails_closed() {
    let mut iso = SessionIsolation::new();
    iso.grant(grant("planner-s1", "/work/main", AgentRole::Planner))
        .expect("planner grant");

    let err = iso
        .finish(
            "planner-s1",
            ObservedEdits {
                files_edited: 1,
                lines_changed: 12,
            },
        )
        .unwrap_err();
    assert!(
        matches!(
            err,
            IsolationViolation::ReadOnlyBreach {
                ref session_id,
                files_edited: 1,
                lines_changed: 12
            } if session_id == "planner-s1"
        ),
        "a read-only session that edited files must fail closed, got {err}"
    );
    assert!(!iso.is_active("planner-s1"));

    // Zero edits is the only acceptable finish for a read-only role.
    iso.grant(grant("reviewer-s2", "/work/main", AgentRole::Reviewer))
        .expect("reviewer grant");
    iso.finish("reviewer-s2", ObservedEdits::none())
        .expect("a zero-edit reviewer finish must be accepted");
}

// --- AC3: modifying sessions get independent worktrees ----------------------

#[test]
fn two_mutating_sessions_use_two_distinct_worktrees() {
    let mut iso = SessionIsolation::new();
    iso.grant(grant("builder-a", "/work/feat-a", AgentRole::Implementer))
        .expect("first builder");
    iso.grant(grant("builder-b", "/work/feat-b", AgentRole::Implementer))
        .expect("second builder on its own worktree");

    assert_eq!(iso.writer("/work/feat-a"), Some("builder-a"));
    assert_eq!(iso.writer("/work/feat-b"), Some("builder-b"));
    assert_eq!(iso.holders("/work/feat-a"), vec!["builder-a".to_string()]);
    assert_eq!(iso.holders("/work/feat-b"), vec!["builder-b".to_string()]);
}

#[test]
fn two_mutating_sessions_collide_on_one_worktree_and_fail_closed() {
    let mut iso = SessionIsolation::new();
    iso.grant(grant("builder-a", "/work/feat", AgentRole::Implementer))
        .expect("first builder");

    let err = iso
        .grant(grant("builder-b", "/work/feat", AgentRole::Implementer))
        .unwrap_err();
    assert!(
        matches!(
            err,
            IsolationViolation::WorktreeCollision {
                ref session_id,
                ref worktree,
                ref held_by
            } if session_id == "builder-b"
                && worktree == "/work/feat"
                && held_by == "builder-a"
        ),
        "a second writer on the same worktree must fail closed, got {err}"
    );
    assert!(!iso.is_active("builder-b"));
    assert_eq!(iso.session_count(), 1);
    assert_eq!(iso.writer("/work/feat"), Some("builder-a"));

    // The collision is about the policy, not the role name: a documentation
    // writer is also a mutating lane.
    let err = iso
        .grant(grant(
            "doc-writer",
            "/work/feat",
            AgentRole::DocumentationWriter,
        ))
        .unwrap_err();
    assert!(
        matches!(err, IsolationViolation::WorktreeCollision { .. }),
        "any mutating lane collides with the active writer, got {err}"
    );
}

#[test]
fn read_only_sessions_may_join_a_writer_worktree() {
    let mut iso = SessionIsolation::new();
    iso.grant(grant("builder-s1", "/work/feat", AgentRole::Implementer))
        .expect("builder grant");
    iso.grant(grant("tester-s2", "/work/feat", AgentRole::Tester))
        .expect("test session reads the builder worktree");
    iso.grant(grant("reviewer-s3", "/work/feat", AgentRole::Reviewer))
        .expect("reviewer reads the builder worktree");

    assert_eq!(iso.writer("/work/feat"), Some("builder-s1"));
    assert_eq!(
        iso.holders("/work/feat"),
        vec![
            "builder-s1".to_string(),
            "reviewer-s3".to_string(),
            "tester-s2".to_string()
        ]
    );
}

#[test]
fn writer_claim_releases_when_the_builder_finishes() {
    let mut iso = SessionIsolation::new();
    iso.grant(grant("builder-a", "/work/feat", AgentRole::Implementer))
        .expect("first builder");
    iso.finish(
        "builder-a",
        ObservedEdits {
            files_edited: 3,
            lines_changed: 40,
        },
    )
    .expect("a builder may report edits");
    assert_eq!(iso.writer("/work/feat"), None);

    iso.grant(grant("builder-b", "/work/feat", AgentRole::Implementer))
        .expect("the released worktree admits the next writer");
    assert_eq!(iso.writer("/work/feat"), Some("builder-b"));
}

// --- grant well-formedness --------------------------------------------------

#[test]
fn policy_mismatch_fails_closed() {
    let mut iso = SessionIsolation::new();
    let reviewer_with_builder_policy = SessionGrant {
        session_id: "reviewer-s1".to_string(),
        worktree: "/work/feat".to_string(),
        role: AgentRole::Reviewer,
        policy: RolePolicy::Builder,
    };
    let err = iso.grant(reviewer_with_builder_policy).unwrap_err();
    assert!(
        matches!(
            err,
            IsolationViolation::PolicyMismatch {
                role: AgentRole::Reviewer,
                policy: RolePolicy::Builder
            }
        ),
        "a reviewer holding the builder policy must be refused, got {err}"
    );

    let planner_with_reviewer_policy = SessionGrant {
        session_id: "planner-s1".to_string(),
        worktree: "/work/main".to_string(),
        role: AgentRole::Planner,
        policy: RolePolicy::Reviewer,
    };
    assert!(matches!(
        iso.grant(planner_with_reviewer_policy),
        Err(IsolationViolation::PolicyMismatch { .. })
    ));
    assert_eq!(iso.session_count(), 0);
}

#[test]
fn empty_grant_fields_fail_closed() {
    let mut iso = SessionIsolation::new();
    assert!(matches!(
        iso.grant(grant("", "/work/main", AgentRole::Planner)),
        Err(IsolationViolation::EmptyField {
            field: "session_id"
        })
    ));
    assert!(matches!(
        iso.grant(grant("planner-s1", "   ", AgentRole::Planner)),
        Err(IsolationViolation::EmptyField { field: "worktree" })
    ));
    assert_eq!(iso.session_count(), 0);
}

#[test]
fn finishing_an_unknown_session_fails_closed() {
    let mut iso = SessionIsolation::new();
    let err = iso.finish("ghost-s1", ObservedEdits::none()).unwrap_err();
    assert!(
        matches!(
            err,
            IsolationViolation::UnknownSession { ref session_id }
                if session_id == "ghost-s1"
        ),
        "finishing a session that was never granted must fail closed, got {err}"
    );
}

// --- AC4: the reviewer's output validates as approve|changes_required|... ---

#[test]
fn review_verdict_accepts_exactly_three_tokens() {
    assert_eq!(
        ReviewVerdict::variants(),
        [
            ReviewVerdict::Approve,
            ReviewVerdict::ChangesRequired,
            ReviewVerdict::Uncertain
        ]
    );
    for verdict in ReviewVerdict::variants() {
        assert_eq!(ReviewVerdict::parse(verdict.as_str()).unwrap(), verdict);
        assert_eq!(verdict.to_string(), verdict.as_str());
    }
    // Surrounding whitespace is transport noise, not a new token.
    assert_eq!(
        ReviewVerdict::parse("  changes_required\n").unwrap(),
        ReviewVerdict::ChangesRequired
    );
}

// Trailing whitespace ("approve ") is accepted — `parse` treats surrounding
// whitespace as transport noise. Everything below is rejected after trimming.
#[test]
fn review_verdict_rejects_everything_else() {
    for token in [
        "approved",
        "APPROVE",
        "changes-required",
        "changes required",
        "merge",
        "lgtm",
        "uncertainty",
        "approve|changes_required",
        "approve extra",
        "",
        "  ",
    ] {
        let err = ReviewVerdict::parse(token).unwrap_err();
        assert!(
            err.contains("approve|changes_required|uncertain"),
            "rejection must name the accepted tokens, got {err:?} for {token:?}"
        );
    }
}

// --- structured artifacts between sessions ----------------------------------

#[test]
fn session_artifacts_roundtrip_the_wire_format() {
    let artifacts = [
        SessionArtifact::Plan {
            summary: "split the parser into lexer and parser".to_string(),
            steps: vec![
                "extract the lexer".to_string(),
                "rewrite the parser over tokens".to_string(),
            ],
        },
        SessionArtifact::Diff {
            path: "work/feat.diff".to_string(),
            additions: 120,
            deletions: 34,
        },
        SessionArtifact::TestReceipt {
            command: "cargo test --workspace --no-fail-fast".to_string(),
            passed: true,
            failures: 0,
        },
        SessionArtifact::Review {
            verdict: ReviewVerdict::Approve,
            notes: "diff matches the plan; tests green".to_string(),
        },
    ];
    let kinds = ["plan", "diff", "test_receipt", "review"];
    for (artifact, kind) in artifacts.iter().zip(kinds) {
        assert_eq!(artifact.kind(), kind);
        artifact.validate().unwrap();
        let wire = artifact.to_json();
        let back = SessionArtifact::from_json(&wire)
            .unwrap_or_else(|err| panic!("wire roundtrip failed for {kind}: {err}\nwire: {wire}"));
        assert_eq!(&back, artifact);
        assert_eq!(back.kind(), kind);
    }
}

#[test]
fn session_artifact_wire_format_rejects_garbage() {
    let mut iso = SessionIsolation::new();
    iso.grant(grant("planner-s1", "/work/main", AgentRole::Planner))
        .expect("planner grant");

    for wire in [
        r#"{"kind":"verdict","verdict":"approve"}"#,
        r#"{"kind":"plan","steps":["no summary"]}"#,
        r#"{"kind":"diff","path":"a.diff","additions":1}"#,
        "not json at all",
        r#"{"kind":"plan","summary":"x","steps":["a"]"#,
        "",
    ] {
        assert!(
            SessionArtifact::from_json(wire).is_err(),
            "wire format must reject {wire:?}"
        );
    }

    // The one field the reviewer lane must carry is a verdict token.
    let err = SessionArtifact::from_json(
        r#"{"kind":"review","verdict":"approved","notes":"looks fine"}"#,
    )
    .unwrap_err();
    assert!(
        err.contains("`approved`"),
        "a non-token verdict must be rejected at the field, got {err:?}"
    );
}

#[test]
fn session_artifact_validation_refuses_empty_content() {
    assert!(SessionArtifact::Plan {
        summary: "   ".to_string(),
        steps: vec!["step one".to_string()],
    }
    .validate()
    .is_err());
    assert!(SessionArtifact::Plan {
        summary: "a plan".to_string(),
        steps: vec!["one".to_string(), " ".to_string()],
    }
    .validate()
    .is_err());
    assert!(SessionArtifact::Diff {
        path: "".to_string(),
        additions: 1,
        deletions: 0,
    }
    .validate()
    .is_err());
    assert!(SessionArtifact::TestReceipt {
        command: "  ".to_string(),
        passed: true,
        failures: 0,
    }
    .validate()
    .is_err());
    assert!(SessionArtifact::Review {
        verdict: ReviewVerdict::Uncertain,
        notes: "".to_string(),
    }
    .validate()
    .is_err());
}

// --- the full multi-role run --------------------------------------------------

#[test]
fn multi_role_run_end_to_end() {
    let mut iso = SessionIsolation::new();

    // 1. Planner and scout: read-only, parallel, on the main worktree.
    iso.grant(grant("planner-s1", "/work/main", AgentRole::Planner))
        .expect("planner grant");
    iso.grant(grant("scout-s2", "/work/main", AgentRole::Explorer))
        .expect("scout grant");

    // 2. The builder works in its own worktree, independently.
    iso.grant(grant(
        "builder-s3",
        "/work/feat-3324",
        AgentRole::Implementer,
    ))
    .expect("builder grant");
    assert_eq!(
        iso.holders("/work/main"),
        vec!["planner-s1".to_string(), "scout-s2".to_string()]
    );

    // 3. Structured handoff: plan -> builder.
    let plan = SessionArtifact::Plan {
        summary: "isolate the planner, builder and reviewer sessions".to_string(),
        steps: vec![
            "extend the aar contract with a worktree ownership registry".to_string(),
            "wire the read-only finish check and the review verdict token".to_string(),
        ],
    };
    plan.validate().expect("plan carries its content");
    let plan_wire = plan.to_json();

    // 4. The builder finishes with edits.
    iso.finish(
        "builder-s3",
        ObservedEdits {
            files_edited: 4,
            lines_changed: 210,
        },
    )
    .expect("a builder may report edits");

    // 5. Test session and reviewer join the builder's worktree: read-only.
    iso.grant(grant("tester-s4", "/work/feat-3324", AgentRole::Tester))
        .expect("test grant");
    iso.grant(grant("reviewer-s5", "/work/feat-3324", AgentRole::Reviewer))
        .expect("reviewer grant");

    // 6. Structured handoff: diff + test receipt -> reviewer.
    let diff = SessionArtifact::Diff {
        path: "work/feat-3324.diff".to_string(),
        additions: 180,
        deletions: 30,
    };
    let receipt = SessionArtifact::TestReceipt {
        command: "cargo test --workspace --no-fail-fast".to_string(),
        passed: true,
        failures: 0,
    };
    for artifact in [&diff, &receipt] {
        artifact.validate().expect("handoff carries its content");
        SessionArtifact::from_json(&artifact.to_json()).expect("handoff roundtrips");
    }

    // 7. The reviewer returns a verdict token.
    let verdict = ReviewVerdict::parse("approve").expect("verdict token");
    let review = SessionArtifact::Review {
        verdict,
        notes: "sessions are distinct; no read-only edits; worktrees independent".to_string(),
    };
    review.validate().expect("review carries its content");
    let review_back = SessionArtifact::from_json(&review.to_json()).expect("review roundtrips");
    assert_eq!(review_back, review);

    // 8. The read-only sessions finish with zero edits (AC2).
    iso.finish("planner-s1", ObservedEdits::none())
        .expect("planner made no edits");
    iso.finish("scout-s2", ObservedEdits::none())
        .expect("scout made no edits");
    iso.finish("tester-s4", ObservedEdits::none())
        .expect("test session made no edits");
    iso.finish("reviewer-s5", ObservedEdits::none())
        .expect("reviewer made no edits");
    let err = iso.finish("builder-s3", ObservedEdits::none()).unwrap_err();
    assert!(
        matches!(
            err,
            IsolationViolation::UnknownSession { ref session_id }
                if session_id == "builder-s3"
        ),
        "a finished session cannot be finished twice, got {err}"
    );

    // The run used five distinct sessions and two distinct worktrees.
    let worktrees = vec!["/work/main", "/work/feat-3324"];
    assert_eq!(
        worktrees.len(),
        worktrees
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        "the modifying session used its own worktree, distinct from the read-only one"
    );
    assert_eq!(iso.session_count(), 0);
    assert_eq!(iso.writer("/work/feat-3324"), None);
    let _ = plan_wire;
}

#[test]
fn pi_session_results_fold_into_the_isolation_check() {
    let events = vec![
        parse_pi_event("file_read path=src/a.rs").expect("event"),
        parse_pi_event("file_edit path=src/a.rs lines=4").expect("event"),
        parse_pi_event("file_edit path=src/b.rs lines=2").expect("event"),
        parse_pi_event("result success=true summary=done").expect("event"),
    ];

    // The same event stream is a breach when folded under a read-only role...
    let planner_result = fold_events("planner-s1", AgentRole::Planner, &events);
    let edits = ObservedEdits::from(&planner_result);
    assert_eq!(
        edits,
        ObservedEdits {
            files_edited: 2,
            lines_changed: 6
        }
    );
    let mut iso = SessionIsolation::new();
    iso.grant(grant("planner-s1", "/work/main", AgentRole::Planner))
        .expect("planner grant");
    let err = iso.finish("planner-s1", edits).unwrap_err();
    assert!(
        matches!(
            err,
            IsolationViolation::ReadOnlyBreach {
                files_edited: 2,
                lines_changed: 6,
                ..
            }
        ),
        "folded planner edits must trip the breach, got {err}"
    );

    // ...and an acceptable finish when folded under the builder.
    let builder_result = fold_events("builder-s2", AgentRole::Implementer, &events);
    let mut iso = SessionIsolation::new();
    iso.grant(grant("builder-s2", "/work/feat", AgentRole::Implementer))
        .expect("builder grant");
    iso.finish("builder-s2", ObservedEdits::from(&builder_result))
        .expect("the builder may report its edits");

    // A read-only session with a clean event stream passes.
    let clean = fold_events(
        "reviewer-s3",
        AgentRole::Reviewer,
        &[parse_pi_event("result success=true summary=ok").expect("event")],
    );
    let mut iso = SessionIsolation::new();
    iso.grant(grant("reviewer-s3", "/work/feat", AgentRole::Reviewer))
        .expect("reviewer grant");
    iso.finish("reviewer-s3", ObservedEdits::from(&clean))
        .expect("a zero-edit reviewer passes");
}
