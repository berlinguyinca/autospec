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
/// share); only then does scoring order what remains.
pub fn route(request: &CapabilityRequest, offers: &[NodeOffer]) -> RoutingDecision {
    let required_free = request.required_free_context();
    let mut candidates = Vec::new();
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

        candidates.push(CandidateScore {
            node_id: offer.node_id.clone(),
            score,
            reasons,
        });
    }

    candidates.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.node_id.cmp(&right.node_id))
    });

    let selected = candidates
        .first()
        .map(|candidate| candidate.node_id.clone());
    let mut rationale = vec![format!(
        "required_free_context={required_free} eligible={} rejected={}",
        candidates.len(),
        rejected.len()
    )];
    match &selected {
        Some(node) => rationale.push(format!("selected {node}")),
        None => rationale.push("no eligible node".to_string()),
    }

    RoutingDecision {
        selected,
        candidates,
        rejected,
        rationale,
    }
}

fn normalize_rate(observed: f64, reference: f64) -> f64 {
    if reference <= 0.0 {
        return 0.0;
    }
    (observed / reference).clamp(0.0, 1.0)
}
