use std::fs;
use std::io;
use std::path::Path;

use autospec_core::autonomous::no_work::NoWorkTier;
use autospec_core::autonomous::waterfall::{sha256_hex, SealedEvidence, TierReceipt, TierStatus};

use super::WaterfallStoreError;

#[derive(Debug, Clone, Copy)]
pub(in crate::commands::autonomous) enum Tier1EvidenceArtifact {
    ReadyPage,
    ReadFailure,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::commands::autonomous) enum Tier15EvidenceArtifact {
    Observation,
    ReadFailure,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum WaterfallEvidenceArtifact {
    Tier1(Tier1EvidenceArtifact),
    Tier15(Tier15EvidenceArtifact),
}

impl WaterfallEvidenceArtifact {
    fn tier(self) -> NoWorkTier {
        match self {
            Self::Tier1(_) => NoWorkTier::Tier1,
            Self::Tier15(_) => NoWorkTier::Tier1_5,
        }
    }

    fn file_name(self) -> &'static str {
        match self {
            Self::Tier1(Tier1EvidenceArtifact::ReadyPage) => "ready-page.json",
            Self::Tier1(Tier1EvidenceArtifact::ReadFailure)
            | Self::Tier15(Tier15EvidenceArtifact::ReadFailure) => "read-failure.json",
            Self::Tier15(Tier15EvidenceArtifact::Observation) => "observation.json",
        }
    }

    fn reference(self, pass_id: u64) -> Result<String, WaterfallStoreError> {
        if pass_id == 0 {
            return Err(WaterfallStoreError::InvalidReceipt(
                "waterfall evidence pass id must be positive".to_string(),
            ));
        }
        Ok(format!(
            "waterfall/{pass_id}/{}/{}",
            self.tier().as_str(),
            self.file_name()
        ))
    }
}

pub(super) fn persist(
    root: &Path,
    pass_id: u64,
    artifact: WaterfallEvidenceArtifact,
    contents: &str,
) -> Result<SealedEvidence, WaterfallStoreError> {
    let reference = artifact.reference(pass_id)?;
    let evidence = SealedEvidence::new(&reference, sha256_hex(contents.as_bytes()))
        .map_err(WaterfallStoreError::InvalidReceipt)?;
    let path = root.join(&reference);
    match fs::read_to_string(&path) {
        Ok(existing) if existing == contents => Ok(evidence),
        Ok(_) => Err(WaterfallStoreError::InvalidReceipt(
            "conflicting sealed waterfall evidence".to_string(),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            super::atomic_write(&path, contents)?;
            Ok(evidence)
        }
        Err(error) => Err(WaterfallStoreError::Diagnostic(format!(
            "cannot read waterfall evidence {}: {error}",
            path.display()
        ))),
    }
}

pub(super) fn verify(
    root: &Path,
    pass_id: u64,
    artifact: WaterfallEvidenceArtifact,
    receipt: &TierReceipt,
) -> Result<(), WaterfallStoreError> {
    let reference = artifact.reference(pass_id)?;
    let path = root.join(&reference);
    let contents = fs::read_to_string(&path).map_err(|error| {
        let message = if error.kind() == io::ErrorKind::NotFound {
            format!("missing sealed waterfall evidence {reference}")
        } else {
            format!("cannot read waterfall evidence {}: {error}", path.display())
        };
        WaterfallStoreError::InvalidReceipt(message)
    })?;
    let digest = sha256_hex(contents.as_bytes());
    let marker = format!("\"reference\":\"{reference}\",\"digest\":\"{digest}\"");
    if receipt.to_json().contains(&marker) {
        Ok(())
    } else {
        Err(WaterfallStoreError::InvalidReceipt(
            "sealed waterfall evidence does not match receipt".to_string(),
        ))
    }
}

pub(super) fn artifact_for_receipt(
    receipt: &TierReceipt,
) -> Result<WaterfallEvidenceArtifact, WaterfallStoreError> {
    match (receipt.tier(), receipt.status()) {
        (NoWorkTier::Tier1, TierStatus::Exhausted { .. }) => Ok(WaterfallEvidenceArtifact::Tier1(
            Tier1EvidenceArtifact::ReadyPage,
        )),
        (NoWorkTier::Tier1, TierStatus::Failed { .. }) => Ok(WaterfallEvidenceArtifact::Tier1(
            Tier1EvidenceArtifact::ReadFailure,
        )),
        (NoWorkTier::Tier1_5, TierStatus::Exhausted { .. } | TierStatus::Produced { .. }) => Ok(
            WaterfallEvidenceArtifact::Tier15(Tier15EvidenceArtifact::Observation),
        ),
        (NoWorkTier::Tier1_5, TierStatus::Failed { .. }) => Ok(WaterfallEvidenceArtifact::Tier15(
            Tier15EvidenceArtifact::ReadFailure,
        )),
        (tier, status) => Err(WaterfallStoreError::InvalidState(format!(
            "{} receipt has unexpected {} status",
            tier.as_str(),
            status.as_str()
        ))),
    }
}
