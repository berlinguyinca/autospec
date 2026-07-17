use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

mod compose;
mod compose_normalize;
mod diagnostic;
mod identity;
mod manifest;
mod manifest_v2;
mod maven;
mod resource_plan;
mod resources;
mod session;
mod shell_command;

pub use compose::{ComposeOverride, ComposeOwnership, ComposePolicy};
pub use compose_normalize::{
    ComposeNormalizer, NormalizationEdit, NormalizationPlan, ResourceKind, RuntimeResourcesReport,
};
pub use diagnostic::IsolationDiagnostic;
pub use identity::{load_generation_token, EnvironmentIdentity};
pub use manifest::{RuntimeEnvError, RuntimeManifest, RuntimeMode};
pub use maven::{MavenArgPlatform, MavenArgs, MavenPurgeTarget};
pub use resources::{
    read_json, write_file_atomic, write_json_atomic, ComposeExport, ComposeIsolation, ComposePlan,
    ComposeResourceConfig, EnvironmentLifecycle, EnvironmentOwner, ExportProtocol, ExportValue,
    MavenIsolation, MavenPlan, MavenResourceConfig, OwnedVolume, ResolvedExport, ResourceInventory,
    ResourcePlan, RuntimeResources, SessionRecord,
};
pub use session::{random_session_token, ProcessIdentity, ReleaseDecision, SessionSet};

const BROKER_OWNED_ENVIRONMENT_KEYS: [&str; 9] = [
    "AGENT_ENV_ID",
    "AGENT_ENV_MODE",
    "AGENT_ENV_REPO",
    "AGENT_ENV_MANIFEST",
    "AGENT_FRONTEND_PORT",
    "AGENT_BACKEND_PORT",
    "AGENT_PUBLIC_URL",
    "AUTOSPEC_PUBLIC_URL",
    "COMPOSE_PROJECT_NAME",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeContext {
    pub repo: PathBuf,
    pub manifest: RuntimeManifest,
    pub mode: RuntimeMode,
    pub environment_id: String,
    pub environment_dir: PathBuf,
    pub env_file: PathBuf,
}

impl RuntimeContext {
    pub fn new(
        mut manifest: RuntimeManifest,
        repo: &Path,
        requested_mode: &str,
        state_root: &Path,
    ) -> Result<Self, RuntimeEnvError> {
        let manifest_relative_path = manifest.path().strip_prefix(repo).map_err(|_| {
            RuntimeEnvError::new(format!(
                "runtime manifest {} is not inside repo {}",
                manifest.path().display(),
                repo.display()
            ))
        })?;
        let repo = std::fs::canonicalize(repo).map_err(|error| {
            RuntimeEnvError::new(format!("repo does not exist: {} ({error})", repo.display()))
        })?;
        manifest.path = repo.join(manifest_relative_path);
        let mode = manifest.selected_mode(requested_mode)?.clone();
        let slug = slugify(&manifest.name_or_repo_basename(&repo));
        let name = if slug.is_empty() { "agent_env" } else { &slug };
        let checksum = cksum(&format!("{}:{}", repo.display(), mode.name()))?;
        let environment_id = format!("{name}-{checksum}");
        let environment_dir = state_root.join(&environment_id);
        let env_file = environment_dir.join("env");

        Ok(Self {
            repo,
            manifest,
            mode,
            environment_id,
            environment_dir,
            env_file,
        })
    }

    pub fn new_with_identity(
        mut manifest: RuntimeManifest,
        repo: &Path,
        requested_mode: &str,
        state_root: &Path,
        identity: &EnvironmentIdentity,
    ) -> Result<Self, RuntimeEnvError> {
        let manifest_relative_path = manifest.path().strip_prefix(repo).map_err(|_| {
            RuntimeEnvError::new(format!(
                "runtime manifest {} is not inside repo {}",
                manifest.path().display(),
                repo.display()
            ))
        })?;
        let repo = std::fs::canonicalize(repo).map_err(|error| {
            RuntimeEnvError::new(format!("repo does not exist: {} ({error})", repo.display()))
        })?;
        manifest.path = repo.join(manifest_relative_path);
        let mode = manifest.selected_mode(requested_mode)?.clone();
        if identity.canonical_repo != repo || identity.mode != mode.name() {
            return Err(RuntimeEnvError::new(
                "runtime environment identity does not match the selected repository and mode",
            ));
        }
        let environment_id = identity.environment_id.clone();
        let environment_dir = state_root.join(&environment_id);
        let env_file = environment_dir.join("env");
        Ok(Self {
            repo,
            manifest,
            mode,
            environment_id,
            environment_dir,
            env_file,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeState {
    values: Vec<(String, String)>,
}

impl RuntimeState {
    pub fn from_context(context: &RuntimeContext, frontend_port: u16, backend_port: u16) -> Self {
        let public_url = format!("http://127.0.0.1:{frontend_port}");
        let compose_project_name = format!("agent_{}", compose_slugify(&context.environment_id));
        let mut values = vec![
            ("AGENT_ENV_ID".to_string(), context.environment_id.clone()),
            (
                "AGENT_ENV_MODE".to_string(),
                context.mode.name().to_string(),
            ),
            (
                "AGENT_ENV_REPO".to_string(),
                context.repo.display().to_string(),
            ),
            (
                "AGENT_ENV_MANIFEST".to_string(),
                context.manifest.path().display().to_string(),
            ),
            ("AGENT_FRONTEND_PORT".to_string(), frontend_port.to_string()),
            ("AGENT_BACKEND_PORT".to_string(), backend_port.to_string()),
            ("AGENT_PUBLIC_URL".to_string(), public_url.clone()),
            ("AUTOSPEC_PUBLIC_URL".to_string(), public_url),
            ("COMPOSE_PROJECT_NAME".to_string(), compose_project_name),
        ];
        values.extend(context.mode.env().iter().cloned());
        Self { values }
    }

    pub fn render_env_file(&self) -> String {
        self.values
            .iter()
            .map(|(key, value)| format!("export {key}={}\n", shell_quote(value)))
            .collect()
    }

    pub fn from_env_file(source: &str) -> Result<Self, RuntimeEnvError> {
        let mut values = Vec::new();
        let mut seen = BTreeSet::new();
        for (index, line) in source.lines().enumerate() {
            if line.is_empty() {
                continue;
            }
            let Some(value) = line.strip_prefix("export ") else {
                return Err(RuntimeEnvError::new(format!(
                    "invalid environment file line {}",
                    index + 1
                )));
            };
            let Some((key, quoted_value)) = value.split_once('=') else {
                return Err(RuntimeEnvError::new(format!(
                    "invalid environment file line {}",
                    index + 1
                )));
            };
            if !is_valid_environment_name(key) || !seen.insert(key.to_string()) {
                return Err(RuntimeEnvError::new(format!(
                    "invalid environment file line {}",
                    index + 1
                )));
            }
            values.push((key.to_string(), parse_shell_quote(quoted_value)?));
        }
        for required in BROKER_OWNED_ENVIRONMENT_KEYS {
            if !seen.contains(required) {
                return Err(RuntimeEnvError::new(format!(
                    "missing required environment value: {required}"
                )));
            }
        }
        Ok(Self { values })
    }

    pub fn value(&self, key: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value.as_str())
    }

    pub fn values(&self) -> impl Iterator<Item = (&str, &str)> {
        self.values
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }

    pub fn validate_child_environment(
        &self,
        context: &RuntimeContext,
    ) -> Result<(), RuntimeEnvError> {
        let mut allowed = BROKER_OWNED_ENVIRONMENT_KEYS
            .into_iter()
            .chain(["MAVEN_ARGS"])
            .map(str::to_string)
            .collect::<BTreeSet<_>>();
        allowed.extend(context.mode.env().iter().map(|(key, _)| key.clone()));
        allowed.extend(
            context
                .manifest
                .resources()
                .compose
                .exports
                .iter()
                .map(|export| export.env.clone()),
        );
        if let Some((key, _)) = self.values.iter().find(|(key, _)| !allowed.contains(key)) {
            return Err(RuntimeEnvError::new(format!(
                "RUNTIME_CHILD_ENV_UNDECLARED: {key}"
            )));
        }
        Ok(())
    }

    pub fn replace_existing_value(
        &mut self,
        key: &str,
        value: impl Into<String>,
    ) -> Result<(), RuntimeEnvError> {
        let Some((_, existing_value)) = self
            .values
            .iter_mut()
            .find(|(candidate, _)| candidate == key)
        else {
            return Err(RuntimeEnvError::new(format!(
                "missing runtime environment value: {key}"
            )));
        };
        *existing_value = value.into();
        Ok(())
    }

    pub fn set_value(
        &mut self,
        key: &str,
        value: impl Into<String>,
    ) -> Result<(), RuntimeEnvError> {
        if !is_valid_environment_name(key) {
            return Err(RuntimeEnvError::new(format!(
                "invalid runtime environment name: {key}"
            )));
        }
        let value = value.into();
        if let Some((_, existing)) = self
            .values
            .iter_mut()
            .find(|(candidate, _)| candidate == key)
        {
            *existing = value;
        } else {
            self.values.push((key.to_string(), value));
        }
        Ok(())
    }
}

fn is_valid_environment_name(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some(character) if character.is_ascii_uppercase() || character == '_')
        && characters.all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut previous_was_underscore = false;
    for character in value.chars() {
        let next = if character.is_ascii_uppercase() {
            character.to_ascii_lowercase()
        } else if character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '-'
            || character == '_'
        {
            character
        } else {
            '_'
        };
        if next == '_' && previous_was_underscore {
            continue;
        }
        slug.push(next);
        previous_was_underscore = next == '_';
    }
    slug.trim_matches('_').to_string()
}

fn compose_slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut previous_was_underscore = false;
    for character in value.chars() {
        let next = if character.is_ascii_uppercase() {
            character.to_ascii_lowercase()
        } else if character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_' {
            character
        } else {
            '_'
        };
        if next == '_' && previous_was_underscore {
            continue;
        }
        slug.push(next);
        previous_was_underscore = next == '_';
    }
    slug.trim_matches('_').to_string()
}

fn cksum(value: &str) -> Result<u32, RuntimeEnvError> {
    let mut child = Command::new("cksum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| RuntimeEnvError::new(format!("could not run cksum: {error}")))?;
    child
        .stdin
        .take()
        .ok_or_else(|| RuntimeEnvError::new("could not write cksum input"))?
        .write_all(value.as_bytes())
        .map_err(|error| RuntimeEnvError::new(format!("could not write cksum input: {error}")))?;
    let output = child
        .wait_with_output()
        .map_err(|error| RuntimeEnvError::new(format!("could not read cksum output: {error}")))?;
    if !output.status.success() {
        return Err(RuntimeEnvError::new("cksum failed"));
    }
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .ok_or_else(|| RuntimeEnvError::new("cksum produced no checksum"))?
        .parse::<u32>()
        .map_err(|error| RuntimeEnvError::new(format!("invalid cksum output: {error}")))
}

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn parse_shell_quote(value: &str) -> Result<String, RuntimeEnvError> {
    let characters = value.chars().collect::<Vec<_>>();
    if characters.first() != Some(&'\'') {
        return Err(RuntimeEnvError::new("invalid environment file value"));
    }
    let mut result = String::new();
    let mut index = 1;
    let mut in_quote = true;
    while index < characters.len() {
        if in_quote {
            if characters[index] == '\'' {
                in_quote = false;
            } else {
                result.push(characters[index]);
            }
            index += 1;
            continue;
        }
        if index == characters.len() {
            break;
        }
        if index + 2 < characters.len()
            && characters[index] == '\\'
            && characters[index + 1] == '\''
            && characters[index + 2] == '\''
        {
            result.push('\'');
            index += 3;
            in_quote = true;
            continue;
        }
        return Err(RuntimeEnvError::new("invalid environment file value"));
    }
    if in_quote {
        return Err(RuntimeEnvError::new("invalid environment file value"));
    }
    Ok(result)
}
