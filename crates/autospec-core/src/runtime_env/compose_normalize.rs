use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{
    ComposeExport, ComposePlan, EnvironmentIdentity, IsolationDiagnostic, RuntimeEnvError,
};

mod edit;
mod fingerprint;
mod manifest;
mod plan;
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
    repo: PathBuf,
    path: PathBuf,
    original: Vec<u8>,
    rendered: Vec<u8>,
    identity: Option<fingerprint::FileIdentity>,
    parent_existed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NormalizationPlan {
    pub schema_version: u32,
    pub fingerprint: String,
    pub input_paths: Vec<PathBuf>,
    pub manifest_path: PathBuf,
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
        let manifest = manifest::load_or_default(&repo)?;
        let identity = EnvironmentIdentity::resolve(&repo, "local", None)?;
        let resources = super::resource_plan::for_repo_allow_empty(&repo, &identity)?;
        let compose = resources.compose.ok_or_else(|| {
            RuntimeEnvError::new("NORMALIZE_COMPOSE_NOT_FOUND: no Compose file was detected")
        })?;
        let model = resolved::load(&repo, &compose)?;
        plan::build(&repo, manifest, compose, &identity.environment_id, model)
    }

    pub fn apply(plan: &NormalizationPlan, expected: &str) -> Result<(), RuntimeEnvError> {
        transaction::apply(plan, expected)
    }

    pub fn verify(plan: &NormalizationPlan) -> Result<(), RuntimeEnvError> {
        transaction::verify(plan)
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

#[cfg(test)]
mod transaction_tests {
    use std::collections::HashSet;
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
    fn unchanged_input_race_performs_zero_autospec_renames() {
        let mut fixture = TransactionFixture::new(2);
        fixture.files[1].rendered = fixture.files[1].original.clone();

        let error = commit_files(
            &fixture.files,
            &Faults {
                mutate_before_recheck: Some(1),
                ..Faults::default()
            },
        )
        .expect_err("an unchanged fingerprint input mutation must abort commit");

        transaction::assert_error(&error.to_string(), "NORMALIZE_STALE_SOURCE");
        assert_eq!(std::fs::read(&fixture.files[0].path).unwrap(), b"old-0");
        assert_eq!(
            std::fs::read(&fixture.files[1].path).unwrap(),
            b"external mutation"
        );
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

    #[test]
    fn restore_rename_failure_cleans_staged_bytes_and_continues_rollback() {
        let fixture = TransactionFixture::new(3);
        let renamed = commit_files(&fixture.files, &Faults::default()).unwrap();

        let error = rollback_result::<()>(
            RuntimeEnvError::new("NORMALIZE_POLICY_FAILED: injected verification failure"),
            &renamed,
            &Faults {
                fail_restore_rename_at: HashSet::from([0]),
                ..Faults::default()
            },
        )
        .unwrap_err()
        .to_string();

        transaction::assert_error(&error, "NORMALIZE_POLICY_FAILED");
        transaction::assert_error(&error, "rollback[0]");
        assert_eq!(std::fs::read(&fixture.files[0].path).unwrap(), b"new-0");
        assert_eq!(std::fs::read(&fixture.files[1].path).unwrap(), b"old-1");
        assert_eq!(std::fs::read(&fixture.files[2].path).unwrap(), b"old-2");
        fixture.assert_no_temporaries();
    }

    #[test]
    fn absent_destination_stage_failure_removes_created_parent() {
        let fixture = TransactionFixture::new(0);
        let path = fixture.root.join(".autospec/runtime.yml");
        let file = PlannedFile {
            repo: fixture.root.clone(),
            path,
            original: Vec::new(),
            rendered: b"version: 2\n".to_vec(),
            identity: None,
            parent_existed: false,
        };

        commit_files(
            &[file],
            &Faults {
                fail_stage_at: Some(0),
                ..Faults::default()
            },
        )
        .expect_err("stage failure must abort commit");

        assert!(!fixture.root.join(".autospec").exists());
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
            let root = std::fs::canonicalize(root).unwrap();
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
            identity: Some(fingerprint::file_identity(&path).unwrap()),
            parent_existed: true,
            repo: root.to_path_buf(),
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
