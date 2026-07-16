use autospec_core::autonomous::drain::{decide, DrainDecision, DrainObservation, DrainProgress};

#[test]
fn completed_child_precedes_a_stall_timeout() {
    let observation = DrainObservation::completed(124, 30, 30);

    assert_eq!(
        decide(&observation),
        DrainDecision::Complete { exit_code: 124 }
    );
}

#[test]
fn output_or_artifact_progress_keeps_a_live_child_running() {
    for progress in [DrainProgress::ChildOutput, DrainProgress::Artifact] {
        let observation = DrainObservation::live(30, 30, progress);

        assert_eq!(decide(&observation), DrainDecision::Wait, "{progress:?}");
    }
}

#[test]
fn external_progress_warns_instead_of_terminating_a_quiet_child() {
    for progress in [DrainProgress::Heartbeat, DrainProgress::Github] {
        let observation = DrainObservation::live(30, 30, progress);

        assert_eq!(
            decide(&observation),
            DrainDecision::WarnExternalProgress,
            "{progress:?}"
        );
    }
}

#[test]
fn silent_live_child_is_terminated_only_after_the_stall_window() {
    let waiting = DrainObservation::live(29, 30, DrainProgress::None);
    let stalled = DrainObservation::live(30, 30, DrainProgress::None);

    assert_eq!(decide(&waiting), DrainDecision::Wait);
    assert_eq!(decide(&stalled), DrainDecision::TerminateStalled);
}
