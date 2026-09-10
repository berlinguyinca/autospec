//! Insights observability (spec section 49).
//!
//! The eleven `autospec_insights_*` counters and gauges the Continuous
//! Improvement Engine exposes, behind a typed registry that other insights
//! modules increment. The registry is allocation-free and shared: modules
//! hold an `Arc<InsightsMetrics>` and increment concurrently.
//!
//! `LogFields` carries the structured log identifiers section 49 requires.
//! It holds ids and opaque labels only — never prompt text — so a log line
//! can never leak session content.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};

/// The eleven section 49 metrics, addressed by variant rather than by string
/// so a renamed metric is a compile error at every call site.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Metric {
    /// Normalized sessions ingested.
    SessionsIngested,
    /// Normalized events ingested.
    EventsIngested,
    /// Ingestion failures (corrupted or quarantined sessions).
    IngestionErrors,
    /// Enrichment jobs queued or completed on InferWeave nodes.
    EnrichmentJobs,
    /// Enrichment jobs that failed.
    EnrichmentFailures,
    /// Findings currently active (gauge).
    FindingsActive,
    /// Improvement proposals currently active (gauge).
    ProposalsActive,
    /// Proposals that passed validation.
    ProposalsValidated,
    /// Proposals that regressed during evaluation.
    ProposalsRegressed,
    /// Current analysis queue depth (gauge).
    AnalysisQueueDepth,
    /// GPU-seconds spent on analysis (cumulative whole seconds).
    AnalysisGpuSeconds,
}

impl Metric {
    /// The full section 49 name, e.g. `autospec_insights_sessions_ingested_total`.
    pub const fn name(self) -> &'static str {
        match self {
            Metric::SessionsIngested => "autospec_insights_sessions_ingested_total",
            Metric::EventsIngested => "autospec_insights_events_ingested_total",
            Metric::IngestionErrors => "autospec_insights_ingestion_errors_total",
            Metric::EnrichmentJobs => "autospec_insights_enrichment_jobs_total",
            Metric::EnrichmentFailures => "autospec_insights_enrichment_failures_total",
            Metric::FindingsActive => "autospec_insights_findings_active",
            Metric::ProposalsActive => "autospec_insights_proposals_active",
            Metric::ProposalsValidated => "autospec_insights_proposals_validated_total",
            Metric::ProposalsRegressed => "autospec_insights_proposals_regressed_total",
            Metric::AnalysisQueueDepth => "autospec_insights_analysis_queue_depth",
            Metric::AnalysisGpuSeconds => "autospec_insights_analysis_gpu_seconds",
        }
    }
}

/// The section 49 registry: eleven atomic counters shared across the engine.
///
/// Counters are monotonically increasing totals; the `*_active`,
/// `analysis_queue_depth` and `analysis_gpu_seconds` entries are gauges set or
/// accumulated by whichever stage owns them. Label cardinality is bounded
/// because the registry itself carries no per-session or per-repo labels —
/// those live in `LogFields` on the structured log record.
#[derive(Default)]
pub struct InsightsMetrics {
    sessions_ingested_total: AtomicI64,
    events_ingested_total: AtomicI64,
    ingestion_errors_total: AtomicI64,
    enrichment_jobs_total: AtomicI64,
    enrichment_failures_total: AtomicI64,
    findings_active: AtomicI64,
    proposals_active: AtomicI64,
    proposals_validated_total: AtomicI64,
    proposals_regressed_total: AtomicI64,
    analysis_queue_depth: AtomicI64,
    analysis_gpu_seconds: AtomicI64,
}

impl InsightsMetrics {
    /// A fresh, all-zero registry.
    pub fn new() -> Self {
        Self::default()
    }

    fn counter(&self, metric: Metric) -> &AtomicI64 {
        match metric {
            Metric::SessionsIngested => &self.sessions_ingested_total,
            Metric::EventsIngested => &self.events_ingested_total,
            Metric::IngestionErrors => &self.ingestion_errors_total,
            Metric::EnrichmentJobs => &self.enrichment_jobs_total,
            Metric::EnrichmentFailures => &self.enrichment_failures_total,
            Metric::FindingsActive => &self.findings_active,
            Metric::ProposalsActive => &self.proposals_active,
            Metric::ProposalsValidated => &self.proposals_validated_total,
            Metric::ProposalsRegressed => &self.proposals_regressed_total,
            Metric::AnalysisQueueDepth => &self.analysis_queue_depth,
            Metric::AnalysisGpuSeconds => &self.analysis_gpu_seconds,
        }
    }

    /// Add `delta` to a metric.
    pub fn add(&self, metric: Metric, delta: i64) {
        self.counter(metric).fetch_add(delta, Ordering::Relaxed);
    }

    /// Increment a metric by one.
    pub fn increment(&self, metric: Metric) {
        self.add(metric, 1);
    }

    /// Set a gauge to an absolute value.
    pub fn set(&self, metric: Metric, value: i64) {
        self.counter(metric).store(value, Ordering::Relaxed);
    }
}

impl InsightsMetrics {
    /// Snapshot of all eleven metrics, keyed by the full section 49 name.
    pub fn snapshot(&self) -> BTreeMap<String, i64> {
        const ALL: [Metric; 11] = [
            Metric::SessionsIngested,
            Metric::EventsIngested,
            Metric::IngestionErrors,
            Metric::EnrichmentJobs,
            Metric::EnrichmentFailures,
            Metric::FindingsActive,
            Metric::ProposalsActive,
            Metric::ProposalsValidated,
            Metric::ProposalsRegressed,
            Metric::AnalysisQueueDepth,
            Metric::AnalysisGpuSeconds,
        ];
        ALL.iter()
            .map(|&m| {
                (
                    m.name().to_string(),
                    self.counter(m).load(Ordering::Relaxed),
                )
            })
            .collect()
    }

    /// Render the snapshot as Prometheus-style `name value` lines, sorted by
    /// name. For scrape endpoints and for tests.
    pub fn render(&self) -> String {
        self.snapshot()
            .iter()
            .map(|(name, value)| format!("{name} {value}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Structured log identifiers section 49 requires on insights log records.
///
/// Carries ids and opaque labels only. `repo` and `model` are emitted as
/// given by the caller — the engine never enriches them — so a caller that
/// only logs ids keeps private repository names out of the log.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogFields {
    /// Session the record pertains to.
    pub session_id: Option<String>,
    /// Finding the record pertains to.
    pub finding_id: Option<String>,
    /// Proposal the record pertains to.
    pub proposal_id: Option<String>,
    /// Repository identifier.
    pub repo: Option<String>,
    /// Model identifier.
    pub model: Option<String>,
    /// Work item the record pertains to.
    pub work_item_id: Option<String>,
}

impl LogFields {
    /// Build a [`LogFields`] from any subset of the six section 49
    /// identifiers; absent ones pass `None`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: Option<String>,
        finding_id: Option<String>,
        proposal_id: Option<String>,
        repo: Option<String>,
        model: Option<String>,
        work_item_id: Option<String>,
    ) -> Self {
        Self {
            session_id,
            finding_id,
            proposal_id,
            repo,
            model,
            work_item_id,
        }
    }

    /// The fields set on this record, as `(field_name, value)` pairs in fixed
    /// section 49 order. Unset or empty identifiers are omitted, so a log
    /// line never carries an empty-string id.
    pub fn fields(&self) -> Vec<(&'static str, &str)> {
        let values: [(&'static str, &Option<String>); 6] = [
            ("session_id", &self.session_id),
            ("finding_id", &self.finding_id),
            ("proposal_id", &self.proposal_id),
            ("repo", &self.repo),
            ("model", &self.model),
            ("work_item_id", &self.work_item_id),
        ];
        let mut out = Vec::new();
        for (name, value) in values {
            if let Some(v) = value {
                if !v.is_empty() {
                    out.push((name, v.as_str()));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const EXPECTED_NAMES: [&str; 11] = [
        "autospec_insights_sessions_ingested_total",
        "autospec_insights_events_ingested_total",
        "autospec_insights_ingestion_errors_total",
        "autospec_insights_enrichment_jobs_total",
        "autospec_insights_enrichment_failures_total",
        "autospec_insights_findings_active",
        "autospec_insights_proposals_active",
        "autospec_insights_proposals_validated_total",
        "autospec_insights_proposals_regressed_total",
        "autospec_insights_analysis_queue_depth",
        "autospec_insights_analysis_gpu_seconds",
    ];

    #[test]
    fn snapshot_keys_equal_the_eleven_section_49_names_exactly() {
        let metrics = InsightsMetrics::new();
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.len(), 11);
        let keys: Vec<&str> = snapshot.keys().map(String::as_str).collect();
        for name in EXPECTED_NAMES {
            assert!(keys.contains(&name), "snapshot is missing {name}: {keys:?}");
        }
        assert!(snapshot.keys().all(|k| k.starts_with("autospec_insights_")));
    }

    #[test]
    fn snapshot_starts_at_zero() {
        assert!(InsightsMetrics::new().snapshot().values().all(|&v| v == 0));
    }

    #[test]
    fn increment_set_and_add_move_the_right_counter() {
        let metrics = InsightsMetrics::new();
        metrics.increment(Metric::SessionsIngested);
        metrics.add(Metric::EventsIngested, 41);
        metrics.set(Metric::AnalysisQueueDepth, 7);
        metrics.increment(Metric::AnalysisGpuSeconds);

        let snap = metrics.snapshot();
        assert_eq!(snap["autospec_insights_sessions_ingested_total"], 1);
        assert_eq!(snap["autospec_insights_events_ingested_total"], 41);
        assert_eq!(snap["autospec_insights_analysis_queue_depth"], 7);
        assert_eq!(snap["autospec_insights_analysis_gpu_seconds"], 1);
        assert_eq!(snap["autospec_insights_findings_active"], 0);
    }

    #[test]
    fn render_emits_sorted_prometheus_lines() {
        let metrics = InsightsMetrics::new();
        metrics.increment(Metric::IngestionErrors);
        let rendered = metrics.render();
        let mut lines: Vec<&str> = rendered.lines().collect();
        let sorted = lines.clone();
        lines.sort_unstable();
        assert_eq!(lines, sorted);
        assert_eq!(lines.len(), 11);
        assert!(lines.contains(&"autospec_insights_ingestion_errors_total 1"));
        assert!(lines.contains(&"autospec_insights_proposals_active 0"));
    }

    #[test]
    fn concurrent_increments_from_eight_threads_sum_exactly() {
        let metrics = Arc::new(InsightsMetrics::new());
        const THREADS: u64 = 8;
        const PER_THREAD: u64 = 1000;
        let handles = (0..THREADS)
            .map(|_| {
                let metrics = Arc::clone(&metrics);
                std::thread::spawn(move || {
                    for _ in 0..PER_THREAD {
                        metrics.increment(Metric::EventsIngested);
                    }
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().expect("increment thread panicked");
        }
        assert_eq!(
            metrics.snapshot()["autospec_insights_events_ingested_total"],
            (THREADS * PER_THREAD) as i64
        );
    }

    #[test]
    fn log_fields_serialize_set_ids_only() {
        let fields = LogFields::new(
            Some("sess-1".into()),
            None,
            Some("prop-9".into()),
            None,
            None,
            None,
        );
        let pairs = fields.fields();
        assert_eq!(
            pairs,
            vec![("session_id", "sess-1"), ("proposal_id", "prop-9")]
        );
    }

    #[test]
    fn log_fields_omits_empty_strings() {
        let fields = LogFields::new(
            Some(String::new()),
            Some("find-2".into()),
            None,
            Some("repo".into()),
            Some("model-x".into()),
            Some("wi-3".into()),
        );
        let pairs = fields.fields();
        assert_eq!(
            pairs,
            vec![
                ("finding_id", "find-2"),
                ("repo", "repo"),
                ("model", "model-x"),
                ("work_item_id", "wi-3"),
            ]
        );
    }

    #[test]
    fn log_fields_default_has_no_fields() {
        assert!(LogFields::default().fields().is_empty());
    }

    #[test]
    fn log_fields_exposes_the_six_section_49_names() {
        let fields = LogFields::new(
            Some("s".into()),
            Some("f".into()),
            Some("p".into()),
            Some("r".into()),
            Some("m".into()),
            Some("w".into()),
        );
        let names: Vec<&str> = fields.fields().iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names,
            vec![
                "session_id",
                "finding_id",
                "proposal_id",
                "repo",
                "model",
                "work_item_id"
            ]
        );
    }
}
