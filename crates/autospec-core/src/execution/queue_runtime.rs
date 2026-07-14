use std::fs;
use std::path::Path;

use super::queue::{ExecutionQueue, QueueIngestionReceipt, QueueStatus};
use super::queue_storage::{
    load_with_recovery, now, save_if_current, validate_queue, QueueLock, QueuePaths,
};
use super::result::IngestedAgentResult;

impl ExecutionQueue {
    pub fn create_if_absent(
        root: impl AsRef<Path>,
        run_id: impl Into<String>,
        spec_ids: Vec<String>,
    ) -> Result<Self, String> {
        Self::create_if_absent_at(root, run_id, spec_ids, now())
    }

    pub fn create_if_absent_at(
        root: impl AsRef<Path>,
        run_id: impl Into<String>,
        spec_ids: Vec<String>,
        timestamp: u64,
    ) -> Result<Self, String> {
        let mut queue = Self::new_at(run_id, spec_ids, timestamp);
        validate_queue(&queue)?;
        let paths = QueuePaths::new(root.as_ref(), &queue.run_id)?;
        let _lock = QueueLock::acquire(&paths)?;
        if load_with_recovery(&paths, &queue.run_id)?.is_some() || paths.directory.exists() {
            return Err(format!("queue already exists for run: {}", queue.run_id));
        }
        save_if_current(&mut queue, &paths)?;
        Ok(queue)
    }

    pub fn save(&mut self, root: impl AsRef<Path>) -> Result<(), String> {
        validate_queue(self)?;
        let paths = QueuePaths::new(root.as_ref(), &self.run_id)?;
        let _lock = QueueLock::acquire(&paths)?;
        save_if_current(self, &paths)
    }

    pub fn ingest_agent_result(
        root: impl AsRef<Path>,
        result: &IngestedAgentResult,
        retry_limit: u32,
    ) -> Result<QueueIngestionReceipt, String> {
        Self::ingest_agent_result_at(root, result, retry_limit, now())
    }

    pub fn ingest_agent_result_at(
        root: impl AsRef<Path>,
        result: &IngestedAgentResult,
        retry_limit: u32,
        timestamp: u64,
    ) -> Result<QueueIngestionReceipt, String> {
        result.validate()?;
        let paths = QueuePaths::new(root.as_ref(), &result.run_id)?;
        let _lock = QueueLock::acquire(&paths)?;
        let mut queue = load_with_recovery(&paths, &result.run_id)?
            .ok_or_else(|| format!("queue does not exist for run: {}", result.run_id))?;
        let entry = queue
            .entry(&result.spec_id)
            .ok_or_else(|| format!("unknown queue spec: {}", result.spec_id))?;
        if !entry
            .agent_result_ids
            .iter()
            .any(|id| id == &result.result_id)
            && matches!(
                entry.status,
                QueueStatus::Passed
                    | QueueStatus::Blocked
                    | QueueStatus::Deferred
                    | QueueStatus::Superseded
            )
        {
            return Err(format!(
                "cannot apply a new result to terminal queue entry: {}",
                result.spec_id
            ));
        }
        let persisted = result.persist_locked(root.as_ref())?;
        let application = queue.apply_agent_result_at(&persisted, retry_limit, timestamp)?;
        let status = queue
            .entry(&persisted.spec_id)
            .ok_or_else(|| format!("unknown queue spec: {}", persisted.spec_id))?
            .status
            .clone();
        save_if_current(&mut queue, &paths)?;
        Ok(QueueIngestionReceipt {
            application,
            status,
        })
    }

    pub fn load_named(root: impl AsRef<Path>, run_id: &str) -> Result<Option<Self>, String> {
        let paths = QueuePaths::new(root.as_ref(), run_id)?;
        if !paths.directory.exists() {
            return Ok(None);
        }
        let _lock = QueueLock::acquire(&paths)?;
        load_with_recovery(&paths, run_id)
    }

    pub fn load_latest_incomplete(root: impl AsRef<Path>) -> Result<Option<Self>, String> {
        let runs = root.as_ref().join(".autospec").join("runs");
        let entries = match fs::read_dir(&runs) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "failed to read run directory {}: {error}",
                    runs.display()
                ))
            }
        };
        let mut latest: Option<Self> = None;
        for entry in entries {
            let entry = entry.map_err(|error| format!("failed to read run entry: {error}"))?;
            if !entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_dir()
            {
                continue;
            }
            let run_id = entry.file_name().to_string_lossy().to_string();
            let queue = match Self::load_named(root.as_ref(), &run_id) {
                Ok(Some(queue)) => queue,
                Ok(None) => continue,
                Err(error)
                    if error.starts_with("invalid queue file")
                        || error.starts_with("invalid queue recovery file") =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            };
            if queue.next_incomplete().is_some()
                && latest.as_ref().is_none_or(|current| {
                    (queue.updated_at, &queue.run_id) > (current.updated_at, &current.run_id)
                })
            {
                latest = Some(queue);
            }
        }
        Ok(latest)
    }
}
