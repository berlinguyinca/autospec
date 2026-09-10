//! Pool-size history: a slow drain must be visible before it reaches zero.

use std::fmt;

/// How many consecutive declines make a pool trend a *sustained* one. One
/// decline is a preemption; a run of them is a drain.
pub const DEFAULT_DECLINE_WINDOW: usize = 3;

/// One reconciler pass' observation of the pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolSample {
    /// Pass sequence number or epoch seconds; only ordering is used.
    pub at: u64,
    /// How many components the pool reported knowing about.
    pub size: usize,
}

/// The trend over the recent pool samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolTrend {
    /// Not enough history to say anything.
    NoHistory,
    /// No sustained decline.
    Stable,
    /// The last delta is up.
    Growing { delta: usize },
    /// `window` consecutive declines: the pool is draining.
    Draining { drop: usize, window: usize },
    /// The pool is at zero after having held members.
    Drained { was: usize },
}

impl fmt::Display for PoolTrend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoHistory => f.write_str("no history"),
            Self::Stable => f.write_str("stable"),
            Self::Growing { delta } => write!(f, "growing +{delta}"),
            Self::Draining { drop, window } => write!(f, "DRAINING -{drop} over {window} passes"),
            Self::Drained { was } => write!(f, "DRAINED: pool empty (was {was})"),
        }
    }
}

/// Pool-size history for a reconciler that must notice slow drain.
///
/// "One gateway up … nothing to do" is a statement about the *job*, repeated
/// every interval forever. This keeps the number that actually matters — how
/// many components the pool knows about — and turns a run of declines into a
/// verdict on the same line.
#[derive(Debug, Clone)]
pub struct PoolMonitor {
    samples: Vec<PoolSample>,
    decline_window: usize,
}

impl Default for PoolMonitor {
    fn default() -> Self {
        Self::new(DEFAULT_DECLINE_WINDOW)
    }
}

impl PoolMonitor {
    /// A monitor that calls `decline_window` consecutive declines a drain.
    pub fn new(decline_window: usize) -> Self {
        Self {
            samples: Vec::new(),
            decline_window: decline_window.max(1),
        }
    }

    /// Record one pass. The window keeps at most `decline_window + 1` samples.
    pub fn record(&mut self, at: u64, size: usize) {
        self.samples.push(PoolSample { at, size });
        let keep = self.decline_window + 1;
        if self.samples.len() > keep {
            let excess = self.samples.len() - keep;
            self.samples.drain(..excess);
        }
    }

    /// Latest observed pool size.
    pub fn size(&self) -> Option<usize> {
        self.samples.last().map(|s| s.size)
    }

    /// Highest pool size seen in the retained window.
    pub fn peak(&self) -> Option<usize> {
        self.samples.iter().map(|s| s.size).max()
    }

    /// The trend over the retained samples.
    pub fn trend(&self) -> PoolTrend {
        let last = match self.samples.last() {
            Some(sample) => sample,
            None => return PoolTrend::NoHistory,
        };
        if self.samples.len() < 2 {
            return PoolTrend::NoHistory;
        }
        let first = &self.samples[0];
        if last.size == 0 && first.size > 0 {
            return PoolTrend::Drained { was: first.size };
        }
        let declines = self
            .samples
            .windows(2)
            .filter(|w| w[1].size < w[0].size)
            .count();
        if declines >= self.decline_window {
            return PoolTrend::Draining {
                drop: first.size.saturating_sub(last.size),
                window: self.decline_window,
            };
        }
        if last.size > first.size {
            return PoolTrend::Growing {
                delta: last.size - first.size,
            };
        }
        PoolTrend::Stable
    }

    /// True when the trend is a sustained decline the operator must see.
    pub fn decline_flagged(&self) -> bool {
        matches!(
            self.trend(),
            PoolTrend::Draining { .. } | PoolTrend::Drained { .. }
        )
    }

    /// The reconciler's line for one pass.
    ///
    /// It always carries the pool size next to the gateway count, so "nothing
    /// to do" can never again be said without naming the number that is going
    /// down. `verb` is what the pass did (e.g. `"nothing to do"`).
    pub fn reconcile_line(&self, gateways_running: usize, verb: &str) -> String {
        let pool = self
            .size()
            .map(|size| size.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let peak = self
            .peak()
            .map(|peak| peak.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        format!(
            "gateways: {gateways_running} up; pool: {pool} registered (peak {peak}); trend: {}; {verb}",
            self.trend()
        )
    }
}
