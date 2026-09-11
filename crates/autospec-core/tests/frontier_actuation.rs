//! Regression tests for the actuation-gap invariants (issue #4268).
//!
//! Incident configuration: the frontier loop reported **8 ready, 0
//! dispatched** every pass. Both numbers were true. Dispatch was impossible —
//! the actuator (`iw-dispatch.sh`) requires a staged spec per issue, and the
//! producer of staged specs (`iw-stage.sh`) was a manual step outside the
//! loop, run by whoever remembered to run it.
//!
//! The steady-state case alone cannot see this bug: with everything staged,
//! every rule agrees and the loop dispatches. The tests below reconstruct the
//! incident's eight dependency-closed issues with **nothing staged**, and the
//! end-to-end case in `new_issue_is_staged_and_dispatched_by_the_loop_itself`
//! runs the loop against a brand-new issue rather than a pre-staged backlog.

use autospec_core::frontier_actuation::{
    line_names_gap, stage, stop_line_names_guard, Actuator, BlockedCandidate, Candidate,
    FrontierLoop, Guard, LoopVerdict, Precondition, StagingOutcome, TickReport, World,
    MIN_STAGED_SPEC_BYTES,
};

/// A body long enough to be a specification rather than a title restated.
fn real_body(issue: &str) -> String {
    format!(
        "Wire `{issue}` into the frontier loop: stage the body as a spec under \
         `iw/issues/specs/`, then dispatch a worker against the staged path."
    )
}

/// The incident's eight dependency-closed issues (#305, #306, #310, #313 and
/// four more), every one with a real body.
fn incident_candidates() -> Vec<Candidate> {
    ["305", "306", "310", "313", "318", "321", "326", "331"]
        .into_iter()
        .map(|issue| Candidate::new(issue, real_body(issue)))
        .collect()
}

/// The actuator as it was wired on the cluster: it guards `staged-spec` and
/// nothing produces it.
fn incident_actuator() -> Actuator {
    Actuator::new("iw-dispatch").requiring(Precondition::guarded(Guard::StagedSpec))
}

/// The actuator after the fix: the guard wired to the producer that satisfies
/// it, running inside the loop's own pass.
fn fixed_actuator() -> Actuator {
    Actuator::new("iw-dispatch").requiring(Precondition::with_producer(
        Guard::StagedSpec,
        "iw-stage.sh",
    ))
}

// ── Incident reconstruction ─────────────────────────────────────────────────

#[test]
fn incident_reports_ready_and_zero_dispatched_with_an_unstaged_backlog() {
    let loop_ = FrontierLoop::new(incident_actuator());
    let mut world = World::default(); // nobody ran iw-stage.sh

    let report = loop_.tick(&mut world, &incident_candidates());

    assert_eq!(report.observed, 8);
    assert_eq!(report.ready, 8, "every issue is dependency-closed");
    assert_eq!(report.staged, 0, "nothing staged the specs");
    assert_eq!(report.dispatched, 0, "and so nothing could be dispatched");
    assert_eq!(report.gap(), 8, "the gap the old report never printed");
    assert!(report.reconciles());
    assert!(report.stalled());
    assert_eq!(world.dispatched, Vec::<String>::new());
}

#[test]
fn incident_report_line_names_the_block_and_its_reason() {
    let loop_ = FrontierLoop::new(incident_actuator());
    let mut world = World::default();

    let report = loop_.tick(&mut world, &incident_candidates());

    assert_eq!(
        report.line(),
        "8 ready, 0 dispatched, 8 blocked: no staged spec"
    );
    assert_eq!(
        report.blocked.first().map(|b| b.reason.as_str()),
        Some("no staged spec")
    );
    assert!(
        report.blocked.iter().all(|b| b.producerless),
        "every hold is a missing producer, not a producer's refusal"
    );
}

#[test]
fn old_report_line_without_the_gap_is_refused() {
    let loop_ = FrontierLoop::new(incident_actuator());
    let mut world = World::default();
    let report = loop_.tick(&mut world, &incident_candidates());

    // The line the cluster actually printed: two true numbers, no gap.
    assert!(
        !line_names_gap("8 ready, 0 dispatched", &report),
        "a line that prints the gap count nowhere must not satisfy invariant 1"
    );
    // The line the module renders does.
    assert!(line_names_gap(&report.line(), &report));
}

#[test]
fn incident_verdict_is_a_permanent_stop_naming_its_guard() {
    let loop_ = FrontierLoop::new(incident_actuator());
    let mut world = World::default();

    let report = loop_.tick(&mut world, &incident_candidates());
    let verdict = report.verdict();

    assert_eq!(
        verdict,
        LoopVerdict::PermanentStop {
            guard: "staged-spec".to_string(),
            count: 8,
        }
    );
    assert!(verdict.permanent_stop());
    assert!(stop_line_names_guard(&verdict.line(), Guard::StagedSpec));
    assert!(
        verdict.line().contains("permanent stop"),
        "'0 dispatched' must never render as idleness: {}",
        verdict.line()
    );
}

#[test]
fn guard_without_a_producer_is_found_before_it_holds_anything() {
    let loop_ = FrontierLoop::new(incident_actuator());

    // The wiring mistake is visible with an empty world: no candidate needed.
    let findings = loop_.preflight();
    assert_eq!(findings.len(), 1);
    assert!(findings[0].contains("staged-spec"));
    assert!(findings[0].contains("iw-dispatch"));
    assert!(findings[0].contains("permanent stop"));

    let fixed = FrontierLoop::new(fixed_actuator());
    assert!(
        fixed.preflight().is_empty(),
        "a wired guard is not a finding: {:?}",
        fixed.preflight()
    );
}

// ── The fix: the producer inside the loop's pass ────────────────────────────

#[test]
fn producer_in_the_loop_stages_and_dispatches_the_same_backlog() {
    let loop_ = FrontierLoop::new(fixed_actuator());
    let mut world = World::default(); // identical starting world: nothing staged

    let report = loop_.tick(&mut world, &incident_candidates());

    assert_eq!(report.ready, 8);
    assert_eq!(report.staged, 8, "the loop staged the specs itself");
    assert_eq!(report.dispatched, 8);
    assert_eq!(report.gap(), 0);
    assert!(report.blocked.is_empty());
    assert!(world.staged.len() == 8 && world.dispatched.len() == 8);
    assert_eq!(report.line(), "8 ready, 8 staged, 8 dispatched");
    assert_eq!(report.verdict(), LoopVerdict::Acted { dispatched: 8 });
}

#[test]
fn new_issue_is_staged_and_dispatched_by_the_loop_itself() {
    // The end-to-end case the issue asks for: a pre-staged backlog the loop
    // could already dispatch, plus one brand-new issue nobody has staged. The
    // old loop dispatched the backlog and left the new issue "ready" forever.
    let mut world = World::default();
    for issue in ["305", "306", "307"] {
        world.staged.insert(issue.to_string());
    }

    let mut candidates: Vec<Candidate> = ["305", "306", "307"]
        .into_iter()
        .map(|issue| Candidate::new(issue, real_body(issue)))
        .collect();
    let fresh = Candidate::new("310", real_body("310"));
    candidates.push(fresh);

    let loop_ = FrontierLoop::new(fixed_actuator());
    let report = loop_.tick(&mut world, &candidates);

    assert_eq!(report.staged, 1, "only the new issue needed staging");
    assert_eq!(report.dispatched, 4, "the new item is not left in queue");
    assert!(world.is_staged("310"), "the loop produced the spec");
    assert!(world.is_dispatched("310"));
    assert!(report.blocked.is_empty());

    // The same tick against the incident's actuator: #310 is ready, unstaged,
    // and nothing in the system can stage it.
    let mut stale_world = World::default();
    for issue in ["305", "306", "307"] {
        stale_world.staged.insert(issue.to_string());
    }
    let stuck = FrontierLoop::new(incident_actuator()).tick(&mut stale_world, &candidates);
    assert_eq!(stuck.dispatched, 3);
    assert_eq!(stuck.staged, 0);
    assert_eq!(stuck.blocked.len(), 1);
    assert_eq!(stuck.blocked[0].issue, "310");
    assert!(stuck.blocked[0].producerless);
    assert_eq!(
        stuck.line(),
        "4 ready, 3 dispatched, 1 blocked: no staged spec"
    );
}

#[test]
fn pre_staged_item_dispatches_without_running_the_producer() {
    let mut world = World::default();
    world.staged.insert("305".to_string());

    let report = FrontierLoop::new(fixed_actuator())
        .tick(&mut world, &[Candidate::new("305", real_body("305"))]);

    assert_eq!(
        report.staged, 0,
        "the producer is not re-run for held state"
    );
    assert_eq!(report.dispatched, 1);
}

// ── The producer is a refusal, not a bypass ─────────────────────────────────

#[test]
fn staging_refuses_an_empty_or_trivial_body() {
    assert_eq!(
        stage(""),
        StagingOutcome::Refused {
            reason: format!(
                "body too short to implement from (0 bytes, min {MIN_STAGED_SPEC_BYTES})"
            )
        }
    );
    assert!(!stage("   \n\t ").staged(), "whitespace is not a spec");
    assert!(!stage("fix the thing").staged());
    assert!(stage(&real_body("310")).staged());
    assert!(stage(&"x".repeat(MIN_STAGED_SPEC_BYTES)).staged());
}

#[test]
fn a_refused_stage_blocks_with_a_reason_and_is_not_a_stop() {
    let loop_ = FrontierLoop::new(fixed_actuator());
    let mut world = World::default();

    let report = loop_.tick(&mut world, &[Candidate::new("322", "fix the thing")]);

    assert_eq!(report.ready, 1);
    assert_eq!(report.staged, 0);
    assert_eq!(
        report.dispatched, 0,
        "refusal never dispatches onto nothing"
    );
    let BlockedCandidate {
        reason,
        producerless,
        guard,
        ..
    } = report.blocked.first().expect("one blocked item");
    assert!(reason.contains("stage refused"));
    assert!(reason.contains("too short"));
    assert!(!producerless, "a real refusal has a release: a better body");
    assert_eq!(*guard, Guard::StagedSpec);
    assert!(!report.verdict().permanent_stop());
    assert_eq!(
        report.verdict(),
        LoopVerdict::BlockedWithProducer {
            reason: reason.clone(),
            count: 1,
        }
    );
    assert!(!world.is_staged("322"));
}

// ── Report integrity ────────────────────────────────────────────────────────

#[test]
fn unready_items_are_observed_but_never_ready() {
    let loop_ = FrontierLoop::new(fixed_actuator());
    let mut world = World::default();

    let report = loop_.tick(
        &mut world,
        &[
            Candidate::new("305", real_body("305")),
            Candidate::waiting("309", real_body("309")),
        ],
    );

    assert_eq!(report.observed, 2, "the denominator counts both");
    assert_eq!(report.ready, 1, "only the dependency-closed one is work");
    assert_eq!(report.dispatched, 1);
    assert!(report.reconciles());
    assert!(!world.is_dispatched("309"));
}

#[test]
fn a_report_that_does_not_reconcile_is_a_defect() {
    // Ready work with neither a dispatch nor a block reason: a state that
    // cannot exist, printed as if it could.
    let hole = TickReport {
        observed: 8,
        ready: 8,
        staged: 0,
        dispatched: 0,
        blocked: Vec::new(),
    };
    assert_eq!(hole.gap(), 8);
    assert!(!hole.reconciles());
    // And the line it renders is refused by invariant 1 too: a gap with no
    // blocked entries cannot be stated honestly, so the report has no honest
    // line to render. `reconciles()` and `line_names_gap()` are separate
    // assertions and this fixture fails both.
    assert!(!line_names_gap(&hole.line(), &hole));
    assert_eq!(hole.line(), "8 ready, 0 dispatched");

    let overdrawn = TickReport {
        observed: 8,
        ready: 1,
        staged: 0,
        dispatched: 1,
        blocked: vec![BlockedCandidate {
            issue: "305".to_string(),
            guard: Guard::StagedSpec,
            reason: "no staged spec".to_string(),
            producerless: true,
        }],
    };
    assert!(!overdrawn.reconciles());
}

#[test]
fn mixed_reasons_are_grouped_and_both_named_in_the_line() {
    let loop_ = FrontierLoop::new(fixed_actuator());
    let mut world = World::default();

    let report = loop_.tick(
        &mut world,
        &[
            Candidate::new("305", real_body("305")),
            Candidate::new("322", "fix the thing"),
        ],
    );

    assert_eq!(report.ready, 2);
    assert_eq!(report.dispatched, 1);
    assert_eq!(report.gap(), 1);
    assert!(report.reconciles());
    assert_eq!(report.distinct_reasons().len(), 1);
    assert!(line_names_gap(&report.line(), &report));
    assert!(report.line().contains("1 blocked: stage refused"));
    // A mixed report — one transient refusal, one missing producer — is a
    // stop: the unownable hold dominates the pass.
    let mixed = TickReport {
        observed: 2,
        ready: 2,
        staged: 0,
        dispatched: 0,
        blocked: vec![
            BlockedCandidate {
                issue: "322".to_string(),
                guard: Guard::StagedSpec,
                reason: "stage refused: body too short".to_string(),
                producerless: false,
            },
            BlockedCandidate {
                issue: "310".to_string(),
                guard: Guard::StagedSpec,
                reason: "no staged spec".to_string(),
                producerless: true,
            },
        ],
    };
    assert_eq!(
        mixed.verdict(),
        LoopVerdict::PermanentStop {
            guard: "staged-spec".to_string(),
            count: 1,
        }
    );
    assert_eq!(mixed.distinct_reasons().len(), 2);
    assert!(line_names_gap(&mixed.line(), &mixed));
    assert!(mixed.line().contains("1 blocked: no staged spec"));
}

#[test]
fn an_idle_pass_is_not_rendered_as_a_stop() {
    let loop_ = FrontierLoop::new(incident_actuator());
    let mut world = World::default();

    let report = loop_.tick(&mut world, &[Candidate::waiting("309", real_body("309"))]);

    assert_eq!(report.ready, 0);
    assert_eq!(report.verdict(), LoopVerdict::Idle);
    assert_eq!(report.line(), "0 ready, 0 dispatched");
    assert!(!report.stalled());
    assert!(line_names_gap(&report.line(), &report));
}
