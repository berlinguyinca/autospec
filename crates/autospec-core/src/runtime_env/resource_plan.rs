use std::path::{Path, PathBuf};

use super::manifest::{RuntimeEnvError, RuntimeManifest, RuntimeMode};
use super::resources::{ComposeIsolation, ComposePlan, MavenPlan, ResourcePlan, RuntimeResources};
use super::shell_command::compose_authority;
use super::EnvironmentIdentity;

pub(super) fn for_repo(
    repo: &Path,
    identity: &EnvironmentIdentity,
) -> Result<ResourcePlan, RuntimeEnvError> {
    for_repo_with_overrides(repo, identity, None, None, false).map(|(plan, _)| plan)
}

pub(super) fn for_repo_allow_empty(
    repo: &Path,
    identity: &EnvironmentIdentity,
) -> Result<ResourcePlan, RuntimeEnvError> {
    build_for_repo_with_overrides(repo, identity, None, None, false, false).map(|(plan, _)| plan)
}

pub(super) fn for_repo_with_overrides(
    repo: &Path,
    identity: &EnvironmentIdentity,
    maven_override: Option<&str>,
    compose_override: Option<&str>,
    whole_environment_disabled: bool,
) -> Result<(ResourcePlan, bool), RuntimeEnvError> {
    build_for_repo_with_overrides(
        repo,
        identity,
        maven_override,
        compose_override,
        whole_environment_disabled,
        true,
    )
}

fn build_for_repo_with_overrides(
    repo: &Path,
    identity: &EnvironmentIdentity,
    maven_override: Option<&str>,
    compose_override: Option<&str>,
    whole_environment_disabled: bool,
    require_runtime_action: bool,
) -> Result<(ResourcePlan, bool), RuntimeEnvError> {
    let manifest = read_optional_manifest(repo)?;
    let selected_mode = if manifest.modes.is_empty() {
        None
    } else {
        Some(manifest.selected_mode(&identity.mode)?)
    };
    let (maven, compose) = detected_plans(repo, identity, &manifest)?;
    let mut plan = ResourcePlan::new(identity.clone(), maven, compose)?;
    let bypassed = plan.apply_invocation_overrides(
        maven_override,
        compose_override,
        whole_environment_disabled,
    )?;
    reject_dual_compose_authority(&manifest, selected_mode, plan.compose.as_ref())?;
    if require_runtime_action {
        require_managed_resource_or_command(selected_mode, &plan)?;
    }
    Ok((plan, bypassed))
}

fn read_optional_manifest(repo: &Path) -> Result<RuntimeManifest, RuntimeEnvError> {
    let has_manifest =
        repo.join(".autospec/runtime.yml").is_file() || repo.join(".agent-runtime.yml").is_file();
    if has_manifest {
        RuntimeManifest::read_from_repo(repo)
    } else {
        Ok(RuntimeManifest {
            path: PathBuf::new(),
            name: None,
            version: 1,
            default_mode: None,
            modes: Vec::new(),
            resources: RuntimeResources::default(),
        })
    }
}

fn detected_plans(
    repo: &Path,
    identity: &EnvironmentIdentity,
    manifest: &RuntimeManifest,
) -> Result<(Option<MavenPlan>, Option<ComposePlan>), RuntimeEnvError> {
    let maven = (repo.join("pom.xml").is_file() || repo.join(".mvn").is_dir()).then(|| MavenPlan {
        isolation: manifest.resources.maven.isolation.clone(),
        local_prefix: format!("autospec/{}", identity.environment_id),
    });
    let files = if manifest.resources.compose.files.is_empty() {
        detect_compose_file(repo)
            .map(|path| canonical_compose_file(repo, &path))
            .transpose()?
            .into_iter()
            .collect()
    } else {
        explicit_compose_files(repo, &manifest.resources.compose.files)?
    };
    let compose = (!files.is_empty()).then(|| ComposePlan {
        isolation: manifest.resources.compose.isolation.clone(),
        files,
        project_name: compose_project_name(&identity.environment_id),
        exports: manifest.resources.compose.exports.clone(),
        preserve_volumes: manifest.resources.compose.preserve_volumes.clone(),
        shared_networks: manifest.resources.compose.shared_networks.clone(),
        shared_volumes: manifest.resources.compose.shared_volumes.clone(),
    });
    Ok((maven, compose))
}

fn reject_dual_compose_authority(
    manifest: &RuntimeManifest,
    mode: Option<&RuntimeMode>,
    compose: Option<&ComposePlan>,
) -> Result<(), RuntimeEnvError> {
    let dual_authority = manifest.version == 1
        && compose.is_some_and(|plan| plan.isolation == ComposeIsolation::Managed)
        && mode
            .and_then(RuntimeMode::command)
            .map(compose_authority)
            .transpose()?
            .unwrap_or(false);
    if dual_authority {
        return Err(RuntimeEnvError::new(
            "RUNTIME_DUAL_COMPOSE_AUTHORITY: runtime mode command and broker both manage Compose",
        ));
    }
    Ok(())
}

fn require_managed_resource_or_command(
    mode: Option<&RuntimeMode>,
    plan: &ResourcePlan,
) -> Result<(), RuntimeEnvError> {
    let command_missing = mode
        .and_then(RuntimeMode::command)
        .is_none_or(|command| command.trim().is_empty());
    let managed_maven = plan
        .maven
        .as_ref()
        .is_some_and(|resource| resource.isolation != super::MavenIsolation::Off);
    let managed_compose = plan
        .compose
        .as_ref()
        .is_some_and(|resource| resource.isolation != ComposeIsolation::Off);
    if !managed_maven && !managed_compose && command_missing {
        return Err(RuntimeEnvError::new(
            "runtime resource plan is empty and selected mode has no command",
        ));
    }
    Ok(())
}

fn explicit_compose_files(repo: &Path, files: &[PathBuf]) -> Result<Vec<PathBuf>, RuntimeEnvError> {
    let mut canonical = Vec::new();
    for file in files {
        let resolved = canonical_compose_file(repo, &repo.join(file))?;
        if canonical.contains(&resolved) {
            return Err(RuntimeEnvError::new("duplicate Compose file"));
        }
        canonical.push(resolved);
    }
    Ok(canonical)
}

fn canonical_compose_file(repo: &Path, file: &Path) -> Result<PathBuf, RuntimeEnvError> {
    let canonical_repo = std::fs::canonicalize(repo).map_err(|error| {
        RuntimeEnvError::new(format!(
            "could not canonicalize repository {}: {error}",
            repo.display()
        ))
    })?;
    let canonical = std::fs::canonicalize(file).map_err(|error| {
        RuntimeEnvError::new(format!(
            "Compose file does not exist: {} ({error})",
            file.display()
        ))
    })?;
    if !canonical.starts_with(&canonical_repo) {
        return Err(RuntimeEnvError::new(format!(
            "Compose file is outside the repository: {}",
            file.display()
        )));
    }
    if !canonical.is_file() {
        return Err(RuntimeEnvError::new(format!(
            "Compose path is not a regular file: {}",
            file.display()
        )));
    }
    Ok(canonical)
}

fn detect_compose_file(repo: &Path) -> Option<PathBuf> {
    [
        "compose.yaml",
        "compose.yml",
        "docker-compose.yaml",
        "docker-compose.yml",
    ]
    .into_iter()
    .map(|name| repo.join(name))
    .find(|path| path.is_file())
}

fn compose_project_name(environment_id: &str) -> String {
    let slug = environment_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("agent_{slug}")
}
