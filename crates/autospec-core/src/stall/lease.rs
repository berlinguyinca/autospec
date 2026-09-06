//! A claim as a lease with an expiry, renewed by observed progress.
//!
//! A label is not a lease: nothing about a label says who still holds it or when
//! it stops being held, so an implementer that dies mid-run leaves its issue
//! claimed forever. A lease carries an expiry, and the expiry does the releasing.
//!
//! Renewal is driven by progress the supervisor observes from outside the child
//! (commits appearing, the transcript growing, the working tree changing), never
//! by a timer the agent controls. A wedged agent cannot renew, and its lease
//! expires on its own.

/// Lease duration and renewal cadence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeasePolicy {
    /// How long a lease lives from issue or renewal.
    pub duration_secs: u64,
    /// Minimum spacing between two renewals, so a chatty transcript cannot make
    /// a lease immortal without making progress observable.
    pub renewal_interval_secs: u64,
}

impl Default for LeasePolicy {
    fn default() -> Self {
        Self {
            duration_secs: default_duration_secs(),
            renewal_interval_secs: default_renewal_interval_secs(),
        }
    }
}

impl LeasePolicy {
    /// Read the policy from the pipeline's environment.
    pub fn from_env() -> Self {
        Self {
            duration_secs: env_u64("AUTOSPEC_ISSUE_LEASE_SECS", default_duration_secs()),
            renewal_interval_secs: env_u64(
                "AUTOSPEC_ISSUE_LEASE_RENEW_SECS",
                default_renewal_interval_secs(),
            ),
        }
    }

    /// A renewal interval at or above the lease duration makes renewal
    /// meaningless: the lease would always expire first.
    pub fn validate(&self) -> Result<(), String> {
        if self.duration_secs == 0 {
            return Err("issue lease duration must be greater than zero".to_string());
        }
        if self.renewal_interval_secs == 0 {
            return Err("issue lease renewal interval must be greater than zero".to_string());
        }
        if self.renewal_interval_secs >= self.duration_secs {
            return Err(
                "issue lease renewal interval must be shorter than the lease duration".to_string(),
            );
        }
        Ok(())
    }
}

fn default_duration_secs() -> u64 {
    900
}

fn default_renewal_interval_secs() -> u64 {
    120
}

fn env_u64(name: &str, fallback: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(fallback)
}

/// What one observation of progress did to a lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Renewal {
    /// Progress was observed and the lease now expires at the inner timestamp.
    Renewed { expires_at: u64 },
    /// Nothing has moved since the last observation, so nothing is renewed.
    NoProgress,
    /// Progress exists but the renewal interval has not elapsed yet.
    TooSoon,
    /// The lease had already expired when this observation arrived.
    Expired,
}

impl Renewal {
    pub fn renewed(&self) -> bool {
        matches!(self, Renewal::Renewed { .. })
    }
}

/// A monotonic revision of observed work on one attempt.
///
/// The three inputs are all read by the supervisor: how many commits the
/// worktree has ahead of base, how large the session transcript is, and how
/// large the uncommitted diff is. Commits count as work here — a tree-only
/// check reports a committing agent as having produced nothing.
pub fn progress_revision(
    commits_ahead: u64,
    transcript_bytes: u64,
    working_tree_bytes: u64,
) -> u64 {
    commits_ahead
        .saturating_mul(1_000_000)
        .saturating_add(transcript_bytes)
        .saturating_add(working_tree_bytes)
}

/// One implementer's hold on one issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueLease {
    pub issue: u64,
    pub worker_id: String,
    pub branch: String,
    pub model: String,
    pub policy: LeasePolicy,
    pub started_at: u64,
    pub expires_at: u64,
    last_renewal_at: u64,
    renewed_revision: u64,
    pending_progress: bool,
}

impl IssueLease {
    /// Take a lease. The policy is validated so a misconfigured pipeline fails
    /// at claim time rather than producing leases nobody can renew.
    pub fn take(
        issue: u64,
        worker_id: impl Into<String>,
        branch: impl Into<String>,
        model: impl Into<String>,
        at: u64,
        policy: LeasePolicy,
    ) -> Result<Self, String> {
        policy.validate()?;
        if issue == 0 {
            return Err("issue lease requires a positive issue number".to_string());
        }
        Ok(Self {
            issue,
            worker_id: worker_id.into(),
            branch: branch.into(),
            model: model.into(),
            expires_at: at.saturating_add(policy.duration_secs),
            started_at: at,
            last_renewal_at: at,
            policy,
            renewed_revision: 0,
            pending_progress: false,
        })
    }

    pub fn is_expired_at(&self, now: u64) -> bool {
        now >= self.expires_at
    }

    /// Renew from the latest observed progress revision.
    pub fn observe_progress(&mut self, at: u64, revision: u64) -> Renewal {
        if self.is_expired_at(at) {
            return Renewal::Expired;
        }
        if revision > self.renewed_revision {
            self.pending_progress = true;
        }
        if !self.pending_progress {
            return Renewal::NoProgress;
        }
        if at
            < self
                .last_renewal_at
                .saturating_add(self.policy.renewal_interval_secs)
        {
            return Renewal::TooSoon;
        }
        self.last_renewal_at = at;
        self.expires_at = at.saturating_add(self.policy.duration_secs);
        self.renewed_revision = revision;
        self.pending_progress = false;
        Renewal::Renewed {
            expires_at: self.expires_at,
        }
    }

    /// Seconds a lease has left at `now`.
    pub fn remaining_secs(&self, now: u64) -> u64 {
        self.expires_at.saturating_sub(now)
    }

    /// Seconds the attempt has held the lease at `now`.
    pub fn held_secs(&self, now: u64) -> u64 {
        now.saturating_sub(self.started_at)
    }
}
