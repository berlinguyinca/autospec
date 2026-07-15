pub const STALE_LEASE_SECS: u64 = 300;
pub const ABANDONED_LEASE_SECS: u64 = 10_800;
pub const ISSUE_FAILURE_CAP: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopMode {
    Graceful,
    Immediate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleTier {
    Tier1,
    Tier15,
    Tier2,
    Tier3,
    Tier4,
    Tier5,
    Tier6,
    Tier7,
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleReject {
    InvalidScope,
    CrossScopeClaim,
    StaleLease,
    TerminalClaim,
    OwnershipMismatch,
    FailureCap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Continue,
    Wait,
    Halt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Budget {
    WithinCap,
    SoftCap,
    HardCap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkReason {
    HumanGate,
    HealthWait,
    HealthHalt,
    BudgetSoftCap,
    BudgetHardCap,
    IdleRescan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleDecision {
    Stop { mode: StopMode },
    Reject(LifecycleReject),
    Park { reason: ParkReason },
    Run { tier: LifecycleTier },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleInput {
    repo: String,
    claim_repo: Option<String>,
    stop_mode: Option<StopMode>,
    health: Health,
    budget: Budget,
    human_gate: bool,
    lease_age_secs: u64,
    claim_terminal: bool,
    ownership_matches: bool,
    failure_count: u8,
    next_tier: LifecycleTier,
}

impl LifecycleInput {
    pub fn ready(repo: impl Into<String>) -> Self {
        Self {
            repo: repo.into(),
            claim_repo: None,
            stop_mode: None,
            health: Health::Continue,
            budget: Budget::WithinCap,
            human_gate: false,
            lease_age_secs: 0,
            claim_terminal: false,
            ownership_matches: true,
            failure_count: 0,
            next_tier: LifecycleTier::Tier1,
        }
    }

    pub fn with_stop(mut self, mode: StopMode) -> Self {
        self.stop_mode = Some(mode);
        self
    }

    pub fn with_claim_repo(mut self, repo: impl Into<String>) -> Self {
        self.claim_repo = Some(repo.into());
        self
    }

    pub fn with_lease_age_secs(mut self, value: u64) -> Self {
        self.lease_age_secs = value;
        self
    }

    pub fn with_terminal_claim(mut self) -> Self {
        self.claim_terminal = true;
        self
    }

    pub fn with_ownership_mismatch(mut self) -> Self {
        self.ownership_matches = false;
        self
    }

    pub fn with_failure_count(mut self, count: u8) -> Self {
        self.failure_count = count;
        self
    }

    pub fn with_health(mut self, health: Health) -> Self {
        self.health = health;
        self
    }

    pub fn with_budget(mut self, budget: Budget) -> Self {
        self.budget = budget;
        self
    }

    pub fn with_human_gate(mut self) -> Self {
        self.human_gate = true;
        self
    }

    pub fn with_tier(mut self, tier: LifecycleTier) -> Self {
        self.next_tier = tier;
        self
    }
}

pub fn decide(input: &LifecycleInput) -> LifecycleDecision {
    if let Some(mode) = input.stop_mode {
        return LifecycleDecision::Stop { mode };
    }
    if !is_repo_scope(&input.repo) {
        return LifecycleDecision::Reject(LifecycleReject::InvalidScope);
    }
    if input
        .claim_repo
        .as_deref()
        .is_some_and(|claim_repo| claim_repo != input.repo)
    {
        return LifecycleDecision::Reject(LifecycleReject::CrossScopeClaim);
    }
    if input.lease_age_secs > STALE_LEASE_SECS {
        return LifecycleDecision::Reject(LifecycleReject::StaleLease);
    }
    if input.claim_terminal {
        return LifecycleDecision::Reject(LifecycleReject::TerminalClaim);
    }
    if !input.ownership_matches {
        return LifecycleDecision::Reject(LifecycleReject::OwnershipMismatch);
    }
    if input.failure_count >= ISSUE_FAILURE_CAP {
        return LifecycleDecision::Reject(LifecycleReject::FailureCap);
    }
    if input.human_gate {
        return LifecycleDecision::Park {
            reason: ParkReason::HumanGate,
        };
    }
    match input.health {
        Health::Continue => {}
        Health::Wait => {
            return LifecycleDecision::Park {
                reason: ParkReason::HealthWait,
            };
        }
        Health::Halt => {
            return LifecycleDecision::Park {
                reason: ParkReason::HealthHalt,
            };
        }
    }
    match input.budget {
        Budget::WithinCap => {}
        Budget::SoftCap => {
            return LifecycleDecision::Park {
                reason: ParkReason::BudgetSoftCap,
            };
        }
        Budget::HardCap => {
            return LifecycleDecision::Park {
                reason: ParkReason::BudgetHardCap,
            };
        }
    }
    match input.next_tier {
        LifecycleTier::Idle => LifecycleDecision::Park {
            reason: ParkReason::IdleRescan,
        },
        tier => LifecycleDecision::Run { tier },
    }
}

fn is_repo_scope(repo: &str) -> bool {
    let mut parts = repo.split('/');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(owner), Some(name), None) if !owner.is_empty() && !name.is_empty()
    )
}
