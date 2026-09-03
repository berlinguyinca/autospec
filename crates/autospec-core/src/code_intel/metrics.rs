use std::collections::BTreeMap;

use serde::Serialize;

use super::schema::{Operation, ResultSource};

/// One completed gateway operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationRecord {
    pub workspace: String,
    pub language: String,
    pub operation: Operation,
    pub source: ResultSource,
    pub latency_ms: u64,
    pub cache_hit: bool,
    pub failed: bool,
}

impl OperationRecord {
    pub fn new(
        workspace: impl Into<String>,
        language: impl Into<String>,
        operation: Operation,
        source: ResultSource,
        latency_ms: u64,
    ) -> Self {
        Self {
            workspace: workspace.into(),
            language: language.into(),
            operation,
            source,
            latency_ms,
            cache_hit: false,
            failed: false,
        }
    }

    pub fn cached(mut self) -> Self {
        self.cache_hit = true;
        self
    }

    pub fn failed(mut self) -> Self {
        self.failed = true;
        self
    }

    fn is_fallback(&self) -> bool {
        self.source != ResultSource::Lsp
    }
}

/// Latency and health counters for one workspace/language pair.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct MetricsSnapshot {
    pub workspace: String,
    pub language: String,
    pub operations: usize,
    pub failures: usize,
    pub cache_hits: usize,
    pub fallbacks: usize,
    pub latency_p50_ms: u64,
    pub latency_p95_ms: u64,
    pub latency_p99_ms: u64,
}

impl MetricsSnapshot {
    /// Share of operations that had to degrade below the semantic tier.
    pub fn fallback_rate(&self) -> f64 {
        ratio(self.fallbacks, self.operations)
    }

    pub fn cache_hit_rate(&self) -> f64 {
        ratio(self.cache_hits, self.operations)
    }

    pub fn failure_rate(&self) -> f64 {
        ratio(self.failures, self.operations)
    }
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        return 0.0;
    }
    numerator as f64 / denominator as f64
}

/// Accumulates operation records into per-workspace/language snapshots.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MetricsRegistry {
    records: Vec<OperationRecord>,
}

impl MetricsRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, record: OperationRecord) {
        self.records.push(record);
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// One snapshot per workspace/language pair, ordered deterministically so
    /// the dashboard renders stably between refreshes.
    pub fn snapshots(&self) -> Vec<MetricsSnapshot> {
        let mut grouped: BTreeMap<(String, String), Vec<&OperationRecord>> = BTreeMap::new();
        for record in &self.records {
            grouped
                .entry((record.workspace.clone(), record.language.clone()))
                .or_default()
                .push(record);
        }
        grouped
            .into_iter()
            .map(|((workspace, language), records)| snapshot(workspace, language, &records))
            .collect()
    }

    pub fn to_json_string(&self) -> Result<String, String> {
        serde_json::to_string(&self.snapshots()).map_err(|error| error.to_string())
    }
}

fn snapshot(workspace: String, language: String, records: &[&OperationRecord]) -> MetricsSnapshot {
    let mut latencies: Vec<u64> = records.iter().map(|record| record.latency_ms).collect();
    latencies.sort_unstable();
    MetricsSnapshot {
        workspace,
        language,
        operations: records.len(),
        failures: records.iter().filter(|record| record.failed).count(),
        cache_hits: records.iter().filter(|record| record.cache_hit).count(),
        fallbacks: records.iter().filter(|record| record.is_fallback()).count(),
        latency_p50_ms: percentile(&latencies, 50),
        latency_p95_ms: percentile(&latencies, 95),
        latency_p99_ms: percentile(&latencies, 99),
    }
}

/// Nearest-rank percentile over a sorted slice.
///
/// Nearest-rank (rather than interpolation) keeps every reported value an
/// observed latency, which is what an operator wants when chasing one slow
/// server.
fn percentile(sorted: &[u64], percentile: u64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (percentile as usize * sorted.len()).div_ceil(100);
    let index = rank.max(1) - 1;
    sorted[index.min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(workspace: &str, latency_ms: u64) -> OperationRecord {
        OperationRecord::new(
            workspace,
            "rust",
            Operation::References,
            ResultSource::Lsp,
            latency_ms,
        )
    }

    #[test]
    fn an_empty_registry_reports_no_snapshots() {
        let registry = MetricsRegistry::new();

        assert!(registry.is_empty());
        assert!(registry.snapshots().is_empty());
    }

    #[test]
    fn records_group_by_workspace_and_language() {
        let mut registry = MetricsRegistry::new();
        registry.record(record("issue-421", 10));
        registry.record(record("issue-422", 20));
        registry.record(OperationRecord::new(
            "issue-421",
            "python",
            Operation::Definition,
            ResultSource::Lsp,
            30,
        ));

        let snapshots = registry.snapshots();

        assert_eq!(snapshots.len(), 3);
        assert_eq!(snapshots[0].workspace, "issue-421");
        assert_eq!(snapshots[0].language, "python");
        assert_eq!(snapshots[1].language, "rust");
    }

    #[test]
    fn percentiles_use_observed_latencies() {
        let mut registry = MetricsRegistry::new();
        for latency in [10, 20, 30, 40, 50, 60, 70, 80, 90, 100] {
            registry.record(record("issue-421", latency));
        }

        let snapshot = &registry.snapshots()[0];

        assert_eq!(snapshot.latency_p50_ms, 50);
        assert_eq!(snapshot.latency_p95_ms, 100);
        assert_eq!(snapshot.latency_p99_ms, 100);
    }

    #[test]
    fn a_single_record_reports_itself_at_every_percentile() {
        let mut registry = MetricsRegistry::new();
        registry.record(record("issue-421", 42));

        let snapshot = &registry.snapshots()[0];

        assert_eq!(snapshot.latency_p50_ms, 42);
        assert_eq!(snapshot.latency_p99_ms, 42);
    }

    #[test]
    fn fallback_and_cache_rates_are_reported() {
        let mut registry = MetricsRegistry::new();
        registry.record(record("issue-421", 10).cached());
        registry.record(OperationRecord::new(
            "issue-421",
            "rust",
            Operation::References,
            ResultSource::Ripgrep,
            5,
        ));
        registry.record(record("issue-421", 15).failed());
        registry.record(record("issue-421", 20));

        let snapshot = &registry.snapshots()[0];

        assert_eq!(snapshot.operations, 4);
        assert_eq!(snapshot.fallback_rate(), 0.25);
        assert_eq!(snapshot.cache_hit_rate(), 0.25);
        assert_eq!(snapshot.failure_rate(), 0.25);
    }

    #[test]
    fn rates_are_zero_when_nothing_was_recorded() {
        let snapshot = MetricsSnapshot::default();

        assert_eq!(snapshot.fallback_rate(), 0.0);
        assert_eq!(snapshot.cache_hit_rate(), 0.0);
    }

    #[test]
    fn semantic_results_are_never_counted_as_fallbacks() {
        let mut registry = MetricsRegistry::new();
        registry.record(record("issue-421", 10));

        assert_eq!(registry.snapshots()[0].fallbacks, 0);
    }

    #[test]
    fn snapshots_serialize_for_the_dashboard() {
        let mut registry = MetricsRegistry::new();
        registry.record(record("issue-421", 10));

        let json = registry.to_json_string().unwrap();

        assert!(json.contains("\"workspace\":\"issue-421\""));
        assert!(json.contains("\"latency_p95_ms\":10"));
    }
}
