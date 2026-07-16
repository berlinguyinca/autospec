mod codec;

pub const NO_WORK_SCHEMA: u64 = 1;
pub const IDEATION_DRY_PASS_THRESHOLD: u64 = 2;
pub const IDEATION_CANDIDATE_LIMIT: u64 = 5;
pub const EVIDENCE_DIGEST_HEX_LENGTH: usize = 64;
pub const IDEATION_SCORE_FIELDS: [&str; 4] = ["impact", "importance", "risk", "effort"];
pub const IDEATION_QUESTIONS: [&str; 6] = [
    "What features are missing?",
    "What can we do here?",
    "Find 5 new features.",
    "Rank them by impact and importance.",
    "Which are safe to implement autonomously now?",
    "Which need a planning/spec issue first?",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoWorkTier {
    Tier1,
    Tier1_5,
    Tier2,
    Tier3,
    Tier4,
}

impl NoWorkTier {
    pub const ALL: [Self; 5] = [
        Self::Tier1,
        Self::Tier1_5,
        Self::Tier2,
        Self::Tier3,
        Self::Tier4,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tier1 => "tier1",
            Self::Tier1_5 => "tier1_5",
            Self::Tier2 => "tier2",
            Self::Tier3 => "tier3",
            Self::Tier4 => "tier4",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "tier1" => Ok(Self::Tier1),
            "tier1_5" => Ok(Self::Tier1_5),
            "tier2" => Ok(Self::Tier2),
            "tier3" => Ok(Self::Tier3),
            "tier4" => Ok(Self::Tier4),
            _ => Err(format!("unknown no-work tier: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DryReason {
    NoProposalsGenerated,
    Deduplicated,
    VerificationRejected,
    RoiFiltered,
    AlreadyImplemented,
}

impl DryReason {
    pub const ALL: [Self; 5] = [
        Self::NoProposalsGenerated,
        Self::Deduplicated,
        Self::VerificationRejected,
        Self::RoiFiltered,
        Self::AlreadyImplemented,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoProposalsGenerated => "no_proposals_generated",
            Self::Deduplicated => "deduplicated",
            Self::VerificationRejected => "verification_rejected",
            Self::RoiFiltered => "roi_filtered",
            Self::AlreadyImplemented => "already_implemented",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "no_proposals_generated" => Ok(Self::NoProposalsGenerated),
            "deduplicated" => Ok(Self::Deduplicated),
            "verification_rejected" => Ok(Self::VerificationRejected),
            "roi_filtered" => Ok(Self::RoiFiltered),
            "already_implemented" => Ok(Self::AlreadyImplemented),
            _ => Err(format!("unknown no-work dry reason: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TierOutcome {
    Produced { count: u64 },
    Dry { reason: DryReason },
    NotRun { reason: String },
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoWorkObservation {
    pub repo: String,
    pub pass_id: u64,
    pub evidence_digest: String,
    pub tiers: Vec<(NoWorkTier, TierOutcome)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierEvidence {
    pub digest: String,
    pub reference: String,
}

impl NoWorkObservation {
    pub fn evidence_for(&self, tier: NoWorkTier) -> TierEvidence {
        TierEvidence {
            digest: self.evidence_digest.clone(),
            reference: evidence_reference(self.pass_id, tier),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoWorkDecision {
    IdleRescan,
    RequestIdeation,
}

impl NoWorkDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::IdleRescan => "idle_rescan",
            Self::RequestIdeation => "ideation_backlog_refresh_required",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "idle_rescan" => Ok(Self::IdleRescan),
            "ideation_backlog_refresh_required" => Ok(Self::RequestIdeation),
            _ => Err(format!("unknown no-work decision: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdeationRequest {
    pub candidate_limit: u64,
    pub disposition: &'static str,
    pub remote_mutation: &'static str,
    pub score_fields: [&'static str; 4],
    pub questions: [&'static str; 6],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoWorkState {
    repo: String,
    pass_id: u64,
    evidence_digest: String,
    consecutive_dry_passes: u64,
    tiers: Vec<(NoWorkTier, TierOutcome)>,
    dry_pass_history: Vec<NoWorkObservation>,
}

impl NoWorkState {
    pub fn record(
        previous: Option<&NoWorkState>,
        observation: NoWorkObservation,
    ) -> Result<Self, String> {
        validate_observation(&observation)?;

        if let Some(previous) = previous {
            previous.validate()?;
            if previous.repo != observation.repo {
                return Err(
                    "no-work observation repository does not match previous state".to_string(),
                );
            }
            if observation.pass_id == previous.pass_id {
                if previous.matches(&observation) {
                    return Ok(previous.clone());
                }
                return Err("conflicting duplicate no-work pass".to_string());
            }
            if observation.pass_id < previous.pass_id {
                return Err("stale pass: no-work observation".to_string());
            }
            let next_pass = previous
                .pass_id
                .checked_add(1)
                .ok_or_else(|| "no-work pass sequence overflow".to_string())?;
            if observation.pass_id != next_pass {
                return Err("no-work pass must be exactly next in sequence".to_string());
            }
        }

        let complete_dry = is_complete_dry(&observation.tiers);
        let consecutive_dry_passes = if complete_dry {
            previous.map_or(Ok(1), |state| {
                state
                    .consecutive_dry_passes
                    .checked_add(1)
                    .ok_or_else(|| "no-work dry-pass counter overflow".to_string())
            })?
        } else {
            0
        };
        let dry_pass_history = if complete_dry {
            let mut history =
                previous.map_or_else(Vec::new, |state| state.dry_pass_history.clone());
            history.push(observation.clone());
            let retained = IDEATION_DRY_PASS_THRESHOLD as usize;
            if history.len() > retained {
                history.drain(..history.len() - retained);
            }
            history
        } else {
            Vec::new()
        };

        Ok(Self {
            repo: observation.repo,
            pass_id: observation.pass_id,
            evidence_digest: observation.evidence_digest,
            consecutive_dry_passes,
            tiers: observation.tiers,
            dry_pass_history,
        })
    }

    pub fn repo(&self) -> &str {
        &self.repo
    }

    pub fn pass_id(&self) -> u64 {
        self.pass_id
    }

    pub fn tiers(&self) -> &[(NoWorkTier, TierOutcome)] {
        &self.tiers
    }

    pub fn consecutive_dry_passes(&self) -> u64 {
        self.consecutive_dry_passes
    }

    pub fn dry_pass_history(&self) -> &[NoWorkObservation] {
        &self.dry_pass_history
    }

    pub fn decision(&self) -> NoWorkDecision {
        if self.consecutive_dry_passes >= IDEATION_DRY_PASS_THRESHOLD {
            NoWorkDecision::RequestIdeation
        } else {
            NoWorkDecision::IdleRescan
        }
    }

    pub fn candidate_limit(&self) -> u64 {
        IDEATION_CANDIDATE_LIMIT
    }

    pub fn ideation_request(&self) -> Option<IdeationRequest> {
        (self.decision() == NoWorkDecision::RequestIdeation).then_some(IdeationRequest {
            candidate_limit: IDEATION_CANDIDATE_LIMIT,
            disposition: "planning_only",
            remote_mutation: "none",
            score_fields: IDEATION_SCORE_FIELDS,
            questions: IDEATION_QUESTIONS,
        })
    }

    pub fn to_json(&self) -> String {
        codec::state_json(self)
    }

    pub fn parse_json(input: &str) -> Result<Self, String> {
        codec::parse_state(input)
    }

    fn matches(&self, observation: &NoWorkObservation) -> bool {
        self.repo == observation.repo
            && self.pass_id == observation.pass_id
            && self.evidence_digest == observation.evidence_digest
            && self.tiers == observation.tiers
    }

    fn validate(&self) -> Result<(), String> {
        let observation = NoWorkObservation {
            repo: self.repo.clone(),
            pass_id: self.pass_id,
            evidence_digest: self.evidence_digest.clone(),
            tiers: self.tiers.clone(),
        };
        validate_observation(&observation)?;
        if is_complete_dry(&self.tiers) {
            if self.consecutive_dry_passes == 0 {
                return Err(
                    "complete dry no-work state must have a positive dry-pass count".to_string(),
                );
            }
            let expected_history_len =
                self.consecutive_dry_passes.min(IDEATION_DRY_PASS_THRESHOLD) as usize;
            if self.dry_pass_history.len() != expected_history_len {
                return Err("no-work dry-pass history does not match its bounded count".to_string());
            }
            if self.dry_pass_history.last() != Some(&observation) {
                return Err(
                    "no-work dry-pass history must end with the current observation".to_string(),
                );
            }
            for source in &self.dry_pass_history {
                validate_observation(source)?;
                if source.repo != self.repo || !is_complete_dry(&source.tiers) {
                    return Err(
                        "no-work dry-pass history must contain same-repository dry observations"
                            .to_string(),
                    );
                }
            }
            if self
                .dry_pass_history
                .windows(2)
                .any(|window| window[1].pass_id != window[0].pass_id + 1)
            {
                return Err(
                    "no-work dry-pass history must contain consecutive pass IDs".to_string()
                );
            }
        } else if self.consecutive_dry_passes != 0 {
            return Err("non-dry no-work state must reset the dry-pass count".to_string());
        } else if !self.dry_pass_history.is_empty() {
            return Err("non-dry no-work state must clear dry-pass history".to_string());
        }
        Ok(())
    }
}

fn validate_observation(observation: &NoWorkObservation) -> Result<(), String> {
    if observation.repo.trim().is_empty() {
        return Err("no-work observation repository must not be empty".to_string());
    }
    if observation.pass_id == 0 {
        return Err("no-work pass_id must be positive".to_string());
    }
    if !is_sealed_digest(&observation.evidence_digest) {
        return Err(format!(
            "no-work evidence digest must be {EVIDENCE_DIGEST_HEX_LENGTH} lower-case hexadecimal characters"
        ));
    }
    for (index, (tier, outcome)) in observation.tiers.iter().enumerate() {
        let expected = NoWorkTier::ALL.get(index).copied();
        if expected != Some(*tier) {
            if observation.tiers[..index]
                .iter()
                .any(|(seen, _)| seen == tier)
            {
                return Err(format!("duplicate tier: no-work {}", tier.as_str()));
            }
            if NoWorkTier::ALL.contains(tier) {
                return expected.map_or_else(
                    || {
                        Err(format!(
                            "no-work observation must contain exactly {} tiers",
                            NoWorkTier::ALL.len()
                        ))
                    },
                    |expected| {
                        Err(format!(
                            "no-work tiers must be ordered; expected {}",
                            expected.as_str()
                        ))
                    },
                );
            }
            return Err(format!("unknown no-work tier: {}", tier.as_str()));
        }
        validate_outcome(outcome)?;
    }
    if let Some(missing) = NoWorkTier::ALL.get(observation.tiers.len()) {
        return Err(format!("missing tier: no-work {}", missing.as_str()));
    }
    Ok(())
}

fn validate_outcome(outcome: &TierOutcome) -> Result<(), String> {
    match outcome {
        TierOutcome::Produced { count } if *count == 0 => {
            Err("no-work produced count must be positive".to_string())
        }
        TierOutcome::NotRun { reason } | TierOutcome::Failed { reason }
            if reason.trim().is_empty() =>
        {
            Err("no-work outcome reason must not be empty".to_string())
        }
        _ => Ok(()),
    }
}

fn is_complete_dry(tiers: &[(NoWorkTier, TierOutcome)]) -> bool {
    tiers
        .iter()
        .all(|(_, outcome)| matches!(outcome, TierOutcome::Dry { .. }))
}

pub(super) fn is_sealed_digest(value: &str) -> bool {
    value.len() == EVIDENCE_DIGEST_HEX_LENGTH
        && value.bytes().all(|byte| {
            byte.is_ascii_digit() || (byte.is_ascii_lowercase() && byte.is_ascii_hexdigit())
        })
}

pub(super) fn evidence_reference(pass_id: u64, tier: NoWorkTier) -> String {
    format!("waterfall/{pass_id}/{}.json", tier.as_str())
}
