use std::collections::HashSet;
use std::path::Path;

use super::{
    diagnostic, IsolationDiagnostic, NormalizationEdit, NormalizationPlan, PlannedFile,
    NORMALIZATION_SCHEMA_VERSION,
};
use super::{edit, fingerprint, manifest, resolved};
use crate::runtime_env::{ComposePlan, RuntimeEnvError, RuntimeManifest, COMPOSE_POLICY_VERSION};

pub(super) fn build(
    repo: &Path,
    runtime_manifest: RuntimeManifest,
    compose: ComposePlan,
    environment_id: &str,
    model: resolved::ResolvedModel,
) -> Result<NormalizationPlan, RuntimeEnvError> {
    let manifest_path = manifest::canonical_path(&runtime_manifest)?;
    let mut files = fingerprint::read_inputs(repo, &compose.files, &manifest_path)?;
    let (compose_indices, inspected) = inspect_sources(&files, &manifest_path)?;
    let candidates = candidate_sets(&inspected);
    let decision = decide_or_reject_flow(DecisionInput {
        repo,
        environment_id,
        runtime_manifest: &runtime_manifest,
        manifest_path: &manifest_path,
        files: &files,
        compose: &compose,
        model: &model,
        candidates: &candidates,
    })?;
    render_decision(
        repo,
        &runtime_manifest,
        &manifest_path,
        &compose,
        &compose_indices,
        &inspected,
        &decision,
        &mut files,
    )?;
    let diagnostics = decision.diagnostics.clone();
    let edits = resolved::collect_edits(
        &runtime_manifest,
        &manifest_path,
        &compose,
        &files,
        decision,
    );
    finish(
        repo,
        environment_id,
        compose,
        model,
        files,
        edits,
        diagnostics,
    )
}

fn candidate_sets(inspected: &[edit::InspectedCompose]) -> Vec<Vec<edit::Candidate>> {
    inspected
        .iter()
        .map(|source| source.candidates.clone())
        .collect()
}

fn inspect_sources(
    files: &[PlannedFile],
    manifest_path: &Path,
) -> Result<(Vec<usize>, Vec<edit::InspectedCompose>), RuntimeEnvError> {
    let indices = files
        .iter()
        .enumerate()
        .filter(|(_, file)| file.path != manifest_path)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let inspected = indices
        .iter()
        .map(|index| edit::inspect(&files[*index].original))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((indices, inspected))
}

struct DecisionInput<'a> {
    repo: &'a Path,
    environment_id: &'a str,
    runtime_manifest: &'a RuntimeManifest,
    manifest_path: &'a Path,
    files: &'a [PlannedFile],
    compose: &'a ComposePlan,
    model: &'a resolved::ResolvedModel,
    candidates: &'a [Vec<edit::Candidate>],
}

fn decide_or_reject_flow(input: DecisionInput<'_>) -> Result<resolved::Decision, RuntimeEnvError> {
    let source = input
        .files
        .iter()
        .find(|file| file.path == input.manifest_path)
        .ok_or_else(|| RuntimeEnvError::new("runtime manifest was not planned"))?;
    if source.identity.is_none() {
        return Ok(resolved::decide(
            input.repo,
            input.environment_id,
            input.compose,
            input.runtime_manifest,
            &input.model.value,
            input.candidates,
        ));
    }
    let text = std::str::from_utf8(&source.original)
        .map_err(|_| RuntimeEnvError::new("runtime manifest must be UTF-8"))?;
    if manifest::flow_mapping_unsupported(text)? {
        return Ok(resolved::unresolved(vec![diagnostic(
            input.environment_id,
            input.repo,
            "NORMALIZE_FLOW_MANIFEST_UNSUPPORTED",
            "resources.compose",
            "flow-mapping",
        )]));
    }
    Ok(resolved::decide(
        input.repo,
        input.environment_id,
        input.compose,
        input.runtime_manifest,
        &input.model.value,
        input.candidates,
    ))
}

#[allow(clippy::too_many_arguments)]
fn render_decision(
    repo: &Path,
    runtime_manifest: &RuntimeManifest,
    manifest_path: &Path,
    compose: &ComposePlan,
    compose_indices: &[usize],
    inspected: &[edit::InspectedCompose],
    decision: &resolved::Decision,
    files: &mut [PlannedFile],
) -> Result<(), RuntimeEnvError> {
    if !decision.diagnostics.is_empty() {
        return Ok(());
    }
    render_approved(files, compose_indices, inspected, &decision.approved);
    manifest::render(
        files,
        manifest_path,
        runtime_manifest,
        repo,
        &compose.files,
        &decision.exports,
    )
}

fn finish(
    repo: &Path,
    environment_id: &str,
    compose: ComposePlan,
    model: resolved::ResolvedModel,
    files: Vec<PlannedFile>,
    edits: Vec<NormalizationEdit>,
    diagnostics: Vec<IsolationDiagnostic>,
) -> Result<NormalizationPlan, RuntimeEnvError> {
    let fingerprint = fingerprint::digest(
        &files,
        &model.bytes,
        NORMALIZATION_SCHEMA_VERSION,
        COMPOSE_POLICY_VERSION,
    )?;
    let input_paths = relative_paths(repo, files.iter().map(|file| file.path.as_path()))?;
    let manifest_path = files
        .iter()
        .find(|file| !compose.files.contains(&file.path))
        .map(|file| file.path.as_path())
        .ok_or_else(|| RuntimeEnvError::new("runtime manifest was not planned"))?;
    let manifest_path = relative_path(repo, manifest_path)?;
    Ok(NormalizationPlan {
        schema_version: NORMALIZATION_SCHEMA_VERSION,
        fingerprint,
        input_paths,
        manifest_path,
        edits,
        remaining_diagnostics: diagnostics,
        repo: repo.to_path_buf(),
        files,
        environment_id: environment_id.to_string(),
        compose,
    })
}

fn relative_paths<'a>(
    repo: &Path,
    paths: impl Iterator<Item = &'a Path>,
) -> Result<Vec<std::path::PathBuf>, RuntimeEnvError> {
    let mut paths = paths
        .map(|path| relative_path(repo, path))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    Ok(paths)
}

fn relative_path(repo: &Path, path: &Path) -> Result<std::path::PathBuf, RuntimeEnvError> {
    path.strip_prefix(repo).map(Path::to_path_buf).map_err(|_| {
        RuntimeEnvError::new(format!(
            "normalization input is outside repository: {}",
            path.display()
        ))
    })
}

fn render_approved(
    files: &mut [PlannedFile],
    compose_indices: &[usize],
    inspected: &[edit::InspectedCompose],
    approved: &HashSet<(usize, usize)>,
) {
    for (source_index, file_index) in compose_indices.iter().enumerate() {
        let local = approved
            .iter()
            .filter(|(file, _)| *file == source_index)
            .map(|(_, index)| *index)
            .collect::<HashSet<_>>();
        files[*file_index].rendered = inspected[source_index].render(&local);
    }
}
