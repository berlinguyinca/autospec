use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use autospec_core::autonomous::waterfall::sha256_hex;
use autospec_core::managed_project::{ManagedProjectBinding, ProductKey};
use serde_json::{json, Value};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

#[path = "managed_project/github.rs"]
mod github;
#[path = "managed_project/store.rs"]
mod store;

pub use github::{
    reconcile_issue, resolve_or_create_project, retry_pending_projections, verify_managed_marker,
};
pub use store::ManagedProjectStore;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteProject {
    pub node_id: String,
    pub number: u64,
    pub url: String,
    pub title: String,
    pub owner: String,
    pub readme: String,
}

#[derive(Debug)]
pub struct ManagedProjectError(String);

impl ManagedProjectError {
    pub(super) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ManagedProjectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ManagedProjectError {}

impl ManagedProjectStore {
    pub fn record_project(
        &mut self,
        owner: &str,
        node_id: &str,
        number: u64,
        url: &str,
        title: &str,
    ) -> Result<(), ManagedProjectError> {
        if [owner, node_id, url, title]
            .iter()
            .any(|value| value.trim().is_empty())
            || number == 0
        {
            return Err(ManagedProjectError::new(
                "managed project identity fields must not be empty",
            ));
        }
        let _lock = ProductLock::acquire(&self.root.join(store::LOCK_FILE))?;
        self.refresh_from_journal()?;
        let payload = json!({
            "owner": owner,
            "node_id": node_id,
            "number": number,
            "url": url,
            "title": title,
        });
        if self.binding.project_node_id.is_some()
            && project_binding_payload(&self.binding) != Some(payload.clone())
        {
            return Err(ManagedProjectError::new(
                "managed project binding conflicts with the verified remote project",
            ));
        }
        self.append_event_locked(
            format!("project:bind:{}", self.product_key.as_str()),
            "project-bound",
            payload,
        )
    }
}

fn apply_project_binding(
    binding: &mut ManagedProjectBinding,
    payload: &Value,
) -> Result<(), ManagedProjectError> {
    let object = payload
        .as_object()
        .ok_or_else(|| ManagedProjectError::new("project binding payload must be an object"))?;
    let string = |field: &str| {
        object
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                ManagedProjectError::new(format!("project binding payload has invalid {field}"))
            })
    };
    binding.owner = Some(string("owner")?);
    binding.project_node_id = Some(string("node_id")?);
    binding.project_number = Some(
        object
            .get("number")
            .and_then(Value::as_u64)
            .filter(|number| *number > 0)
            .ok_or_else(|| {
                ManagedProjectError::new("project binding payload has invalid number")
            })?,
    );
    binding.project_url = Some(string("url")?);
    binding.project_title = Some(string("title")?);
    Ok(())
}

fn project_binding_payload(binding: &ManagedProjectBinding) -> Option<Value> {
    Some(json!({
        "owner": binding.owner.as_deref()?,
        "node_id": binding.project_node_id.as_deref()?,
        "number": binding.project_number?,
        "url": binding.project_url.as_deref()?,
        "title": binding.project_title.as_deref()?,
    }))
}

pub(super) struct JournalCheckpoint {
    pub(super) high_watermark: u64,
    pub(super) digest: String,
}

pub(super) struct PersistedBinding {
    pub(super) binding: ManagedProjectBinding,
    pub(super) checkpoint: Option<JournalCheckpoint>,
}

pub(super) fn read_persisted_binding(
    path: &Path,
    product_key: &ProductKey,
) -> Result<PersistedBinding, ManagedProjectError> {
    let value: Value = serde_json::from_str(&read_private_file(path)?).map_err(|error| {
        ManagedProjectError::new(format!("invalid managed project binding: {error}"))
    })?;
    let binding: ManagedProjectBinding =
        serde_json::from_value(value.clone()).map_err(|error| {
            ManagedProjectError::new(format!("invalid managed project binding: {error}"))
        })?;
    validate_binding_identity(&binding, product_key)?;
    let high_watermark = value.get("journal_high_watermark").and_then(Value::as_u64);
    let digest = value.get("journal_digest").and_then(Value::as_str);
    let checkpoint = match (high_watermark, digest) {
        (None, None) => None,
        (Some(high_watermark), Some(digest))
            if digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()) =>
        {
            Some(JournalCheckpoint {
                high_watermark,
                digest: digest.to_owned(),
            })
        }
        _ => {
            return Err(ManagedProjectError::new(
                "managed project binding has invalid journal checkpoint metadata",
            ))
        }
    };
    Ok(PersistedBinding {
        binding,
        checkpoint,
    })
}

pub(super) fn binding_document(
    binding: &ManagedProjectBinding,
    checkpoint: &JournalCheckpoint,
) -> Result<Vec<u8>, ManagedProjectError> {
    let mut value = serde_json::to_value(binding).map_err(ManagedProjectError::from)?;
    let object = value
        .as_object_mut()
        .expect("managed project binding serializes as an object");
    object.insert(
        "journal_high_watermark".to_owned(),
        Value::from(checkpoint.high_watermark),
    );
    object.insert(
        "journal_digest".to_owned(),
        Value::from(checkpoint.digest.clone()),
    );
    let mut document = serde_json::to_vec_pretty(&value).map_err(ManagedProjectError::from)?;
    document.push(b'\n');
    Ok(document)
}

pub(super) fn validate_replay_checkpoint(
    persisted: Option<&PersistedBinding>,
    replayed: &ManagedProjectBinding,
    prefix_digests: &[String],
) -> Result<(), ManagedProjectError> {
    let Some(persisted) = persisted else {
        return Ok(());
    };
    let Some(checkpoint) = &persisted.checkpoint else {
        return if persisted.binding == *replayed {
            Ok(())
        } else {
            Err(ManagedProjectError::new(
                "managed project binding without a checkpoint disagrees with its journal",
            ))
        };
    };
    let index = usize::try_from(checkpoint.high_watermark)
        .map_err(|_| ManagedProjectError::new("journal checkpoint high-watermark is too large"))?;
    let digest = prefix_digests.get(index).ok_or_else(|| {
        ManagedProjectError::new("managed project journal is behind the binding high-watermark")
    })?;
    if digest != &checkpoint.digest {
        return Err(ManagedProjectError::new(
            "managed project journal does not match the binding checkpoint",
        ));
    }
    Ok(())
}

pub(super) fn snapshot_needs_update(
    persisted: Option<&PersistedBinding>,
    replayed: &ManagedProjectBinding,
    high_watermark: u64,
    digest: &str,
) -> bool {
    persisted.is_none_or(|persisted| {
        persisted.binding != *replayed
            || persisted.checkpoint.as_ref().is_none_or(|checkpoint| {
                checkpoint.high_watermark != high_watermark || checkpoint.digest != digest
            })
    })
}

pub(super) fn empty_journal_digest() -> String {
    sha256_hex(b"managed-project-journal-v1")
}

pub(super) fn extend_journal_digest(prior: &str, line: &[u8]) -> String {
    let mut input = Vec::with_capacity(prior.len() + 1 + line.len());
    input.extend_from_slice(prior.as_bytes());
    input.push(0);
    input.extend_from_slice(line);
    sha256_hex(&input)
}

fn validate_binding_identity(
    binding: &ManagedProjectBinding,
    product_key: &ProductKey,
) -> Result<(), ManagedProjectError> {
    if binding.schema_version != ManagedProjectBinding::SCHEMA_VERSION {
        return Err(ManagedProjectError::new(
            "unsupported managed project binding schema",
        ));
    }
    if &binding.product_key != product_key {
        return Err(ManagedProjectError::new(
            "managed project binding product key does not match its state directory",
        ));
    }
    Ok(())
}

static TEMP_SERIAL: AtomicU64 = AtomicU64::new(1);

pub(super) fn append_synced_line(
    path: &Path,
    line: &[u8],
    fail_after: Option<usize>,
) -> Result<(), ManagedProjectError> {
    reject_unsafe_file(path)?;
    let mut file = open_private_file(path)?;
    let original_length = file.metadata().map_err(io_error)?.len();
    file.seek(SeekFrom::End(0)).map_err(io_error)?;
    let write_result = if let Some(limit) = fail_after {
        let partial = limit.min(line.len().saturating_sub(1));
        file.write_all(&line[..partial])
            .and_then(|_| Err(std::io::Error::other("injected partial journal append")))
    } else {
        file.write_all(line)
    };
    if let Err(error) = write_result {
        if let Err(rollback) = file
            .set_len(original_length)
            .and_then(|_| file.seek(SeekFrom::Start(original_length)).map(|_| ()))
            .and_then(|_| file.sync_all())
        {
            return Err(ManagedProjectError::new(format!(
                "journal append failed ({error}) and rollback failed ({rollback})"
            )));
        }
        return Err(io_error(error));
    }
    file.sync_all().map_err(io_error)
}

pub(super) fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), ManagedProjectError> {
    reject_unsafe_file(path)?;
    let serial = TEMP_SERIAL.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_extension(format!("tmp-{}-{serial}", std::process::id()));
    let mut file = private_options()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(io_error)?;
    validate_open_file(&file)?;
    if let Err(error) = file.write_all(contents).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(io_error(error));
    }
    fs::rename(&temporary, path).map_err(io_error)?;
    File::open(path.parent().expect("managed project file has parent"))
        .and_then(|directory| directory.sync_all())
        .map_err(io_error)
}

pub(super) fn ensure_private_directory(path: &Path) -> Result<(), ManagedProjectError> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ManagedProjectError::new(
                "managed project state path is not a safe directory",
            ));
        }
        validate_owner(&metadata)?;
        #[cfg(unix)]
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(ManagedProjectError::new(
                "managed project state directory permissions must be private",
            ));
        }
    } else {
        #[cfg(unix)]
        fs::DirBuilder::new()
            .mode(0o700)
            .create(path)
            .map_err(io_error)?;
        #[cfg(not(unix))]
        fs::create_dir(path).map_err(io_error)?;
    }
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
    Ok(())
}

pub(super) fn ensure_private_file(path: &Path) -> Result<(), ManagedProjectError> {
    reject_unsafe_file(path)?;
    let file = private_options()
        .create(true)
        .append(true)
        .open(path)
        .map_err(io_error)?;
    validate_open_file(&file)
}

pub(super) fn reject_unsafe_file(path: &Path) -> Result<(), ManagedProjectError> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(ManagedProjectError::new(
                "managed project state file is not a safe regular file",
            ));
        }
        validate_owner(&metadata)?;
        #[cfg(unix)]
        if metadata.permissions().mode() & 0o777 != 0o600 {
            return Err(ManagedProjectError::new(
                "managed project state file permissions must be 0600",
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn validate_owner(metadata: &fs::Metadata) -> Result<(), ManagedProjectError> {
    if metadata.uid() != nix::unistd::geteuid().as_raw() {
        return Err(ManagedProjectError::new(
            "managed project state ownership mismatch",
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_owner(_metadata: &fs::Metadata) -> Result<(), ManagedProjectError> {
    Ok(())
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    options
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC);
    options
}

pub(super) fn read_private_file(path: &Path) -> Result<String, ManagedProjectError> {
    let mut file = private_options().read(true).open(path).map_err(io_error)?;
    validate_open_file(&file)?;
    let mut document = String::new();
    file.read_to_string(&mut document).map_err(io_error)?;
    Ok(document)
}

pub(super) fn open_private_file(path: &Path) -> Result<File, ManagedProjectError> {
    let file = private_options()
        .read(true)
        .write(true)
        .open(path)
        .map_err(io_error)?;
    validate_open_file(&file)?;
    Ok(file)
}

pub(super) struct ProductLock(File);

impl ProductLock {
    pub(super) fn acquire(path: &Path) -> Result<Self, ManagedProjectError> {
        ensure_private_file(path)?;
        let file = open_private_file(path)?;
        file.lock().map_err(|error| {
            ManagedProjectError::new(format!("cannot lock managed project state: {error}"))
        })?;
        Ok(Self(file))
    }
}

impl Drop for ProductLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn validate_open_file(file: &File) -> Result<(), ManagedProjectError> {
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() {
        return Err(ManagedProjectError::new(
            "managed project descriptor is not a regular file",
        ));
    }
    validate_owner(&metadata)?;
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o777 != 0o600 {
        return Err(ManagedProjectError::new(
            "managed project descriptor permissions must be 0600",
        ));
    }
    Ok(())
}

pub(super) fn io_error(error: std::io::Error) -> ManagedProjectError {
    ManagedProjectError::new(error.to_string())
}
