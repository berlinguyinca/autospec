use super::{field_repository, repository_name_references, Discovery};
use std::path::{Path, PathBuf};

pub(super) fn scan(root: &Path) -> Result<Vec<Discovery>, std::io::Error> {
    let mut discoveries = Vec::new();
    for path in managed_issue_files(root)? {
        let source = std::fs::read_to_string(&path)?;
        let location = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        for line in managed_relationship_lines(&source) {
            let lower = line.to_ascii_lowercase();
            let (evidence, value) = if let Some(value) = prefixed_value(line, "Source spec:") {
                ("source-spec", value)
            } else if let Some(value) = prefixed_value(line, "Tracker:") {
                ("tracker", value)
            } else if let Some(value) = prefixed_value(line, "Depends on") {
                ("issue-reference", value)
            } else if let Some(value) = prefixed_value(line, "Blocks") {
                ("issue-reference", value)
            } else {
                ("name-similarity", line)
            };
            if evidence != "name-similarity" {
                if let Some(repository) = field_repository(value) {
                    discoveries.push(Discovery::repository(
                        repository,
                        evidence,
                        location.clone(),
                    ));
                }
                continue;
            }
            if lower.contains("repository") {
                discoveries.extend(
                    repository_name_references(value)
                        .into_iter()
                        .map(|name| Discovery::proposed(name, location.clone())),
                );
            }
        }
    }
    Ok(discoveries)
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
