use std::path::Path;
use std::str::FromStr;

use yaml_edit::{Document, TextPosition, YamlNode};

use super::{diagnostic, NormalizationEdit, ResourceKind};
use crate::runtime_env::{
    ComposeExport, ExportProtocol, ExportValue, IsolationDiagnostic, RuntimeEnvError,
};

pub(super) struct EditedCompose {
    pub bytes: Vec<u8>,
    pub edits: Vec<NormalizationEdit>,
    pub exports: Vec<ComposeExport>,
    pub diagnostics: Vec<IsolationDiagnostic>,
}

#[derive(Clone)]
struct PortEdit {
    service: String,
    index: usize,
    target: u16,
    protocol: ExportProtocol,
    source: SourceEdit,
}

#[derive(Clone)]
struct SourceEdit {
    start: usize,
    end: usize,
    replacement: String,
}

pub(super) fn edit_compose(
    source: &[u8],
    environment_id: &str,
    repo: &Path,
) -> Result<EditedCompose, RuntimeEnvError> {
    let text = std::str::from_utf8(source)
        .map_err(|_| RuntimeEnvError::new("Compose source must be UTF-8"))?;
    let document = Document::from_str(text)
        .map_err(|error| RuntimeEnvError::new(format!("could not parse Compose YAML: {error}")))?;
    let root = document
        .as_mapping()
        .ok_or_else(|| RuntimeEnvError::new("Compose root must be a mapping"))?;
    let mut diagnostics = Vec::new();
    reject_yaml_references(&root, environment_id, repo, &mut diagnostics);
    let ports = collect_ports(&root, text, environment_id, repo, &mut diagnostics);
    reject_unsafe_resources(&root, environment_id, repo, &mut diagnostics);
    reject_identity_references(&root, environment_id, repo, &mut diagnostics);
    if http_count(&ports) > 1 {
        diagnostics.push(diagnostic(
            environment_id,
            repo,
            "COMPOSE_HTTP_AMBIGUOUS",
            "services.*.ports",
            "multiple-http-candidates",
        ));
    }
    if !diagnostics.is_empty() {
        return Ok(unchanged(source, diagnostics));
    }
    let (mut changes, mut edits, mut exports) = port_changes(&ports);
    collect_name_changes(&root, text, &mut changes, &mut edits);
    add_public_url(&ports, &mut exports);
    Ok(EditedCompose {
        bytes: apply_source_edits(text, changes),
        edits,
        exports,
        diagnostics,
    })
}

fn unchanged(source: &[u8], diagnostics: Vec<IsolationDiagnostic>) -> EditedCompose {
    EditedCompose {
        bytes: source.to_vec(),
        edits: Vec::new(),
        exports: Vec::new(),
        diagnostics,
    }
}

fn reject_yaml_references(
    root: &yaml_edit::Mapping,
    environment_id: &str,
    repo: &Path,
    diagnostics: &mut Vec<IsolationDiagnostic>,
) {
    if root.values().any(|value| node_has_reference(&value)) {
        diagnostics.push(diagnostic(
            environment_id,
            repo,
            "COMPOSE_UNSAFE_YAML_REFERENCE",
            "compose",
            "anchor-or-alias",
        ));
    }
}

fn node_has_reference(node: &YamlNode) -> bool {
    node.as_alias().is_some()
        || node.to_string().trim_start().starts_with('&')
        || node
            .as_mapping()
            .is_some_and(|mapping| mapping.values().any(|value| node_has_reference(&value)))
        || node
            .as_sequence()
            .is_some_and(|sequence| sequence.values().any(|value| node_has_reference(&value)))
}

#[rustfmt::skip]
fn collect_ports(root: &yaml_edit::Mapping, source: &str, environment_id: &str,
    repo: &Path, diagnostics: &mut Vec<IsolationDiagnostic>) -> Vec<PortEdit> {
    let Some(services) = root.get_mapping("services") else { return Vec::new() };
    let mut edits = Vec::new();
    for (key, value) in services.iter() {
        let Some(service) = scalar(&key) else { continue };
        let Some(mapping) = value.as_mapping() else { continue };
        if scalar_at(mapping, "network_mode").as_deref() == Some("host") {
            diagnostics.push(diagnostic(environment_id, repo, "COMPOSE_HOST_NETWORK",
                &format!("services.{service}.network_mode"), "host"));
        }
        let Some(ports) = mapping.get_sequence("ports") else { continue };
        for (index, port) in ports.values().enumerate() {
            match parse_port(&service, index, &port, source) {
                Ok(Some(edit)) => edits.push(edit),
                Ok(None) => {}
                Err(evidence) => diagnostics.push(diagnostic(environment_id, repo,
                    "COMPOSE_UNDECLARED_PORT", &format!("services.{service}.ports[{index}]"), &evidence)),
            }
        }
    }
    edits
}

#[rustfmt::skip]
fn parse_port(service: &str, index: usize, node: &YamlNode, source: &str) -> Result<Option<PortEdit>, String> {
    if let Some(value) = scalar(node) {
        let parts = value.split(':').collect::<Vec<_>>();
        if parts.len() == 1 && valid_port(parts[0]).is_some() { return Ok(None) }
        if parts.len() != 2 { return Err(value) }
        valid_port(parts[0]).ok_or_else(|| value.clone())?;
        let target = valid_port(parts[1]).ok_or_else(|| value.clone())?;
        let range = node.as_scalar().expect("scalar branch").byte_range();
        return Ok(Some(PortEdit { service: service.to_string(), index, target,
            protocol: ExportProtocol::Tcp, source: replace_range(range, target.to_string()) }));
    }
    let mapping = node.as_mapping().ok_or_else(|| node.to_string())?;
    let Some(published) = mapping.get("published") else { return Ok(None) };
    let published_number = published.to_i64().and_then(|value| u16::try_from(value).ok());
    if published_number.is_none_or(|value| value == 0) { return Err(node.to_string()) }
    let target = mapping.get("target").and_then(|value| value.to_i64())
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| node.to_string())?;
    let protocol = port_protocol(mapping).ok_or_else(|| node.to_string())?;
    let range = published.as_scalar().ok_or_else(|| node.to_string())?.byte_range();
    Ok(Some(PortEdit { service: service.to_string(), index, target, protocol,
        source: remove_line(source, range) }))
}

fn port_protocol(mapping: &yaml_edit::Mapping) -> Option<ExportProtocol> {
    let transport = scalar_at(mapping, "protocol").unwrap_or_else(|| "tcp".to_string());
    match (
        transport.as_str(),
        scalar_at(mapping, "app_protocol").as_deref(),
    ) {
        ("tcp", Some("http")) => Some(ExportProtocol::Http),
        ("tcp", Some("https")) => Some(ExportProtocol::Https),
        ("tcp", None) => Some(ExportProtocol::Tcp),
        ("udp", None) => Some(ExportProtocol::Udp),
        _ => None,
    }
}

#[rustfmt::skip]
fn reject_unsafe_resources(root: &yaml_edit::Mapping, environment_id: &str,
    repo: &Path, diagnostics: &mut Vec<IsolationDiagnostic>) {
    for kind in ["networks", "volumes"] {
        let Some(resources) = root.get_mapping(kind) else { continue };
        for (key, value) in resources.iter() {
            let Some(logical) = scalar(&key) else { continue };
            let Some(mapping) = value.as_mapping() else { continue };
            if mapping.get("external").and_then(|value| value.to_bool()) == Some(true) {
                diagnostics.push(diagnostic(environment_id, repo, "COMPOSE_EXTERNAL_UNDECLARED",
                    &format!("{kind}.{logical}.external"), &logical));
            }
        }
    }
}

#[rustfmt::skip]
fn reject_identity_references(root: &yaml_edit::Mapping, environment_id: &str,
    repo: &Path, diagnostics: &mut Vec<IsolationDiagnostic>) {
    let Some(services) = root.get_mapping("services") else { return };
    for (key, value) in services.iter() {
        let Some(name) = scalar(&key) else { continue };
        let Some(service) = value.as_mapping() else { continue };
        let references = root.values().map(|node| scalar_count(&node, &name)).sum::<usize>();
        if scalar_at(service, "container_name").as_deref() == Some(name.as_str()) && references > 1 {
            diagnostics.push(diagnostic(environment_id, repo, "COMPOSE_CONTAINER_NAME_REFERENCE",
                &format!("services.{name}.container_name"), &name));
        }
    }
}

#[rustfmt::skip]
fn scalar_count(node: &YamlNode, expected: &str) -> usize {
    if scalar(node).as_deref() == Some(expected) { return 1 }
    if let Some(mapping) = node.as_mapping() {
        return mapping.values().map(|value| scalar_count(&value, expected)).sum();
    }
    node.as_sequence().map(|sequence|
        sequence.values().map(|value| scalar_count(&value, expected)).sum()).unwrap_or(0)
}

fn port_changes(
    ports: &[PortEdit],
) -> (Vec<SourceEdit>, Vec<NormalizationEdit>, Vec<ComposeExport>) {
    let mut changes = Vec::new();
    let mut edits = Vec::new();
    let mut exports = Vec::new();
    for port in ports {
        changes.push(port.source.clone());
        let value = if matches!(port.protocol, ExportProtocol::Http | ExportProtocol::Https) {
            ExportValue::Url
        } else {
            ExportValue::Port
        };
        let export = compose_export(port, export_name(port), value);
        exports.push(export.clone());
        edits.push(NormalizationEdit::RemovePublishedPort {
            service: port.service.clone(),
            index: port.index,
            export,
        });
    }
    (changes, edits, exports)
}

#[rustfmt::skip]
fn collect_name_changes(root: &yaml_edit::Mapping, source: &str,
    changes: &mut Vec<SourceEdit>, edits: &mut Vec<NormalizationEdit>) {
    if let Some(services) = root.get_mapping("services") {
        for (key, value) in services.iter() {
            let Some(name) = scalar(&key) else { continue };
            let Some(service) = value.as_mapping() else { continue };
            let Some(node) = service.get("container_name") else { continue };
            if scalar(&node).as_deref() == Some(name.as_str()) {
                changes.push(remove_node_line(source, &node));
                edits.push(NormalizationEdit::RemoveRedundantContainerName { service: name });
            }
        }
    }
    for (name, kind) in [("networks", ResourceKind::Network), ("volumes", ResourceKind::Volume)] {
        collect_resource_names(root, source, name, kind, changes, edits);
    }
}

#[rustfmt::skip]
fn collect_resource_names(root: &yaml_edit::Mapping, source: &str, name: &str,
    kind: ResourceKind, changes: &mut Vec<SourceEdit>, edits: &mut Vec<NormalizationEdit>) {
    let Some(resources) = root.get_mapping(name) else { return };
    for (key, value) in resources.iter() {
        let Some(logical) = scalar(&key) else { continue };
        let Some(resource) = value.as_mapping() else { continue };
        let Some(node) = resource.get("name") else { continue };
        if scalar(&node).as_deref() == Some(&format!("${{COMPOSE_PROJECT_NAME}}_{logical}")) {
            changes.push(remove_node_line(source, &node));
            edits.push(NormalizationEdit::RemoveProjectScopedResourceName {
                kind: kind.clone(), logical_key: logical });
        }
    }
}

fn add_public_url(ports: &[PortEdit], exports: &mut Vec<ComposeExport>) {
    let http = ports
        .iter()
        .filter(|port| matches!(port.protocol, ExportProtocol::Http | ExportProtocol::Https))
        .collect::<Vec<_>>();
    if let [port] = http.as_slice() {
        exports.push(compose_export(
            port,
            "AUTOSPEC_PUBLIC_URL".to_string(),
            ExportValue::Url,
        ));
    }
}

fn compose_export(port: &PortEdit, env: String, value: ExportValue) -> ComposeExport {
    ComposeExport {
        service: port.service.clone(),
        target: port.target,
        protocol: port.protocol.clone(),
        env,
        value,
    }
}

#[rustfmt::skip]
fn export_name(port: &PortEdit) -> String {
    let service = port.service.chars().map(|character|
        if character.is_ascii_alphanumeric() { character.to_ascii_uppercase() } else { '_' }
    ).collect::<String>();
    let protocol = match port.protocol {
        ExportProtocol::Http => "HTTP",
        ExportProtocol::Https => "HTTPS",
        ExportProtocol::Tcp => "TCP",
        ExportProtocol::Udp => "UDP",
    };
    format!("AUTOSPEC_COMPOSE_{service}_{}_{protocol}", port.target)
}

fn valid_port(value: &str) -> Option<u16> {
    value.parse::<u16>().ok().filter(|port| *port > 0)
}

fn scalar(node: &YamlNode) -> Option<String> {
    node.as_scalar().map(|value| value.as_string())
}

fn scalar_at(mapping: &yaml_edit::Mapping, key: &str) -> Option<String> {
    mapping.get(key).and_then(|node| scalar(&node))
}

fn replace_range(range: TextPosition, replacement: String) -> SourceEdit {
    SourceEdit {
        start: range.start as usize,
        end: range.end as usize,
        replacement,
    }
}

fn remove_node_line(source: &str, node: &YamlNode) -> SourceEdit {
    let range = node
        .as_scalar()
        .expect("name fields are scalars")
        .byte_range();
    remove_line(source, range)
}

fn remove_line(source: &str, range: TextPosition) -> SourceEdit {
    let start = source[..range.start as usize]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let end = source[range.end as usize..]
        .find('\n')
        .map_or(source.len(), |index| range.end as usize + index + 1);
    SourceEdit {
        start,
        end,
        replacement: String::new(),
    }
}

fn apply_source_edits(source: &str, edits: Vec<SourceEdit>) -> Vec<u8> {
    let mut edits = edits;
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.start));
    let mut output = source.to_string();
    for edit in edits {
        output.replace_range(edit.start..edit.end, &edit.replacement);
    }
    output.into_bytes()
}

fn http_count(edits: &[PortEdit]) -> usize {
    edits
        .iter()
        .filter(|edit| matches!(edit.protocol, ExportProtocol::Http | ExportProtocol::Https))
        .count()
}
