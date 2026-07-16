use crate::autonomous::waterfall::FunnelCounts;

use super::model::{
    StrictCollectorEvidence, Tier2Deduplication, Tier2GeneratedProposals, Tier2VerifierVerdicts,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier2PartialEvidence(PartialEvidenceState);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PartialEvidenceState {
    None {
        funnel: FunnelCounts,
    },
    Collector {
        collector: StrictCollectorEvidence,
        funnel: FunnelCounts,
    },
    Generated {
        collector: StrictCollectorEvidence,
        generated: Tier2GeneratedProposals,
        funnel: FunnelCounts,
    },
    Deduplicated {
        collector: StrictCollectorEvidence,
        generated: Tier2GeneratedProposals,
        deduplication: Tier2Deduplication,
        funnel: FunnelCounts,
    },
    Verified {
        collector: StrictCollectorEvidence,
        generated: Tier2GeneratedProposals,
        deduplication: Tier2Deduplication,
        verification: Tier2VerifierVerdicts,
        funnel: FunnelCounts,
    },
}

impl Tier2PartialEvidence {
    pub fn funnel(&self) -> &FunnelCounts {
        match &self.0 {
            PartialEvidenceState::None { funnel }
            | PartialEvidenceState::Collector { funnel, .. }
            | PartialEvidenceState::Generated { funnel, .. }
            | PartialEvidenceState::Deduplicated { funnel, .. }
            | PartialEvidenceState::Verified { funnel, .. } => funnel,
        }
    }

    pub(super) fn state(&self) -> &PartialEvidenceState {
        &self.0
    }

    pub(super) fn none() -> Self {
        Self(PartialEvidenceState::None {
            funnel: FunnelCounts::new(0, 0, 0, 0, 0).expect("zero funnel counts are valid"),
        })
    }

    pub(super) fn collector(collector: StrictCollectorEvidence, funnel: FunnelCounts) -> Self {
        Self(PartialEvidenceState::Collector { collector, funnel })
    }

    pub(super) fn generated(
        collector: StrictCollectorEvidence,
        generated: Tier2GeneratedProposals,
        funnel: FunnelCounts,
    ) -> Self {
        Self(PartialEvidenceState::Generated {
            collector,
            generated,
            funnel,
        })
    }

    pub(super) fn deduplicated(
        collector: StrictCollectorEvidence,
        generated: Tier2GeneratedProposals,
        deduplication: Tier2Deduplication,
        funnel: FunnelCounts,
    ) -> Self {
        Self(PartialEvidenceState::Deduplicated {
            collector,
            generated,
            deduplication,
            funnel,
        })
    }

    pub(super) fn verified(
        collector: StrictCollectorEvidence,
        generated: Tier2GeneratedProposals,
        deduplication: Tier2Deduplication,
        verification: Tier2VerifierVerdicts,
        funnel: FunnelCounts,
    ) -> Self {
        Self(PartialEvidenceState::Verified {
            collector,
            generated,
            deduplication,
            verification,
            funnel,
        })
    }
}
