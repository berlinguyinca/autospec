use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanOptions {
    pub repo_dir: PathBuf,
    pub num_specialists: usize,
    pub force: bool,
}

impl ScanOptions {
    pub fn new(repo_dir: impl AsRef<Path>) -> Self {
        Self {
            repo_dir: repo_dir.as_ref().to_path_buf(),
            num_specialists: 3,
            force: false,
        }
    }

    pub fn with_num_specialists(mut self, num_specialists: usize) -> Self {
        self.num_specialists = num_specialists.min(6);
        self
    }

    pub fn force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecialistRoster {
    pub schema_version: u8,
    pub domains: Vec<DetectedDomain>,
    pub suggested_specialists: Vec<SuggestedSpecialist>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedDomain {
    pub name: String,
    pub score: usize,
    pub evidence: Vec<FileLineEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileLineEvidence {
    pub file: String,
    pub line: usize,
    pub r#match: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestedSpecialist {
    pub slug: String,
    pub persona: String,
    pub lens: String,
    pub why: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Copy)]
struct DomainSpec {
    name: &'static str,
    aliases: &'static [&'static str],
    persona: &'static str,
    lens: &'static str,
}

const LEXICON: &[DomainSpec] = &[
    DomainSpec { name: "ms-data", aliases: &["metabolomics", "mzml", "mzxml", "mzdata", "raw-ms", "raw ms", "mass-spec", "mass spec", "mass-spectrometry", "lc-ms", "lc ms", "gc-ms", "gc ms", "ms1", "ms2", "ms/ms", "centroid", "profile-mode", "profile mode", "peak-picking", "peak picking", "feature-table", "feature table"], persona: "MS data specialist", lens: "raw/centroid/profile MS data handling, peak picking, and feature-table reproducibility" },
    DomainSpec { name: "chemical-ids", aliases: &["inchi", "inchikey", "smiles", "canonical-smiles", "canonical smiles", "pubchem", "hmdb", "chebi", "lipidmaps", "cas-number", "cas number", "adduct", "formula", "exact-mass", "exact mass"], persona: "Chemical identifiers specialist", lens: "chemical identifier normalization, adduct/formula ambiguity, and cross-database traceability" },
    DomainSpec { name: "lc-binbase", aliases: &["binbase", "retention-index", "retention index", "retention-time", "retention time", "kovats", "lc-bin", "lc bin", "gc-bin", "gc bin", "alignment-bin", "alignment bin", "chromatogram", "rt-alignment", "rt alignment"], persona: "LC-BinBase specialist", lens: "LC/GC retention alignment, BinBase-style bins, and chromatographic reproducibility" },
    DomainSpec { name: "mona-sirius", aliases: &["mona", "massbank", "sirius", "csi:fingerid", "fingerid", "canopus", "gnps", "spectral-library", "spectral library", "fragmentation-tree", "fragmentation tree", "ms2query"], persona: "MoNA/SIRIUS specialist", lens: "spectral-library search, SIRIUS/CSI:FingerID annotation confidence, and offline fixture boundaries" },
    DomainSpec { name: "hpc-reliability", aliases: &["lab-ops", "lab ops", "slurm", "sbatch", "squeue", "snakemake", "nextflow", "cwl", "singularity", "apptainer", "hpc", "cluster", "array-job", "array job", "job-array", "job array", "checkpoint", "scratch-space", "scratch space"], persona: "HPC reliability specialist", lens: "cluster scheduling, retry/checkpoint behavior, scratch-space cleanup, and reproducible lab pipelines" },
    DomainSpec { name: "trading", aliases: &["ccxt", "backtrader", "zipline", "alpaca", "quantlib", "ta-lib", "talib", "backtest", "backtesting", "order-book", "order book", "market-data", "market data", "ohlcv", "exchange-api", "exchange api", "trading", "brokerage", "portfolio"], persona: "Quantitative trading strategist", lens: "missing risk controls, order-execution correctness, and market-data integrity" },
    DomainSpec { name: "healthcare", aliases: &["hl7", "fhir", "hipaa", "dicom", "ehr", "emr", "patient", "clinical", "healthcare", "phi", "icd-10", "icd10"], persona: "Healthcare compliance reviewer", lens: "PHI handling, HIPAA boundaries, and clinical-data integrity" },
    DomainSpec { name: "payments", aliases: &["stripe", "braintree", "paypal", "pci", "pci-dss", "payment", "checkout", "invoice", "billing", "ledger", "chargeback"], persona: "Payments reliability engineer", lens: "idempotency, reconciliation, and PCI trust boundaries" },
    DomainSpec { name: "ml", aliases: &["pytorch", "torch", "tensorflow", "keras", "scikit-learn", "sklearn", "huggingface", "transformers", "xgboost", "lightgbm", "inference", "training-loop", "training loop", "model-registry", "model registry"], persona: "ML systems reviewer", lens: "training/serving skew, data leakage, and reproducibility" },
    DomainSpec { name: "security", aliases: &["oauth", "jwt", "bcrypt", "argon2", "crypto", "cryptography", "tls", "mtls", "vault", "secrets-manager", "secrets manager", "rbac", "authz", "authentication"], persona: "Application security advisor", lens: "authz boundaries, secret handling, and injection surfaces" },
    DomainSpec { name: "infra", aliases: &["kubernetes", "k8s", "terraform", "helm", "docker-compose", "docker compose", "ansible", "pulumi", "cloudformation", "prometheus", "grafana"], persona: "Infrastructure reliability engineer", lens: "blast radius, rollout safety, and observability gaps" },
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

pub fn scan_specialists(options: &ScanOptions) -> io::Result<SpecialistRoster> {
    if !options.force {
        let cache = cache_path(&options.repo_dir);
        if cache.is_file() {
            let cached = fs::read_to_string(&cache)?;
            if is_valid_roster_json(&cached) {
                if let Some(roster) = parse_generated_roster(&cached) {
                    return Ok(roster);
                }
            }
        }
    }
    let roster = derive_roster(options);
    persist_roster(&options.repo_dir, &roster)?;
    Ok(roster)
}

pub fn scan_specialists_json(options: &ScanOptions) -> io::Result<String> {
    if !options.force {
        let cache = cache_path(&options.repo_dir);
        if cache.is_file() {
            let cached = fs::read_to_string(&cache)?;
            if is_valid_roster_json(&cached) {
                return Ok(cached);
            }
        }
    }
    let roster = derive_roster(options);
    let json = roster.to_json_pretty();
    persist_json(&options.repo_dir, &json)?;
    Ok(json)
}

fn derive_roster(options: &ScanOptions) -> SpecialistRoster {
    let repo_dir = &options.repo_dir;
    let mut hits: Vec<Vec<FileLineEvidence>> = vec![Vec::new(); LEXICON.len()];
    let mut scan_files = root_scan_files(repo_dir);
    let (repo_names, path_signals) = path_signals(repo_dir, &mut scan_files);

    scan_files.sort();
    scan_files.dedup();
    for file in scan_files {
        if let Ok(content) = fs::read_to_string(&file) {
            let rel = relative_path(repo_dir, &file);
            for (line_index, line) in content.lines().enumerate() {
                let normalized = normalize(line);
                for (domain_index, spec) in LEXICON.iter().enumerate() {
                    if matches_any(&normalized, spec.aliases) {
                        record(
                            &mut hits[domain_index],
                            rel.clone(),
                            line_index + 1,
                            line.trim(),
                        );
                    }
                }
            }
        }
    }

    for name in repo_names {
        let normalized = normalize(&name);
        for (domain_index, spec) in LEXICON.iter().enumerate() {
            if matches_any(&normalized, spec.aliases) {
                record(
                    &mut hits[domain_index],
                    ".".to_string(),
                    1,
                    &format!("repo-name: {name}"),
                );
            }
        }
    }

    for path_signal in path_signals {
        let normalized = normalize(&path_signal);
        for (domain_index, spec) in LEXICON.iter().enumerate() {
            if matches_any(&normalized, spec.aliases) {
                record(
                    &mut hits[domain_index],
                    path_signal.clone(),
                    1,
                    &format!("code path: {path_signal}"),
                );
            }
        }
    }

    let mut domains = Vec::new();
    for (index, evidence) in hits.into_iter().enumerate() {
        if evidence.is_empty() {
            continue;
        }
        domains.push(DetectedDomain {
            name: LEXICON[index].name.to_string(),
            score: evidence.len(),
            evidence,
        });
    }
    domains.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.name.cmp(&right.name))
    });

    let suggested_specialists = domains
        .iter()
        .take(options.num_specialists.min(6))
        .filter_map(|domain| {
            let spec = LEXICON.iter().find(|candidate| candidate.name == domain.name)?;
            let first = domain.evidence.first()?;
            Some(SuggestedSpecialist {
                slug: format!("{}-specialist", domain.name),
                persona: spec.persona.to_string(),
                lens: spec.lens.to_string(),
                why: format!(
                    "Repo signals indicate a {} domain; a specialist lens surfaces domain-specific gaps the universal researchers miss.",
                    domain.name
                ),
                evidence: format!("{}:{} ({})", first.file, first.line, first.r#match),
            })
        })
        .collect();

    SpecialistRoster {
        schema_version: 1,
        domains,
        suggested_specialists,
    }
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
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if should_skip_dir(&name) || depth >= 3 {
                continue;
            }
            if depth == 0 {
                signals.insert(name.clone());
            }
            let rel = relative_path(repo_dir, &path);
            signals.insert(format!("{rel}/"));
            walk_paths(repo_dir, &path, depth + 1, scan_files, signals);
        } else if path.is_file() {
            let rel = relative_path(repo_dir, &path);
            if name.ends_with(".csproj") {
                scan_files.push(path);
            }
            signals.insert(rel);
        }
    }
}

fn should_skip_dir(name: &str) -> bool {
    name.starts_with('.') || SKIP_DIRS.contains(&name)
}

fn record(evidence: &mut Vec<FileLineEvidence>, file: String, line: usize, matched: &str) {
    if evidence.len() >= 8 {
        return;
    }
    evidence.push(FileLineEvidence {
        file,
        line,
        r#match: matched.chars().take(120).collect(),
    });
}

fn normalize(value: &str) -> String {
    value.to_ascii_lowercase().replace('_', "-")
}

fn matches_any(haystack: &str, aliases: &[&str]) -> bool {
    aliases
        .iter()
        .any(|alias| contains_tokenish(haystack, alias))
}

fn contains_tokenish(haystack: &str, needle: &str) -> bool {
    if haystack.contains(needle) {
        return true;
    }
    let dashed = needle.replace(' ', "-");
    let spaced = needle.replace('-', " ");
    haystack.contains(&dashed) || haystack.contains(&spaced)
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .and_then(|rel| rel.to_str())
        .unwrap_or_else(|| path.to_str().unwrap_or("."))
        .replace('\\', "/")
}

fn cache_path(repo_dir: &Path) -> PathBuf {
    repo_dir.join(".autospec/explore-specialists.json")
}

fn persist_roster(repo_dir: &Path, roster: &SpecialistRoster) -> io::Result<()> {
    persist_json(repo_dir, &roster.to_json_pretty())
}

fn persist_json(repo_dir: &Path, json: &str) -> io::Result<()> {
    let path = cache_path(repo_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, json)
}

fn is_valid_roster_json(json: &str) -> bool {
    json.contains("\"schema_version\"")
        && json.contains('1')
        && json.contains("\"domains\"")
        && json.contains("\"suggested_specialists\"")
}

fn parse_generated_roster(json: &str) -> Option<SpecialistRoster> {
    // The scanner owns cache generation and normally reuses its own deterministic
    // pretty JSON. If an older/external valid cache is present, callers that need
    // bytes can use scan_specialists_json; typed callers refresh rather than
    // trust a lossy ad-hoc parser.
    if !json.contains("\"schema_version\": 1") && !json.contains("\"schema_version\":1") {
        return None;
    }
    Some(SpecialistRoster {
        schema_version: 1,
        domains: parse_domains(json),
        suggested_specialists: parse_specialists(json),
    })
}

fn parse_domains(json: &str) -> Vec<DetectedDomain> {
    let mut domains = Vec::new();
    let Some(domains_block) = array_block_after(json, "\"domains\"") else {
        return domains;
    };
    for object in top_level_objects(domains_block) {
        let Some(name) = string_field(object, "name") else {
            continue;
        };
        let score = number_field(object, "score").unwrap_or(0);
        let evidence_block = array_block_after(object, "\"evidence\"").unwrap_or("");
        let evidence = top_level_objects(evidence_block)
            .into_iter()
            .filter_map(|ev| {
                Some(FileLineEvidence {
                    file: string_field(ev, "file")?,
                    line: number_field(ev, "line")?,
                    r#match: string_field(ev, "match")?,
                })
            })
            .collect::<Vec<_>>();
        domains.push(DetectedDomain {
            name,
            score,
            evidence,
        });
    }
    domains
}

fn parse_specialists(json: &str) -> Vec<SuggestedSpecialist> {
    let mut specialists = Vec::new();
    let Some(block) = array_block_after(json, "\"suggested_specialists\"") else {
        return specialists;
    };
    for object in top_level_objects(block) {
        if let (Some(slug), Some(persona), Some(lens), Some(why), Some(evidence)) = (
            string_field(object, "slug"),
            string_field(object, "persona"),
            string_field(object, "lens"),
            string_field(object, "why"),
            string_field(object, "evidence"),
        ) {
            specialists.push(SuggestedSpecialist {
                slug,
                persona,
                lens,
                why,
                evidence,
            });
        }
    }
    specialists
}

fn array_block_after<'a>(json: &'a str, marker: &str) -> Option<&'a str> {
    let start = json.find(marker)?;
    let open = json[start..].find('[')? + start;
    let mut depth = 0usize;
    for (offset, ch) in json[open..].char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&json[open + 1..open + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

fn top_level_objects(block: &str) -> Vec<&str> {
    let mut objects = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in block.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => {
                if depth == 0 {
                    start = Some(index);
                }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    if let Some(s) = start.take() {
                        objects.push(&block[s..=index]);
                    }
                }
            }
            _ => {}
        }
    }
    objects
}

fn string_field(object: &str, field: &str) -> Option<String> {
    let marker = format!("\"{field}\"");
    let start = object.find(&marker)?;
    let colon = object[start..].find(':')? + start;
    let quote = object[colon..].find('"')? + colon;
    let mut result = String::new();
    let mut escaped = false;
    for ch in object[quote + 1..].chars() {
        if escaped {
            result.push(match ch {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                other => other,
            });
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return Some(result);
        } else {
            result.push(ch);
        }
    }
    None
}

fn number_field(object: &str, field: &str) -> Option<usize> {
    let marker = format!("\"{field}\"");
    let start = object.find(&marker)?;
    let colon = object[start..].find(':')? + start;
    let digits = object[colon + 1..]
        .chars()
        .skip_while(|c| c.is_whitespace())
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>();
    digits.parse().ok()
}

impl SpecialistRoster {
    pub fn to_json_pretty(&self) -> String {
        let domains = self
            .domains
            .iter()
            .map(DetectedDomain::to_json_pretty)
            .collect::<Vec<_>>()
            .join(",\n");
        let specialists = self
            .suggested_specialists
            .iter()
            .map(SuggestedSpecialist::to_json_pretty)
            .collect::<Vec<_>>()
            .join(",\n");
        format!(
            "{{\n  \"schema_version\": {},\n  \"domains\": [{}{}{}],\n  \"suggested_specialists\": [{}{}{}]\n}}\n",
            self.schema_version,
            if domains.is_empty() { "" } else { "\n" },
            indent_block(&domains, 4),
            if domains.is_empty() { "" } else { "\n  " },
            if specialists.is_empty() { "" } else { "\n" },
            indent_block(&specialists, 4),
            if specialists.is_empty() { "" } else { "\n  " },
        )
    }
}

impl DetectedDomain {
    fn to_json_pretty(&self) -> String {
        let evidence = self
            .evidence
            .iter()
            .map(FileLineEvidence::to_json_pretty)
            .collect::<Vec<_>>()
            .join(",\n");
        format!(
            "{{\n  \"name\": {},\n  \"score\": {},\n  \"evidence\": [\n{}\n  ]\n}}",
            json_string(&self.name),
            self.score,
            indent_block(&evidence, 4)
        )
    }
}

impl FileLineEvidence {
    fn to_json_pretty(&self) -> String {
        format!(
            "{{\n  \"file\": {},\n  \"line\": {},\n  \"match\": {}\n}}",
            json_string(&self.file),
            self.line,
            json_string(&self.r#match)
        )
    }
}

impl SuggestedSpecialist {
    fn to_json_pretty(&self) -> String {
        format!(
            "{{\n  \"slug\": {},\n  \"persona\": {},\n  \"lens\": {},\n  \"why\": {},\n  \"evidence\": {}\n}}",
            json_string(&self.slug),
            json_string(&self.persona),
            json_string(&self.lens),
            json_string(&self.why),
            json_string(&self.evidence)
        )
    }
}

fn indent_block(block: &str, spaces: usize) -> String {
    if block.is_empty() {
        return String::new();
    }
    let prefix = " ".repeat(spaces);
    block
        .lines()
        .map(|line| format!("{prefix}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32))
            }
            character => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}
