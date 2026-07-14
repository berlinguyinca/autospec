use std::collections::BTreeSet;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const REQUIRED_ENVIRONMENT_VALUES: [&str; 9] = [
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
pub struct RuntimeEnvError {
    message: String,
}

impl RuntimeEnvError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for RuntimeEnvError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for RuntimeEnvError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeMode {
    name: String,
    command: Option<String>,
    down: Option<String>,
    env: Vec<(String, String)>,
}

impl RuntimeMode {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn command(&self) -> Option<&str> {
        self.command.as_deref()
    }

    pub fn down(&self) -> Option<&str> {
        self.down.as_deref()
    }

    pub fn env(&self) -> &[(String, String)] {
        &self.env
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeManifest {
    path: PathBuf,
    name: Option<String>,
    default_mode: Option<String>,
    modes: Vec<RuntimeMode>,
}

impl RuntimeManifest {
    pub fn read_from_repo(repo: &Path) -> Result<Self, RuntimeEnvError> {
        let autospec_path = repo.join(".autospec/runtime.yml");
        let agent_path = repo.join(".agent-runtime.yml");
        let path = if autospec_path.is_file() {
            autospec_path
        } else if agent_path.is_file() {
            agent_path
        } else {
            return Err(RuntimeEnvError::new(format!(
                "no runtime manifest found in {} (.autospec/runtime.yml or .agent-runtime.yml)",
                repo.display()
            )));
        };
        let source = std::fs::read_to_string(&path).map_err(|error| {
            RuntimeEnvError::new(format!(
                "could not read runtime manifest {}: {error}",
                path.display()
            ))
        })?;
        let mut manifest = Self::parse(&source)?;
        manifest.path = path;
        Ok(manifest)
    }

    pub fn parse(source: &str) -> Result<Self, RuntimeEnvError> {
        let mut version = None;
        let mut name = None;
        let mut default_mode = None;
        let mut modes = Vec::new();
        let mut in_modes = false;
        let mut current_mode = None;
        let mut in_env = false;

        for (index, raw_line) in source.lines().enumerate() {
            let line_number = index + 1;
            if raw_line.trim().is_empty() || raw_line.trim_start().starts_with('#') {
                continue;
            }
            let indent = raw_line.len() - raw_line.trim_start_matches(' ').len();
            let content = raw_line.trim();

            if indent == 0 {
                current_mode = None;
                in_env = false;
                in_modes = content == "modes:";
                if in_modes {
                    continue;
                }
                let Some((key, value)) = split_mapping(content) else {
                    continue;
                };
                match key {
                    "version" => version = Some(unquote(value)),
                    "name" => name = Some(unquote(value)),
                    "default_mode" => default_mode = Some(unquote(value)),
                    _ => {}
                }
                continue;
            }

            if !in_modes {
                continue;
            }

            if indent == 2 && content.ends_with(':') {
                let mode_name = content.trim_end_matches(':').trim().to_string();
                if mode_name.is_empty() {
                    return Err(RuntimeEnvError::new(format!(
                        "empty runtime mode at line {line_number}"
                    )));
                }
                if modes
                    .iter()
                    .any(|mode: &RuntimeMode| mode.name == mode_name)
                {
                    return Err(RuntimeEnvError::new(format!(
                        "duplicate runtime mode: {mode_name}"
                    )));
                }
                modes.push(RuntimeMode {
                    name: mode_name,
                    command: None,
                    down: None,
                    env: Vec::new(),
                });
                current_mode = Some(modes.len() - 1);
                in_env = false;
                continue;
            }

            let Some(mode_index) = current_mode else {
                continue;
            };
            if indent == 4 {
                if content == "env:" {
                    in_env = true;
                    continue;
                }
                in_env = false;
                let Some((key, value)) = split_mapping(content) else {
                    continue;
                };
                match key {
                    "command" => modes[mode_index].command = Some(value.trim().to_string()),
                    "down" => modes[mode_index].down = Some(value.trim().to_string()),
                    _ => {}
                }
                continue;
            }

            if indent == 6 && in_env {
                let Some((key, value)) = split_mapping(content) else {
                    continue;
                };
                let key = key.trim().to_string();
                if !is_valid_environment_name(&key) {
                    return Err(RuntimeEnvError::new(format!(
                        "invalid environment name: {key}"
                    )));
                }
                if modes[mode_index]
                    .env
                    .iter()
                    .any(|(existing, _)| existing == &key)
                {
                    return Err(RuntimeEnvError::new(format!(
                        "duplicate environment name: {key}"
                    )));
                }
                modes[mode_index].env.push((key, unquote(value)));
            }
        }

        if let Some(version) = version {
            if version != "1" {
                return Err(RuntimeEnvError::new(format!(
                    "unsupported runtime manifest version: {version}"
                )));
            }
        }
        if modes.is_empty() {
            return Err(RuntimeEnvError::new("runtime manifest has no modes"));
        }

        Ok(Self {
            path: PathBuf::new(),
            name,
            default_mode,
            modes,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn selected_mode(&self, requested_mode: &str) -> Result<&RuntimeMode, RuntimeEnvError> {
        let selected_name = if requested_mode == "auto" {
            self.default_mode
                .as_deref()
                .or_else(|| self.modes.first().map(RuntimeMode::name))
        } else {
            Some(requested_mode)
        };
        let Some(selected_name) = selected_name else {
            return Err(RuntimeEnvError::new(
                "runtime manifest has no selectable mode",
            ));
        };
        self.modes
            .iter()
            .find(|mode| mode.name == selected_name)
            .ok_or_else(|| RuntimeEnvError::new(format!("unknown runtime mode: {selected_name}")))
    }

    fn name_or_repo_basename(&self, repo: &Path) -> String {
        self.name.clone().unwrap_or_else(|| {
            repo.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("agent_env")
                .to_string()
        })
    }
}

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
        let checksum = cksum(&format!("{}:{}", repo.display(), mode.name))?;
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
        values.extend(context.mode.env.iter().cloned());
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
        for required in REQUIRED_ENVIRONMENT_VALUES {
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
}

fn split_mapping(content: &str) -> Option<(&str, &str)> {
    let (key, value) = content.split_once(':')?;
    Some((key.trim(), value.trim()))
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2
        && ((value.starts_with('\"') && value.ends_with('\"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
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

fn shell_quote(value: &str) -> String {
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
