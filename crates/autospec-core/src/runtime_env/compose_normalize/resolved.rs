use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

use serde_json::Value;

use super::edit::Candidate;
use super::{
    diagnostic, normalization_edit, NormalizationEdit, PlannedFile, RuntimeResourcesReport,
};
use crate::runtime_env::{
    ComposeExport, ComposeIsolation, ComposePlan, ComposePolicy, ExportProtocol, ExportValue,
    IsolationDiagnostic, RuntimeEnvError, RuntimeManifest,
};

mod candidates;

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
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        RuntimeEnvError::new(format!(
            "COMPOSE_CONFIG_INVALID_JSON: could not parse resolved Compose model: {error}"
        ))
    })?;
    let bytes = fingerprint_bytes(repo, plan, &value)?;
    Ok(ResolvedModel { bytes, value })
}

fn fingerprint_bytes(
    repo: &Path,
    plan: &ComposePlan,
    model: &Value,
) -> Result<Vec<u8>, RuntimeEnvError> {
    let mut stable = model.clone();
    normalize_generated_names(&mut stable, &plan.project_name);
    normalize_repo_paths(&mut stable, repo);
    serde_json::to_vec(&stable).map_err(|error| {
        RuntimeEnvError::new(format!(
            "COMPOSE_CONFIG_INVALID_JSON: could not stabilize resolved model: {error}"
        ))
    })
}

fn normalize_generated_names(model: &mut Value, project_name: &str) {
    if model.get("name").and_then(Value::as_str) == Some(project_name) {
        model["name"] = Value::String("${AUTOSPEC_PROJECT}".to_string());
    }
    for kind in ["networks", "volumes"] {
        let Some(resources) = model.get_mut(kind).and_then(Value::as_object_mut) else {
            continue;
        };
        for (logical, settings) in resources {
            let generated = format!("{project_name}_{logical}");
            if settings.get("name").and_then(Value::as_str) == Some(&generated) {
                settings["name"] = Value::String(format!("${{AUTOSPEC_PROJECT}}_{logical}"));
            }
        }
    }
}

fn normalize_repo_paths(model: &mut Value, repo: &Path) {
    let Some(services) = model.get_mut("services").and_then(Value::as_object_mut) else {
        return;
    };
    for service in services.values_mut() {
        let Some(volumes) = service.get_mut("volumes").and_then(Value::as_array_mut) else {
            continue;
        };
        for volume in volumes {
            let Some(source) = volume.get_mut("source") else {
                continue;
            };
            normalize_repo_path(source, repo);
        }
    }
}

fn normalize_repo_path(value: &mut Value, repo: &Path) {
    let Some(path) = value.as_str().map(Path::new) else {
        return;
    };
    let Ok(relative) = path.strip_prefix(repo) else {
        return;
    };
    *value = Value::String(format!("${{AUTOSPEC_REPO}}/{}", relative.display()));
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
    let flat = candidates::flatten(candidates);
    let approved_keys = candidates::eligible(&policy, plan, model, &flat);
    let proposed = candidates::proposed_exports(&flat)
        .into_iter()
        .filter(|(key, _)| approved_keys.contains(key))
        .collect::<Vec<_>>();
    let diagnostics = collect_diagnostics(
        repo,
        environment_id,
        manifest,
        model,
        &flat,
        &approved_keys,
        &proposed,
        policy,
    );
    if !diagnostics.is_empty() {
        return unresolved(diagnostics);
    }
    approve(
        flat.into_iter()
            .filter(|(file, index, _)| approved_keys.contains(&(*file, *index)))
            .collect(),
        &manifest.resources().compose.exports,
        proposed,
    )
}

#[allow(clippy::too_many_arguments)]
fn collect_diagnostics(
    repo: &Path,
    environment_id: &str,
    manifest: &RuntimeManifest,
    model: &Value,
    flat: &[(usize, usize, &Candidate)],
    approved_keys: &HashSet<(usize, usize)>,
    proposed: &[((usize, usize), ComposeExport)],
    policy: Vec<IsolationDiagnostic>,
) -> Vec<IsolationDiagnostic> {
    let mut diagnostics = Vec::new();
    candidates::reject_unsafe_references(repo, environment_id, flat, &mut diagnostics);
    candidates::reject_container_references(repo, environment_id, model, flat, &mut diagnostics);
    reject_export_conflicts(
        repo,
        environment_id,
        &manifest.resources().compose.exports,
        proposed,
        &mut diagnostics,
    );
    reject_http_ambiguity(
        repo,
        environment_id,
        &manifest.resources().compose.exports,
        proposed,
        &mut diagnostics,
    );
    let eligible = candidates::eligible_diagnostics(flat, approved_keys);
    diagnostics.extend(
        policy
            .into_iter()
            .filter(|item| !eligible.contains(&(item.code.clone(), item.resource.clone()))),
    );
    sort_diagnostics(&mut diagnostics);
    diagnostics
}

pub(super) fn unresolved(diagnostics: Vec<IsolationDiagnostic>) -> Decision {
    Decision {
        approved: HashSet::new(),
        edits: Vec::new(),
        exports: Vec::new(),
        diagnostics,
    }
}

pub(super) fn collect_edits(
    runtime_manifest: &RuntimeManifest,
    manifest_path: &Path,
    compose: &ComposePlan,
    files: &[PlannedFile],
    mut decision: Decision,
) -> Vec<NormalizationEdit> {
    let changed = files
        .iter()
        .any(|file| file.path == manifest_path && file.original != file.rendered);
    if changed {
        let mut exports = runtime_manifest.resources().compose.exports.clone();
        for export in &decision.exports {
            if !exports.contains(export) {
                exports.push(export.clone());
            }
        }
        decision
            .edits
            .push(NormalizationEdit::UpsertRuntimeResources {
                resources: RuntimeResourcesReport {
                    compose_files: compose.files.clone(),
                    exports,
                },
            });
    }
    decision.edits
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

fn approve(
    flat: Vec<(usize, usize, &Candidate)>,
    existing: &[ComposeExport],
    proposed: Vec<((usize, usize), ComposeExport)>,
) -> Decision {
    let mut exports = Vec::new();
    for (_, export) in &proposed {
        if !existing.contains(export) && !exports.contains(export) {
            exports.push(export.clone());
        }
    }
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

fn sort_diagnostics(diagnostics: &mut Vec<IsolationDiagnostic>) {
    diagnostics.sort_by(|left, right| {
        left.resource
            .cmp(&right.resource)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.evidence.cmp(&right.evidence))
    });
    diagnostics.dedup();
}
