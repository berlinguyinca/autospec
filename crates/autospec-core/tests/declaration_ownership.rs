//! Declaration-ownership contract for control loops (issue #3759).
//!
//! Where a control loop owns a resource, change the declaration, never the
//! resource. The three acceptance scenarios:
//!
//! * declared parameters (slots, hours, partition) are reviewed against the
//!   workload prompt ceiling before they take effect (issue #3749);
//! * the declaration is the loop's only capacity input — there is no API
//!   path that submits or cancels a worker directly;
//! * every cancellation of a surplus worker is logged with its reason,
//!   naming the declaration being enforced.

use autospec_core::declaration_ownership::{
    validate_declaration, CapacityDeclaration, CapacityReconciler, DeclarationError, LiveWorker,
    RevertNotice,
};

const WINDOW: u64 = 262144;

/// The incident declaration from issue #3759: `desired.sh set
/// qwen3.8-27b 9 --hours 4 --slots 8` — two fields that were never
/// reviewed against the workload.
fn incident_declaration() -> CapacityDeclaration {
    CapacityDeclaration {
        model: "qwen3.8-27b".to_owned(),
        want: 9,
        part: "low".to_owned(),
        hours: 4,
        slots: 8,
    }
}

/// A worker the loop submitted from `decl`.
fn loop_worker(job_id: u64, decl: &CapacityDeclaration) -> LiveWorker {
    LiveWorker {
        job_id,
        model: decl.model.clone(),
        part: decl.part.clone(),
        hours: decl.hours,
        slots: decl.slots,
    }
}

// -- Acceptance 2: declared parameters reviewed against the prompt
//    ceiling, not left at defaults (#3749).

#[test]
fn slots_below_the_prompt_ceiling_are_rejected() {
    // The incident arithmetic: 262144 / 8 = 32768 tokens/slot, below a
    // fleet whose smallest prompt does not fit 32768.
    let ceiling = 65536;
    let decl = incident_declaration();
    assert_eq!(
        validate_declaration(&decl, WINDOW, ceiling),
        Err(DeclarationError::TokensPerSlotBelowCeiling {
            window: WINDOW,
            slots: 8,
            tokens_per_slot: 32768,
            prompt_ceiling: ceiling,
        })
    );
    // The operator's fix — the same two fields, reviewed — passes.
    let mut fixed = decl.clone();
    fixed.hours = 168;
    fixed.slots = 4;
    assert_eq!(validate_declaration(&fixed, WINDOW, ceiling), Ok(()));
}

#[test]
fn slots_that_exactly_cover_the_ceiling_pass() {
    let mut decl = incident_declaration();
    decl.slots = 4;
    // 262144 / 4 = 65536 = ceiling: covering the ceiling exactly is
    // enough.
    assert_eq!(validate_declaration(&decl, WINDOW, 65536), Ok(()));
}

#[test]
fn zero_slots_zero_hours_and_empty_names_are_rejected() {
    let mut decl = incident_declaration();
    decl.slots = 0;
    assert_eq!(
        validate_declaration(&decl, WINDOW, 0),
        Err(DeclarationError::ZeroSlots)
    );
    decl.slots = 4;
    decl.hours = 0;
    assert_eq!(
        validate_declaration(&decl, WINDOW, 0),
        Err(DeclarationError::ZeroHours)
    );
    decl.hours = 4;
    decl.model = "  ".to_owned();
    assert_eq!(
        validate_declaration(&decl, WINDOW, 0),
        Err(DeclarationError::EmptyModel)
    );
    decl.model = "qwen3.8-27b".to_owned();
    decl.part = "".to_owned();
    assert_eq!(
        validate_declaration(&decl, WINDOW, 0),
        Err(DeclarationError::EmptyPartition)
    );
}

#[test]
fn declaration_errors_are_displayable_with_the_parameters() {
    let err = DeclarationError::TokensPerSlotBelowCeiling {
        window: WINDOW,
        slots: 8,
        tokens_per_slot: 32768,
        prompt_ceiling: 65536,
    };
    let message = err.to_string();
    assert!(message.contains("32768"), "{message}");
    assert!(message.contains("65536"), "{message}");
    assert!(message.contains("8"), "{message}");
}

// -- Acceptance 1: the declaration is the only capacity input.

#[test]
fn a_declaration_that_fails_the_review_never_takes_effect() {
    // A ceiling the incident declaration passes: the bad declaration is
    // *valid* but bad for the workload; only the review catches it.
    let mut reconciler = CapacityReconciler::new(incident_declaration(), WINDOW, 16384).unwrap();
    let mut bad = incident_declaration();
    bad.slots = 32; // 262144 / 32 = 8192 < 16384
    assert!(matches!(
        reconciler.set_declaration(bad),
        Err(DeclarationError::TokensPerSlotBelowCeiling { .. })
    ));
    // Fail-closed: the previous declaration is still in force.
    assert_eq!(reconciler.declaration().slots, 8);
    assert_eq!(reconciler.declaration().want, 9);
}

// -- The incident: a manual capacity fix reverted on the loop's own
//    schedule, and the revert must name the declaration it enforces.

#[test]
fn surplus_is_trimmed_newest_first_with_a_notice_naming_the_declaration() {
    let decl = incident_declaration();
    let reconciler = CapacityReconciler::new(decl.clone(), WINDOW, 16384).unwrap();

    // Nine loop workers on the bad declaration, plus three manual
    // replacements the operator submitted with the correct slot count.
    let mut live: Vec<LiveWorker> = (1..=9).map(|id| loop_worker(id, &decl)).collect();
    for id in 10..=12 {
        live.push(LiveWorker {
            job_id: id,
            model: decl.model.clone(),
            part: decl.part.clone(),
            hours: 168,
            slots: 4,
        });
    }

    let report = reconciler.reconcile(&live);
    assert_eq!(report.submit, 0);
    // Newest first: the operator's just-submitted replacements (the
    // incident), not the loop's own workers.
    assert_eq!(
        report.cancels.iter().map(|n| n.job_id).collect::<Vec<_>>(),
        vec![12, 11, 10]
    );
    for notice in &report.cancels {
        assert_eq!(notice.declaration, decl);
        assert_eq!(notice.live, 12);
        let line = notice.log_line();
        // The log names the declaration being enforced and says why.
        assert!(line.contains("qwen3.8-27b"), "{line}");
        assert!(line.contains("want 9"), "{line}");
        assert!(line.contains("12 live"), "{line}");
        assert!(line.contains("CANCEL"), "{line}");
    }
}

#[test]
fn the_fix_goes_through_the_declaration_not_the_resource() {
    // The incident declaration does not survive the review at the real
    // ceiling — the loop can no longer even hold it.
    let held: Result<CapacityReconciler, DeclarationError> =
        CapacityReconciler::new(incident_declaration(), WINDOW, 65536);
    assert!(matches!(
        held,
        Err(DeclarationError::TokensPerSlotBelowCeiling { .. })
    ));

    // The operator's fix, expressed as a declaration change:
    let mut reconciler = CapacityReconciler::new(
        CapacityDeclaration {
            model: "qwen3.8-27b".to_owned(),
            want: 9,
            part: "low".to_owned(),
            hours: 168,
            slots: 4,
        },
        WINDOW,
        65536,
    )
    .unwrap();
    reconciler
        .set_declaration(CapacityDeclaration {
            want: 12,
            ..reconciler.declaration().clone()
        })
        .unwrap();

    // Twelve live workers (nine on the old parameters, three manual)
    // now match the declaration: nothing is trimmed.
    let mut live: Vec<LiveWorker> = (1..=9)
        .map(|id| loop_worker(id, &incident_declaration()))
        .collect();
    for id in 10..=12 {
        live.push(LiveWorker {
            job_id: id,
            model: "qwen3.8-27b".to_owned(),
            part: "low".to_owned(),
            hours: 168,
            slots: 4,
        });
    }
    let report = reconciler.reconcile(&live);
    assert_eq!(report.cancels, Vec::<RevertNotice>::new());
    assert_eq!(report.submit, 0);
}

#[test]
fn shortfall_is_met_by_submits_derived_from_the_declaration() {
    let decl = incident_declaration();
    let reconciler = CapacityReconciler::new(decl, WINDOW, 16384).unwrap();
    let live: Vec<LiveWorker> = (1..=6)
        .map(|id| loop_worker(id, &reconciler.declaration()))
        .collect();
    let report = reconciler.reconcile(&live);
    assert_eq!(report.submit, 3);
    assert_eq!(report.cancels, Vec::<RevertNotice>::new());
}

#[test]
fn workers_of_other_models_are_not_this_declaration_resource() {
    let decl = incident_declaration();
    let reconciler = CapacityReconciler::new(decl.clone(), WINDOW, 16384).unwrap();
    let mut live: Vec<LiveWorker> = (1..=9).map(|id| loop_worker(id, &decl)).collect();
    live.push(LiveWorker {
        job_id: 100,
        model: "qwen3.8-30b".to_owned(),
        part: "low".to_owned(),
        hours: 168,
        slots: 4,
    });
    let report = reconciler.reconcile(&live);
    assert_eq!(report.cancels, Vec::<RevertNotice>::new());
    assert_eq!(report.submit, 0);
}

#[test]
fn reconcile_is_deterministic_for_a_given_observation() {
    let decl = incident_declaration();
    let reconciler = CapacityReconciler::new(decl, WINDOW, 16384).unwrap();
    let live: Vec<LiveWorker> = (1..=11)
        .map(|id| loop_worker(id, &reconciler.declaration()))
        .collect();
    assert_eq!(reconciler.reconcile(&live), reconciler.reconcile(&live));
    let report = reconciler.reconcile(&live);
    assert_eq!(
        report.cancels.iter().map(|n| n.job_id).collect::<Vec<_>>(),
        vec![11, 10]
    );
}

#[test]
fn scaling_to_zero_cancels_every_worker_with_a_notice() {
    let mut decl = incident_declaration();
    decl.want = 0;
    let reconciler = CapacityReconciler::new(decl, WINDOW, 16384).unwrap();
    let live: Vec<LiveWorker> = (1..=3)
        .map(|id| loop_worker(id, &reconciler.declaration()))
        .collect();
    let report = reconciler.reconcile(&live);
    assert_eq!(report.cancels.len(), 3);
    assert!(report.cancels[0].log_line().contains("want 0"));
}

// -- The split (issue #4379): the reconciler was the only component that
//    could submit a missing worker, and it was pinned to dry-run because
//    its other half trims surplus. The safe half must be able to run
//    without the destructive one, or a remover (the wedged-worker
//    rotation) with no live replacer drains the fleet.

#[test]
fn no_trim_submits_the_deficit_that_the_disabled_reconciler_never_saw() {
    // The incident shape: rotated workers left the fleet at 5 against a
    // desired 12; nothing ever submitted the missing 7.
    let decl = incident_declaration();
    let reconciler =
        CapacityReconciler::with_no_trim(CapacityDeclaration { want: 12, ..decl }, WINDOW, 16384)
            .unwrap();
    assert!(reconciler.no_trim());

    let live: Vec<LiveWorker> = (1..=5)
        .map(|id| loop_worker(id, reconciler.declaration()))
        .collect();
    let report = reconciler.reconcile(&live);
    assert_eq!(report.submit, 7);
    assert_eq!(report.cancels, Vec::<RevertNotice>::new());
}

#[test]
fn no_trim_leaves_surplus_alone_instead_of_cancelling_warm_workers() {
    // Twelve live, want nine: the default mode cancels the newest three,
    // the no-trim mode submits nothing and cancels nothing.
    let decl = incident_declaration();
    let live: Vec<LiveWorker> = (1..=12).map(|id| loop_worker(id, &decl)).collect();

    let trimming = CapacityReconciler::new(decl.clone(), WINDOW, 16384).unwrap();
    assert!(!trimming.no_trim());
    let report = trimming.reconcile(&live);
    assert_eq!(report.submit, 0);
    assert_eq!(
        report.cancels.iter().map(|n| n.job_id).collect::<Vec<_>>(),
        vec![12, 11, 10]
    );

    let no_trim = CapacityReconciler::with_no_trim(decl, WINDOW, 16384).unwrap();
    let report = no_trim.reconcile(&live);
    assert_eq!(report.submit, 0);
    assert_eq!(report.cancels, Vec::<RevertNotice>::new());
}

#[test]
fn no_trim_at_exact_want_is_idle_and_at_zero_want_cancels_nothing() {
    let decl = incident_declaration();
    let reconciler = CapacityReconciler::with_no_trim(decl.clone(), WINDOW, 16384).unwrap();
    let live: Vec<LiveWorker> = (1..=9)
        .map(|id| loop_worker(id, reconciler.declaration()))
        .collect();
    let report = reconciler.reconcile(&live);
    assert_eq!(report.submit, 0);
    assert_eq!(report.cancels, Vec::<RevertNotice>::new());

    // Scaling to zero is a declaration the operator must enforce with the
    // destructive half on; no-trim must not cancel a single worker.
    let mut decl_zero = decl;
    decl_zero.want = 0;
    let zero = CapacityReconciler::with_no_trim(decl_zero, WINDOW, 16384).unwrap();
    let report = zero.reconcile(&live);
    assert_eq!(report.submit, 0);
    assert_eq!(report.cancels, Vec::<RevertNotice>::new());
}

#[test]
fn no_trim_still_reviews_the_declaration_and_ignores_other_models() {
    // The declaration review is identical in both modes: a declaration
    // that fails it never takes effect, no-trim or not.
    let mut decl = incident_declaration();
    decl.slots = 32; // 262144 / 32 = 8192 < 16384
    assert!(matches!(
        CapacityReconciler::with_no_trim(decl.clone(), WINDOW, 16384),
        Err(DeclarationError::TokensPerSlotBelowCeiling { .. })
    ));

    let decl = incident_declaration();
    let reconciler = CapacityReconciler::with_no_trim(decl, WINDOW, 16384).unwrap();
    let live = vec![LiveWorker {
        job_id: 100,
        model: "qwen3.8-30b".to_owned(),
        part: "low".to_owned(),
        hours: 168,
        slots: 4,
    }];
    // Other model's surplus is not this declaration's resource and is left
    // alone even though the declared model is at zero live.
    let report = reconciler.reconcile(&live);
    assert_eq!(report.submit, 9);
    assert_eq!(report.cancels, Vec::<RevertNotice>::new());
}
