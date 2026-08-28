use super::{field_repository, repository_name_references, Discovery};
use autospec_core::managed_project::RelationshipKind;
use std::path::{Path, PathBuf};

pub(super) fn scan(root: &Path) -> Result<Vec<Discovery>, std::io::Error> {
    let mut discoveries = Vec::new();
    for path in managed_issue_files(root)? {
        let source_issue = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.parse::<u64>().ok())
            .filter(|number| *number > 0);
        let source = std::fs::read_to_string(&path)?;
        let location = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        discoveries.extend(scan_source(&source, source_issue, &location));
    }
    Ok(discoveries)
}

pub(super) fn scan_source(
    source: &str,
    source_issue: Option<u64>,
    location: &str,
) -> Vec<Discovery> {
    let mut discoveries = Vec::new();
    for line in managed_relationship_lines(source) {
        let lower = line.to_ascii_lowercase();
        let (evidence, kind, value) = if let Some(value) = prefixed_value(line, "Source spec:") {
            ("source-spec", RelationshipKind::Implements, value)
        } else if let Some(value) = prefixed_value(line, "Tracker:") {
            ("tracker", RelationshipKind::Tracks, value)
        } else if let Some(value) = prefixed_value(line, "Depends on") {
            ("issue-reference", RelationshipKind::DependsOn, value)
        } else if let Some(value) = prefixed_value(line, "Blocks") {
            ("issue-reference", RelationshipKind::Blocks, value)
        } else {
            ("name-similarity", RelationshipKind::DependsOn, line)
        };
        if evidence != "name-similarity" {
            let target = canonical_target(value);
            let repository = target
                .as_deref()
                .and_then(field_repository)
                .or_else(|| field_repository(value))
                .map(str::to_owned);
            if let Some(repository) = repository {
                let target = target.unwrap_or_else(|| repository.clone());
                discoveries.push(Discovery::typed_repository(
                    repository,
                    target,
                    source_issue,
                    kind,
                    evidence,
                    location.to_owned(),
                ));
            }
            continue;
        }
        if lower.contains("repository") {
            discoveries.extend(
                repository_name_references(value)
                    .into_iter()
                    .map(|name| Discovery::proposed(name, location.to_owned())),
            );
        }
    }
    discoveries
}

fn canonical_target(value: &str) -> Option<String> {
    let token = value
        .split_whitespace()
        .find(|token| token.to_ascii_lowercase().contains("github.com/") || token.contains('#'))?
        .trim_matches(|character: char| matches!(character, '<' | '>' | '(' | ')' | ',' | '.'));
    if let Some(reference) = token
        .strip_prefix("https://")
        .or_else(|| token.strip_prefix("HTTPS://"))
    {
        let mut parts = reference.split('/');
        if !parts.next()?.eq_ignore_ascii_case("github.com") {
            return None;
        }
        let owner = parts.next()?.to_ascii_lowercase();
        let repo = parts.next()?.trim_end_matches(".git").to_ascii_lowercase();
        let target_kind = parts.next()?.to_ascii_lowercase();
        if !matches!(target_kind.as_str(), "issues" | "pull") {
            return None;
        }
        let number = parts
            .next()?
            .split(['?', '#', '/'])
            .next()?
            .parse::<u64>()
            .ok()?;
        if owner.is_empty() || repo.is_empty() || number == 0 {
            return None;
        }
        return Some(format!(
            "https://github.com/{owner}/{repo}/{target_kind}/{number}"
        ));
    }
    let (repository, number) = token.split_once('#')?;
    let repository = field_repository(repository)?;
    let number = number.parse::<u64>().ok()?;
    (number > 0).then(|| format!("https://github.com/{repository}/issues/{number}"))
}

fn managed_issue_files(root: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    let directory = root.join(".autospec/issues");
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Ok(Vec::new());
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn managed_relationship_lines(source: &str) -> impl Iterator<Item = &str> {
    let mut managed = false;
    source.lines().filter(move |line| {
        if line.starts_with("## ") {
            managed = line.to_ascii_lowercase().contains("autospec");
            return false;
        }
        managed && !line.trim().is_empty()
    })
}

fn prefixed_value<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let trimmed = line.trim();
    if trimmed.len() < prefix.len() || !trimmed[..prefix.len()].eq_ignore_ascii_case(prefix) {
        return None;
    }
    Some(trimmed[prefix.len()..].trim())
}
