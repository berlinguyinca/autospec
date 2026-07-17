use std::path::Path;
use std::process::Command;

use super::{fingerprint, ComposeNormalizer, NormalizationPlan, PlannedFile};
use crate::runtime_env::{
    write_file_atomic, ComposeIsolation, ComposePlan, ComposePolicy, EnvironmentIdentity,
    RuntimeEnvError, RuntimeManifest,
};

const PROFILE_FLAG: &str = concat!("-", "-profile");
const ALL_RESOURCES_FLAG: &str = concat!("-", "-all-resources");
const PROJECT_NAME_FLAG: &str = concat!("-", "-project-name");
const FORMAT_FLAG: &str = concat!("-", "-format");

pub(super) fn apply(plan: &NormalizationPlan, expected: &str) -> Result<(), RuntimeEnvError> {
    validate_apply(plan, expected)?;
    let changed = plan
        .files
        .iter()
        .filter(|file| file.original != file.rendered)
        .collect::<Vec<_>>();
    for file in &changed {
        if let Err(error) = write_file_atomic(&file.path, &file.rendered) {
            rollback(&changed)?;
            return Err(error);
        }
    }
    if let Err(error) = ComposeNormalizer::verify(plan) {
        rollback(&changed)?;
        return Err(error);
    }
    Ok(())
}

fn validate_apply(plan: &NormalizationPlan, expected: &str) -> Result<(), RuntimeEnvError> {
    if expected != plan.fingerprint {
        return Err(RuntimeEnvError::new(format!(
            "NORMALIZE_STALE_FINGERPRINT: expected {}, received {expected}",
            plan.fingerprint
        )));
    }
    if !plan.remaining_diagnostics.is_empty() {
        return Err(RuntimeEnvError::new(
            "NORMALIZE_UNRESOLVED_DIAGNOSTICS: unsafe Compose input remains unchanged",
        ));
    }
    if fingerprint(&read_current(&plan.files)?)? != plan.fingerprint {
        return Err(RuntimeEnvError::new(
            "NORMALIZE_STALE_SOURCE: source bytes changed after planning",
        ));
    }
    Ok(())
}

pub(super) fn verify(plan: &NormalizationPlan) -> Result<(), RuntimeEnvError> {
    let identity = EnvironmentIdentity::resolve(&plan.repo, "local", None)?;
    let resource_plan = RuntimeManifest::resource_plan_for_repo(&plan.repo, &identity)?;
    let compose = resource_plan.compose.ok_or_else(|| {
        RuntimeEnvError::new("NORMALIZE_VERIFY_NO_COMPOSE: normalized Compose plan is absent")
    })?;
    let model = resolved_model(&plan.repo, &compose)?;
    let diagnostics = ComposePolicy::evaluate_json_in_context(
        &model,
        &compose,
        &plan.environment_id,
        &plan.repo,
    )?;
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(RuntimeEnvError::new(format!(
            "NORMALIZE_POLICY_FAILED: {}",
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code.as_str())
                .collect::<Vec<_>>()
                .join(",")
        )))
    }
}

fn read_current(files: &[PlannedFile]) -> Result<Vec<PlannedFile>, RuntimeEnvError> {
    files
        .iter()
        .map(|file| {
            let original = std::fs::read(&file.path).map_err(|error| {
                RuntimeEnvError::new(format!(
                    "could not re-read {}: {error}",
                    file.path.display()
                ))
            })?;
            Ok(PlannedFile {
                path: file.path.clone(),
                rendered: original.clone(),
                original,
            })
        })
        .collect()
}

fn rollback(files: &[&PlannedFile]) -> Result<(), RuntimeEnvError> {
    let errors = files
        .iter()
        .filter_map(|file| write_file_atomic(&file.path, &file.original).err())
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(RuntimeEnvError::new(format!(
            "NORMALIZE_ROLLBACK_FAILED: {}",
            errors.join("; ")
        )))
    }
}

fn resolved_model(repo: &Path, plan: &ComposePlan) -> Result<Vec<u8>, RuntimeEnvError> {
    if plan.isolation != ComposeIsolation::Managed {
        return Err(RuntimeEnvError::new("NORMALIZE_VERIFY_COMPOSE_DISABLED"));
    }
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
        .current_dir(repo)
        .output()
        .map_err(|error| RuntimeEnvError::new(format!("NORMALIZE_COMPOSE_CONFIG_EXEC: {error}")))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(RuntimeEnvError::new(format!(
            "NORMALIZE_COMPOSE_CONFIG_FAILED: {}",
            String::from_utf8_lossy(&output.stderr).trim_end()
        )))
    }
}
