//! Integration tests for `autospec_core::log_freshness` (issue #4246).
//!
//! One fixture test per rule, plus a regression test that reconstructs
//! the incident: the three-day-old `cron-regsweep.log` whose last line
//! looked exactly like current state, the two-minute-old `regsweep.log`
//! that was, and the gateway that said 10 workers.

use autospec_core::log_freshness::{
    age_secs, authoritative_log, error_log_standing, freshness, hit_line, monument_finding,
    outage_grounding, read_last_line, sweep_report, sweep_verdict, AuthoritativeLog,
    ErrorLogHygiene, ErrorLogStanding, Freshness, LastLineReading, LiveState, LogFile, LogRole,
    OutageGrounding, SweepHit, SweepVerdict,
};

/// The incident, in seconds. `NOW` is the moment of the sweep
/// (Sep 10 17:07); the supervisor error log was last written
/// Sep 7 09:45, 3d 7h 22m earlier; the component's own log was
/// written 2 minutes before the sweep.
const NOW: u64 = 1_800_000_000;
const CRON_LOG_MTIME: u64 = NOW - 285_720; // 3d 7h 22m
const REGSWEEP_LOG_MTIME: u64 = NOW - 120; // 2m
/// The investigation window: 15 minutes.
const WINDOW: u64 = 900;

/// `cron-regsweep.log`: 191 bytes, two error lines, never written to
/// again.
fn cron_log() -> LogFile {
    LogFile {
        path: "cron-regsweep.log".into(),
        mtime_secs: CRON_LOG_MTIME,
        role: LogRole::ErrorOnly,
    }
}

/// `regsweep.log`: written 2 minutes before the sweep, ending
/// `sweep done: pool=10 gateway=10`.
fn regsweep_log() -> LogFile {
    LogFile {
        path: "regsweep.log".into(),
        mtime_secs: REGSWEEP_LOG_MTIME,
        role: LogRole::Operational,
    }
}

// ── Rule 1: mtime is part of the evidence ─────────────────────────────────

#[test]
fn age_is_saturating_and_the_window_edge_is_fresh() {
    assert_eq!(age_secs(NOW, CRON_LOG_MTIME), 285_720);
    // A clock that rewinds (mtime after now) is zero age, never an
    // underflow.
    assert_eq!(age_secs(NOW, NOW + 500), 0);
    assert_eq!(freshness(NOW, NOW + 500, WINDOW), Freshness::Fresh);
    // Exactly at the window edge is not *older than* the window.
    assert_eq!(freshness(NOW, NOW - WINDOW, WINDOW), Freshness::Fresh);
    assert_eq!(freshness(NOW, NOW - WINDOW - 1, WINDOW), Freshness::Stale);
    assert_eq!(freshness(NOW, CRON_LOG_MTIME, WINDOW), Freshness::Stale);
    assert_eq!(freshness(NOW, REGSWEEP_LOG_MTIME, WINDOW), Freshness::Fresh);
}

#[test]
fn hit_line_carries_the_mtime_and_flags_stale_matches() {
    let hit = SweepHit {
        file: cron_log(),
        line: "regsweep.sh: line 94: unexpected EOF while looking for matching `''".into(),
    };
    let line = hit_line(&hit, NOW, WINDOW);
    assert!(
        line.contains("STALE"),
        "a stale match must be flagged plainly: {line}"
    );
    assert!(
        line.contains("3d 7h"),
        "the match must carry the file's age: {line}"
    );
    assert!(
        line.contains("cron-regsweep.log"),
        "the match must name its file: {line}"
    );
    assert!(
        line.contains("line 94: unexpected EOF"),
        "the match itself, verbatim: {line}"
    );

    let fresh = SweepHit {
        file: regsweep_log(),
        line: "sweep done: pool=10 gateway=10".into(),
    };
    let line = hit_line(&fresh, NOW, WINDOW);
    assert!(
        !line.contains("STALE"),
        "a fresh match is not flagged stale: {line}"
    );
    assert!(
        line.contains("last written 2m ago"),
        "the fresh match carries its age too: {line}"
    );
}

// ── The sweep verdict ─────────────────────────────────────────────────────

#[test]
fn the_incident_sweep_is_stale_only_and_says_so() {
    // The sweep greps both logs for error|fail. The healthy log's last
    // line does not match; only the supervisor log's two error lines do.
    let hits = incident_hits();
    assert_eq!(sweep_verdict(&hits, NOW, WINDOW), SweepVerdict::StaleOnly);

    let report = sweep_report(&hits, NOW, WINDOW);
    assert!(
        report.contains("STALE"),
        "the report flags the stale matches: {report}"
    );
    assert!(
        report.contains("evidence about the past"),
        "the verdict says the sweep is not evidence about now: {report}"
    );
    assert!(
        report.contains("query the live state"),
        "the verdict names the check that would answer: {report}"
    );

    // The other verdicts: nothing, all fresh, mixed.
    assert_eq!(sweep_verdict(&[], NOW, WINDOW), SweepVerdict::Empty);
    assert_eq!(sweep_report(&[], NOW, WINDOW), "no matches");
    let fresh_hits = vec![SweepHit {
        file: regsweep_log(),
        line: "error: transient blip".into(),
    }];
    assert_eq!(
        sweep_verdict(&fresh_hits, NOW, WINDOW),
        SweepVerdict::AllFresh
    );
    let mixed = vec![fresh_hits[0].clone(), incident_hits()[0].clone()];
    assert_eq!(sweep_verdict(&mixed, NOW, WINDOW), SweepVerdict::Mixed);
}

// ── Rule 2: state beats history ──────────────────────────────────────────

#[test]
fn a_live_query_decides_and_a_log_alone_cannot_declare() {
    // The authoritative answer was a 15-second query.
    let gateway = LiveState {
        source: "gateway /v1/workers".into(),
        detail: "10 registered, all four models present".into(),
        healthy: true,
    };
    // Live query taken and healthy: the outage claim is refuted by
    // state, whatever the sweep said.
    match outage_grounding(285_720, WINDOW, true, Some(&gateway)) {
        OutageGrounding::Live { supports, detail } => {
            assert!(
                !supports,
                "a healthy live state does not support an outage claim"
            );
            assert!(detail.contains("gateway /v1/workers"), "{detail}");
        }
        other => panic!("expected Live, got {other:?}"),
    }
    // A live query that observes unhealth supports the claim — the log
    // is still not the basis.
    let sick = LiveState {
        source: "squeue".into(),
        detail: "0 of 10 workers registered".into(),
        healthy: false,
    };
    match outage_grounding(0, WINDOW, true, Some(&sick)) {
        OutageGrounding::Live { supports, .. } => {
            assert!(supports, "an unhealthy live observation supports the claim")
        }
        other => panic!("expected Live, got {other:?}"),
    }
    // The incident's actual state: no query was taken, but state is
    // queryable. A log-grounded claim is refused — the agent had a
    // 15-second query and did not take it.
    assert_eq!(
        outage_grounding(285_720, WINDOW, true, None),
        OutageGrounding::RefusedStateQueryable
    );
    // State not queryable and the log fresh: the best available
    // evidence.
    assert_eq!(
        outage_grounding(120, WINDOW, false, None),
        OutageGrounding::FreshLog
    );
    // State not queryable and the log stale: refused, with the age.
    assert_eq!(
        outage_grounding(285_720, WINDOW, false, None),
        OutageGrounding::RefusedStale { age_secs: 285_720 }
    );
}

// ── Rule 3: two logs for one component is a trap ─────────────────────────

#[test]
fn the_fresh_log_is_the_present_and_the_stale_error_log_is_not() {
    assert_eq!(
        read_last_line(&cron_log(), NOW, WINDOW),
        LastLineReading::History { age_secs: 285_720 }
    );
    assert_eq!(
        read_last_line(&regsweep_log(), NOW, WINDOW),
        LastLineReading::Now
    );
    // Two logs for the component: the fresh operational log is the
    // present; the stale error log is a monument, not the present.
    let logs = vec![cron_log(), regsweep_log()];
    assert_eq!(
        authoritative_log(&logs, NOW, WINDOW),
        AuthoritativeLog::Fresh("regsweep.log".into())
    );
    // Only the stale error log: no log of the component says what is
    // happening.
    assert_eq!(
        authoritative_log(&[cron_log()], NOW, WINDOW),
        AuthoritativeLog::NoneFresh {
            oldest_age_secs: 285_720
        }
    );
    // No logs at all: same verdict, zero age.
    assert_eq!(
        authoritative_log(&[], NOW, WINDOW),
        AuthoritativeLog::NoneFresh { oldest_age_secs: 0 }
    );
}

// ── Rule 4: a stale error file should not survive its cause ──────────────

#[test]
fn a_stale_error_log_without_safeguards_is_a_monument() {
    let bare = ErrorLogHygiene {
        per_line_timestamps: false,
        rotates: false,
    };
    assert_eq!(
        error_log_standing(&cron_log(), &bare, NOW, WINDOW),
        ErrorLogStanding::Monument
    );
    let finding =
        monument_finding(&cron_log(), &bare, NOW, WINDOW).expect("a monument names its fix");
    assert!(finding.contains("cron-regsweep.log"), "{finding}");
    assert!(finding.contains("3d 7h"), "{finding}");

    // A safeguard makes the stale entry dated or bounded, not a
    // monument.
    let timestamped = ErrorLogHygiene {
        per_line_timestamps: true,
        rotates: false,
    };
    assert_eq!(
        error_log_standing(&cron_log(), &timestamped, NOW, WINDOW),
        ErrorLogStanding::Dated
    );
    assert_eq!(
        monument_finding(&cron_log(), &timestamped, NOW, WINDOW),
        None
    );
    let rotated = ErrorLogHygiene {
        per_line_timestamps: false,
        rotates: true,
    };
    assert_eq!(
        error_log_standing(&cron_log(), &rotated, NOW, WINDOW),
        ErrorLogStanding::Dated
    );

    // A fresh error log is the present, whatever its hygiene.
    let fresh_err = LogFile {
        path: "cron-regsweep.log".into(),
        mtime_secs: NOW - 30,
        role: LogRole::ErrorOnly,
    };
    assert_eq!(
        error_log_standing(&fresh_err, &bare, NOW, WINDOW),
        ErrorLogStanding::RecentFailure
    );
}

// ── The regression: the false outage never declares ──────────────────────

fn incident_hits() -> Vec<SweepHit> {
    vec![
        SweepHit {
            file: cron_log(),
            line: "regsweep.sh: line 94: unexpected EOF while looking for matching `''".into(),
        },
        SweepHit {
            file: cron_log(),
            line: "regsweep.sh: line 95: syntax error: unexpected end of file".into(),
        },
    ]
}

#[test]
fn regression_the_three_day_old_log_does_not_declare_an_outage() {
    // Step 1 — the sweep: only the three-day-old error log matches.
    // The detection rule flags every match with its age, and the
    // verdict says the sweep is evidence about the past.
    let hits = incident_hits();
    assert_eq!(sweep_verdict(&hits, NOW, WINDOW), SweepVerdict::StaleOnly);
    for hit in &hits {
        assert!(hit_line(hit, NOW, WINDOW).contains("STALE"));
    }

    // Step 2 — the two logs for the component disagree. The fresh one
    // (written 2 minutes earlier, ending `sweep done: pool=10
    // gateway=10`) is the present.
    assert_eq!(
        authoritative_log(&[cron_log(), regsweep_log()], NOW, WINDOW),
        AuthoritativeLog::Fresh("regsweep.log".into())
    );

    // Step 3 — the 15-second query the agent skipped: 10 registered
    // workers. The outage claim, whatever the sweep said, is decided
    // by state — and state says no.
    let gateway = LiveState {
        source: "gateway /v1/workers".into(),
        detail: "10 registered, all four models present".into(),
        healthy: true,
    };
    match outage_grounding(age_secs(NOW, CRON_LOG_MTIME), WINDOW, true, Some(&gateway)) {
        OutageGrounding::Live { supports, .. } => {
            assert!(!supports, "the outage claim is refuted by the live state")
        }
        other => panic!("expected Live, got {other:?}"),
    }

    // Step 4 — the file itself is the defect to fix: a stale
    // error-only log with neither timestamps nor rotation.
    let bare = ErrorLogHygiene {
        per_line_timestamps: false,
        rotates: false,
    };
    assert!(
        monument_finding(&cron_log(), &bare, NOW, WINDOW).is_some(),
        "the three-day-old error log is a monument until it gains a safeguard"
    );
}
