//! Verify a key discriminates before ordering by it (#3791).
//!
//! A sort, filter or freshness key derived from a copied artifact
//! describes the copy, not the thing. `cp` and `scp` set the
//! destination's mtime to the time of copy, so every patch from one
//! pull appears to have been written in the same second: 248 of 259
//! local patch files shared the identical mtime, a sort keyed on it
//! stayed stable and kept the input glob order, and the change it was
//! meant to make was invisible — the log line still read "success".
//!
//! Two primitives, one per invariant:
//!
//! 1. **Carry freshness and provenance as data.** Where an artifact is
//!    copied, its source timestamps travel with it explicitly
//!    ([`CopiedArtifact`]); nothing is inferred from filesystem
//!    metadata that transport rewrites.
//! 2. **Check the key before ordering.** A key used to order or select
//!    is checked for discrimination before use
//!    ([`key_discrimination`], [`KeyDiscrimination::is_degenerate`]):
//!    fewer distinct values than a stated fraction of the items is a
//!    defect to report, not silently accept.
//!
//! The ordering step ([`order_by_source_freshness`]) reports the
//! key's distinct-value count and the head of the resulting list, so
//! the ordering is falsifiable: with a degenerate key the head is the
//! input head, and the reader can see the sort did nothing.

use std::collections::HashSet;
use std::time::SystemTime;

/// Default minimum discrimination bar: a key is degenerate when it
/// takes fewer distinct values than `1 / MIN_DISTINCT_DENOMINATOR` of
/// the items (one distinct value per twenty items). 248 of 259 patches
/// sharing one copy-time mtime is ~1 in 259 — orders of magnitude
/// below this bar.
pub const MIN_DISTINCT_DENOMINATOR: u32 = 20;

/// A copied artifact with its freshness and provenance carried as data.
///
/// The destination's mtime is set by the copy (`cp`, `scp`) to the
/// copy time, so every artifact in one pull carries it identically and
/// it discriminates nothing. The source mtime is what an ordering
/// needs, and it exists only because it was recorded at the source and
/// shipped explicitly — never inferred from metadata on the copy side
/// that transport rewrites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopiedArtifact<T> {
    /// The artifact itself (a patch identity, a blob, ...).
    pub payload: T,
    /// The artifact's mtime at its source, recorded before the copy.
    pub source_mtime: SystemTime,
    /// Where the artifact came from (origin path, the pull that copied
    /// it, ...). Provenance is data, not inference.
    pub source: String,
}

impl<T> CopiedArtifact<T> {
    /// Construct an artifact with its source freshness and provenance.
    pub fn new(payload: T, source_mtime: SystemTime, source: impl Into<String>) -> Self {
        Self {
            payload,
            source_mtime,
            source: source.into(),
        }
    }
}

/// The result of checking a key for discrimination before use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyDiscrimination {
    /// Distinct values the key takes across the items.
    pub distinct: usize,
    /// Number of items the key was computed over.
    pub total: usize,
}

impl KeyDiscrimination {
    /// A key is degenerate when it takes fewer distinct values than
    /// `1 / denominator` of the items — the caller-stated bar (see
    /// [`MIN_DISTINCT_DENOMINATOR`]). An empty input is vacuously fine:
    /// there is no order to falsify.
    pub fn is_degenerate(self, denominator: u32) -> bool {
        let denominator = denominator.max(1) as u64;
        let (distinct, total) = (self.distinct as u64, self.total as u64);
        distinct < total.div_ceil(denominator)
    }
}

/// Check a key for discrimination before anything is ordered by it:
/// count the distinct values it takes across the items.
pub fn key_discrimination<K: Eq + std::hash::Hash>(
    key: impl IntoIterator<Item = K>,
) -> KeyDiscrimination {
    let mut distinct = HashSet::new();
    let mut total = 0usize;
    for value in key {
        distinct.insert(value);
        total += 1;
    }
    KeyDiscrimination {
        distinct: distinct.len(),
        total,
    }
}

/// The result of ordering copied artifacts by their carried freshness.
///
/// Carries exactly the fields a reader needs to falsify the order: the
/// key's distinct-value count, the total, and the head of the resulting
/// list. `key_degenerate` flags the defect that made the #3783 bug
/// invisible: the order is well-formed but inert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderReport<T> {
    /// Artifacts, newest source mtime first; ties keep input order.
    pub ordered: Vec<CopiedArtifact<T>>,
    /// Distinct values of the freshness key across the input.
    pub key_distinct: usize,
    /// Number of artifacts ordered.
    pub key_total: usize,
    /// The payload at the head of the resulting list — the falsifiable
    /// claim ("X leads N candidates"). With a degenerate key this is
    /// the input head, and the reader can see the sort did nothing.
    pub head: Option<T>,
    /// True when the key took fewer distinct values than the stated
    /// fraction of the items: the ordering is inert, and this report is
    /// a defect, not a success.
    pub key_degenerate: bool,
}

/// Order copied artifacts by the freshness carried with them (source
/// mtime, newest first), check the key for discrimination, and report
/// the key's distinct-value count and the head of the result.
///
/// Stable: ties keep input order. `min_distinct_denominator` is the
/// caller-stated bar (see [`MIN_DISTINCT_DENOMINATOR`]); a key below it
/// does not abort the order — it is flagged in the report, because the
/// defect to surface is "the key ordered nothing", and a silent success
/// is what made it invisible in the first place.
pub fn order_by_source_freshness<T: Clone>(
    artifacts: &[CopiedArtifact<T>],
    min_distinct_denominator: u32,
) -> OrderReport<T> {
    let key = key_discrimination(artifacts.iter().map(|artifact| artifact.source_mtime));
    let mut ordered = artifacts.to_vec();
    ordered.sort_by(|a, b| b.source_mtime.cmp(&a.source_mtime));
    OrderReport {
        head: ordered.first().map(|artifact| artifact.payload.clone()),
        key_degenerate: key.is_degenerate(min_distinct_denominator),
        key_distinct: key.distinct,
        key_total: key.total,
        ordered,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn artifact(id: &str, source_mtime: SystemTime) -> CopiedArtifact<String> {
        CopiedArtifact::new(id.to_string(), source_mtime, format!("pull-of-{id}"))
    }

    /// The #3783 key: 248 of the copies share the pull time, the
    /// remaining 11 carry distinct source times (12 distinct values).
    fn copy_or_source_time(i: u64, pull_time: SystemTime) -> SystemTime {
        if i < 248 {
            pull_time
        } else {
            at(1_010_000 + 10_000 * (i - 248))
        }
    }

    /// Regression (#3791): 259 patches copied in one operation, so every
    /// copy shares the pull time. Order by the freshness carried with them
    /// (source mtimes spread across 85 hours) and the intended order
    /// comes out, with the key verified, not inferred.
    #[test]
    fn copied_artifacts_order_by_carried_source_freshness() {
        let hours = 85 * 3600;
        // Input in arbitrary (oldest-first) glob order.
        let artifacts: Vec<_> = (0..259u64)
            .map(|i| artifact(&format!("autospec-{i}.patch"), at(i * hours / 258)))
            .collect();

        let report = order_by_source_freshness(&artifacts, MIN_DISTINCT_DENOMINATOR);

        assert_eq!(report.key_total, 259);
        assert_eq!(report.key_distinct, 259);
        assert!(!report.key_degenerate);
        // Newest source time leads, not the glob head.
        assert_eq!(report.head.as_deref(), Some("autospec-258.patch"));
        assert!(report
            .ordered
            .windows(2)
            .all(|w| w[0].source_mtime >= w[1].source_mtime));
    }

    /// The #3783 case: freshness taken from the copy. 248 of 259 copies
    /// share the pull time, so the key has 12 distinct values for 259
    /// items — below the 1-in-20 bar. The report must say the key is
    /// degenerate instead of silently keeping the input order.
    #[test]
    fn copy_time_key_is_reported_degenerate_not_silently_accepted() {
        let pull_time = at(1_000_000);
        let mut artifacts = Vec::new();
        for i in 0..259u64 {
            let id = format!("autospec-{i}.patch");
            artifacts.push(artifact(&id, copy_or_source_time(i, pull_time)));
        }

        let report = order_by_source_freshness(&artifacts, MIN_DISTINCT_DENOMINATOR);

        assert_eq!(report.key_total, 259);
        assert_eq!(report.key_distinct, 12);
        assert!(report.key_degenerate);
        // The 248 tied copies keep their input order: the sort is inert
        // within the tie, and the report says so.
        let tied: Vec<_> = report
            .ordered
            .iter()
            .filter(|a| a.source_mtime == pull_time)
            .map(|a| a.payload.clone())
            .collect();
        let expected_tied: Vec<_> = (0..248).map(|i| format!("autospec-{i}.patch")).collect();
        assert_eq!(tied, expected_tied);
    }

    #[test]
    fn degenerate_threshold_is_caller_stated() {
        let key = key_discrimination([1, 1, 1, 1, 2]);
        assert_eq!((key.distinct, key.total), (2, 5));
        // 2 < ceil(5/1) = 5: degenerate at a 1-in-1 bar.
        assert!(key.is_degenerate(1));
        // 2 >= ceil(5/5) = 1: fine at a 1-in-5 bar.
        assert!(!key.is_degenerate(5));
    }

    #[test]
    fn empty_input_is_not_degenerate() {
        let key = key_discrimination(std::iter::empty::<u8>());
        assert_eq!((key.distinct, key.total), (0, 0));
        assert!(!key.is_degenerate(MIN_DISTINCT_DENOMINATOR));
    }
}
