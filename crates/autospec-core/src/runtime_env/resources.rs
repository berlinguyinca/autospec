use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::ffi::CString;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use getrandom::fill;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
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
        let mut plan = Self {
            schema_version: 1,
            digest: String::new(),
            identity,
            maven,
            compose,
        };
        plan.refresh_digest()?;
        Ok(plan)
    }

    fn refresh_digest(&mut self) -> Result<(), RuntimeEnvError> {
        let content = ResourcePlanContent {
            schema_version: 1,
            identity: &self.identity,
            maven: &self.maven,
            compose: &self.compose,
        };
        let encoded = serde_json::to_vec(&content).map_err(|error| {
            RuntimeEnvError::new(format!("could not encode runtime resource plan: {error}"))
        })?;
        self.digest = hex_digest(Sha256::digest(encoded).as_slice());
        Ok(())
    }

    pub fn apply_invocation_overrides(
        &mut self,
        maven_value: Option<&str>,
        compose_value: Option<&str>,
        whole_environment_disabled: bool,
    ) -> Result<bool, RuntimeEnvError> {
        Self::validate_invocation_override_values(maven_value, compose_value)?;

        if whole_environment_disabled || maven_value == Some("off") {
            if let Some(maven) = &mut self.maven {
                maven.isolation = MavenIsolation::Off;
            }
        }
        if whole_environment_disabled || compose_value == Some("off") {
            if let Some(compose) = &mut self.compose {
                compose.isolation = ComposeIsolation::Off;
            }
        }

        self.refresh_digest()?;

        Ok(whole_environment_disabled || maven_value.is_some() || compose_value.is_some())
    }

    pub fn validate_invocation_override_values(
        maven_value: Option<&str>,
        compose_value: Option<&str>,
    ) -> Result<(), RuntimeEnvError> {
        validate_off_override("AUTOSPEC_MAVEN_ISOLATION", maven_value)?;
        validate_off_override("AUTOSPEC_COMPOSE_ISOLATION", compose_value)
    }
}

fn validate_off_override(key: &str, value: Option<&str>) -> Result<(), RuntimeEnvError> {
    match value {
        None | Some("off") => Ok(()),
        Some(_) => Err(RuntimeEnvError::new(format!("{key} must be 'off'"))),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeResources {
    pub maven: MavenResourceConfig,
    pub compose: ComposeResourceConfig,
}

impl Default for RuntimeResources {
    fn default() -> Self {
        Self {
            maven: MavenResourceConfig {
                isolation: MavenIsolation::SplitLocal,
            },
            compose: ComposeResourceConfig {
                isolation: ComposeIsolation::Managed,
                files: Vec::new(),
                exports: Vec::new(),
                preserve_volumes: Vec::new(),
                shared_networks: Vec::new(),
                shared_volumes: Vec::new(),
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MavenResourceConfig {
    pub isolation: MavenIsolation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComposeResourceConfig {
    pub isolation: ComposeIsolation,
    pub files: Vec<PathBuf>,
    pub exports: Vec<ComposeExport>,
    pub preserve_volumes: Vec<String>,
    pub shared_networks: Vec<String>,
    pub shared_volumes: Vec<String>,
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

impl ExportProtocol {
    pub(super) fn parse(value: &str) -> Result<Self, RuntimeEnvError> {
        match value {
            "http" => Ok(Self::Http),
            "https" => Ok(Self::Https),
            "tcp" => Ok(Self::Tcp),
            "udp" => Ok(Self::Udp),
            value => Err(RuntimeEnvError::new(format!(
                "unsupported Compose export protocol: {value}"
            ))),
        }
    }
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
    #[serde(default)]
    pub shared_networks: Vec<String>,
    #[serde(default)]
    pub shared_volumes: Vec<String>,
}

impl ComposePlan {
    pub fn canonical_url_export_index(&self) -> Result<Option<usize>, RuntimeEnvError> {
        let explicit = self
            .exports
            .iter()
            .enumerate()
            .filter(|(_, export)| export.env == "AUTOSPEC_PUBLIC_URL")
            .collect::<Vec<_>>();
        if let [(index, export)] = explicit.as_slice() {
            if matches!(
                export.protocol,
                ExportProtocol::Http | ExportProtocol::Https
            ) && export.value == ExportValue::Url
            {
                return Ok(Some(*index));
            }
        }
        if !explicit.is_empty() {
            return Err(RuntimeEnvError::new(
                "COMPOSE_CANONICAL_URL_INVALID: AUTOSPEC_PUBLIC_URL must be one URL-valued HTTP(S) export",
            ));
        }
        let candidates = self
            .exports
            .iter()
            .enumerate()
            .filter(|(_, export)| {
                matches!(
                    export.protocol,
                    ExportProtocol::Http | ExportProtocol::Https
                )
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        match candidates.as_slice() {
            [] => Ok(None),
            [index] => Ok(Some(*index)),
            _ => Err(RuntimeEnvError::new(
                "COMPOSE_CANONICAL_URL_AMBIGUOUS: declare exactly one HTTP(S) export or AUTOSPEC_PUBLIC_URL",
            )),
        }
    }
}

pub(super) fn validate_logical_keys(
    values: &[String],
    message: &str,
) -> Result<(), RuntimeEnvError> {
    let valid = |value: &String| {
        !value.is_empty()
            && value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "_.-".contains(character))
            && value
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_alphanumeric())
    };
    if values.iter().all(valid) {
        Ok(())
    } else {
        Err(RuntimeEnvError::new(message))
    }
}

pub(super) fn reject_duplicates<T: Eq + std::hash::Hash>(
    values: &[T],
    message: &str,
) -> Result<(), RuntimeEnvError> {
    let mut seen = std::collections::HashSet::new();
    if values.iter().any(|value| !seen.insert(value)) {
        Err(RuntimeEnvError::new(message))
    } else {
        Ok(())
    }
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

impl ResolvedExport {
    pub fn render(&self, declaration: &ComposeExport) -> Result<String, RuntimeEnvError> {
        if self.env != declaration.env || self.host != "127.0.0.1" || self.port == 0 {
            return Err(RuntimeEnvError::new(
                "resolved Compose export does not match its validated declaration",
            ));
        }
        match (&declaration.protocol, &declaration.value) {
            (ExportProtocol::Http, ExportValue::Url) => {
                Ok(format!("http://{}:{}", self.host, self.port))
            }
            (ExportProtocol::Https, ExportValue::Url) => {
                Ok(format!("https://{}:{}", self.host, self.port))
            }
            (ExportProtocol::Tcp | ExportProtocol::Udp, ExportValue::Port) => {
                Ok(self.port.to_string())
            }
            (_, ExportValue::HostPort) => Ok(format!("{}:{}", self.host, self.port)),
            _ => Err(RuntimeEnvError::new(
                "resolved Compose export has an incompatible value type",
            )),
        }
    }

    pub fn canonical_url(&self, declaration: &ComposeExport) -> Result<String, RuntimeEnvError> {
        if self.env != declaration.env || self.host != "127.0.0.1" || self.port == 0 {
            return Err(RuntimeEnvError::new(
                "resolved Compose export does not match its validated declaration",
            ));
        }
        match declaration.protocol {
            ExportProtocol::Http => Ok(format!("http://{}:{}", self.host, self.port)),
            ExportProtocol::Https => Ok(format!("https://{}:{}", self.host, self.port)),
            ExportProtocol::Tcp | ExportProtocol::Udp => Err(RuntimeEnvError::new(
                "resolved Compose export cannot provide a canonical HTTP(S) URL",
            )),
        }
    }
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
    #[serde(default)]
    pub frontend_port: Option<u16>,
    #[serde(default)]
    pub backend_port: Option<u16>,
    #[serde(default)]
    pub maven_arguments: Option<String>,
    #[serde(default)]
    pub initial_overrides: Vec<(String, String)>,
    pub maven_local_prefix: Option<PathBuf>,
}

impl ResourceInventory {
    pub fn deletable_volumes(&self, preserved: &[String]) -> Vec<String> {
        self.volumes
            .iter()
            .filter(|volume| {
                volume
                    .logical_key
                    .as_ref()
                    .is_none_or(|key| !preserved.contains(key))
            })
            .map(|volume| volume.id.clone())
            .collect()
    }
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
    let encoded = serde_json::to_vec(value).map_err(|error| {
        RuntimeEnvError::new(format!(
            "could not encode runtime state {}: {error}",
            path.display()
        ))
    })?;
    write_file_atomic(path, &encoded)
}

pub fn write_file_atomic(path: &Path, encoded: &[u8]) -> Result<(), RuntimeEnvError> {
    let parent = path.parent().ok_or_else(|| {
        RuntimeEnvError::new(format!(
            "runtime state path has no parent: {}",
            path.display()
        ))
    })?;
    let parent_directory = open_private_directory(parent)?;

    #[cfg(unix)]
    {
        let destination = path.file_name().ok_or_else(|| {
            RuntimeEnvError::new(format!(
                "runtime state path has no filename: {}",
                path.display()
            ))
        })?;
        write_file_atomic_at(&parent_directory, Path::new(destination), encoded)
    }

    #[cfg(not(unix))]
    write_file_atomic_by_path(path, parent, &parent_directory, encoded)
}

#[cfg(not(unix))]
fn write_file_atomic_by_path(
    path: &Path,
    parent: &Path,
    parent_directory: &std::fs::File,
    encoded: &[u8],
) -> Result<(), RuntimeEnvError> {
    let temporary = create_temporary_path(path)?;
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut file = options.open(&temporary).map_err(|error| {
            RuntimeEnvError::new(format!(
                "could not create runtime state {}: {error}",
                temporary.display()
            ))
        })?;
        set_private_descriptor_mode(&file, 0o600)?;
        file.write_all(encoded)
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
        parent_directory.sync_all().map_err(|error| {
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

#[cfg(unix)]
fn write_file_atomic_at(
    parent: &std::fs::File,
    destination: &Path,
    encoded: &[u8],
) -> Result<(), RuntimeEnvError> {
    let temporary = create_temporary_path(destination)?;
    let destination_name = relative_cstring(destination)?;
    let temporary_name = relative_cstring(&temporary)?;
    let result = (|| {
        let flags =
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW;
        // SAFETY: the directory descriptor and NUL-terminated name are valid for
        // this call; ownership of a successful descriptor transfers to File.
        let descriptor = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                temporary_name.as_ptr(),
                flags,
                0o600 as libc::mode_t,
            )
        };
        if descriptor < 0 {
            return Err(RuntimeEnvError::new(format!(
                "could not create runtime state {}: {}",
                temporary.display(),
                std::io::Error::last_os_error()
            )));
        }
        // SAFETY: openat returned a new owned descriptor and File assumes its
        // sole ownership exactly once.
        let mut file = unsafe { std::fs::File::from_raw_fd(descriptor) };
        set_private_descriptor_mode(&file, 0o600)?;
        file.write_all(encoded)
            .and_then(|()| file.sync_all())
            .map_err(|error| {
                RuntimeEnvError::new(format!(
                    "could not write runtime state {}: {error}",
                    temporary.display()
                ))
            })?;
        // SAFETY: both names are valid C strings and both directory descriptors
        // remain open for the duration of the atomic promotion.
        let promoted = unsafe {
            libc::renameat(
                parent.as_raw_fd(),
                temporary_name.as_ptr(),
                parent.as_raw_fd(),
                destination_name.as_ptr(),
            )
        };
        if promoted != 0 {
            return Err(RuntimeEnvError::new(format!(
                "could not finalize runtime state {}: {}",
                destination.display(),
                std::io::Error::last_os_error()
            )));
        }
        parent.sync_all().map_err(|error| {
            RuntimeEnvError::new(format!(
                "could not synchronize runtime state directory: {error}"
            ))
        })
    })();
    if result.is_err() {
        // SAFETY: the descriptor and temporary name remain valid; cleanup is
        // best-effort and never follows a symlink outside the held directory.
        unsafe {
            libc::unlinkat(parent.as_raw_fd(), temporary_name.as_ptr(), 0);
        }
    }
    result
}

#[cfg(unix)]
fn relative_cstring(path: &Path) -> Result<CString, RuntimeEnvError> {
    if path.components().count() != 1 {
        return Err(RuntimeEnvError::new(format!(
            "runtime state descriptor path is not a filename: {}",
            path.display()
        )));
    }
    CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        RuntimeEnvError::new(format!(
            "runtime state filename contains NUL: {}",
            path.display()
        ))
    })
}

fn open_private_directory(path: &Path) -> Result<std::fs::File, RuntimeEnvError> {
    std::fs::create_dir_all(path).map_err(|error| {
        RuntimeEnvError::new(format!(
            "could not create runtime state directory {}: {error}",
            path.display()
        ))
    })?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW);
    let directory = options.open(path).map_err(|error| {
        #[cfg(unix)]
        if error.raw_os_error() == Some(libc::ELOOP)
            || (error.raw_os_error() == Some(libc::ENOTDIR)
                && std::fs::symlink_metadata(path)
                    .map(|metadata| metadata.file_type().is_symlink())
                    .unwrap_or(false))
        {
            return RuntimeEnvError::new(format!(
                "RUNTIME_STATE_SYMLINK_REJECTED: {}",
                path.display()
            ));
        }
        RuntimeEnvError::new(format!(
            "could not securely open runtime state directory {}: {error}",
            path.display()
        ))
    })?;
    if !directory
        .metadata()
        .map_err(|error| {
            RuntimeEnvError::new(format!(
                "could not inspect runtime state directory {}: {error}",
                path.display()
            ))
        })?
        .is_dir()
    {
        return Err(RuntimeEnvError::new(format!(
            "runtime state path is not a directory: {}",
            path.display()
        )));
    }
    set_private_descriptor_mode(&directory, 0o700)?;
    Ok(directory)
}

#[cfg(unix)]
fn set_private_descriptor_mode(file: &std::fs::File, mode: u32) -> Result<(), RuntimeEnvError> {
    file.set_permissions(std::fs::Permissions::from_mode(mode))
        .map_err(|error| RuntimeEnvError::new(format!("could not secure runtime state: {error}")))
}

#[cfg(not(unix))]
fn set_private_descriptor_mode(_file: &std::fs::File, _mode: u32) -> Result<(), RuntimeEnvError> {
    Ok(())
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

#[cfg(all(test, unix))]
mod descriptor_tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn atomic_write_stays_in_the_open_parent_after_path_substitution() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "autospec-runtime-openat-{}-{unique}",
            std::process::id()
        ));
        let parent = root.join("state");
        let retained = root.join("retained-state");
        let external = root.join("external-state");
        std::fs::create_dir_all(&parent).unwrap();
        std::fs::create_dir(&external).unwrap();
        let parent_directory = open_private_directory(&parent).unwrap();
        std::fs::rename(&parent, &retained).unwrap();
        symlink(&external, &parent).unwrap();

        write_file_atomic_at(&parent_directory, Path::new("env"), b"retained").unwrap();

        assert_eq!(std::fs::read(retained.join("env")).unwrap(), b"retained");
        assert!(!external.join("env").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
