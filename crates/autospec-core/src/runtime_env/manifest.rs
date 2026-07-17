use super::{
    is_valid_environment_name, EnvironmentIdentity, ResourcePlan, RuntimeResources,
    BROKER_OWNED_ENVIRONMENT_KEYS,
};
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEnvError {
    message: String,
}

impl RuntimeEnvError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
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
    pub(super) name: String,
    pub(super) command: Option<String>,
    pub(super) down: Option<String>,
    pub(super) env: Vec<(String, String)>,
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
    pub(crate) path: PathBuf,
    pub(crate) name: Option<String>,
    pub(super) version: u32,
    pub(super) default_mode: Option<String>,
    pub(super) modes: Vec<RuntimeMode>,
    pub(super) resources: RuntimeResources,
}

#[derive(Default)]
struct ManifestParseState {
    version: Option<String>,
    name: Option<String>,
    default_mode: Option<String>,
    in_modes: bool,
    current_mode: Option<usize>,
    in_env: bool,
}

impl ManifestParseState {
    fn parse_top_level(&mut self, content: &str) {
        self.current_mode = None;
        self.in_env = false;
        self.in_modes = content == "modes:";
        if self.in_modes {
            return;
        }
        let Some((key, value)) = split_mapping(content) else {
            return;
        };
        match key {
            "version" => self.version = Some(unquote(value)),
            "name" => self.name = Some(unquote(value)),
            "default_mode" => self.default_mode = Some(unquote(value)),
            _ => {}
        }
    }
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
        if super::manifest_v2::is_v2(source) || manifest_version(source).as_deref() == Some("2") {
            return Self::parse_v2(source);
        }
        Self::parse_v1(source)
    }

    fn parse_v1(source: &str) -> Result<Self, RuntimeEnvError> {
        let (state, modes) = parse_legacy_fields(source)?;
        validate_version(state.version.as_deref())?;
        if modes.is_empty() {
            return Err(RuntimeEnvError::new("runtime manifest has no modes"));
        }
        validate_default_mode(state.default_mode.as_deref(), &modes)?;

        Ok(Self {
            path: PathBuf::new(),
            name: state.name,
            version: 1,
            default_mode: state.default_mode,
            modes,
            resources: RuntimeResources::default(),
        })
    }

    fn parse_v2(source: &str) -> Result<Self, RuntimeEnvError> {
        let parsed = super::manifest_v2::parse(source)?;
        validate_default_mode(parsed.default_mode.as_deref(), &parsed.modes)?;
        Ok(Self {
            path: PathBuf::new(),
            name: parsed.name,
            version: 2,
            default_mode: parsed.default_mode,
            modes: parsed.modes,
            resources: parsed.resources,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn resources(&self) -> &RuntimeResources {
        &self.resources
    }

    pub fn resource_plan_for_repo(
        repo: &Path,
        identity: &EnvironmentIdentity,
    ) -> Result<ResourcePlan, RuntimeEnvError> {
        super::resource_plan::for_repo(repo, identity)
    }

    pub fn resource_plan_for_repo_with_overrides(
        repo: &Path,
        identity: &EnvironmentIdentity,
        maven: Option<&str>,
        compose: Option<&str>,
        whole_environment_disabled: bool,
    ) -> Result<(ResourcePlan, bool), RuntimeEnvError> {
        super::resource_plan::for_repo_with_overrides(
            repo,
            identity,
            maven,
            compose,
            whole_environment_disabled,
        )
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

    pub(crate) fn name_or_repo_basename(&self, repo: &Path) -> String {
        self.name.clone().unwrap_or_else(|| {
            repo.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("agent_env")
                .to_string()
        })
    }
}

fn manifest_version(source: &str) -> Option<String> {
    source.lines().find_map(|raw_line| {
        let line = raw_line.trim();
        if raw_line.len() != raw_line.trim_start_matches(' ').len() || line.starts_with('#') {
            return None;
        }
        let (key, value) = split_mapping(line)?;
        (key == "version").then(|| unquote(value))
    })
}

fn parse_legacy_fields(
    source: &str,
) -> Result<(ManifestParseState, Vec<RuntimeMode>), RuntimeEnvError> {
    let mut state = ManifestParseState::default();
    let mut modes = Vec::new();
    let logical_lines = super::shell_command::join_line_continuations(source);
    for (index, raw_line) in logical_lines.iter().enumerate() {
        if raw_line.trim().is_empty() || raw_line.trim_start().starts_with('#') {
            continue;
        }
        let indent = raw_line.len() - raw_line.trim_start_matches(' ').len();
        let content = raw_line.trim();
        if indent == 0 {
            state.parse_top_level(content);
            continue;
        }
        if !state.in_modes {
            continue;
        }
        if let Some(mode_index) = add_mode(content, indent, index + 1, &mut modes)? {
            state.current_mode = Some(mode_index);
            state.in_env = false;
            continue;
        }
        let Some(mode_index) = state.current_mode else {
            continue;
        };
        if set_mode_field(content, indent, &mut modes[mode_index], &mut state.in_env) {
            continue;
        }
        add_environment_entry(content, indent, state.in_env, &mut modes[mode_index])?;
    }
    Ok((state, modes))
}

fn add_mode(
    content: &str,
    indent: usize,
    line_number: usize,
    modes: &mut Vec<RuntimeMode>,
) -> Result<Option<usize>, RuntimeEnvError> {
    if indent != 2 || !content.ends_with(':') {
        return Ok(None);
    }
    let mode_name = content.trim_end_matches(':').trim().to_string();
    if mode_name.is_empty() {
        return Err(RuntimeEnvError::new(format!(
            "empty runtime mode at line {line_number}"
        )));
    }
    if modes.iter().any(|mode| mode.name == mode_name) {
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
    Ok(Some(modes.len() - 1))
}

fn set_mode_field(content: &str, indent: usize, mode: &mut RuntimeMode, in_env: &mut bool) -> bool {
    if indent != 4 {
        return false;
    }
    if content == "env:" {
        *in_env = true;
        return true;
    }
    *in_env = false;
    let Some((key, value)) = split_mapping(content) else {
        return true;
    };
    match key {
        "command" => mode.command = Some(unquote(value)),
        "down" => mode.down = Some(unquote(value)),
        _ => {}
    }
    true
}

fn add_environment_entry(
    content: &str,
    indent: usize,
    in_env: bool,
    mode: &mut RuntimeMode,
) -> Result<(), RuntimeEnvError> {
    if indent != 6 || !in_env {
        return Ok(());
    }
    let Some((key, value)) = split_mapping(content) else {
        return Ok(());
    };
    let key = key.trim().to_string();
    if !is_valid_environment_name(&key) {
        return Err(RuntimeEnvError::new(format!(
            "invalid environment name: {key}"
        )));
    }
    if BROKER_OWNED_ENVIRONMENT_KEYS.contains(&key.as_str()) {
        return Err(RuntimeEnvError::new(format!(
            "reserved runtime environment name: {key}"
        )));
    }
    if mode.env.iter().any(|(existing, _)| existing == &key) {
        return Err(RuntimeEnvError::new(format!(
            "duplicate environment name: {key}"
        )));
    }
    mode.env.push((key, unquote(value)));
    Ok(())
}

fn validate_version(version: Option<&str>) -> Result<(), RuntimeEnvError> {
    match version {
        None | Some("1") => Ok(()),
        Some(version) => Err(RuntimeEnvError::new(format!(
            "unsupported runtime manifest version: {version}"
        ))),
    }
}

fn validate_default_mode(
    default_mode: Option<&str>,
    modes: &[RuntimeMode],
) -> Result<(), RuntimeEnvError> {
    let Some(default_mode) = default_mode else {
        return Ok(());
    };
    if modes.iter().any(|mode| mode.name == default_mode) {
        return Ok(());
    }
    Err(RuntimeEnvError::new(format!(
        "default runtime mode is not declared: {default_mode}"
    )))
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
