//! Verify the effect, never the edit (issue #3755).
//!
//! The regression tests run in the configuration the incident required:
//! a supervising agent editing a running fleet, with verification plans
//! that re-read the edit, selectors that return empty result sets, and a
//! gate stricter than the repository's. Each of the nine incidents was
//! caught the same way — by observing the effect, never by re-reading
//! the edit — and re-reading the change would have caught none of them.

use autospec_core::effect_verification::{
    ad_hoc_gate_findings, blast_radius_findings, choose_change, outcome_line, plan_findings,
    run_probe, ship_verdict, sixty_second_answer, BlastRadius, Change, GateApplication, Probe,
    ProbeKind, ProbeOutcome, ShipVerdict, Target, VERIFY_WINDOW_SECONDS,
};

fn effect_probe(command: &str, signal: &str, seconds: u32) -> Probe {
    Probe {
        kind: ProbeKind::Effect,
        command: command.to_string(),
        expected_signal: signal.to_string(),
        seconds,
    }
}

fn reread_probe(command: &str) -> Probe {
    Probe {
        kind: ProbeKind::Reread,
        command: command.to_string(),
        expected_signal: String::new(),
        seconds: 1,
    }
}

fn running_change(summary: &str, value: u32, probes: Vec<Probe>) -> Change {
    Change {
        summary: summary.to_string(),
        target: Target::Running,
        value,
        probes,
    }
}

// --- Invariant 1: verify the effect, not the edit -------------------------

#[test]
fn only_an_effect_probe_with_a_signal_in_the_window_answers() {
    // Run the thing once and read its output: the #1 catch (grep the
    // logs for what actually killed the jobs).
    let grep_logs = effect_probe("grep killed out/issue-*/run.log", "SIGKILL", 2);
    assert!(grep_logs.answers_within_window());
    // A re-read observes what was written, not what the system does.
    assert!(!reread_probe("git diff").answers_within_window());
    // A probe that cannot name its pass signal cannot distinguish.
    assert!(!effect_probe("topup.sh", "", 2).answers_within_window());
    // A probe that takes longer than the window answers too late.
    assert!(
        !effect_probe("full selector soak", "selected", VERIFY_WINDOW_SECONDS + 1)
            .answers_within_window()
    );
    // The window itself is in time.
    assert!(
        effect_probe("draw from selector", "selected", VERIFY_WINDOW_SECONDS)
            .answers_within_window()
    );
}

#[test]
fn running_change_with_effect_probe_ships() {
    // Incident #3's catch: run the topup and read the scheduler's verdict.
    let change = running_change(
        "insert a comment into the topup script",
        5,
        vec![effect_probe("topup.sh --once", "Submitted batch job", 3)],
    );
    assert_eq!(ship_verdict(&change), ShipVerdict::Ship { probes: vec![0] });
    assert!(plan_findings(&change).is_empty());
    let answer = sixty_second_answer(&change);
    assert!(answer.starts_with("within 60s: run \"topup.sh --once\""));
    assert!(answer.contains("Submitted batch job"));
}

#[test]
fn running_change_with_no_probes_waits_for_idle() {
    // The uncomfortable version: "how will I know within sixty seconds if
    // it is not" is unanswered, so the change waits for a moment when the
    // system is idle.
    let change = running_change("replace the runner image", 9, Vec::new());
    assert_eq!(ship_verdict(&change), ShipVerdict::WaitForIdle);
    let findings = plan_findings(&change);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("NO_EFFECT_PROBE:"));
    assert_eq!(
        sixty_second_answer(&change),
        "no answer within 60s: the change waits for a moment when the system is idle"
    );
}

#[test]
fn reread_only_plan_is_the_insidious_shape() {
    // The plan *looks* like verification: it re-reads the edit.
    // Re-reading the change would have caught none of the nine incidents,
    // because each edit was correct in the sense its author intended.
    let change = running_change(
        "weight the endpoint selector by free slots",
        8,
        vec![reread_probe("git diff select-worker.sh")],
    );
    assert_eq!(ship_verdict(&change), ShipVerdict::RereadOnly);
    let findings = plan_findings(&change);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("REREAD_ONLY:"));
    assert!(findings[0].contains("would have caught none of the nine"));
    assert_eq!(
        sixty_second_answer(&change),
        "no answer: the plan re-reads the edit, which would have caught none of the nine; the change waits for an idle system"
    );
}

#[test]
fn idle_change_ships_without_a_probe() {
    // The 60-second question is about breaking a live system; idleness is
    // the state the other verdicts wait for, and a change there cannot
    // break a live consumer.
    let change = Change {
        summary: "replace the runner image".to_string(),
        target: Target::Idle,
        value: 9,
        probes: Vec::new(),
    };
    assert_eq!(
        ship_verdict(&change),
        ShipVerdict::Ship { probes: Vec::new() }
    );
    assert!(plan_findings(&change).is_empty());
    assert_eq!(
        sixty_second_answer(&change),
        "the system is idle: a change there cannot break a live consumer"
    );
}

#[test]
fn probe_outside_the_window_is_a_finding_and_does_not_ship() {
    let change = running_change(
        "full selector soak before redeploy",
        6,
        vec![effect_probe(
            "soak the selector for five minutes",
            "selected",
            300,
        )],
    );
    let findings = plan_findings(&change);
    assert_eq!(findings.len(), 2);
    assert!(findings[0].starts_with("PROBE_OVER_WINDOW:"));
    assert!(findings[0].contains("300s"));
    // The probe is an effect probe, but it answers too late: the plan
    // does not answer either, and the change waits.
    assert!(findings[1].starts_with("NO_EFFECT_PROBE:"));
    assert_eq!(ship_verdict(&change), ShipVerdict::WaitForIdle);
}

#[test]
fn probe_without_a_named_signal_is_a_finding() {
    let change = running_change(
        "run topup and look at the output",
        4,
        vec![effect_probe("topup.sh --once", "", 3)],
    );
    let findings = plan_findings(&change);
    assert_eq!(findings.len(), 2);
    assert!(findings[0].starts_with("PROBE_NO_SIGNAL:"));
    // The probe is defective, so the plan does not answer either.
    assert!(findings[1].starts_with("NO_EFFECT_PROBE:"));
    assert_eq!(ship_verdict(&change), ShipVerdict::WaitForIdle);
}

// --- Invariant 2: a verification that returns nothing is a failure --------

#[test]
fn incident_five_empty_draws_are_failures_not_passes() {
    // The symptom of the free-slot-weighted selector: draw from it five
    // times and get nothing. An empty result set reads as "no problems
    // found" — it is a failure.
    let probe = effect_probe("draw from the selector", "worker selected", 1);
    for _ in 0..5 {
        let outcome = run_probe(&probe, "");
        assert_eq!(outcome, ProbeOutcome::NoSignal);
        let line = outcome_line(&probe, &outcome);
        assert!(line.starts_with("fail: draw from the selector returned nothing"));
        assert!(line.contains("a verification that returns nothing is a failure, not a pass"));
    }
}

#[test]
fn whitespace_only_output_is_no_signal() {
    let probe = effect_probe("draw from the selector", "worker selected", 1);
    assert_eq!(run_probe(&probe, "   \n\n"), ProbeOutcome::NoSignal);
}

#[test]
fn incident_three_topup_run_catches_the_sbatch_rejection() {
    // The comment inserted inside sbatch line-continuations: running the
    // topup shows sbatch reject the wrap. The probe ran, produced
    // output, and the expected signal is not in it — a failure, which is
    // how the incident was actually caught.
    let probe = effect_probe("topup.sh --once", "Submitted batch job", 3);
    let output = "sbatch: error: invalid character in line continuation";
    assert_eq!(run_probe(&probe, output), ProbeOutcome::Failed);
    assert!(
        outcome_line(&probe, &ProbeOutcome::Failed).contains("does not carry the expected signal")
    );
}

#[test]
fn probe_with_named_signal_passes_on_the_signal() {
    let probe = effect_probe("topup.sh --once", "Submitted batch job", 3);
    assert_eq!(
        run_probe(&probe, "Submitted batch job 1234567 (topup)"),
        ProbeOutcome::Pass
    );
    assert!(outcome_line(&probe, &ProbeOutcome::Pass).starts_with("pass: "));
}

#[test]
fn a_probe_that_cannot_name_its_signal_cannot_read_a_pass() {
    // Fail-closed: a probe with no named signal reads neither emptiness
    // nor content as a pass.
    let probe = effect_probe("topup.sh --once", "", 3);
    assert_eq!(run_probe(&probe, "anything at all"), ProbeOutcome::Failed);
    assert_eq!(run_probe(&probe, ""), ProbeOutcome::NoSignal);
}

// --- Invariant 3: state the blast radius before editing shared state ------

#[test]
fn incident_three_blast_radius_was_never_stated() {
    // The comment went into a file the dispatcher reads on every topup
    // cycle; none of the three questions had been asked.
    let radius = BlastRadius::default();
    let findings = blast_radius_findings(&radius);
    assert_eq!(findings.len(), 3);
    assert!(findings[0].starts_with("BLAST_NO_READERS:"));
    assert!(findings[1].starts_with("BLAST_NO_MID_READ:"));
    assert!(findings[2].starts_with("BLAST_NO_EXCLUSIVITY:"));
}

#[test]
fn stated_blast_radius_is_clean() {
    let radius = BlastRadius {
        readers: vec![
            "the dispatcher (topup.sh is re-read on every topup cycle)".to_string(),
        ],
        on_mid_read_change: "the dispatcher submits a corrupted job and goes down (incident #3 took it down ~10 min)".to_string(),
        exclusivity: "topup.sh has no other writer; edits are taken through the flock the topup holds".to_string(),
    };
    assert!(blast_radius_findings(&radius).is_empty());
}

#[test]
fn partially_stated_blast_radius_names_each_missing_question() {
    let radius = BlastRadius {
        readers: vec!["the dispatcher".to_string()],
        on_mid_read_change: String::new(),
        exclusivity: String::new(),
    };
    let findings = blast_radius_findings(&radius);
    assert_eq!(findings.len(), 2);
    assert!(findings[0].starts_with("BLAST_NO_MID_READ:"));
    assert!(findings[1].starts_with("BLAST_NO_EXCLUSIVITY:"));
}

// --- Invariant 4: never raise a bar ad hoc --------------------------------

#[test]
fn incident_six_clippy_d_warnings_is_a_different_gate() {
    // The supervisor hand-ran `clippy -D warnings`, stricter than CI: the
    // lints it fired do not exist as failures on main, and the gate
    // nearly discarded a good patch.
    let application = GateApplication {
        gate: "clippy".to_string(),
        repo_flags: Vec::new(),
        applied_flags: vec!["-D".to_string(), "warnings".to_string()],
    };
    let findings = ad_hoc_gate_findings(&application);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("AD_HOC_GATE:"));
    assert!(findings[0].contains("adds -D warnings"));
    assert!(findings[0].contains("stricter than the repository's gate"));
    assert!(findings[0]
        .contains("a gate matches the repository's definition or it is a different gate"));
}

#[test]
fn the_repositorys_gate_is_clean() {
    let application = GateApplication {
        gate: "clippy".to_string(),
        repo_flags: vec!["--workspace".to_string()],
        applied_flags: vec!["--workspace".to_string()],
    };
    assert!(ad_hoc_gate_findings(&application).is_empty());
}

#[test]
fn a_looser_gate_is_also_a_different_gate() {
    // The opposite direction: dropping the repository's strictness can
    // accept what the repository rejects.
    let application = GateApplication {
        gate: "clippy".to_string(),
        repo_flags: vec!["-D".to_string(), "warnings".to_string()],
        applied_flags: Vec::new(),
    };
    let findings = ad_hoc_gate_findings(&application);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].contains("drops -D warnings"));
    assert!(findings[0].contains("looser than the repository's gate"));
}

// --- Invariant 5: prefer the change you can verify cheaply ----------------

#[test]
fn the_better_unverifiable_change_loses() {
    // Incident #5: a genuine improvement to load distribution, higher
    // value than the ranking it replaced — and not cheaply verifiable,
    // so it must lose to the verifiable change, no matter how much worse
    // and more partial that one is.
    let weighted = running_change("weight the endpoint selector by free slots", 10, Vec::new());
    let total = running_change(
        "keep the total ranking",
        3,
        vec![effect_probe("draw from the selector", "worker selected", 1)],
    );
    assert_eq!(choose_change(&[weighted, total]), Some(1));
}

#[test]
fn among_shippable_changes_the_highest_value_wins() {
    let a = running_change("a", 3, vec![effect_probe("run a", "ok", 1)]);
    let b = running_change("b", 9, vec![effect_probe("run b", "ok", 1)]);
    let c = running_change("c", 5, vec![effect_probe("run c", "ok", 1)]);
    assert_eq!(choose_change(&[a, b, c]), Some(1));
}

#[test]
fn ties_keep_the_first_index() {
    let a = running_change("a", 5, vec![effect_probe("run a", "ok", 1)]);
    let b = running_change("b", 5, vec![effect_probe("run b", "ok", 1)]);
    assert_eq!(choose_change(&[a, b]), Some(0));
}

#[test]
fn when_nothing_may_ship_the_answer_is_none() {
    let a = running_change("a", 10, Vec::new());
    let b = running_change("b", 9, vec![reread_probe("git diff")]);
    assert_eq!(choose_change(&[a, b]), None);
    assert_eq!(choose_change(&[]), None);
}

// --- The session, end to end ----------------------------------------------

#[test]
fn the_supervisors_session_reconstructed() {
    // The shape of the incident: the supervisor edits the running fleet,
    // and the only verification that caught the damage observed the
    // effect.
    //
    // #3: the comment inside the sbatch line-continuations. The plan
    // that re-reads the edit is the shape that looks verified — and it
    // waits, it does not ship.
    let edit = running_change(
        "annotate the topup script",
        5,
        vec![reread_probe("sed -n p topup.sh")],
    );
    assert_eq!(ship_verdict(&edit), ShipVerdict::RereadOnly);
    let effect = running_change(
        "annotate the topup script",
        5,
        vec![effect_probe("topup.sh --once", "Submitted batch job", 3)],
    );
    assert!(matches!(
        ship_verdict(&effect),
        ShipVerdict::Ship { ref probes } if probes == &vec![0]
    ));
    // And running it shows sbatch reject the wrap: the probe fails, and
    // the failure is what is reported.
    let rejected = run_probe(
        &effect.probes[0],
        "sbatch: error: invalid character in line continuation",
    );
    assert_eq!(rejected, ProbeOutcome::Failed);
    assert!(outcome_line(&effect.probes[0], &rejected).starts_with("fail: "));
    //
    // #5: the selector returns nothing. Every draw is a failure, none of
    // them a pass, and the unverified change is not the one chosen.
    let probe = effect.probes[0].clone();
    for _ in 0..5 {
        assert_eq!(run_probe(&probe, ""), ProbeOutcome::NoSignal);
    }
    assert_eq!(
        choose_change(&[edit, effect.clone(), weighted_selector()]),
        Some(1)
    );
    //
    // #6: the ad hoc gate is refused, naming the flag it adds.
    let gate = GateApplication {
        gate: "clippy".to_string(),
        repo_flags: Vec::new(),
        applied_flags: vec!["-D".to_string(), "warnings".to_string()],
    };
    let findings = ad_hoc_gate_findings(&gate);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].contains("adds -D warnings"));
}

fn weighted_selector() -> Change {
    running_change("weight the endpoint selector by free slots", 10, Vec::new())
}
