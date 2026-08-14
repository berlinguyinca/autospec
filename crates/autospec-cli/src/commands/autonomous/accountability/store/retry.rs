use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

impl AccountabilityStore {
    pub fn schedule_projection_retry(
        &mut self,
        retry_after_seconds: Option<u64>,
    ) -> Result<(), AccountabilityError> {
        self.state.projection_retry_attempt = self.state.projection_retry_attempt.saturating_add(1);
        let exponential = 30_u64
            .saturating_mul(1_u64 << self.state.projection_retry_attempt.min(7))
            .min(3_600);
        let delay = retry_after_seconds.unwrap_or(exponential).min(86_400);
        self.state.next_projection_retry_at = Some(unix_timestamp()?.saturating_add(delay));
        self.persist_state()
    }
}

pub(super) fn unix_timestamp() -> Result<u64, AccountabilityError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| AccountabilityError::new("system clock precedes Unix epoch"))
}
