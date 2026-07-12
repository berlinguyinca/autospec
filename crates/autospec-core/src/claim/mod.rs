#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimState {
    Claimed,
    Merged,
}

impl ClaimState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ClaimState::Claimed => "claimed",
            ClaimState::Merged => "merged",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimLease {
    pub issue_number: u64,
    pub worker_id: String,
    pub branch: String,
    pub claimed_at: u64,
    pub updated_at: u64,
    pub ttl_seconds: u64,
    pub state: ClaimState,
}

impl ClaimLease {
    pub fn new(
        issue_number: u64,
        worker_id: impl Into<String>,
        branch: impl Into<String>,
        claimed_at: u64,
        updated_at: u64,
        ttl_seconds: u64,
    ) -> Self {
        Self {
            issue_number,
            worker_id: worker_id.into(),
            branch: branch.into(),
            claimed_at,
            updated_at,
            ttl_seconds,
            state: ClaimState::Claimed,
        }
    }

    pub fn terminal_merged(
        issue_number: u64,
        worker_id: impl Into<String>,
        branch: impl Into<String>,
        merged_at: u64,
    ) -> Self {
        Self {
            issue_number,
            worker_id: worker_id.into(),
            branch: branch.into(),
            claimed_at: merged_at,
            updated_at: merged_at,
            ttl_seconds: 0,
            state: ClaimState::Merged,
        }
    }

    pub fn is_stale_at(&self, server_timestamp: u64) -> bool {
        if self.state == ClaimState::Merged {
            return false;
        }
        server_timestamp.saturating_sub(self.updated_at) > self.ttl_seconds
    }

    pub fn is_terminal(&self) -> bool {
        self.state == ClaimState::Merged
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClaimRunState {
    pub lease: Option<ClaimLease>,
}

impl ClaimRunState {
    pub fn new(lease: Option<ClaimLease>) -> Self {
        Self { lease }
    }

    pub fn terminal_merged(
        issue_number: u64,
        worker_id: impl Into<String>,
        branch: impl Into<String>,
        merged_at: u64,
    ) -> Self {
        Self::new(Some(ClaimLease::terminal_merged(
            issue_number,
            worker_id,
            branch,
            merged_at,
        )))
    }

    pub fn accepts_claim(&self, incoming: &ClaimLease, server_timestamp: u64) -> bool {
        match &self.lease {
            None => true,
            Some(current) if current.is_terminal() => false,
            Some(current) if current.worker_id == incoming.worker_id => true,
            Some(current) => current.is_stale_at(server_timestamp),
        }
    }
}
