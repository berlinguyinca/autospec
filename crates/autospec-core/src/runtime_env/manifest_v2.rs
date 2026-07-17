use std::collections::HashSet;
use std::path::PathBuf;
use std::str::FromStr;

use yaml_edit::{Document, Mapping};

use super::is_valid_environment_name;
use super::manifest::RuntimeEnvError;
use super::manifest::RuntimeMode;
use super::resources::{
    reject_duplicates, validate_logical_keys, ComposeExport, ComposeIsolation,
    ComposeResourceConfig, ExportProtocol, ExportValue, MavenIsolation, MavenResourceConfig,
    RuntimeResources,
};

pub(super) struct ParsedManifest {
    pub(super) name: Option<String>,
    pub(super) default_mode: Option<String>,
    pub(super) modes: Vec<RuntimeMode>,
    pub(super) resources: RuntimeResources,
}

pub(super) fn is_v2(source: &str) -> bool {
    Document::from_str(source)
        .ok()
        .and_then(|document| document.as_mapping())
        .and_then(|mapping| mapping.get("version"))
        .and_then(|value| value.as_scalar().map(|scalar| scalar.as_string()))
        .is_some_and(|value| value == "2")
}

pub(super) fn parse(source: &str) -> Result<ParsedManifest, RuntimeEnvError> {
    let document = Document::from_str(source).map_err(|error| {
        RuntimeEnvError::new(format!("could not parse runtime manifest YAML: {error}"))
    })?;
    let root = document
        .as_mapping()
        .ok_or_else(|| RuntimeEnvError::new("runtime manifest version 2 root must be a mapping"))?;
    validate_allowed_keys(
        &root,
        &["version", "name", "default_mode", "resources", "modes"],
        "runtime manifest",
    )?;
    let version = required_scalar(&root, "version", "runtime manifest version")?;
    if version != "2" {
        return Err(RuntimeEnvError::new(format!(
            "unsupported runtime manifest version: {version}"
        )));
    }
    let resources = optional_mapping(&root, "resources", "runtime resources")?
        .as_ref()
        .map(parse_resources)
        .transpose()?
        .unwrap_or_default();
    Ok(ParsedManifest {
        name: optional_scalar(&root, "name", "runtime manifest name")?,
        default_mode: optional_scalar(&root, "default_mode", "default runtime mode")?,
        modes: parse_modes(&root)?,
        resources,
    })
}

fn parse_modes(root: &Mapping) -> Result<Vec<RuntimeMode>, RuntimeEnvError> {
    let Some(mapping) = optional_mapping(root, "modes", "runtime modes")? else {
        return Ok(Vec::new());
    };
    let mut modes = Vec::new();
    for entry in mapping.entries() {
        let name = entry
            .key_node()
            .ok_or_else(|| RuntimeEnvError::new("runtime mode name is missing"))?
            .as_scalar()
            .map(|value| value.as_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RuntimeEnvError::new("runtime mode name must be a string"))?;
        if modes.iter().any(|mode: &RuntimeMode| mode.name == name) {
            return Err(RuntimeEnvError::new(format!(
                "duplicate runtime mode: {name}"
            )));
        }
        let value = entry
            .value_node()
            .ok_or_else(|| RuntimeEnvError::new(format!("runtime mode {name} has no value")))?;
        let fields = value.as_mapping().ok_or_else(|| {
            RuntimeEnvError::new(format!("runtime mode {name} must be a mapping"))
        })?;
        validate_allowed_keys(fields, &["command", "down", "env"], "runtime mode")?;
        modes.push(RuntimeMode {
            name,
            command: optional_scalar(fields, "command", "runtime mode command")?,
            down: optional_scalar(fields, "down", "runtime mode down command")?,
            env: parse_mode_environment(fields)?,
        });
    }
    Ok(modes)
}

fn parse_mode_environment(fields: &Mapping) -> Result<Vec<(String, String)>, RuntimeEnvError> {
    let Some(mapping) = optional_mapping(fields, "env", "runtime mode environment")? else {
        return Ok(Vec::new());
    };
    let mut environment = Vec::new();
    for entry in mapping.entries() {
        let key = entry
            .key_node()
            .ok_or_else(|| RuntimeEnvError::new("runtime environment name is missing"))?
            .as_scalar()
            .map(|value| value.as_string())
            .ok_or_else(|| RuntimeEnvError::new("runtime environment name must be a string"))?;
        if !is_valid_environment_name(&key)
            || super::BROKER_OWNED_ENVIRONMENT_KEYS.contains(&key.as_str())
        {
            return Err(RuntimeEnvError::new(format!(
                "invalid environment name: {key}"
            )));
        }
        let value = entry
            .value_node()
            .ok_or_else(|| RuntimeEnvError::new(format!("runtime environment {key} has no value")))?
            .as_scalar()
            .map(|value| value.as_string())
            .ok_or_else(|| {
                RuntimeEnvError::new(format!("runtime environment {key} must be a scalar"))
            })?;
        environment.push((key, value));
    }
    Ok(environment)
}

fn parse_resources(resources: &Mapping) -> Result<RuntimeResources, RuntimeEnvError> {
    validate_allowed_keys(resources, &["maven", "compose"], "runtime resources")?;
    let maven = optional_mapping(resources, "maven", "Maven resources")?
        .as_ref()
        .map(parse_maven_resources)
        .transpose()?
        .unwrap_or(MavenResourceConfig {
            isolation: MavenIsolation::SplitLocal,
        });
    let compose = optional_mapping(resources, "compose", "Compose resources")?
        .as_ref()
        .map(parse_compose_resources)
        .transpose()?
        .unwrap_or_else(|| RuntimeResources::default().compose);
    Ok(RuntimeResources { maven, compose })
}

fn parse_maven_resources(mapping: &Mapping) -> Result<MavenResourceConfig, RuntimeEnvError> {
    validate_allowed_keys(mapping, &["isolation"], "Maven resources")?;
    let isolation = match optional_scalar(mapping, "isolation", "Maven isolation")?.as_deref() {
        None | Some("split-local") => MavenIsolation::SplitLocal,
        Some("off") => MavenIsolation::Off,
        Some(value) => {
            return Err(RuntimeEnvError::new(format!(
                "unsupported Maven isolation: {value}"
            )))
        }
    };
    Ok(MavenResourceConfig { isolation })
}

fn parse_compose_resources(mapping: &Mapping) -> Result<ComposeResourceConfig, RuntimeEnvError> {
    validate_allowed_keys(
        mapping,
        &[
            "isolation",
            "files",
            "exports",
            "preserve_volumes",
            "shared_resources",
        ],
        "Compose resources",
    )?;
    let isolation = parse_compose_isolation(mapping)?;
    let files = scalar_list(mapping, "files", "Compose files")?
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    reject_duplicates(&files, "duplicate Compose file")?;
    let exports = parse_exports(mapping)?;
    let preserve_volumes = scalar_list(mapping, "preserve_volumes", "preserved Compose volumes")?;
    validate_logical_keys(&preserve_volumes, "invalid preserved Compose volume key")?;
    reject_duplicates(&preserve_volumes, "duplicate preserved Compose volume key")?;
    let (shared_networks, shared_volumes) = parse_shared_resources(mapping)?;
    Ok(ComposeResourceConfig {
        isolation,
        files,
        exports,
        preserve_volumes,
        shared_networks,
        shared_volumes,
    })
}

fn parse_compose_isolation(mapping: &Mapping) -> Result<ComposeIsolation, RuntimeEnvError> {
    match optional_scalar(mapping, "isolation", "Compose isolation")?.as_deref() {
        None => Ok(ComposeIsolation::Managed),
        Some("off") => Ok(ComposeIsolation::Off),
        Some(value) => Err(RuntimeEnvError::new(format!(
            "unsupported Compose isolation: {value}"
        ))),
    }
}

fn parse_shared_resources(
    mapping: &Mapping,
) -> Result<(Vec<String>, Vec<String>), RuntimeEnvError> {
    let Some(shared) = optional_mapping(mapping, "shared_resources", "shared Compose resources")?
    else {
        return Ok((Vec::new(), Vec::new()));
    };
    validate_allowed_keys(
        &shared,
        &["networks", "volumes"],
        "shared Compose resources",
    )?;
    let networks = scalar_list(&shared, "networks", "shared Compose networks")?;
    let volumes = scalar_list(&shared, "volumes", "shared Compose volumes")?;
    validate_logical_keys(&networks, "invalid shared Compose network key")?;
    validate_logical_keys(&volumes, "invalid shared Compose volume key")?;
    reject_duplicates(&networks, "duplicate shared Compose network key")?;
    reject_duplicates(&volumes, "duplicate shared Compose volume key")?;
    Ok((networks, volumes))
}

fn parse_exports(mapping: &Mapping) -> Result<Vec<ComposeExport>, RuntimeEnvError> {
    let Some(node) = mapping.get("exports") else {
        return Ok(Vec::new());
    };
    let sequence = node
        .as_sequence()
        .ok_or_else(|| RuntimeEnvError::new("Compose exports must be a list"))?;
    let mut exports = Vec::new();
    let mut environment_names = HashSet::new();
    for node in sequence.values() {
        let mapping = node
            .as_mapping()
            .ok_or_else(|| RuntimeEnvError::new("each Compose export must be a mapping"))?;
        let export = parse_export(mapping)?;
        if !environment_names.insert(export.env.clone()) {
            return Err(RuntimeEnvError::new(format!(
                "duplicate Compose export environment name: {}",
                export.env
            )));
        }
        exports.push(export);
    }
    Ok(exports)
}

fn parse_export(mapping: &Mapping) -> Result<ComposeExport, RuntimeEnvError> {
    validate_allowed_keys(
        mapping,
        &["service", "target", "protocol", "env", "value"],
        "Compose export",
    )?;
    let service = required_scalar(mapping, "service", "Compose export service")?;
    let target = mapping
        .get("target")
        .and_then(|value| value.to_i64())
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| RuntimeEnvError::new("Compose export target must be 1..=65535"))?;
    let protocol = ExportProtocol::parse(&required_scalar(
        mapping,
        "protocol",
        "Compose export protocol",
    )?)?;
    let env = required_scalar(mapping, "env", "Compose export environment name")?;
    if !is_valid_environment_name(&env) {
        return Err(RuntimeEnvError::new(format!(
            "invalid Compose export environment name: {env}"
        )));
    }
    let value = parse_export_value(mapping, &protocol)?;
    Ok(ComposeExport {
        service,
        target,
        protocol,
        env,
        value,
    })
}

fn parse_export_value(
    mapping: &Mapping,
    protocol: &ExportProtocol,
) -> Result<ExportValue, RuntimeEnvError> {
    let value = match optional_scalar(mapping, "value", "Compose export value")?.as_deref() {
        None if matches!(protocol, ExportProtocol::Http | ExportProtocol::Https) => {
            ExportValue::Url
        }
        None => ExportValue::Port,
        Some("url") => ExportValue::Url,
        Some("port") => ExportValue::Port,
        Some("host-port") => ExportValue::HostPort,
        Some(value) => {
            return Err(RuntimeEnvError::new(format!(
                "unsupported Compose export value: {value}"
            )))
        }
    };
    let compatible = matches!(value, ExportValue::HostPort)
        || matches!(protocol, ExportProtocol::Http | ExportProtocol::Https)
            && matches!(value, ExportValue::Url)
        || matches!(protocol, ExportProtocol::Tcp | ExportProtocol::Udp)
            && matches!(value, ExportValue::Port);
    if !compatible {
        return Err(RuntimeEnvError::new(
            "incompatible Compose export protocol and value",
        ));
    }
    Ok(value)
}

fn validate_allowed_keys(
    mapping: &Mapping,
    allowed: &[&str],
    label: &str,
) -> Result<(), RuntimeEnvError> {
    let mut seen = HashSet::new();
    for key in mapping.keys() {
        let key = key
            .as_scalar()
            .map(|scalar| scalar.as_string())
            .ok_or_else(|| RuntimeEnvError::new(format!("{label} key must be a string")))?;
        if !seen.insert(key.clone()) {
            return Err(RuntimeEnvError::new(format!(
                "duplicate {label} key: {key}"
            )));
        }
        if !allowed.contains(&key.as_str()) {
            let prefix = if label == "runtime resources" {
                "unknown runtime resource key"
            } else {
                "unknown key"
            };
            return Err(RuntimeEnvError::new(format!("{prefix} in {label}: {key}")));
        }
    }
    Ok(())
}

fn optional_mapping(
    mapping: &Mapping,
    key: &str,
    label: &str,
) -> Result<Option<Mapping>, RuntimeEnvError> {
    let Some(node) = mapping.get(key) else {
        return Ok(None);
    };
    node.as_mapping()
        .cloned()
        .map(Some)
        .ok_or_else(|| RuntimeEnvError::new(format!("{label} must be a mapping")))
}

fn optional_scalar(
    mapping: &Mapping,
    key: &str,
    label: &str,
) -> Result<Option<String>, RuntimeEnvError> {
    let Some(node) = mapping.get(key) else {
        return Ok(None);
    };
    node.as_scalar()
        .map(|scalar| scalar.as_string())
        .map(Some)
        .ok_or_else(|| RuntimeEnvError::new(format!("{label} must be a scalar")))
}

fn required_scalar(mapping: &Mapping, key: &str, label: &str) -> Result<String, RuntimeEnvError> {
    optional_scalar(mapping, key, label)?
        .filter(|value| !value.is_empty())
        .ok_or_else(|| RuntimeEnvError::new(format!("{label} is required")))
}

fn scalar_list(mapping: &Mapping, key: &str, label: &str) -> Result<Vec<String>, RuntimeEnvError> {
    let Some(node) = mapping.get(key) else {
        return Ok(Vec::new());
    };
    let sequence = node
        .as_sequence()
        .ok_or_else(|| RuntimeEnvError::new(format!("{label} must be a list")))?;
    sequence
        .values()
        .map(|value| {
            value
                .as_scalar()
                .map(|scalar| scalar.as_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| RuntimeEnvError::new(format!("{label} entries must be strings")))
        })
        .collect()
}
