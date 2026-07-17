use std::path::{Path, PathBuf};

use super::manifest::{RuntimeEnvError, RuntimeManifest, RuntimeMode};
use super::resources::{ComposeIsolation, ComposePlan, MavenPlan, ResourcePlan, RuntimeResources};
use super::EnvironmentIdentity;

pub(super) fn for_repo(
    repo: &Path,
    identity: &EnvironmentIdentity,
) -> Result<ResourcePlan, RuntimeEnvError> {
    let manifest = read_optional_manifest(repo)?;
    let selected_mode = if manifest.modes.is_empty() {
        None
    } else {
        Some(manifest.selected_mode(&identity.mode)?)
    };
    let (maven, compose) = detected_plans(repo, identity, &manifest);
    reject_dual_compose_authority(&manifest, selected_mode, compose.as_ref())?;
    require_command_for_empty_plan(selected_mode, &maven, &compose)?;
    ResourcePlan::new(identity.clone(), maven, compose)
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
) -> (Option<MavenPlan>, Option<ComposePlan>) {
    let maven = (repo.join("pom.xml").is_file() || repo.join(".mvn").exists()).then(|| MavenPlan {
        isolation: manifest.resources.maven.isolation.clone(),
        local_prefix: format!("autospec/{}", identity.environment_id),
    });
    let files: Vec<PathBuf> = if manifest.resources.compose.files.is_empty() {
        detect_compose_file(repo).into_iter().collect()
    } else {
        manifest
            .resources
            .compose
            .files
            .iter()
            .map(|path| repo.join(path))
            .collect()
    };
    let compose = (!files.is_empty()).then(|| ComposePlan {
        isolation: manifest.resources.compose.isolation.clone(),
        files,
        project_name: compose_project_name(&identity.environment_id),
        exports: manifest.resources.compose.exports.clone(),
        preserve_volumes: manifest.resources.compose.preserve_volumes.clone(),
    });
    (maven, compose)
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
            .is_some_and(command_manages_compose);
    if dual_authority {
        return Err(RuntimeEnvError::new(
            "RUNTIME_DUAL_COMPOSE_AUTHORITY: runtime mode command and broker both manage Compose",
        ));
    }
    Ok(())
}

fn require_command_for_empty_plan(
    mode: Option<&RuntimeMode>,
    maven: &Option<MavenPlan>,
    compose: &Option<ComposePlan>,
) -> Result<(), RuntimeEnvError> {
    let command_missing = mode
        .and_then(RuntimeMode::command)
        .is_none_or(|command| command.trim().is_empty());
    if maven.is_none() && compose.is_none() && command_missing {
        return Err(RuntimeEnvError::new(
            "runtime resource plan is empty and selected mode has no command",
        ));
    }
    Ok(())
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

fn command_manages_compose(command: &str) -> bool {
    let tokens = command
        .split_whitespace()
        .map(|token| token.trim_matches(|character: char| "'\";|&()".contains(character)))
        .map(|token| {
            Path::new(token)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(token)
        })
        .collect::<Vec<_>>();
    tokens.contains(&"docker-compose")
        || tokens
            .windows(2)
            .any(|tokens| tokens == ["docker", "compose"])
}
