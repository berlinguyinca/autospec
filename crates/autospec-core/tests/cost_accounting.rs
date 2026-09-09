//! Integration tests for GPU-hour cost accounting against the synthetic fleet
//! run history from #3793 (341 runs, 556.5 GPU-hours).

use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use autospec_core::cost::{scan_out_dir, summarize};

/// 2026-09-01T00:00:00Z
const BASE: i64 = 1_788_220_800;

/// (status, run count, total agent seconds) — one row per terminal status in
/// the #3793 table; the totals sum to 341 runs / 2,003,400 s (556.5 h).
const FLEET: &[(&str, u64, u64)] = &[
    ("VERIFIED", 177, 943_200),
    ("NEW-TEST-FAILURES", 125, 697_680),
    ("TIMEOUT", 9, 204_480),
    ("FMT-DIRTY", 11, 57_960),
    ("TEST-TIMEOUT", 9, 49_320),
    ("NO-OUTPUT", 9, 42_120),
    ("BUILD-FAIL", 1, 8_640),
];

struct Dir {
    path: PathBuf,
}

impl Dir {
    fn new(tag: &str) -> Dir {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("autospec-cost-it-{tag}-{}-{nanos}", process::id()));
        fs::create_dir_all(&path).unwrap();
        Dir { path }
    }

    fn write(&self, issue: &str, lines: &[(&str, &str)]) {
        let dir = self.path.join(issue);
        fs::create_dir_all(&dir).unwrap();
        let body: String = lines
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(dir.join("status.txt"), body + "\n").unwrap();
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Per-run seconds for a status row: the first `rem` runs get one extra second
/// so the row total is exact.
fn secs_split(total: u64, n: u64) -> Vec<u64> {
    let base = total / n;
    let rem = total % n;
    (0..n).map(|i| base + if i < rem { 1 } else { 0 }).collect()
}

fn fleet_report(dir: &Dir, stamps: bool) -> (autospec_core::cost::CostScan, Vec<u64>) {
    let mut per_run = Vec::new();
    for &(status, count, total) in FLEET {
        for secs in secs_split(total, count) {
            let issue = format!("issue-{:03}", per_run.len());
            let secs_u = secs;
            let secs = secs.to_string();
            let mut lines: Vec<(&str, &str)> = vec![
                ("status", status),
                ("agent_secs", secs.as_str()),
                ("gpus", "1"),
                ("worker", "w-1"),
            ];
            let at = if stamps {
                iso_z(BASE + (per_run.len() as i64) * 600)
            } else {
                String::new()
            };
            if !at.is_empty() {
                lines.push(("finished_at", at.as_str()));
            }
            dir.write(&issue, &lines);
            per_run.push(secs_u);
        }
    }
    let scan = scan_out_dir(&dir.path).unwrap();
    (scan, per_run)
}

/// Render `epoch` seconds as `YYYY-MM-DDTHH:MM:SSZ` (inverse of the
/// `days_from_civil`-based parser in `cost::time`).
fn iso_z(epoch: i64) -> String {
    let days = (epoch.div_euclid(86_400)) as i128;
    let secs = epoch.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    format!(
        "{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z",
        h = secs / 3600,
        mi = (secs % 3600) / 60,
        s = secs % 60
    )
}

#[test]
fn fleet_history_reports_totals_shares_and_defects() {
    let dir = Dir::new("fleet");
    let (scan, _per_run) = fleet_report(&dir, false);
    let report = summarize("out", &scan, None, 10.0);
    let cum = &report.cumulative;

    assert_eq!(cum.records, 341);
    assert!((cum.total_gpu_hours - 556.5).abs() < 0.01);
    assert_eq!(cum.by_status.len(), 7);
    // Sorted by GPU-hours, descending.
    assert_eq!(cum.by_status[0].status, "VERIFIED");
    assert_eq!(cum.by_status[0].runs, 177);
    assert!((cum.by_status[0].gpu_hours - 262.0).abs() < 0.01);
    let shares: f64 = cum.by_status.iter().map(|b| b.share_percent).sum();
    assert!((shares - 100.0).abs() < 0.1);
    for bucket in &cum.by_status {
        assert_eq!(bucket.flagged, bucket.share_percent > 10.0);
    }

    // Known-defect cost: #3857 (NEW-TEST-FAILURES) and #3936 (TIMEOUT + NO-OUTPUT).
    assert_eq!(cum.defects.len(), 2);
    assert_eq!(cum.defects[0].issue, "#3857");
    assert!((cum.defects[0].gpu_hours - 193.8).abs() < 0.01);
    assert_eq!(cum.defects[1].issue, "#3936");
    assert!((cum.defects[1].gpu_hours - 68.5).abs() < 0.01);

    let flagged: std::collections::BTreeSet<_> =
        cum.flags.iter().map(|f| f.status.as_str()).collect();
    assert_eq!(
        flagged,
        ["NEW-TEST-FAILURES", "TIMEOUT", "VERIFIED",]
            .into_iter()
            .collect()
    );
}

#[test]
fn window_scope_splits_recent_from_cumulative() {
    let dir = Dir::new("window");
    let (scan, per_run) = fleet_report(&dir, true);
    // since = 170 intervals in: the last 171 runs (index >= 170) are in scope.
    let since_epoch = BASE + 170 * 600;
    let since = iso_z(since_epoch);
    let report = summarize("out", &scan, Some((since_epoch, since.clone())), 10.0);

    assert_eq!(report.since.as_deref(), Some(since.as_str()));
    let window = report.window.as_ref().expect("window scope present");
    let expected: u64 = per_run.iter().enumerate().skip(170).map(|(_, s)| *s).sum();
    assert_eq!(window.records, 171);
    assert!((window.total_gpu_hours - expected as f64 / 3600.0).abs() < 0.01);
    assert_eq!(report.cumulative.records, 341);
    assert!(window.total_gpu_hours < report.cumulative.total_gpu_hours);
}

#[test]
fn rework_hours_are_separated_from_productive() {
    let dir = Dir::new("rework");
    for (issue, disposition) in [
        ("issue-1", "productive"),
        ("issue-2", "discarded"),
        ("issue-3", "held"),
        ("issue-4", "superseded"),
    ] {
        dir.write(
            issue,
            &[
                ("status", "VERIFIED"),
                ("agent_secs", "3600"),
                ("disposition", disposition),
            ],
        );
    }
    let scan = scan_out_dir(&dir.path).unwrap();
    let report = summarize("out", &scan, None, 10.0);
    let cum = &report.cumulative;
    assert_eq!(cum.records, 4);
    assert!((cum.total_gpu_hours - 4.0).abs() < 0.01);
    assert!((cum.productive_gpu_hours - 1.0).abs() < 0.01);
    assert!((cum.rework_gpu_hours - 3.0).abs() < 0.01);
    assert_eq!(cum.by_disposition.len(), 4);
    for name in ["productive", "discarded", "held", "superseded"] {
        let bucket = cum
            .by_disposition
            .iter()
            .find(|b| b.disposition == name)
            .unwrap_or_else(|| panic!("{name} bucket missing"));
        assert_eq!(bucket.runs, 1);
        assert!((bucket.gpu_hours - 1.0).abs() < 0.01);
    }
}

#[test]
fn missing_directory_yields_an_empty_report() {
    let scan = scan_out_dir(Path::new("/nonexistent/autospec-cost-it")).unwrap();
    let report = summarize("out", &scan, None, 10.0);
    assert_eq!(report.cumulative.records, 0);
    assert!(report.window.is_none());
    assert!(report
        .to_text()
        .contains("no costed run records under out — nothing to cost"));
}

#[test]
fn json_report_is_an_object_with_schema_version() {
    let dir = Dir::new("json");
    dir.write("issue-1", &[("status", "VERIFIED"), ("agent_secs", "3600")]);
    let scan = scan_out_dir(&dir.path).unwrap();
    let report = summarize("out", &scan, None, 10.0);
    let value: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["cumulative"]["records"], 1);
    assert!(value["window"].is_null());
}
