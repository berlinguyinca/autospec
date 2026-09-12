//! The InferWeave capability contract (AAR spec section 12).
//!
//! AAR asks for capabilities, never for a physical node. The rule that drives
//! the whole scorer is the one in the spec: a faster node lacking sufficient
//! free context loses to an eligible node that has enough. Context is a hard
//! filter, not a score contribution, because no amount of speed makes a
//! request fit in a window that is too small.

/// How the caller wants latency traded against throughput and cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatencyPriority {
    Latency,
    Balanced,
    Throughput,
}

impl LatencyPriority {
    pub fn as_str(&self) -> &'static str {
        match self {
            LatencyPriority::Latency => "latency",
            LatencyPriority::Balanced => "balanced",
            LatencyPriority::Throughput => "throughput",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "latency" => LatencyPriority::Latency,
            "balanced" => LatencyPriority::Balanced,
            "throughput" => LatencyPriority::Throughput,
            _ => return None,
        })
    }
}

/// One active session's resource demand.
///
/// A session is a seat, not a slot: its demand is context plus projected
/// growth plus KV and model footprint, so a node that can hold four idle
/// sessions may not hold two growing ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SessionSeat {
    pub current_context_tokens: u64,
    pub projected_growth_tokens: u64,
    pub kv_tokens: u64,
}

impl SessionSeat {
    /// Total context tokens this seat will occupy.
    pub fn demand(&self) -> u64 {
        self.current_context_tokens
            .saturating_add(self.projected_growth_tokens)
            .saturating_add(self.kv_tokens)
    }
}

/// What AAR asks InferWeave for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRequest {
    pub model_class: String,
    pub model_allowlist: Vec<String>,
    pub minimum_context_free: u64,
    pub prefer_local: bool,
    pub session_affinity: bool,
    pub prefix_cache_key: String,
    pub latency_priority: LatencyPriority,
    /// Existing session this request continues, when affinity applies.
    pub session_id: String,
    pub seat: SessionSeat,
}

impl Default for CapabilityRequest {
    fn default() -> Self {
        Self {
            model_class: "coding-local".to_string(),
            model_allowlist: Vec::new(),
            minimum_context_free: 24_000,
            prefer_local: true,
            session_affinity: true,
            prefix_cache_key: String::new(),
            latency_priority: LatencyPriority::Balanced,
            session_id: String::new(),
            seat: SessionSeat::default(),
        }
    }
}

impl CapabilityRequest {
    /// Total free context a node must have to be eligible.
    pub fn required_free_context(&self) -> u64 {
        self.minimum_context_free.max(self.seat.demand())
    }

    /// Render the request in the spec section 12 YAML shape.
    pub fn to_yaml(&self) -> String {
        let allowlist = if self.model_allowlist.is_empty() {
            "[]".to_string()
        } else {
            format!("[{}]", self.model_allowlist.join(", "))
        };
        format!(
            "model_class: {}\nmodel_allowlist: {allowlist}\nminimum_context_free: {}\nprefer_local: {}\nsession_affinity: {}\nprefix_cache_key: \"{}\"\nlatency_priority: {}\n",
            self.model_class,
            self.required_free_context(),
            self.prefer_local,
            self.session_affinity,
            self.prefix_cache_key,
            self.latency_priority.as_str()
        )
    }
}

/// What one node reports about itself.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeOffer {
    pub node_id: String,
    pub served_models: Vec<String>,
    pub model_classes: Vec<String>,
    pub free_context_tokens: u64,
    pub total_context_tokens: u64,
    pub is_local: bool,
    /// Prefix cache keys this node already holds warm.
    pub warm_prefix_cache_keys: Vec<String>,
    /// Session this node already serves, if any.
    pub affinity_session_id: Option<String>,
    /// 0.0 idle to 1.0 saturated.
    pub utilization: f64,
    /// Live agent jobs running on this node; the first tiebreaker among
    /// equally scored workers.
    pub queue_depth: u32,
    pub observed_prefill_tokens_per_second: f64,
    pub observed_decode_tokens_per_second: f64,
    /// 0.0 free to 1.0 expensive; geography and network cost.
    pub network_cost: f64,
    /// Remaining fair-share for the requesting tenant, 0.0 to 1.0.
    pub qos_share_remaining: f64,
    pub overloaded: bool,
}

impl NodeOffer {
    fn serves(&self, request: &CapabilityRequest) -> bool {
        let class_ok = request.model_class.is_empty()
            || self
                .model_classes
                .iter()
                .any(|class| class == &request.model_class);
        let model_ok = request.model_allowlist.is_empty()
            || request
                .model_allowlist
                .iter()
                .any(|model| self.served_models.contains(model));
        class_ok && model_ok
    }
}

/// A node that passed every hard filter, with its score.
#[derive(Debug, Clone, PartialEq)]
pub struct CandidateScore {
    pub node_id: String,
    pub score: f64,
    pub reasons: Vec<String>,
}

/// The routing outcome, including why every rejected node lost.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutingDecision {
    pub selected: Option<String>,
    pub candidates: Vec<CandidateScore>,
    pub rejected: Vec<(String, String)>,
    pub rationale: Vec<String>,
}

impl RoutingDecision {
    pub fn is_routed(&self) -> bool {
        self.selected.is_some()
    }
}

/// Score node offers against a capability request.
///
/// Hard filters run first (model/class, overload, free context, exhausted fair
/// share); only then does scoring order what remains. Workers that tie on
/// score go to the one with the fewest live agent jobs, and any remaining
/// tie is broken at random, so simultaneous dispatches to identical workers
/// spread across the fleet instead of converging on a single one.
pub fn route(request: &CapabilityRequest, offers: &[NodeOffer]) -> RoutingDecision {
    let required_free = request.required_free_context();
    let mut candidates: Vec<(CandidateScore, u32)> = Vec::new();
    let mut rejected = Vec::new();

    for offer in offers {
        if !offer.serves(request) {
            rejected.push((
                offer.node_id.clone(),
                format!(
                    "does not serve required model class {} or allowlist",
                    request.model_class
                ),
            ));
            continue;
        }
        if offer.overloaded {
            rejected.push((offer.node_id.clone(), "node reports overload".to_string()));
            continue;
        }
        if offer.qos_share_remaining <= 0.0 {
            rejected.push((offer.node_id.clone(), "fair-share exhausted".to_string()));
            continue;
        }
        if offer.free_context_tokens < required_free {
            rejected.push((
                offer.node_id.clone(),
                format!(
                    "free context {} < required {required_free}",
                    offer.free_context_tokens
                ),
            ));
            continue;
        }

        let mut score = 0.0;
        let mut reasons = Vec::new();

        if request.session_affinity
            && !request.session_id.is_empty()
            && offer.affinity_session_id.as_deref() == Some(request.session_id.as_str())
        {
            score += 3.0;
            reasons.push("session affinity".to_string());
        }
        if !request.prefix_cache_key.is_empty()
            && offer
                .warm_prefix_cache_keys
                .contains(&request.prefix_cache_key)
        {
            score += 2.0;
            reasons.push("warm prefix cache".to_string());
        }
        if request.prefer_local && offer.is_local {
            score += 1.0;
            reasons.push("local node".to_string());
        }

        let headroom = if offer.total_context_tokens > 0 {
            offer.free_context_tokens as f64 / offer.total_context_tokens as f64
        } else {
            0.0
        };
        score += headroom;
        reasons.push(format!("context headroom {headroom:.2}"));

        score += (1.0 - offer.utilization.clamp(0.0, 1.0)) * 0.75;
        score -= f64::from(offer.queue_depth) * 0.1;
        score -= offer.network_cost.clamp(0.0, 1.0) * 0.5;
        score += offer.qos_share_remaining.clamp(0.0, 1.0) * 0.25;

        let (prefill_weight, decode_weight) = match request.latency_priority {
            LatencyPriority::Latency => (0.6, 0.4),
            LatencyPriority::Balanced => (0.3, 0.3),
            LatencyPriority::Throughput => (0.1, 0.5),
        };
        score += normalize_rate(offer.observed_prefill_tokens_per_second, 2_000.0) * prefill_weight;
        score += normalize_rate(offer.observed_decode_tokens_per_second, 120.0) * decode_weight;
        reasons.push(format!(
            "observed prefill {:.0} tok/s, decode {:.0} tok/s",
            offer.observed_prefill_tokens_per_second, offer.observed_decode_tokens_per_second
        ));

        candidates.push((
            CandidateScore {
                node_id: offer.node_id.clone(),
                score,
                reasons,
            },
            offer.queue_depth,
        ));
    }

    candidates.sort_by(|(left, left_jobs), (right, right_jobs)| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left_jobs.cmp(right_jobs))
    });

    let (selected, tie_size) = match candidates.first() {
        Some((best, best_jobs)) => {
            let tied: Vec<String> = candidates
                .iter()
                .filter(|(candidate, jobs)| candidate.score == best.score && *jobs == *best_jobs)
                .map(|(candidate, _)| candidate.node_id.clone())
                .collect();
            (Some(pick_among_tied(&tied)), tied.len())
        }
        None => (None, 0),
    };

    let candidates: Vec<CandidateScore> = candidates
        .into_iter()
        .map(|(candidate, _)| candidate)
        .collect();
    let mut rationale = vec![format!(
        "required_free_context={required_free} eligible={} rejected={}",
        candidates.len(),
        rejected.len()
    )];
    match (&selected, tie_size) {
        (Some(node), tie_size) if tie_size > 1 => rationale.push(format!(
            "{tie_size} candidates tied on score and live jobs; chose {node} among them at random"
        )),
        (Some(node), _) => rationale.push(format!("selected {node}")),
        (None, _) => rationale.push("no eligible node".to_string()),
    }

    RoutingDecision {
        selected,
        candidates,
        rejected,
        rationale,
    }
}

/// Choose one node from a tied group of equally ranked workers.
///
/// A deterministic tiebreak (such as lexicographic node id) turns the
/// selector into a constant: every simultaneous dispatch to a set of
/// identical workers lands on the same one. A random pick spreads them.
/// An entropy failure degrades to the first candidate rather than failing
/// the whole routing decision.
fn pick_among_tied(tied: &[String]) -> String {
    if tied.len() == 1 {
        return tied[0].clone();
    }
    let mut bytes = [0_u8; 4];
    match getrandom::fill(&mut bytes) {
        Ok(()) => tied[u32::from_le_bytes(bytes) as usize % tied.len()].clone(),
        Err(_) => tied[0].clone(),
    }
}

fn normalize_rate(observed: f64, reference: f64) -> f64 {
    if reference <= 0.0 {
        return 0.0;
    }
    (observed / reference).clamp(0.0, 1.0)
}

/// The kind of check a liveness probe runs against a worker.
///
/// The split is load-bearing, not stylistic. A check whose cost is constant
/// can be trusted as a verdict at any utilization; a check whose cost scales
/// with load is only trustworthy on an idle worker, which is exactly when a
/// verdict is least needed.
#[derive(Debug, Clone, PartialEq)]
pub enum ProbeCheck {
    /// A constant-cost control-plane call (e.g. `GET /health`). Reads
    /// metadata; consumes no GPU or KV capacity.
    Health,
    /// A constant-cost inventory call (e.g. `GET /v1/models`). Lists what is
    /// loaded; does not run the model.
    ModelList,
    /// A one-token completion. Its cost scales with load: on a busy worker
    /// it queues behind production traffic, so its latency measures the
    /// queue, not the health.
    ///
    /// Starvation argument: a completion probe starves itself of evidence
    /// exactly when the evidence is needed most. The busier the worker, the
    /// longer the probe's own completion waits behind production traffic, and
    /// the more likely a healthy worker misses its deadline and is evicted.
    /// A worker at peak demand is the one the pool can least afford to lose,
    /// so a completion may never gate eviction; it is legal only in the
    /// `verification` slot, where `starvation_argument` records why it is
    /// still run at all.
    Completion { starvation_argument: String },
}

impl ProbeCheck {
    /// True when the check's cost scales with the load on the worker it
    /// monitors. Only constant-cost checks may gate eviction.
    pub fn cost_scales_with_load(&self) -> bool {
        matches!(self, ProbeCheck::Completion { .. })
    }

    /// The starvation argument, present only for checks whose cost scales
    /// with load.
    pub fn starvation_argument(&self) -> Option<&str> {
        match self {
            ProbeCheck::Completion {
                starvation_argument,
            } => Some(starvation_argument.as_str()),
            ProbeCheck::Health | ProbeCheck::ModelList => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ProbeCheck::Health => "health",
            ProbeCheck::ModelList => "model-list",
            ProbeCheck::Completion { .. } => "completion",
        }
    }
}

/// How a gateway keeps a pool of workers honest.
///
/// The liveness check is the cheap one and is the only verdict that gates
/// pool membership. A costly verification, when present, lives in a separate
/// slot, and its result is information, not eviction authority.
#[derive(Debug, Clone, PartialEq)]
pub struct LivenessProbe {
    /// How often the probe runs.
    pub interval_secs: u32,
    /// How long an attempt may take before it is inconclusive rather than a
    /// failure verdict.
    pub deadline_secs: u32,
    /// The constant-cost check whose verdict gates pool membership.
    pub liveness: ProbeCheck,
    /// An optional costly check; its verdict never gates eviction.
    pub verification: Option<ProbeCheck>,
    /// The identity the worker must report to be the worker being probed.
    pub expected_identity: Option<String>,
}

impl LivenessProbe {
    /// The rules a probe must satisfy before it may gate a pool.
    ///
    /// 1. The liveness check must not consume the very resource whose
    ///    exhaustion it is supposed to survive: a completion runs the model
    ///    and takes GPU and KV capacity, so it is rejected from this slot.
    /// 2. A costly check is legal only in `verification`, where it cannot
    ///    evict.
    /// 3. Any check whose cost scales with load must carry an explicit,
    ///    non-blank starvation argument.
    pub fn validate(&self) -> Result<(), String> {
        if self.interval_secs == 0 {
            return Err("probe interval must be positive".to_string());
        }
        if self.deadline_secs == 0 {
            return Err("probe deadline must be positive".to_string());
        }
        if self.liveness.cost_scales_with_load() {
            return Err(
                "the liveness check must be constant-cost: a completion consumes the \
                 very resource whose exhaustion the probe is supposed to survive"
                    .to_string(),
            );
        }
        if let Some(verification) = &self.verification {
            if verification.cost_scales_with_load()
                && verification
                    .starvation_argument()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
            {
                return Err(
                    "a check whose cost scales with load must carry an explicit \
                     starvation argument"
                        .to_string(),
                );
            }
        }
        Ok(())
    }
}

/// One raw observation from a probe attempt.
#[derive(Debug, Clone, PartialEq)]
pub enum ProbeSignal {
    /// The endpoint answered with a success status. Carries the identity it
    /// reported, when the check surfaces one.
    Live { identity: Option<String> },
    /// The connection was refused: the process or port is gone.
    Refused,
    /// The connection was reset mid-probe.
    Reset,
    /// The endpoint answered with an error status.
    ErrorStatus { status: u16 },
    /// The check did not finish within its deadline. On a busy worker the
    /// check queues behind production traffic, so a deadline miss says
    /// something about the check, not about the worker.
    DeadlineExceeded,
}

/// Why a worker is definitively out of the pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadReason {
    Refused,
    Reset,
    ErrorStatus,
    IdentityMismatch,
}

impl DeadReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            DeadReason::Refused => "refused",
            DeadReason::Reset => "reset",
            DeadReason::ErrorStatus => "error-status",
            DeadReason::IdentityMismatch => "identity-mismatch",
        }
    }
}

/// The classification a probe attempt receives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeVerdict {
    /// The worker is alive and is the worker being probed.
    Alive,
    /// The attempt produced no usable signal; the worker stays in the pool.
    Inconclusive,
    /// The worker is definitively dead or is not the worker being probed.
    Dead { reason: DeadReason },
}

/// Classify one probe observation against the probe's identity expectation.
///
/// Only signals that are a definitive statement about the worker classify as
/// dead. A deadline miss is a statement about the check, and it classifies as
/// inconclusive no matter how many misses a worker has accumulated.
pub fn classify_probe(probe: &LivenessProbe, signal: &ProbeSignal) -> ProbeVerdict {
    match signal {
        ProbeSignal::Live { identity } => {
            if let (Some(expected), Some(observed)) = (&probe.expected_identity, identity) {
                if observed != expected {
                    return ProbeVerdict::Dead {
                        reason: DeadReason::IdentityMismatch,
                    };
                }
            }
            ProbeVerdict::Alive
        }
        ProbeSignal::Refused => ProbeVerdict::Dead {
            reason: DeadReason::Refused,
        },
        ProbeSignal::Reset => ProbeVerdict::Dead {
            reason: DeadReason::Reset,
        },
        ProbeSignal::ErrorStatus { .. } => ProbeVerdict::Dead {
            reason: DeadReason::ErrorStatus,
        },
        ProbeSignal::DeadlineExceeded => ProbeVerdict::Inconclusive,
    }
}

/// What the pool does to a worker after a liveness verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolAction {
    /// The worker remains eligible for routing.
    Keep,
    /// The worker is removed from the pool.
    Evict,
}

impl ProbeVerdict {
    /// Only a definitive failure evicts. A timeout is not a failure verdict:
    /// on a busy worker the probe queues behind production traffic and misses
    /// its deadline, and evicting on that removes exactly the worker the pool
    /// needs most.
    pub fn pool_action(&self) -> PoolAction {
        match self {
            ProbeVerdict::Alive | ProbeVerdict::Inconclusive => PoolAction::Keep,
            ProbeVerdict::Dead { .. } => PoolAction::Evict,
        }
    }
}

/// One registration probe: the two steps the gateway runs before it lets a
/// worker (re-)register.
///
/// The identity step is a constant-cost inventory call; the liveness step
/// is a one-token completion whose cost scales with load, so on a busy
/// worker it queues behind production traffic and times out even though
/// the worker is healthy.
#[derive(Debug, Clone, PartialEq)]
pub struct RegistrationProbe {
    /// What the identity step (`GET /v1/models`) returned.
    pub identity: ProbeSignal,
    /// The model list the identity step observed. An empty list means the
    /// step established nothing — the endpoint may have timed out on this
    /// step as well — and the worker's identity is unknown, not
    /// "anything goes".
    pub observed_models: Vec<String>,
    /// What the liveness step (one-token completion) returned.
    pub liveness: ProbeSignal,
}

/// Why a (re-)registration was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalReason {
    /// Either probe step said something definitive about the worker:
    /// refused, reset, error status, or a reported identity that
    /// contradicts the expected one.
    WorkerDead { reason: DeadReason },
    /// The liveness step timed out (busy is not dead) but the identity step
    /// established no model list: the worker proved nothing at all, and an
    /// unknown identity must never route into the permissive branch.
    IdentityNeverEstablished,
    /// The observed model list does not include the requested model.
    DoesNotServeModel,
}

impl RefusalReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            RefusalReason::WorkerDead { reason } => reason.as_str(),
            RefusalReason::IdentityNeverEstablished => "identity-never-established",
            RefusalReason::DoesNotServeModel => "does-not-serve-model",
        }
    }
}

/// The admission decision for a (re-)registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionVerdict {
    /// The worker may (re-)register; it serves the requested model and the
    /// liveness step completed, so the checks on the success path actually
    /// ran against it.
    Admitted,
    /// The worker may (re-)register **provisionally**: the liveness step
    /// timed out ("busy is not dead") and the measurement it carries never
    /// completed, so a check sitting on the success path — such as a
    /// throughput floor — never ran against this worker (issue #4411). The
    /// bypass is not an exit: the worker is marked never measured and the
    /// measurement must succeed within a bounded window before the worker
    /// counts as healthy (see [`admission_obligation`](super::admission_obligation)).
    AdmittedProvisionally,
    /// Registration is refused.
    Refused { reason: RefusalReason },
}

impl AdmissionVerdict {
    pub fn is_admitted(&self) -> bool {
        matches!(
            self,
            AdmissionVerdict::Admitted | AdmissionVerdict::AdmittedProvisionally
        )
    }

    /// Whether the admission carries a measurement obligation: a worker
    /// admitted through the timeout branch was never measured, and the
    /// bounded window decides when "never measured" becomes "unmeasurable".
    pub fn requires_measurement(&self) -> bool {
        matches!(self, AdmissionVerdict::AdmittedProvisionally)
    }
}

/// Whether the worker's *observed* model list says it serves `model`.
///
/// Fails closed on an empty list. "Nothing observed costs nothing" was
/// safe only relative to the callers that existed when the default was
/// written: every one of them rejected on any probe error, so the
/// permissive branch could never decide. The moment this predicate
/// decides by itself — an admission that proceeds on an inconclusive
/// liveness step — an empty list proves nothing, and "unknown" must never
/// be routed into the permissive branch.
///
/// An unconstrained request (`model` empty) is satisfied only by a
/// non-empty observed list, never by the absence of one.
pub fn serves_model(observed: &[String], model: &str) -> bool {
    if observed.is_empty() {
        return false;
    }
    model.is_empty() || observed.iter().any(|m| m == model)
}

/// Decide whether a worker may (re-)register to serve `model`.
///
/// The same observation drives two decisions — the liveness loop's
/// keep/evict verdict and this admission — and the interpretation belongs
/// in the one shared predicate both consult: [`classify_probe`]. A
/// deadline miss on the liveness step is "busy is not dead" here too, so
/// the busiest worker can still re-register instead of expiring at its TTL
/// and turning into `no worker for model`.
///
/// A definitive failure on either step refuses. A timed-out liveness step
/// then rests on what the identity step actually established, and an
/// identity that was never established refuses: the permissive branch
/// decides only from an observation.
///
/// The permissive branch is not an exit (issue #4411): when the liveness
/// step timed out, the measurement it carries never ran, and any check
/// placed on the success path — the incident's throughput floor — was
/// bypassed by the very workers it existed to catch. Such an admission
/// returns [`AdmissionVerdict::AdmittedProvisionally`]: the worker is
/// admitted, marked never measured, and must complete the measurement
/// within a bounded window before it counts as healthy.
pub fn admit(
    probe: &LivenessProbe,
    registration: &RegistrationProbe,
    model: &str,
) -> AdmissionVerdict {
    let identity = classify_probe(probe, &registration.identity);
    let liveness = classify_probe(probe, &registration.liveness);

    if let ProbeVerdict::Dead { reason } = liveness {
        return AdmissionVerdict::Refused {
            reason: RefusalReason::WorkerDead { reason },
        };
    }
    if let ProbeVerdict::Dead { reason } = identity {
        return AdmissionVerdict::Refused {
            reason: RefusalReason::WorkerDead { reason },
        };
    }

    if !serves_model(&registration.observed_models, model) {
        let reason = if registration.observed_models.is_empty() {
            RefusalReason::IdentityNeverEstablished
        } else {
            RefusalReason::DoesNotServeModel
        };
        return AdmissionVerdict::Refused { reason };
    }

    // "Busy is not dead" is a bypass that carries an obligation, not an
    // exit: the liveness step never completed, so the worker was never
    // measured and the success-path checks never ran against it. Admit it
    // provisionally; the bounded window in
    // [`admission_obligation`](super::admission_obligation) decides when a
    // worker that can never complete a measurement stops counting as
    // healthy (issue #4411).
    if let ProbeVerdict::Inconclusive = liveness {
        return AdmissionVerdict::AdmittedProvisionally;
    }

    AdmissionVerdict::Admitted
}
