//! Issue #4293: the `cron-*.log` decoys.
//!
//! The incident shape: `*/10 * * * * /bin/bash $L/topup.sh >> $L/logs/cron-topup.log 2>&1`
//! while `topup.sh` sets `LOG="$L/logs/topup.log"` — two candidate
//! destinations, and the one the schedule names stays empty on every
//! healthy run. The empty log reads as a systemic failure; what exonerated
//! the fleet was the lockfile mtimes, which is coincidence, not a liveness
//! signal.
//!
//! #4246 is the control case: there the log was real and the writer had
//! stopped. Here the writer is fine and the log is a decoy. The two must
//! be distinguishable mechanically, from durable state, without reading
//! either log.

use autospec_core::cron_log_contract::{
    audit_destination, parse_command, scan_heartbeat_coverage, script_log, stderr_name,
    stray_redirect_name, CronCommand, CronLine, DestinationOwner, DestinationOwnership, JobStatus,
    Stream,
};

// ── The incident, end to end ──────────────────────────────────────────────

const CRON_TOPUP: &str = "*/10 * * * * /bin/bash $L/topup.sh >> $L/logs/cron-topup.log 2>&1";

const TOPUP_SCRIPT: &str = r#"#!/bin/bash
set -euo pipefail
L=/quobyte/metabolomicsgrp/it/llm/autospec-fleet
LOG="$L/logs/topup.log"
LOCK="$L/locks/topup.lock"

main() {
  local work
  work=$(pending_work)
  if [ -n "$work" ]; then
    do_topup "$work" >> "$LOG"
  fi
}
main "$@"
"#;

#[test]
fn incident_cron_line_parses_into_one_both_stream_destination() {
    let line = CronLine::parse(CRON_TOPUP).expect("six-field cron line");
    assert_eq!(line.schedule, "*/10 * * * *");
    assert_eq!(line.command.command, "/bin/bash $L/topup.sh");
    assert_eq!(
        line.command.redirects,
        vec![autospec_core::cron_log_contract::CronRedirect {
            target: "$L/logs/cron-topup.log".to_string(),
            stream: Stream::Both,
        }]
    );
    assert_eq!(
        line.command.destinations(),
        vec!["$L/logs/cron-topup.log".to_string()]
    );
}

#[test]
fn incident_audits_as_decoy_for_all_three_fleet_jobs() {
    let jobs = [
        ("topup", CRON_TOPUP, TOPUP_SCRIPT),
        (
            "regsweep",
            "*/30 * * * * /bin/bash $L/regsweep.sh >> $L/logs/cron-regsweep.log 2>&1",
            "#!/bin/bash\nL=/quobyte/metabolomicsgrp/it/llm/autospec-fleet\nLOG=$L/logs/regsweep.log\ndo_sweep\n",
        ),
        (
            "autoscale",
            "*/15 * * * * /bin/bash $L/autoscale.sh >> $L/logs/cron-autoscale.log",
            "#!/bin/bash\nexport LOG=\"${LOG:-$L/logs/autoscale.log}\"\ncheck_scale\n",
        ),
    ];
    for (job, cron_line, script) in jobs {
        let parsed = CronLine::parse(cron_line).expect("parses");
        let ownership = audit_destination(&parsed.command, script);
        let (schedule, script_path) = match &ownership {
            DestinationOwnership::Decoy { schedule, script } => (schedule.clone(), script.clone()),
            other => panic!("{job}: expected Decoy, got {other:?}"),
        };
        // The schedule's path is the one an operator checks — and it is the
        // one that reads empty.
        assert!(schedule.ends_with(".log"), "{job}: {schedule}");
        assert_ne!(schedule, script_path, "{job}");
        // The decoy name is a stray redirect named like the job's log.
        assert_eq!(
            stray_redirect_name(job, &schedule),
            Some(stderr_name(job)),
            "{job}: decoy must be renamed to <job>.stderr"
        );
        // The finding names both paths.
        let finding = ownership.finding(job).expect("decoy has a finding");
        assert!(finding.contains(&schedule), "{job}: {finding}");
        assert!(finding.contains(&script_path), "{job}: {finding}");
    }
}

#[test]
fn the_fixed_cron_line_drops_the_redirect() {
    let line = CronLine::parse(CRON_TOPUP).unwrap();
    assert_eq!(
        line.render_without_redirects(),
        "*/10 * * * * /bin/bash $L/topup.sh"
    );
    assert_eq!(line.command.without_redirects(), "/bin/bash $L/topup.sh");
    // And the corrected schedule audits clean against the same script.
    let fixed = CronLine::parse(line.render_without_redirects().as_str()).unwrap();
    assert_eq!(
        audit_destination(&fixed.command, TOPUP_SCRIPT),
        DestinationOwnership::Single {
            by: DestinationOwner::Script,
            path: "$L/logs/topup.log".to_string(),
        }
    );
}

#[test]
fn renamed_stderr_redirect_audits_clean_and_names_clean() {
    let renamed = "*/10 * * * * /bin/bash $L/topup.sh 2>> $L/logs/topup.stderr";
    let line = CronLine::parse(renamed).unwrap();
    assert_eq!(line.command.redirects[0].stream, Stream::Stderr);
    // A stderr-only redirect is not a log destination: the job's log is
    // still just the one the script owns, and invariant 1 holds.
    assert_eq!(
        audit_destination(&line.command, TOPUP_SCRIPT),
        DestinationOwnership::Single {
            by: DestinationOwner::Script,
            path: "$L/logs/topup.log".to_string(),
        }
    );
    // And the name reads as good news when empty: no rename suggestion.
    assert_eq!(stray_redirect_name("topup", "$L/logs/topup.stderr"), None);
}

#[test]
fn a_stderr_only_redirect_named_like_a_log_still_gets_renamed() {
    // Invariant 1 cannot see this — a stderr-only target is not a log
    // destination — so invariant 2 must: the name reads as the job's log
    // and would be checked for liveness.
    let cron = parse_command("/bin/bash $L/topup.sh 2>> $L/logs/cron-topup.log");
    assert_eq!(
        audit_destination(&cron, TOPUP_SCRIPT),
        DestinationOwnership::Single {
            by: DestinationOwner::Script,
            path: "$L/logs/topup.log".to_string(),
        }
    );
    assert_eq!(
        stray_redirect_name("topup", "$L/logs/cron-topup.log"),
        Some("topup.stderr".to_string())
    );
}

// ── Invariant 1: the destination audit, case by case ─────────────────────

#[test]
fn redirect_forms_parse_to_the_right_streams() {
    let c = parse_command("/bin/bash x.sh > out.log");
    assert_eq!(c.redirects[0].stream, Stream::Stdout);

    let c = parse_command("/bin/bash x.sh 2> err.log");
    assert_eq!(c.redirects.len(), 1);
    assert_eq!(c.redirects[0].stream, Stream::Stderr);
    assert_eq!(c.redirects[0].target, "err.log");

    let c = parse_command("/bin/bash x.sh &> both.log");
    assert_eq!(c.redirects.len(), 1);
    assert_eq!(c.redirects[0].stream, Stream::Both);

    let c = parse_command("/bin/bash x.sh > out.log 2> err.log");
    assert_eq!(c.redirects.len(), 2);
    assert_eq!(c.redirects[0].stream, Stream::Stdout);
    assert_eq!(c.redirects[1].stream, Stream::Stderr);

    let c = parse_command("/bin/bash x.sh 2>&-");
    assert!(c.redirects.is_empty());
    assert_eq!(c.command, "/bin/bash x.sh");

    let c = parse_command("/bin/bash x.sh");
    assert!(c.redirects.is_empty());
    assert_eq!(c.destinations(), Vec::<String>::new());
}

#[test]
fn script_log_recognizes_the_declared_forms() {
    assert_eq!(
        script_log("#!/bin/bash\nLOG=\"$L/logs/topup.log\"\n"),
        Some("$L/logs/topup.log".to_string())
    );
    assert_eq!(
        script_log("#!/bin/bash\nLOG=$L/logs/regsweep.log\n"),
        Some("$L/logs/regsweep.log".to_string())
    );
    assert_eq!(
        script_log("#!/bin/bash\nexport LOG='$L/logs/autoscale.log'\n"),
        Some("$L/logs/autoscale.log".to_string())
    );
    assert_eq!(
        script_log("#!/bin/bash\nLOG=\"${LOG:-$L/logs/autoscale.log}\"\n"),
        Some("$L/logs/autoscale.log".to_string())
    );
    assert_eq!(script_log("#!/bin/bash\necho hi\n"), None);
    // First assignment wins: it is the declaration.
    assert_eq!(
        script_log("LOG=a.log\nLOG=b.log\n"),
        Some("a.log".to_string())
    );
}

#[test]
fn one_side_owning_the_log_is_single() {
    let no_redirect = parse_command("/bin/bash x.sh");
    assert_eq!(
        audit_destination(&no_redirect, "LOG=$L/logs/x.log\n"),
        DestinationOwnership::Single {
            by: DestinationOwner::Script,
            path: "$L/logs/x.log".to_string(),
        }
    );

    let schedule_owned = parse_command("/bin/bash x.sh >> $L/logs/cron-x.log 2>&1");
    assert_eq!(
        audit_destination(&schedule_owned, "#!/bin/bash\necho hi\n"),
        DestinationOwnership::Single {
            by: DestinationOwner::Schedule,
            path: "$L/logs/cron-x.log".to_string(),
        }
    );
}

#[test]
fn both_sides_naming_the_same_path_is_shared_not_decoy() {
    let cron = parse_command("/bin/bash x.sh >> $L/logs/x.log 2>&1");
    let ownership = audit_destination(&cron, "LOG=\"$L/logs/x.log\"\n");
    assert_eq!(
        ownership,
        DestinationOwnership::Shared {
            path: "$L/logs/x.log".to_string()
        }
    );
    assert!(
        ownership.finding("x").is_none(),
        "shared is one log; no finding"
    );
}

#[test]
fn neither_side_naming_a_path_is_none() {
    let cron = parse_command("/bin/bash x.sh");
    assert_eq!(
        audit_destination(&cron, "#!/bin/bash\necho hi\n"),
        DestinationOwnership::None
    );
}

#[test]
fn a_line_with_no_command_has_no_destination_to_audit() {
    assert!(CronLine::parse("*/10 * * * *").is_none());
    assert!(CronLine::parse("   ").is_none());
}

// ── Invariant 2: the decoy name check ─────────────────────────────────────

#[test]
fn only_log_named_stray_redirects_get_the_stderr_suggestion() {
    assert_eq!(
        stray_redirect_name("topup", "$L/logs/cron-topup.log"),
        Some("topup.stderr".to_string())
    );
    assert_eq!(stderr_name("regsweep"), "regsweep.stderr");
    // Already reads as good news.
    assert_eq!(stray_redirect_name("topup", "$L/logs/topup.stderr"), None);
    // Not a log name at all.
    assert_eq!(stray_redirect_name("topup", "/dev/null"), None);
}

// ── Invariant 3: the no-op run must still beat ────────────────────────────

#[test]
fn the_incident_script_has_no_heartbeat_site_at_all() {
    let cov = scan_heartbeat_coverage(TOPUP_SCRIPT);
    assert_eq!(cov.total(), 0);
    assert!(!cov.beats_on_every_run());
    let finding = cov.finding("topup").expect("no sites → finding");
    assert!(finding.contains("topup"), "{finding}");
    assert!(finding.contains("no-op"), "{finding}");
}

#[test]
fn beats_in_both_branches_do_not_guarantee_the_no_op_beat() {
    let script = r#"#!/bin/bash
if [ -n "$work" ]; then
  log_heartbeat topup "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "did work"
else
  log_heartbeat topup "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "nothing to do"
fi
"#;
    let cov = scan_heartbeat_coverage(script);
    assert_eq!(cov.total(), 2);
    assert_eq!(cov.unconditional(), 0);
    assert!(!cov.beats_on_every_run());
}

#[test]
fn a_main_flow_beat_guarantees_the_no_op_beat() {
    let script = r#"#!/bin/bash
outcome="nothing to do"
if [ -n "$work" ]; then
  do_work
  outcome="did work"
fi
log_heartbeat topup "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$outcome"
"#;
    let cov = scan_heartbeat_coverage(script);
    assert_eq!(cov.total(), 1);
    assert_eq!(cov.unconditional(), 1);
    assert!(cov.beats_on_every_run());
    assert!(cov.finding("topup").is_none());
}

#[test]
fn the_inline_log_form_counts_as_a_beat() {
    let script = r#"#!/bin/bash
echo "heartbeat: topup $(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$LOG"
"#;
    let cov = scan_heartbeat_coverage(script);
    assert_eq!(cov.total(), 1);
    assert!(cov.beats_on_every_run());
}

#[test]
fn loop_bodies_and_comments_do_not_guarantee_a_beat() {
    let script = r#"#!/bin/bash
for f in "$L"/logs/*.log; do
  log_heartbeat sweep "$(date -u)" "saw $f"
done
# log_heartbeat sweep "commented out"
"#;
    let cov = scan_heartbeat_coverage(script);
    assert_eq!(cov.total(), 1, "the commented line is not a site");
    assert_eq!(
        cov.unconditional(),
        0,
        "a zero-iteration loop is the no-op run"
    );
    assert!(!cov.beats_on_every_run());
}

#[test]
fn a_word_merely_containing_heartbeat_is_not_a_beat() {
    let script = "echo myheartbeats\n";
    assert_eq!(scan_heartbeat_coverage(script).total(), 0);
}

// ── Invariant 4: liveness from durable state, not log mtimes ──────────────

#[test]
fn durable_state_reports_a_healthy_job_as_live() {
    let mut st = JobStatus::new();
    st.record_success("topup", 1_000_000);
    // Interval 600s; last success 120s ago.
    assert_eq!(
        st.status_line("topup", 1_000_120, 600),
        "topup: last success 120s ago (run #1)"
    );
}

#[test]
fn durable_state_reports_a_silent_job_as_silence_not_death() {
    let mut st = JobStatus::new();
    st.record_success("regsweep", 1_000_000);
    // Four days of silence, interval 1800s: stale, described as silence.
    let line = st.status_line("regsweep", 1_000_000 + 4 * 86_400, 1800);
    assert!(
        line.starts_with("regsweep: no successful run since "),
        "{line}"
    );
    assert!(line.contains("of silence"), "{line}");
    assert!(!line.contains("dead"), "{line}");
    assert!(line.contains("(run #1)"), "{line}");
}

#[test]
fn a_job_that_only_failed_says_so() {
    let mut st = JobStatus::new();
    st.record_failure("autoscale", 1_000_000);
    st.record_failure("autoscale", 1_000_600);
    st.record_failure("autoscale", 1_001_200);
    assert_eq!(
        st.status_line("autoscale", 1_001_200, 900),
        "autoscale: no successful run on record (last attempt 0s ago, 3 consecutive failures) (run #3)"
    );
}

#[test]
fn a_job_never_observed_gets_a_line_that_says_that() {
    let st = JobStatus::new();
    assert_eq!(
        st.status_line("topup", 1_000_000, 600),
        "topup: no successful run on record"
    );
}

#[test]
fn a_success_resets_the_failure_streak() {
    let mut st = JobStatus::new();
    st.record_failure("topup", 1_000_000);
    st.record_failure("topup", 1_000_600);
    st.record_success("topup", 1_001_200);
    assert_eq!(
        st.status_line("topup", 1_001_200, 600),
        "topup: last success 0s ago (run #3)"
    );
}

#[test]
fn status_lines_cover_every_observed_job_in_name_order() {
    let mut st = JobStatus::new();
    st.record_success("topup", 1_000_000);
    st.record_failure("regsweep", 1_000_000);
    let lines = st.status_lines(1_000_000, 600);
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with("regsweep:"), "{lines:?}");
    assert!(lines[1].starts_with("topup:"), "{lines:?}");
}

#[test]
fn job_status_round_trips_through_json() {
    let mut st = JobStatus::new();
    st.record_success("topup", 1_000_000);
    st.record_failure("regsweep", 1_000_001);
    let v = st.to_json();
    let back = JobStatus::from_json(&v).expect("round-trips");
    assert_eq!(back, st);
    let empty = JobStatus::new();
    assert_eq!(JobStatus::from_json(&empty.to_json()).unwrap(), empty);
}

// ── #4293 vs #4246: decoy and dead-writer must read differently ──────────

#[test]
fn the_decoy_and_the_dead_writer_are_distinguishable_from_state() {
    // #4293 (this issue): the writer is healthy; the schedule's log is a
    // decoy. Durable state says live, and the destination audit explains
    // why the log at the schedule's path is empty.
    let mut healthy = JobStatus::new();
    healthy.record_success("topup", 1_000_000);
    let healthy_line = healthy.status_line("topup", 1_000_120, 600);
    assert!(
        healthy_line.contains("last success 120s ago"),
        "{healthy_line}"
    );

    let parsed = CronLine::parse(CRON_TOPUP).unwrap();
    let ownership = audit_destination(&parsed.command, TOPUP_SCRIPT);
    assert!(matches!(ownership, DestinationOwnership::Decoy { .. }));

    // #4246 (control): the log is the job's own and the writer stopped.
    // Durable state says stale; the destination audit is clean, so the
    // silence is about the job, not about a mis-named file.
    let mut stopped = JobStatus::new();
    stopped.record_success("topup", 1_000_000);
    let stopped_line = stopped.status_line("topup", 1_000_000 + 4 * 86_400, 600);
    assert!(
        stopped_line.contains("no successful run since"),
        "{stopped_line}"
    );

    let clean = CronLine::parse("*/10 * * * * /bin/bash $L/topup.sh").unwrap();
    assert!(matches!(
        audit_destination(&clean.command, TOPUP_SCRIPT),
        DestinationOwnership::Single {
            by: DestinationOwner::Script,
            ..
        }
    ));

    // The two lines must differ in kind, not just in numbers: one is a
    // success age, the other a silence since a timestamp.
    assert_ne!(healthy_line, stopped_line);
}

// Keep the unused-import list honest if the API shifts.
#[allow(dead_code)]
fn _pin_api() -> CronCommand {
    parse_command("/bin/bash x.sh")
}
