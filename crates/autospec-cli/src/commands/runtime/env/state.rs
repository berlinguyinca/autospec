use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use autospec_core::runtime_env::{
    reserve_loopback_port, EnvironmentLifecycle, EnvironmentOwner, PortRegistry, PortReservation,
    ResourceInventory, ResourcePlan, RuntimeContext, RuntimeState,
};
#[cfg(unix)]
use nix::fcntl::{openat, OFlag};
#[cfg(unix)]
use nix::sys::stat::Mode;

use crate::commands::CommandFailure;

pub(super) use autospec_core::runtime_env::{read_json, write_json_atomic};

const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
const PRIVATE_FILE_MODE: u32 = 0o600;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct StateLayout {
    pub(super) environment_dir: PathBuf,
    pub(super) owner: PathBuf,
    pub(super) plan: PathBuf,
    pub(super) env: PathBuf,
    pub(super) inventory: PathBuf,
    pub(super) lease: PathBuf,
    pub(super) sessions: PathBuf,
}

impl StateLayout {
    pub(super) fn new(root: &Path, environment_id: &str) -> Self {
        let environment_dir = root.join(environment_id);
        Self {
            owner: environment_dir.join("owner.json"),
            plan: environment_dir.join("plan.json"),
            env: environment_dir.join("env"),
            inventory: environment_dir.join("inventory.json"),
            lease: environment_dir.join("lease.lock"),
            sessions: environment_dir.join("sessions"),
            environment_dir,
        }
    }

    fn prepare(&self) -> Result<(), CommandFailure> {
        let root = self
            .environment_dir
            .parent()
            .expect("runtime environment directory has a state root");
        ensure_private_directory(root)?;
        ensure_private_directory(&self.environment_dir)?;
        ensure_private_directory(&self.sessions)
    }
}

pub(super) struct AuthoritativeState {
    pub(super) owner: EnvironmentOwner,
    pub(super) plan: ResourcePlan,
    pub(super) inventory: ResourceInventory,
}

pub(super) fn layout_for_context(context: &RuntimeContext) -> StateLayout {
    let root = context
        .environment_dir
        .parent()
        .expect("runtime environment directory has a state root");
    StateLayout::new(root, &context.environment_id)
}

pub(super) fn validate_private_regular_file(path: &Path) -> Result<(), CommandFailure> {
    reject_symlink(path)?;
    let metadata = fs::metadata(path).map_err(io_error)?;
    if !metadata.is_file() {
        return Err(CommandFailure::diagnostic(format!(
            "runtime state is not a regular file: {}",
            path.display()
        )));
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o777 != PRIVATE_FILE_MODE {
        return Err(CommandFailure::diagnostic(format!(
            "runtime state file is not private: {}",
            path.display()
        )));
    }
    Ok(())
}

pub(super) fn read_authoritative_state(
    layout: &StateLayout,
) -> Result<Option<AuthoritativeState>, CommandFailure> {
    let present = [
        layout.owner.is_file(),
        layout.plan.is_file(),
        layout.inventory.is_file(),
    ];
    if present.iter().all(|value| !value) {
        return Ok(None);
    }
    if !present.iter().all(|value| *value) {
        return Err(CommandFailure::diagnostic(format!(
            "RUNTIME_PARTIAL_STATE: owner.json, plan.json, and inventory.json must all exist under {}",
            layout.environment_dir.display()
        )));
    }
    Ok(Some(AuthoritativeState {
        owner: read_json(&layout.owner).map_err(runtime_error)?,
        plan: read_json(&layout.plan).map_err(runtime_error)?,
        inventory: read_json(&layout.inventory).map_err(runtime_error)?,
    }))
}

pub(super) fn initialize_authoritative_state(
    layout: &StateLayout,
    plan: &ResourcePlan,
) -> Result<EnvironmentOwner, CommandFailure> {
    let owner = EnvironmentOwner {
        schema_version: 1,
        identity: plan.identity.clone(),
        host: std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown-host".to_string()),
        created_at_unix_ms: unix_time_ms()?,
        manifest_digest: plan.digest.clone(),
        lifecycle: EnvironmentLifecycle::Planned,
    };
    let inventory = ResourceInventory {
        schema_version: 1,
        environment_id: plan.identity.environment_id.clone(),
        ..ResourceInventory::default()
    };
    write_json_atomic(&layout.plan, plan).map_err(runtime_error)?;
    write_json_atomic(&layout.inventory, &inventory).map_err(runtime_error)?;
    write_json_atomic(&layout.owner, &owner).map_err(runtime_error)?;
    Ok(owner)
}

pub(super) fn write_lifecycle(
    layout: &StateLayout,
    owner: &mut EnvironmentOwner,
    lifecycle: EnvironmentLifecycle,
) -> Result<(), CommandFailure> {
    owner.lifecycle = lifecycle;
    write_json_atomic(&layout.owner, owner).map_err(runtime_error)
}

fn unix_time_ms() -> Result<u64, CommandFailure> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    u64::try_from(elapsed.as_millis())
        .map_err(|_| CommandFailure::diagnostic("runtime timestamp exceeds u64"))
}

fn runtime_error(error: autospec_core::runtime_env::RuntimeEnvError) -> CommandFailure {
    CommandFailure::diagnostic(error.to_string())
}

pub(super) struct EnvironmentLease {
    _file: File,
}

const MAX_BIND_ATTEMPTS: usize = 5;

pub(super) struct PortReservations {
    root: PathBuf,
    environment_id: String,
    frontend: PortReservation,
    backend: PortReservation,
}

impl PortReservations {
    pub(super) fn ports(&self) -> (u16, u16) {
        (self.frontend.port(), self.backend.port())
    }

    pub(super) fn release_for_launch(&mut self) -> (u16, u16) {
        (
            self.frontend.release_for_launch(),
            self.backend.release_for_launch(),
        )
    }

    pub(super) fn release_claims(self) -> Result<(), CommandFailure> {
        release_ports(&self.root, &self.environment_id)
    }
}

pub(super) fn claim_ports(
    context: &RuntimeContext,
    frontend: Option<&str>,
    backend: Option<&str>,
) -> Result<PortReservations, CommandFailure> {
    let root = context
        .environment_dir
        .parent()
        .expect("environment state root");
    let mut lease = PortRegistryLease::acquire(root)?;
    lease.registry.release_environment(&context.environment_id);
    let frontend = lease.reserve(&context.environment_id, frontend)?;
    let backend = match lease.reserve(&context.environment_id, backend) {
        Ok(reservation) => reservation,
        Err(error) => {
            frontend
                .release_claim(&mut lease.registry)
                .map_err(port_failure)?;
            return Err(error);
        }
    };
    lease.persist()?;
    Ok(PortReservations {
        root: root.to_path_buf(),
        environment_id: context.environment_id.clone(),
        frontend,
        backend,
    })
}

pub(super) fn release_ports(root: &Path, environment_id: &str) -> Result<(), CommandFailure> {
    let mut lease = PortRegistryLease::acquire(root)?;
    lease.registry.release_environment(environment_id);
    lease.persist()
}

struct PortRegistryLease {
    _file: File,
    registry_path: PathBuf,
    registry: PortRegistry,
}

impl PortRegistryLease {
    fn acquire(root: &Path) -> Result<Self, CommandFailure> {
        let directory = root.join("ports");
        ensure_private_directory(root)?;
        ensure_private_directory(&directory)?;
        let file = open_locked(&directory.join("lease.lock"))?;
        let registry_path = directory.join("registry.json");
        let registry = if registry_path.is_file() {
            read_json(&registry_path).map_err(runtime_error)?
        } else {
            PortRegistry::default()
        };
        Ok(Self {
            _file: file,
            registry_path,
            registry,
        })
    }

    fn reserve(
        &mut self,
        environment_id: &str,
        value: Option<&str>,
    ) -> Result<PortReservation, CommandFailure> {
        let requested = value.map(parse_port).transpose()?;
        reserve_loopback_port(
            &mut self.registry,
            environment_id,
            requested,
            MAX_BIND_ATTEMPTS,
        )
        .map_err(port_failure)
    }

    fn persist(&self) -> Result<(), CommandFailure> {
        write_json_atomic(&self.registry_path, &self.registry).map_err(runtime_error)
    }
}

fn open_locked(path: &Path) -> Result<File, CommandFailure> {
    let file = open_private_lock(path)?;
    file.lock().map_err(io_error)?;
    Ok(file)
}

fn parse_port(value: &str) -> Result<u16, CommandFailure> {
    value
        .parse::<u16>()
        .map_err(|_| CommandFailure::diagnostic("PORT_INVALID"))
}

fn port_failure(error: autospec_core::runtime_env::PortClaimError) -> CommandFailure {
    CommandFailure::diagnostic(format!("{}: port {}", error.code, error.port))
}

fn io_error(error: std::io::Error) -> CommandFailure {
    CommandFailure::diagnostic(error.to_string())
}

impl EnvironmentLease {
    pub(super) fn acquire(environment_dir: &Path) -> Result<Self, CommandFailure> {
        StateLayout::new(
            environment_dir
                .parent()
                .expect("runtime environment directory has a state root"),
            environment_dir
                .file_name()
                .and_then(|value| value.to_str())
                .expect("runtime environment ID is UTF-8"),
        )
        .prepare()?;
        let path = environment_dir.join("lease.lock");
        let file = open_private_lock(&path)?;
        file.lock().map_err(|error| {
            CommandFailure::diagnostic(format!(
                "could not lock runtime environment lease {}: {error}",
                path.display()
            ))
        })?;
        Ok(Self { _file: file })
    }

    pub(super) fn try_acquire(environment_dir: &Path) -> Result<Option<Self>, CommandFailure> {
        StateLayout::new(
            environment_dir
                .parent()
                .expect("runtime environment directory has a state root"),
            environment_dir
                .file_name()
                .and_then(|value| value.to_str())
                .expect("runtime environment ID is UTF-8"),
        )
        .prepare()?;
        let path = environment_dir.join("lease.lock");
        let file = open_private_lock(&path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(CommandFailure::diagnostic(format!(
                "could not try runtime environment lease {}: {error}",
                path.display()
            ))),
        }
    }
}

pub(super) fn write_runtime_state(
    context: &RuntimeContext,
    state: &RuntimeState,
) -> Result<(), CommandFailure> {
    write_private_atomic(&context.env_file, state.render_env_file().as_bytes())
}

pub(super) fn read_runtime_state(context: &RuntimeContext) -> Result<RuntimeState, CommandFailure> {
    let source = fs::read_to_string(&context.env_file).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not read runtime environment {}: {error}",
            context.env_file.display()
        ))
    })?;
    RuntimeState::from_env_file(&source)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))
}

fn ensure_private_directory(path: &Path) -> Result<(), CommandFailure> {
    reject_symlink(path)?;
    fs::create_dir_all(path).map_err(io_error)?;
    secure_directory_descriptor(path)
}

fn write_private_atomic(path: &Path, encoded: &[u8]) -> Result<(), CommandFailure> {
    autospec_core::runtime_env::write_file_atomic(path, encoded).map_err(runtime_error)
}

fn open_private_lock(path: &Path) -> Result<File, CommandFailure> {
    #[cfg(unix)]
    {
        let parent = path.parent().ok_or_else(|| {
            CommandFailure::diagnostic(format!(
                "runtime lock path has no parent: {}",
                path.display()
            ))
        })?;
        let name = path.file_name().ok_or_else(|| {
            CommandFailure::diagnostic(format!(
                "runtime lock path has no filename: {}",
                path.display()
            ))
        })?;
        let parent_directory = open_private_directory_handle(parent)?;
        open_private_lock_at(&parent_directory, Path::new(name))
    }

    #[cfg(not(unix))]
    {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        reject_symlink(path)?;
        let file = options
            .open(path)
            .map_err(|error| private_open_error(path, error))?;
        set_descriptor_mode(&file, PRIVATE_FILE_MODE)?;
        Ok(file)
    }
}

#[cfg(unix)]
fn secure_directory_descriptor(path: &Path) -> Result<(), CommandFailure> {
    let directory = open_private_directory_handle(path)?;
    set_descriptor_mode(&directory, PRIVATE_DIRECTORY_MODE)
}

#[cfg(unix)]
fn open_private_directory_handle(path: &Path) -> Result<File, CommandFailure> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_DIRECTORY | nix::libc::O_NOFOLLOW);
    let directory = options
        .open(path)
        .map_err(|error| private_open_error(path, error))?;
    if !directory.metadata().map_err(io_error)?.is_dir() {
        return Err(CommandFailure::diagnostic(format!(
            "RUNTIME_STATE_NOT_DIRECTORY: {}",
            path.display()
        )));
    }
    Ok(directory)
}

#[cfg(not(unix))]
fn secure_directory_descriptor(path: &Path) -> Result<(), CommandFailure> {
    reject_symlink(path)?;
    Ok(())
}

fn private_open_error(path: &Path, error: std::io::Error) -> CommandFailure {
    #[cfg(unix)]
    if error.raw_os_error() == Some(nix::libc::ELOOP)
        || (error.raw_os_error() == Some(nix::libc::ENOTDIR)
            && fs::symlink_metadata(path)
                .map(|metadata| metadata.file_type().is_symlink())
                .unwrap_or(false))
    {
        return CommandFailure::diagnostic(format!(
            "RUNTIME_STATE_SYMLINK_REJECTED: {}",
            path.display()
        ));
    }
    CommandFailure::diagnostic(format!(
        "could not securely open runtime state {}: {error}",
        path.display()
    ))
}

#[cfg(unix)]
fn open_private_lock_at(parent: &File, name: &Path) -> Result<File, CommandFailure> {
    validate_relative_name(name)?;
    let flags = OFlag::O_RDWR | OFlag::O_CREAT | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW;
    let descriptor = openat(
        parent,
        name,
        flags,
        Mode::from_bits_truncate(PRIVATE_FILE_MODE as nix::libc::mode_t),
    )
    .map_err(|error| private_open_error(name, std::io::Error::from_raw_os_error(error as i32)))?;
    let file = File::from(descriptor);
    set_descriptor_mode(&file, PRIVATE_FILE_MODE)?;
    Ok(file)
}

#[cfg(unix)]
fn validate_relative_name(path: &Path) -> Result<(), CommandFailure> {
    if path.components().count() != 1 {
        return Err(CommandFailure::diagnostic(format!(
            "runtime state descriptor path is not a filename: {}",
            path.display()
        )));
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), CommandFailure> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(CommandFailure::diagnostic(
            format!("RUNTIME_STATE_SYMLINK_REJECTED: {}", path.display()),
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(error)),
    }
}

#[cfg(unix)]
fn set_descriptor_mode(file: &File, mode: u32) -> Result<(), CommandFailure> {
    let permissions = fs::Permissions::from_mode(mode);
    file.set_permissions(permissions).map_err(io_error)
}

#[cfg(not(unix))]
fn set_descriptor_mode(_file: &File, _mode: u32) -> Result<(), CommandFailure> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn test_directory(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock is after the Unix epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "autospec-runtime-state-{name}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).expect("create state test directory");
        directory
    }

    #[cfg(unix)]
    #[test]
    fn atomic_state_write_does_not_follow_the_legacy_predictable_symlink() {
        let directory = test_directory("atomic-symlink");
        let destination = directory.join("env");
        let target = directory.join("external-target");
        let predictable = destination.with_extension(format!("tmp-{}", std::process::id()));
        fs::write(&target, "sentinel").expect("write attack target");
        symlink(&target, &predictable).expect("create predictable temporary symlink");

        write_private_atomic(&destination, b"replacement").expect("write state atomically");

        assert_eq!(fs::read_to_string(&target).unwrap(), "sentinel");
        assert_eq!(fs::read_to_string(&destination).unwrap(), "replacement");
        assert!(fs::symlink_metadata(&predictable)
            .unwrap()
            .file_type()
            .is_symlink());
        fs::remove_dir_all(directory).expect("remove state test directory");
    }

    #[cfg(unix)]
    #[test]
    fn lock_open_rejects_a_symlink_without_changing_its_target() {
        let directory = test_directory("lock-symlink");
        let target = directory.join("external-target");
        let lock = directory.join("lease.lock");
        fs::write(&target, "sentinel").expect("write attack target");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640))
            .expect("set target permissions");
        symlink(&target, &lock).expect("create lock symlink");

        let error = open_locked(&lock).expect_err("symlinked lock must be rejected");

        assert!(error.message.contains("RUNTIME_STATE_SYMLINK_REJECTED"));
        assert_eq!(fs::read_to_string(&target).unwrap(), "sentinel");
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        fs::remove_dir_all(directory).expect("remove state test directory");
    }

    #[cfg(unix)]
    #[test]
    fn lock_open_stays_in_the_open_parent_after_path_substitution() {
        let directory = test_directory("lock-openat");
        let parent = directory.join("state");
        let retained = directory.join("retained-state");
        let external = directory.join("external-state");
        fs::create_dir(&parent).unwrap();
        fs::create_dir(&external).unwrap();
        let parent_directory = open_private_directory_handle(&parent).unwrap();
        fs::rename(&parent, &retained).unwrap();
        symlink(&external, &parent).unwrap();

        let lock = open_private_lock_at(&parent_directory, Path::new("lease.lock")).unwrap();
        lock.unlock().unwrap();

        assert!(retained.join("lease.lock").is_file());
        assert!(!external.join("lease.lock").exists());
        fs::remove_dir_all(directory).expect("remove state test directory");
    }
}
