//! A priority was attached to an unmeasured number (issue #4561).
//!
//! #4560 claimed that one conflict shape was "the dominant conflict in
//! this backlog" and "the highest-yield single change available to
//! conversion throughput". Every individual number behind the claim was
//! real:
//!
//! - 5 of 15 conflict *region instances* were `additive_declarations`
//! - the top three contended *files* held 62 of 96 conflicts
//! - those files were append-only indexes
//!
//! None of them supported the claim. Region-instance share is not
//! per-patch yield — a patch is held if **any** region is unresolvable,
//! so a shape appearing in many patches alongside other shapes frees
//! none of them. File contention is not region kind. Both were
//! measurements of adjacent quantities, promoted to the quantity the
//! decision needed, on a 15-region sample, and written into a filed
//! issue **with a priority attached**.
//!
//! The correct measurement took one scan: apply each patch to a
//! scratch worktree, classify both sides of every conflict region, and
//! count patches whose regions are *all* resolvable. 266 regions, 57
//! patches, ~10 minutes. The result: the shape was worth **11%** of
//! conflict-held patches, and the dominant shape was something else
//! entirely (76% `code × code`, which no classifier can resolve).
//!
//! Invariants, each encoded as a checkable primitive:
//!
//! 1. **A quantity used to rank work must be the quantity that ranks
//!    it.** "How often does shape X appear" and "how many items would
//!    resolving X free" are different questions with different answers
//!    ([`Metric`], [`RankingTarget`], [`judge_claim`]).
//! 2. **Sample size is part of the claim.** A share without a
//!    denominator reads as measured and is not; a share below
//!    [`MIN_MEASURED_N`] is a hypothesis, and the claim line always
//!    carries n ([`Share`], [`ShareStatus`]).
//! 3. **Any-of blocks all-of.** Where an item is blocked if *any* part
//!    fails, per-part frequency overstates per-item yield, and the
//!    overstatement grows with the number of parts
//!    ([`Backlog::per_part_share`], [`Backlog::per_item_yield`],
//!    [`Backlog::yield_comparison`], [`any_of_gap`]).
//! 4. **A priority in an issue is a claim, and carries the same
//!    evidentiary burden as a technical assertion.** "Highest-yield"
//!    is falsifiable. Unmeasured, the issue says what is known and
//!    omits the ranking ([`RankingClaim::priority`], [`judge_claim`],
//!    [`unknown_ordering_line`]).
//! 5. **Prefer the cheap direct measurement to the clever indirect
//!    one.** A number reasoned from an adjacent count is not a
//!    measurement of the decision's quantity ([`EvidenceKind`]).

/// Smallest denominator at which a share reads as a measurement rather
/// than a hypothesis (invariant 2). The incident's 15-region sample is
/// on this side of the line; its 266-region / 57-patch scan is on the
/// other.
pub const MIN_MEASURED_N: usize = 30;

/// Whether a share's sample size lets it read as a measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareStatus {
    /// The denominator is below [`MIN_MEASURED_N`]: a hypothesis,
    /// which must be reported as one — never as a finding.
    Hypothesis,
    /// The denominator meets [`MIN_MEASURED_N`]: the share reads as
    /// measured.
    Measured,
}

impl ShareStatus {
    /// The label the claim line carries.
    pub fn label(self) -> &'static str {
        match self {
            ShareStatus::Hypothesis => "hypothesis",
            ShareStatus::Measured => "measured",
        }
    }
}

/// A count with its denominator — a share that cannot be read back
/// without its sample size (invariant 2). A share without a
/// denominator is not a share; it is an integer that reads as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Share {
    /// The numerator: the count of the unit being claimed.
    pub numerator: usize,
    /// The denominator: the population the numerator was counted in.
    pub denominator: usize,
}

impl Share {
    /// A well-formed share: a non-empty population, a numerator no
    /// larger than it.
    pub fn new(numerator: usize, denominator: usize) -> Option<Self> {
        if denominator == 0 || numerator > denominator {
            return None;
        }
        Some(Self {
            numerator,
            denominator,
        })
    }

    /// The share as a fraction in `[0.0, 1.0]`.
    pub fn fraction(self) -> f64 {
        self.numerator as f64 / self.denominator as f64
    }

    /// Whether the sample size lets the share read as a measurement
    /// (invariant 2).
    pub fn status(self) -> ShareStatus {
        if self.denominator >= MIN_MEASURED_N {
            ShareStatus::Measured
        } else {
            ShareStatus::Hypothesis
        }
    }

    /// The line for the claim. The denominator and its status are
    /// always rendered — `5 of 15 (33%, n=15, hypothesis)` — so the
    /// reader can never take the fraction without the sample.
    pub fn line(self) -> String {
        format!(
            "{} of {} ({}%, n={}, {})",
            self.numerator,
            self.denominator,
            (self.fraction() * 100.0).round() as u32,
            self.denominator,
            self.status().label()
        )
    }
}

/// The two quantities the incident confused (invariant 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    /// How often a shape appears among parts — "5 of 15 regions are
    /// additive". A real measurement, and not the quantity a yield
    /// decision ranks on.
    PerPartFrequency,
    /// How many items are freed if the shape is resolvable — the
    /// quantity that ranks a change by what it unblocks.
    PerItemYield,
}

impl Metric {
    /// The label the verdict line carries.
    pub fn label(self) -> &'static str {
        match self {
            Metric::PerPartFrequency => "per-part frequency",
            Metric::PerItemYield => "per-item yield",
        }
    }
}

/// How the number was obtained (invariant 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceKind {
    /// The item itself was measured — each patch applied to a scratch
    /// worktree, every conflict region classified.
    Direct,
    /// Reasoned from an adjacent count (file-contention share,
    /// region-instance share). Faster to produce, and the quantity is
    /// not the one the decision ranks on.
    Indirect,
}

/// What the ranking is for — the decision the number drives
/// (invariant 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RankingTarget {
    /// The ranking chooses which change to make, and its effect is
    /// measured in items freed. Only per-item yield ranks this.
    FreeItems,
    /// The ranking describes the population for a report and no work
    /// is being chosen from it; the metric choice is the reporter's.
    /// The sample-size rule still applies to anything that is filed
    /// with a priority.
    Describe,
}

/// One held item (a patch) and the shape of each of its parts (a
/// conflict region). An item is freed only when **every** part is
/// resolvable — any-of blocks all-of (invariant 3).
///
/// Constructed from the direct measurement: apply each patch to a
/// scratch worktree, classify both sides of every conflict region,
/// and record one shape per region.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Backlog {
    items: Vec<Vec<String>>,
}

impl Backlog {
    /// One entry per held item; one shape string per part of the item.
    /// An empty entry is an item with no parts: it is held by nothing
    /// and is never counted as freed.
    pub fn new(items: Vec<Vec<String>>) -> Self {
        Self { items }
    }

    /// The number of items (the denominator of per-item yield).
    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    /// The number of parts across all items (the denominator of
    /// per-part frequency).
    pub fn part_count(&self) -> usize {
        self.items.iter().map(|item| item.len()).sum()
    }

    /// The share of part *instances* carrying `shape` (per-part
    /// frequency). This is the number #4560 measured.
    pub fn per_part_share(&self, shape: &str) -> Option<Share> {
        let hits = self
            .items
            .iter()
            .flat_map(|item| item.iter())
            .filter(|part| part.as_str() == shape)
            .count();
        Share::new(hits, self.part_count())
    }

    /// The share of items whose parts are *all* in `resolvable`
    /// (per-item yield). This is the number the yield decision ranks
    /// on, and the one #4560 did not measure. `None` for an empty
    /// backlog.
    pub fn per_item_yield(&self, resolvable: &[&str]) -> Option<Share> {
        let freed = self
            .items
            .iter()
            .filter(|item| {
                !item.is_empty() && item.iter().all(|part| resolvable.contains(&part.as_str()))
            })
            .count();
        Share::new(freed, self.item_count())
    }

    /// The two quantities side by side (invariant 3): what was
    /// claimed (`per_part_share` of `shape`) against what the decision
    /// needs (`per_item_yield` of a resolver covering `resolvable`,
    /// which should include `shape`). `None` if either denominator is
    /// empty.
    pub fn yield_comparison(&self, shape: &str, resolvable: &[&str]) -> Option<YieldComparison> {
        Some(YieldComparison {
            per_part: self.per_part_share(shape)?,
            per_item: self.per_item_yield(resolvable)?,
        })
    }
}

/// Per-part frequency against the per-item yield it was promoted to
/// (invariant 3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct YieldComparison {
    /// The measured share of part instances (what was claimed).
    pub per_part: Share,
    /// The share of items actually freed (what the decision needs).
    pub per_item: Share,
}

impl YieldComparison {
    /// The signed gap in percentage points: the claim minus the
    /// decision's quantity. Positive where any-of overstates — the
    /// common case, where the shape rides through many items
    /// alongside other shapes — and negative where the shape is
    /// concentrated in the very few items that are pure. Either way
    /// the number is not the decision's quantity.
    pub fn gap_points(self) -> f64 {
        (self.per_part.fraction() - self.per_item.fraction()) * 100.0
    }

    /// The line for the report, both denominators visible.
    pub fn line(self) -> String {
        format!(
            "per-part {} vs per-item {}: {} points",
            self.per_part.line(),
            self.per_item.line(),
            (self.gap_points() * 10.0).round() as i32 / 10
        )
    }
}

/// An item with `parts` independent parts, each resolvable with
/// probability `per_part_success`, is freed only when *all* of them
/// are (invariant 3). That is `per_part_success ^ parts` — while the
/// per-part frequency reports `per_part_success`.
pub fn any_of_item_yield(per_part_success: f64, parts: usize) -> f64 {
    per_part_success.powi(parts as i32)
}

/// The gap between per-part frequency and per-item yield at `parts`
/// (invariant 3). For `0 < per_part_success < 1` and `parts >= 1` the
/// gap is non-decreasing in `parts` and positive from `parts == 2`
/// up: the overstatement grows with the number of parts.
pub fn any_of_gap(per_part_success: f64, parts: usize) -> f64 {
    per_part_success - any_of_item_yield(per_part_success, parts)
}

/// A claim that one candidate ranks highest — the shape of #4560 as
/// filed ("the dominant conflict", "the highest-yield single change").
#[derive(Debug, Clone, PartialEq)]
pub struct RankingClaim {
    /// What is being ranked ("additive_declarations").
    pub subject: String,
    /// The decision the ranking drives (invariant 1).
    pub target: RankingTarget,
    /// The quantity the number actually measures (invariant 1).
    pub metric: Metric,
    /// The number, with its denominator (invariant 2).
    pub share: Share,
    /// How the number was obtained (invariant 5).
    pub evidence: EvidenceKind,
    /// Whether a priority was attached ("highest-yield", "dominant",
    /// an issue priority). A priority is a claim with an evidentiary
    /// burden (invariant 4).
    pub priority: bool,
}

/// Why a claim does or does not stand (invariants 1, 2, 4, 5).
#[derive(Debug, Clone, PartialEq)]
pub enum ClaimVerdict {
    /// The metric matches the decision, and (where a priority rides
    /// on the number) the sample is stated and the evidence direct.
    Admissible,
    /// Invariant 1: a per-item decision ranked on per-part frequency.
    /// This fires with or without a priority — a recommendation is
    /// still a decision.
    MetricMismatch,
    /// Invariants 2 and 4: a priority on a share whose denominator is
    /// below [`MIN_MEASURED_N`] — a hypothesis filed as a finding.
    SmallSample {
        /// The denominator the priority was attached to.
        n: usize,
    },
    /// Invariant 5: a priority on a number reasoned from an adjacent
    /// count rather than measured.
    IndirectEvidence,
}

impl ClaimVerdict {
    /// The line for the report, naming what a corrected issue says.
    pub fn line(&self, claim: &RankingClaim) -> String {
        match self {
            ClaimVerdict::Admissible => {
                format!("admissible: {} at {}", claim.subject, claim.share.line())
            }
            ClaimVerdict::MetricMismatch => format!(
                "metric mismatch: '{}' is ranked on {} but the decision ranks on per-item yield; a part's share of instances is not the share of items freed",
                claim.subject,
                claim.metric.label()
            ),
            ClaimVerdict::SmallSample { n } => format!(
                "small sample: priority on {} — n={} is a hypothesis below MIN_MEASURED_N={}; state the number as a hypothesis or omit the ranking",
                claim.share.line(),
                n,
                MIN_MEASURED_N
            ),
            ClaimVerdict::IndirectEvidence => format!(
                "indirect evidence: priority on a number reasoned from an adjacent count; measure {} directly or omit the ranking",
                claim.subject
            ),
        }
    }
}

/// Judge a ranking claim (invariants 1, 2, 4, 5). The metric check
/// always applies — a recommendation is a decision — while the sample
/// and evidence checks apply to priorities: a priority is a claim, and
/// unmeasured, the issue says what is known and omits the ranking.
pub fn judge_claim(claim: &RankingClaim) -> ClaimVerdict {
    if claim.target == RankingTarget::FreeItems && claim.metric == Metric::PerPartFrequency {
        return ClaimVerdict::MetricMismatch;
    }
    if claim.priority {
        if claim.share.status() == ShareStatus::Hypothesis {
            return ClaimVerdict::SmallSample {
                n: claim.share.denominator,
            };
        }
        if claim.evidence == EvidenceKind::Indirect {
            return ClaimVerdict::IndirectEvidence;
        }
    }
    ClaimVerdict::Admissible
}

/// What an issue that cannot measure the ranking says instead of a
/// priority (invariant 4): what is known, with n, and the ordering
/// reported as unknown — never inferred from the counts nearest to
/// hand.
pub fn unknown_ordering_line(subject: &str, share: Share) -> String {
    format!(
        "ordering unknown: {} measured at {} — no priority attached until the ranking quantity is measured directly",
        subject,
        share.line()
    )
}
