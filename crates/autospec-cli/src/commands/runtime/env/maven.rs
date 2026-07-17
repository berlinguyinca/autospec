use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use autospec_core::runtime_env::{
    read_json, write_json_atomic, IsolationDiagnostic, MavenIsolation, MavenPlan, MavenPurgeTarget,
    ResourceInventory, RuntimeContext, RuntimeState,
};

use crate::commands::CommandFailure;

use super::state::StateLayout;

pub(super) struct MavenAdapter;

impl MavenAdapter {
    pub(super) fn configure(
        plan: Option<&MavenPlan>,
        context: &RuntimeContext,
        state: &mut RuntimeState,
        layout: &StateLayout,
    ) -> Result<(), CommandFailure> {
        let Some(plan) = enabled_plan(plan) else {
            return Ok(());
        };
        Self::probe(&context.repo)?;
        let arguments = MavenPlan::arguments(&caller_maven_args()?, &context.environment_id)
            .map_err(diagnostic)?;
        let repository = Self::effective_local_repository(&context.repo, &arguments.render())?;
        let target = MavenPurgeTarget::for_environment(&repository, &context.environment_id)
            .map_err(diagnostic)?;
        validate_plan_prefix(plan, &context.environment_id)?;
        reject_symlinked_prefix(&repository, target.target())?;
        record_prefix(
            layout,
            &context.environment_id,
            Some(target.target().to_path_buf()),
        )?;
        state
            .set_value("MAVEN_ARGS", arguments.render())
            .map_err(|error| failure(error.to_string()))
    }

    pub(super) fn probe(repo: &Path) -> Result<String, CommandFailure> {
        let output = Command::new("mvn")
            .arg("--version")
            .current_dir(repo)
            .output()
            .map_err(|error| failure(format!("MAVEN_EXECUTABLE_UNAVAILABLE: {error}")))?;
        if !output.status.success() {
            return Err(failure(format!(
                "MAVEN_VERSION_PROBE_FAILED: Maven exited with {}",
                output.status
            )));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let version = stdout
            .lines()
            .find_map(|line| line.split_once("Apache Maven ").map(|(_, value)| value))
            .and_then(|value| value.split_whitespace().next())
            .ok_or_else(|| failure("MAVEN_VERSION_INVALID: could not parse mvn --version"))?;
        if version.split('.').next() != Some("4") {
            return Err(failure(format!(
                "MAVEN_VERSION_UNSUPPORTED: Maven 4 is required, found {version}"
            )));
        }
        Ok(version.to_string())
    }

    pub(super) fn effective_local_repository(
        repo: &Path,
        arguments: &str,
    ) -> Result<PathBuf, CommandFailure> {
        let output = Command::new("mvn")
            .args([
                "help:evaluate",
                "-Dexpression=settings.localRepository",
                "-q",
                "-DforceStdout",
            ])
            .env("MAVEN_ARGS", arguments)
            .current_dir(repo)
            .output()
            .map_err(|error| failure(format!("MAVEN_REPOSITORY_PROBE_FAILED: {error}")))?;
        if !output.status.success() {
            return Err(failure(format!(
                "MAVEN_REPOSITORY_PROBE_FAILED: Maven exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let value = String::from_utf8(output.stdout)
            .map_err(|_| failure("MAVEN_REPOSITORY_INVALID: Maven output is not UTF-8"))?;
        let path = value
            .lines()
            .map(str::trim)
            .filter_map(|line| {
                line.rsplit_once("[stdout] ")
                    .map(|(_, path)| path)
                    .or_else(|| (!line.is_empty() && !line.starts_with('[')).then_some(line))
            })
            .next_back()
            .map(PathBuf::from)
            .ok_or_else(|| failure("MAVEN_REPOSITORY_INVALID: Maven returned no repository"))?;
        if !path.is_absolute() {
            return Err(failure(format!(
                "MAVEN_REPOSITORY_INVALID: effective repository is not absolute: {}",
                path.display()
            )));
        }
        fs::create_dir_all(&path).map_err(|error| {
            failure(format!(
                "MAVEN_REPOSITORY_INVALID: could not create {}: {error}",
                path.display()
            ))
        })?;
        fs::canonicalize(&path).map_err(|error| {
            failure(format!(
                "MAVEN_REPOSITORY_INVALID: could not resolve {}: {error}",
                path.display()
            ))
        })
    }

    pub(super) fn purge_owned_prefix(
        plan: Option<&MavenPlan>,
        context: &RuntimeContext,
        state: &RuntimeState,
        layout: &StateLayout,
        inventory: &ResourceInventory,
    ) -> Result<(), CommandFailure> {
        let plan = enabled_plan(plan).ok_or_else(|| {
            failure("MAVEN_PURGE_NOT_MANAGED: this environment has no managed Maven prefix")
        })?;
        validate_plan_prefix(plan, &context.environment_id)?;
        Self::probe(&context.repo)?;
        let arguments = MavenPlan::arguments(
            state.value("MAVEN_ARGS").unwrap_or_default(),
            &context.environment_id,
        )
        .map_err(diagnostic)?;
        let repository = Self::effective_local_repository(&context.repo, &arguments.render())?;
        let target = MavenPurgeTarget::for_environment(&repository, &context.environment_id)
            .map_err(diagnostic)?;
        if inventory.environment_id != context.environment_id
            || inventory.maven_local_prefix.as_deref() != Some(target.target())
        {
            return Err(failure(
                "MAVEN_PURGE_IDENTITY_MISMATCH: inventory does not own the effective Maven prefix",
            ));
        }
        reject_symlinked_prefix(&repository, target.target())?;
        record_prefix(
            layout,
            &context.environment_id,
            Some(target.target().to_path_buf()),
        )?;
        match fs::remove_dir_all(target.target()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(failure(format!(
                    "MAVEN_PURGE_FAILED: could not remove {}: {error}",
                    target.target().display()
                )))
            }
        }
        record_prefix(layout, &context.environment_id, None)
    }
}

fn enabled_plan(plan: Option<&MavenPlan>) -> Option<&MavenPlan> {
    plan.filter(|plan| plan.isolation == MavenIsolation::SplitLocal)
}

fn caller_maven_args() -> Result<String, CommandFailure> {
    let Some(value) = std::env::var_os("MAVEN_ARGS") else {
        return Ok(String::new());
    };
    value
        .into_string()
        .map_err(|_| failure("MAVEN_ARGUMENT_PARSE: MAVEN_ARGS must contain valid UTF-8 text"))
}

fn validate_plan_prefix(plan: &MavenPlan, environment_id: &str) -> Result<(), CommandFailure> {
    if plan.local_prefix != format!("autospec/{environment_id}") {
        return Err(failure(
            "MAVEN_PURGE_IDENTITY_MISMATCH: resource plan has an unexpected Maven prefix",
        ));
    }
    Ok(())
}

fn reject_symlinked_prefix(repository: &Path, target: &Path) -> Result<(), CommandFailure> {
    let autospec = repository.join("autospec");
    for path in [&autospec, target] {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(failure(format!(
                    "MAVEN_PURGE_SYMLINK: refusing symlinked Maven prefix {}",
                    path.display()
                )))
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(failure(format!(
                    "MAVEN_PURGE_BOUNDARY_FAILED: could not inspect {}: {error}",
                    path.display()
                )))
            }
        }
    }
    Ok(())
}

fn record_prefix(
    layout: &StateLayout,
    environment_id: &str,
    prefix: Option<PathBuf>,
) -> Result<(), CommandFailure> {
    let mut inventory: ResourceInventory =
        read_json(&layout.inventory).map_err(|error| failure(error.to_string()))?;
    if inventory.environment_id != environment_id {
        return Err(failure(
            "MAVEN_PURGE_IDENTITY_MISMATCH: inventory belongs to another environment",
        ));
    }
    inventory.maven_local_prefix = prefix;
    write_json_atomic(&layout.inventory, &inventory).map_err(|error| failure(error.to_string()))
}

fn diagnostic(diagnostic: IsolationDiagnostic) -> CommandFailure {
    failure(format!("{}: {}", diagnostic.code, diagnostic.evidence))
}

fn failure(message: impl Into<String>) -> CommandFailure {
    CommandFailure::diagnostic(message.into())
}
