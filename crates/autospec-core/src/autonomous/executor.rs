//! Typed identity for one supervised implementation invocation.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorInvocation {
    pub repo: String,
    pub issue: u64,
    pub worker_id: String,
    pub branch: String,
    pub claim_id: String,
    pub invocation_id: String,
    pub expected_commit: String,
}

impl ExecutorInvocation {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("repo", self.repo.as_str()),
            ("worker_id", self.worker_id.as_str()),
            ("branch", self.branch.as_str()),
            ("claim_id", self.claim_id.as_str()),
            ("invocation_id", self.invocation_id.as_str()),
        ] {
            if value.is_empty() || value.len() > 256 {
                return Err(format!("executor {name} must be 1..256 bytes"));
            }
        }
        if self.issue == 0 {
            return Err("executor issue must be positive".to_string());
        }
        if !((self.expected_commit.len() == 40 || self.expected_commit.len() == 64)
            && self.expected_commit.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return Err("executor expected_commit must be 40 or 64 hex characters".to_string());
        }
        Ok(())
    }
}
