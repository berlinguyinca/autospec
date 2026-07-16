use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::lexicon;
use super::roster_json::parse_proposal_specialists;
use super::{FileLineEvidence, SpecialistRoster, SuggestedSpecialist};

pub(crate) fn derive_roster(
    repo_dir: &Path,
    proposal_input: Option<&str>,
    limit: usize,
) -> SpecialistRoster {
    let mut hits = lexicon::empty_hits();
    let mut scan_files = root_scan_files(repo_dir);
    let (repo_names, path_signals) = path_signals(repo_dir, &mut scan_files);
    scan_files.sort();
    scan_files.dedup();
    for file in scan_files {
        scan_file(repo_dir, &file, &mut hits);
    }
    lexicon::scan_identifiers(&repo_names, ".", "repo-name", &mut hits);
    lexicon::scan_identifiers(&path_signals, "", "code path", &mut hits);

    let domains = lexicon::ranked_domains(hits);
    let suggested_specialists = proposal_input
        .and_then(parse_proposal_specialists)
        .unwrap_or_else(|| fallback_specialists(&domains));
    SpecialistRoster {
        schema_version: 1,
        domains,
        suggested_specialists,
    }
    .capped(limit)
}

fn root_scan_files(repo_dir: &Path) -> Vec<PathBuf> {
    lexicon::signal_file_names()
        .map(|name| repo_dir.join(name))
        .filter(|path| path.is_file())
        .collect()
}

fn path_signals(repo_dir: &Path, scan_files: &mut Vec<PathBuf>) -> (Vec<String>, Vec<String>) {
    let repo_name = repo_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_string();
    let mut signals = BTreeSet::new();
    walk_paths(repo_dir, repo_dir, 0, scan_files, &mut signals);
    (vec![repo_name], signals.into_iter().collect())
}

fn walk_paths(
    repo_dir: &Path,
    current: &Path,
    depth: usize,
    scan_files: &mut Vec<PathBuf>,
    signals: &mut BTreeSet<String>,
) {
    let Ok(entries) = fs::read_dir(current) else {
        return;
    };
    let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if file_type.is_dir() {
            visit_directory(repo_dir, &path, &name, depth, scan_files, signals);
        } else if file_type.is_file() {
            visit_file(repo_dir, path, &name, scan_files, signals);
        }
    }
}

fn visit_directory(
    repo_dir: &Path,
    path: &Path,
    name: &str,
    depth: usize,
    scan_files: &mut Vec<PathBuf>,
    signals: &mut BTreeSet<String>,
) {
    if lexicon::should_skip_dir(name) || depth >= 3 {
        return;
    }
    if depth == 0 {
        signals.insert(name.to_string());
    }
    let relative = relative_path(repo_dir, path);
    signals.insert(format!("{relative}/"));
    walk_paths(repo_dir, path, depth + 1, scan_files, signals);
}

fn visit_file(
    repo_dir: &Path,
    path: PathBuf,
    name: &str,
    scan_files: &mut Vec<PathBuf>,
    signals: &mut BTreeSet<String>,
) {
    if name.ends_with(".csproj") {
        scan_files.push(path.clone());
    }
    signals.insert(relative_path(repo_dir, &path));
}

fn scan_file(repo_dir: &Path, file: &Path, hits: &mut [Vec<FileLineEvidence>]) {
    let Ok(content) = fs::read_to_string(file) else {
        return;
    };
    let relative = relative_path(repo_dir, file);
    for (index, line) in content.lines().enumerate() {
        lexicon::scan_line(&relative, index + 1, line, hits);
    }
}

fn fallback_specialists(domains: &[super::DetectedDomain]) -> Vec<SuggestedSpecialist> {
    domains
        .iter()
        .filter_map(lexicon::fallback_specialist)
        .collect()
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .and_then(|relative| relative.to_str())
        .unwrap_or_else(|| path.to_str().unwrap_or("."))
        .replace('\\', "/")
}
