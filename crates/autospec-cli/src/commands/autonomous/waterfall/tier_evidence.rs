use std::fs;
use std::io;

use autospec_core::autonomous::no_work::NoWorkTier;
use autospec_core::autonomous::waterfall::{SealedEvidence, TierReceipt, WaterfallState};

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

    pub(in crate::commands::autonomous) fn clear_obsolete_tier2_policy_evidence(
        &self,
        pass_id: u64,
        expected: &str,
    ) -> Result<(), WaterfallStoreError> {
        evidence::clear_obsolete_tier2_policy(&self.root, pass_id, expected)
    }

    pub(in crate::commands::autonomous) fn remove_unreferenced_tier2_receipt(
        &self,
        state: &WaterfallState,
        receipt: &TierReceipt,
    ) -> Result<(), WaterfallStoreError> {
        if state.current_tier() != NoWorkTier::Tier2
            || receipt.tier() != NoWorkTier::Tier2
            || receipt.pass_id() != state.next_pass_id()
            || state
                .completed_receipts()
                .iter()
                .any(|completed| completed.tier == NoWorkTier::Tier2)
        {
            return Err(WaterfallStoreError::InvalidReceipt(
                "Tier 2 receipt is referenced or outside the current cursor".to_string(),
            ));
        }
        self.verify_tier2_evidence(state.next_pass_id(), receipt)?;
        let receipt_path = self.receipt_path(receipt)?;
        match fs::remove_file(&receipt_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(WaterfallStoreError::InvalidReceipt(
                    "obsolete Tier 2 receipt disappeared during rotation".to_string(),
                ))
            }
            Err(error) => {
                return Err(WaterfallStoreError::Diagnostic(format!(
                    "cannot remove obsolete Tier 2 receipt {}: {error}",
                    receipt_path.display()
                )))
            }
        }
        Ok(())
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

    pub(in crate::commands::autonomous) fn clear_unreferenced_tier4_evidence(
        &self,
        pass_id: u64,
    ) -> Result<(), WaterfallStoreError> {
        evidence::clear_unreferenced_tier4(&self.root, pass_id)
    }

    pub(in crate::commands::autonomous) fn verify_tier4_evidence(
        &self,
        pass_id: u64,
        receipt: &TierReceipt,
    ) -> Result<(), WaterfallStoreError> {
        evidence::verify_tier4(
            &self.root,
            pass_id,
            receipt,
            self.expected_tier4_source_policy.as_ref(),
        )
    }
}
