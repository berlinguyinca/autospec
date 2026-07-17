use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{
    ComposeExport, ComposePlan, EnvironmentIdentity, IsolationDiagnostic, RuntimeEnvError,
    RuntimeManifest, COMPOSE_POLICY_VERSION,
};

mod edit;
mod fingerprint;
mod manifest;
mod resolved;
mod transaction;

const NORMALIZATION_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum ResourceKind {
    Network,
    Volume,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum NormalizationEdit {
    RemovePublishedPort {
        service: String,
        index: usize,
        export: ComposeExport,
    },
    RemoveRedundantContainerName {
        service: String,
    },
    RemoveProjectScopedResourceName {
        kind: ResourceKind,
        logical_key: String,
    },
    UpsertRuntimeResources {
        resources: RuntimeResourcesReport,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RuntimeResourcesReport {
    pub compose_files: Vec<PathBuf>,
    pub exports: Vec<ComposeExport>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PlannedFile {
    path: PathBuf,
    original: Vec<u8>,
    rendered: Vec<u8>,
    identity: fingerprint::FileIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NormalizationPlan {
    pub schema_version: u32,
    pub fingerprint: String,
    pub edits: Vec<NormalizationEdit>,
    pub remaining_diagnostics: Vec<IsolationDiagnostic>,
    #[serde(skip)]
    repo: PathBuf,
    #[serde(skip)]
    files: Vec<PlannedFile>,
    #[serde(skip)]
    environment_id: String,
    #[serde(skip)]
    compose: ComposePlan,
}

pub struct ComposeNormalizer;

impl ComposeNormalizer {
    pub fn plan(repo: &Path) -> Result<NormalizationPlan, RuntimeEnvError> {
        let repo = std::fs::canonicalize(repo).map_err(|error| {
            RuntimeEnvError::new(format!("could not canonicalize repository: {error}"))
        })?;
        let manifest = RuntimeManifest::read_from_repo(&repo)?;
        let identity = EnvironmentIdentity::resolve(&repo, "local", None)?;
        let resources = RuntimeManifest::resource_plan_for_repo(&repo, &identity)?;
        let compose = resources.compose.ok_or_else(|| {
            RuntimeEnvError::new("NORMALIZE_COMPOSE_NOT_FOUND: no Compose file was detected")
        })?;
        let model = resolved::load(&repo, &compose)?;
        plan_resolved(&repo, manifest, compose, &identity.environment_id, model)
    }

    pub fn apply(plan: &NormalizationPlan, expected: &str) -> Result<(), RuntimeEnvError> {
        transaction::apply(plan, expected)
    }

    pub fn verify(plan: &NormalizationPlan) -> Result<(), RuntimeEnvError> {
        transaction::verify(plan)
    }
}

fn plan_resolved(
    repo: &Path,
    manifest: RuntimeManifest,
    compose: ComposePlan,
    environment_id: &str,
    model: resolved::ResolvedModel,
) -> Result<NormalizationPlan, RuntimeEnvError> {
    let manifest_path = std::fs::canonicalize(manifest.path()).map_err(|error| {
        RuntimeEnvError::new(format!("could not canonicalize runtime manifest: {error}"))
    })?;
    let mut files = fingerprint::read_inputs(repo, &compose.files, &manifest_path)?;
    let compose_indices = files
        .iter()
        .enumerate()
        .filter(|(_, file)| file.path != manifest_path)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let inspected = compose_indices
        .iter()
        .map(|index| edit::inspect(&files[*index].original))
        .collect::<Result<Vec<_>, _>>()?;
    let candidates = inspected
        .iter()
        .map(|source| source.candidates.clone())
        .collect::<Vec<_>>();
    let decision = resolved::decide(
        repo,
        environment_id,
        &compose,
        &manifest,
        &model.value,
        &candidates,
    );
    let mut reported_exports = manifest.resources().compose.exports.clone();
    for export in &decision.exports {
        if !reported_exports.contains(export) {
            reported_exports.push(export.clone());
        }
    }
    if decision.diagnostics.is_empty() {
        render_approved(&mut files, &compose_indices, &inspected, &decision.approved);
        manifest::render(
            &mut files,
            &manifest_path,
            &manifest,
            repo,
            &compose.files,
            &decision.exports,
        )?;
    }
    let mut edits = decision.edits;
    if files
        .iter()
        .any(|file| file.path == manifest_path && file.original != file.rendered)
    {
        edits.push(NormalizationEdit::UpsertRuntimeResources {
            resources: RuntimeResourcesReport {
                compose_files: compose.files.clone(),
                exports: reported_exports,
            },
        });
    }
    let fingerprint = fingerprint::digest(
        &files,
        &model.bytes,
        NORMALIZATION_SCHEMA_VERSION,
        COMPOSE_POLICY_VERSION,
    )?;
    Ok(NormalizationPlan {
        schema_version: NORMALIZATION_SCHEMA_VERSION,
        fingerprint,
        edits,
        remaining_diagnostics: decision.diagnostics,
        repo: repo.to_path_buf(),
        files,
        environment_id: environment_id.to_string(),
        compose,
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

pub(super) fn diagnostic(
    environment_id: &str,
    repo: &Path,
    code: &str,
    resource: &str,
    evidence: &str,
) -> IsolationDiagnostic {
    IsolationDiagnostic {
        schema_version: 1,
        code: code.to_string(),
        environment_id: environment_id.to_string(),
        resource: resource.to_string(),
        evidence: evidence.to_string(),
        recovery_command: format!(
            "autospec runtime env normalize-compose --repo {} --check",
            super::shell_quote(&repo.display().to_string())
        ),
    }
}

fn normalization_edit(
    candidate: &edit::Candidate,
    export: Option<ComposeExport>,
) -> NormalizationEdit {
    match &candidate.kind {
        edit::CandidateKind::Port { service, index, .. } => {
            NormalizationEdit::RemovePublishedPort {
                service: service.clone(),
                index: *index,
                export: export.expect("approved port candidate has an export"),
            }
        }
        edit::CandidateKind::ContainerName { service } => {
            NormalizationEdit::RemoveRedundantContainerName {
                service: service.clone(),
            }
        }
        edit::CandidateKind::ResourceName { kind, logical_key } => {
            NormalizationEdit::RemoveProjectScopedResourceName {
                kind: kind.clone(),
                logical_key: logical_key.clone(),
            }
        }
    }
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

fn escape_pointer(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod transaction_tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::transaction::{commit_files, rollback_result, Faults};
    use super::*;

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn stage_rename_and_parent_sync_faults_restore_every_destination() {
        for faults in [
            Faults {
                fail_stage_at: Some(1),
                ..Faults::default()
            },
            Faults {
                fail_rename_at: Some(1),
                ..Faults::default()
            },
            Faults {
                fail_parent_sync: true,
                ..Faults::default()
            },
        ] {
            let fixture = TransactionFixture::new(3);
            commit_files(&fixture.files, &faults).expect_err("fault must abort commit");
            fixture.assert_originals();
            fixture.assert_no_temporaries();
        }
    }

    #[test]
    fn stale_recheck_performs_zero_autospec_renames() {
        let fixture = TransactionFixture::new(2);
        let error = commit_files(
            &fixture.files,
            &Faults {
                mutate_before_recheck: Some(0),
                ..Faults::default()
            },
        )
        .expect_err("mutation must make the plan stale");
        transaction::assert_error(&error.to_string(), "NORMALIZE_STALE_SOURCE");
        assert_eq!(
            std::fs::read(&fixture.files[0].path).unwrap(),
            b"external mutation"
        );
        assert_eq!(std::fs::read(&fixture.files[1].path).unwrap(), b"old-1");
        fixture.assert_no_temporaries();
    }

    #[test]
    fn verification_and_restore_faults_preserve_primary_failure_evidence() {
        let fixture = TransactionFixture::new(3);
        let renamed = commit_files(&fixture.files, &Faults::default()).unwrap();
        let error: RuntimeEnvError = rollback_result::<()>(
            RuntimeEnvError::new("NORMALIZE_POLICY_FAILED: injected verification failure"),
            &renamed,
            &Faults::default(),
        )
        .unwrap_err();
        transaction::assert_error(&error.to_string(), "NORMALIZE_POLICY_FAILED");
        fixture.assert_originals();

        let fixture = TransactionFixture::new(3);
        let error = commit_files(
            &fixture.files,
            &Faults {
                fail_rename_at: Some(2),
                fail_restore_at: HashSet::from([0, 1]),
                ..Faults::default()
            },
        )
        .unwrap_err()
        .to_string();
        transaction::assert_error(&error, "NORMALIZE_RENAME_FAILED");
        transaction::assert_error(&error, "rollback[0]");
        transaction::assert_error(&error, "rollback[1]");
        assert_eq!(std::fs::read(&fixture.files[2].path).unwrap(), b"old-2");
    }

    struct TransactionFixture {
        root: PathBuf,
        files: Vec<PlannedFile>,
    }

    impl TransactionFixture {
        fn new(count: usize) -> Self {
            let root = std::env::temp_dir().join(format!(
                "autospec-normalize-transaction-{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            let files = (0..count).map(|index| planned_file(&root, index)).collect();
            Self { root, files }
        }

        fn assert_originals(&self) {
            for (index, file) in self.files.iter().enumerate() {
                assert_eq!(
                    std::fs::read(&file.path).unwrap(),
                    format!("old-{index}").as_bytes()
                );
            }
        }

        fn assert_no_temporaries(&self) {
            let names = std::fs::read_dir(&self.root)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            assert!(
                names.iter().all(|name| !name.ends_with(".tmp")),
                "{names:?}"
            );
        }
    }

    fn planned_file(root: &Path, index: usize) -> PlannedFile {
        let path = root.join(format!("file-{index}.yml"));
        let original = format!("old-{index}").into_bytes();
        std::fs::write(&path, &original).unwrap();
        PlannedFile {
            identity: fingerprint::file_identity(&path).unwrap(),
            path,
            original,
            rendered: format!("new-{index}").into_bytes(),
        }
    }

    impl Drop for TransactionFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}
