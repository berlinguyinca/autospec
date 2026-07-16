use crate::coordination::RemoteIssue;

#[derive(Debug, Clone)]
pub struct Tier15Input {
    pub(super) open: Vec<RemoteIssue>,
    pub(super) closed: Vec<RemoteIssue>,
    pub(super) budget: usize,
}

impl Tier15Input {
    pub fn new(open: Vec<RemoteIssue>, closed: Vec<RemoteIssue>, budget: usize) -> Self {
        Self {
            open,
            closed,
            budget,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier15Observation {
    pub(super) open_observed: usize,
    pub(super) open_deduplicated: usize,
    pub(super) closed_observed: usize,
    pub(super) budget: usize,
    pub(super) decisions: Vec<Tier15Decision>,
}

impl Tier15Observation {
    pub fn decisions(&self) -> &[Tier15Decision] {
        &self.decisions
    }

    pub fn produced_count(&self) -> usize {
        self.decisions
            .iter()
            .filter(|decision| matches!(decision, Tier15Decision::Produced { .. }))
            .count()
    }

    pub fn evidence_json(&self) -> String {
        let decisions = self
            .decisions
            .iter()
            .map(Tier15Decision::json)
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"schema\":1,\"kind\":\"tier15_observation\",\"open_observed\":{},\"open_deduplicated\":{},\"closed_observed\":{},\"budget\":{},\"decisions\":[{decisions}]}}\n",
            self.open_observed, self.open_deduplicated, self.closed_observed, self.budget
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier15Classification {
    NeedsClassify,
    NeedsTemplate,
    Unlabeled,
}

impl Tier15Classification {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::NeedsClassify => "needs_classify",
            Self::NeedsTemplate => "needs_template",
            Self::Unlabeled => "unlabeled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier15SkipReason {
    ExcludedLabel,
    ClosedFingerprint,
    AlreadyGroomed,
    BudgetExhausted,
}

impl Tier15SkipReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::ExcludedLabel => "excluded_label",
            Self::ClosedFingerprint => "closed_fingerprint",
            Self::AlreadyGroomed => "already_groomed",
            Self::BudgetExhausted => "budget_exhausted",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier15HoldReason {
    ThinIntent,
    AmbiguousIntent,
    Dependency,
}

impl Tier15HoldReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::ThinIntent => "thin_intent",
            Self::AmbiguousIntent => "ambiguous_intent",
            Self::Dependency => "dependency",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier15QuarantineReason {
    ExistingSecurity,
}

impl Tier15QuarantineReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::ExistingSecurity => "existing_security",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier15Route {
    Split,
    Template,
}

impl Tier15Route {
    fn as_str(self) -> &'static str {
        match self {
            Self::Split => "split",
            Self::Template => "template",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier15RouteReason {
    Epic,
    TemplateRequired,
    StructuredIntent,
    BroadScope,
}

impl Tier15RouteReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Epic => "epic",
            Self::TemplateRequired => "template_required",
            Self::StructuredIntent => "structured_intent",
            Self::BroadScope => "broad_scope",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tier15Decision {
    Produced {
        number: u64,
        classification: Tier15Classification,
    },
    Skipped {
        number: u64,
        classification: Tier15Classification,
        reason: Tier15SkipReason,
    },
    Held {
        number: u64,
        classification: Tier15Classification,
        reason: Tier15HoldReason,
    },
    Quarantined {
        number: u64,
        classification: Tier15Classification,
        reason: Tier15QuarantineReason,
    },
    Routed {
        number: u64,
        classification: Tier15Classification,
        route: Tier15Route,
        reason: Tier15RouteReason,
    },
}

impl Tier15Decision {
    pub fn eligibility(&self) -> Tier15Eligibility {
        match self {
            Self::Produced { .. } => Tier15Eligibility::Produced,
            Self::Skipped { reason, .. } => Tier15Eligibility::Skipped(*reason),
            Self::Held { reason, .. } => Tier15Eligibility::Held(*reason),
            Self::Quarantined { reason, .. } => Tier15Eligibility::Quarantined(*reason),
            Self::Routed { route, reason, .. } => Tier15Eligibility::Routed {
                route: *route,
                reason: *reason,
            },
        }
    }

    fn json(&self) -> String {
        match self {
            Self::Produced {
                number,
                classification,
            } => format!(
                "{{\"number\":{number},\"classification\":\"{}\",\"decision\":\"produced\"}}",
                classification.as_str()
            ),
            Self::Skipped {
                number,
                classification,
                reason,
            } => format!(
                "{{\"number\":{number},\"classification\":\"{}\",\"decision\":\"skipped\",\"reason\":\"{}\"}}",
                classification.as_str(),
                reason.as_str()
            ),
            Self::Held {
                number,
                classification,
                reason,
            } => format!(
                "{{\"number\":{number},\"classification\":\"{}\",\"decision\":\"held\",\"reason\":\"{}\"}}",
                classification.as_str(),
                reason.as_str()
            ),
            Self::Quarantined {
                number,
                classification,
                reason,
            } => format!(
                "{{\"number\":{number},\"classification\":\"{}\",\"decision\":\"quarantined\",\"reason\":\"{}\"}}",
                classification.as_str(),
                reason.as_str()
            ),
            Self::Routed {
                number,
                classification,
                route,
                reason,
            } => format!(
                "{{\"number\":{number},\"classification\":\"{}\",\"decision\":\"routed\",\"route\":\"{}\",\"reason\":\"{}\"}}",
                classification.as_str(),
                route.as_str(),
                reason.as_str()
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier15Eligibility {
    Produced,
    Skipped(Tier15SkipReason),
    Held(Tier15HoldReason),
    Quarantined(Tier15QuarantineReason),
    Routed {
        route: Tier15Route,
        reason: Tier15RouteReason,
    },
}
