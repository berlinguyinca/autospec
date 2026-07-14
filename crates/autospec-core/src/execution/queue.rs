use super::queue_storage::now;
use super::result::{AgentOutcome, IngestedAgentResult};

pub(super) const QUEUE_SCHEMA: u64 = 3;
pub(super) const AGENT_RESULTS_QUEUE_SCHEMA: u64 = 2;
pub(super) const LEGACY_QUEUE_SCHEMA: u64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueStatus {
    Pending,
    Running,
    Passed,
    Failed,
    Blocked,
    Deferred,
    Superseded,
}

impl QueueStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
            Self::Deferred => "deferred",
            Self::Superseded => "superseded",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "passed" => Ok(Self::Passed),
            "failed" => Ok(Self::Failed),
            "blocked" => Ok(Self::Blocked),
            "deferred" => Ok(Self::Deferred),
            "superseded" => Ok(Self::Superseded),
            _ => Err(format!("unknown queue status: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureKind {
    Validation,
    Environment,
    Agent,
    Dependency,
    Safety,
}

impl FailureKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::Environment => "environment",
            Self::Agent => "agent",
            Self::Dependency => "dependency",
            Self::Safety => "safety",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "validation" => Ok(Self::Validation),
            "environment" => Ok(Self::Environment),
            "agent" => Ok(Self::Agent),
            "dependency" => Ok(Self::Dependency),
            "safety" => Ok(Self::Safety),
            _ => Err(format!("unknown failure kind: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueValidationStatus {
    Passed,
    Failed,
}

impl QueueValidationStatus {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
        }
    }

    pub(super) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "passed" => Ok(Self::Passed),
            "failed" => Ok(Self::Failed),
            _ => Err(format!("unknown queue validation status: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueValidationResult {
    pub status: QueueValidationStatus,
    pub summary: String,
}

impl QueueValidationResult {
    pub fn new(status: QueueValidationStatus, summary: impl Into<String>) -> Self {
        Self {
            status,
            summary: summary.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueResultApplication {
    Applied,
    AlreadyApplied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueIngestionReceipt {
    pub application: QueueResultApplication,
    pub status: QueueStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueEntry {
    pub spec_id: String,
    pub status: QueueStatus,
    pub attempts: u32,
    pub failure_kind: Option<FailureKind>,
    pub blocker: Option<String>,
    pub started_at: Option<u64>,
    pub updated_at: u64,
    pub validation: Option<QueueValidationResult>,
    pub agent_result_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionQueue {
    pub run_id: String,
    pub updated_at: u64,
    pub(super) revision: u64,
    pub(super) entries: Vec<QueueEntry>,
}

impl ExecutionQueue {
    pub fn new(run_id: impl Into<String>, spec_ids: Vec<String>) -> Self {
        Self::new_at(run_id, spec_ids, now())
    }

    pub fn new_at(run_id: impl Into<String>, spec_ids: Vec<String>, timestamp: u64) -> Self {
        Self {
            run_id: run_id.into(),
            updated_at: timestamp,
            revision: 0,
            entries: spec_ids
                .into_iter()
                .map(|spec_id| QueueEntry {
                    spec_id,
                    status: QueueStatus::Pending,
                    attempts: 0,
                    failure_kind: None,
                    blocker: None,
                    started_at: None,
                    updated_at: timestamp,
                    validation: None,
                    agent_result_ids: Vec::new(),
                })
                .collect(),
        }
    }

    pub fn entry(&self, spec_id: &str) -> Option<&QueueEntry> {
        self.entries.iter().find(|entry| entry.spec_id == spec_id)
    }

    pub fn next_incomplete(&self) -> Option<&QueueEntry> {
        self.entries.iter().find(|entry| {
            matches!(
                entry.status,
                QueueStatus::Pending | QueueStatus::Running | QueueStatus::Failed
            )
        })
    }

    pub fn mark_started_at(&mut self, spec_id: &str, timestamp: u64) -> Result<(), String> {
        self.update(spec_id, timestamp, |entry| {
            if !matches!(
                entry.status,
                QueueStatus::Pending | QueueStatus::Failed | QueueStatus::Running
            ) {
                return Err(format!("cannot restart terminal queue entry: {spec_id}"));
            }
            entry.status = QueueStatus::Running;
            entry.started_at.get_or_insert(timestamp);
            Ok(())
        })
    }

    pub fn mark_passed_at(&mut self, spec_id: &str, timestamp: u64) -> Result<(), String> {
        self.update(spec_id, timestamp, |entry| {
            entry.status = QueueStatus::Passed;
            entry.blocker = None;
            Ok(())
        })
    }

    pub fn record_validation_at(
        &mut self,
        spec_id: &str,
        validation: QueueValidationResult,
        timestamp: u64,
    ) -> Result<(), String> {
        self.update(spec_id, timestamp, |entry| {
            entry.validation = Some(validation);
            Ok(())
        })
    }

    pub fn mark_passed(&mut self, spec_id: &str) -> Result<(), String> {
        self.mark_passed_at(spec_id, now())
    }

    pub fn record_failure(
        &mut self,
        spec_id: &str,
        failure_kind: FailureKind,
        retry_limit: u32,
    ) -> Result<(), String> {
        let timestamp = now();
        let retry_limit_exceeded = {
            let entry = self.entry_mut(spec_id)?;
            entry.attempts += 1;
            entry.failure_kind = Some(failure_kind);
            entry.updated_at = timestamp;
            if entry.attempts > retry_limit {
                entry.status = QueueStatus::Blocked;
                entry.blocker = Some("retry limit exceeded".to_string());
                true
            } else {
                entry.status = QueueStatus::Failed;
                false
            }
        };
        self.updated_at = timestamp;
        if retry_limit_exceeded {
            Err(format!("retry limit exceeded for {spec_id}"))
        } else {
            Ok(())
        }
    }

    pub fn block(&mut self, spec_id: &str, reason: impl Into<String>) -> Result<(), String> {
        let reason = reason.into();
        self.update(spec_id, now(), |entry| {
            entry.status = QueueStatus::Blocked;
            entry.blocker = Some(reason);
            Ok(())
        })
    }

    pub(crate) fn apply_agent_result_at(
        &mut self,
        result: &IngestedAgentResult,
        retry_limit: u32,
        timestamp: u64,
    ) -> Result<QueueResultApplication, String> {
        result.validate()?;
        if result.run_id != self.run_id {
            return Err(format!(
                "agent result run id does not match queue: {}",
                result.run_id
            ));
        }
        let entry = self.entry_mut(&result.spec_id)?;
        if entry
            .agent_result_ids
            .iter()
            .any(|id| id == &result.result_id)
        {
            return Ok(QueueResultApplication::AlreadyApplied);
        }
        if matches!(
            entry.status,
            QueueStatus::Passed
                | QueueStatus::Blocked
                | QueueStatus::Deferred
                | QueueStatus::Superseded
        ) {
            return Err(format!(
                "cannot apply a new result to terminal queue entry: {}",
                result.spec_id
            ));
        }

        entry.started_at.get_or_insert(timestamp);
        match &result.outcome {
            AgentOutcome::Passed => {
                entry.status = QueueStatus::Passed;
                entry.failure_kind = None;
                entry.blocker = None;
                entry.validation = Some(QueueValidationResult::new(
                    QueueValidationStatus::Passed,
                    result.agent_result.validation.clone(),
                ));
            }
            AgentOutcome::Failed { failure_kind } => {
                entry.attempts += 1;
                entry.failure_kind = Some(failure_kind.clone());
                entry.blocker = None;
                entry.validation = Some(QueueValidationResult::new(
                    QueueValidationStatus::Failed,
                    result.agent_result.validation.clone(),
                ));
                if entry.attempts > retry_limit {
                    entry.status = QueueStatus::Blocked;
                    entry.blocker = Some("retry limit exceeded".to_string());
                } else {
                    entry.status = QueueStatus::Failed;
                }
            }
            AgentOutcome::Blocked => {
                entry.status = QueueStatus::Blocked;
                entry.failure_kind = None;
                entry.blocker = Some(
                    result
                        .agent_result
                        .blockers
                        .iter()
                        .map(|blocker| blocker.trim())
                        .filter(|blocker| !blocker.is_empty())
                        .collect::<Vec<_>>()
                        .join("; "),
                );
            }
        }
        entry.agent_result_ids.push(result.result_id.clone());
        entry.updated_at = timestamp;
        self.updated_at = timestamp;
        Ok(QueueResultApplication::Applied)
    }

    pub fn handoff_markdown(&self, spec_id: &str) -> Option<String> {
        let entry = self.entry(spec_id)?;
        Some(format!(
            "# Blocked Spec: {}\n\nRun: {}\n\nStatus: {}\n\nReason: {}\n",
            entry.spec_id,
            self.run_id,
            entry.status.as_str(),
            entry.blocker.as_deref().unwrap_or("blocked without reason")
        ))
    }

    pub fn final_report_markdown(&self) -> String {
        format!(
            "# AutoSpec Run Report\n\nRun: {}\n\npassed: {}\nfailed: {}\nblocked: {}\ndeferred: {}\nsuperseded: {}\n",
            self.run_id,
            self.count(QueueStatus::Passed),
            self.count(QueueStatus::Failed),
            self.count(QueueStatus::Blocked),
            self.count(QueueStatus::Deferred),
            self.count(QueueStatus::Superseded)
        )
    }

    fn update(
        &mut self,
        spec_id: &str,
        timestamp: u64,
        change: impl FnOnce(&mut QueueEntry) -> Result<(), String>,
    ) -> Result<(), String> {
        let entry = self.entry_mut(spec_id)?;
        change(entry)?;
        entry.updated_at = timestamp;
        self.updated_at = timestamp;
        Ok(())
    }

    fn count(&self, status: QueueStatus) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.status == status)
            .count()
    }

    fn entry_mut(&mut self, spec_id: &str) -> Result<&mut QueueEntry, String> {
        self.entries
            .iter_mut()
            .find(|entry| entry.spec_id == spec_id)
            .ok_or_else(|| format!("unknown queue spec: {spec_id}"))
    }
}

pub(crate) fn is_valid_run_id(value: &str) -> bool {
    value
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}
