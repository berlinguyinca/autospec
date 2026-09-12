//! An empty file you created is not evidence about a process you did not
//! instrument, and `pgrep -f` matches its own caller (issue #4145).
//!
//! The regression tests run in the configuration the incident required: a
//! conversion pass launched with its stdout redirected to a capture file,
//! while the pass itself logged to its own file via `LOG=`, and its liveness
//! checked with `pgrep -f` on a pattern drawn from its own arguments. The
//! capture stayed at 0 bytes and the `pgrep` matched its own caller — the
//! pass had in fact run for 88 minutes and converted 8 patches.

use autospec_core::process_evidence::{
    capture_routed, diagnose, diagnosis_finding, liveness_verdict, self_referential_finding,
    unrouted_capture_finding, CapturedStream, Diagnosis, EvidenceSource, Liveness, LivenessCheck,
    Observation, OutputDestination, ProcessMatch,
};

/// The pass's own log, where every line actually went. (The wrapper's stdout
/// redirect pointed at `/tmp/convpass/convpassW.log`, which stayed at 0 B.)
const OWN_LOG: &str = "/tmp/convpass/convpass.log";
/// The `pgrep -f` pattern, drawn from the pass's own arguments.
const PATTERN: &str = "convpass.sh 3194";
/// The two PIDs the incident's `pgrep` returned — the shell running it.
const SELF_PIDS: [u32; 2] = [3142082, 3142084];
/// The pass's terminal line in its own log: it finished normally.
const TERMINAL_LINE: &str = "converted=8 held=4 skipped=1";

fn own_log() -> OutputDestination {
    OutputDestination::OwnLog {
        path: OWN_LOG.to_string(),
    }
}

fn stdout_capture() -> EvidenceSource {
    EvidenceSource::CapturedStream(CapturedStream::Stdout)
}

fn read_own_log() -> EvidenceSource {
    EvidenceSource::OwnLog {
        path: OWN_LOG.to_string(),
    }
}

fn self_matches() -> Vec<ProcessMatch> {
    SELF_PIDS
        .iter()
        .map(|&pid| ProcessMatch {
            pid,
            cmdline: format!("bash -c pgrep -f '{PATTERN}'"),
            is_program: false,
        })
        .collect()
}

// --- Invariant 1: a redirect is not evidence about an unrouted stream ------

#[test]
fn incident_stdout_capture_is_unrouted() {
    // The pass writes to its own log; the wrapper captured stdout. The
    // capture is empty on every run, success and failure alike.
    assert!(!capture_routed(&own_log(), &stdout_capture()));
    let findings = unrouted_capture_finding(&own_log(), &stdout_capture());
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("UNROUTED_CAPTURE:"));
    assert!(findings[0].contains(OWN_LOG));
    assert!(findings[0].contains("stdout"));
    // The remedy names the program's own log variable.
    assert!(findings[0].contains("LOG="));
}

#[test]
fn reading_the_programs_own_log_is_routed() {
    // The fix: read the program's own log (set LOG= to it), not a stream it
    // never writes to. The same file that was useless as a stdout capture is
    // the present when read as the program's log.
    assert!(capture_routed(&own_log(), &read_own_log()));
    assert!(unrouted_capture_finding(&own_log(), &read_own_log()).is_empty());
}

#[test]
fn own_log_is_unrouted_on_both_streams() {
    // A program with its own log writes to neither stream a wrapper
    // redirected, so both captures are unrouted.
    assert!(!capture_routed(&own_log(), &stdout_capture()));
    let stderr = EvidenceSource::CapturedStream(CapturedStream::Stderr);
    assert!(!capture_routed(&own_log(), &stderr));
    assert!(!unrouted_capture_finding(&own_log(), &stderr).is_empty());
}

#[test]
fn a_stdout_programs_stdout_capture_is_routed() {
    // Control: a program that writes to stdout, captured on stdout, is
    // routed — an empty capture there is meaningful evidence, not a trap.
    assert!(capture_routed(
        &OutputDestination::Stdout,
        &stdout_capture()
    ));
    assert!(unrouted_capture_finding(&OutputDestination::Stdout, &stdout_capture()).is_empty());
}

#[test]
fn a_stderr_programs_stdout_capture_is_unrouted() {
    assert!(!capture_routed(
        &OutputDestination::Stderr,
        &stdout_capture()
    ));
    assert!(!unrouted_capture_finding(&OutputDestination::Stderr, &stdout_capture()).is_empty());
}

#[test]
fn reading_a_different_own_log_is_unrouted() {
    // A reader pointed at a log file the program does not write is still
    // unrouted — the path, not the "it is a log file" shape, is what matters.
    let wrong = EvidenceSource::OwnLog {
        path: "/tmp/other/convpass.log".to_string(),
    };
    assert!(!capture_routed(&own_log(), &wrong));
    assert!(!unrouted_capture_finding(&own_log(), &wrong).is_empty());
}

// --- Invariant 2: a liveness check must not be able to match itself --------

#[test]
fn incident_pgrep_matches_only_its_own_caller() {
    // The incident: `pgrep -f 'convpass.sh 3194'` returned the two PIDs of
    // the shell running that very pgrep. The verdict is SelfReferential, not
    // Alive — the check is incapable of reporting dead.
    let check = LivenessCheck::Pattern {
        pattern: PATTERN.to_string(),
    };
    assert_eq!(
        liveness_verdict(&check, &self_matches()),
        Liveness::SelfReferential
    );
    let findings = self_referential_finding(&check, &self_matches());
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("SELF_REFERENTIAL_LIVENESS:"));
    assert!(findings[0].contains(PATTERN));
    // The remedy names both the PID check and the caller exclusion.
    assert!(findings[0].contains("kill -0"));
}

#[test]
fn a_pid_check_is_never_self_referential() {
    // The designed-in half: a PID recorded at launch is a specific identity
    // the checker can never be, so it reports dead cleanly.
    let pid = 3143001;
    let check = LivenessCheck::Pid { pid };
    let alive = vec![ProcessMatch {
        pid,
        cmdline: format!("bash {PATTERN} 3210"),
        is_program: true,
    }];
    assert_eq!(liveness_verdict(&check, &alive), Liveness::Alive { pid });
    assert!(self_referential_finding(&check, &alive).is_empty());
    // Program gone: Dead, and never SelfReferential.
    assert_eq!(liveness_verdict(&check, &[]), Liveness::Dead);
    assert!(self_referential_finding(&check, &[]).is_empty());
}

#[test]
fn a_pattern_check_excluding_the_caller_reports_dead() {
    // The remedy working: the caller excluded its own PIDs and the program is
    // genuinely gone, so the pattern check has no matches and reports dead —
    // not SelfReferential, not Alive.
    let check = LivenessCheck::Pattern {
        pattern: PATTERN.to_string(),
    };
    assert_eq!(liveness_verdict(&check, &[]), Liveness::Dead);
    assert!(self_referential_finding(&check, &[]).is_empty());
}

#[test]
fn a_pattern_check_finds_the_program_among_its_callers() {
    // The program is running alongside the caller's shells: a non-self match
    // decides it alive, and the caller matches are ignored.
    let check = LivenessCheck::Pattern {
        pattern: PATTERN.to_string(),
    };
    let mut matches = self_matches();
    matches.push(ProcessMatch {
        pid: 3143001,
        cmdline: format!("bash {PATTERN} 3210"),
        is_program: true,
    });
    assert_eq!(
        liveness_verdict(&check, &matches),
        Liveness::Alive { pid: 3143001 }
    );
    assert!(self_referential_finding(&check, &matches).is_empty());
}

// --- Invariant 3: two claims need two pieces of evidence -------------------

fn incident_undecidable() -> Observation {
    // The moment the pass was declared "alive" on self-referential evidence:
    // the liveness check matched only its caller, and the reader looked at
    // the empty stdout capture.
    Observation {
        dest: own_log(),
        source: stdout_capture(),
        terminal_line_present: false,
        liveness: Liveness::SelfReferential,
    }
}

#[test]
fn incident_self_referential_liveness_is_undecidable() {
    let obs = incident_undecidable();
    assert_eq!(
        diagnose(&obs),
        Diagnosis::Undecidable {
            reasons: vec![
                "the liveness check matched only its own caller, so it is incapable of \
reporting 'dead' — neither 'running' nor 'not running' is supported by it"
                    .to_string(),
            ],
        }
    );
    let findings = diagnosis_finding(&obs);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("UNDECIDABLE_DIAGNOSIS:"));
}

#[test]
fn incident_finished_pass_read_correctly_is_finished() {
    // After the pass finished, read the way the fix prescribes: a PID check
    // says it is gone, and the reader reads the program's own log, which
    // carries the terminal line. Both claims are supported — Finished.
    let obs = Observation {
        dest: own_log(),
        source: read_own_log(),
        terminal_line_present: true,
        liveness: Liveness::Dead,
    };
    assert_eq!(diagnose(&obs), Diagnosis::Finished);
    assert!(diagnosis_finding(&obs).is_empty());
}

#[test]
fn incident_finished_pass_read_from_unrouted_capture_is_not_finished() {
    // The incident's second error: the pass had finished (a correct check
    // says it is not running), but the reader looked at the empty stdout
    // capture and concluded "produced nothing." "Not running" is supported;
    // "produced no output" is not — the pass converted 8 patches into its
    // own log.
    let obs = Observation {
        dest: own_log(),
        source: stdout_capture(),
        terminal_line_present: true, // in the program's actual log
        liveness: Liveness::Dead,
    };
    assert_eq!(diagnose(&obs), Diagnosis::NotRunningUnknownOutput);
    let findings = diagnosis_finding(&obs);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("CONFLATED_CLAIMS:"));
    assert!(findings[0].contains("produced no output"));
}

#[test]
fn a_wedged_pass_is_running_not_dead() {
    // A wedged run produces no new output and is running: a correct liveness
    // check says running, so "not running" is unsupported — the reader must
    // not declare it dead.
    let obs = Observation {
        dest: own_log(),
        source: read_own_log(),
        terminal_line_present: false,
        liveness: Liveness::Alive { pid: 3143001 },
    };
    assert_eq!(diagnose(&obs), Diagnosis::Running);
    let findings = diagnosis_finding(&obs);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("CONFLATED_CLAIMS:"));
    assert!(findings[0].contains("running"));
}

#[test]
fn a_finished_pass_without_a_terminal_line_did_not_finish() {
    // Not running, and the program's own log carries no terminal line: it
    // died mid-run, so "produced no output" is not supported even though the
    // source is the right one.
    let obs = Observation {
        dest: own_log(),
        source: read_own_log(),
        terminal_line_present: false,
        liveness: Liveness::Dead,
    };
    assert_eq!(diagnose(&obs), Diagnosis::NotRunningUnknownOutput);
    assert!(!diagnosis_finding(&obs).is_empty());
}

// --- The incident, end to end ----------------------------------------------

#[test]
fn incident_end_to_end() {
    // Both errors, in the order the incident produced them.
    let check = LivenessCheck::Pattern {
        pattern: PATTERN.to_string(),
    };

    // First "alive," on evidence that is self-referential: the pgrep matched
    // its own caller, and the reader read the empty stdout capture.
    let undecidable = incident_undecidable();
    assert_eq!(
        liveness_verdict(&check, &self_matches()),
        undecidable.liveness
    );
    assert!(!diagnosis_finding(&undecidable).is_empty());
    assert!(!unrouted_capture_finding(&undecidable.dest, &undecidable.source).is_empty());

    // Then "dead" after the pass legitimately finished: the terminal line the
    // reader missed carried converted=8.
    let terminal = Observation {
        dest: own_log(),
        source: read_own_log(),
        terminal_line_present: true,
        liveness: Liveness::Dead,
    };
    assert_eq!(diagnose(&terminal), Diagnosis::Finished);
    assert!(diagnosis_finding(&terminal).is_empty());
    assert!(terminal.terminal_line_present);
    assert!(TERMINAL_LINE.contains("converted=8"));
}
