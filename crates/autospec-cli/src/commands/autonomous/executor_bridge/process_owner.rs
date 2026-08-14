use std::process::{Command, ExitStatus};

mod unix_group;

pub(super) struct OwnedChildTree {
    inner: PlatformOwnedChild,
    identity: DurableProcessOwner,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DurableProcessOwner {
    pub pid: u32,
    pub container_id: String,
    pub process_start: String,
    pub launch_nonce: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RecoveryDisposition {
    Completed,
    SidecarOwns,
    Quarantine,
}

enum PlatformOwnedChild {
    Unix(unix_group::UnixOwnedChild),
}

impl OwnedChildTree {
    pub(super) fn spawn(command: &mut Command, launch_nonce: String) -> Result<Self, String> {
        let mut inner = unix_group::UnixOwnedChild::spawn(command)?;
        let pid = inner.id();
        let (boot_id, process_start) = match super::process_birth_identity(pid) {
            Ok(Some(identity)) => identity,
            Ok(None) => {
                let reason =
                    "spawned process exited before its creation identity was captured".to_string();
                return match inner.terminate() {
                    Ok(_) => Err(reason),
                    Err(cleanup) => Err(format!("{reason}; cleanup failed: {cleanup}")),
                };
            }
            Err(reason) => {
                return match inner.terminate() {
                    Ok(_) => Err(reason),
                    Err(cleanup) => Err(format!("{reason}; cleanup failed: {cleanup}")),
                };
            }
        };
        Ok(Self {
            inner: PlatformOwnedChild::Unix(inner),
            identity: DurableProcessOwner {
                pid,
                container_id: pid.to_string(),
                process_start: format!("{boot_id}:{process_start}"),
                launch_nonce,
            },
        })
    }

    pub(super) fn identity(&self) -> DurableProcessOwner {
        self.identity.clone()
    }

    pub(super) fn try_wait(&mut self) -> Result<Option<ExitStatus>, String> {
        match &mut self.inner {
            PlatformOwnedChild::Unix(child) => child.try_wait(),
        }
    }

    pub(super) fn wait(&mut self) -> Result<ExitStatus, String> {
        match &mut self.inner {
            PlatformOwnedChild::Unix(child) => child.wait(),
        }
    }

    pub(super) fn terminate(&mut self) -> Result<ExitStatus, String> {
        match &mut self.inner {
            PlatformOwnedChild::Unix(child) => child.terminate(),
        }
    }
}

pub(super) fn recover_owner(_identity: &DurableProcessOwner) -> RecoveryDisposition {
    RecoveryDisposition::Quarantine
}

impl DurableProcessOwner {
    pub(super) fn document(&self, attempt_id: &str, intent_digest: &str) -> String {
        serde_json::json!({
            "schema": 1,
            "attempt_id": attempt_id,
            "intent_digest": intent_digest,
            "owner": {
                "pid": self.pid,
                "container_id": self.container_id,
                "process_start": self.process_start,
                "launch_nonce": self.launch_nonce,
            },
        })
        .to_string()
    }

    pub(super) fn from_document(
        body: &str,
        expected_attempt_id: &str,
        expected_intent_digest: &str,
    ) -> Result<Self, String> {
        let value: serde_json::Value = serde_json::from_str(body)
            .map_err(|error| format!("parse durable process owner: {error}"))?;
        if value.get("schema").and_then(serde_json::Value::as_u64) != Some(1)
            || value.get("attempt_id").and_then(serde_json::Value::as_str)
                != Some(expected_attempt_id)
            || value
                .get("intent_digest")
                .and_then(serde_json::Value::as_str)
                != Some(expected_intent_digest)
        {
            return Err("durable process owner differs from its invocation intent".to_string());
        }
        let owner = value
            .get("owner")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| "durable process owner is missing".to_string())?;
        let pid = owner
            .get("pid")
            .and_then(serde_json::Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok())
            .filter(|pid| *pid > 0)
            .ok_or_else(|| "durable process owner PID is invalid".to_string())?;
        let container_id = owner
            .get("container_id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| *id == pid.to_string())
            .ok_or_else(|| "durable process owner container ID is invalid".to_string())?
            .to_string();
        let process_start = owner
            .get("process_start")
            .and_then(serde_json::Value::as_str)
            .filter(|identity| !identity.is_empty())
            .ok_or_else(|| "durable process owner start identity is invalid".to_string())?
            .to_string();
        if !process_start
            .split_once(':')
            .is_some_and(|(boot, start)| !boot.is_empty() && !start.is_empty())
        {
            return Err("durable process owner creation identity is malformed".to_string());
        }
        let launch_nonce = owner
            .get("launch_nonce")
            .and_then(serde_json::Value::as_str)
            .filter(|nonce| !nonce.is_empty())
            .ok_or_else(|| "durable process owner launch nonce is invalid".to_string())?
            .to_string();
        if launch_nonce != expected_attempt_id {
            return Err("durable process owner launch nonce differs from its attempt".to_string());
        }
        let identity = Self {
            pid,
            container_id,
            process_start,
            launch_nonce,
        };
        if identity.document(expected_attempt_id, expected_intent_digest) != body {
            return Err("durable process owner is non-canonical or has extra fields".to_string());
        }
        Ok(identity)
    }

    #[cfg(test)]
    fn fixture_for_current_process() -> Self {
        let (boot_id, process_start) = super::process_birth_identity(std::process::id())
            .expect("observe current process identity")
            .expect("current process is live");
        Self {
            pid: std::process::id(),
            container_id: std::process::id().to_string(),
            process_start: format!("{boot_id}:{process_start}"),
            launch_nonce: "fixture-nonce".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::thread;
    use std::time::Duration;

    fn shell_command(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg(script);
        command
    }

    #[test]
    fn wait_preserves_zero_exit() {
        let mut command = shell_command("sleep 0.05; exit 0");
        let mut owned = OwnedChildTree::spawn(&mut command, "nonce-zero".into()).unwrap();
        assert!(owned.wait().expect("wait for owned child").success());
    }

    #[test]
    fn try_wait_preserves_nonzero_exit() {
        let mut command = shell_command("sleep 0.05; exit 17");
        let mut owned = OwnedChildTree::spawn(&mut command, "nonce-nonzero".into()).unwrap();
        let status = loop {
            if let Some(status) = owned.try_wait().expect("poll owned child") {
                break status;
            }
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(status.code(), Some(17));
    }

    #[test]
    fn terminate_reaps_the_owned_process_group() {
        let mut command = shell_command("sleep 30 & wait");
        let mut owned = OwnedChildTree::spawn(&mut command, "nonce-a".into()).unwrap();
        let identity = owned.identity();
        let status = owned.terminate().expect("terminate owned group");
        assert!(!status.success());
        assert_eq!(identity.pid.to_string(), identity.container_id);
    }

    #[test]
    fn terminate_signals_descendants_in_the_owned_process_group() {
        let marker = std::env::temp_dir().join(format!(
            "autospec-owned-descendant-{}-{}",
            std::process::id(),
            super::super::DIRECT_TRANSACTION_SEQUENCE
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let mut command = shell_command(
            "sh -c 'trap \"printf term >> \\\"$AUTOSPEC_DESCENDANT_MARKER\\\"; exit 0\" TERM; printf ready > \"$AUTOSPEC_DESCENDANT_MARKER\"; while :; do sleep 1; done' & wait",
        );
        command.env("AUTOSPEC_DESCENDANT_MARKER", &marker);
        let mut owned = OwnedChildTree::spawn(&mut command, "nonce-tree".into()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !marker.exists() && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(marker.exists(), "descendant did not publish readiness");
        let _ = owned.terminate().expect("terminate owned descendant tree");
        assert_eq!(
            std::fs::read_to_string(&marker).expect("descendant signal marker"),
            "readyterm"
        );
        std::fs::remove_file(marker).expect("remove descendant signal marker");
    }

    #[test]
    fn durable_identity_without_live_owner_cannot_signal() {
        let identity = DurableProcessOwner::fixture_for_current_process();
        assert_eq!(recover_owner(&identity), RecoveryDisposition::Quarantine);
    }
}
