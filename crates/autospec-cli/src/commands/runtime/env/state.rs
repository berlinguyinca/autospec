use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::runtime_env::{
    EnvironmentLifecycle, EnvironmentOwner, ResourceInventory, ResourcePlan, RuntimeContext,
    RuntimeState,
};

use crate::commands::CommandFailure;

pub(super) use autospec_core::runtime_env::{read_json, write_json_atomic};

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

impl EnvironmentLease {
    pub(super) fn acquire(environment_dir: &Path) -> Result<Self, CommandFailure> {
        fs::create_dir_all(environment_dir).map_err(|error| {
            CommandFailure::diagnostic(format!(
                "could not create runtime environment {}: {error}",
                environment_dir.display()
            ))
        })?;
        let path = environment_dir.join("lease.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| {
                CommandFailure::diagnostic(format!(
                    "could not open runtime environment lease {}: {error}",
                    path.display()
                ))
            })?;
        file.lock().map_err(|error| {
            CommandFailure::diagnostic(format!(
                "could not lock runtime environment lease {}: {error}",
                path.display()
            ))
        })?;
        Ok(Self { _file: file })
    }
}

pub(super) fn write_runtime_state(
    context: &RuntimeContext,
    state: &RuntimeState,
) -> Result<(), CommandFailure> {
    let temporary = context
        .env_file
        .with_extension(format!("tmp-{}", std::process::id()));
    let mut file = File::create(&temporary).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not write runtime environment {}: {error}",
            temporary.display()
        ))
    })?;
    file.write_all(state.render_env_file().as_bytes())
        .map_err(|error| {
            CommandFailure::diagnostic(format!(
                "could not write runtime environment {}: {error}",
                temporary.display()
            ))
        })?;
    fs::rename(&temporary, &context.env_file).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not finalize runtime environment {}: {error}",
            context.env_file.display()
        ))
    })
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
