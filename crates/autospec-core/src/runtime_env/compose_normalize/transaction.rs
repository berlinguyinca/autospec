use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::{fingerprint, resolved, ComposeNormalizer, PlannedFile, NORMALIZATION_SCHEMA_VERSION};
use crate::runtime_env::{
    ComposePolicy, EnvironmentIdentity, RuntimeEnvError, RuntimeManifest, COMPOSE_POLICY_VERSION,
};
use getrandom::fill;

#[derive(Default)]
pub(super) struct Faults {
    pub(super) fail_stage_at: Option<usize>,
    pub(super) mutate_before_recheck: Option<usize>,
    pub(super) fail_rename_at: Option<usize>,
    pub(super) fail_restore_at: HashSet<usize>,
    pub(super) fail_restore_rename_at: HashSet<usize>,
    pub(super) fail_parent_sync: bool,
}

struct Staged<'a> {
    file: &'a PlannedFile,
    temporary: PathBuf,
}

pub(super) fn apply(
    plan: &super::NormalizationPlan,
    expected: &str,
) -> Result<(), RuntimeEnvError> {
    validate_apply(plan, expected)?;
    let renamed = commit_files(&plan.files, &Faults::default())?;
    if let Err(primary) = ComposeNormalizer::verify(plan) {
        return rollback_result(primary, &renamed, &Faults::default());
    }
    Ok(())
}

fn validate_apply(plan: &super::NormalizationPlan, expected: &str) -> Result<(), RuntimeEnvError> {
    if expected != plan.fingerprint {
        return Err(RuntimeEnvError::new(format!(
            "NORMALIZE_STALE_FINGERPRINT: expected {}, received {expected}",
            plan.fingerprint
        )));
    }
    if !plan.remaining_diagnostics.is_empty() {
        return Err(RuntimeEnvError::new(
            "NORMALIZE_UNRESOLVED_DIAGNOSTICS: unsafe Compose input remains unchanged",
        ));
    }
    let current = fingerprint::read_current(&plan.files)?;
    let model = resolved::load(&plan.repo, &plan.compose)?;
    let current_fingerprint = fingerprint::digest(
        &current,
        &model.bytes,
        NORMALIZATION_SCHEMA_VERSION,
        COMPOSE_POLICY_VERSION,
    )?;
    if current_fingerprint != plan.fingerprint {
        return Err(RuntimeEnvError::new(
            "NORMALIZE_STALE_SOURCE: source bytes, identities, or resolved model changed after planning",
        ));
    }
    Ok(())
}

pub(super) fn verify(plan: &super::NormalizationPlan) -> Result<(), RuntimeEnvError> {
    let identity = EnvironmentIdentity::resolve(&plan.repo, "local", None)?;
    let resources = RuntimeManifest::resource_plan_for_repo(&plan.repo, &identity)?;
    let compose = resources.compose.ok_or_else(|| {
        RuntimeEnvError::new("NORMALIZE_VERIFY_NO_COMPOSE: normalized Compose plan is absent")
    })?;
    let model = resolved::load(&plan.repo, &compose)?;
    let diagnostics = ComposePolicy::evaluate_in_context(
        &model.value,
        &compose,
        &plan.environment_id,
        &plan.repo,
    );
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(RuntimeEnvError::new(format!(
            "NORMALIZE_POLICY_FAILED: {}",
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code.as_str())
                .collect::<Vec<_>>()
                .join(",")
        )))
    }
}

pub(super) fn commit_files<'a>(
    files: &'a [PlannedFile],
    faults: &Faults,
) -> Result<Vec<&'a PlannedFile>, RuntimeEnvError> {
    let changed = files
        .iter()
        .filter(|file| file.original != file.rendered)
        .collect::<Vec<_>>();
    let staged = stage_all(&changed, faults)?;
    if let Some(index) = faults.mutate_before_recheck {
        std::fs::write(&changed[index].path, b"external mutation").map_err(|error| {
            RuntimeEnvError::new(format!("could not inject source mutation: {error}"))
        })?;
    }
    if let Err(error) = fingerprint::recheck_all(&changed) {
        cleanup_temporaries(&staged);
        return Err(error);
    }
    rename_staged(staged, faults)
}

fn stage_all<'a>(
    files: &[&'a PlannedFile],
    faults: &Faults,
) -> Result<Vec<Staged<'a>>, RuntimeEnvError> {
    let mut staged = Vec::new();
    for (index, file) in files.iter().enumerate() {
        match stage(file) {
            Ok(item) if faults.fail_stage_at != Some(index) => staged.push(item),
            Ok(item) => {
                staged.push(item);
                cleanup_temporaries(&staged);
                return Err(RuntimeEnvError::new(
                    "NORMALIZE_STAGE_FAILED: injected failure",
                ));
            }
            Err(error) => {
                cleanup_temporaries(&staged);
                return Err(error);
            }
        }
    }
    Ok(staged)
}

fn stage<'a>(file: &'a PlannedFile) -> Result<Staged<'a>, RuntimeEnvError> {
    let parent = file
        .path
        .parent()
        .ok_or_else(|| RuntimeEnvError::new("normalization path has no parent"))?;
    for _ in 0..32 {
        let temporary = parent.join(temporary_name(&file.path)?);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(handle) => return finish_stage(file, temporary, handle),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(RuntimeEnvError::new(format!(
                    "NORMALIZE_STAGE_FAILED: {}: {error}",
                    temporary.display()
                )))
            }
        }
    }
    Err(RuntimeEnvError::new(
        "NORMALIZE_STAGE_FAILED: temporary name exhaustion",
    ))
}

fn finish_stage<'a>(
    file: &'a PlannedFile,
    temporary: PathBuf,
    mut handle: File,
) -> Result<Staged<'a>, RuntimeEnvError> {
    if let Err(error) = handle
        .write_all(&file.rendered)
        .and_then(|()| handle.sync_all())
    {
        drop(handle);
        let _ = std::fs::remove_file(&temporary);
        return Err(RuntimeEnvError::new(format!(
            "NORMALIZE_STAGE_FAILED: {}: {error}",
            temporary.display()
        )));
    }
    Ok(Staged { file, temporary })
}

fn rename_staged<'a>(
    staged: Vec<Staged<'a>>,
    faults: &Faults,
) -> Result<Vec<&'a PlannedFile>, RuntimeEnvError> {
    let mut renamed = Vec::new();
    for (index, item) in staged.iter().enumerate() {
        let result = if faults.fail_rename_at == Some(index) {
            Err(std::io::Error::other("injected rename failure"))
        } else {
            std::fs::rename(&item.temporary, &item.file.path)
        };
        if let Err(error) = result {
            cleanup_temporaries(&staged[index..]);
            let primary = RuntimeEnvError::new(format!(
                "NORMALIZE_RENAME_FAILED: {}: {error}",
                item.file.path.display()
            ));
            return rollback_result(primary, &renamed, faults);
        }
        renamed.push(item.file);
    }
    if let Err(error) = sync_parents(&renamed, faults) {
        return rollback_result(error, &renamed, faults);
    }
    Ok(renamed)
}

pub(super) fn rollback_result<T>(
    primary: RuntimeEnvError,
    renamed: &[&PlannedFile],
    faults: &Faults,
) -> Result<T, RuntimeEnvError> {
    let errors = restore_all(renamed, faults);
    if errors.is_empty() {
        Err(primary)
    } else {
        Err(RuntimeEnvError::new(format!(
            "{}; NORMALIZE_ROLLBACK_FAILED: {}",
            primary,
            errors.join("; ")
        )))
    }
}

fn restore_all(files: &[&PlannedFile], faults: &Faults) -> Vec<String> {
    let mut errors = Vec::new();
    for (index, file) in files.iter().enumerate() {
        if faults.fail_restore_at.contains(&index) {
            errors.push(format!(
                "rollback[{index}] {}: injected restore failure",
                file.path.display()
            ));
            continue;
        }
        if let Err(error) = restore(file, faults.fail_restore_rename_at.contains(&index)) {
            errors.push(format!(
                "rollback[{index}] {}: {error}",
                file.path.display()
            ));
        }
    }
    errors
}

fn restore(file: &PlannedFile, fail_rename: bool) -> Result<(), RuntimeEnvError> {
    let staged = stage_original(file)?;
    let result = if fail_rename {
        Err(std::io::Error::other("injected restore rename failure"))
    } else {
        std::fs::rename(&staged, &file.path)
    };
    if let Err(error) = result {
        let cleanup = std::fs::remove_file(&staged)
            .err()
            .map(|cleanup| format!("; could not clean staged rollback: {cleanup}"))
            .unwrap_or_default();
        return Err(RuntimeEnvError::new(format!(
            "could not restore destination: {error}{cleanup}"
        )));
    }
    sync_parent(&file.path)
}

fn stage_original(file: &PlannedFile) -> Result<PathBuf, RuntimeEnvError> {
    let rollback = PlannedFile {
        path: file.path.clone(),
        original: file.rendered.clone(),
        rendered: file.original.clone(),
        identity: file.identity.clone(),
    };
    stage(&rollback).map(|staged| staged.temporary)
}

fn sync_parents(files: &[&PlannedFile], faults: &Faults) -> Result<(), RuntimeEnvError> {
    if faults.fail_parent_sync {
        return Err(RuntimeEnvError::new(
            "NORMALIZE_PARENT_SYNC_FAILED: injected failure",
        ));
    }
    let parents = files
        .iter()
        .filter_map(|file| file.path.parent())
        .collect::<HashSet<_>>();
    for parent in parents {
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| {
                RuntimeEnvError::new(format!(
                    "NORMALIZE_PARENT_SYNC_FAILED: {}: {error}",
                    parent.display()
                ))
            })?;
    }
    Ok(())
}

fn sync_parent(path: &Path) -> Result<(), RuntimeEnvError> {
    let parent = path
        .parent()
        .ok_or_else(|| RuntimeEnvError::new("normalization path has no parent"))?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| RuntimeEnvError::new(format!("could not sync rollback parent: {error}")))
}

fn cleanup_temporaries(staged: &[Staged<'_>]) {
    for item in staged {
        let _ = std::fs::remove_file(&item.temporary);
    }
}

fn temporary_name(path: &Path) -> Result<String, RuntimeEnvError> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let mut random = [0_u8; 32];
    fill(&mut random).map_err(|error| {
        RuntimeEnvError::new(format!(
            "NORMALIZE_STAGE_FAILED: could not generate nonce: {error}"
        ))
    })?;
    let mut token = String::with_capacity(64);
    for byte in random {
        write!(&mut token, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(format!(".{name}.autospec-{token}.tmp"))
}

#[cfg(test)]
pub(super) fn assert_error(message: &str, expected: &str) {
    assert!(message.contains(expected), "{message}");
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::temporary_name;

    #[test]
    fn temporary_names_use_fixed_width_cryptographic_tokens() {
        let prefix = ".compose.yaml.autospec-";
        let suffix = ".tmp";
        let names = (0..32)
            .map(|_| temporary_name(Path::new("compose.yaml")).unwrap())
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(names.len(), 32);
        for name in names {
            let token = name
                .strip_prefix(prefix)
                .and_then(|value| value.strip_suffix(suffix))
                .unwrap();
            assert_eq!(token.len(), 64);
            assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
    }
}
