//! Over-committed unified KV deadlocks the server (issue #4377).
//!
//! `--kv-unified-per-slot` is a ceiling on what ONE slot may draw from the
//! shared KV pool. Setting it to the whole pool is what makes context
//! dynamic — a single large request can have everything. Set it to the full
//! pool while leaving the slot count at N > 1 and every one of the N slots
//! is entitled to the entire pool: a blocking resource that all of them are
//! over-entitled to. When concurrent large requests arrive they deadlock
//! competing for KV — `llama-server` keeps answering `/health`, stops
//! generating, and the GPUs sit at 0% still holding their allocation.
//!
//! Reproduced deterministically: six concurrent 31.7k-token requests.
//!
//! | slots | per-slot ceiling | unified | result  | after    |
//! |------:|-----------------:|:-------:|:--------|:---------|
//! |     4 | whole pool       |  yes    | 000 × 6 | wedged, GPU 0%, 50 GB held |
//! |     8 | 32k              |   no    | 400 × 6 (clean rejection) | healthy |
//! |     1 | whole pool       |  yes    | 200 × 6 (served serially) | healthy |
//!
//! The invariants from the issue, each mapped to a primitive here:
//!
//! 1. **A per-slot ceiling above pool/slots is an over-commitment, and
//!    over-commitment of a blocking resource is a deadlock.** Either the
//!    ceiling is a fair share, or the slot count is one. There is no safe
//!    middle setting ([`KvConfig::classify`], [`fundable_slots`]).
//! 2. **Test a resource-allocation change under concurrency before shipping
//!    it.** A single request cannot reveal contention
//!    ([`admit`], [`serial_check`]).
//! 3. **A latent deadlock is indistinguishable from a healthy configuration
//!    until load arrives** ([`latent`]).
//! 4. **When a change makes one component fail, check every component that
//!    took the same change** ([`fleet_audit`], [`same_change`]).
//!
//! The fix falls out of the model: `parallel = 1` whenever the per-slot
//! ceiling is the whole pool ([`fundable_slots`], [`KvConfig::remediated`]).
//! Concurrent requests then queue, which is a wait, not a hang.
//!
//! Everything here is pure: no I/O, no clock, no subprocess. The caller
//! observes the worker's configuration and load and calls these with the
//! observed values.

/// The shared-KV configuration of one worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KvConfig {
    /// How many slots (parallel sequences) the server runs.
    pub slots: u32,
    /// The per-slot ceiling: what one slot may draw from the KV pool, in MiB.
    /// With `unified`, slots draw from one shared pool; without, each slot
    /// has its own bounded budget.
    pub per_slot_mib: u64,
    /// The size of the KV pool, in MiB.
    pub pool_mib: u64,
    /// Whether the slots share one KV pool.
    pub unified: bool,
}

impl KvConfig {
    /// A worker always runs at least one slot.
    pub fn new(slots: u32, per_slot_mib: u64, pool_mib: u64, unified: bool) -> Option<Self> {
        (slots > 0).then_some(Self {
            slots,
            per_slot_mib,
            pool_mib,
            unified,
        })
    }

    /// The fair share: what each slot may draw for all slots to be
    /// simultaneously funded.
    pub fn fair_share(&self) -> u64 {
        self.pool_mib / u64::from(self.slots)
    }

    /// Invariant 1: classify the configuration.
    ///
    /// The total entitlement — `slots × per-slot ceiling` — either fits in
    /// the pool or it does not. With more than one slot, an entitlement
    /// above the pool means each slot is entitled to more than a fair share
    /// of a blocking resource: over-commitment, a deadlock under concurrent
    /// load. With one slot there is nothing to compete with, so a single
    /// slot is always the safe shape — concurrent requests queue (a wait,
    /// not a hang) — and everything else that fits is a fair share. There is
    /// no safe middle setting.
    pub fn classify(&self) -> KvClass {
        if self.slots >= 2
            && u64::from(self.slots).saturating_mul(self.per_slot_mib) > self.pool_mib
        {
            KvClass::OverCommitted
        } else if self.slots == 1 {
            KvClass::SingleSlot
        } else {
            KvClass::FairShare
        }
    }

    /// The remediated configuration: the slot count capped at what the pool
    /// can fund for this ceiling, everything else unchanged. This is the
    /// fix — `parallel = 1` when the ceiling is the whole pool — and it
    /// leaves already-safe configurations untouched.
    pub fn remediated(&self) -> KvConfig {
        if self.classify() != KvClass::OverCommitted {
            return *self;
        }
        let slots = self
            .slots
            .min(fundable_slots(self.pool_mib, self.per_slot_mib));
        // fundable_slots is >= 1 and self.slots is >= 1, so this is valid.
        match Self::new(slots, self.per_slot_mib, self.pool_mib, self.unified) {
            Some(cfg) => cfg,
            None => *self,
        }
    }

    /// The one-line report of the configuration.
    pub fn line(&self) -> String {
        format!(
            "slots={} per-slot={} MiB pool={} MiB {}",
            self.slots,
            self.per_slot_mib,
            self.pool_mib,
            if self.unified {
                "unified"
            } else {
                "separate kv"
            }
        )
    }
}

/// The safety class of a [`KvConfig`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KvClass {
    /// The per-slot ceiling is at most pool/slots: every slot's ceiling is a
    /// fair share and all slots are simultaneously fundable.
    FairShare,
    /// One slot is entitled to the whole pool: a single large request can
    /// have everything, and concurrent requests queue. A wait, not a hang.
    SingleSlot,
    /// More than one slot is each entitled to more than a fair share of a
    /// blocking resource. Over-commitment: concurrent large requests
    /// deadlock competing for KV.
    OverCommitted,
}

impl KvClass {
    pub fn label(&self) -> &'static str {
        match self {
            KvClass::FairShare => "fair share",
            KvClass::SingleSlot => "single slot, whole pool",
            KvClass::OverCommitted => {
                "over-committed: each slot is entitled to more than pool/slots"
            }
        }
    }
}

/// Invariant 1: how many slots the pool can fund at this ceiling.
///
/// The fix: `parallel = 1` whenever the per-slot ceiling is the whole pool.
/// A ceiling of zero imposes no bound, so any slot count is fundable.
pub fn fundable_slots(pool_mib: u64, per_slot_mib: u64) -> u32 {
    if per_slot_mib == 0 {
        return 0;
    }
    let funded = pool_mib / per_slot_mib;
    if funded == 0 {
        // The ceiling exceeds the pool: one slot is still fundable, because
        // a single request physically draws at most the pool.
        1
    } else {
        funded as u32
    }
}

/// The result of admitting `n_concurrent` requests, each needing up to
/// `request_mib` of KV, against a worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmitResult {
    /// Every request was served; the surplus waited for a free slot, which
    /// is a wait, not a hang.
    Served {
        /// How many requests waited their turn behind the in-flight ones.
        queued: u32,
    },
    /// The requests cannot fit their own ceiling: the server rejects them
    /// cleanly (400) and stays healthy.
    Rejected { rejected: u32 },
    /// The in-flight requests deadlocked competing for KV: the server keeps
    /// answering `/health`, stops generating, GPUs at 0%, allocation held.
    Deadlocked {
        /// How many in-flight requests are wedged.
        stuck: u32,
        /// How many queued requests are starved behind the wedge.
        queued: u32,
        /// How much of the pool the wedge is holding.
        held_mib: u64,
    },
}

/// Invariant 2: admit `n_concurrent` requests, each needing up to
/// `request_mib` of KV, against `cfg`.
///
/// A request whose need exceeds its own ceiling (or the pool) is rejected
/// cleanly — a 400, with the server healthy afterwards. Otherwise the
/// in-flight set is the concurrent requests capped at the slot count. On a
/// unified pool the in-flight set is fundable only if its total entitlement
/// — `in-flight × per-slot ceiling` — fits in the pool. When two or more
/// requests are in flight and the entitlement exceeds the pool, they
/// deadlock competing for the KV: this is the incident.
pub fn admit(cfg: &KvConfig, request_mib: u64, n_concurrent: u32) -> AdmitResult {
    if request_mib > cfg.per_slot_mib || request_mib > cfg.pool_mib {
        return AdmitResult::Rejected {
            rejected: n_concurrent,
        };
    }
    let in_flight = n_concurrent.min(cfg.slots);
    if cfg.unified && in_flight >= 2 {
        let entitled = u64::from(in_flight).saturating_mul(cfg.per_slot_mib);
        if entitled > cfg.pool_mib {
            return AdmitResult::Deadlocked {
                stuck: in_flight,
                queued: n_concurrent - in_flight,
                held_mib: entitled.min(cfg.pool_mib),
            };
        }
    }
    AdmitResult::Served {
        queued: n_concurrent - in_flight,
    }
}

/// Invariant 2: the serial check — one small request, the smoke test that
/// passed on every worker in the incident. A single request cannot reveal
/// contention, so every over-committed configuration passes it.
pub fn serial_check(cfg: &KvConfig) -> bool {
    !matches!(admit(cfg, 1, 1), AdmitResult::Deadlocked { .. })
}

/// Invariant 3: a configuration that is over-committed yet passes the
/// serial check. Until load arrives it is indistinguishable from a healthy
/// configuration — the fleet looked 5/6 healthy while every one of those
/// workers carried the same fault.
pub fn latent(cfg: &KvConfig) -> bool {
    cfg.classify() == KvClass::OverCommitted && serial_check(cfg)
}

/// A worker and its shared-KV configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerKv {
    pub worker: String,
    pub model: String,
    pub cfg: KvConfig,
}

/// The fleet-wide audit run after one worker has failed (invariant 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetReport {
    /// Every worker in the fleet.
    pub total: usize,
    /// The workers carrying an over-committed configuration — not just the
    /// one that failed first.
    pub overcommitted: Vec<String>,
    /// How many over-committed workers still pass the serial check: latent
    /// deadlocks that look exactly like working configurations.
    pub looking_healthy: usize,
}

/// Invariant 4: audit every worker, not just the victim. When a change makes
/// one component fail, check every component that took the same change — the
/// worker that failed first is the earliest victim, not the only one.
pub fn fleet_audit(workers: &[WorkerKv]) -> FleetReport {
    let overcommitted = workers
        .iter()
        .filter(|w| w.cfg.classify() == KvClass::OverCommitted)
        .map(|w| w.worker.clone())
        .collect();
    let looking_healthy = workers.iter().filter(|w| latent(&w.cfg)).count();
    FleetReport {
        total: workers.len(),
        overcommitted,
        looking_healthy,
    }
}

impl FleetReport {
    /// A fleet with no over-committed worker.
    pub fn clean(&self) -> bool {
        self.overcommitted.is_empty()
    }

    /// The report line. A dirty fleet says so by name, and says that its
    /// latent deadlocks are passing the serial check.
    pub fn line(&self) -> String {
        if self.clean() {
            return format!(
                "fleet: {} worker(s), no over-committed KV configuration",
                self.total
            );
        }
        format!(
            "fleet: {} worker(s), {} over-committed ({}) — {} still pass the serial \
             check: a latent deadlock is indistinguishable from a healthy \
             configuration until load arrives",
            self.total,
            self.overcommitted.len(),
            self.overcommitted.join(", "),
            self.looking_healthy
        )
    }
}

/// Invariant 4: the workers that took the same change as `victim` — the
/// identical configuration. The set carries the same fault, whether or not
/// it has fired in each one. Returns nothing when `victim` is not in the
/// fleet.
pub fn same_change<'a>(workers: &'a [WorkerKv], victim: &str) -> Vec<&'a WorkerKv> {
    let cfg = match workers.iter().find(|w| w.worker == victim) {
        Some(victim) => victim.cfg,
        None => return Vec::new(),
    };
    workers.iter().filter(|w| w.cfg == cfg).collect()
}
