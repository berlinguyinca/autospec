//! Heartbeat liveness for periodic processes (issue #3995).
//!
//! Liveness is judged from the process's own heartbeat lines — never from
//! the mtime of a log file. The three acceptance scenarios:
//!
//! * a pass that ran and did nothing is still liveness;
//! * a process that did not run is reported as "no heartbeat since T",
//!   never as "dead";
//! * a check that reads a log other than the process's declared log is a
//!   broken check, not a verdict about the process at all.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::heartbeat::{
    assess_liveness, format_utc, parse_heartbeat, parse_utc, write_heartbeat, HealthVerdict,
    Liveness, LogHealthCheck,
};

/// The topup loop's cadence: one pass every ten minutes.
const INTERVAL: u64 = 600;
/// A fixed "now" so the tests never race the wall clock.
const NOW: u64 = 1_800_000_000;

fn temp_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after the epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "autospec-heartbeat-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("temp dir is created");
    path
}

fn check(process_log: &std::path::Path) -> LogHealthCheck {
    LogHealthCheck {
        step: "topup".to_string(),
        process_log: process_log.to_path_buf(),
        interval_secs: INTERVAL,
    }
}

/// AC4: a process that ran but did nothing is healthy.
#[test]
fn ran_and_did_nothing_is_live() {
    let beat = format_utc(NOW - 60);
    let lines = vec![
        "2027-01-15T07:58:00Z topup: nothing to dispatch".to_string(),
        format!("heartbeat: topup {beat} nothing to do"),
    ];
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    // One beat, one minute ago, with a did-nothing outcome: still live.
    let verdict =
        check(Path::new("/logs/topup.log")).verdict(Path::new("/logs/topup.log"), &lines, NOW);
    assert_eq!(
        verdict,
        HealthVerdict::Process(Liveness::Live { silent_secs: 60 })
    );
}

/// AC4: a process that did not run is a stale heartbeat, and the wording
/// says silence — never "dead".
#[test]
fn not_run_is_stale_not_dead() {
    let lines = ["heartbeat: topup 2026-09-08T10:00:00Z dispatched 1"];
    let last = parse_utc("2026-09-08T10:00:00Z").expect("stamp is well-formed");
    assert!(
        NOW.saturating_sub(last) > 2 * INTERVAL,
        "the beat is well outside the silence window"
    );
    let verdict =
        check(Path::new("/logs/topup.log")).verdict(Path::new("/logs/topup.log"), &lines, NOW);
    let liveness = match &verdict {
        HealthVerdict::Process(l) => l,
        HealthVerdict::BrokenCheck { .. } => {
            panic!("path matched; verdict must be about the process")
        }
    };
    assert!(
        matches!(liveness, Liveness::Stale { silent_secs, .. } if *silent_secs == NOW - last),
        "expected Stale, got {liveness:?}"
    );
    let report = liveness.describe();
    assert!(
        report.starts_with("no heartbeat since 2026-09-08T10:00:00Z"),
        "report wording: {report}"
    );
    assert!(
        !report.contains("dead"),
        "a check may report silence, never death: {report}"
    );
}

/// AC4: no heartbeat at all is "no record", not "dead".
#[test]
fn no_heartbeat_is_no_record() {
    let lines = [
        "2026-09-09T13:50:00Z cron-topup: wrapper started",
        "2026-09-09T13:50:01Z wrapper exited 0",
    ];
    let verdict =
        check(Path::new("/logs/topup.log")).verdict(Path::new("/logs/topup.log"), &lines, NOW);
    assert_eq!(verdict, HealthVerdict::Process(Liveness::NoRecord));
    assert!(!Liveness::NoRecord.describe().contains("dead"));
}

/// AC4: a check that reads a log other than the process's declared log is a
/// broken check. Neither the silence of the wrong file nor a fresh beat in
/// it is evidence about the process.
#[test]
fn wrong_log_path_is_broken_check() {
    let right = Path::new("/logs/topup.log");
    let wrong = Path::new("/logs/cron-topup.log");

    // The wrong file is empty and stale — the #3995 incident. The verdict
    // must not indict the process.
    let verdict = check(right).verdict(wrong, &[], NOW);
    assert_eq!(
        verdict,
        HealthVerdict::BrokenCheck {
            expected: right.to_path_buf(),
            observed: wrong.to_path_buf(),
        }
    );

    // And a fresh beat in the wrong file must not exonerate it either.
    let fresh = ["heartbeat: topup 2026-09-09T14:06:00Z dispatched 3"];
    let verdict = check(right).verdict(wrong, &fresh, NOW);
    assert!(
        matches!(verdict, HealthVerdict::BrokenCheck { .. }),
        "a beat in the wrong file is not evidence: {verdict:?}"
    );
}

/// Silence at exactly the threshold is still live; one second past is stale.
#[test]
fn staleness_boundary() {
    let at = NOW - 2 * INTERVAL;
    let lines = [format!("heartbeat: topup {} did work", format_utc(at))];
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let step = "topup";
    assert!(
        matches!(
            assess_liveness(step, &lines, INTERVAL, at + 2 * INTERVAL),
            Liveness::Live { silent_secs: 1200 }
        ),
        "at the threshold the process is still live"
    );
    assert!(
        matches!(
            assess_liveness(step, &lines, INTERVAL, at + 2 * INTERVAL + 1),
            Liveness::Stale {
                silent_secs: 1201,
                ..
            }
        ),
        "one second past the threshold the check reports silence"
    );
}

/// A beat stamped in the future is a clock anomaly, but it can never make a
/// process look stale.
#[test]
fn future_beat_is_live() {
    let lines = ["heartbeat: topup 2030-01-01T00:00:00Z ok"];
    let verdict = assess_liveness("topup", &lines, INTERVAL, NOW);
    assert_eq!(verdict, Liveness::Live { silent_secs: 0 });
}

/// Heartbeats for other steps are ignored.
#[test]
fn other_steps_are_ignored() {
    let lines = [
        "heartbeat: regsweep 2026-09-09T14:05:00Z swept 0",
        "heartbeat: autoscale 2026-09-09T14:05:00Z at capacity",
    ];
    assert_eq!(
        assess_liveness("topup", &lines, INTERVAL, NOW),
        Liveness::NoRecord
    );
}

/// The newest beat wins, regardless of line order.
#[test]
fn newest_beat_wins() {
    let lines = [
        "heartbeat: topup 2026-09-09T13:00:00Z dispatched 1",
        "heartbeat: topup 2026-09-09T14:00:00Z dispatched 2",
        "heartbeat: topup 2026-09-09T13:30:00Z dispatched 1",
    ];
    let last = parse_heartbeat(lines[1]).expect("well-formed heartbeat");
    assert_eq!(
        assess_liveness("topup", &lines, INTERVAL, NOW),
        Liveness::Stale {
            silent_secs: NOW - last.at,
            last,
        }
    );
}

#[test]
fn parse_shell_shape_and_outcome_shape() {
    // The shell library's shape: step + stamp, no outcome.
    let hb = parse_heartbeat("heartbeat: topup 2026-09-09T14:00:00Z")
        .expect("shell-shaped heartbeat parses");
    assert_eq!(hb.step, "topup");
    assert_eq!(hb.at, parse_utc("2026-09-09T14:00:00Z").unwrap());
    assert!(hb.outcome.is_empty());

    // The outcome shape: the rest of the line is the outcome.
    let hb = parse_heartbeat("heartbeat: topup 2026-09-09T14:00:00Z dispatched 3")
        .expect("outcome-shaped heartbeat parses");
    assert_eq!(hb.outcome, "dispatched 3");
}

#[test]
fn parse_rejects_non_heartbeats() {
    let bad = [
        "",
        "2026-09-09T14:00:00Z dispatch started",
        "heartbeat: topup",
        "heartbeat: topup not-a-date",
        "heartbeat: topup 2026-09-09T14:00:00+00:00",
        "heartbeat: topup 2023-02-29T00:00:00Z",
        "heartbeat: 2026-09-09T14:00:00Z",
        "HEARTBEAT: topup 2026-09-09T14:00:00Z",
    ];
    for line in bad {
        assert_eq!(parse_heartbeat(line), None, "{line:?} must not parse");
    }
}

#[test]
fn utc_format_and_parse_round_trip() {
    // The format carries a 4-digit year, so the representable range ends at
    // 9999-12-31T23:59:59Z.
    for &secs in &[
        0u64,
        1,
        86_399,
        86_400,
        1_709_164_800,
        NOW,
        253_385_065_845, // 9999-06-15T12:30:45Z
        253_402_300_799, // 9999-12-31T23:59:59Z
    ] {
        let stamp = format_utc(secs);
        assert_eq!(parse_utc(&stamp), Some(secs), "round trip at {secs}");
    }
    // Pinned known values.
    assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(parse_utc("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(format_utc(1_709_164_800), "2024-02-29T00:00:00Z");
    assert_eq!(parse_utc("2024-02-29T00:00:00Z"), Some(1_709_164_800));
}

#[test]
fn parse_rejects_impossible_dates_and_shaped_wrong_stamps() {
    let bad = [
        "2023-02-29T00:00:00Z", // not a leap year
        "2026-13-01T00:00:00Z",
        "2026-01-00T00:00:00Z",
        "2026-01-32T00:00:00Z",
        "2026-09-09T24:00:00Z",
        "2026-09-09T12:60:00Z",
        "2026-09-09T12:00:60Z",
        "2026-09-09 12:00:00Z",
        "2026-09-09T12:00:00",
        "2026-09-09T12:00:00.5Z",
        "26-09-09T12:00:00Z",
        "10000-01-01T00:00:00Z", // 5-digit year: out of range for the 4-digit-year format
    ];
    for stamp in bad {
        assert_eq!(parse_utc(stamp), None, "{stamp:?} must not parse");
    }
}

#[test]
fn writer_appends_parseable_lines() {
    let dir = temp_dir("writer");
    let log = dir.join("topup.log");
    let at = UNIX_EPOCH + std::time::Duration::from_secs(NOW);

    write_heartbeat(&log, "topup", at, "dispatched 3").expect("write succeeds");
    let first = fs::read_to_string(&log).expect("log is readable");
    assert_eq!(
        first,
        format!("heartbeat: topup {} dispatched 3\n", format_utc(NOW))
    );

    // A second pass appends; the newest beat wins.
    write_heartbeat(&log, "topup", at, "nothing to do").expect("append succeeds");
    let all = fs::read_to_string(&log).expect("log is readable");
    let lines: Vec<&str> = all.lines().collect();
    assert_eq!(lines.len(), 2);
    let newest = autospec_core::heartbeat::latest_heartbeat(&lines, "topup").expect("beat");
    assert_eq!(newest.outcome, "nothing to do");
    for line in &lines {
        let hb = parse_heartbeat(line).expect("every written line parses back");
        assert_eq!(hb.at, NOW);
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn writer_matches_shell_shape_without_outcome() {
    let dir = temp_dir("writer-shell");
    let log = dir.join("sub").join("topup.log");
    let at = UNIX_EPOCH + std::time::Duration::from_secs(NOW);

    write_heartbeat(&log, "topup", at, "").expect("write succeeds");
    let line = fs::read_to_string(&log).expect("log is readable");
    // The exact shape scripts/lib/autospec-log-status.sh emits: prefix, step,
    // UTC stamp, nothing else.
    assert_eq!(line, format!("heartbeat: topup {}\n", format_utc(NOW)));

    let _ = fs::remove_dir_all(&dir);
}
