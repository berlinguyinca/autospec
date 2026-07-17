use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use autospec_core::runtime_env::{RuntimeContext, RuntimeState};

use crate::commands::CommandFailure;

#[allow(unused_imports)]
pub(super) use autospec_core::runtime_env::{read_json, write_json_atomic};

#[allow(dead_code)]
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
    #[allow(dead_code)]
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
