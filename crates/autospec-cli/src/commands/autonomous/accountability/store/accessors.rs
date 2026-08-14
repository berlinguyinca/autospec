use super::*;

impl AccountabilityStore {
    pub fn has_event(&self, kind: &EventKind) -> bool {
        self.events.iter().any(|record| {
            record.segment_chain_digest == self.state.segment_chain_digest && &record.kind == kind
        })
    }

    pub fn create_attempted(&self) -> bool {
        self.state.create_attempted
    }

    pub fn desired_projection_digest(&self) -> Option<&str> {
        self.state.desired_digest.as_deref()
    }

    pub fn recovery_projection(&self) -> (RecoveryState, Vec<u64>, Vec<u64>) {
        (
            self.state.recovery_state,
            self.state.linked_issues.clone(),
            self.state.linked_pull_requests.clone(),
        )
    }
}
