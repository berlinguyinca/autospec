use super::{quoted_values, Discovery};
use std::path::Path;

pub(super) fn scan(root: &Path) -> Vec<Discovery> {
    let Ok(source) = std::fs::read_to_string(root.join("Cargo.toml")) else {
        return Vec::new();
    };
    let mut discoveries = Vec::new();
    let mut section = String::new();
    let mut workspace_members = false;
    for raw_line in source.lines() {
        let line = strip_comment(raw_line).trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_ascii_lowercase();
            workspace_members = false;
            continue;
        }
        if section == "workspace" && line.starts_with("members") {
            workspace_members = true;
        }
        if workspace_members {
            for member in quoted_values(line) {
                discoveries.push(Discovery::workspace(
                    root.join(member),
                    "manifest-dependency",
                    "Cargo.toml:workspace.members",
                ));
            }
            if line.contains(']') {
                workspace_members = false;
            }
            continue;
        }
        if !dependency_section(&section) {
            continue;
        }
        let Some((_, value)) = line.split_once('=') else {
            continue;
        };
        if let Some(git) = named_string(value, "git") {
            discoveries.push(Discovery::repository(
                git,
                "manifest-dependency",
                "Cargo.toml:dependency.git",
            ));
        }
        if let Some(path) = named_string(value, "path") {
            discoveries.push(Discovery::workspace(
                root.join(path),
                "manifest-dependency",
                "Cargo.toml:dependency.path",
            ));
        }
    }
    discoveries
}

fn dependency_section(section: &str) -> bool {
    section == "dependencies"
        || section == "dev-dependencies"
        || section == "build-dependencies"
        || section == "workspace.dependencies"
        || section.ends_with(".dependencies")
        || section.ends_with(".dev-dependencies")
        || section.ends_with(".build-dependencies")
}

fn named_string<'a>(value: &'a str, name: &str) -> Option<&'a str> {
    let marker = format!("{name} =");
    let tail = value.split_once(&marker)?.1.trim_start();
    quoted_values(tail).into_iter().next()
}

fn strip_comment(line: &str) -> &str {
    let mut quote = None;
    for (index, character) in line.char_indices() {
        match character {
            '\"' | '\'' if quote == Some(character) => quote = None,
            '\"' | '\'' if quote.is_none() => quote = Some(character),
            '#' if quote.is_none() => return &line[..index],
            _ => {}
        }
    }
    line
}
