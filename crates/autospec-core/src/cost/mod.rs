//! GPU-hour cost accounting over the fleet's per-run status records.
//!
//! The fleet harness leaves one directory per run under `out/`
//! (`out/issue-*/`), and a run that reaches a terminal status writes a
//! `status.txt` recording how long it ran and on how many GPUs. This module
//! reads those records and answers the questions that were being answered by
//! hand:
//!
//! - How many runs and GPU-hours did each terminal status cost, in a window
//!   and cumulatively?
//! - How much of that is *rework* — runs later discarded, held, or superseded
//!   — versus productive time?
//! - How many hours map onto the known defect issues (#3857, #3936)?
//! - Does any status exceed its share threshold?
//!
//! Guarantees:
//!
//! - **No invented numbers.** GPU-hours come only from records that carry
//!   both a terminal `status` and an `agent_secs` value. Runs that died
//!   before recording either are surfaced as `incomplete` (or `no_record` /
//!   `malformed`), never silently dropped from the denominator.
//! - **Observed, not potential.** Every GPU-hour figure in the report is
//!   *observed* — measured from a costed run record — and is labelled as such.
//!   Queue exposure (the runs with no costed record: `incomplete`,
//!   `no_record`, or `malformed`) is *potential* cost: it is rendered as a
//!   count with the condition that would realise it (the run finishes and
//!   records a terminal status plus `agent_secs`), never in GPU-hours.
//! - **Deterministic.** Records are scanned in directory order and the
//!   report renders in fixed order (buckets by GPU-hours descending, status
//!   name as the tie-break), so two runs of the command over the same
//!   `out/` tree produce identical output.
//! - **Tolerant of growth, strict about what it reads.** Unknown keys in a
//!   `status.txt` are ignored (the harness may add lines); a known key with
//!   an unreadable value fails that file into `malformed` with a reason,
//!   because a silently dropped number would understate the very cost this
//!   module exists to surface.
//!
//! The window is selected by a `--since` ISO-8601 instant: a record counts
//! toward the window when its `finished_at` (or `started_at`, for a run that
//! never finished) is at or after the instant. Records with no timestamp
//! count toward the cumulative total only.

mod record;
mod report;
mod scan;
mod time;

/// Terminal statuses mapped onto the known defect issues, in report order.
pub const DEFAULT_DEFECT_MAP: &[(&str, &str)] = &[
    ("NEW-TEST-FAILURES", "#3857"),
    ("TIMEOUT", "#3936"),
    ("NO-OUTPUT", "#3936"),
];

pub use record::{Disposition, RunRecord};
pub use report::{
    CostReport, CostSummary, DefectCost, DispositionBucket, IssueCost, StatusBucket, ThresholdFlag,
};
pub use scan::{scan_out_dir, CostScan, MalformedRecord};
pub use time::parse_iso8601;

use report::build_report;

/// Build the cost report for a scan: cumulative over every costed record and
/// the `--since` window when one was requested.
pub fn summarize(
    out_dir: &str,
    scan: &CostScan,
    since: Option<(i64, String)>,
    threshold_percent: f64,
) -> CostReport {
    build_report(
        out_dir,
        scan.records.len() as u64,
        &scan.records,
        &scan.no_record,
        &scan.malformed,
        since,
        threshold_percent,
    )
}

/// Whether a terminal status is productive work or rework.
pub fn disposition_is_rework(disposition: Disposition) -> bool {
    disposition.is_rework()
}

#[cfg(test)]
mod tests {
    use super::*;
    use report::build_report;
    use std::path::Path;

    fn record(status: &str, seconds: u64, finished_at: Option<i64>) -> RunRecord {
        let mut record = RunRecord::new("issue-test");
        record.status = Some(status.to_string());
        record.agent_secs = Some(seconds);
        record.finished_at = finished_at;
        record
    }

    #[test]
    fn synthetic_history_produces_expected_status_totals() {
        // The run table from the #3793 review: status, runs, GPU-hours.
        let table: &[(&str, u64, f64)] = &[
            ("VERIFIED", 177, 262.0),
            ("NEW-TEST-FAILURES", 125, 193.8),
            ("TIMEOUT", 9, 56.8),
            ("FMT-DIRTY", 11, 16.1),
            ("TEST-TIMEOUT", 9, 13.7),
            ("NO-OUTPUT", 9, 11.7),
            ("BUILD-FAIL", 1, 2.4),
        ];
        let mut records = Vec::new();
        let mut counter = 0_u64;
        for (status, runs, hours) in table {
            let total_secs = (hours * 3600.0).round() as u64;
            let base = total_secs / runs;
            for run in 0..*runs {
                let secs = if run + 1 == *runs {
                    total_secs - base * (runs - 1)
                } else {
                    base
                };
                counter += 1;
                records.push(record(status, secs, Some(1_700_000_000 + counter as i64)));
            }
        }
        let scan = CostScan {
            records,
            no_record: Vec::new(),
            malformed: Vec::new(),
        };
        let report = build_report(
            "out",
            scan.records.len() as u64,
            &scan.records,
            &scan.no_record,
            &scan.malformed,
            None,
            10.0,
        );
        let summary = &report.cumulative;
        assert_eq!(summary.records, 341);
        assert!(
            (summary.total_gpu_hours - 556.5).abs() < 0.01,
            "total: {}",
            summary.total_gpu_hours
        );
        assert_eq!(summary.by_status.len(), 7);
        // Largest bucket first, with the issue's headline numbers.
        let verified = &summary.by_status[0];
        assert_eq!(verified.status, "VERIFIED");
        assert_eq!(verified.runs, 177);
        assert!((verified.gpu_hours - 262.0).abs() < 0.01);
        assert!((verified.share_percent - 47.1).abs() < 0.1);
        let ntf = &summary.by_status[1];
        assert_eq!(ntf.status, "NEW-TEST-FAILURES");
        assert_eq!(ntf.runs, 125);
        assert!((ntf.gpu_hours - 193.8).abs() < 0.01);
        // The two leaders exceed the 10% threshold; the small ones do not.
        assert!(verified.flagged && ntf.flagged);
        assert!(!summary.by_status[6].flagged);
    }

    #[test]
    fn window_and_cumulative_agree_when_window_covers_everything() {
        let records = vec![
            record("VERIFIED", 3600, Some(100)),
            record("TIMEOUT", 7200, Some(200)),
        ];
        let no_since = build_report("out", 2, &records, &[], &[], None, 10.0);
        let since = build_report(
            "out",
            2,
            &records,
            &[],
            &[],
            Some((0, "2026-01-01T00:00:00Z".into())),
            10.0,
        );
        assert!(no_since.window.is_none());
        let window = since.window.as_ref().expect("window present with --since");
        assert_eq!(window.records, 2);
        assert_eq!(window.total_gpu_hours, no_since.cumulative.total_gpu_hours);
        assert_eq!(window.by_status, no_since.cumulative.by_status);
    }

    #[test]
    fn window_excludes_older_runs_and_records_without_stamps() {
        let mut no_stamp = record("VERIFIED", 3600, None);
        no_stamp.started_at = None;
        let records = vec![
            record("VERIFIED", 3600, Some(100)),
            record("TIMEOUT", 7200, Some(300)),
            no_stamp,
        ];
        let report = build_report(
            "out",
            3,
            &records,
            &[],
            &[],
            Some((200, "2026-06-01T00:00:00Z".into())),
            10.0,
        );
        assert_eq!(report.cumulative.records, 3);
        // All three are costed; the untimestamped one still counts toward
        // the cumulative total (it just cannot be placed in the window).
        assert!((report.cumulative.total_gpu_hours - 4.0).abs() < 0.001);
        let window = report.window.expect("window present");
        assert_eq!(window.records, 1);
        assert_eq!(window.by_status[0].status, "TIMEOUT");
        assert!((window.total_gpu_hours - 2.0).abs() < 0.001);
        assert_eq!(window.defects.len(), 1);
        assert_eq!(window.defects[0].issue, "#3936");
    }

    #[test]
    fn rework_hours_are_separated_from_productive() {
        let mut discarded = record("NEW-TEST-FAILURES", 3600, Some(100));
        discarded.disposition = Disposition::Discarded;
        let mut held = record("TIMEOUT", 3600, Some(200));
        held.disposition = Disposition::Held;
        let mut superseded = record("NO-OUTPUT", 3600, Some(300));
        superseded.disposition = Disposition::Superseded;
        let productive = record("VERIFIED", 3600, Some(400));
        let records = vec![discarded, held, superseded, productive];
        let report = build_report("out", 4, &records, &[], &[], None, 10.0);
        let summary = &report.cumulative;
        assert!((summary.productive_gpu_hours - 1.0).abs() < 0.001);
        assert!((summary.rework_gpu_hours - 3.0).abs() < 0.001);
        let by_disposition: std::collections::HashMap<_, _> = summary
            .by_disposition
            .iter()
            .map(|b| (b.disposition.as_str(), b.gpu_hours))
            .collect();
        assert!((by_disposition["productive"] - 1.0).abs() < 0.001);
        assert!((by_disposition["discarded"] - 1.0).abs() < 0.001);
        assert!((by_disposition["held"] - 1.0).abs() < 0.001);
        assert!((by_disposition["superseded"] - 1.0).abs() < 0.001);
        assert!(!disposition_is_rework(Disposition::Productive));
        assert!(disposition_is_rework(Disposition::Superseded));
    }

    #[test]
    fn defect_costs_group_statuses_on_their_issue() {
        let records = vec![
            record("NEW-TEST-FAILURES", 3600, Some(100)), // #3857: 1h
            record("TIMEOUT", 3600, Some(200)),           // #3936
            record("NO-OUTPUT", 7200, Some(300)),         // #3936
            record("VERIFIED", 3600, Some(400)),          // unmapped
        ];
        let report = build_report("out", 4, &records, &[], &[], None, 10.0);
        let defects = &report.cumulative.defects;
        assert_eq!(defects.len(), 2);
        assert_eq!(defects[0].issue, "#3857");
        assert_eq!(defects[0].statuses, vec!["NEW-TEST-FAILURES".to_string()]);
        assert!((defects[0].gpu_hours - 1.0).abs() < 0.001);
        assert_eq!(defects[1].issue, "#3936");
        assert_eq!(
            defects[1].statuses,
            vec!["TIMEOUT".to_string(), "NO-OUTPUT".to_string()]
        );
        assert!((defects[1].gpu_hours - 3.0).abs() < 0.001);
        assert!((defects[1].share_percent - 60.0).abs() < 0.1);
    }

    #[test]
    fn incomplete_records_are_listed_but_not_costed() {
        let mut no_status = RunRecord::new("issue-a");
        no_status.agent_secs = Some(3600);
        let mut no_secs = RunRecord::new("issue-b");
        no_secs.status = Some("VERIFIED".into());
        let costed = record("TIMEOUT", 3600, Some(100));
        let records = vec![no_status, no_secs, costed];
        let report = build_report("out", 3, &records, &["issue-c".into()], &[], None, 10.0);
        assert_eq!(report.cumulative.records, 1);
        assert_eq!(
            report.incomplete,
            vec!["issue-a".to_string(), "issue-b".to_string()]
        );
        assert_eq!(report.no_record, vec!["issue-c".to_string()]);
    }

    #[test]
    fn empty_scan_renders_an_empty_report_not_an_error() {
        let report = build_report("out", 0, &[], &[], &[], None, 10.0);
        assert_eq!(report.cumulative.records, 0);
        assert_eq!(report.cumulative.by_status, Vec::<StatusBucket>::new());
        let text = report.to_text();
        assert!(text.contains("no costed run records under out"), "{text}");
    }

    /// Issue #3983, AC2: the per-issue cost table ranks by cumulative hours
    /// but shows each issue's most recent outcome beside it, so a costly issue
    /// whose latest run succeeded is not misread as "failing now".
    #[test]
    fn by_issue_ranks_cumulative_hours_beside_latest_outcome() {
        fn run(issue: &str, status: &str, seconds: u64, finished_at: Option<i64>) -> RunRecord {
            let mut record = RunRecord::new(issue);
            record.status = Some(status.to_string());
            record.agent_secs = Some(seconds);
            record.finished_at = finished_at;
            record
        }
        // issue-3192: two early timeouts, then a recent success.
        // issue-3805: three consecutive timeouts, the latest run silent.
        let records = vec![
            run("issue-3192", "TIMEOUT", 7200, Some(100)),
            run("issue-3192", "TIMEOUT", 7200, Some(200)),
            run("issue-3192", "VERIFIED", 3600, Some(300)),
            run("issue-3805", "TIMEOUT", 7200, Some(100)),
            run("issue-3805", "TIMEOUT", 7200, Some(200)),
            run("issue-3805", "TIMEOUT", 7200, Some(300)),
        ];
        let report = build_report("out", 6, &records, &[], &[], None, 10.0);
        let by_issue = &report.cumulative.by_issue;
        // Ranked by cumulative hours, largest first: 3805 (6.0h) above 3192 (5.0h).
        assert_eq!(by_issue[0].issue, "issue-3805");
        assert_eq!(by_issue[0].runs, 3);
        assert!((by_issue[0].gpu_hours - 6.0).abs() < 0.001);
        assert_eq!(by_issue[0].latest_status.as_deref(), Some("TIMEOUT"));
        assert_eq!(by_issue[1].issue, "issue-3192");
        assert_eq!(by_issue[1].runs, 3);
        assert!((by_issue[1].gpu_hours - 5.0).abs() < 0.001);
        // The latest outcome is the success, not the earlier timeouts.
        assert_eq!(by_issue[1].latest_status.as_deref(), Some("VERIFIED"));
        let text = report.to_text();
        assert!(
            text.contains("issues (ranked by cumulative hours)"),
            "{text}"
        );
        assert!(text.contains("issue-3192"), "{text}");
        assert!(text.contains("VERIFIED"), "{text}");
    }

    /// A stamped run always outranks an untimestamped one for "latest"; the
    /// cost table must not let a missing stamp hide the true most-recent run.
    #[test]
    fn by_issue_prefers_the_most_recent_stamped_run_for_latest() {
        fn run(issue: &str, status: &str, seconds: u64, finished_at: Option<i64>) -> RunRecord {
            let mut record = RunRecord::new(issue);
            record.status = Some(status.to_string());
            record.agent_secs = Some(seconds);
            record.finished_at = finished_at;
            record
        }
        // An untimestamped success listed first, then a stamped timeout that
        // finished later: the timeout is the latest outcome.
        let records = vec![
            run("issue-1", "VERIFIED", 1800, None),
            run("issue-1", "TIMEOUT", 3600, Some(500)),
        ];
        let report = build_report("out", 2, &records, &[], &[], None, 10.0);
        let by_issue = &report.cumulative.by_issue;
        assert_eq!(by_issue.len(), 1);
        assert_eq!(by_issue[0].latest_status.as_deref(), Some("TIMEOUT"));
        assert!((by_issue[0].gpu_hours - 1.5).abs() < 0.001);
    }

    #[test]
    fn scan_classifies_run_directories() {
        let dir = std::env::temp_dir().join(format!(
            "autospec-cost-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("issue-1")).expect("dir");
        std::fs::write(
            dir.join("issue-1/status.txt"),
            "status: VERIFIED\nagent_secs: 3600\n",
        )
        .expect("write");
        std::fs::create_dir_all(dir.join("issue-2")).expect("dir");
        std::fs::write(dir.join("issue-2/status.txt"), "worker: g1\n").expect("write");
        std::fs::create_dir_all(dir.join("issue-3")).expect("dir");
        std::fs::write(dir.join("issue-4.txt"), "not a dir\n").expect("write");
        std::fs::create_dir_all(dir.join("issue-4")).expect("dir");
        std::fs::write(
            dir.join("issue-4/status.txt"),
            "status: VERIFIED\nagent_secs: abc\n",
        )
        .expect("write");
        let scan = scan_out_dir(&dir).expect("scan");
        assert_eq!(scan.records.len(), 2);
        assert!(scan.records[0].is_costed());
        assert!(!scan.records[1].is_costed());
        assert_eq!(scan.no_record, vec!["issue-3".to_string()]);
        assert_eq!(scan.malformed.len(), 1);
        assert_eq!(scan.malformed[0].issue, "issue-4");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_of_missing_directory_is_empty() {
        let scan = scan_out_dir(Path::new("/nonexistent-autospec-cost-out-3940")).expect("scan");
        assert!(scan.records.is_empty() && scan.no_record.is_empty());
    }
}
