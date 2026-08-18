mod codec;
mod sha256;

use std::io::{self, Read};

use sha2::{Digest, Sha256};

use super::no_work::{is_sealed_digest, DryReason, NoWorkTier};

pub const WATERFALL_RECEIPT_SCHEMA: u64 = 1;
pub const WATERFALL_STATE_SCHEMA: u64 = 1;

pub fn sha256_hex(input: &[u8]) -> String {
    sha256::hex(input)
}

pub fn sha256_reader_hex(mut input: impl Read) -> io::Result<String> {
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(test)]
mod reader_digest_tests {
    use super::sha256_reader_hex;

    #[test]
    fn streaming_sha256_matches_the_standard_abc_vector() {
        assert_eq!(
            sha256_reader_hex(&b"abc"[..]).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunnelCounts {
    pub observed: u64,
    pub deduplicated: u64,
    pub verified: u64,
    pub roi_approved: u64,
    pub ranked: u64,
}

impl FunnelCounts {
    pub fn new(
        observed: u64,
        deduplicated: u64,
        verified: u64,
        roi_approved: u64,
        ranked: u64,
    ) -> Result<Self, String> {
        let counts = Self {
            observed,
            deduplicated,
            verified,
            roi_approved,
            ranked,
        };
        counts.validate()?;
        Ok(counts)
    }

    fn validate(&self) -> Result<(), String> {
        if self.deduplicated > self.observed
            || self.verified > self.deduplicated
            || self.roi_approved > self.verified
            || self.ranked > self.roi_approved
        {
            return Err("waterfall funnel counts must be monotonic".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedEvidence {
    pub reference: String,
    pub digest: String,
}

impl SealedEvidence {
    pub fn new(reference: impl Into<String>, digest: impl Into<String>) -> Result<Self, String> {
        let evidence = Self {
            reference: reference.into(),
            digest: digest.into(),
        };
        evidence.validate()?;
        Ok(evidence)
    }

    fn validate(&self) -> Result<(), String> {
        if !is_sealed_reference(&self.reference) {
            return Err("waterfall evidence reference is not sealed".to_string());
        }
        if !is_sealed_digest(&self.digest) {
            return Err("waterfall evidence digest is not sealed".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TierStatus {
    Exhausted { reason: DryReason },
    Produced { count: u64 },
    Failed { reason: String },
    Blocked { reason: String },
    NotRun { reason: String },
}

impl TierStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Exhausted { .. } => "exhausted",
            Self::Produced { .. } => "produced",
            Self::Failed { .. } => "failed",
            Self::Blocked { .. } => "blocked",
            Self::NotRun { .. } => "not_run",
        }
    }

    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Produced { count } if *count == 0 => {
                Err("waterfall produced count must be positive".to_string())
            }
            Self::Failed { reason } | Self::Blocked { reason } | Self::NotRun { reason }
                if reason.trim().is_empty() =>
            {
                Err("waterfall status reason must not be empty".to_string())
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierReceipt {
    repo: String,
    pass_id: u64,
    tier: NoWorkTier,
    producer_version: String,
    started_at: u64,
    completed_at: u64,
    status: TierStatus,
    funnel: FunnelCounts,
    evidence: Vec<SealedEvidence>,
    digest: String,
}

impl TierReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo: impl Into<String>,
        pass_id: u64,
        tier: NoWorkTier,
        producer_version: impl Into<String>,
        started_at: u64,
        completed_at: u64,
        status: TierStatus,
        funnel: FunnelCounts,
        evidence: Vec<SealedEvidence>,
    ) -> Result<Self, String> {
        let mut receipt = Self {
            repo: repo.into(),
            pass_id,
            tier,
            producer_version: producer_version.into(),
            started_at,
            completed_at,
            status,
            funnel,
            evidence,
            digest: String::new(),
        };
        receipt.validate_fields()?;
        receipt.digest = codec::receipt_digest(&receipt);
        Ok(receipt)
    }

    pub fn repo(&self) -> &str {
        &self.repo
    }

    pub fn pass_id(&self) -> u64 {
        self.pass_id
    }

    pub fn tier(&self) -> NoWorkTier {
        self.tier
    }

    pub fn status(&self) -> &TierStatus {
        &self.status
    }

    pub fn producer_version(&self) -> &str {
        &self.producer_version
    }

    pub fn funnel(&self) -> &FunnelCounts {
        &self.funnel
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn evidence(&self) -> &[SealedEvidence] {
        &self.evidence
    }

    pub fn reference(&self) -> String {
        receipt_reference(self.pass_id, self.tier)
    }

    pub fn to_json(&self) -> String {
        codec::receipt_json(self)
    }

    pub fn parse_json(
        input: &str,
        expected_repo: &str,
        expected_pass_id: u64,
        expected_tier: NoWorkTier,
    ) -> Result<Self, String> {
        codec::parse_receipt(input, expected_repo, expected_pass_id, expected_tier)
    }

    fn validate(&self) -> Result<(), String> {
        self.validate_fields()?;
        if !is_sealed_digest(&self.digest) {
            return Err("waterfall receipt digest is not sealed".to_string());
        }
        if self.digest != codec::receipt_digest(self) {
            return Err("waterfall receipt digest does not match sealed payload".to_string());
        }
        Ok(())
    }

    fn validate_fields(&self) -> Result<(), String> {
        validate_repo(&self.repo)?;
        if self.pass_id == 0 {
            return Err("waterfall receipt pass id must be positive".to_string());
        }
        if self.producer_version.trim().is_empty() {
            return Err("waterfall receipt producer version must not be empty".to_string());
        }
        if self.completed_at < self.started_at {
            return Err("waterfall receipt completion precedes start".to_string());
        }
        self.status.validate()?;
        self.funnel.validate()?;
        if self.evidence.is_empty() {
            return Err("waterfall receipt must retain evidence".to_string());
        }
        for evidence in &self.evidence {
            evidence.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedReceipt {
    pub tier: NoWorkTier,
    pub digest: String,
    pub reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaterfallState {
    repo: String,
    next_pass_id: u64,
    current_tier: NoWorkTier,
    completed_receipts: Vec<CompletedReceipt>,
}

impl WaterfallState {
    pub fn new(
        repo: impl Into<String>,
        next_pass_id: u64,
        current_tier: NoWorkTier,
    ) -> Result<Self, String> {
        let state = Self {
            repo: repo.into(),
            next_pass_id,
            current_tier,
            completed_receipts: Vec::new(),
        };
        state.validate()?;
        Ok(state)
    }

    pub fn repo(&self) -> &str {
        &self.repo
    }

    pub fn next_pass_id(&self) -> u64 {
        self.next_pass_id
    }

    pub fn current_tier(&self) -> NoWorkTier {
        self.current_tier
    }

    pub fn completed_receipts(&self) -> &[CompletedReceipt] {
        &self.completed_receipts
    }

    pub fn record_receipt(mut self, receipt: &TierReceipt) -> Result<Self, String> {
        receipt.validate()?;
        if receipt.repo != self.repo {
            return Err("waterfall receipt repository does not match state".to_string());
        }
        if receipt.pass_id != self.next_pass_id {
            return Err("waterfall receipt pass does not match state cursor".to_string());
        }
        if receipt.tier != self.current_tier {
            return Err("waterfall receipt tier does not match state cursor".to_string());
        }
        if receipt.tier == NoWorkTier::Tier4 && !tier4_rollover_receipt(receipt) {
            return Err("Tier 4 receipt cannot advance the waterfall cursor".to_string());
        }
        if self.current_tier == NoWorkTier::Tier1 && !self.completed_receipts.is_empty() {
            self.completed_receipts.clear();
        }
        self.completed_receipts.push(CompletedReceipt {
            tier: receipt.tier,
            digest: receipt.digest.clone(),
            reference: receipt.reference(),
        });
        match next_tier(self.current_tier) {
            Some(tier) => self.current_tier = tier,
            None => {
                self.next_pass_id = self
                    .next_pass_id
                    .checked_add(1)
                    .ok_or_else(|| "waterfall pass counter overflow".to_string())?;
                self.current_tier = NoWorkTier::Tier1;
            }
        }
        self.validate()?;
        Ok(self)
    }

    pub fn to_json(&self) -> String {
        codec::state_json(self)
    }

    pub fn parse_json(input: &str, expected_repo: &str) -> Result<Self, String> {
        codec::parse_state(input, expected_repo)
    }

    fn validate(&self) -> Result<(), String> {
        validate_repo(&self.repo)?;
        if self.next_pass_id == 0 {
            return Err("waterfall state next pass id must be positive".to_string());
        }
        let retained_prior_pass = self.current_tier == NoWorkTier::Tier1 && self.next_pass_id > 1;
        let expected_completed = if retained_prior_pass {
            NoWorkTier::ALL.len()
        } else {
            tier_index(self.current_tier)
        };
        if self.completed_receipts.len() != expected_completed {
            return Err("waterfall state completed receipts do not match current tier".to_string());
        }
        let receipt_pass_id = if retained_prior_pass {
            self.next_pass_id - 1
        } else {
            self.next_pass_id
        };
        for (index, receipt) in self.completed_receipts.iter().enumerate() {
            let tier = NoWorkTier::ALL[index];
            if receipt.tier != tier {
                return Err("waterfall state completed receipts must be ordered".to_string());
            }
            if !is_sealed_digest(&receipt.digest) {
                return Err("waterfall state completed receipt digest is not sealed".to_string());
            }
            if receipt.reference != receipt_reference(receipt_pass_id, tier) {
                return Err("waterfall state receipt reference is not derived".to_string());
            }
        }
        Ok(())
    }
}

fn tier4_rollover_receipt(receipt: &TierReceipt) -> bool {
    matches!(
        receipt.status(),
        TierStatus::Exhausted {
            reason: DryReason::NoProposalsGenerated
                | DryReason::VerificationRejected
                | DryReason::RoiFiltered,
        }
    ) || (receipt.producer_version() == "rust-tier4-disabled-policy-v1"
        && matches!(
            receipt.status(),
            TierStatus::NotRun { reason } if reason == super::tier4::DISABLED_REASON
        ))
}

pub fn receipt_reference(pass_id: u64, tier: NoWorkTier) -> String {
    format!("waterfall/{pass_id}/{}.json", tier.as_str())
}

fn validate_repo(repo: &str) -> Result<(), String> {
    if repo.trim().is_empty() || repo.starts_with('/') || repo.contains("..") {
        return Err("waterfall repository scope is invalid".to_string());
    }
    Ok(())
}

fn is_sealed_reference(reference: &str) -> bool {
    !reference.is_empty()
        && !reference.starts_with('/')
        && reference
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn tier_index(tier: NoWorkTier) -> usize {
    NoWorkTier::ALL
        .iter()
        .position(|candidate| *candidate == tier)
        .expect("closed no-work tier has an index")
}

fn next_tier(tier: NoWorkTier) -> Option<NoWorkTier> {
    NoWorkTier::ALL.get(tier_index(tier) + 1).copied()
}
