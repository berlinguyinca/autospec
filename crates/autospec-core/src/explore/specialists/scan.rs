use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::roster_json::parse_proposal_specialists;
use super::{DetectedDomain, FileLineEvidence, SpecialistRoster, SuggestedSpecialist};

#[derive(Debug, Clone, Copy)]
struct DomainSpec {
    name: &'static str,
    aliases: &'static [&'static str],
    persona: &'static str,
    lens: &'static str,
}

const LEXICON: &[DomainSpec] = &[
    DomainSpec {
        name: "ms-data",
        aliases: &["metabolomics", "mzml", "mzxml", "mzdata", "raw-ms", "raw ms", "mass-spec", "mass spec", "mass-spectrometry", "lc-ms", "lc ms", "gc-ms", "gc ms", "ms1", "ms2", "ms/ms", "centroid", "profile-mode", "profile mode", "peak-picking", "peak picking", "feature-table", "feature table"],
        persona: "MS data specialist",
        lens: "raw/centroid/profile MS data handling, peak picking, and feature-table reproducibility",
    },
    DomainSpec {
        name: "chemical-ids",
        aliases: &["inchi", "inchikey", "smiles", "canonical-smiles", "canonical smiles", "pubchem", "hmdb", "chebi", "lipidmaps", "cas-number", "cas number", "adduct", "formula", "exact-mass", "exact mass"],
        persona: "Chemical identifiers specialist",
        lens: "chemical identifier normalization, adduct/formula ambiguity, and cross-database traceability",
    },
    DomainSpec {
        name: "lc-binbase",
        aliases: &["binbase", "retention-index", "retention index", "retention-time", "retention time", "kovats", "lc-bin", "lc bin", "gc-bin", "gc bin", "alignment-bin", "alignment bin", "chromatogram", "rt-alignment", "rt alignment"],
        persona: "LC-BinBase specialist",
        lens: "LC/GC retention alignment, BinBase-style bins, and chromatographic reproducibility",
    },
    DomainSpec {
        name: "mona-sirius",
        aliases: &["mona", "massbank", "sirius", "csi:fingerid", "fingerid", "canopus", "gnps", "spectral-library", "spectral library", "fragmentation-tree", "fragmentation tree", "ms2query"],
        persona: "MoNA/SIRIUS specialist",
        lens: "spectral-library search, SIRIUS/CSI:FingerID annotation confidence, and offline fixture boundaries",
    },
    DomainSpec {
        name: "hpc-reliability",
        aliases: &["lab-ops", "lab ops", "slurm", "sbatch", "squeue", "snakemake", "nextflow", "cwl", "singularity", "apptainer", "hpc", "cluster", "array-job", "array job", "job-array", "job array", "checkpoint", "scratch-space", "scratch space"],
        persona: "HPC reliability specialist",
        lens: "cluster scheduling, retry/checkpoint behavior, scratch-space cleanup, and reproducible lab pipelines",
    },
    DomainSpec {
        name: "trading",
        aliases: &["ccxt", "backtrader", "zipline", "alpaca", "quantlib", "ta-lib", "talib", "backtest", "backtesting", "order-book", "order book", "market-data", "market data", "ohlcv", "exchange-api", "exchange api", "trading", "brokerage", "portfolio"],
        persona: "Quantitative trading strategist",
        lens: "missing risk controls, order-execution correctness, and market-data integrity",
    },
    DomainSpec {
        name: "healthcare",
        aliases: &["hl7", "fhir", "hipaa", "dicom", "ehr", "emr", "patient", "clinical", "healthcare", "phi", "icd-10", "icd10"],
        persona: "Healthcare compliance reviewer",
        lens: "PHI handling, HIPAA boundaries, and clinical-data integrity",
    },
    DomainSpec {
        name: "payments",
        aliases: &["stripe", "braintree", "paypal", "pci", "pci-dss", "payment", "checkout", "invoice", "billing", "ledger", "chargeback"],
        persona: "Payments reliability engineer",
        lens: "idempotency, reconciliation, and PCI trust boundaries",
    },
    DomainSpec {
        name: "ml",
        aliases: &["pytorch", "torch", "tensorflow", "keras", "scikit-learn", "sklearn", "huggingface", "transformers", "xgboost", "lightgbm", "inference", "training-loop", "training loop", "model-registry", "model registry"],
        persona: "ML systems reviewer",
        lens: "training/serving skew, data leakage, and reproducibility",
    },
    DomainSpec {
        name: "security",
        aliases: &["oauth", "jwt", "bcrypt", "argon2", "crypto", "cryptography", "tls", "mtls", "vault", "secrets-manager", "secrets manager", "rbac", "authz", "authentication"],
        persona: "Application security advisor",
        lens: "authz boundaries, secret handling, and injection surfaces",
    },
    DomainSpec {
        name: "infra",
        aliases: &["kubernetes", "k8s", "terraform", "helm", "docker-compose", "docker compose", "ansible", "pulumi", "cloudformation", "prometheus", "grafana"],
        persona: "Infrastructure reliability engineer",
        lens: "blast radius, rollout safety, and observability gaps",
    },
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

pub(crate) fn derive_roster(
    repo_dir: &Path,
    proposal_input: Option<&str>,
    limit: usize,
) -> SpecialistRoster {
    let mut hits = vec![Vec::new(); LEXICON.len()];
    let mut scan_files = root_scan_files(repo_dir);
    let (repo_names, path_signals) = path_signals(repo_dir, &mut scan_files);
    scan_files.sort();
    scan_files.dedup();
    for file in scan_files {
        scan_file(repo_dir, &file, &mut hits);
    }
    scan_identifiers(&repo_names, ".", "repo-name", &mut hits);
    scan_identifiers(&path_signals, "", "code path", &mut hits);

    let domains = ranked_domains(hits);
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
    MANIFESTS
        .iter()
        .chain(DOCS.iter())
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
    if should_skip_dir(name) || depth >= 3 {
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
        scan_line(&relative, index + 1, line, hits);
    }
}

fn scan_line(relative: &str, line_number: usize, line: &str, hits: &mut [Vec<FileLineEvidence>]) {
    let normalized = normalize(line);
    for (index, spec) in LEXICON.iter().enumerate() {
        if matches_any(&normalized, spec.aliases) {
            record(
                &mut hits[index],
                relative.to_string(),
                line_number,
                line.trim(),
            );
        }
    }
}

fn scan_identifiers(
    values: &[String],
    file: &str,
    source: &str,
    hits: &mut [Vec<FileLineEvidence>],
) {
    for value in values {
        let normalized = normalize(value);
        for (index, spec) in LEXICON.iter().enumerate() {
            if matches_any(&normalized, spec.aliases) {
                let evidence_file = if file.is_empty() {
                    value.clone()
                } else {
                    file.to_string()
                };
                record(
                    &mut hits[index],
                    evidence_file,
                    1,
                    &format!("{source}: {value}"),
                );
            }
        }
    }
}

fn ranked_domains(hits: Vec<Vec<FileLineEvidence>>) -> Vec<DetectedDomain> {
    let mut domains = hits
        .into_iter()
        .enumerate()
        .filter_map(|(index, evidence)| {
            (!evidence.is_empty()).then(|| DetectedDomain {
                name: LEXICON[index].name.to_string(),
                score: evidence.len(),
                evidence,
            })
        })
        .collect::<Vec<_>>();
    domains.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.name.cmp(&right.name))
    });
    domains
}

fn fallback_specialists(domains: &[DetectedDomain]) -> Vec<SuggestedSpecialist> {
    domains
        .iter()
        .filter_map(|domain| {
            let spec = LEXICON.iter().find(|spec| spec.name == domain.name)?;
            let evidence = domain.evidence.first()?;
            Some(SuggestedSpecialist {
                slug: format!("{}-specialist", domain.name),
                persona: spec.persona.to_string(),
                lens: spec.lens.to_string(),
                why: format!(
                    "Repo signals indicate a {} domain; a specialist lens surfaces domain-specific gaps the universal researchers miss.",
                    domain.name
                ),
                evidence: format!("{}:{} ({})", evidence.file, evidence.line, evidence.r#match),
            })
        })
        .collect()
}

fn should_skip_dir(name: &str) -> bool {
    name.starts_with('.') || SKIP_DIRS.contains(&name)
}

fn record(evidence: &mut Vec<FileLineEvidence>, file: String, line: usize, matched: &str) {
    if evidence.len() < 8 {
        evidence.push(FileLineEvidence {
            file,
            line,
            r#match: matched.chars().take(120).collect(),
        });
    }
}

fn normalize(value: &str) -> String {
    value.to_ascii_lowercase().replace('_', "-")
}

fn matches_any(haystack: &str, aliases: &[&str]) -> bool {
    aliases.iter().any(|alias| {
        token_contains(haystack, alias)
            || token_contains(haystack, &alias.replace(' ', "-"))
            || token_contains(haystack, &alias.replace('-', " "))
    })
}

fn token_contains(haystack: &str, needle: &str) -> bool {
    let mut offset = 0;
    while let Some(found) = haystack[offset..].find(needle) {
        let start = offset + found;
        let end = start + needle.len();
        if boundary(haystack[..start].chars().next_back())
            && boundary(haystack[end..].chars().next())
        {
            return true;
        }
        offset = end;
    }
    false
}

fn boundary(character: Option<char>) -> bool {
    character.is_none_or(|character| !character.is_ascii_alphanumeric())
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .and_then(|relative| relative.to_str())
        .unwrap_or_else(|| path.to_str().unwrap_or("."))
        .replace('\\', "/")
}
