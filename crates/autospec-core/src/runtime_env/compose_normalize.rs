use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::Serialize;
use sha2::{Digest, Sha256};
use yaml_edit::Document;

use super::{
    ComposeExport, EnvironmentIdentity, IsolationDiagnostic, RuntimeEnvError, RuntimeManifest,
};

mod edit;
mod transaction;

const NORMALIZATION_SCHEMA_VERSION: u32 = 1;
const COMPOSE_POLICY_VERSION: u32 = 1;

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
}

impl NormalizationPlan {
    pub fn rendered_file(&self, path: &Path) -> Option<String> {
        self.rendered_bytes(path)
            .and_then(|bytes| String::from_utf8(bytes).ok())
    }

    pub fn rendered_bytes(&self, path: &Path) -> Option<Vec<u8>> {
        let canonical = std::fs::canonicalize(path).ok()?;
        self.files
            .iter()
            .find(|file| file.path == canonical)
            .map(|file| file.rendered.clone())
    }

    pub fn changed_files(&self) -> usize {
        self.files
            .iter()
            .filter(|file| file.original != file.rendered)
            .count()
    }

    pub fn to_json(&self) -> Result<String, RuntimeEnvError> {
        serde_json::to_string(self).map_err(|error| {
            RuntimeEnvError::new(format!("could not encode normalization plan: {error}"))
        })
    }
}

pub struct ComposeNormalizer;

impl ComposeNormalizer {
    pub fn plan(repo: &Path) -> Result<NormalizationPlan, RuntimeEnvError> {
        let repo = std::fs::canonicalize(repo).map_err(|error| {
            RuntimeEnvError::new(format!("could not canonicalize repository: {error}"))
        })?;
        let manifest = RuntimeManifest::read_from_repo(&repo)?;
        let identity = EnvironmentIdentity::resolve(&repo, "local", None)?;
        let resource_plan = RuntimeManifest::resource_plan_for_repo(&repo, &identity)?;
        let compose = resource_plan.compose.ok_or_else(|| {
            RuntimeEnvError::new("NORMALIZE_COMPOSE_NOT_FOUND: no Compose file was detected")
        })?;
        Self::plan_inputs(
            &repo,
            &compose.files,
            manifest.path(),
            &identity.environment_id,
        )
    }

    #[rustfmt::skip]
    pub fn plan_inputs(repo: &Path, compose_files: &[PathBuf], manifest_path: &Path, environment_id: &str) -> Result<NormalizationPlan, RuntimeEnvError> {
        let manifest_path = std::fs::canonicalize(manifest_path).map_err(|error| {
            RuntimeEnvError::new(format!("could not canonicalize runtime manifest: {error}"))
        })?;
        let manifest = RuntimeManifest::read_from_repo(repo)?;
        let mut files = read_inputs(repo, compose_files, &manifest_path)?;
        let mut edits = Vec::new();
        let mut exports = Vec::new();
        let mut diagnostics = Vec::new();
        for file in files.iter_mut().filter(|file| file.path != manifest_path) {
            let edited = edit::edit_compose(&file.original, environment_id, repo)?;
            file.rendered = edited.bytes;
            edits.extend(edited.edits);
            exports.extend(edited.exports);
            diagnostics.extend(edited.diagnostics);
        }
        diagnostics.sort_by(|left, right| {
            left.resource
                .cmp(&right.resource)
                .then_with(|| left.code.cmp(&right.code))
        });
        if diagnostics.is_empty() {
            let manifest_file = files.iter_mut().find(|file| file.path == manifest_path)
                .ok_or_else(|| RuntimeEnvError::new("runtime manifest was not planned"))?;
            let rendered = render_manifest(&manifest_file.original, &manifest, repo, compose_files, &exports)?;
            if rendered != manifest_file.original {
                let resources = RuntimeResourcesReport { compose_files: compose_files.to_vec(), exports };
                edits.push(NormalizationEdit::UpsertRuntimeResources { resources });
                manifest_file.rendered = rendered;
            }
        } else {
            for file in &mut files {
                file.rendered.clone_from(&file.original);
            }
            edits.clear();
        }
        let fingerprint = fingerprint(&files)?;
        Ok(NormalizationPlan { schema_version: NORMALIZATION_SCHEMA_VERSION, fingerprint, edits,
            remaining_diagnostics: diagnostics, repo: repo.to_path_buf(), files,
            environment_id: environment_id.to_string() })
    }

    pub fn apply(plan: &NormalizationPlan, expected: &str) -> Result<(), RuntimeEnvError> {
        transaction::apply(plan, expected)
    }

    pub fn verify(plan: &NormalizationPlan) -> Result<(), RuntimeEnvError> {
        transaction::verify(plan)
    }
}

fn read_inputs(
    repo: &Path,
    compose_files: &[PathBuf],
    manifest_path: &Path,
) -> Result<Vec<PlannedFile>, RuntimeEnvError> {
    let mut paths = compose_files.to_vec();
    paths.push(manifest_path.to_path_buf());
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .map(|path| {
            let canonical = std::fs::canonicalize(&path).map_err(|error| {
                RuntimeEnvError::new(format!(
                    "could not canonicalize {}: {error}",
                    path.display()
                ))
            })?;
            if !canonical.starts_with(repo) {
                return Err(RuntimeEnvError::new(format!(
                    "normalization input is outside repository: {}",
                    path.display()
                )));
            }
            let original = std::fs::read(&canonical).map_err(|error| {
                RuntimeEnvError::new(format!("could not read {}: {error}", canonical.display()))
            })?;
            Ok(PlannedFile {
                path: canonical,
                rendered: original.clone(),
                original,
            })
        })
        .collect()
}

fn fingerprint(files: &[PlannedFile]) -> Result<String, RuntimeEnvError> {
    let mut digest = Sha256::new();
    digest.update(NORMALIZATION_SCHEMA_VERSION.to_be_bytes());
    digest.update(COMPOSE_POLICY_VERSION.to_be_bytes());
    for file in files {
        let path = file.path.to_str().ok_or_else(|| {
            RuntimeEnvError::new(format!(
                "normalization path is not UTF-8: {}",
                file.path.display()
            ))
        })?;
        digest.update(path.len().to_be_bytes());
        digest.update(path.as_bytes());
        digest.update(file.original.len().to_be_bytes());
        digest.update(&file.original);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn render_manifest(
    source: &[u8],
    manifest: &RuntimeManifest,
    repo: &Path,
    compose_files: &[PathBuf],
    new_exports: &[ComposeExport],
) -> Result<Vec<u8>, RuntimeEnvError> {
    let text = std::str::from_utf8(source)
        .map_err(|_| RuntimeEnvError::new("runtime manifest must be UTF-8"))?;
    let document = Document::from_str(text).map_err(|error| {
        RuntimeEnvError::new(format!("could not parse runtime manifest YAML: {error}"))
    })?;
    let root = document
        .as_mapping()
        .ok_or_else(|| RuntimeEnvError::new("runtime manifest root must be a mapping"))?;
    if root.get("version").and_then(|value| value.to_i64()) != Some(2) {
        return Err(RuntimeEnvError::new(
            "NORMALIZE_MANIFEST_V2_REQUIRED: Compose normalization requires runtime manifest version 2",
        ));
    }
    let files = compose_files
        .iter()
        .map(|file| {
            file.strip_prefix(repo)
                .unwrap_or(file)
                .display()
                .to_string()
        })
        .collect::<Vec<_>>();
    let mut exports = manifest.resources().compose.exports.clone();
    for export in new_exports {
        if !exports.contains(export) {
            exports.push(export.clone())
        }
    }
    let files = serde_json::to_string(&files).map_err(json_error)?;
    let exports = serde_json::to_string(&exports.iter().map(export_value).collect::<Vec<_>>())
        .map_err(json_error)?;
    render_manifest_fields(text, &root, &files, &exports)
}

fn export_value(export: &ComposeExport) -> serde_json::Value {
    let protocol = match export.protocol {
        super::ExportProtocol::Http => "http",
        super::ExportProtocol::Https => "https",
        super::ExportProtocol::Tcp => "tcp",
        super::ExportProtocol::Udp => "udp",
    };
    let value = match export.value {
        super::ExportValue::Url => "url",
        super::ExportValue::Port => "port",
        super::ExportValue::HostPort => "host-port",
    };
    serde_json::json!({
        "service": export.service, "target": export.target, "protocol": protocol,
        "env": export.env, "value": value,
    })
}

fn render_manifest_fields(
    source: &str,
    root: &yaml_edit::Mapping,
    files: &str,
    exports: &str,
) -> Result<Vec<u8>, RuntimeEnvError> {
    let Some(resources) = root.get_mapping("resources") else {
        let block = format!("resources:\n  compose:\n    files: {files}\n    exports: {exports}\n");
        return Ok(insert_mapping_field(source, root, &block));
    };
    let Some(compose) = resources.get_mapping("compose") else {
        let block = format!("compose:\n  files: {files}\n  exports: {exports}\n");
        return Ok(insert_mapping_field(source, &resources, &block));
    };
    let mut edits = Vec::new();
    replace_or_insert_sequence(source, &compose, "files", files, &mut edits)?;
    replace_or_insert_sequence(source, &compose, "exports", exports, &mut edits)?;
    Ok(apply_byte_edits(source, edits))
}

#[rustfmt::skip]
fn replace_or_insert_sequence(source: &str, mapping: &yaml_edit::Mapping, key: &str,
    encoded: &str, edits: &mut Vec<(usize, usize, String)>) -> Result<(), RuntimeEnvError> {
    if let Some(node) = mapping.get(key) {
        let sequence = node.as_sequence().ok_or_else(||
            RuntimeEnvError::new(format!("runtime Compose {key} must be a list")))?;
        let range = sequence.byte_range();
        edits.push((range.start as usize, range.end as usize, encoded.to_string()));
    } else {
        let range = mapping.byte_range();
        let indent = line_indent(source, range.start as usize);
        let prefix = if source.as_bytes().get(range.end as usize - 1) == Some(&b'\n') { "" } else { "\n" };
        edits.push((range.end as usize, range.end as usize,
            format!("{prefix}{}{key}: {encoded}\n", " ".repeat(indent))));
    }
    Ok(())
}

#[rustfmt::skip]
fn insert_mapping_field(source: &str, mapping: &yaml_edit::Mapping, block: &str) -> Vec<u8> {
    let range = mapping.byte_range();
    let indent = line_indent(source, range.start as usize);
    let prefix = if source.as_bytes().get(range.end as usize - 1) == Some(&b'\n') { "" } else { "\n" };
    let block = block.lines().map(|line| format!("{}{line}\n", " ".repeat(indent))).collect::<String>();
    apply_byte_edits(source,
        vec![(range.end as usize, range.end as usize, format!("{prefix}{block}"))])
}

fn line_indent(source: &str, offset: usize) -> usize {
    let start = source[..offset].rfind('\n').map_or(0, |index| index + 1);
    source[start..offset]
        .bytes()
        .take_while(|byte| *byte == b' ')
        .count()
}

fn apply_byte_edits(source: &str, mut edits: Vec<(usize, usize, String)>) -> Vec<u8> {
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.0));
    let mut output = source.to_string();
    for (start, end, replacement) in edits {
        output.replace_range(start..end, &replacement)
    }
    output.into_bytes()
}

fn json_error(error: serde_json::Error) -> RuntimeEnvError {
    RuntimeEnvError::new(format!("could not encode runtime resources: {error}"))
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
