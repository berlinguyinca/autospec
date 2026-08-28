use super::ManagedProjectError;
use autospec_core::managed_project::ManagedProjectPolicy;
use std::path::Path;
use std::process::Command;

pub(super) enum Admission {
    Admitted(String),
    OutOfBound(String),
    Inaccessible(String),
}

pub fn normalize_github_repository(value: &str) -> Option<String> {
    let value = value
        .trim()
        .trim_matches(|character: char| "\"'`()[]{}<>,;".contains(character));
    let path = [
        "git@github.com:",
        "ssh://git@github.com/",
        "https://github.com/",
        "http://github.com/",
        "github:",
        "github.com/",
    ]
    .iter()
    .find_map(|prefix| value.strip_prefix(prefix))
    .or_else(|| (value.matches('/').count() == 1).then_some(value))?;
    let mut components = path.split('/');
    let owner = clean_component(components.next()?);
    let repository = clean_component(components.next()?);
    if !valid_component(owner, false) || !valid_component(repository, true) {
        return None;
    }
    Some(format!("{owner}/{repository}").to_ascii_lowercase())
}

pub(super) fn repository_admission(value: &str, policy: &ManagedProjectPolicy) -> Admission {
    let Some(repository) = normalize_github_repository(value) else {
        return Admission::Inaccessible(value.to_owned());
    };
    if allowed(&repository, policy) {
        Admission::Admitted(repository)
    } else {
        Admission::OutOfBound(repository)
    }
}

pub(super) fn workspace_admission(path: &Path, policy: &ManagedProjectPolicy) -> Admission {
    match workspace_repository(path) {
        Ok(repository) if allowed(&repository, policy) => Admission::Admitted(repository),
        Ok(repository) => Admission::OutOfBound(repository),
        Err(_) => Admission::Inaccessible(path.display().to_string()),
    }
}

pub(super) fn allowed(repository: &str, policy: &ManagedProjectPolicy) -> bool {
    let Some((owner, _)) = repository.split_once('/') else {
        return false;
    };
    owner.eq_ignore_ascii_case(policy.owner.trim())
        && policy.repo_allowlist.iter().any(|pattern| {
            let pattern = pattern.to_ascii_lowercase();
            match pattern.split_once('*') {
                Some((prefix, suffix)) => {
                    repository.starts_with(prefix) && repository.ends_with(suffix)
                }
                None => repository == pattern,
            }
        })
}

pub(super) fn normalize_repository(value: &str) -> String {
    normalize_github_repository(value).unwrap_or_else(|| value.trim().to_ascii_lowercase())
}

pub(crate) fn field_repository(value: &str) -> Option<&str> {
    value
        .split(|character: char| {
            !(character.is_ascii_alphanumeric()
                || matches!(character, '-' | '_' | '.' | '/' | ':' | '@' | '#' | '?'))
        })
        .find(|token| normalize_github_repository(token).is_some())
}

fn workspace_repository(path: &Path) -> Result<String, ManagedProjectError> {
    if !path.join(".git").exists() {
        return Err(ManagedProjectError::new(
            "workspace has no repository-local Git metadata",
        ));
    }
    let output = Command::new("git")
        .args([
            "-C",
            path.to_string_lossy().as_ref(),
            "remote",
            "get-url",
            "origin",
        ])
        .output()
        .map_err(|error| {
            ManagedProjectError::new(format!("cannot inspect workspace remote: {error}"))
        })?;
    if !output.status.success() {
        return Err(ManagedProjectError::new("workspace has no verified origin"));
    }
    normalize_github_repository(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| ManagedProjectError::new("workspace origin is not a GitHub repository"))
}

fn clean_component(value: &str) -> &str {
    value
        .split(['#', '?'])
        .next()
        .unwrap_or_default()
        .trim_end_matches(".git")
        .trim_matches(|character: char| "\"'`()[]{}<>,;".contains(character))
}

fn valid_component(value: &str, repository: bool) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || character == '-'
                || (repository && matches!(character, '_' | '.'))
        })
}
