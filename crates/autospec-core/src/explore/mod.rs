use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use json::is_valid_roster_cache;
use lexicon::domain_specs;

mod json;
mod lexicon;

const CACHE_PATH: &str = ".autospec/explore-specialists.json";
const MAX_SPECIALISTS: usize = 6;
const EVIDENCE_CAP: usize = 8;
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "vendor",
    ".venv",
    "venv",
    "target",
    "dist",
    "build",
    ".autospec",
    "__pycache__",
];
const MANIFESTS: &[&str] = &[
    "package.json",
    "requirements.txt",
    "pyproject.toml",
    "go.mod",
    "Cargo.toml",
    "Gemfile",
    "pom.xml",
    "build.gradle",
    "environment.yml",
    "environment.yaml",
    "conda.yml",
    "renv.lock",
    "DESCRIPTION",
    "Snakefile",
    "nextflow.config",
];
const DOCS: &[&str] = &["README.md", "AGENTS.md", "README.rst", "docs/README.md"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainEvidence {
    pub file: String,
    pub line: usize,
    pub match_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedDomain {
    pub name: String,
    pub score: usize,
    pub evidence: Vec<DomainEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestedSpecialist {
    pub slug: String,
    pub persona: String,
    pub lens: String,
    pub why: String,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecialistRoster {
    pub schema_version: u32,
    pub domains: Vec<DetectedDomain>,
    pub suggested_specialists: Vec<SuggestedSpecialist>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecialistScanOptions {
    pub repo_dir: PathBuf,
    pub num_specialists: usize,
    pub force: bool,
}

impl SpecialistScanOptions {
    pub fn new(repo_dir: impl AsRef<Path>, num_specialists: usize) -> Self {
        Self {
            repo_dir: repo_dir.as_ref().to_path_buf(),
            num_specialists,
            force: false,
        }
    }

    fn capped_num_specialists(&self) -> usize {
        self.num_specialists.min(MAX_SPECIALISTS)
    }
}

pub fn discover_specialists_json(options: &SpecialistScanOptions) -> Result<String, String> {
    let repo_dir = canonical_or_original(&options.repo_dir);
    let cache_path = repo_dir.join(CACHE_PATH);
    if !options.force {
        if let Ok(cached) = fs::read_to_string(&cache_path) {
            if is_valid_roster_cache(&cached) {
                return Ok(cached);
            }
        }
    }

    let mut scan_options = options.clone();
    scan_options.repo_dir = repo_dir.clone();
    let output = scan_specialist_roster(&scan_options)?.to_json_pretty();
    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    fs::write(&cache_path, &output)
        .map_err(|error| format!("could not write {}: {error}", cache_path.display()))?;
    Ok(output)
}

pub fn scan_specialist_roster(options: &SpecialistScanOptions) -> Result<SpecialistRoster, String> {
    let root = canonical_or_original(&options.repo_dir);
    let mut hits = domain_specs()
        .iter()
        .map(|spec| (spec.name, Vec::<DomainEvidence>::new()))
        .collect::<Vec<_>>();

    let mut scan_files = Vec::new();
    for relative in MANIFESTS.iter().chain(DOCS.iter()) {
        let path = root.join(relative);
        if path.is_file() {
            scan_files.push(path);
        }
    }

    let mut repo_name_signals = Vec::new();
    if let Some(name) = root.file_name().and_then(|name| name.to_str()) {
        repo_name_signals.push(name.to_string());
    }
    let mut path_signals = BTreeSet::new();
    let mut dir_names = BTreeSet::new();
    collect_path_signals(
        &root,
        &root,
        0,
        &mut scan_files,
        &mut path_signals,
        &mut dir_names,
    );

    for file in &scan_files {
        let Ok(content) = fs::read_to_string(file) else {
            continue;
        };
        let relative = relative_path(&root, file);
        for (index, line) in content.lines().enumerate() {
            for spec in domain_specs() {
                if matches_any_token(line, spec.tokens) {
                    record_hit(&mut hits, spec.name, &relative, index + 1, line.trim());
                }
            }
        }
    }

    for name in repo_name_signals {
        for spec in domain_specs() {
            if matches_any_token(&name, spec.tokens) {
                record_hit(&mut hits, spec.name, ".", 1, &format!("repo-name: {name}"));
            }
        }
    }

    for signal in dir_names.into_iter().chain(path_signals) {
        for spec in domain_specs() {
            if matches_any_token(&signal, spec.tokens) {
                record_hit(
                    &mut hits,
                    spec.name,
                    &signal,
                    1,
                    &format!("code path: {signal}"),
                );
            }
        }
    }

    let mut domains = hits
        .into_iter()
        .filter_map(|(name, evidence)| {
            if evidence.is_empty() {
                None
            } else {
                Some(DetectedDomain {
                    name: name.to_string(),
                    score: evidence.len(),
                    evidence,
                })
            }
        })
        .collect::<Vec<_>>();
    domains.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.name.cmp(&right.name))
    });

    let suggested_specialists = domains
        .iter()
        .take(options.capped_num_specialists())
        .filter_map(specialist_for_domain)
        .collect::<Vec<_>>();

    Ok(SpecialistRoster {
        schema_version: 1,
        domains,
        suggested_specialists,
    })
}

fn specialist_for_domain(domain: &DetectedDomain) -> Option<SuggestedSpecialist> {
    let spec = domain_specs()
        .iter()
        .find(|spec| spec.name == domain.name)?;
    let first = domain.evidence.first()?;
    Some(SuggestedSpecialist {
        slug: format!("{}-specialist", spec.name),
        persona: spec.persona.to_string(),
        lens: spec.lens.to_string(),
        why: format!(
            "Repo signals indicate a {} domain; a specialist lens surfaces domain-specific gaps the universal researchers miss.",
            spec.name
        ),
        evidence: format!("{}:{} ({})", first.file, first.line, first.match_text),
    })
}

fn collect_path_signals(
    root: &Path,
    current: &Path,
    depth: usize,
    scan_files: &mut Vec<PathBuf>,
    path_signals: &mut BTreeSet<String>,
    dir_names: &mut BTreeSet<String>,
) {
    let Ok(entries) = fs::read_dir(current) else {
        return;
    };
    let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());

    let mut child_dirs = Vec::new();
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if should_skip_dir(&name) || depth >= 3 {
                continue;
            }
            if depth == 0 {
                dir_names.insert(name.clone());
            }
            let relative = relative_path(root, &path);
            path_signals.insert(format!("{relative}/"));
            child_dirs.push(path);
        } else if path.is_file() {
            let relative = relative_path(root, &path);
            if name.ends_with(".csproj") {
                scan_files.push(path);
            }
            path_signals.insert(relative);
        }
    }

    for child in child_dirs {
        collect_path_signals(root, &child, depth + 1, scan_files, path_signals, dir_names);
    }
}

fn should_skip_dir(name: &str) -> bool {
    name.starts_with('.') || SKIP_DIRS.contains(&name)
}

fn record_hit(
    hits: &mut [(&'static str, Vec<DomainEvidence>)],
    domain: &'static str,
    file: &str,
    line: usize,
    match_text: &str,
) {
    let Some((_, evidence)) = hits.iter_mut().find(|(name, _)| *name == domain) else {
        return;
    };
    if evidence.len() >= EVIDENCE_CAP {
        return;
    }
    evidence.push(DomainEvidence {
        file: file.to_string(),
        line,
        match_text: truncate_chars(match_text, 120),
    });
}

fn matches_any_token(value: &str, tokens: &[&str]) -> bool {
    let normalized = normalize_signal(value);
    tokens.iter().any(|token| normalized.contains(token))
}

fn normalize_signal(value: &str) -> String {
    value
        .chars()
        .map(|character| match character.to_ascii_lowercase() {
            '_' | '-' => ' ',
            character => character,
        })
        .collect()
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string()
}

fn canonical_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}
