use autospec_core::autonomous::waterfall::{SealedEvidence, TierReceipt};

use super::{evidence, Tier2EvidenceArtifact, Tier3EvidenceArtifact, Tier4EvidenceArtifact};
use super::{WaterfallStore, WaterfallStoreError};

impl WaterfallStore {
    pub(in crate::commands::autonomous) fn persist_tier2_evidence(
        &self,
        pass_id: u64,
        artifact: Tier2EvidenceArtifact,
        contents: &str,
    ) -> Result<SealedEvidence, WaterfallStoreError> {
        evidence::persist(
            &self.root,
            pass_id,
            evidence::WaterfallEvidenceArtifact::Tier2(artifact),
            contents,
        )
    }

    pub(in crate::commands::autonomous) fn verify_tier2_evidence(
        &self,
        pass_id: u64,
        receipt: &TierReceipt,
    ) -> Result<(), WaterfallStoreError> {
        evidence::verify_tier2(&self.root, pass_id, receipt)
    }

    pub(in crate::commands::autonomous) fn persist_tier3_evidence(
        &self,
        pass_id: u64,
        artifact: Tier3EvidenceArtifact,
        contents: &str,
    ) -> Result<SealedEvidence, WaterfallStoreError> {
        evidence::persist(
            &self.root,
            pass_id,
            evidence::WaterfallEvidenceArtifact::Tier3(artifact),
            contents,
        )
    }

    pub(in crate::commands::autonomous) fn verify_tier3_evidence(
        &self,
        pass_id: u64,
        receipt: &TierReceipt,
    ) -> Result<(), WaterfallStoreError> {
        evidence::verify_tier3(&self.root, pass_id, receipt)
    }

    pub(in crate::commands::autonomous) fn persist_tier4_evidence(
        &self,
        pass_id: u64,
        artifact: Tier4EvidenceArtifact,
        contents: &str,
    ) -> Result<SealedEvidence, WaterfallStoreError> {
        evidence::persist(
            &self.root,
            pass_id,
            evidence::WaterfallEvidenceArtifact::Tier4(artifact),
            contents,
        )
    }

    pub(in crate::commands::autonomous) fn verify_tier4_evidence(
        &self,
        pass_id: u64,
        receipt: &TierReceipt,
    ) -> Result<(), WaterfallStoreError> {
        evidence::verify_tier4(&self.root, pass_id, receipt)
    }
}
