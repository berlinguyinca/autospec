//! Provider-neutral native session identity, fencing, and lineage.
//!
//! Implements the "Native session delta" of
//! `docs/specs/2026-09-01-observational-memory-native-sessions-readiness-integration-delta-design.md`:
//! multi-harness scoped native IDs, creation intent/idempotency for
//! crash-after-create, heartbeat/lease/fencing and reconciliation, lineage to
//! work item, stage, role, worktree, branch, PR, model/provider, truthful
//! hidden-context capability reporting, and typed events.
//!
//! Degraded/fallback mapping is capability-only: capabilities can only shrink
//! against a ceiling, and no field here encodes tool, privacy, role, or
//! separation-of-duties policy, so a capability change cannot weaken policy.

use std::fmt;

/// Harnesses that can host a native coding session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SessionHarness {
    Claude,
    Codex,
    OpenCode,
    Pi,
}

impl SessionHarness {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionHarness::Claude => "claude",
            SessionHarness::Codex => "codex",
            SessionHarness::OpenCode => "opencode",
            SessionHarness::Pi => "pi",
        }
    }

    /// Parse a canonical harness name. Unknown names are an error, not a
    /// silent fallback.
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            "opencode" => Ok(Self::OpenCode),
            "pi" => Ok(Self::Pi),
            other => Err(format!("unknown session harness {other:?}")),
        }
    }
}

/// Why a session is being created. Retried creates after a crash carry
/// [`CreationIntent::RetryAfterCrash`]; idempotency is keyed separately so the
/// two intents never collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreationIntent {
    Fresh,
    RetryAfterCrash,
}

/// Lineage of a native session to its work item, stage, role, worktree,
/// branch, PR, and the model/provider that serves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLineage {
    pub work_item: String,
    pub stage: String,
    pub role: String,
    pub worktree: String,
    pub branch: String,
    pub pull_request: Option<String>,
    pub model: String,
    pub provider: String,
}

impl SessionLineage {
    pub fn validate(&self) -> Result<(), String> {
        let fields: [(&str, &str); 7] = [
            ("work_item", &self.work_item),
            ("stage", &self.stage),
            ("role", &self.role),
            ("worktree", &self.worktree),
            ("branch", &self.branch),
            ("model", &self.model),
            ("provider", &self.provider),
        ];
        for (field, value) in fields {
            if value.is_empty() {
                return Err(format!("lineage {field} must be non-empty"));
            }
        }
        if let Some(pr) = &self.pull_request {
            if pr.is_empty() {
                return Err("lineage pull_request must be non-empty when set".to_string());
            }
        }
        Ok(())
    }
}

/// Truthful capability report for one harness's native session support.
/// A harness that does not expose a capability must never claim it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionCapabilities {
    pub resume: bool,
    pub attach: bool,
    pub event_stream: bool,
    /// Whether the harness exposes its hidden context (system prompt and
    /// pre-session state) for inspection.
    pub hidden_context: bool,
}

impl SessionCapabilities {
    pub const fn full() -> Self {
        Self {
            resume: true,
            attach: true,
            event_stream: true,
            hidden_context: true,
        }
    }

    /// Degraded fallback mapping: capabilities only ever shrink here. Tool,
    /// privacy, role, and separation-of-duties policy are not capabilities
    /// and cannot be weakened by this mapping.
    pub fn degraded(self) -> Self {
        Self {
            resume: self.resume,
            attach: false,
            event_stream: false,
            hidden_context: false,
        }
    }

    pub fn is_degraded(self) -> bool {
        !self.resume || !self.attach || !self.event_stream || !self.hidden_context
    }

    /// `self` reports nothing that `ceiling` does not report.
    pub fn is_subset_of(self, ceiling: Self) -> bool {
        self.resume <= ceiling.resume
            && self.attach <= ceiling.attach
            && self.event_stream <= ceiling.event_stream
            && self.hidden_context <= ceiling.hidden_context
    }
}

/// Lease on a session: who holds it and at which epoch. Epochs are monotonic;
/// a client at an older epoch is stale and must not mutate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    pub holder: String,
    pub epoch: u64,
}

/// Lifecycle state of a native session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Active,
    Finished,
}

/// Typed events streamed from a native session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    Created {
        intent: CreationIntent,
    },
    Heartbeat {
        tick: u64,
    },
    Resumed {
        epoch: u64,
    },
    Work {
        artifact: String,
    },
    Fallback {
        from: SessionHarness,
        to: SessionHarness,
    },
    Finished {
        outcome: String,
    },
}

/// Fence violations. A stale epoch or unknown holder must never mutate the
/// session; these errors are the proof of that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionFenceError {
    StaleEpoch { provided: u64, current: u64 },
    UnknownHolder { provided: String, current: String },
    Finished,
}

impl fmt::Display for SessionFenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleEpoch { provided, current } => {
                write!(f, "stale lease epoch {provided}, session is at {current}")
            }
            Self::UnknownHolder { provided, current } => {
                write!(
                    f,
                    "unknown lease holder {provided:?}, session held by {current:?}"
                )
            }
            Self::Finished => write!(f, "session already finished"),
        }
    }
}

impl std::error::Error for SessionFenceError {}

/// Schema version of [`NativeSessionV1`]. Frozen additive records only.
pub const NATIVE_SESSION_SCHEMA: u32 = 1;

/// A versioned, scoped native session record.
///
/// Mutation is fenced: [`NativeSessionV1::apply`] refuses stale epochs and
/// unknown holders, so a stale client can never rewrite lineage, lease, or
/// events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeSessionV1 {
    pub schema_version: u32,
    pub harness: SessionHarness,
    pub native_id: String,
    pub creation_intent: CreationIntent,
    pub idempotency_key: String,
    pub lineage: SessionLineage,
    pub capabilities: SessionCapabilities,
    pub lease: Lease,
    pub heartbeat_tick: u64,
    pub state: SessionState,
    pub events: Vec<SessionEvent>,
}

impl NativeSessionV1 {
    pub fn new(
        harness: SessionHarness,
        native_id: impl Into<String>,
        intent: CreationIntent,
        idempotency_key: impl Into<String>,
        lineage: SessionLineage,
        capabilities: SessionCapabilities,
        holder: impl Into<String>,
    ) -> Result<Self, String> {
        let native_id = native_id.into();
        let idempotency_key = idempotency_key.into();
        let holder = holder.into();
        if native_id.is_empty() {
            return Err("native_id must be non-empty".to_string());
        }
        if idempotency_key.is_empty() {
            return Err("idempotency_key must be non-empty".to_string());
        }
        if holder.is_empty() {
            return Err("lease holder must be non-empty".to_string());
        }
        lineage.validate()?;
        Ok(Self {
            schema_version: NATIVE_SESSION_SCHEMA,
            harness,
            native_id,
            creation_intent: intent,
            idempotency_key,
            lineage,
            capabilities,
            lease: Lease { holder, epoch: 1 },
            heartbeat_tick: 0,
            state: SessionState::Active,
            events: vec![SessionEvent::Created { intent }],
        })
    }

    /// The scoped native ID: harness-scoped so the same native ID under two
    /// harnesses is never the same session.
    pub fn scoped_id(&self) -> String {
        format!("{}:{}", self.harness.as_str(), self.native_id)
    }

    /// Check the lease fence without mutating.
    pub fn fence(&self, holder: &str, epoch: u64) -> Result<(), SessionFenceError> {
        if self.state == SessionState::Finished {
            return Err(SessionFenceError::Finished);
        }
        if holder != self.lease.holder {
            return Err(SessionFenceError::UnknownHolder {
                provided: holder.to_string(),
                current: self.lease.holder.clone(),
            });
        }
        if epoch != self.lease.epoch {
            return Err(SessionFenceError::StaleEpoch {
                provided: epoch,
                current: self.lease.epoch,
            });
        }
        Ok(())
    }

    /// Fenced mutation: a stale epoch, unknown holder, or finished session
    /// cannot change the record.
    pub fn apply(
        &mut self,
        holder: &str,
        epoch: u64,
        event: SessionEvent,
    ) -> Result<(), SessionFenceError> {
        self.fence(holder, epoch)?;
        if let SessionEvent::Heartbeat { tick } = &event {
            self.heartbeat_tick = *tick;
        }
        if matches!(event, SessionEvent::Finished { .. }) {
            self.state = SessionState::Finished;
        }
        self.events.push(event);
        Ok(())
    }

    /// Record a heartbeat under the lease fence.
    pub fn heartbeat(&mut self, holder: &str, epoch: u64) -> Result<(), SessionFenceError> {
        let tick = self.heartbeat_tick + 1;
        self.apply(holder, epoch, SessionEvent::Heartbeat { tick })
    }

    /// Lease handoff: bump the epoch (invalidating every stale client) and
    /// hand the lease to `holder`. Recovery path — after a crash the old
    /// holder is unknown, so only the runtime's own resume authority may
    /// call this.
    pub fn take_lease(&mut self, holder: &str) -> Result<u64, SessionFenceError> {
        if self.state == SessionState::Finished {
            return Err(SessionFenceError::Finished);
        }
        if holder.is_empty() {
            return Err(SessionFenceError::UnknownHolder {
                provided: String::new(),
                current: self.lease.holder.clone(),
            });
        }
        self.lease.epoch += 1;
        self.lease.holder = holder.to_string();
        self.events.push(SessionEvent::Resumed {
            epoch: self.lease.epoch,
        });
        Ok(self.lease.epoch)
    }
}
