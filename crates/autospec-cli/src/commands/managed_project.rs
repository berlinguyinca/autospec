use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

#[path = "managed_project/store.rs"]
mod store;

pub use store::ManagedProjectStore;

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

static TEMP_SERIAL: AtomicU64 = AtomicU64::new(1);

pub(super) fn append_synced_line(
    path: &Path,
    value: &serde_json::Value,
) -> Result<(), ManagedProjectError> {
    reject_unsafe_file(path)?;
    let mut file = private_options()
        .append(true)
        .create(true)
        .open(path)
        .map_err(io_error)?;
    validate_open_file(&file)?;
    serde_json::to_writer(&mut file, value).map_err(ManagedProjectError::from)?;
    file.write_all(b"\n").map_err(io_error)?;
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
