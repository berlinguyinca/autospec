use std::collections::HashSet;
use std::str::FromStr;

use yaml_edit::anchor_resolution::DocumentResolvedExt;
use yaml_edit::{AsYaml, Document, Mapping, TextPosition, YamlNode};

use super::ResourceKind;
use crate::runtime_env::{
    ComposeExport, ComposePlan, ExportProtocol, ExportValue, RuntimeEnvError,
};

#[derive(Clone, Debug)]
pub(super) enum CandidateKind {
    Port {
        service: String,
        index: usize,
        target: u16,
        protocol: ExportProtocol,
    },
    ContainerName {
        service: String,
    },
    ResourceName {
        kind: ResourceKind,
        logical_key: String,
    },
}

#[derive(Clone, Debug)]
pub(super) struct Candidate {
    pub kind: CandidateKind,
    pub resource: String,
    pub target_resource: Option<String>,
    pub evidence: String,
    pub unsafe_reference: bool,
    source_edit: SourceEdit,
}

#[derive(Clone, Debug)]
struct SourceEdit {
    start: usize,
    end: usize,
    replacement: String,
}

pub(super) struct InspectedCompose {
    source: String,
    pub candidates: Vec<Candidate>,
}

impl InspectedCompose {
    pub fn render(&self, approved: &HashSet<usize>) -> Vec<u8> {
        let edits = self
            .candidates
            .iter()
            .enumerate()
            .filter(|(index, _)| approved.contains(index))
            .map(|(_, candidate)| candidate.source_edit.clone())
            .collect();
        apply_source_edits(&self.source, edits)
    }
}

pub(super) fn inspect(source: &[u8]) -> Result<InspectedCompose, RuntimeEnvError> {
    let text = std::str::from_utf8(source)
        .map_err(|_| RuntimeEnvError::new("Compose source must be UTF-8"))?;
    let document = Document::from_str(text)
        .map_err(|error| RuntimeEnvError::new(format!("could not parse Compose YAML: {error}")))?;
    let root = document
        .as_mapping()
        .ok_or_else(|| RuntimeEnvError::new("Compose root must be a mapping"))?;
    let anchors = anchor_ranges(&document);
    let mut candidates = collect_ports(&root, &anchors);
    collect_container_names(&root, &anchors, &mut candidates);
    collect_resource_names(&root, &anchors, &mut candidates);
    Ok(InspectedCompose {
        source: text.to_string(),
        candidates,
    })
}

fn collect_ports(root: &Mapping, anchors: &[(usize, usize)]) -> Vec<Candidate> {
    let Some(services) = root.get_mapping("services") else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    for (key, node) in services.iter() {
        let (Some(service), Some(mapping)) = (scalar(&key), node.as_mapping()) else {
            continue;
        };
        let Some(ports) = mapping.get_sequence("ports") else {
            continue;
        };
        for (index, port) in ports.values().enumerate() {
            if let Some(candidate) = port_candidate(&service, index, &port, anchors) {
                candidates.push(candidate);
            }
        }
    }
    candidates
}

fn port_candidate(
    service: &str,
    index: usize,
    node: &YamlNode,
    anchors: &[(usize, usize)],
) -> Option<Candidate> {
    if let Some(value) = scalar(node) {
        let parts = value.split(':').collect::<Vec<_>>();
        let (published, target) = match parts.as_slice() {
            [published, target] => (valid_port(published)?, valid_port(target)?),
            _ => return None,
        };
        let range = node_range(node)?;
        return Some(port(
            service,
            index,
            target,
            ExportProtocol::Tcp,
            published.to_string(),
            overlaps_anchor(range, anchors) || node.as_alias().is_some(),
            replace_range(range, target.to_string()),
        ));
    }
    let mapping = node.as_mapping()?;
    let (published, edit, unsafe_reference) = removable_field(mapping, "published", anchors)?;
    let published_evidence = scalar(&published)
        .and_then(|value| valid_port(&value).map(|port| port.to_string()))
        .or_else(|| unsafe_reference.then(|| published.to_string()))?;
    let target = mapping
        .get("target")
        .and_then(|value| scalar(&value))
        .and_then(|value| valid_port(&value))?;
    let protocol = port_protocol(mapping)?;
    Some(port(
        service,
        index,
        target,
        protocol,
        published_evidence,
        unsafe_reference,
        edit,
    ))
}

#[allow(clippy::too_many_arguments)]
fn port(
    service: &str,
    index: usize,
    target: u16,
    protocol: ExportProtocol,
    evidence: String,
    unsafe_reference: bool,
    source_edit: SourceEdit,
) -> Candidate {
    Candidate {
        kind: CandidateKind::Port {
            service: service.to_string(),
            index,
            target,
            protocol,
        },
        resource: format!("services.{service}.ports[{index}].published"),
        target_resource: Some(format!("services.{service}.ports[{index}].target")),
        evidence,
        unsafe_reference,
        source_edit,
    }
}

fn collect_container_names(
    root: &Mapping,
    anchors: &[(usize, usize)],
    candidates: &mut Vec<Candidate>,
) {
    let Some(services) = root.get_mapping("services") else {
        return;
    };
    for (key, node) in services.iter() {
        let (Some(service), Some(mapping)) = (scalar(&key), node.as_mapping()) else {
            continue;
        };
        let Some((value, edit, unsafe_reference)) =
            removable_field(mapping, "container_name", anchors)
        else {
            continue;
        };
        candidates.push(Candidate {
            kind: CandidateKind::ContainerName {
                service: service.clone(),
            },
            resource: format!("services.{service}.container_name"),
            target_resource: None,
            evidence: scalar(&value).unwrap_or_else(|| value.to_string()),
            unsafe_reference,
            source_edit: edit,
        });
    }
}

fn collect_resource_names(
    root: &Mapping,
    anchors: &[(usize, usize)],
    candidates: &mut Vec<Candidate>,
) {
    for (name, kind) in [
        ("networks", ResourceKind::Network),
        ("volumes", ResourceKind::Volume),
    ] {
        let Some(resources) = root.get_mapping(name) else {
            continue;
        };
        for (key, node) in resources.iter() {
            let (Some(logical), Some(mapping)) = (scalar(&key), node.as_mapping()) else {
                continue;
            };
            let Some((value, edit, unsafe_reference)) = removable_field(mapping, "name", anchors)
            else {
                continue;
            };
            let evidence = scalar(&value).unwrap_or_else(|| value.to_string());
            if evidence != format!("${{COMPOSE_PROJECT_NAME}}_{logical}") {
                continue;
            }
            candidates.push(Candidate {
                kind: CandidateKind::ResourceName {
                    kind: kind.clone(),
                    logical_key: logical.clone(),
                },
                resource: format!("{name}.{logical}.name"),
                target_resource: None,
                evidence,
                unsafe_reference,
                source_edit: edit,
            });
        }
    }
}

fn removable_field(
    mapping: &Mapping,
    key: &str,
    anchors: &[(usize, usize)],
) -> Option<(YamlNode, SourceEdit, bool)> {
    let entry = mapping.find_entry_by_key(key)?;
    let key_node = entry.key_node()?;
    let value = entry.value_node()?;
    let key_range = node_range(&key_node)?;
    let value_range = node_range(&value)?;
    let combined = TextPosition {
        start: key_range.start,
        end: value_range.end,
    };
    let unsafe_reference = value.as_alias().is_some() || overlaps_anchor(combined, anchors);
    Some((
        value,
        SourceEdit {
            start: combined.start as usize,
            end: combined.end as usize,
            replacement: String::new(),
        },
        unsafe_reference,
    ))
}

fn anchor_ranges(document: &Document) -> Vec<(usize, usize)> {
    let registry = document.build_anchor_registry();
    registry
        .anchor_names()
        .filter_map(|name| registry.resolve(name).cloned())
        .map(|node| {
            let range = yaml_edit::advanced::syntax_node_range(&node);
            (
                u32::from(range.start()) as usize,
                u32::from(range.end()) as usize,
            )
        })
        .collect()
}

fn overlaps_anchor(range: TextPosition, anchors: &[(usize, usize)]) -> bool {
    anchors
        .iter()
        .any(|(start, end)| *start < range.end as usize && (range.start as usize) < *end)
}

fn node_range(node: &YamlNode) -> Option<TextPosition> {
    let range = yaml_edit::advanced::syntax_node_range(node.as_node()?);
    Some(TextPosition {
        start: u32::from(range.start()),
        end: u32::from(range.end()),
    })
}

fn port_protocol(mapping: &Mapping) -> Option<ExportProtocol> {
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

pub(super) fn candidate_export(candidate: &Candidate) -> Option<ComposeExport> {
    let CandidateKind::Port {
        service,
        target,
        protocol,
        ..
    } = &candidate.kind
    else {
        return None;
    };
    let value = if matches!(protocol, ExportProtocol::Http | ExportProtocol::Https) {
        ExportValue::Url
    } else {
        ExportValue::Port
    };
    Some(ComposeExport {
        service: service.clone(),
        target: *target,
        protocol: protocol.clone(),
        env: export_name(service, *target, protocol),
        value,
    })
}

fn export_name(service: &str, target: u16, protocol: &ExportProtocol) -> String {
    let service = service
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let protocol = match protocol {
        ExportProtocol::Http => "HTTP",
        ExportProtocol::Https => "HTTPS",
        ExportProtocol::Tcp => "TCP",
        ExportProtocol::Udp => "UDP",
    };
    format!("AUTOSPEC_COMPOSE_{service}_{target}_{protocol}")
}

fn scalar(node: &YamlNode) -> Option<String> {
    node.as_scalar().map(|value| value.as_string())
}

fn scalar_at(mapping: &Mapping, key: &str) -> Option<String> {
    mapping.get(key).and_then(|node| scalar(&node))
}

fn valid_port(value: &str) -> Option<u16> {
    value.parse::<u16>().ok().filter(|port| *port > 0)
}

fn replace_range(range: TextPosition, replacement: String) -> SourceEdit {
    SourceEdit {
        start: range.start as usize,
        end: range.end as usize,
        replacement,
    }
}

fn apply_source_edits(source: &str, mut edits: Vec<SourceEdit>) -> Vec<u8> {
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.start));
    let mut output = source.to_string();
    for edit in edits {
        output.replace_range(edit.start..edit.end, &edit.replacement);
    }
    output.into_bytes()
}

pub(super) fn resolved_resource_matches(
    model: &serde_json::Value,
    plan: &ComposePlan,
    candidate: &Candidate,
    logical: &str,
) -> bool {
    model
        .pointer(&format!("/{}", candidate.resource.replace('.', "/")))
        .and_then(serde_json::Value::as_str)
        == Some(&format!("{}_{}", plan.project_name, logical))
}
