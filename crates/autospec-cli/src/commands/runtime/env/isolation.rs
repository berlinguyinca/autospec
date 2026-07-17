use std::path::Path;

use autospec_core::runtime_env::{
    load_generation_token, EnvironmentIdentity, ResourcePlan, RuntimeManifest,
};

use crate::commands::CommandFailure;

#[derive(Clone)]
pub(super) struct InvocationIsolation {
    pub(super) bypassed: bool,
    pub(super) whole_environment_disabled: bool,
    pub(super) plan: Option<ResourcePlan>,
}

pub(super) fn invocation_isolation(
    repo: &Path,
    requested_mode: &str,
) -> Result<InvocationIsolation, CommandFailure> {
    let whole_environment_disabled = whole_environment_disabled()?;
    if whole_environment_disabled {
        return Ok(InvocationIsolation {
            bypassed: bypass_without_planning(true)?,
            whole_environment_disabled: true,
            plan: None,
        });
    }
    let maven = environment_value("AUTOSPEC_MAVEN_ISOLATION")?;
    let compose = environment_value("AUTOSPEC_COMPOSE_ISOLATION")?;
    let identity = planning_identity(repo, requested_mode)?;
    let (plan, bypassed) = RuntimeManifest::resource_plan_for_repo_with_overrides(
        repo,
        &identity,
        maven.as_deref(),
        compose.as_deref(),
        whole_environment_disabled,
    )
    .or_else(|error| {
        if error
            .to_string()
            .contains("resource plan is empty and selected mode has no command")
        {
            validate_without_plan(
                &identity,
                maven.as_deref(),
                compose.as_deref(),
                whole_environment_disabled,
            )
        } else {
            Err(error)
        }
    })
    .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    Ok(InvocationIsolation {
        bypassed,
        whole_environment_disabled,
        plan: Some(plan),
    })
}

pub(super) fn bypass_without_planning(
    whole_environment_disabled: bool,
) -> Result<bool, CommandFailure> {
    let maven = environment_value("AUTOSPEC_MAVEN_ISOLATION")?;
    let compose = environment_value("AUTOSPEC_COMPOSE_ISOLATION")?;
    ResourcePlan::validate_invocation_override_values(maven.as_deref(), compose.as_deref())
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    Ok(whole_environment_disabled || maven.is_some() || compose.is_some())
}

pub(super) fn whole_environment_disabled() -> Result<bool, CommandFailure> {
    match environment_value("AUTOSPEC_ENV_DISABLE")?.as_deref() {
        None => Ok(false),
        Some("1") => Ok(true),
        Some(value) => Err(CommandFailure::diagnostic(format!(
            "unsupported AUTOSPEC_ENV_DISABLE value: {value:?}; expected '1'"
        ))),
    }
}

pub(super) fn planning_identity(
    repo: &Path,
    requested_mode: &str,
) -> Result<EnvironmentIdentity, CommandFailure> {
    let manifest = RuntimeManifest::read_from_repo(repo)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    let mode = manifest
        .selected_mode(requested_mode)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    let generation = load_generation_token(repo)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    EnvironmentIdentity::resolve(repo, mode.name(), generation.as_deref())
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))
}

fn validate_without_plan(
    identity: &EnvironmentIdentity,
    maven: Option<&str>,
    compose: Option<&str>,
    whole_environment_disabled: bool,
) -> Result<(ResourcePlan, bool), autospec_core::runtime_env::RuntimeEnvError> {
    let mut plan = ResourcePlan::new(identity.clone(), None, None)?;
    let bypassed = plan.apply_invocation_overrides(maven, compose, whole_environment_disabled)?;
    Ok((plan, bypassed))
}

fn environment_value(key: &str) -> Result<Option<String>, CommandFailure> {
    let Some(value) = std::env::var_os(key) else {
        return Ok(None);
    };
    value
        .into_string()
        .map(Some)
        .map_err(|_| CommandFailure::diagnostic(format!("{key} must contain valid UTF-8 text")))
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static NEXT_REPO: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn planning_identity_uses_selected_mode_and_generation_token() {
        let suffix = NEXT_REPO.fetch_add(1, Ordering::Relaxed);
        let repo = std::env::temp_dir().join(format!(
            "autospec-cli-isolation-{}-{suffix}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(repo.join(".autospec")).unwrap();
        std::fs::write(
            repo.join(".autospec/runtime.yml"),
            "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: true\n",
        )
        .unwrap();
        assert!(Command::new("git")
            .arg("init")
            .arg(&repo)
            .status()
            .unwrap()
            .success());

        let identity = planning_identity(&repo, "auto").expect("identity resolves");

        assert_eq!(identity.mode, "local");
        assert!(identity.generation.is_some());
        assert_eq!(identity.generation, load_generation_token(&repo).unwrap());
        let _ = std::fs::remove_dir_all(repo);
    }
}
