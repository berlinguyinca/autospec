use std::collections::HashSet;
use std::path::Path;

use serde_json::Value;

use super::super::diagnostic;
use super::super::edit::{candidate_export, resolved_resource_matches, Candidate, CandidateKind};
use crate::runtime_env::{ComposeExport, ComposePlan, IsolationDiagnostic};

pub(super) fn flatten(candidates: &[Vec<Candidate>]) -> Vec<(usize, usize, &Candidate)> {
    candidates
        .iter()
        .enumerate()
        .flat_map(|(file, items)| {
            items
                .iter()
                .enumerate()
                .map(move |(index, item)| (file, index, item))
        })
        .collect()
}

pub(super) fn proposed_exports(
    flat: &[(usize, usize, &Candidate)],
) -> Vec<((usize, usize), ComposeExport)> {
    flat.iter()
        .filter_map(|(file, index, candidate)| {
            candidate_export(candidate).map(|export| ((*file, *index), export))
        })
        .collect()
}

pub(super) fn reject_unsafe_references(
    repo: &Path,
    environment_id: &str,
    flat: &[(usize, usize, &Candidate)],
    diagnostics: &mut Vec<IsolationDiagnostic>,
) {
    diagnostics.extend(
        flat.iter()
            .filter(|(_, _, item)| item.unsafe_reference)
            .map(|(_, _, item)| {
                diagnostic(
                    environment_id,
                    repo,
                    "COMPOSE_UNSAFE_YAML_REFERENCE",
                    &item.resource,
                    &item.evidence,
                )
            }),
    );
}

pub(super) fn reject_container_references(
    repo: &Path,
    environment_id: &str,
    model: &Value,
    flat: &[(usize, usize, &Candidate)],
    diagnostics: &mut Vec<IsolationDiagnostic>,
) {
    for (_, _, candidate) in flat {
        let CandidateKind::ContainerName { service } = &candidate.kind else {
            continue;
        };
        let resolved = model
            .pointer(&format!(
                "/services/{}/container_name",
                escape_pointer(service)
            ))
            .and_then(Value::as_str);
        if resolved != Some(service) || value_references(model, service, &candidate.resource) {
            diagnostics.push(diagnostic(
                environment_id,
                repo,
                "COMPOSE_CONTAINER_NAME_REFERENCE",
                &candidate.resource,
                resolved.unwrap_or(&candidate.evidence),
            ));
        }
    }
}

fn value_references(value: &Value, identity: &str, own_path: &str) -> bool {
    fn visit(value: &Value, path: &str, identity: &str, own_path: &str) -> bool {
        match value {
            Value::Object(values) => values.iter().any(|(key, value)| {
                let next = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                visit(value, &next, identity, own_path)
            }),
            Value::Array(values) => values.iter().enumerate().any(|(index, value)| {
                visit(value, &format!("{path}[{index}]"), identity, own_path)
            }),
            Value::String(value) if path != own_path => {
                value == identity || value.contains(&format!("container:{identity}"))
            }
            _ => false,
        }
    }
    visit(value, "", identity, own_path)
}

pub(super) fn eligible(
    policy: &[IsolationDiagnostic],
    plan: &ComposePlan,
    model: &Value,
    flat: &[(usize, usize, &Candidate)],
) -> HashSet<(usize, usize)> {
    let mut eligible = HashSet::new();
    for (file, index, candidate) in flat {
        let code = match &candidate.kind {
            CandidateKind::Port { .. } => Some("COMPOSE_FIXED_PORT"),
            CandidateKind::ContainerName { service } if resolved_container_is(model, service) => {
                Some("COMPOSE_CONTAINER_NAME")
            }
            CandidateKind::ResourceName { logical_key, .. }
                if resolved_resource_matches(model, plan, candidate, logical_key) =>
            {
                eligible.insert((*file, *index));
                None
            }
            _ => None,
        };
        if code.is_some_and(|code| {
            policy
                .iter()
                .any(|item| item.code == code && item.resource == candidate.resource)
        }) {
            eligible.insert((*file, *index));
        }
    }
    eligible
}

pub(super) fn eligible_diagnostics(
    flat: &[(usize, usize, &Candidate)],
    approved: &HashSet<(usize, usize)>,
) -> HashSet<(String, String)> {
    let mut eligible = HashSet::new();
    for (file, index, candidate) in flat {
        if !approved.contains(&(*file, *index)) {
            continue;
        }
        let code = match candidate.kind {
            CandidateKind::Port { .. } => "COMPOSE_FIXED_PORT",
            CandidateKind::ContainerName { .. } => "COMPOSE_CONTAINER_NAME",
            CandidateKind::ResourceName { .. } => "COMPOSE_GLOBAL_NAME",
        };
        eligible.insert((code.to_string(), candidate.resource.clone()));
        if let CandidateKind::Port { .. } = candidate.kind {
            if let Some(target) = &candidate.target_resource {
                eligible.insert(("COMPOSE_UNDECLARED_PORT".to_string(), target.clone()));
            }
        }
    }
    eligible
}

fn resolved_container_is(model: &Value, service: &str) -> bool {
    model
        .pointer(&format!(
            "/services/{}/container_name",
            escape_pointer(service)
        ))
        .and_then(Value::as_str)
        == Some(service)
}

fn escape_pointer(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}
