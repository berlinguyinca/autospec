use autospec_core::agent::{build_review_argv, HarnessKind, ReviewDispatchOutcome};

#[test]
fn every_harness_receives_one_slash_command_argument() {
    for kind in [HarnessKind::Claude, HarnessKind::Codex, HarnessKind::OpenCode] {
        let argv = build_review_argv(kind, "2026-07-22T00:00:00Z", "/tmp/gaps.json");
        let command = argv.last().expect("review command argument");

        assert!(command.contains("/autospec-review --remediation --since 2026-07-22T00:00:00Z --emit-gaps /tmp/gaps.json"));
        assert_eq!(argv.iter().filter(|arg| arg.as_str() == "--remediation").count(), 0);
    }
}

#[test]
fn dispatch_outcomes_keep_failed_and_zero_gap_states_distinct() {
    assert_ne!(
        ReviewDispatchOutcome::DispatchFailed,
        ReviewDispatchOutcome::ZeroGapsEmitted
    );
}
