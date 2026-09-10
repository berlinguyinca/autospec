//! Repository knowledge cache and staged test resolution (unified routing
//! foundation).
//!
//! Model context is working memory, not repository memory (spec section 6).
//! The module, symbol, dependency, interface, build-command and test maps
//! live in a deterministic cache file on disk, outside the active context:
//! the agent reads a short staged plan, not a re-scanned repository. Every
//! record carries a source digest, so a refresh re-derives only the records
//! whose file actually changed and leaves every untouched record
//! byte-identical.
//!
//! Resolution is staged, narrowest first: Stage 1 targeted commands, Stage 2
//! module commands, Stage 3 repository commands. A later stage can never be
//! started before every earlier stage has passed.
//!
//! The module is pure and provider-neutral like the rest of AAR: it derives
//! records from a path plus contents, renders and parses the cache file, and
//! computes staged plans. The driver performs the file I/O.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::autonomous::test_gate::reverse_closure;

/// Directory, relative to the worktree root, holding the knowledge cache.
pub const KNOWLEDGE_DIR: &str = ".autospec/knowledge";

/// Cache file, relative to the worktree root, holding the whole map.
pub const KNOWLEDGE_FILE: &str = ".autospec/knowledge/repository.json";

/// The Stage 3 command that covers the whole repository.
pub const REPOSITORY_TEST_COMMAND: &str = "cargo test --workspace --no-fail-fast";

/// The languages the fixture mappings understand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    /// Rust crate sources and tests under `crates/<pkg>/`.
    Rust,
    /// Shell scripts and bats fixtures.
    Shell,
    /// Markdown documentation.
    Documentation,
    /// Anything the resolver cannot map to a narrower check.
    Unknown,
}

impl Language {
    pub fn as_str(&self) -> &'static str {
        match self {
            Language::Rust => "rust",
            Language::Shell => "shell",
            Language::Documentation => "documentation",
            Language::Unknown => "unknown",
        }
    }
}

/// Classify a repository-relative path, narrowest fixture first.
pub fn language_for(path: &str) -> Language {
    match path {
        path if path.ends_with(".rs") => Language::Rust,
        path if path.ends_with(".sh") || path.ends_with(".bats") => Language::Shell,
        path if path.ends_with(".md") || path.ends_with(".markdown") => Language::Documentation,
        _ => Language::Unknown,
    }
}

/// sha256 of a file's contents, for entry invalidation.
pub fn digest_of(contents: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(contents.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// One cached module: its identity, source digest, symbol/dependency/
/// interface maps, and the build and test commands that cover it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleRecord {
    /// Stable identity (`<pkg>::<module>` for Rust, the path otherwise).
    pub module: String,
    /// Repository-relative source path.
    pub path: String,
    /// sha256 of the source contents this record was derived from.
    pub source_digest: String,
    /// Top-level items the module defines.
    pub symbols: Vec<String>,
    /// In-repo items the module depends on (crate imports, sources, links).
    pub dependencies: Vec<String>,
    /// Public surface the module exposes.
    pub interfaces: Vec<String>,
    /// Command that builds (or syntax-checks) this module.
    pub build_command: String,
    /// Test commands, narrowest first; the last entry is the repository check.
    pub test_commands: Vec<String>,
}

impl ModuleRecord {
    /// Stage commands from the test map: the first entry is Stage 1
    /// (targeted), the middle entries Stage 2 (module), the last entry
    /// Stage 3 (repository). A single-entry map is both targeted and
    /// repository — there is no narrower check for that module.
    pub fn staged_commands(&self) -> [Vec<String>; 3] {
        let count = self.test_commands.len();
        match count {
            0 => [vec![], vec![], vec![]],
            1 => {
                let only = self.test_commands[0].clone();
                [vec![only.clone()], vec![], vec![only]]
            }
            _ => {
                let first = self.test_commands[0].clone();
                let middle = self.test_commands[1..count - 1].to_vec();
                let last = self.test_commands[count - 1].clone();
                [vec![first], middle, vec![last]]
            }
        }
    }
}

/// Derive a record from one file: language maps, digest, and commands.
pub fn scan_module(path: &str, contents: &str) -> ModuleRecord {
    let digest = digest_of(contents);
    let language = language_for(path);
    let (symbols, dependencies, interfaces) = match language {
        Language::Rust => scan_rust(contents),
        Language::Shell => scan_shell(contents),
        Language::Documentation => scan_documentation(contents),
        Language::Unknown => (Vec::new(), Vec::new(), Vec::new()),
    };
    let (module, build_command, test_commands) = match language {
        Language::Rust => rust_module_identity_and_commands(path),
        Language::Shell => {
            let command = if path.ends_with(".bats") {
                format!("bats {path}")
            } else {
                format!("bash -n {path}")
            };
            (
                path.to_string(),
                command.clone(),
                vec![command, REPOSITORY_TEST_COMMAND.to_string()],
            )
        }
        Language::Documentation => (
            path.to_string(),
            "n/a - documentation is not built".to_string(),
            vec![REPOSITORY_TEST_COMMAND.to_string()],
        ),
        Language::Unknown => (
            path.to_string(),
            REPOSITORY_TEST_COMMAND.to_string(),
            vec![REPOSITORY_TEST_COMMAND.to_string()],
        ),
    };
    ModuleRecord {
        module,
        path: path.to_string(),
        source_digest: digest,
        symbols,
        dependencies,
        interfaces,
        build_command,
        test_commands,
    }
}

/// The whole cache, keyed by source path so rendering is byte-stable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KnowledgeCache {
    records: BTreeMap<String, ModuleRecord>,
    /// Workspace crate dependency graph: crate -> the crates it directly
    /// depends on. Test scoping closes over this map in reverse, so a
    /// change to a library crate also runs the tests of every crate that
    /// depends on it, transitively (#3767).
    crate_dependencies: BTreeMap<String, Vec<String>>,
}

/// What a refresh did: which entries were re-derived, kept, or dropped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefreshReport {
    pub refreshed: Vec<String>,
    pub unchanged: Vec<String>,
    pub removed: Vec<String>,
}

impl KnowledgeCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Insert or replace the record for a path.
    pub fn record(&mut self, record: ModuleRecord) {
        self.records.insert(record.path.clone(), record);
    }

    pub fn get(&self, path: &str) -> Option<&ModuleRecord> {
        self.records.get(path)
    }

    /// Canonical bytes for one record; equal records render byte-identically.
    pub fn render_record(record: &ModuleRecord) -> String {
        serde_json::to_string_pretty(record).expect("ModuleRecord serializes")
    }

    /// Canonical bytes for the whole cache, stable across renders.
    pub fn render(&self) -> String {
        #[derive(Serialize)]
        struct CacheFile<'a> {
            records: &'a BTreeMap<String, ModuleRecord>,
            crate_dependencies: &'a BTreeMap<String, Vec<String>>,
        }
        serde_json::to_string_pretty(&CacheFile {
            records: &self.records,
            crate_dependencies: &self.crate_dependencies,
        })
        .expect("cache serializes")
    }

    /// Parse a rendered cache file back into entries.
    ///
    /// Cache files written before #3767 carry no `crate_dependencies`
    /// field; those load with an empty graph, and scoping then stays on
    /// the changed crates while Stage 3 covers the rest.
    pub fn load(&mut self, contents: &str) -> Result<(), String> {
        #[derive(Deserialize)]
        struct CacheFile {
            #[serde(default)]
            records: BTreeMap<String, ModuleRecord>,
            #[serde(default)]
            crate_dependencies: BTreeMap<String, Vec<String>>,
        }
        let file: CacheFile =
            serde_json::from_str(contents).map_err(|error| format!("bad cache file: {error}"))?;
        self.records = file.records;
        self.crate_dependencies = file.crate_dependencies;
        Ok(())
    }

    /// Set the workspace crate dependency graph (crate -> the crates it
    /// directly depends on). An empty graph means "no graph data": scoping
    /// stays on the changed crates and Stage 3 covers the rest.
    pub fn set_crate_dependencies(&mut self, graph: BTreeMap<String, Vec<String>>) {
        self.crate_dependencies = graph;
    }

    /// The workspace crate dependency graph.
    pub fn crate_dependencies(&self) -> &BTreeMap<String, Vec<String>> {
        &self.crate_dependencies
    }

    /// Re-derive the entries for a repository's current sources.
    ///
    /// `sources` is the full current path -> contents map, not just the
    /// changed files. A record whose source digest is unchanged is kept as
    /// stored (byte-identical rendering); a changed file is re-scanned; a
    /// record whose file no longer exists is dropped.
    pub fn refresh(&mut self, sources: &BTreeMap<String, String>) -> RefreshReport {
        let mut report = RefreshReport::default();
        let stale: Vec<String> = self
            .records
            .keys()
            .filter(|path| !sources.contains_key(*path))
            .cloned()
            .collect();
        for path in stale {
            self.records.remove(&path);
            report.removed.push(path);
        }
        for (path, contents) in sources {
            let digest = digest_of(contents);
            if let Some(record) = self.records.get(path) {
                if record.source_digest == digest {
                    report.unchanged.push(path.clone());
                    continue;
                }
            }
            self.record(scan_module(path, contents));
            report.refreshed.push(path.clone());
        }
        report
    }
}

/// A verification stage, narrowest to broadest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Targeted: the narrowest commands that exercise the changed paths.
    One,
    /// Module: the commands that cover the whole affected modules.
    Two,
    /// Repository: the whole-repository check.
    Three,
}

impl Stage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Stage::One => "one",
            Stage::Two => "two",
            Stage::Three => "three",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Stage::One => 0,
            Stage::Two => 1,
            Stage::Three => 2,
        }
    }

    pub fn from_index(index: usize) -> Self {
        match index {
            0 => Stage::One,
            1 => Stage::Two,
            _ => Stage::Three,
        }
    }
}

/// The staged commands for a set of changed paths.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StagePlan {
    stage1: Vec<String>,
    stage2: Vec<String>,
    stage3: Vec<String>,
}

impl StagePlan {
    /// Commands for one stage, in resolution order without duplicates.
    pub fn commands(&self, stage: Stage) -> &[String] {
        match stage {
            Stage::One => &self.stage1,
            Stage::Two => &self.stage2,
            Stage::Three => &self.stage3,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.stage1.is_empty() && self.stage2.is_empty() && self.stage3.is_empty()
    }

    fn add(&mut self, stage: Stage, command: &str) {
        let commands = match stage {
            Stage::One => &mut self.stage1,
            Stage::Two => &mut self.stage2,
            Stage::Three => &mut self.stage3,
        };
        if !commands.iter().any(|existing| existing == command) {
            commands.push(command.to_string());
        }
    }
}

/// Resolve changed source paths to the narrowest staged test commands.
///
/// A path present in the cache resolves from the cache's test map; anything
/// else falls back to the deterministic fixture mapping for its language.
pub fn resolve_stages(changed: &[&str], cache: &KnowledgeCache) -> StagePlan {
    let mut plan = StagePlan::default();
    let mut seen = BTreeSet::new();
    for path in changed {
        if !seen.insert(*path) {
            continue;
        }
        if let Some(record) = cache.get(path) {
            for (index, commands) in record.staged_commands().into_iter().enumerate() {
                let stage = Stage::from_index(index);
                for command in commands {
                    plan.add(stage, &command);
                }
            }
        } else {
            match language_for(path) {
                Language::Rust => match crate_package(path) {
                    Some(package) => {
                        let filter = rust_module_filter(path).or_else(|| rust_test_filter(path));
                        match filter {
                            Some(filter) => {
                                plan.add(Stage::One, &format!("cargo test -p {package} {filter}"))
                            }
                            None => plan.add(Stage::One, &format!("cargo test -p {package}")),
                        }
                        plan.add(Stage::Two, &format!("cargo test -p {package}"));
                        plan.add(Stage::Three, REPOSITORY_TEST_COMMAND);
                    }
                    None => plan.add(Stage::Three, REPOSITORY_TEST_COMMAND),
                },
                Language::Shell => {
                    let command = if path.ends_with(".bats") {
                        format!("bats {path}")
                    } else {
                        format!("bash -n {path}")
                    };
                    plan.add(Stage::One, &command);
                    plan.add(Stage::Three, REPOSITORY_TEST_COMMAND);
                }
                Language::Documentation => {
                    // Documentation has no narrower check than the
                    // repository validation.
                    plan.add(Stage::One, REPOSITORY_TEST_COMMAND);
                    plan.add(Stage::Three, REPOSITORY_TEST_COMMAND);
                }
                Language::Unknown => plan.add(Stage::Three, REPOSITORY_TEST_COMMAND),
            }
        }
    }
    // Widen Stage 2 to the reverse-dependency closure of the changed
    // crates (#3767): a patch that touches only a library crate can break
    // a dependent crate's tests, so scoping to the changed crates alone
    // could go green over a broken dependent. The changed crates' own
    // Stage 2 commands are already present and dedupe. The closure is the
    // same computation the attribution gate uses for blast radius.
    for crate_name in affected_test_crates(changed, cache) {
        plan.add(Stage::Two, &format!("cargo test -p {crate_name}"));
    }
    plan
}

/// The set of crates whose tests must run for a set of changed paths: the
/// changed crates plus every crate that depends on them, transitively —
/// the reverse-dependency closure (#3767).
///
/// The closure is computed by the same function the attribution gate uses
/// for its blast radius (`test_gate::reverse_closure`), so a change is
/// graded against the same set of crates its tests were scoped to. Non-Rust
/// paths contribute nothing: a patch that touches no compiled code cannot
/// break a compiled test by the dependency graph. With no graph data in the
/// cache the set is just the changed crates, and the Stage 3 repository
/// check covers the rest.
pub fn affected_test_crates(changed: &[&str], cache: &KnowledgeCache) -> BTreeSet<String> {
    let mut seed = BTreeSet::new();
    for path in changed {
        if language_for(path) != Language::Rust {
            continue;
        }
        if let Some(crate_name) = crate_package(path) {
            seed.insert(crate_name.to_string());
        }
    }
    reverse_closure(&seed, cache.crate_dependencies())
}

/// Outcome of a started stage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    #[default]
    Pending,
    Passed,
    Failed,
}

/// Which stages have passed so far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StageProgress {
    statuses: [StageStatus; 3],
}

impl StageProgress {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn status(&self, stage: Stage) -> StageStatus {
        self.statuses[stage.index()]
    }

    /// Record a stage's outcome; a failed stage may be re-run.
    pub fn record(&mut self, stage: Stage, passed: bool) {
        self.statuses[stage.index()] = if passed {
            StageStatus::Passed
        } else {
            StageStatus::Failed
        };
    }

    /// A stage may start only when every earlier stage has passed.
    pub fn allow(&self, stage: Stage) -> Result<(), String> {
        for earlier in 0..stage.index() {
            if self.statuses[earlier] != StageStatus::Passed {
                return Err(format!(
                    "stage {} cannot start before stage {} passes",
                    stage.index() + 1,
                    earlier + 1,
                ));
            }
        }
        Ok(())
    }

    /// The earliest stage that has not passed yet, or `None` once the whole
    /// plan has passed.
    pub fn next(&self) -> Option<Stage> {
        for index in 0..3 {
            if self.statuses[index] != StageStatus::Passed {
                return Some(Stage::from_index(index));
            }
        }
        None
    }
}

// --- fixture mapping internals -------------------------------------------------

fn crate_package(path: &str) -> Option<&str> {
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() < 2 || segments[0] != "crates" {
        return None;
    }
    let package = segments[1];
    if package.is_empty() || package == "." || package == ".." {
        return None;
    }
    Some(package)
}

/// `crates/<pkg>/src/a/b.rs` -> `a::b`; `mod.rs` names its directory;
/// `lib.rs` has no narrower filter.
fn rust_module_filter(path: &str) -> Option<String> {
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() < 4 || segments[0] != "crates" || segments[2] != "src" {
        return None;
    }
    let mut parts: Vec<&str> = segments[3..].to_vec();
    let stem = parts.pop()?.strip_suffix(".rs")?;
    if stem == "lib" {
        return None;
    }
    if stem != "mod" {
        parts.push(stem);
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("::"))
}

/// `crates/<pkg>/tests/<name>.rs` -> `<name>` (the integration test binary).
fn rust_test_filter(path: &str) -> Option<String> {
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() < 4 || segments[0] != "crates" || segments[2] != "tests" {
        return None;
    }
    let name = segments[3].strip_suffix(".rs")?;
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn rust_module_identity_and_commands(path: &str) -> (String, String, Vec<String>) {
    let Some(package) = crate_package(path) else {
        return (
            path.to_string(),
            REPOSITORY_TEST_COMMAND.to_string(),
            vec![REPOSITORY_TEST_COMMAND.to_string()],
        );
    };
    let filter = rust_module_filter(path).or_else(|| rust_test_filter(path));
    let module = match &filter {
        Some(filter) => format!("{package}::{filter}"),
        None => package.to_string(),
    };
    let crate_command = format!("cargo test -p {package}");
    let mut test_commands = match &filter {
        Some(filter) => vec![format!("cargo test -p {package} {filter}")],
        None => Vec::new(),
    };
    test_commands.push(crate_command);
    test_commands.push(REPOSITORY_TEST_COMMAND.to_string());
    (module, format!("cargo build -p {package}"), test_commands)
}

/// Sorted, de-duplicated.
fn dedup(items: Vec<String>) -> Vec<String> {
    items
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn identifier_chars(name: &str) -> String {
    name.chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

fn is_identifier(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

const RUST_ITEM_KEYWORDS: [&str; 14] = [
    "pub struct",
    "pub enum",
    "pub trait",
    "pub type",
    "pub const",
    "pub static",
    "struct",
    "enum",
    "trait",
    "type",
    "const",
    "static",
    "fn",
    "mod",
];

/// (symbols, in-repo dependencies, public interfaces) from Rust source.
fn scan_rust(contents: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut symbols = Vec::new();
    let mut dependencies = Vec::new();
    let mut interfaces = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("use ") {
            let dependency = rest.split(';').next().unwrap_or("").trim();
            if dependency.starts_with("crate::") || dependency.starts_with("super::") {
                dependencies.push(dependency.to_string());
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("pub fn ") {
            let name = identifier_chars(rest);
            if !name.is_empty() {
                symbols.push(name.clone());
                interfaces.push(name);
            }
            continue;
        }
        if let Some(keyword) = RUST_ITEM_KEYWORDS
            .iter()
            .find(|keyword| trimmed.starts_with(*keyword))
        {
            if let Some(name) = trimmed[keyword.len()..].split_whitespace().next() {
                let name = identifier_chars(name);
                if !name.is_empty() {
                    symbols.push(name);
                }
            }
        }
    }
    (dedup(symbols), dedup(dependencies), dedup(interfaces))
}

/// (functions, sourced files, long options) from shell source.
fn scan_shell(contents: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut symbols = Vec::new();
    let mut dependencies = Vec::new();
    let mut interfaces = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("function ") {
            if let Some(name) = rest.split_whitespace().next() {
                let name = identifier_chars(name);
                if !name.is_empty() {
                    symbols.push(name);
                }
            }
            continue;
        }
        if let Some(open) = trimmed.find("()") {
            let name = &trimmed[..open];
            if is_identifier(name) {
                symbols.push(name.to_string());
            }
        }
        if let Some(rest) = trimmed.strip_prefix("source ") {
            if let Some(dependency) = rest.split_whitespace().next() {
                dependencies.push(dependency.to_string());
            }
        }
        for token in trimmed.split_whitespace() {
            if let Some(flag) = token.strip_prefix("--") {
                if flag.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
                    interfaces.push(format!("--{flag}"));
                }
            }
        }
    }
    (dedup(symbols), dedup(dependencies), dedup(interfaces))
}

/// (headings, local link targets, headings) from markdown.
fn scan_documentation(contents: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut headings = Vec::new();
    let mut dependencies = Vec::new();
    for line in contents.lines() {
        if let Some(rest) = line.trim_start().strip_prefix('#') {
            let title = rest
                .trim_start_matches('#')
                .trim()
                .trim_end_matches('#')
                .trim();
            if !title.is_empty() {
                headings.push(title.to_string());
                continue;
            }
        }
        for (index, _) in line.match_indices("](") {
            let rest = &line[index + 2..];
            if let Some(close) = rest.find(')') {
                let target = rest[..close].trim();
                if !target.is_empty()
                    && !target.starts_with("http://")
                    && !target.starts_with("https://")
                    && !target.starts_with('#')
                {
                    dependencies.push(target.to_string());
                }
            }
        }
    }
    let headings = dedup(headings);
    (headings.clone(), dedup(dependencies), headings)
}
