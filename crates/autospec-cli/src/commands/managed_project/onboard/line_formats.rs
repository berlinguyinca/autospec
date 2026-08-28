use super::{field_repository, Discovery};
use std::path::Path;

pub(super) fn scan(root: &Path) -> Vec<Discovery> {
    let mut discoveries = gitmodules(root);
    discoveries.extend(go_mod(root));
    discoveries.extend(fleet(root.join("autospec-fleet.yml")));
    discoveries.extend(fleet(root.join(".autospec/fleet.yml")));
    discoveries
}

fn gitmodules(root: &Path) -> Vec<Discovery> {
    let Ok(source) = std::fs::read_to_string(root.join(".gitmodules")) else {
        return Vec::new();
    };
    source
        .lines()
        .filter_map(|line| line.trim().strip_prefix("url ="))
        .map(|url| Discovery::repository(url.trim(), "submodule", ".gitmodules:url"))
        .collect()
}

fn go_mod(root: &Path) -> Vec<Discovery> {
    let Ok(source) = std::fs::read_to_string(root.join("go.mod")) else {
        return Vec::new();
    };
    let mut discoveries = Vec::new();
    for line in source.lines().map(str::trim) {
        let value = if let Some(module) = line.strip_prefix("module ") {
            Some(module)
        } else if let Some((_, replacement)) = line
            .strip_prefix("replace ")
            .and_then(|line| line.split_once("=>"))
        {
            replacement.split_whitespace().next()
        } else {
            None
        };
        if let Some(repository) = value.and_then(field_repository) {
            discoveries.push(Discovery::repository(
                repository,
                "manifest-dependency",
                "go.mod:module-or-replace",
            ));
        }
    }
    discoveries
}

fn fleet(path: impl AsRef<Path>) -> Vec<Discovery> {
    let path = path.as_ref();
    let Ok(source) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut in_repositories = false;
    let mut discoveries = Vec::new();
    for raw_line in source.lines() {
        let line = raw_line.trim();
        if !raw_line.starts_with(char::is_whitespace) {
            in_repositories = line == "repositories:";
            continue;
        }
        if !in_repositories {
            continue;
        }
        let value = line
            .strip_prefix('-')
            .map(str::trim)
            .or_else(|| line.strip_prefix("url:").map(str::trim));
        if let Some(repository) = value.and_then(field_repository) {
            discoveries.push(Discovery::repository(
                repository,
                "fleet",
                path.display().to_string(),
            ));
        }
    }
    discoveries
}
