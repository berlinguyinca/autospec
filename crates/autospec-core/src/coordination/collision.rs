//! Predict file collisions before dispatching a parallel batch (issue #3564).
//!
//! Parallel dispatch does not remove integration cost, it moves it into a
//! phase that cannot be parallelised. The overlap that decides whether the
//! trade is worth it can be estimated up front, from three inputs:
//!
//! 1. the issue text — file paths, `path:line` references, and `dir/`
//!    references it cites;
//! 2. the repo's own signals — a file flagged as a conflict hotspot in
//!    `AGENTS.md`/`CONTRIBUTING`, or one appearing in a high share of recent
//!    commits (supplied through [`CommitHistory`, a VCS-agnostic interface
//!    with no GitHub or git-host assumption]);
//! 3. history across batches — [`HotspotLedger`], which turns a file that
//!    keeps colliding into a refactoring suggestion instead of another
//!    resolved-by-hand conflict.
//!
//! Predicted colliders are serialised into later dispatch waves ("prefer
//! breadth": issues touching disjoint areas travel together), and every
//! hotspot is named in a warning with its exact count. A batch with no
//! predicted overlap yields a single wave and is dispatched fully in
//! parallel, unchanged.
//!
//! The planner stays pure: callers supply issue texts and repo signals, this
//! module touches no I/O.

use std::collections::{BTreeMap, BTreeSet};

use crate::state::json::{JsonParser, JsonValue};

/// A file touched by at least this share of recent commits is a statistical
/// hotspot even when the repo never declared it.
pub const STATISTICAL_HOTSPOT_SHARE: f64 = 0.4;

/// Maximum batch entries retained by a [`HotspotLedger`].
pub const LEDGER_MAX_BATCHES: usize = 32;

/// A file that was a collision hotspot in at least this many distinct
/// batches is reported as a refactoring suggestion.
pub const REFACTOR_SUGGESTION_MIN_BATCHES: usize = 2;

const KNOWN_SOURCE_EXTENSIONS: &[&str] = &[
    "rs",
    "go",
    "py",
    "ts",
    "tsx",
    "mts",
    "cts",
    "js",
    "jsx",
    "mjs",
    "cjs",
    "java",
    "kt",
    "kts",
    "scala",
    "sc",
    "rb",
    "php",
    "c",
    "h",
    "cc",
    "cpp",
    "cxx",
    "hh",
    "hpp",
    "cs",
    "swift",
    "sh",
    "bash",
    "zsh",
    "fish",
    "ps1",
    "bat",
    "cmd",
    "sql",
    "proto",
    "graphql",
    "tf",
    "bats",
    "toml",
    "yaml",
    "yml",
    "json",
    "json5",
    "jsonc",
    "md",
    "mdx",
    "rst",
    "adoc",
    "txt",
    "lock",
    "nix",
    "gradle",
    "sbt",
    "cabal",
    "cmake",
    "mk",
    "dockerfile",
    "perl",
    "pl",
    "lua",
    "r",
    "zig",
    "nim",
    "ex",
    "exs",
    "erl",
    "hrl",
    "hs",
    "ml",
    "mli",
    "vue",
    "svelte",
    "sass",
    "scss",
    "less",
    "html",
    "xml",
    "ini",
    "cfg",
    "conf",
    "properties",
];

/// Version-control input for hotspot estimation. A `git` repository, a GitHub
/// API dump, or a test fixture can all implement it; the planner never
/// assumes which VCS or host produced the history.
pub trait CommitHistory {
    /// Files touched by each of the most recent commits, newest first. One
    /// entry per commit (an empty entry for a commit with no file changes).
    fn touched_files_per_commit(&self, max_commits: usize) -> Vec<Vec<String>>;
}

/// Compute each file's share of recent commits (commits touching the file /
/// total commits examined).
pub fn commit_shares(history: &dyn CommitHistory, max_commits: usize) -> BTreeMap<String, f64> {
    let commits = history.touched_files_per_commit(max_commits);
    if commits.is_empty() {
        return BTreeMap::new();
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for commit in &commits {
        let deduplicated: BTreeSet<&String> = commit.iter().collect();
        for file in deduplicated {
            *counts.entry(file.clone()).or_insert(0) += 1;
        }
    }
    let total = commits.len();
    counts
        .into_iter()
        .map(|(file, count)| (file, count as f64 / total as f64))
        .collect()
}

/// Repo-declared conflict hotspots: every path cited on a line of a governance
/// document (`AGENTS.md`, `CONTRIBUTING`) that talks about conflicts or
/// hotspots — e.g. "a large edit to `cmd/gateway/main.go` will conflict".
pub fn parse_declared_hotspots(document: &str) -> BTreeSet<String> {
    let mut hotspots = BTreeSet::new();
    for line in document.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("conflict") || lower.contains("hotspot") {
            hotspots.extend(extract_path_tokens(line));
        }
    }
    hotspots
}

/// Estimate the set of files an issue is likely to touch from its text:
/// path-shaped tokens (`src/x.rs`, `cmd/gateway/`), `path:line` references,
/// and bare `name.ext` files. Directories are kept with their trailing slash
/// and overlap everything beneath them.
pub fn estimate_touch_set(text: &str) -> BTreeSet<String> {
    extract_path_tokens(text)
}

fn extract_path_tokens(text: &str) -> BTreeSet<String> {
    let mut tokens = BTreeSet::new();
    for raw in text.split(|c: char| {
        !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '#'))
    }) {
        if let Some(path) = normalize_path_token(raw) {
            tokens.insert(path);
        }
    }
    tokens
}

fn normalize_path_token(raw: &str) -> Option<String> {
    let token = raw
        .trim_start_matches('#')
        .trim_end_matches(['.', ':', '-', '_']);
    if token.is_empty() || token.contains('*') {
        return None;
    }
    // Strip `path:line` (and `path:line:column`) suffixes; anything else with
    // a colon left over (URLs, ratios, timestamps) is not a path.
    let mut token = token;
    while let Some((head, tail)) = token.rsplit_once(':') {
        if tail.is_empty() || !tail.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        token = head;
    }
    let token = token.strip_prefix("./").unwrap_or(token);
    if token.is_empty() {
        return None;
    }
    if token.contains('/') {
        let is_dir = token.ends_with('/');
        let body = token.trim_end_matches('/');
        let mut segments = body.split('/');
        let first = segments.next().unwrap_or_default();
        let last = body.rsplit('/').next().unwrap_or_default();
        if first.is_empty()
            || first.contains('.')
            || body.split('/').any(|segment| segment.is_empty())
        {
            return None;
        }
        // Prose like "and/or" or a bare package path with neither a file
        // extension nor a trailing slash is not evidence of a touch. Repo
        // convention declares directories with a trailing slash.
        if !is_dir && !last.contains('.') {
            return None;
        }
        return Some(token.to_string());
    }
    let (_, extension) = token.rsplit_once('.')?;
    let has_known_extension = KNOWN_SOURCE_EXTENSIONS
        .iter()
        .any(|known| known.eq_ignore_ascii_case(extension));
    has_known_extension.then(|| token.to_string())
}

/// Repo-derived inputs to the collision estimate.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RepoSignals {
    /// Paths declared as conflict hotspots in governance documents.
    pub declared_hotspots: BTreeSet<String>,
    /// Per-file share of recent commits, from [`commit_shares`].
    pub commit_share: BTreeMap<String, f64>,
}

impl RepoSignals {
    /// Declared hotspots plus statistically hot files (a high share of recent
    /// commits).
    pub fn hotspots(&self) -> BTreeSet<String> {
        let mut hotspots = self.declared_hotspots.clone();
        for (file, share) in &self.commit_share {
            if *share >= STATISTICAL_HOTSPOT_SHARE {
                hotspots.insert(file.clone());
            }
        }
        hotspots
    }
}

/// An issue referencing a hotspot's parent directory (e.g. `cmd/gateway/`)
/// is predicted to touch the hotspot file itself, not just the directory.
fn expand_with_hotspots(paths: &BTreeSet<String>, hotspots: &BTreeSet<String>) -> BTreeSet<String> {
    let mut expanded = paths.clone();
    for hotspot in hotspots {
        if paths.iter().any(|path| path_overlaps(path, hotspot)) {
            expanded.insert(hotspot.clone());
        }
    }
    expanded
}

/// True when two touch-set entries can land on the same file: equal paths, or
/// a directory entry covering the other path.
fn path_overlaps(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match (a.ends_with('/'), b.ends_with('/')) {
        (true, true) => a.starts_with(b) || b.starts_with(a),
        (true, false) => b.starts_with(a),
        (false, true) => a.starts_with(b),
        (false, false) => false,
    }
}

fn sets_overlap(a: &BTreeSet<String>, b: &BTreeSet<String>) -> bool {
    a.iter()
        .any(|path| b.iter().any(|other| path_overlaps(path, other)))
}

/// A named, counted hotspot warning for the batch dispatch log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollisionWarning {
    pub path: String,
    pub issue_count: usize,
    pub batch_size: usize,
    pub message: String,
}

/// A refactoring suggestion raised by a hotspot that persists across batches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefactorSuggestion {
    pub path: String,
    pub batch_count: usize,
    pub message: String,
}

/// The dispatch-time collision verdict for one batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollisionPlan {
    /// Dispatch order: issues within a wave are predicted disjoint and may
    /// run in parallel; a collider waits for the wave covering its overlap.
    pub waves: Vec<Vec<u64>>,
    /// Hotspot warnings, each naming the count and the file.
    pub warnings: Vec<CollisionWarning>,
    /// Files predicted to be touched by two or more issues in the batch.
    pub colliding_files: Vec<String>,
}

impl CollisionPlan {
    /// True when nothing is predicted to collide and the batch dispatches
    /// fully in parallel, unchanged.
    pub fn is_fully_parallel(&self) -> bool {
        self.waves.len() <= 1
    }
}

/// Estimate per-issue touch sets and order the batch into dispatch waves
/// before anything runs. `issues` are `(number, text)` pairs in dispatch
/// order; text is typically the issue title plus body.
pub fn predict_collisions(issues: &[(u64, &str)], signals: &RepoSignals) -> CollisionPlan {
    let hotspots = signals.hotspots();
    let estimates: Vec<(u64, BTreeSet<String>)> = issues
        .iter()
        .map(|(number, text)| {
            (
                *number,
                expand_with_hotspots(&estimate_touch_set(text), &hotspots),
            )
        })
        .collect();

    let batch_size = issues.len();
    let mut path_counts: BTreeMap<String, usize> = BTreeMap::new();
    for (_, paths) in &estimates {
        for path in paths {
            if path_counts.contains_key(path) {
                continue;
            }
            let count = estimates
                .iter()
                .filter(|(_, other)| other.iter().any(|entry| path_overlaps(entry, path)))
                .count();
            path_counts.insert(path.clone(), count);
        }
    }

    let mut colliding: Vec<(String, usize)> = path_counts
        .into_iter()
        .filter(|(_, count)| *count >= 2)
        .collect();
    // A directory whose predicted collision is fully explained by a more
    // specific file inside it earns no separate warning: name the file.
    let candidates = colliding.clone();
    colliding.retain(|(path, count)| {
        !path.ends_with('/')
            || !candidates.iter().any(|(other_path, other_count)| {
                other_path != path
                    && !other_path.ends_with('/')
                    && other_path.starts_with(path)
                    && other_count >= count
            })
    });
    colliding.sort_by(|(path_a, count_a), (path_b, count_b)| {
        count_b.cmp(count_a).then_with(|| path_a.cmp(path_b))
    });
    let warnings = colliding
        .iter()
        .map(|(path, count)| CollisionWarning {
            path: path.clone(),
            issue_count: *count,
            batch_size,
            message: format!(
                "{count} of {batch_size} issues are likely to touch {path}; consider serialising or splitting that file first"
            ),
        })
        .collect();
    let colliding_files = colliding
        .iter()
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();

    // Prefer breadth: greedy first-fit keeps a parallel wave wide and queues
    // predicted colliders behind it instead of paying for rebases later.
    let mut waves: Vec<(Vec<u64>, BTreeSet<String>)> = Vec::new();
    for (number, paths) in &estimates {
        match waves
            .iter_mut()
            .find(|(_, union)| !sets_overlap(union, paths))
        {
            Some(wave) => {
                wave.0.push(*number);
                wave.1.extend(paths.iter().cloned());
            }
            None => waves.push((vec![*number], paths.clone())),
        }
    }

    CollisionPlan {
        waves: waves.into_iter().map(|(numbers, _)| numbers).collect(),
        warnings,
        colliding_files,
    }
}

/// One recorded batch: its dispatch signature and the files predicted to
/// collide in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerEntry {
    pub signature: String,
    pub hotspots: Vec<String>,
}

/// Cross-batch hotspot history. Recording the same batch signature again
/// (e.g. a re-polled `queue ready`) does not inflate hotspot counts; only
/// distinct batches do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HotspotLedger {
    pub entries: Vec<LedgerEntry>,
}

impl HotspotLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, signature: &str, hotspots: &[String]) {
        if signature.is_empty() {
            return;
        }
        match self
            .entries
            .iter_mut()
            .find(|entry| entry.signature == signature)
        {
            Some(entry) => entry.hotspots = hotspots.to_vec(),
            None => {
                self.entries.push(LedgerEntry {
                    signature: signature.to_string(),
                    hotspots: hotspots.to_vec(),
                });
                if self.entries.len() > LEDGER_MAX_BATCHES {
                    self.entries.remove(0);
                }
            }
        }
    }

    /// Distinct batches in which each file was a collision hotspot.
    pub fn hotspot_batch_counts(&self) -> BTreeMap<String, usize> {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for entry in &self.entries {
            let deduplicated: BTreeSet<&String> = entry.hotspots.iter().collect();
            for path in deduplicated {
                *counts.entry(path.clone()).or_insert(0) += 1;
            }
        }
        counts
    }

    /// Files that keep colliding across batches: a persistent hotspot is a
    /// refactoring signal, more valuable than resolving the same conflict
    /// every batch.
    pub fn suggestions(&self) -> Vec<RefactorSuggestion> {
        let mut persistent: Vec<(String, usize)> = self
            .hotspot_batch_counts()
            .into_iter()
            .filter(|(_, count)| *count >= REFACTOR_SUGGESTION_MIN_BATCHES)
            .collect();
        persistent.sort_by(|(path_a, count_a), (path_b, count_b)| {
            count_b.cmp(count_a).then_with(|| path_a.cmp(path_b))
        });
        persistent
            .into_iter()
            .map(|(path, batch_count)| RefactorSuggestion {
                batch_count,
                message: format!(
                    "{path} has been a collision hotspot in {batch_count} batches; consider splitting this file before dispatching more parallel work at it"
                ),
                path,
            })
            .collect()
    }

    pub fn to_json(&self) -> String {
        let entries = self
            .entries
            .iter()
            .map(|entry| {
                let hotspots = entry
                    .hotspots
                    .iter()
                    .map(|path| json_escape(path))
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    "{{\"signature\":{},\"hotspots\":[{}]}}",
                    json_escape(&entry.signature),
                    hotspots
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!("{{\"batches\":[{entries}]}}")
    }

    pub fn from_json(input: &str) -> Result<Self, String> {
        let context = "collision ledger";
        let mut object = JsonParser::new(input).parse()?.into_object(context)?;
        let unknown = object.keys().find(|key| key.as_str() != "batches").cloned();
        if let Some(key) = unknown {
            return Err(format!("{context}.{key} is unknown"));
        }
        let batches = object
            .remove("batches")
            .ok_or_else(|| format!("{context}.batches is required"))?
            .into_array(&format!("{context}.batches"))?;
        let mut entries = Vec::new();
        for (index, value) in batches.into_iter().enumerate() {
            let entry_context = format!("{context}.batches[{index}]");
            let mut entry = value.into_object(&entry_context)?;
            let unknown = entry
                .keys()
                .find(|key| !matches!(key.as_str(), "signature" | "hotspots"))
                .cloned();
            if let Some(key) = unknown {
                return Err(format!("{entry_context}.{key} is unknown"));
            }
            let signature = entry
                .remove("signature")
                .ok_or_else(|| format!("{entry_context}.signature is required"))?
                .into_string(&format!("{entry_context}.signature"))?;
            let hotspots = entry
                .remove("hotspots")
                .unwrap_or(JsonValue::Array(Vec::new()))
                .into_array(&format!("{entry_context}.hotspots"))?
                .into_iter()
                .enumerate()
                .map(|(path_index, path)| {
                    let path_context = format!("{entry_context}.hotspots[{path_index}]");
                    path.into_string(&path_context)
                })
                .collect::<Result<Vec<_>, String>>()?;
            entries.push(LedgerEntry {
                signature,
                hotspots,
            });
        }
        Ok(Self { entries })
    }
}

fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
