//! Regression tests for #4434. Each case is a real incident from the session
//! that produced the type, not an invented example.

use autospec_core::gate_verdict::{
    absolute, differential, parse_diff_stat, verify_change_size, DiffStat, GateVerdict, TestRun,
    UnderstatedChange,
};
use std::collections::BTreeSet;

/// A run whose gate stated it completed (completion marker present).
fn run(exit: i32, lines: usize, failures: &[&str]) -> TestRun {
    TestRun {
        exit_code: exit,
        result_lines: lines,
        failures: failures
            .iter()
            .map(|s| s.to_string())
            .collect::<BTreeSet<_>>(),
        completed: true,
    }
}

/// A run whose gate never emitted its completion marker: killed mid-run
/// (#4457). Every count in it is unmeasured.
fn incomplete(exit: i32, lines: usize, failures: &[&str]) -> TestRun {
    TestRun {
        exit_code: exit,
        result_lines: lines,
        failures: failures
            .iter()
            .map(|s| s.to_string())
            .collect::<BTreeSet<_>>(),
        completed: false,
    }
}

/// INCIDENT: patch #4148 failed to compile (19 x E0596). It produced an empty
/// failure set, and a set-difference against a 2-failure baseline read it as
/// "no new failures" and PASSED it.
#[test]
fn a_build_failure_is_not_an_improvement() {
    let baseline = run(101, 1, &["known_a", "known_b"]);
    let did_not_build = run(101, 0, &[]);
    let v = differential(&baseline, &did_not_build);
    assert!(!v.is_pass(), "a patch that never ran must not pass: {v}");
    assert!(
        v.is_unmeasured(),
        "and it must be reported as unmeasured, not as a failure: {v}"
    );
    assert!(
        v.to_string().contains("never ran"),
        "the message must say why, or the next reader repeats the mistake: {v}"
    );
}

/// INCIDENT: the gate loop `continue`d past one crate's suite but left its ok
/// flag set, reporting PASS without testing.
#[test]
fn a_skipped_suite_is_not_a_pass() {
    let skipped = run(0, 0, &[]);
    let v = absolute(&skipped);
    assert!(!v.is_pass(), "exit 0 with no tests run is not a pass: {v}");
    assert!(v.is_unmeasured());
}

/// A candidate that runs FEWER tests than the baseline has not been judged.
/// "No new failures" while measuring less is measuring less, not improving.
#[test]
fn measuring_less_is_not_improving() {
    let baseline = run(101, 175, &["known"]);
    let partial = run(101, 40, &["known"]);
    let v = differential(&baseline, &partial);
    assert!(v.is_unmeasured(), "a partial run must not pass: {v}");
    assert!(v.to_string().contains("fewer tests"), "{v}");
}

#[test]
fn an_unmeasured_baseline_refuses_to_judge() {
    // Comparing against a baseline that never ran is comparing against
    // nothing; the honest answer is "I do not know".
    let v = differential(&run(101, 0, &[]), &run(0, 100, &[]));
    assert!(v.is_unmeasured(), "{v}");
    assert!(v.to_string().contains("BASELINE"), "{v}");
}

/// The real autospec-cli case: 2 pre-existing failures on main, a patch that
/// introduces none must pass.
#[test]
fn inherited_failures_do_not_block_a_clean_patch() {
    let baseline = run(
        101,
        1,
        &["stale_startup_recovery", "integrated_inactive_local_branch"],
    );
    let candidate = run(
        101,
        1,
        &["stale_startup_recovery", "integrated_inactive_local_branch"],
    );
    assert_eq!(differential(&baseline, &candidate), GateVerdict::Pass);
}

#[test]
fn a_genuinely_new_failure_is_named() {
    let baseline = run(101, 1, &["known"]);
    let candidate = run(101, 1, &["known", "brand_new"]);
    match differential(&baseline, &candidate) {
        GateVerdict::Fail { reasons } => {
            assert_eq!(reasons, vec!["brand_new".to_string()]);
        }
        other => panic!("expected Fail naming the new test, got {other}"),
    }
}

#[test]
fn absolute_passes_a_green_suite_and_fails_a_red_one() {
    assert_eq!(absolute(&run(0, 175, &[])), GateVerdict::Pass);
    assert!(!absolute(&run(101, 175, &["boom"])).is_pass());
}

/// Non-zero exit with no named failures is still a failure, not a pass: the
/// harness itself may have died after reporting results.
#[test]
fn non_zero_exit_without_named_failures_still_fails() {
    let v = absolute(&run(101, 10, &[]));
    assert!(!v.is_pass(), "{v}");
    assert!(
        !v.is_unmeasured(),
        "results were observed, so this is a real failure: {v}"
    );
}

/// There is deliberately no `bool` conversion: a caller cannot coerce
/// NotMeasured into success. This test documents that as intent.
#[test]
fn not_measured_never_reads_as_pass() {
    for v in [
        GateVerdict::NotMeasured {
            why: "anything".into(),
        },
        GateVerdict::Fail {
            reasons: vec!["x".into()],
        },
    ] {
        assert!(!v.is_pass(), "{v}");
    }
    assert!(GateVerdict::Pass.is_pass());
}

// ---------------------------------------------------------------------------
// #4457: a killed gate and a green gate are the same log -- measurements need
// completion markers.
// ---------------------------------------------------------------------------

/// INCIDENT (#4457): a gate ran as a background job under a tool call that
/// timed out at ten minutes. The timeout killed the process group (exit 143),
/// so the log stopped at suite 40 of 48 with no failures yet. Without a
/// completion marker the log reads exactly like a pass.
#[test]
fn a_killed_run_that_looks_green_is_unmeasured() {
    let killed = incomplete(143, 40, &[]);
    let v = absolute(&killed);
    assert!(!v.is_pass(), "a killed run must not pass: {v}");
    assert!(v.is_unmeasured(), "it must be unmeasured: {v}");
    assert!(
        v.to_string().contains("completion marker"),
        "the reason must name the missing marker: {v}"
    );
}

/// The same incident through the differential gate: a killed candidate must
/// not read as "no new failures" against a green baseline.
#[test]
fn a_killed_candidate_is_unmeasured_not_an_improvement() {
    let baseline = run(0, 48, &[]);
    let killed = incomplete(143, 40, &[]);
    let v = differential(&baseline, &killed);
    assert!(!v.is_pass(), "a killed candidate must not pass: {v}");
    assert!(v.is_unmeasured(), "{v}");
    assert!(v.to_string().contains("completion marker"), "{v}");
}

/// A killed BASELINE is equally untrustworthy: its counts may be partial, so
/// there is nothing sound to compare against.
#[test]
fn a_killed_baseline_refuses_to_judge() {
    let baseline = incomplete(143, 40, &[]);
    let candidate = run(0, 48, &[]);
    let v = differential(&baseline, &candidate);
    assert!(!v.is_pass(), "{v}");
    assert!(v.is_unmeasured(), "{v}");
    assert!(v.to_string().contains("BASELINE"), "{v}");
}

/// INCIDENT (#4457): the kill landed before a single suite finished, so the
/// log showed `suites: 0   FAILED: 0`. Zero failures from zero suites is a
/// contradiction, not a pass -- even when the gate did state it completed.
#[test]
fn zero_failures_from_zero_suites_is_a_contradiction() {
    let empty = run(0, 0, &[]); // completed, but zero suites
    let v = absolute(&empty);
    assert!(!v.is_pass(), "zero suites is not a pass: {v}");
    assert!(v.is_unmeasured(), "{v}");
    assert!(v.to_string().contains("contradiction"), "{v}");
}

/// The gate summary reports suites alongside failures, so the zero-suite
/// contradiction is visible in one line.
#[test]
fn the_summary_reports_suites_alongside_failures() {
    assert_eq!(run(0, 0, &[]).summary(), "suites: 0   failed: 0");
    assert_eq!(run(0, 48, &[]).summary(), "suites: 48   failed: 0");
    assert_eq!(
        run(101, 48, &["a", "b"]).summary(),
        "suites: 48   failed: 2"
    );
}

/// A completed green run still passes: the marker does not make a clean run
/// unmeasured.
#[test]
fn a_completed_green_run_still_passes() {
    let green = run(0, 48, &[]);
    assert_eq!(absolute(&green), GateVerdict::Pass);
    assert_eq!(green.summary(), "suites: 48   failed: 0");
}

/// INCIDENT (#4457): a patch applied as 1973 insertions showed only 18 from
/// the unstaged `git diff --stat` -- eight of ten files were new. The staged
/// view counts them; the parser reads the same summary shape from either.
#[test]
fn parse_diff_stat_reads_the_staged_summary() {
    // The per-file bar lines come first; the summary is the last "changed" line.
    let output = "\
 src/a.rs | 100 +++++++++++
 src/b.rs | 50 +++++
 new/c.rs | 1823 ++++++++++++++++
 10 files changed, 1973 insertions(+)
";
    let stat = parse_diff_stat(output).expect("summary line present");
    assert_eq!(
        stat,
        DiffStat {
            files: 10,
            insertions: 1973,
            deletions: 0
        }
    );
    assert_eq!(
        stat.pr_body_line(),
        "Reviewed 10 file(s): 1973 insertion(s), 0 deletion(s) (staged view)."
    );
}

/// A diff that also deletes carries the deletions count.
#[test]
fn parse_diff_stat_reads_deletions() {
    let output = "1 file changed, 5 insertions(+), 3 deletions(-)\n";
    assert_eq!(
        parse_diff_stat(output),
        Some(DiffStat {
            files: 1,
            insertions: 5,
            deletions: 3
        })
    );
}

/// No summary line is no measurement, not an empty stat.
#[test]
fn parse_diff_stat_with_no_summary_is_none() {
    assert_eq!(parse_diff_stat(""), None);
    assert_eq!(parse_diff_stat("src/a.rs | 10 +++++\n"), None);
}

/// The unstaged view understates a patch with new files: the check that
/// catches the incident. The applied size (1973) and the staged measurement
/// agree; the unstaged one (18) does not.
#[test]
fn verify_change_size_catches_the_unstaged_undercount() {
    let staged = DiffStat {
        files: 10,
        insertions: 1973,
        deletions: 0,
    };
    assert!(
        verify_change_size(1973, &staged).is_ok(),
        "staged view agrees"
    );

    let unstaged = DiffStat {
        files: 2,
        insertions: 18,
        deletions: 0,
    };
    let err = verify_change_size(1973, &unstaged).unwrap_err();
    assert_eq!(
        err,
        UnderstatedChange {
            declared_insertions: 1973,
            measured_insertions: 18,
        }
    );
    assert!(err.to_string().contains("staged view"), "{err}");
}
