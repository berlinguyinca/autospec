//! Session isolation and worktree ownership for multi-role runs (issue #3324).
//!
//! A multi-role run starts one session per role: a planner that writes the
//! plan, a builder that makes the change, a test session that runs the tests,
//! and a reviewer that judges the diff. This module is the provider-neutral
//! contract that keeps those sessions apart:
//!
//! * **Distinct sessions.** Every active session holds a unique session id, so
//!   the planner, the builder and the reviewer can never share one session.
//! * **Worktree ownership.** Read-only lanes (planner, scout, test, reviewer,
//!   escalation) may share a worktree and run in parallel. A mutating session
//!   (builder) is the exclusive writer of its worktree: a second writer on the
//!   same worktree is a collision and fails closed.
//! * **No edits from read-only roles.** A read-only session that reports any
//!   filesystem edit when it finishes is a breach; the finish fails closed.
//! * **Structured handoffs.** Sessions pass the plan, the diff, the test
//!   receipt and the review verdict as [`SessionArtifact`] values with a
//!   strict wire format, never as free text.
//!
//! Multiple writers in one worktree are explicitly out of scope: the registry
//! refuses the second writer instead of trying to schedule around it.
//!
//! Everything here is pure state: no I/O, no clock, no randomness.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::capsule::RolePolicy;
use super::topology::AgentRole;

/// A request to run one role session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionGrant {
    pub session_id: String,
    pub worktree: String,
    pub role: AgentRole,
    pub policy: RolePolicy,
}

impl SessionGrant {
    /// A grant is well-formed when its identifiers are non-empty and its
    /// policy matches its role: a reviewer holding the builder policy is a
    /// separation violation, not a configuration.
    pub fn validate(&self) -> Result<(), IsolationViolation> {
        if self.session_id.trim().is_empty() {
            return Err(IsolationViolation::EmptyField {
                field: "session_id",
            });
        }
        if self.worktree.trim().is_empty() {
            return Err(IsolationViolation::EmptyField { field: "worktree" });
        }
        if self.policy != RolePolicy::for_role(self.role) {
            return Err(IsolationViolation::PolicyMismatch {
                role: self.role,
                policy: self.policy,
            });
        }
        Ok(())
    }
}

/// A violation of the isolation contract. Every variant fails closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IsolationViolation {
    /// An empty session id or worktree.
    EmptyField { field: &'static str },
    /// A session id that is already held by an active session.
    DuplicateSession { session_id: String },
    /// The grant's policy does not match its role.
    PolicyMismatch { role: AgentRole, policy: RolePolicy },
    /// A mutating session requested a worktree that already has a writer.
    WorktreeCollision {
        session_id: String,
        worktree: String,
        held_by: String,
    },
    /// A read-only session reported filesystem edits.
    ReadOnlyBreach {
        session_id: String,
        files_edited: usize,
        lines_changed: u64,
    },
    /// A session id that is not active.
    UnknownSession { session_id: String },
}

impl fmt::Display for IsolationViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField { field } => {
                write!(f, "session grant requires a non-empty {field}")
            }
            Self::DuplicateSession { session_id } => {
                write!(f, "session id {session_id} is already active")
            }
            Self::PolicyMismatch { role, policy } => {
                write!(f, "policy {policy:?} does not match role {role:?}")
            }
            Self::WorktreeCollision {
                session_id,
                worktree,
                held_by,
            } => write!(
                f,
                "worktree {worktree} is already written by {held_by}; refusing second writer {session_id}"
            ),
            Self::ReadOnlyBreach {
                session_id,
                files_edited,
                lines_changed,
            } => write!(
                f,
                "read-only session {session_id} edited {files_edited} file(s), {lines_changed} line(s)"
            ),
            Self::UnknownSession { session_id } => {
                write!(f, "session {session_id} is not active")
            }
        }
    }
}

impl std::error::Error for IsolationViolation {}

/// The filesystem edits a session reports having made.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ObservedEdits {
    pub files_edited: usize,
    pub lines_changed: u64,
}

impl ObservedEdits {
    pub fn none() -> Self {
        Self::default()
    }

    pub const fn is_empty(self) -> bool {
        self.files_edited == 0 && self.lines_changed == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveGrant {
    worktree: String,
    policy: RolePolicy,
    writer: bool,
}

/// Worktree ownership across the sessions of one multi-role run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionIsolation {
    active: BTreeMap<String, ActiveGrant>,
    /// worktree -> session id of its exclusive writer.
    writers: BTreeMap<String, String>,
}

impl SessionIsolation {
    pub fn new() -> Self {
        Self::default()
    }

    /// Grant one session.
    ///
    /// Read-only sessions may join any worktree, including one an active
    /// writer holds; they run in parallel. A mutating session claims exclusive
    /// write ownership of its worktree, and a second writer on the same
    /// worktree fails closed with [`IsolationViolation::WorktreeCollision`].
    pub fn grant(&mut self, grant: SessionGrant) -> Result<(), IsolationViolation> {
        grant.validate()?;
        if self.active.contains_key(&grant.session_id) {
            return Err(IsolationViolation::DuplicateSession {
                session_id: grant.session_id,
            });
        }
        let writer = !grant.policy.is_read_only();
        if writer {
            if let Some(held_by) = self.writers.get(&grant.worktree) {
                return Err(IsolationViolation::WorktreeCollision {
                    session_id: grant.session_id,
                    worktree: grant.worktree,
                    held_by: held_by.clone(),
                });
            }
            self.writers
                .insert(grant.worktree.clone(), grant.session_id.clone());
        }
        self.active.insert(
            grant.session_id,
            ActiveGrant {
                worktree: grant.worktree,
                policy: grant.policy,
                writer,
            },
        );
        Ok(())
    }

    /// Finish a session: the observed edits are checked against its policy and
    /// its write claim is released. A read-only session that edited anything
    /// fails closed with [`IsolationViolation::ReadOnlyBreach`]; the session
    /// is over either way, so it leaves the registry.
    pub fn finish(
        &mut self,
        session_id: &str,
        edits: ObservedEdits,
    ) -> Result<(), IsolationViolation> {
        let grant =
            self.active
                .remove(session_id)
                .ok_or_else(|| IsolationViolation::UnknownSession {
                    session_id: session_id.to_string(),
                })?;
        if grant.policy.is_read_only() && !edits.is_empty() {
            return Err(IsolationViolation::ReadOnlyBreach {
                session_id: session_id.to_string(),
                files_edited: edits.files_edited,
                lines_changed: edits.lines_changed,
            });
        }
        if grant.writer {
            self.writers.remove(&grant.worktree);
        }
        Ok(())
    }

    /// The exclusive writer of a worktree, if any active session holds it.
    pub fn writer(&self, worktree: &str) -> Option<&str> {
        self.writers.get(worktree).map(String::as_str)
    }

    /// Every active session holding a worktree (writers and read-only lanes
    /// alike), in session-id order.
    pub fn holders(&self, worktree: &str) -> Vec<String> {
        self.active
            .iter()
            .filter(|(_, grant)| grant.worktree == worktree)
            .map(|(session_id, _)| session_id.clone())
            .collect()
    }

    pub fn is_active(&self, session_id: &str) -> bool {
        self.active.contains_key(session_id)
    }

    pub fn session_count(&self) -> usize {
        self.active.len()
    }
}

/// The reviewer's verdict on a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    Approve,
    ChangesRequired,
    Uncertain,
}

impl ReviewVerdict {
    /// Every verdict, in stable order.
    pub const fn variants() -> [Self; 3] {
        [Self::Approve, Self::ChangesRequired, Self::Uncertain]
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::ChangesRequired => "changes_required",
            Self::Uncertain => "uncertain",
        }
    }

    /// Parse one of `approve|changes_required|uncertain`; anything else is
    /// rejected. The reviewer's output is a verdict token, not prose.
    pub fn parse(token: &str) -> Result<Self, String> {
        match token.trim() {
            "approve" => Ok(Self::Approve),
            "changes_required" => Ok(Self::ChangesRequired),
            "uncertain" => Ok(Self::Uncertain),
            other => Err(format!(
                "review verdict {other:?} is not one of approve|changes_required|uncertain"
            )),
        }
    }
}

impl fmt::Display for ReviewVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A structured artifact passed between role sessions.
///
/// Handoffs are data, not prose: the plan the planner wrote, the diff the
/// builder produced, the test receipt the test session ran, and the verdict
/// the reviewer returned. Each has a fixed shape and a strict wire format, so
/// a malformed handoff fails closed instead of reaching a downstream session
/// as free text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionArtifact {
    /// The planner's plan.
    Plan { summary: String, steps: Vec<String> },
    /// The builder's diff.
    Diff {
        path: String,
        additions: u64,
        deletions: u64,
    },
    /// The test session's receipt.
    TestReceipt {
        command: String,
        passed: bool,
        failures: u64,
    },
    /// The reviewer's verdict.
    Review {
        verdict: ReviewVerdict,
        notes: String,
    },
}

impl SessionArtifact {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Plan { .. } => "plan",
            Self::Diff { .. } => "diff",
            Self::TestReceipt { .. } => "test_receipt",
            Self::Review { .. } => "review",
        }
    }

    /// Refuse a handoff that does not carry its content.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Plan { summary, steps } => {
                if summary.trim().is_empty() {
                    return Err("plan artifact requires a summary".to_string());
                }
                for step in steps {
                    if step.trim().is_empty() {
                        return Err("plan artifact contains an empty step".to_string());
                    }
                }
                Ok(())
            }
            Self::Diff { path, .. } => {
                if path.trim().is_empty() {
                    return Err("diff artifact requires a path".to_string());
                }
                Ok(())
            }
            Self::TestReceipt { command, .. } => {
                if command.trim().is_empty() {
                    return Err("test receipt artifact requires a command".to_string());
                }
                Ok(())
            }
            Self::Review { notes, .. } => {
                if notes.trim().is_empty() {
                    return Err("review artifact requires notes".to_string());
                }
                Ok(())
            }
        }
    }

    /// The strict JSON wire form a downstream session receives.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("session artifact serializes")
    }

    /// Parse one wire line; unknown kinds, missing fields and garbage are
    /// rejected.
    pub fn from_json(line: &str) -> Result<Self, String> {
        serde_json::from_str(line).map_err(|err| format!("session artifact: {err}"))
    }
}
