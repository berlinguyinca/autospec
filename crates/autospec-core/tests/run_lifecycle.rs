//! A run is visible in every state it was in (issue #3606).
//!
//! The regression tests run in the configuration the incident required:
//! three dispatched runs that died with no status file (the 18-byte
//! case), a run that ran 3 h 03 m before its worker was preempted, a
//! dispatch against a worker that no longer existed, a healthy run with
//! no observable progress for 2 h 21 m, and a 1 h 47 m no-output run
//! whose transcript was destroyed with the node-local scratch.

use std::collections::BTreeMap;

use autospec_core::run_lifecycle::StatusRecord;
use autospec_core::run_lifecycle::{
    audit_dispatch, classify_loss, classify_no_output, liveness, reselect, same_endpoint_retries,
    tally, transcript_verdict, unrecorded_liveness_finding, DispatchAudit, FinishReason, Liveness,
    LossClass, NoOutputCause, StartProbe, Terminal, TranscriptEvidence, TranscriptVerdict,
    DEFAULT_FAIL_FAST_WINDOW_SECS, DEFAULT_STALE_AFTER_SECS, STALL_KILL_RC,
};

/// The three runs that died with `agent.out` at 18 bytes and no status
/// file: each directory contained `agent.out` and nothing else.
const SILENT_DEATHS: [&str; 3] = ["issue-3539", "issue-3574", "issue-3590"];

/// The endpoint that no longer existed when its agent was dispatched.
const GHOST_WORKER: &str = "http://q27-c-22682732:8080";

/// The live workers the run's connection was lost to.
const LIVE_WORKERS: [&str; 2] = ["http://q27-a-22682729:8080", "http://q27-b-22682731:8080"];

const MODEL: &str = "qwen3.8-27b";

/// Seconds issue-3590 ran before dying: 3 h 03 m.
const AS3590_ELAPSED_SECS: u64 = 10_980;

/// Seconds iw-30 had been RUNNING with `agent.out` at 0 bytes: 2 h 21 m.
const IW30_ELAPSED_SECS: u64 = 8_460;

/// Seconds issue 51 ran before it was recorded NO-OUTPUT: agent_secs=6416.
const ISSUE51_ELAPSED_SECS: u64 = 6_416;

fn running(run_id: &str, endpoint: &str) -> StatusRecord {
    StatusRecord::start(run_id, endpoint, MODEL, 100_000).unwrap()
}

fn records_for(ids: &[&str]) -> BTreeMap<String, StatusRecord> {
    ids.iter()
        .map(|id| ((*id).to_string(), running(id, "http://worker:8080")))
        .collect()
}

// --- Invariant 2: a run with no status record is FAILED in every tally ---

#[test]
fn silent_deaths_are_failures_in_every_tally() {
    // The incident: 15 runs dispatched, 12 status files, all VERIFIED.
    // The tally reported "12 VERIFIED out of 12 status files" — the
    // denominator was the files, and the three silent deaths were not in
    // it at all.
    let mut dispatched: Vec<String> = Vec::new();
    for n in 1..=15 {
        dispatched.push(format!("issue-{n}"));
    }
    let mut records: BTreeMap<String, StatusRecord> = BTreeMap::new();
    for n in 1..=12 {
        let mut record = running(&format!("issue-{n}"), "http://worker:8080");
        let mut term = Terminal::new("VERIFIED", 0, 100_000 + 600).unwrap();
        term.transcript_copied = true;
        record.finish(term).unwrap();
        records.insert(format!("issue-{n}"), record);
    }
    // The three silent deaths: dispatched, but the runner died before
    // writing the status file.
    assert_eq!(records.len(), 12);

    let tally = tally(&dispatched, &records);
    assert!(tally.reconciles());
    assert_eq!(tally.dispatched, 15);
    assert_eq!(tally.verified, 12);
    assert_eq!(tally.missing, 3);
    // The denominator is the dispatched runs, never the status files:
    // the fixed line is 12/15, not the incident's 12/12.
    let line = tally.line();
    assert_eq!(line, "WARN: tally: 12/15 verified; 3 failed (no-record=3)");
    assert!(!line.contains("12/12"), "the incident's line: {line}");
}

#[test]
fn a_killed_run_stuck_at_running_is_a_failure_not_a_success() {
    // A run the scheduler killed mid-flight leaves its RUNNING record:
    // the failure and the absence no longer look identical, and the
    // stale record is a failure in the tally, not an invisible gap.
    let dispatched = vec!["issue-1".to_string(), "issue-2".to_string()];
    let mut records = records_for(&["issue-1", "issue-2"]);
    let mut record = running("issue-1", "http://worker:8080");
    record
        .finish(Terminal::new("VERIFIED", 0, 100_600).unwrap())
        .unwrap();
    records.insert("issue-1".to_string(), record);
    // issue-2 was killed: its record still reads RUNNING.

    let tally = tally(&dispatched, &records);
    assert_eq!(tally.verified, 1);
    assert_eq!(tally.stale_running, 1);
    assert_eq!(
        tally.line(),
        "WARN: tally: 1/2 verified; 1 failed (stale-running=1)"
    );
}

// --- Invariant 1: the record is written first, not last -------------------

#[test]
fn a_killed_run_leaves_a_record_of_where_it_was() {
    // issue-3590: started against q27-a, lost the connection after
    // 3 h 03 m, and the runner died before writing the terminal state.
    // With the fix the status file already existed at start and was
    // updated on the re-dispatch: the record of where the run was is
    // the RUNNING line with the endpoint and the model.
    let mut record = running("issue-3590", LIVE_WORKERS[0]);
    record.heartbeat(101_000, 4521).unwrap();
    // The worker was preempted: the record transitions to the
    // replacement peer.
    let peers = LIVE_WORKERS
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>();
    let next = reselect(LIVE_WORKERS[0], &peers).unwrap();
    assert_eq!(next, LIVE_WORKERS[1]);
    record
        .redeploy(&next, 100_000 + AS3590_ELAPSED_SECS)
        .unwrap();

    let line = record.line();
    let expected_head = format!(
        "status=RUNNING endpoint={} model={MODEL} attempt=2 started=",
        LIVE_WORKERS[1]
    );
    assert!(
        line.starts_with(&expected_head),
        "the record does not say where the run was: {line}"
    );
    assert!(line.contains("last_heartbeat="));
    assert!(line.contains("counter=4521"));

    // "Which worker did this run use" is answerable after the fact:
    // every attempt is on the record, in order.
    assert_eq!(record.attempts[0].endpoint, LIVE_WORKERS[0]);
    assert_eq!(record.attempts[1].endpoint, LIVE_WORKERS[1]);
    assert_eq!(record.endpoint(), LIVE_WORKERS[1]);
    assert_eq!(record.attempt_number(), 2);
    assert!(same_endpoint_retries(&record).is_empty());
}

#[test]
fn a_record_cannot_start_at_its_own_verdict() {
    // The lifecycle API has no path to a terminal-first record: start()
    // is the only constructor and it is RUNNING, and finish() is the
    // only transition into the terminal state.
    let record = running("issue-1", "http://worker:8080");
    assert!(record.is_running());
    assert!(record.terminal.is_none());
    let line = record.line();
    assert!(line.starts_with("status=RUNNING "));

    let mut record = record;
    record
        .finish(Terminal::new("NO-OUTPUT", 0, 100_600).unwrap())
        .unwrap();
    assert!(!record.is_running());
    assert!(record.line().starts_with("status=NO-OUTPUT "));
}

// --- Invariant 4: unreachable at start costs seconds, not a slot ----------

#[test]
fn unreachable_at_start_costs_seconds_not_a_slot() {
    // q27-c-22682732 did not exist when its agent was dispatched. The
    // dispatch consumed a scheduled slot with no passed probe.
    assert_eq!(audit_dispatch(None, true), DispatchAudit::Unprobed);
    assert_eq!(
        audit_dispatch(Some(StartProbe::Unreachable), true),
        DispatchAudit::PastRefusal
    );
    assert!(audit_dispatch(None, true)
        .warn_line()
        .unwrap()
        .contains("seconds, not a slot"));
    assert_eq!(
        audit_dispatch(Some(StartProbe::Reachable), true),
        DispatchAudit::Sound
    );

    // After the fact: the run died within the fail-fast window with no
    // liveness line — the endpoint was unreachable at start. That is a
    // dispatch defect, a different class from a preemption after
    // progress.
    let mut record = running("issue-3590", GHOST_WORKER);
    record
        .finish(Terminal::new("connection-error", 1, 100_000 + 8).unwrap())
        .unwrap();
    assert_eq!(
        classify_loss(&record, DEFAULT_FAIL_FAST_WINDOW_SECS),
        Some(LossClass::UnreachableAtStart)
    );

    // The 3 h 03 m run is the other class: the endpoint answered for
    // three hours, so the loss is a preemption, and the missing
    // liveness lines are the separate finding.
    let mut record = running("issue-3590", LIVE_WORKERS[0]);
    record
        .finish(Terminal::new("connection-error", 1, 100_000 + AS3590_ELAPSED_SECS).unwrap())
        .unwrap();
    assert_eq!(
        classify_loss(&record, DEFAULT_FAIL_FAST_WINDOW_SECS),
        Some(LossClass::PreemptedMidRun)
    );
}

// --- Invariant 3: liveness is written while work is happening -------------

#[test]
fn a_healthy_run_with_no_lines_is_the_state_a_supervisor_cannot_read() {
    // iw-30: RUNNING for 2 h 21 m, agent.out at 0 bytes. Every
    // observable said dead; the run was working perfectly. From the
    // run's own output the state was unreadable — and that is the
    // finding.
    let record = running("iw-30", "http://worker:8080");
    assert_eq!(
        liveness(
            &record,
            100_000 + IW30_ELAPSED_SECS,
            DEFAULT_STALE_AFTER_SECS
        ),
        Liveness::Unrecorded
    );
    let finding = unrecorded_liveness_finding(&record).unwrap();
    assert!(finding.contains("iw-30"));
    assert!(finding.contains("indistinguishable"));
}

#[test]
fn progressing_is_distinguishable_from_stopped_on_the_runs_own_output() {
    // With heartbeat lines, the same 2 h 21 m window is readable from
    // the run's own output: the counter advances, so the run is
    // progressing, and no inspection of the six inference hosts is
    // needed.
    let now = 100_000 + IW30_ELAPSED_SECS;
    let mut record = running("iw-30", "http://worker:8080");
    record.heartbeat(now - 160, 1_000).unwrap();
    record.heartbeat(now - 60, 61_494).unwrap();
    assert_eq!(
        liveness(&record, now, DEFAULT_STALE_AFTER_SECS),
        Liveness::Progressing
    );
    assert!(unrecorded_liveness_finding(&record).is_none());

    // A line whose counter does not advance: the process is alive,
    // progress is unproven — not a kill decision on its own.
    let mut record = running("iw-31", "http://worker:8080");
    record.heartbeat(now - 160, 5_000).unwrap();
    record.heartbeat(now - 60, 5_000).unwrap();
    assert_eq!(
        liveness(&record, now, DEFAULT_STALE_AFTER_SECS),
        Liveness::Alive
    );

    // And a corpse with lines: silence past the window is a real
    // signal, with the count of how long.
    let mut record = running("iw-32", "http://worker:8080");
    record.heartbeat(100_100, 5_000).unwrap();
    assert_eq!(
        liveness(
            &record,
            100_000 + IW30_ELAPSED_SECS,
            DEFAULT_STALE_AFTER_SECS
        ),
        Liveness::Stalled {
            silent_secs: IW30_ELAPSED_SECS - 100
        }
    );
}

// --- Invariant 6: a failure must not destroy the evidence it names --------

#[test]
fn a_no_output_run_keeps_its_transcript() {
    // issue 51: status=NO-OUTPUT, agent_rc=0, agent_secs=6416,
    // agent.out=0 bytes. The runner's own comment names the mechanism:
    // the maxTokens budget was spent in reasoning_content and the model
    // returned empty content with finish=length. With the transcript
    // destroyed with the node-local scratch, the four causes are one
    // status string.
    let mut term = Terminal::new("NO-OUTPUT", 0, 100_000 + ISSUE51_ELAPSED_SECS).unwrap();
    term.finish_reason = Some(FinishReason::Length);
    term.input_tokens = Some(65_536);
    term.output_tokens = Some(32_768);

    // The transcript was not copied: the failure named the evidence it
    // destroyed.
    match transcript_verdict(&term) {
        TranscriptVerdict::DestroyedEvidence { reason } => {
            assert!(reason.contains("NO-OUTPUT"));
        }
        other => panic!("the 1 h 47 m no-output run must not lose its transcript: {other:?}"),
    }
    // And the cause is Indeterminate, never a guess — so the retry
    // decision is not made on it either.
    assert_eq!(
        classify_no_output(&term, None),
        NoOutputCause::Indeterminate
    );
    assert!(!NoOutputCause::Indeterminate.retry_worthy());

    // The status line carries the fields that would have said so:
    // finish_reason and the token counts are fields, not a comment in a
    // shell script.
    let mut record = running("issue-51", "http://worker:8080");
    record.finish(term.clone()).unwrap();
    let line = record.line();
    assert!(line.contains("finish_reason=length"));
    assert!(line.contains("input_tokens=65536"));
    assert!(line.contains("output_tokens=32768"));
    assert!(line.contains("transcript=no"));

    // With the transcript copied, the four causes separate, and two of
    // them are worth retrying and two are not. The burn case keeps the
    // `finish=length` token; the rest settle with `finish=stop`.
    let term = Terminal {
        transcript_copied: true,
        ..term
    };
    assert_eq!(transcript_verdict(&term), TranscriptVerdict::Kept);
    assert_eq!(
        classify_no_output(&term, Some(&TranscriptEvidence::default())),
        NoOutputCause::ReasoningBurn
    );
    assert!(NoOutputCause::ReasoningBurn.retry_worthy());
    let clean = Terminal {
        finish_reason: Some(FinishReason::Stop),
        ..term
    };
    assert_eq!(
        classify_no_output(
            &clean,
            Some(&TranscriptEvidence {
                tool_calls: 9,
                harness_error: false
            })
        ),
        NoOutputCause::NoChangeNeeded
    );
    assert!(!NoOutputCause::NoChangeNeeded.retry_worthy());
    assert_eq!(
        classify_no_output(&clean, Some(&TranscriptEvidence::default())),
        NoOutputCause::ReadWithoutActing
    );
    assert!(!NoOutputCause::ReadWithoutActing.retry_worthy());
    assert_eq!(
        classify_no_output(
            &clean,
            Some(&TranscriptEvidence {
                tool_calls: 0,
                harness_error: true
            })
        ),
        NoOutputCause::HarnessFailure
    );
    assert!(NoOutputCause::HarnessFailure.retry_worthy());
}

#[test]
fn a_stall_killed_run_keeps_its_transcript() {
    // issue 54: agent_rc=143 — SIGTERM, the stall watchdog fired.
    // Non-zero exit: the transcript is load-bearing.
    let term = Terminal::new("NO-OUTPUT", STALL_KILL_RC, 100_000 + 2_881).unwrap();
    match transcript_verdict(&term) {
        TranscriptVerdict::DestroyedEvidence { reason } => {
            assert!(reason.contains("143"));
            assert!(reason.contains("stall-killed"));
        }
        other => panic!("a stall-killed run must keep its transcript: {other:?}"),
    }
}

// --- Invariant 5: retry against a different pool member -------------------

#[test]
fn a_lost_connection_is_retried_against_a_different_peer() {
    // The re-read of the endpoint directory reports the healthy peers;
    // the retry picks one of them, never the endpoint that just died.
    let peers = [GHOST_WORKER, LIVE_WORKERS[0], LIVE_WORKERS[1]]
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>();
    let next = reselect(GHOST_WORKER, &peers).unwrap();
    assert_eq!(next, LIVE_WORKERS[0]);
    let next = reselect(LIVE_WORKERS[0], &peers).unwrap();
    assert_eq!(next, LIVE_WORKERS[1]);

    // A re-read that reports only the dead worker is a stale directory:
    // the refusal fails closed and names the endpoint it will not
    // re-grant.
    let only_dead = [GHOST_WORKER]
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>();
    let err = reselect(GHOST_WORKER, &only_dead).unwrap_err();
    assert!(err.line().contains(GHOST_WORKER));

    // And a record that retried the endpoint its previous attempt just
    // lost is a finding.
    let mut record = running("issue-3574", GHOST_WORKER);
    record.redeploy(GHOST_WORKER, 100_010).unwrap();
    assert_eq!(same_endpoint_retries(&record), vec![2]);
}

#[test]
fn every_silent_death_dir_now_leaves_a_record() {
    // The three 18-byte runs, under the fixed lifecycle: each one
    // starts with its record, and even the one that dies fastest leaves
    // a status file the tally can count.
    let dispatched: Vec<String> = SILENT_DEATHS.iter().map(|s| s.to_string()).collect();
    let mut records = BTreeMap::new();
    for id in &SILENT_DEATHS {
        let mut record = running(id, GHOST_WORKER);
        record
            .finish(Terminal::new("connection-error", 1, 100_000 + 8).unwrap())
            .unwrap();
        records.insert(id.to_string(), record);
    }
    let tally = tally(&dispatched, &records);
    assert!(tally.reconciles());
    assert_eq!(tally.missing, 0);
    assert_eq!(tally.failed, 3);
    assert_eq!(
        tally.line(),
        "WARN: tally: 0/3 verified; 3 failed (terminal=3)"
    );
    for id in &SILENT_DEATHS {
        let record = records.get(*id).unwrap();
        assert_eq!(
            classify_loss(record, DEFAULT_FAIL_FAST_WINDOW_SECS),
            Some(LossClass::UnreachableAtStart)
        );
    }
}
