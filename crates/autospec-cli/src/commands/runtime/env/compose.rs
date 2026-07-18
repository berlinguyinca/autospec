use std::path::Path;
use std::process::{Command, Output};

use autospec_core::runtime_env::{
    write_file_atomic, ComposeIsolation, ComposePlan, ComposePolicy, IsolationDiagnostic,
    ResourcePlan, RuntimeContext, RuntimeState,
};

use crate::commands::CommandFailure;

mod lifecycle;
mod ownership;

use super::state::StateLayout;

const PROFILE_FLAG: &str = concat!("-", "-profile");
const ALL_RESOURCES_FLAG: &str = concat!("-", "-all-resources");
const PROJECT_NAME_FLAG: &str = concat!("-", "-project-name");
const FORMAT_FLAG: &str = concat!("-", "-format");

pub(super) struct ComposeAdapter;

impl ComposeAdapter {
    pub(super) fn reject_caller_project_name(
        compose: Option<&ComposePlan>,
    ) -> Result<(), CommandFailure> {
        lifecycle::reject_caller_project_name(compose)
    }

    pub(super) fn up(
        compose: Option<&ComposePlan>,
        plan: &ResourcePlan,
        resolved: Option<&[u8]>,
        context: &RuntimeContext,
        state: &mut RuntimeState,
        layout: &StateLayout,
    ) -> Result<(), CommandFailure> {
        lifecycle::up(compose, plan, resolved, context, state, layout)
    }

    pub(super) fn validate_canonical_export(
        compose: Option<&ComposePlan>,
    ) -> Result<(), CommandFailure> {
        let Some(plan) = compose.filter(|plan| plan.isolation == ComposeIsolation::Managed) else {
            return Ok(());
        };
        plan.canonical_url_export_index()
            .map(|_| ())
            .map_err(|error| CommandFailure::diagnostic(error.to_string()))
    }

    pub(super) fn down_owned(
        compose: Option<&ComposePlan>,
        plan: &ResourcePlan,
        context: &RuntimeContext,
        layout: &StateLayout,
    ) -> Result<(), CommandFailure> {
        lifecycle::down_owned(compose, plan, context, layout)
    }

    pub(super) fn validate_resolved_model(
        plan: Option<&ComposePlan>,
        context: &RuntimeContext,
    ) -> Result<Option<Vec<u8>>, CommandFailure> {
        let Some(plan) = plan.filter(|plan| plan.isolation == ComposeIsolation::Managed) else {
            return Ok(None);
        };
        let resolved = Self::resolved_model(plan, context)?;
        let diagnostics = ComposePolicy::evaluate_json_in_context(
            &resolved,
            plan,
            &context.environment_id,
            &context.repo,
        )
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
        if diagnostics.is_empty() {
            Ok(Some(resolved))
        } else {
            Err(CommandFailure::diagnostic(render_diagnostics(&diagnostics)))
        }
    }

    fn resolved_model(
        plan: &ComposePlan,
        context: &RuntimeContext,
    ) -> Result<Vec<u8>, CommandFailure> {
        let mut command = Command::new("docker");
        command.args(["compose", PROFILE_FLAG, "*", ALL_RESOURCES_FLAG]);
        for file in &plan.files {
            command.arg("-f").arg(file);
        }
        let output = command
            .args([
                PROJECT_NAME_FLAG,
                &plan.project_name,
                "config",
                FORMAT_FLAG,
                "json",
            ])
            .current_dir(&context.repo)
            .output()
            .map_err(|error| {
                CommandFailure::diagnostic(format!(
                    "could not execute docker compose config: {error}"
                ))
            })?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(CommandFailure::status(
                String::from_utf8_lossy(&output.stderr).into_owned(),
                output.status.code().unwrap_or(2),
            ))
        }
    }
}

fn render_diagnostics(diagnostics: &[IsolationDiagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "{}: {}={} (environment {}; recovery: {})",
                diagnostic.code,
                diagnostic.resource,
                diagnostic.evidence,
                diagnostic.environment_id,
                diagnostic.recovery_command
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn compose_command(
    plan: &ComposePlan,
    context: &RuntimeContext,
    override_path: &Path,
    tail: &[&str],
) -> Command {
    let mut command = docker_command(&["compose"]);
    for file in &plan.files {
        command.arg("-f").arg(file);
    }
    command
        .arg("-f")
        .arg(override_path)
        .args([PROJECT_NAME_FLAG, &plan.project_name])
        .args(tail)
        .current_dir(&context.repo);
    command
}

fn docker_command(args: &[&str]) -> Command {
    let mut command = Command::new("docker");
    command.args(args);
    command
}

fn run(mut command: Command) -> Result<Output, CommandFailure> {
    command
        .output()
        .map_err(|error| CommandFailure::diagnostic(format!("COMPOSE_EXEC_FAILED: {error}")))
}

fn require_success(output: Output, code: &str) -> Result<Output, CommandFailure> {
    if output.status.success() {
        Ok(output)
    } else {
        Err(CommandFailure::status(
            format!(
                "{code}: {}",
                String::from_utf8_lossy(&output.stderr).trim_end()
            ),
            output.status.code().unwrap_or(2),
        ))
    }
}

fn output_lines(source: &[u8]) -> Result<Vec<String>, CommandFailure> {
    let value = String::from_utf8(source.to_vec())
        .map_err(|_| CommandFailure::diagnostic("COMPOSE_OUTPUT_INVALID_UTF8"))?;
    Ok(value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

fn write_override_atomic(path: &Path, value: &str) -> Result<(), CommandFailure> {
    write_file_atomic(path, value.as_bytes()).map_err(|error| {
        CommandFailure::diagnostic(format!("could not write Compose override: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use super::write_override_atomic;

    #[test]
    fn override_atomic_writes_use_independent_create_new_temporaries() {
        let root =
            std::env::temp_dir().join(format!("autospec-compose-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("override.yaml");
        let barrier = Arc::new(Barrier::new(8));
        let writers = (0..8)
            .map(|index| {
                let target = target.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    write_override_atomic(&target, &format!("writer: {index}\n"))
                })
            })
            .collect::<Vec<_>>();

        for writer in writers {
            writer.join().unwrap().expect("each atomic write succeeds");
        }
        let value = std::fs::read_to_string(&target).unwrap();
        assert!(value.starts_with("writer: "));
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn override_atomic_write_replaces_symlink_without_touching_target() {
        use std::os::unix::fs::symlink;

        let root =
            std::env::temp_dir().join(format!("autospec-compose-symlink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let outside = root.join("outside");
        let target = root.join("override.yaml");
        std::fs::write(&outside, "untouched\n").unwrap();
        symlink(&outside, &target).unwrap();

        write_override_atomic(&target, "services: {}\n").unwrap();

        assert_eq!(std::fs::read_to_string(outside).unwrap(), "untouched\n");
        assert_eq!(std::fs::read_to_string(target).unwrap(), "services: {}\n");
        std::fs::remove_dir_all(root).unwrap();
    }
}
