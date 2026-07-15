use autospec_core::coordination::{
    ConductorEvent, ConductorOutcome, ConductorPhase, ConductorScope, ConductorState,
};

#[test]
fn serialized_high_priority_completion_returns_to_scan() {
    let selected = conductor(ConductorScope::Repository)
        .transition(ConductorEvent::ScanFoundWork)
        .expect("scan finds work")
        .transition(ConductorEvent::SafetyReviewed)
        .expect("review passes")
        .transition(ConductorEvent::Selected {
            issue: 42,
            serialization_reasons: vec!["priority:high".to_string()],
        })
        .expect("selection is valid");
    let next = selected
        .transition(ConductorEvent::Claimed)
        .expect("claim is recorded")
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::Succeeded,
        })
        .expect("outcome is recorded")
        .transition(ConductorEvent::Reconciled)
        .expect("result is reconciled");

    assert_eq!(next.phase(), ConductorPhase::Scan);
    assert_eq!(next.selected_issue(), None);
}

#[test]
fn retryable_dispatch_keeps_the_selected_issue_nonterminal() {
    let state = selected_state()
        .transition(ConductorEvent::Claimed)
        .expect("claim is recorded")
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::Retryable("validation failed".to_string()),
        })
        .expect("retryable outcome is recorded");

    assert_eq!(state.phase(), ConductorPhase::Retry);
    assert_eq!(state.selected_issue(), Some(42));
    assert_eq!(state.retry_count(), 1);
    assert_eq!(state.terminal_reason(), None);
}

#[test]
fn retry_limit_pauses_before_another_dispatch() {
    let state = conductor_with_retry_limit(ConductorScope::Repository, 0)
        .transition(ConductorEvent::ScanFoundWork)
        .expect("scan finds work")
        .transition(ConductorEvent::SafetyReviewed)
        .expect("review passes")
        .transition(ConductorEvent::Selected {
            issue: 42,
            serialization_reasons: Vec::new(),
        })
        .expect("selection is valid")
        .transition(ConductorEvent::Claimed)
        .expect("claim is recorded")
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::Retryable("validation failed".to_string()),
        })
        .expect("outcome is recorded");

    assert_eq!(state.phase(), ConductorPhase::Paused);
    assert_eq!(state.selected_issue(), Some(42));
    assert_eq!(state.pause_reason(), Some("retry_limit_exhausted"));
}

#[test]
fn exhausted_retry_requires_explicit_abandonment_before_a_new_scan() {
    let exhausted = exhausted_retry_state();

    assert!(exhausted
        .clone()
        .transition(ConductorEvent::Resume)
        .is_err());
    let abandoned = exhausted
        .transition(ConductorEvent::AbandonExhausted)
        .expect("explicit abandonment is valid");
    assert_eq!(abandoned.phase(), ConductorPhase::Scan);
    assert_eq!(abandoned.selected_issue(), None);
    assert_eq!(abandoned.retry_count(), 1);
}

#[test]
fn new_selection_resets_the_previous_issue_retry_count() {
    let state = selected_state()
        .transition(ConductorEvent::Claimed)
        .expect("claim is recorded")
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::Retryable("validation failed".to_string()),
        })
        .expect("retryable outcome is recorded")
        .transition(ConductorEvent::RetryScheduled)
        .expect("retry is scheduled")
        .transition(ConductorEvent::Claimed)
        .expect("retry claim is recorded")
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::Succeeded,
        })
        .expect("retry success is recorded")
        .transition(ConductorEvent::Reconciled)
        .expect("retry success is reconciled")
        .transition(ConductorEvent::ScanFoundWork)
        .expect("next scan finds work")
        .transition(ConductorEvent::SafetyReviewed)
        .expect("next review passes")
        .transition(ConductorEvent::Selected {
            issue: 43,
            serialization_reasons: Vec::new(),
        })
        .expect("next selection is valid");

    assert_eq!(state.selected_issue(), Some(43));
    assert_eq!(state.retry_count(), 0);
}

#[test]
fn pause_and_resume_preserve_the_selected_issue_and_retry_count() {
    let paused = selected_state()
        .transition(ConductorEvent::Pause {
            reason: "operator requested stop".to_string(),
        })
        .expect("pause is valid");
    let encoded = paused.to_json();
    let restored = ConductorState::parse_json(&encoded)
        .expect("persisted pause state parses")
        .transition(ConductorEvent::Resume)
        .expect("resume is valid");

    assert_eq!(restored.phase(), ConductorPhase::Claim);
    assert_eq!(restored.selected_issue(), Some(42));
    assert_eq!(restored.retry_count(), 0);
    assert_eq!(restored.pause_reason(), None);
}

#[test]
fn constrained_empty_scan_is_slice_complete_not_all_done() {
    let state = conductor(ConductorScope::Slice)
        .transition(ConductorEvent::ScanEmpty)
        .expect("empty slice is a decision");

    assert_eq!(state.phase(), ConductorPhase::SliceComplete);
    assert_eq!(state.terminal_reason(), Some("slice_empty"));
}

#[test]
fn repository_empty_scan_is_all_done() {
    let state = conductor(ConductorScope::Repository)
        .transition(ConductorEvent::ScanEmpty)
        .expect("empty repository is a decision");

    assert_eq!(state.phase(), ConductorPhase::AllDone);
    assert_eq!(state.terminal_reason(), Some("repository_empty"));
}

#[test]
fn reconciled_state_keeps_the_recorded_outcome_for_resume_audit() {
    let state = selected_state()
        .transition(ConductorEvent::Claimed)
        .expect("claim is recorded")
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::Succeeded,
        })
        .expect("outcome is recorded")
        .transition(ConductorEvent::Reconciled)
        .expect("result is reconciled");
    let restored = ConductorState::parse_json(&state.to_json()).expect("state parses");

    assert_eq!(restored.phase(), ConductorPhase::Scan);
    assert_eq!(restored.selected_issue(), None);
    assert_eq!(restored.last_outcome(), Some(&ConductorOutcome::Succeeded));
}

#[test]
fn persisted_state_rejects_unknown_fields() {
    let state = selected_state();
    let encoded = state.to_json().replace('}', ",\"unexpected\":true}");

    assert!(ConductorState::parse_json(&encoded).is_err());
}

#[test]
fn persisted_state_rejects_terminal_scope_and_reason_forgery() {
    let slice = conductor(ConductorScope::Slice)
        .transition(ConductorEvent::ScanEmpty)
        .expect("empty slice is a decision")
        .to_json()
        .replace("\"phase\":\"slice_complete\"", "\"phase\":\"all_done\"");
    let repository = conductor(ConductorScope::Repository)
        .transition(ConductorEvent::ScanEmpty)
        .expect("empty repository is a decision")
        .to_json()
        .replace("\"phase\":\"all_done\"", "\"phase\":\"slice_complete\"");

    assert!(ConductorState::parse_json(&slice).is_err());
    assert!(ConductorState::parse_json(&repository).is_err());
}

#[test]
fn persisted_paused_state_cannot_resume_into_a_terminal_phase() {
    let paused = selected_state()
        .transition(ConductorEvent::Pause {
            reason: "operator requested stop".to_string(),
        })
        .expect("pause is valid")
        .to_json()
        .replace(
            "\"resume_phase\":\"claim\"",
            "\"resume_phase\":\"all_done\"",
        );

    assert!(ConductorState::parse_json(&paused).is_err());
}

#[test]
fn persisted_paused_state_rejects_an_impossible_resume_projection() {
    let paused = selected_state()
        .transition(ConductorEvent::Pause {
            reason: "operator requested stop".to_string(),
        })
        .expect("pause is valid")
        .to_json();
    let missing_claim = paused.replace("\"selected_issue\":42", "\"selected_issue\":null");
    let invalid_retry = paused.replace("\"resume_phase\":\"claim\"", "\"resume_phase\":\"retry\"");

    assert!(ConductorState::parse_json(&missing_claim).is_err());
    assert!(ConductorState::parse_json(&invalid_retry).is_err());
}

#[test]
fn selected_issue_must_be_a_positive_identifier() {
    let zero_from_event = conductor(ConductorScope::Repository)
        .transition(ConductorEvent::ScanFoundWork)
        .expect("scan finds work")
        .transition(ConductorEvent::SafetyReviewed)
        .expect("review passes")
        .transition(ConductorEvent::Selected {
            issue: 0,
            serialization_reasons: Vec::new(),
        });
    let zero_from_json = selected_state()
        .to_json()
        .replace("\"selected_issue\":42", "\"selected_issue\":0");

    assert!(zero_from_event.is_err());
    assert!(ConductorState::parse_json(&zero_from_json).is_err());
}

#[test]
fn retry_count_overflow_is_rejected_from_valid_persisted_state() {
    let dispatch = selected_state()
        .transition(ConductorEvent::Claimed)
        .expect("claim is recorded")
        .to_json()
        .replace("\"retry_count\":0", "\"retry_count\":4294967295")
        .replace("\"retry_limit\":2", "\"retry_limit\":4294967295");
    let dispatch = ConductorState::parse_json(&dispatch).expect("persisted dispatch is valid");

    assert!(dispatch
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::Retryable("validation failed".to_string()),
        })
        .is_err());
}

#[test]
fn transition_rejects_empty_non_success_outcome_reasons() {
    let dispatch = selected_state()
        .transition(ConductorEvent::Claimed)
        .expect("claim is recorded");

    assert!(dispatch
        .clone()
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::Retryable(String::new()),
        })
        .is_err());
    assert!(dispatch
        .clone()
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::Blocked(String::new()),
        })
        .is_err());
    assert!(dispatch
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::Blocked("retry_limit_exhausted".to_string()),
        })
        .is_err());
}

#[test]
fn constructor_rejects_an_empty_repository() {
    assert!(ConductorState::new("", ConductorScope::Repository, 2).is_err());
}

fn selected_state() -> ConductorState {
    conductor(ConductorScope::Repository)
        .transition(ConductorEvent::ScanFoundWork)
        .expect("scan finds work")
        .transition(ConductorEvent::SafetyReviewed)
        .expect("review passes")
        .transition(ConductorEvent::Selected {
            issue: 42,
            serialization_reasons: Vec::new(),
        })
        .expect("selection is valid")
}

fn conductor(scope: ConductorScope) -> ConductorState {
    conductor_with_retry_limit(scope, 2)
}

fn conductor_with_retry_limit(scope: ConductorScope, retry_limit: u32) -> ConductorState {
    ConductorState::new("berlinguyinca/autospec", scope, retry_limit).expect("repository is valid")
}

fn exhausted_retry_state() -> ConductorState {
    conductor_with_retry_limit(ConductorScope::Repository, 0)
        .transition(ConductorEvent::ScanFoundWork)
        .expect("scan finds work")
        .transition(ConductorEvent::SafetyReviewed)
        .expect("review passes")
        .transition(ConductorEvent::Selected {
            issue: 42,
            serialization_reasons: Vec::new(),
        })
        .expect("selection is valid")
        .transition(ConductorEvent::Claimed)
        .expect("claim is recorded")
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::Retryable("validation failed".to_string()),
        })
        .expect("outcome is recorded")
}
