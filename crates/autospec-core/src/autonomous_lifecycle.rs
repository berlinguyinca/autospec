use std::collections::BTreeMap;

use crate::state::json::{JsonParser, JsonValue};

pub const STALE_LEASE_SECS: u64 = 300;
pub const ABANDONED_LEASE_SECS: u64 = 10_800;
pub const ISSUE_FAILURE_CAP: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConductorLeaseInput {
    claimed: bool,
    heartbeat_age_secs: Option<u64>,
    same_host_pid_dead: bool,
}

impl ConductorLeaseInput {
    pub fn claimed(heartbeat_age_secs: u64, same_host_pid_dead: bool) -> Self {
        Self {
            claimed: true,
            heartbeat_age_secs: Some(heartbeat_age_secs),
            same_host_pid_dead,
        }
    }

    pub fn running(heartbeat_age_secs: u64, same_host_pid_dead: bool) -> Self {
        Self {
            claimed: false,
            heartbeat_age_secs: Some(heartbeat_age_secs),
            same_host_pid_dead,
        }
    }

    pub fn missing_heartbeat(claimed: bool, same_host_pid_dead: bool) -> Self {
        Self {
            claimed,
            heartbeat_age_secs: None,
            same_host_pid_dead,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConductorLeaseReclaim {
    DeadSameHostPid,
    MissingHeartbeat,
    Abandoned,
    ClaimedExpired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConductorLeaseDecision {
    Held,
    Reclaim(ConductorLeaseReclaim),
}

pub fn decide_conductor_lease(input: ConductorLeaseInput) -> ConductorLeaseDecision {
    // A proven-dead owner on this host releases the lease whatever state it is in. The
    // `claimed` state used to be excluded, which parked the repository for STALE_LEASE_SECS
    // (5 minutes) whenever a conductor died inside the claim window -- and the store already
    // disagreed: `release_terminated_owner` treats `status == "claimed"` with a dead
    // `lock_pid` as an abandoned claim it may release. The owner's harness may still be
    // running; the replacement adopts it rather than starting a second one, which is what
    // `foreground_repeated_restart_observes_one_live_harness_until_merge` asserts end to end.
    //
    // `same_host_pid_dead` is only ever true for a pid on the recorded host that we proved
    // absent or terminated, so an unknown or foreign owner still reads as live.
    if input.same_host_pid_dead {
        return ConductorLeaseDecision::Reclaim(ConductorLeaseReclaim::DeadSameHostPid);
    }
    let Some(age) = input.heartbeat_age_secs else {
        return ConductorLeaseDecision::Reclaim(ConductorLeaseReclaim::MissingHeartbeat);
    };
    if age >= ABANDONED_LEASE_SECS {
        return ConductorLeaseDecision::Reclaim(ConductorLeaseReclaim::Abandoned);
    }
    if input.claimed && age >= STALE_LEASE_SECS {
        return ConductorLeaseDecision::Reclaim(ConductorLeaseReclaim::ClaimedExpired);
    }
    ConductorLeaseDecision::Held
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityInput {
    usage: u64,
    usage_cap: u64,
    issue_count: u64,
    issue_cap: u64,
}

impl CapacityInput {
    pub fn new(usage: u64, usage_cap: u64, issue_count: u64, issue_cap: u64) -> Self {
        Self {
            usage,
            usage_cap,
            issue_count,
            issue_cap,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityDecision {
    WithinCap,
    UsageCap,
    IssueCap,
}

pub fn decide_capacity(input: CapacityInput) -> CapacityDecision {
    if input.usage_cap > 0 && input.usage >= input.usage_cap {
        CapacityDecision::UsageCap
    } else if input.issue_cap > 0 && input.issue_count >= input.issue_cap {
        CapacityDecision::IssueCap
    } else {
        CapacityDecision::WithinCap
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryScope {
    owner: String,
    repository: String,
}

impl RepositoryScope {
    pub fn as_str(&self) -> String {
        format!("{}/{}", self.owner, self.repository)
    }
}

impl TryFrom<&str> for RepositoryScope {
    type Error = &'static str;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let mut parts = value.split('/');
        let owner = parts
            .next()
            .ok_or("repository scope requires owner/repository")?;
        let repository = parts
            .next()
            .ok_or("repository scope requires owner/repository")?;
        if parts.next().is_some() || !is_scope_segment(owner) || !is_scope_segment(repository) {
            return Err("repository scope requires canonical owner/repository");
        }
        Ok(Self {
            owner: owner.to_string(),
            repository: repository.to_string(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IssueNumber(u64);

impl IssueNumber {
    pub fn new(value: u64) -> Option<Self> {
        (value > 0).then_some(Self(value))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerId(String);

impl TryFrom<&str> for WorkerId {
    type Error = &'static str;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        is_identifier(value)
            .then(|| Self(value.to_string()))
            .ok_or("worker id must be a non-empty token")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimBranch(String);

impl TryFrom<&str> for ClaimBranch {
    type Error = &'static str;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        is_identifier(value)
            .then(|| Self(value.to_string()))
            .ok_or("claim branch must be a non-empty token")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseFreshness {
    Fresh,
    Stale,
    Abandoned,
}

impl LeaseFreshness {
    pub fn from_age_secs(age_secs: u64) -> Self {
        if age_secs > ABANDONED_LEASE_SECS {
            Self::Abandoned
        } else if age_secs > STALE_LEASE_SECS {
            Self::Stale
        } else {
            Self::Fresh
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimState {
    Active,
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimContext {
    scope: RepositoryScope,
    issue: IssueNumber,
    worker: WorkerId,
    branch: ClaimBranch,
    lease: LeaseFreshness,
    state: ClaimState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimEvidence {
    Missing,
    Observed(ClaimContext),
    Malformed,
}

impl ClaimContext {
    pub fn active(
        scope: RepositoryScope,
        issue: IssueNumber,
        worker: WorkerId,
        branch: ClaimBranch,
        lease: LeaseFreshness,
    ) -> Self {
        Self {
            scope,
            issue,
            worker,
            branch,
            lease,
            state: ClaimState::Active,
        }
    }

    pub fn terminal(
        scope: RepositoryScope,
        issue: IssueNumber,
        worker: WorkerId,
        branch: ClaimBranch,
    ) -> Self {
        Self {
            scope,
            issue,
            worker,
            branch,
            lease: LeaseFreshness::Fresh,
            state: ClaimState::Terminal,
        }
    }

    pub fn lease_freshness(&self) -> LeaseFreshness {
        self.lease
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryClass {
    Initial,
    Retrying { failures: u8 },
}

impl RetryClass {
    fn failures(self) -> u8 {
        match self {
            Self::Initial => 0,
            Self::Retrying { failures } => failures,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleTransition {
    Start,
    Restart,
    Foreground,
}

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
    MalformedClaim,
    CrossScopeClaim,
    StaleLease,
    AbandonedLease,
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
    NoSteering,
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
    scope: Option<RepositoryScope>,
    transition: LifecycleTransition,
    claim: ClaimEvidence,
    expected_claim: Option<(WorkerId, ClaimBranch)>,
    stop_mode: Option<StopMode>,
    health: Health,
    budget: Budget,
    human_gate: bool,
    lease: LeaseFreshness,
    retry: RetryClass,
    next_tier: LifecycleTier,
}

impl LifecycleInput {
    pub fn ready(repo: impl AsRef<str>) -> Self {
        Self {
            scope: RepositoryScope::try_from(repo.as_ref()).ok(),
            transition: LifecycleTransition::Start,
            claim: ClaimEvidence::Missing,
            expected_claim: None,
            stop_mode: None,
            health: Health::Continue,
            budget: Budget::WithinCap,
            human_gate: false,
            lease: LeaseFreshness::Fresh,
            retry: RetryClass::Initial,
            next_tier: LifecycleTier::Tier1,
        }
    }

    pub fn from_scope(scope: RepositoryScope) -> Self {
        let mut input = Self::ready(scope.as_str());
        input.scope = Some(scope);
        input
    }

    pub fn with_transition(mut self, transition: LifecycleTransition) -> Self {
        self.transition = transition;
        self
    }

    pub fn with_stop(mut self, mode: StopMode) -> Self {
        self.stop_mode = Some(mode);
        self
    }

    pub fn with_claim(mut self, claim: ClaimContext) -> Self {
        self.lease = claim.lease;
        self.claim = ClaimEvidence::Observed(claim);
        self
    }

    pub fn with_claim_evidence(mut self, claim: ClaimEvidence) -> Self {
        if let ClaimEvidence::Observed(context) = &claim {
            self.lease = context.lease;
        }
        self.claim = claim;
        self
    }

    pub fn with_expected_claim(mut self, worker: WorkerId, branch: ClaimBranch) -> Self {
        self.expected_claim = Some((worker, branch));
        self
    }

    pub fn with_lease_age_secs(mut self, value: u64) -> Self {
        self.lease = LeaseFreshness::from_age_secs(value);
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

    pub fn with_retry(mut self, retry: RetryClass) -> Self {
        self.retry = retry;
        self
    }

    pub fn with_failure_count(self, count: u8) -> Self {
        self.with_retry(RetryClass::Retrying { failures: count })
    }

    pub fn with_tier(mut self, tier: LifecycleTier) -> Self {
        self.next_tier = tier;
        self
    }
}

pub fn decide(input: &LifecycleInput) -> LifecycleDecision {
    if let Some(mode) = input.stop_mode {
        if input.transition != LifecycleTransition::Restart {
            return LifecycleDecision::Stop { mode };
        }
    }
    let Some(scope) = &input.scope else {
        return LifecycleDecision::Reject(LifecycleReject::InvalidScope);
    };
    match &input.claim {
        ClaimEvidence::Malformed => {
            return LifecycleDecision::Reject(LifecycleReject::MalformedClaim);
        }
        ClaimEvidence::Observed(claim) => {
            if claim.scope != *scope {
                return LifecycleDecision::Reject(LifecycleReject::CrossScopeClaim);
            }
            match claim.lease {
                LeaseFreshness::Fresh => {}
                LeaseFreshness::Stale => {
                    return LifecycleDecision::Reject(LifecycleReject::StaleLease);
                }
                LeaseFreshness::Abandoned => {
                    return LifecycleDecision::Reject(LifecycleReject::AbandonedLease);
                }
            }
            if claim.state == ClaimState::Terminal {
                return LifecycleDecision::Reject(LifecycleReject::TerminalClaim);
            }
            if input
                .expected_claim
                .as_ref()
                .is_some_and(|expected| expected.0 != claim.worker || expected.1 != claim.branch)
            {
                return LifecycleDecision::Reject(LifecycleReject::OwnershipMismatch);
            }
        }
        ClaimEvidence::Missing => match input.lease {
            LeaseFreshness::Fresh => {}
            LeaseFreshness::Stale => {
                return LifecycleDecision::Reject(LifecycleReject::StaleLease);
            }
            LeaseFreshness::Abandoned => {
                return LifecycleDecision::Reject(LifecycleReject::AbandonedLease);
            }
        },
    }
    if input.retry.failures() >= ISSUE_FAILURE_CAP {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleRecord {
    pub scope: RepositoryScope,
    pub decision: LifecycleDecision,
}

impl LifecycleRecord {
    pub fn parse_json(input: &str) -> Result<Self, String> {
        let mut record = JsonParser::new(input)
            .parse()?
            .into_object("lifecycle record")?;
        let version = take_value(&mut record, "version")?.into_number("lifecycle version")?;
        if version != 1 {
            return Err(format!("unsupported lifecycle record version: {version}"));
        }
        let scope = RepositoryScope::try_from(
            take_value(&mut record, "repo")?
                .into_string("lifecycle repo")?
                .as_str(),
        )
        .map_err(str::to_string)?;
        let result = take_value(&mut record, "result")?.into_object("lifecycle result")?;
        if !record.is_empty() {
            return Err("lifecycle record contains unexpected fields".to_string());
        }
        Ok(Self {
            scope,
            decision: parse_decision(result)?,
        })
    }
}

fn parse_decision(mut result: BTreeMap<String, JsonValue>) -> Result<LifecycleDecision, String> {
    let decision = take_value(&mut result, "decision")?.into_string("lifecycle decision")?;
    let parsed = match decision.as_str() {
        "run" => LifecycleDecision::Run {
            tier: parse_tier(take_value(&mut result, "tier")?.into_string("lifecycle tier")?)?,
        },
        "stop" => LifecycleDecision::Stop {
            mode: parse_stop_mode(
                take_value(&mut result, "mode")?.into_string("lifecycle stop mode")?,
            )?,
        },
        "park" => LifecycleDecision::Park {
            reason: parse_park_reason(
                take_value(&mut result, "reason")?.into_string("lifecycle park reason")?,
            )?,
        },
        "reject" => LifecycleDecision::Reject(parse_reject(
            take_value(&mut result, "reason")?.into_string("lifecycle reject reason")?,
        )?),
        _ => return Err("unknown lifecycle decision".to_string()),
    };
    if !result.is_empty() {
        return Err("lifecycle result contains unexpected fields".to_string());
    }
    Ok(parsed)
}

fn take_value(object: &mut BTreeMap<String, JsonValue>, key: &str) -> Result<JsonValue, String> {
    object
        .remove(key)
        .ok_or_else(|| format!("lifecycle record requires {key}"))
}

fn parse_tier(value: String) -> Result<LifecycleTier, String> {
    match value.as_str() {
        "1" => Ok(LifecycleTier::Tier1),
        "1.5" => Ok(LifecycleTier::Tier15),
        "2" => Ok(LifecycleTier::Tier2),
        "3" => Ok(LifecycleTier::Tier3),
        "4" => Ok(LifecycleTier::Tier4),
        "5" => Ok(LifecycleTier::Tier5),
        "6" => Ok(LifecycleTier::Tier6),
        "7" => Ok(LifecycleTier::Tier7),
        _ => Err("unknown lifecycle tier".to_string()),
    }
}

fn parse_stop_mode(value: String) -> Result<StopMode, String> {
    match value.as_str() {
        "graceful" => Ok(StopMode::Graceful),
        "immediate" => Ok(StopMode::Immediate),
        _ => Err("unknown lifecycle stop mode".to_string()),
    }
}

fn parse_park_reason(value: String) -> Result<ParkReason, String> {
    match value.as_str() {
        "human_gate" => Ok(ParkReason::HumanGate),
        "health_wait" => Ok(ParkReason::HealthWait),
        "health_halt" => Ok(ParkReason::HealthHalt),
        "budget_soft_cap" => Ok(ParkReason::BudgetSoftCap),
        "budget_hard_cap" => Ok(ParkReason::BudgetHardCap),
        "idle_rescan" => Ok(ParkReason::IdleRescan),
        "no-steering" => Ok(ParkReason::NoSteering),
        _ => Err("unknown lifecycle park reason".to_string()),
    }
}

fn parse_reject(value: String) -> Result<LifecycleReject, String> {
    match value.as_str() {
        "invalid_scope" => Ok(LifecycleReject::InvalidScope),
        "malformed_claim" => Ok(LifecycleReject::MalformedClaim),
        "cross_scope_claim" => Ok(LifecycleReject::CrossScopeClaim),
        "stale_lease" => Ok(LifecycleReject::StaleLease),
        "abandoned_lease" => Ok(LifecycleReject::AbandonedLease),
        "terminal_claim" => Ok(LifecycleReject::TerminalClaim),
        "ownership_mismatch" => Ok(LifecycleReject::OwnershipMismatch),
        "failure_cap" => Ok(LifecycleReject::FailureCap),
        _ => Err("unknown lifecycle reject reason".to_string()),
    }
}

fn is_scope_segment(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && !value.chars().any(char::is_whitespace)
        && !value.chars().any(char::is_control)
}
