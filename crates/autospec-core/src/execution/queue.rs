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
            QueueStatus::Pending => "pending",
            QueueStatus::Running => "running",
            QueueStatus::Passed => "passed",
            QueueStatus::Failed => "failed",
            QueueStatus::Blocked => "blocked",
            QueueStatus::Deferred => "deferred",
            QueueStatus::Superseded => "superseded",
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
            FailureKind::Validation => "validation",
            FailureKind::Environment => "environment",
            FailureKind::Agent => "agent",
            FailureKind::Dependency => "dependency",
            FailureKind::Safety => "safety",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueEntry {
    pub spec_id: String,
    pub status: QueueStatus,
    pub attempts: u32,
    pub failure_kind: Option<FailureKind>,
    pub blocker: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionQueue {
    pub run_id: String,
    entries: Vec<QueueEntry>,
}

impl ExecutionQueue {
    pub fn new(run_id: impl Into<String>, spec_ids: Vec<String>) -> Self {
        Self {
            run_id: run_id.into(),
            entries: spec_ids
                .into_iter()
                .map(|spec_id| QueueEntry {
                    spec_id,
                    status: QueueStatus::Pending,
                    attempts: 0,
                    failure_kind: None,
                    blocker: None,
                })
                .collect(),
        }
    }

    pub fn entry(&self, spec_id: &str) -> Option<&QueueEntry> {
        self.entries.iter().find(|entry| entry.spec_id == spec_id)
    }

    pub fn next_incomplete(&self) -> Option<&QueueEntry> {
        self.entries
            .iter()
            .find(|entry| matches!(entry.status, QueueStatus::Pending | QueueStatus::Failed))
    }

    pub fn mark_passed(&mut self, spec_id: &str) -> Result<(), String> {
        let entry = self.entry_mut(spec_id)?;
        entry.status = QueueStatus::Passed;
        entry.blocker = None;
        Ok(())
    }

    pub fn record_failure(
        &mut self,
        spec_id: &str,
        failure_kind: FailureKind,
        retry_limit: u32,
    ) -> Result<(), String> {
        let entry = self.entry_mut(spec_id)?;
        entry.attempts += 1;
        entry.failure_kind = Some(failure_kind);
        if entry.attempts > retry_limit {
            entry.status = QueueStatus::Blocked;
            entry.blocker = Some("retry limit exceeded".to_string());
            Err(format!("retry limit exceeded for {spec_id}"))
        } else {
            entry.status = QueueStatus::Failed;
            Ok(())
        }
    }

    pub fn block(&mut self, spec_id: &str, reason: impl Into<String>) -> Result<(), String> {
        let entry = self.entry_mut(spec_id)?;
        entry.status = QueueStatus::Blocked;
        entry.blocker = Some(reason.into());
        Ok(())
    }

    pub fn handoff_markdown(&self, spec_id: &str) -> Option<String> {
        let entry = self.entry(spec_id)?;
        let reason = entry.blocker.as_deref().unwrap_or("blocked without reason");
        Some(format!(
            "# Blocked Spec: {}\n\nRun: {}\n\nStatus: {}\n\nReason: {}\n",
            entry.spec_id,
            self.run_id,
            entry.status.as_str(),
            reason
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
