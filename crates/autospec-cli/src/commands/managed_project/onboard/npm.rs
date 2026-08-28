use super::{expand_workspace_path, normalize_github_repository, Discovery};
use serde_json::Value;
use std::path::Path;

pub(super) fn scan(root: &Path) -> Vec<Discovery> {
    let mut discoveries = package_json(root);
    discoveries.extend(pnpm_workspace(root));
    discoveries
}

fn package_json(root: &Path) -> Vec<Discovery> {
    let Ok(source) = std::fs::read_to_string(root.join("package.json")) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&source) else {
        return Vec::new();
    };
    let mut discoveries = Vec::new();
    if let Some(repository) = repository_value(value.get("repository")).and_then(github_repository)
    {
        discoveries.push(Discovery::repository(
            repository,
            "manifest-dependency",
            "package.json:repository",
        ));
    }
    for field in [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ] {
        for repository in value
            .get(field)
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|dependencies| dependencies.values())
            .filter_map(Value::as_str)
            .filter_map(github_repository)
        {
            discoveries.push(Discovery::repository(
                repository,
                "manifest-dependency",
                format!("package.json:{field}"),
            ));
        }
    }
    let workspaces = value
        .get("workspaces")
        .and_then(Value::as_array)
        .or_else(|| {
            value
                .get("workspaces")
                .and_then(|workspaces| workspaces.get("packages"))
                .and_then(Value::as_array)
        });
    for workspace in workspaces.into_iter().flatten().filter_map(Value::as_str) {
        for path in expand_workspace_path(root, workspace) {
            discoveries.push(Discovery::workspace(
                path,
                "manifest-dependency",
                "package.json:workspaces",
            ));
        }
    }
    discoveries
}

fn github_repository(value: &str) -> Option<&str> {
    normalize_github_repository(value).map(|_| value)
}

fn repository_value(value: Option<&Value>) -> Option<&str> {
    value.and_then(|value| {
        value
            .as_str()
            .or_else(|| value.get("url").and_then(Value::as_str))
    })
}

fn pnpm_workspace(root: &Path) -> Vec<Discovery> {
    let Ok(source) = std::fs::read_to_string(root.join("pnpm-workspace.yaml")) else {
        return Vec::new();
    };
    let mut in_packages = false;
    let mut discoveries = Vec::new();
    for raw_line in source.lines() {
        let line = raw_line.trim();
        if !raw_line.starts_with(char::is_whitespace) {
            in_packages = line == "packages:";
            continue;
        }
        if !in_packages {
            continue;
        }
        let Some(pattern) = line.strip_prefix('-') else {
            continue;
        };
        let pattern = pattern.trim().trim_matches(['\"', '\'']);
        for path in expand_workspace_path(root, pattern) {
            discoveries.push(Discovery::workspace(
                path,
                "manifest-dependency",
                "pnpm-workspace.yaml:packages",
            ));
        }
    }
    discoveries
}
