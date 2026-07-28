use std::fs;
use std::io;
use std::path::Path;

use autospec_core::autonomous::no_work::NoWorkTier;
use autospec_core::autonomous::waterfall::{sha256_hex, SealedEvidence, TierReceipt};

use super::WaterfallStoreError;

mod canonical;
mod tier1_15;
mod tier2;
mod tier3;
mod tier3_consistency;
mod tier3_shape;
mod tier4;
mod tier4_consistency;
mod tier4_shape;

pub(super) fn verify_tier2(
    root: &Path,
    pass_id: u64,
    receipt: &TierReceipt,
) -> Result<(), WaterfallStoreError> {
    tier2::verify_tier2(root, pass_id, receipt)
}

pub(super) fn verify_tier1(
    root: &Path,
    pass_id: u64,
    receipt: &TierReceipt,
) -> Result<(), WaterfallStoreError> {
    tier1_15::verify_tier1(root, pass_id, receipt)
}

pub(super) fn verify_tier15(
    root: &Path,
    pass_id: u64,
    receipt: &TierReceipt,
) -> Result<(), WaterfallStoreError> {
    tier1_15::verify_tier15(root, pass_id, receipt)
}

pub(super) fn verify_tier3(
    root: &Path,
    pass_id: u64,
    receipt: &TierReceipt,
) -> Result<(), WaterfallStoreError> {
    tier3::verify_tier3(root, pass_id, receipt)
}

pub(super) fn verify_tier4(
    root: &Path,
    pass_id: u64,
    receipt: &TierReceipt,
    expected_source_policy: Option<&autospec_core::autonomous::tier4::Tier4SourcePolicy>,
) -> Result<(), WaterfallStoreError> {
    tier4::verify_tier4(root, pass_id, receipt, expected_source_policy)
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::commands::autonomous) enum Tier2EvidenceArtifact {
    Policy,
    Collector,
    Generated,
    Dedup,
    Verification,
    RoiRank,
    Failure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::commands::autonomous) enum Tier3EvidenceArtifact {
    Policy,
    Architecture,
    Coverage,
    Debt,
    Findings,
    Failure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::commands::autonomous) enum Tier4EvidenceArtifact {
    Policy,
    SourcePolicy,
    Sources,
    Generated,
    Dedup,
    Verification,
    RoiRank,
    Failure,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum WaterfallEvidenceArtifact {
    Tier1(Tier1EvidenceArtifact),
    Tier15(Tier15EvidenceArtifact),
    Tier2(Tier2EvidenceArtifact),
    Tier3(Tier3EvidenceArtifact),
    Tier4(Tier4EvidenceArtifact),
}

impl WaterfallEvidenceArtifact {
    fn tier(self) -> NoWorkTier {
        match self {
            Self::Tier1(_) => NoWorkTier::Tier1,
            Self::Tier15(_) => NoWorkTier::Tier1_5,
            Self::Tier2(_) => NoWorkTier::Tier2,
            Self::Tier3(_) => NoWorkTier::Tier3,
            Self::Tier4(_) => NoWorkTier::Tier4,
        }
    }

    fn file_name(self) -> &'static str {
        match self {
            Self::Tier1(Tier1EvidenceArtifact::ReadyPage) => "ready-page.json",
            Self::Tier1(Tier1EvidenceArtifact::ReadFailure)
            | Self::Tier15(Tier15EvidenceArtifact::ReadFailure) => "read-failure.json",
            Self::Tier15(Tier15EvidenceArtifact::Observation) => "observation.json",
            Self::Tier2(Tier2EvidenceArtifact::Policy) => "policy.json",
            Self::Tier2(Tier2EvidenceArtifact::Collector) => "collector.json",
            Self::Tier2(Tier2EvidenceArtifact::Generated) => "generated.json",
            Self::Tier2(Tier2EvidenceArtifact::Dedup) => "dedup.json",
            Self::Tier2(Tier2EvidenceArtifact::Verification) => "verification.json",
            Self::Tier2(Tier2EvidenceArtifact::RoiRank) => "roi-rank.json",
            Self::Tier2(Tier2EvidenceArtifact::Failure) => "failure.json",
            Self::Tier3(Tier3EvidenceArtifact::Policy) => "policy.json",
            Self::Tier3(Tier3EvidenceArtifact::Architecture) => "architecture.json",
            Self::Tier3(Tier3EvidenceArtifact::Coverage) => "coverage.json",
            Self::Tier3(Tier3EvidenceArtifact::Debt) => "debt.json",
            Self::Tier3(Tier3EvidenceArtifact::Findings) => "findings.json",
            Self::Tier3(Tier3EvidenceArtifact::Failure) => "failure.json",
            Self::Tier4(Tier4EvidenceArtifact::Policy) => "policy.json",
            Self::Tier4(Tier4EvidenceArtifact::SourcePolicy) => "source_policy.json",
            Self::Tier4(Tier4EvidenceArtifact::Sources) => "sources.json",
            Self::Tier4(Tier4EvidenceArtifact::Generated) => "generated.json",
            Self::Tier4(Tier4EvidenceArtifact::Dedup) => "dedup.json",
            Self::Tier4(Tier4EvidenceArtifact::Verification) => "verification.json",
            Self::Tier4(Tier4EvidenceArtifact::RoiRank) => "roi_rank.json",
            Self::Tier4(Tier4EvidenceArtifact::Failure) => "failure.json",
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

pub(super) fn clear_unreferenced_tier4(
    root: &Path,
    pass_id: u64,
) -> Result<(), WaterfallStoreError> {
    let artifacts = [
        Tier4EvidenceArtifact::Policy,
        Tier4EvidenceArtifact::SourcePolicy,
        Tier4EvidenceArtifact::Sources,
        Tier4EvidenceArtifact::Generated,
        Tier4EvidenceArtifact::Dedup,
        Tier4EvidenceArtifact::Verification,
        Tier4EvidenceArtifact::RoiRank,
        Tier4EvidenceArtifact::Failure,
    ];
    for artifact in artifacts {
        let reference = WaterfallEvidenceArtifact::Tier4(artifact).reference(pass_id)?;
        let path = root.join(reference);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(WaterfallStoreError::Diagnostic(format!(
                    "cannot clear unreferenced Tier 4 evidence {}: {error}",
                    path.display()
                )));
            }
        }
    }
    clear_tier4_temporaries(root, pass_id, &artifacts)?;
    Ok(())
}

pub(super) fn clear_obsolete_tier2_policy(
    root: &Path,
    pass_id: u64,
    expected: &str,
) -> Result<(), WaterfallStoreError> {
    let reference =
        WaterfallEvidenceArtifact::Tier2(Tier2EvidenceArtifact::Policy).reference(pass_id)?;
    let path = root.join(reference);
    match fs::read_to_string(&path) {
        Ok(contents) if contents == expected => fs::remove_file(&path).map_err(|error| {
            WaterfallStoreError::Diagnostic(format!(
                "cannot clear obsolete Tier 2 policy evidence {}: {error}",
                path.display()
            ))
        }),
        Ok(_) => Err(WaterfallStoreError::InvalidReceipt(
            "unreferenced Tier 2 policy evidence does not match the obsolete policy".to_string(),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(WaterfallStoreError::Diagnostic(format!(
            "cannot inspect obsolete Tier 2 policy evidence {}: {error}",
            path.display()
        ))),
    }
}

pub(super) fn remove_obsolete_tier2_receipt(
    receipt_path: &Path,
) -> Result<(), WaterfallStoreError> {
    match fs::remove_file(receipt_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Err(WaterfallStoreError::InvalidReceipt(
                "obsolete Tier 2 receipt disappeared during rotation".to_string(),
            ))
        }
        Err(error) => Err(WaterfallStoreError::Diagnostic(format!(
            "cannot remove obsolete Tier 2 receipt {}: {error}",
            receipt_path.display()
        ))),
    }
}

fn clear_tier4_temporaries(
    root: &Path,
    pass_id: u64,
    artifacts: &[Tier4EvidenceArtifact],
) -> Result<(), WaterfallStoreError> {
    let directory = root.join(format!("waterfall/{pass_id}/tier4"));
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(WaterfallStoreError::Diagnostic(format!(
                "cannot inspect Tier 4 evidence directory {}: {error}",
                directory.display()
            )))
        }
    };
    for entry in entries {
        let entry = entry.map_err(|error| {
            WaterfallStoreError::Diagnostic(format!(
                "cannot inspect Tier 4 evidence entry in {}: {error}",
                directory.display()
            ))
        })?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if artifacts.iter().any(|artifact| {
            is_atomic_temporary(
                name,
                WaterfallEvidenceArtifact::Tier4(*artifact).file_name(),
            )
        }) {
            fs::remove_file(entry.path()).map_err(|error| {
                WaterfallStoreError::Diagnostic(format!(
                    "cannot clear unreferenced Tier 4 temporary {}: {error}",
                    entry.path().display()
                ))
            })?;
        }
    }
    Ok(())
}

fn is_atomic_temporary(name: &str, artifact: &str) -> bool {
    let prefix = format!(".{artifact}.");
    let Some(sequence) = name
        .strip_prefix(&prefix)
        .and_then(|name| name.strip_suffix(".tmp"))
    else {
        return false;
    };
    let Some((process, counter)) = sequence.split_once('.') else {
        return false;
    };
    !process.is_empty()
        && !counter.is_empty()
        && process.bytes().all(|byte| byte.is_ascii_digit())
        && counter.bytes().all(|byte| byte.is_ascii_digit())
}
