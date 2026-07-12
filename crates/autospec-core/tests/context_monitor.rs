use autospec_core::context::{ContextAction, ContextMonitorEngine, ContextState};

fn action_kinds(actions: &[ContextAction]) -> Vec<&str> {
    actions.iter().map(ContextAction::kind).collect()
}

#[test]
fn context_monitor_compact_before_rollover_even_at_high_usage() {
    let mut engine = ContextMonitorEngine::new();

    let actions = engine.classify_percent(80);

    assert_eq!(engine.state(), ContextState::Compacted);
    assert_eq!(action_kinds(&actions), vec!["compact"]);
}

#[test]
fn context_monitor_compacted_rolls_over_with_handoff_clear_resume_order() {
    let mut engine = ContextMonitorEngine::new();
    assert_eq!(action_kinds(&engine.classify_percent(50)), vec!["compact"]);

    let actions = engine.classify_percent(80);

    assert_eq!(engine.state(), ContextState::Rolled);
    assert_eq!(action_kinds(&actions), vec!["handoff", "clear", "resume"]);
}

#[test]
fn context_monitor_low_usage_resets_rolled_to_normal() {
    let mut engine = ContextMonitorEngine::new();
    engine.classify_percent(50);
    engine.classify_percent(80);
    assert_eq!(engine.state(), ContextState::Rolled);

    let actions = engine.classify_percent(29);

    assert_eq!(engine.state(), ContextState::Normal);
    assert_eq!(action_kinds(&actions), vec!["noop"]);
    assert_eq!(actions[0].payload(), "reset:rolled->normal");
}

#[test]
fn context_monitor_scripted_sequence_matches_python_engine_parity() {
    let mut engine = ContextMonitorEngine::new();
    let sequence = [10, 30, 51, 25, 49, 52, 75, 81];
    let expected_states = [
        ContextState::Normal,
        ContextState::Normal,
        ContextState::Compacted,
        ContextState::Normal,
        ContextState::Normal,
        ContextState::Compacted,
        ContextState::Compacted,
        ContextState::Rolled,
    ];
    let expected_actions = [
        vec![],
        vec![],
        vec!["compact"],
        vec!["noop"],
        vec![],
        vec!["compact"],
        vec![],
        vec!["handoff", "clear", "resume"],
    ];

    for ((pct, expected_state), expected_action) in sequence
        .into_iter()
        .zip(expected_states)
        .zip(expected_actions)
    {
        let actions = engine.classify_percent(pct);
        assert_eq!(engine.state(), expected_state, "state mismatch at {pct}");
        assert_eq!(
            action_kinds(&actions),
            expected_action,
            "actions mismatch at {pct}"
        );
    }
}
