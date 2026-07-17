use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

use serde_json::Value;

use super::edit::{candidate_export, Candidate, CandidateKind};
use super::{diagnostic, escape_pointer, normalization_edit, sort_diagnostics, NormalizationEdit};
use crate::runtime_env::{
    ComposeExport, ComposeIsolation, ComposePlan, ComposePolicy, ExportProtocol, ExportValue,
    IsolationDiagnostic, RuntimeEnvError, RuntimeManifest,
};

pub(super) struct ResolvedModel {
    pub bytes: Vec<u8>,
    pub value: Value,
}

pub(super) struct Decision {
    pub approved: HashSet<(usize, usize)>,
    pub edits: Vec<NormalizationEdit>,
    pub exports: Vec<ComposeExport>,
    pub diagnostics: Vec<IsolationDiagnostic>,
}

pub(super) fn load(repo: &Path, plan: &ComposePlan) -> Result<ResolvedModel, RuntimeEnvError> {
    if plan.isolation != ComposeIsolation::Managed {
        return Err(RuntimeEnvError::new("NORMALIZE_VERIFY_COMPOSE_DISABLED"));
    }
    let mut command = Command::new("docker");
    command.args(["compose", "--profile", "*", "--all-resources"]);
    for file in &plan.files {
        command.arg("-f").arg(file);
    }
    let output = command
        .args([
            "--project-name",
            &plan.project_name,
            "config",
            "--format",
            "json",
        ])
        .current_dir(repo)
        .output()
        .map_err(|error| RuntimeEnvError::new(format!("NORMALIZE_COMPOSE_CONFIG_EXEC: {error}")))?;
    if !output.status.success() {
        return Err(RuntimeEnvError::new(format!(
            "NORMALIZE_COMPOSE_CONFIG_FAILED: {}",
            String::from_utf8_lossy(&output.stderr).trim_end()
        )));
    }
    let value = serde_json::from_slice(&output.stdout).map_err(|error| {
        RuntimeEnvError::new(format!(
            "COMPOSE_CONFIG_INVALID_JSON: could not parse resolved Compose model: {error}"
        ))
    })?;
    Ok(ResolvedModel {
        bytes: output.stdout,
        value,
    })
}

pub(super) fn decide(
    repo: &Path,
    environment_id: &str,
    plan: &ComposePlan,
    manifest: &RuntimeManifest,
    model: &Value,
    candidates: &[Vec<Candidate>],
) -> Decision {
    let policy = ComposePolicy::evaluate_in_context(model, plan, environment_id, repo);
    let mut diagnostics = Vec::new();
    let flat = flatten(candidates);
    reject_unsafe_references(repo, environment_id, &flat, &mut diagnostics);
    reject_container_references(repo, environment_id, model, &flat, &mut diagnostics);
    let approved_keys = eligible_candidates(&policy, plan, model, &flat);
    let proposed = proposed_exports(&flat)
        .into_iter()
        .filter(|(key, _)| approved_keys.contains(key))
        .collect::<Vec<_>>();
    reject_export_conflicts(
        repo,
        environment_id,
        &manifest.resources().compose.exports,
        &proposed,
        &mut diagnostics,
    );
    reject_http_ambiguity(
        repo,
        environment_id,
        &manifest.resources().compose.exports,
        &proposed,
        &mut diagnostics,
    );
    let eligible = eligible_diagnostics(&flat, &approved_keys);
    diagnostics.extend(
        policy
            .into_iter()
            .filter(|item| !eligible.contains(&(item.code.clone(), item.resource.clone()))),
    );
    sort_diagnostics(&mut diagnostics);
    if !diagnostics.is_empty() {
        return Decision {
            approved: HashSet::new(),
            edits: Vec::new(),
            exports: Vec::new(),
            diagnostics,
        };
    }
    approve(
        flat.into_iter()
            .filter(|(file, index, _)| approved_keys.contains(&(*file, *index)))
            .collect(),
        &manifest.resources().compose.exports,
        proposed,
    )
}

fn flatten(candidates: &[Vec<Candidate>]) -> Vec<(usize, usize, &Candidate)> {
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

fn proposed_exports(flat: &[(usize, usize, &Candidate)]) -> Vec<((usize, usize), ComposeExport)> {
    flat.iter()
        .filter_map(|(file, index, candidate)| {
            candidate_export(candidate).map(|export| ((*file, *index), export))
        })
        .collect()
}

fn reject_unsafe_references(
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

fn reject_container_references(
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

fn reject_export_conflicts(
    repo: &Path,
    environment_id: &str,
    existing: &[ComposeExport],
    proposed: &[((usize, usize), ComposeExport)],
    diagnostics: &mut Vec<IsolationDiagnostic>,
) {
    let mut by_env: HashMap<&str, &ComposeExport> = HashMap::new();
    for export in existing {
        by_env.insert(&export.env, export);
    }
    for (_, export) in proposed {
        if let Some(previous) = by_env.insert(&export.env, export) {
            if previous != export {
                diagnostics.push(diagnostic(
                    environment_id,
                    repo,
                    "COMPOSE_EXPORT_ENV_CONFLICT",
                    &format!("resources.compose.exports.{}", export.env),
                    &export.env,
                ));
            }
        }
    }
}

fn reject_http_ambiguity(
    repo: &Path,
    environment_id: &str,
    existing: &[ComposeExport],
    proposed: &[((usize, usize), ComposeExport)],
    diagnostics: &mut Vec<IsolationDiagnostic>,
) {
    if existing
        .iter()
        .any(|export| export.env == "AUTOSPEC_PUBLIC_URL")
    {
        return;
    }
    if unique_http_exports(existing, proposed).len() > 1 {
        diagnostics.push(diagnostic(
            environment_id,
            repo,
            "COMPOSE_HTTP_AMBIGUOUS",
            "services.*.ports",
            "multiple-http-candidates",
        ));
    }
}

fn eligible_candidates(
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
                if super::edit::resolved_resource_matches(model, plan, candidate, logical_key) =>
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

fn eligible_diagnostics(
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

fn approve(
    flat: Vec<(usize, usize, &Candidate)>,
    existing: &[ComposeExport],
    proposed: Vec<((usize, usize), ComposeExport)>,
) -> Decision {
    let mut exports = proposed
        .iter()
        .map(|(_, export)| export.clone())
        .filter(|export| !existing.contains(export))
        .collect::<Vec<_>>();
    let http = unique_http_exports(existing, &proposed);
    if !existing
        .iter()
        .any(|export| export.env == "AUTOSPEC_PUBLIC_URL")
    {
        if let [canonical] = http.as_slice() {
            let mut public = (*canonical).clone();
            public.env = "AUTOSPEC_PUBLIC_URL".to_string();
            public.value = ExportValue::Url;
            exports.push(public);
        }
    }
    let export_by_candidate = proposed.into_iter().collect::<HashMap<_, _>>();
    let mut edits = Vec::new();
    let mut approved = HashSet::new();
    for (file, index, candidate) in flat {
        approved.insert((file, index));
        edits.push(normalization_edit(
            candidate,
            export_by_candidate.get(&(file, index)).cloned(),
        ));
    }
    Decision {
        approved,
        edits,
        exports,
        diagnostics: Vec::new(),
    }
}

fn unique_http_exports<'a>(
    existing: &'a [ComposeExport],
    proposed: &'a [((usize, usize), ComposeExport)],
) -> Vec<&'a ComposeExport> {
    let mut unique = Vec::new();
    for export in existing
        .iter()
        .chain(proposed.iter().map(|(_, export)| export))
    {
        if matches!(
            export.protocol,
            ExportProtocol::Http | ExportProtocol::Https
        ) && !unique.contains(&export)
        {
            unique.push(export);
        }
    }
    unique
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
