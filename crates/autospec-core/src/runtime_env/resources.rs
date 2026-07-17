use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use getrandom::fill;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{EnvironmentIdentity, RuntimeEnvError};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResourcePlan {
    pub schema_version: u32,
    pub digest: String,
    pub identity: EnvironmentIdentity,
    pub maven: Option<MavenPlan>,
    pub compose: Option<ComposePlan>,
}

#[derive(Serialize)]
struct ResourcePlanContent<'a> {
    schema_version: u32,
    identity: &'a EnvironmentIdentity,
    maven: &'a Option<MavenPlan>,
    compose: &'a Option<ComposePlan>,
}

impl ResourcePlan {
    pub fn new(
        identity: EnvironmentIdentity,
        maven: Option<MavenPlan>,
        compose: Option<ComposePlan>,
    ) -> Result<Self, RuntimeEnvError> {
        let content = ResourcePlanContent {
            schema_version: 1,
            identity: &identity,
            maven: &maven,
            compose: &compose,
        };
        let encoded = serde_json::to_vec(&content).map_err(|error| {
            RuntimeEnvError::new(format!("could not encode runtime resource plan: {error}"))
        })?;
        Ok(Self {
            schema_version: 1,
            digest: hex_digest(Sha256::digest(encoded).as_slice()),
            identity,
            maven,
            compose,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MavenPlan {
    pub isolation: MavenIsolation,
    pub local_prefix: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MavenIsolation {
    SplitLocal,
    Off,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ComposeIsolation {
    Managed,
    Off,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ExportProtocol {
    Http,
    Https,
    Tcp,
    Udp,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ExportValue {
    Url,
    Port,
    HostPort,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComposeExport {
    pub service: String,
    pub target: u16,
    pub protocol: ExportProtocol,
    pub env: String,
    pub value: ExportValue,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComposePlan {
    pub isolation: ComposeIsolation,
    pub files: Vec<PathBuf>,
    pub project_name: String,
    pub exports: Vec<ComposeExport>,
    pub preserve_volumes: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OwnedVolume {
    pub logical_key: Option<String>,
    pub id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedExport {
    pub env: String,
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResourceInventory {
    pub schema_version: u32,
    pub environment_id: String,
    pub compose_project: Option<String>,
    pub containers: Vec<String>,
    pub networks: Vec<String>,
    pub volumes: Vec<OwnedVolume>,
    pub exports: Vec<ResolvedExport>,
    pub maven_local_prefix: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentOwner {
    pub schema_version: u32,
    pub identity: EnvironmentIdentity,
    pub host: String,
    pub created_at_unix_ms: u64,
    pub manifest_digest: String,
    pub lifecycle: EnvironmentLifecycle,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum EnvironmentLifecycle {
    Planned,
    Provisioning,
    Active,
    TearingDown,
    CleanupFailed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub schema_version: u32,
    pub session_id: String,
    pub pid: u32,
    pub process_start: String,
    pub harness: String,
    pub host: String,
    pub started_at_unix_ms: u64,
    pub heartbeat_at_unix_ms: u64,
}

pub fn write_json_atomic<T: Serialize + ?Sized>(
    path: &Path,
    value: &T,
) -> Result<(), RuntimeEnvError> {
    let parent = path.parent().ok_or_else(|| {
        RuntimeEnvError::new(format!(
            "runtime state path has no parent: {}",
            path.display()
        ))
    })?;
    std::fs::create_dir_all(parent).map_err(|error| {
        RuntimeEnvError::new(format!(
            "could not create runtime state directory {}: {error}",
            parent.display()
        ))
    })?;
    let encoded = serde_json::to_vec(value).map_err(|error| {
        RuntimeEnvError::new(format!(
            "could not encode runtime state {}: {error}",
            path.display()
        ))
    })?;
    let temporary = create_temporary_path(path)?;
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| {
                RuntimeEnvError::new(format!(
                    "could not create runtime state {}: {error}",
                    temporary.display()
                ))
            })?;
        file.write_all(&encoded)
            .and_then(|()| file.sync_all())
            .map_err(|error| {
                RuntimeEnvError::new(format!(
                    "could not write runtime state {}: {error}",
                    temporary.display()
                ))
            })?;
        std::fs::rename(&temporary, path).map_err(|error| {
            RuntimeEnvError::new(format!(
                "could not finalize runtime state {}: {error}",
                path.display()
            ))
        })?;
        std::fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| {
                RuntimeEnvError::new(format!(
                    "could not synchronize runtime state directory {}: {error}",
                    parent.display()
                ))
            })
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, RuntimeEnvError> {
    let bytes = std::fs::read(path).map_err(|error| {
        RuntimeEnvError::new(format!(
            "could not read runtime state {}: {error}",
            path.display()
        ))
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        RuntimeEnvError::new(format!(
            "could not parse runtime state {}: {error}",
            path.display()
        ))
    })
}

fn create_temporary_path(path: &Path) -> Result<PathBuf, RuntimeEnvError> {
    let mut random = [0_u8; 16];
    fill(&mut random).map_err(|error| {
        RuntimeEnvError::new(format!(
            "could not generate runtime state temporary name: {error}"
        ))
    })?;
    let suffix = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            RuntimeEnvError::new(format!(
                "runtime state path has no UTF-8 filename: {}",
                path.display()
            ))
        })?;
    Ok(path.with_file_name(format!(".{name}.{suffix}.tmp")))
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
