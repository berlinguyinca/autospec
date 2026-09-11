//! Aggregation of scanned run records into a cost report.
//!
//! The report answers the question the fleet keeps asking by hand: *what did
//! the runs cost, what share went where, and how much of it is rework?* It
//! never invents numbers it does not have — hours come only from costed
//! records, shares are computed against the hours actually present, and every
//! bucket that exceeds the configured share threshold is flagged in the
//! report itself (flags do not fail the command: they exist to be read).

use std::collections::BTreeMap;

use serde::Serialize;

use super::record::{Disposition, RunRecord};
use super::scan::MalformedRecord;
use super::DEFAULT_DEFECT_MAP;

/// One terminal status's share of the scoped GPU-hours.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StatusBucket {
    pub status: String,
    pub runs: u64,
    pub gpu_hours: f64,
    /// Share of the scoped total GPU-hours, in percent.
    pub share_percent: f64,
    /// True when the share strictly exceeds the configured threshold.
    pub flagged: bool,
}

/// One disposition's (productive vs rework) share of the scoped GPU-hours.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DispositionBucket {
    pub disposition: String,
    pub runs: u64,
    pub gpu_hours: f64,
}

/// Hours attributable to one known defect issue (#3857, #3936), summed over
/// the terminal statuses that map to it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DefectCost {
    pub issue: String,
    pub statuses: Vec<String>,
    pub gpu_hours: f64,
    pub share_percent: f64,
}

/// One bucket whose share of total GPU-hours exceeded the threshold.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThresholdFlag {
    pub status: String,
    pub share_percent: f64,
}

/// One issue's cumulative cost beside its most recent outcome.
///
/// `gpu_hours` is the sum over every costed run the issue has — a reporting
/// metric for where GPU went. It is deliberately shown next to
/// [`latest_status`], the terminal status of the issue's most recent run: a
/// high cumulative total says the issue failed before, not that it is failing
/// now, so the two are always read together (issue #3983).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IssueCost {
    pub issue: String,
    pub runs: u64,
    /// Cumulative GPU-hours across all of the issue's costed runs.
    pub gpu_hours: f64,
    /// Terminal status of the issue's most recent run, when recorded.
    pub latest_status: Option<String>,
}

/// The numbers for one scope (the window or the full history).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CostSummary {
    /// Costed records in scope.
    pub records: u64,
    pub total_gpu_hours: f64,
    /// Hours from runs whose disposition is productive.
    pub productive_gpu_hours: f64,
    /// Hours from runs later discarded, held, or superseded.
    pub rework_gpu_hours: f64,
    /// Buckets by terminal status, largest first.
    pub by_status: Vec<StatusBucket>,
    /// Buckets by disposition, fixed order.
    pub by_disposition: Vec<DispositionBucket>,
    /// Hours mapped onto known defect issues.
    pub defects: Vec<DefectCost>,
    /// Buckets whose share exceeded the threshold.
    pub flags: Vec<ThresholdFlag>,
    /// Cumulative cost per issue beside its most recent outcome, largest
    /// first. A reporting metric only — never an input to dispatch.
    pub by_issue: Vec<IssueCost>,
}

/// The full report: cumulative always, window only when `--since` was given.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CostReport {
    pub schema_version: u64,
    pub out_dir: String,
    /// The share threshold in percent that triggers a flag.
    pub threshold_percent: f64,
    /// The `--since` value as given, when the window scope is present.
    pub since: Option<String>,
    /// Run directories whose `status.txt` parsed, costed or not.
    pub total_records: u64,
    /// Parsed records missing a status or agent_secs, by run name.
    pub incomplete: Vec<String>,
    /// Run directories with no `status.txt`.
    pub no_record: Vec<String>,
    /// Run directories whose `status.txt` failed to parse.
    pub malformed: Vec<MalformedRecord>,
    pub cumulative: CostSummary,
    /// Present only when a window was requested.
    pub window: Option<CostSummary>,
}

/// Build the report for a scan: cumulative over every costed record, and the
/// window over the records whose finish (or start) stamp is at or after
/// `since` when one was given.
pub fn build_report(
    out_dir: &str,
    total_records: u64,
    records: &[RunRecord],
    no_record: &[String],
    malformed: &[MalformedRecord],
    since: Option<(i64, String)>,
    threshold_percent: f64,
) -> CostReport {
    let windowed: Vec<RunRecord> = match since {
        Some((stamp, _)) => records
            .iter()
            .filter(|record| record.window_stamp().is_some_and(|at| at >= stamp))
            .cloned()
            .collect(),
        None => Vec::new(),
    };
    let incomplete = records
        .iter()
        .filter(|record| !record.is_costed())
        .map(|record| record.issue.clone())
        .collect();
    let since_label = since.as_ref().map(|(_, label)| label.clone());
    CostReport {
        schema_version: 1,
        out_dir: out_dir.to_string(),
        threshold_percent,
        since: since_label,
        total_records,
        incomplete,
        no_record: no_record.to_vec(),
        malformed: malformed.to_vec(),
        cumulative: build_summary(records, threshold_percent),
        window: since.map(|_| build_summary(&windowed, threshold_percent)),
    }
}

fn build_summary(records: &[RunRecord], threshold_percent: f64) -> CostSummary {
    let mut status_hours: BTreeMap<String, (u64, f64)> = BTreeMap::new();
    let mut disposition_hours: BTreeMap<Disposition, (u64, f64)> = BTreeMap::new();
    let mut total_hours = 0.0_f64;
    let mut records_count = 0_u64;
    for record in records.iter().filter(|record| record.is_costed()) {
        records_count += 1;
        let hours = record.gpu_hours().unwrap_or(0.0);
        total_hours += hours;
        let status = record.status.as_deref().unwrap_or_default();
        let bucket = status_hours.entry(status.to_string()).or_default();
        bucket.0 += 1;
        bucket.1 += hours;
        let bucket = disposition_hours.entry(record.disposition).or_default();
        bucket.0 += 1;
        bucket.1 += hours;
    }
    let by_status = finish_status_buckets(status_hours, total_hours, threshold_percent);
    let by_disposition = finish_disposition_buckets(disposition_hours);
    let by_issue = issue_costs(records);
    let productive = by_disposition
        .iter()
        .find(|bucket| bucket.disposition == Disposition::Productive.as_str())
        .map(|bucket| bucket.gpu_hours)
        .unwrap_or(0.0);
    let defects = defect_costs(&by_status, total_hours);
    let flags = flags_from_buckets(&by_status);
    CostSummary {
        records: records_count,
        total_gpu_hours: round2(total_hours),
        productive_gpu_hours: productive,
        rework_gpu_hours: round2(total_hours - productive),
        by_status,
        by_disposition,
        defects,
        flags,
        by_issue,
    }
}

/// Group costed records by issue, summing their GPU-hours (the cumulative
/// total) and recording the most recent run's terminal status beside it.
///
/// "Most recent" is the record with the greatest finish (or start) stamp; a
/// stamped run always wins over an untimestamped one, and between two
/// untimestamped records the later one in input order wins. Deterministic for
/// a given input order, matching the report's other fixed-order guarantees.
fn issue_costs(records: &[RunRecord]) -> Vec<IssueCost> {
    struct Acc {
        runs: u64,
        hours: f64,
        latest_stamp: Option<i64>,
        latest_status: Option<String>,
    }
    let mut by_issue: BTreeMap<String, Acc> = BTreeMap::new();
    for record in records.iter().filter(|record| record.is_costed()) {
        let acc = by_issue.entry(record.issue.clone()).or_insert(Acc {
            runs: 0,
            hours: 0.0,
            latest_stamp: None,
            latest_status: None,
        });
        acc.runs += 1;
        acc.hours += record.gpu_hours().unwrap_or(0.0);
        let newer = match (acc.latest_stamp, record.window_stamp()) {
            (Some(best), Some(next)) => next >= best,
            (Some(_), None) => false,
            (None, Some(_)) => true,
            (None, None) => true,
        };
        if newer {
            acc.latest_stamp = record.window_stamp();
            acc.latest_status = record.status.clone();
        }
    }
    let mut out: Vec<IssueCost> = by_issue
        .into_iter()
        .map(|(issue, acc)| IssueCost {
            issue,
            runs: acc.runs,
            gpu_hours: round2(acc.hours),
            latest_status: acc.latest_status,
        })
        .collect();
    out.sort_by(|a, b| {
        b.gpu_hours
            .total_cmp(&a.gpu_hours)
            .then_with(|| a.issue.cmp(&b.issue))
    });
    out
}

fn finish_status_buckets(
    buckets: BTreeMap<String, (u64, f64)>,
    total_hours: f64,
    threshold_percent: f64,
) -> Vec<StatusBucket> {
    let mut out: Vec<StatusBucket> = buckets
        .into_iter()
        .map(|(status, (runs, hours))| {
            let share = share_percent(hours, total_hours);
            StatusBucket {
                status,
                runs,
                gpu_hours: round2(hours),
                share_percent: round2(share),
                // Compare the unrounded share against the threshold: a bucket
                // at 10.004% with a 10% threshold is over the line.
                flagged: share > threshold_percent,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.gpu_hours
            .total_cmp(&a.gpu_hours)
            .then_with(|| a.status.cmp(&b.status))
    });
    out
}

fn finish_disposition_buckets(
    buckets: BTreeMap<Disposition, (u64, f64)>,
) -> Vec<DispositionBucket> {
    const ORDER: [Disposition; 4] = [
        Disposition::Productive,
        Disposition::Discarded,
        Disposition::Held,
        Disposition::Superseded,
    ];
    ORDER
        .into_iter()
        .map(|disposition| {
            let (runs, hours) = buckets.get(&disposition).copied().unwrap_or((0, 0.0));
            DispositionBucket {
                disposition: disposition.as_str().to_string(),
                runs,
                gpu_hours: round2(hours),
            }
        })
        .collect()
}

fn defect_costs(by_status: &[StatusBucket], total_hours: f64) -> Vec<DefectCost> {
    let mut issues: Vec<(String, Vec<String>, f64)> = Vec::new();
    for (status, issue) in DEFAULT_DEFECT_MAP {
        let hours = by_status
            .iter()
            .find(|bucket| &bucket.status == status)
            .map(|bucket| bucket.gpu_hours)
            .unwrap_or(0.0);
        if hours <= 0.0 {
            continue;
        }
        match issues.iter_mut().find(|(name, _, _)| name == *issue) {
            Some((_, statuses, total)) => {
                statuses.push(status.to_string());
                *total += hours;
            }
            None => issues.push((issue.to_string(), vec![status.to_string()], hours)),
        }
    }
    issues
        .into_iter()
        .map(|(issue, statuses, hours)| DefectCost {
            issue,
            statuses,
            gpu_hours: round2(hours),
            share_percent: round2(share_percent(hours, total_hours)),
        })
        .collect()
}

fn flags_from_buckets(by_status: &[StatusBucket]) -> Vec<ThresholdFlag> {
    by_status
        .iter()
        .filter(|bucket| bucket.flagged)
        .map(|bucket| ThresholdFlag {
            status: bucket.status.clone(),
            share_percent: bucket.share_percent,
        })
        .collect()
}

fn share_percent(hours: f64, total_hours: f64) -> f64 {
    if total_hours > 0.0 {
        hours / total_hours * 100.0
    } else {
        0.0
    }
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

impl CostReport {
    /// The report as the `--json` envelope payload.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("CostReport is fully serializable")
    }

    /// The report as the operator-facing text rendering.
    pub fn to_text(&self) -> String {
        if self.cumulative.records == 0 {
            return format!(
                "no costed run records under {} — nothing to cost{}",
                self.out_dir,
                self.evidence_note()
            );
        }
        let mut out = String::new();
        out.push_str(&self.headline());
        out.push_str(&self.evidence_note());
        out.push_str(&render_summary(
            "cumulative (observed)",
            &self.cumulative,
            self.threshold_percent,
        ));
        if let Some(window) = &self.window {
            out.push('\n');
            out.push_str(&format!(
                "window (since {}): {} observed costed run(s), {} observed GPU-hours\n",
                self.since.as_deref().unwrap_or_default(),
                window.records,
                fmt_hours(window.total_gpu_hours)
            ));
            out.push_str(&render_sections(window, self.threshold_percent));
        }
        out
    }
}

impl CostReport {
    fn headline(&self) -> String {
        format!(
            "GPU cost report — {}: {} observed costed run(s), {} observed GPU-hours cumulative\n",
            self.out_dir,
            self.cumulative.records,
            fmt_hours(self.cumulative.total_gpu_hours)
        )
    }

    fn evidence_note(&self) -> String {
        let incomplete = self.incomplete.len();
        let no_record = self.no_record.len();
        let malformed = self.malformed.len();
        if incomplete == 0 && no_record == 0 && malformed == 0 {
            return String::new();
        }
        let mut note = format!(
            "  queue exposure (potential, not observed GPU-hours): {incomplete} incomplete, {no_record} with no status.txt, {malformed} malformed\n"
        );
        note.push_str(
            "    potential: realised only if those runs finish and record a terminal status plus agent_secs\n",
        );
        if !self.incomplete.is_empty() {
            note.push_str(&format!("    incomplete: {}\n", self.incomplete.join(", ")));
        }
        for record in &self.malformed {
            note.push_str(&format!(
                "    malformed {}: {}\n",
                record.issue, record.reason
            ));
        }
        if no_record > 0 {
            note.push_str(&format!(
                "    no status.txt: {}\n",
                self.no_record.join(", ")
            ));
        }
        note
    }
}

fn render_summary(label: &str, summary: &CostSummary, threshold_percent: f64) -> String {
    format!("{label}\n{}", render_sections(summary, threshold_percent))
}

fn render_sections(summary: &CostSummary, threshold_percent: f64) -> String {
    let mut out = String::new();
    out.push_str(&status_table(&summary.by_status));
    out.push_str(&disposition_line(summary));
    out.push_str(&defect_lines(summary));
    out.push_str(&issue_table(&summary.by_issue));
    out.push_str(&flag_line(summary, threshold_percent));
    out
}

fn status_table(by_status: &[StatusBucket]) -> String {
    if by_status.is_empty() {
        return String::new();
    }
    let width = by_status
        .iter()
        .map(|bucket| bucket.status.len())
        .max()
        .unwrap_or(0);
    let mut out = format!(
        "  {:width$}   runs  GPU-hours   share\n",
        "status",
        width = width
    );
    for bucket in by_status {
        let marker = if bucket.flagged { "*" } else { " " };
        out.push_str(&format!(
            "  {:width$} {marker} {:>4}   {:>9.1}  {:>6.1}%\n",
            bucket.status,
            bucket.runs,
            bucket.gpu_hours,
            bucket.share_percent,
            width = width
        ));
    }
    out
}

fn disposition_line(summary: &CostSummary) -> String {
    let rework_detail = summary
        .by_disposition
        .iter()
        .filter(|bucket| bucket.disposition != Disposition::Productive.as_str())
        .filter(|bucket| bucket.gpu_hours > 0.0)
        .map(|bucket| format!("{} {}", bucket.disposition, fmt_hours(bucket.gpu_hours)))
        .collect::<Vec<_>>()
        .join(", ");
    let rework = if rework_detail.is_empty() {
        String::from("0.0")
    } else {
        rework_detail
    };
    format!(
        "  productive {} GPU-hours | rework {} GPU-hours ({rework})\n",
        fmt_hours(summary.productive_gpu_hours),
        fmt_hours(summary.rework_gpu_hours)
    )
}

fn issue_table(by_issue: &[IssueCost]) -> String {
    if by_issue.is_empty() {
        return String::new();
    }
    let width = by_issue
        .iter()
        .map(|issue| issue.issue.len())
        .max()
        .unwrap_or(0);
    let mut out = format!(
        "  issues (ranked by cumulative hours)\n    {:width$}   runs  GPU-hours  latest\n",
        "issue",
        width = width
    );
    for issue in by_issue {
        let latest = issue
            .latest_status
            .as_deref()
            .map(str::to_string)
            .unwrap_or_else(|| "-".to_string());
        out.push_str(&format!(
            "    {:width$}   {:>4}   {:>9.1}  {latest}\n",
            issue.issue,
            issue.runs,
            issue.gpu_hours,
            width = width
        ));
    }
    out
}

fn defect_lines(summary: &CostSummary) -> String {
    if summary.defects.is_empty() {
        return String::new();
    }
    let mut out = String::from("  defects:\n");
    for defect in &summary.defects {
        out.push_str(&format!(
            "    {}  {:36.36}  {:>7.1} GPU-hours  {:>5.1}%\n",
            defect.issue,
            defect.statuses.join(", "),
            defect.gpu_hours,
            defect.share_percent
        ));
    }
    out
}

fn flag_line(summary: &CostSummary, threshold_percent: f64) -> String {
    if summary.flags.is_empty() {
        return format!("  flags: none (share threshold {threshold_percent:.1}%)\n");
    }
    let listed = summary
        .flags
        .iter()
        .map(|flag| format!("{} {:.1}%", flag.status, flag.share_percent))
        .collect::<Vec<_>>()
        .join(", ");
    format!("  flags (share over threshold): {listed}\n")
}

fn fmt_hours(hours: f64) -> String {
    format!("{hours:.1}")
}
