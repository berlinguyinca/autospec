//! Language-scoped gate sets (#3928): a gate set is scoped to a language,
//! and a verdict must say so.
//!
//! A converted patch once passed the whole gate set — format clean, clippy
//! clean, 678 tests green — while every file it changed was TypeScript.
//! Every number in the report was true, and none of them was evidence about
//! the change: a Rust suite cannot read a `.tsx` file, and nothing in the
//! output said so. Absence of coverage rendered as presence of coverage,
//! and both came out green.
//!
//! The policy is encoded here as pure, testable primitives; callers run the
//! gates and act on the verdict these functions return:
//!
//! 1. **The gate set is derived from the file types the patch touches, via
//!    an explicit mapping — by mapping, not by memory**
//!    ([`GateScopeMapping::gate_set_for`]).
//! 2. **A verdict is refused when a changed path is read by no gate that
//!    ran.** The output is not a pass but an explicit
//!    `UNVERIFIED: <path> is not read by any gate in this run`
//!    ([`verdict`]).
//! 3. **A gate result states its scope.** A result records the file types
//!    its gate reads, and a verdict rendered beside a diff names the file
//!    types it covers — `678 passed` alone is a fact about the Rust
//!    workspace, not about a TypeScript diff ([`GateResult::line`],
//!    [`Verdict::Pass`]).
//! 4. **The mapping is repository configuration, and it is tested.** It
//!    lives in [`GATE_SCOPE_CONFIG_PATH`], adjacent to the gate
//!    definitions, one entry per file type — and the tests carry one case
//!    per file type, including the negative case that a patch touching only
//!    an unmapped type is reported unverified, never passed
//!    ([`GateScopeMapping::repository`]).

use std::collections::{BTreeSet, HashSet};
use std::str::FromStr;

use yaml_edit::{Document, Mapping, YamlNode};

/// The repository config file holding the file-type -> gates mapping.
pub const GATE_SCOPE_CONFIG_PATH: &str = "config/gate-scope.yml";

/// The config version this parser accepts.
pub const SUPPORTED_VERSION: &str = "1";

/// The mapping as shipped in the repository, embedded so the tested value
/// and the config file cannot drift apart.
const REPOSITORY_CONFIG: &str = include_str!("../../../../config/gate-scope.yml");

/// One entry of the mapping: a file-type label, the path criteria that
/// claim a path, and the gates that read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeEntry {
    /// The file-type label, e.g. `Rust`, `web`, `shell script`.
    pub file_type: String,
    /// Extension criterion, without dots and lower-case; empty = none.
    pub extensions: Vec<String>,
    /// Path-prefix criterion; `None` = none.
    pub path_prefix: Option<String>,
    /// The gates that read this file type.
    pub gates: Vec<String>,
}

impl ScopeEntry {
    /// A path is claimed by the entry when every stated criterion holds.
    /// An entry with no criteria claims nothing rather than everything.
    pub fn claims(&self, path: &str) -> bool {
        let has_criterion = !self.extensions.is_empty() || self.path_prefix.is_some();
        if !has_criterion {
            return false;
        }
        let normalized = path.replace('\\', "/");
        if let Some(prefix) = &self.path_prefix {
            if !normalized.starts_with(prefix.as_str()) {
                return false;
            }
        }
        if !self.extensions.is_empty() {
            let file = normalized.rsplit('/').next().unwrap_or(&normalized);
            let lower = file.to_ascii_lowercase();
            let Some(dot) = lower.rfind('.') else {
                return false;
            };
            let extension = &lower[dot + 1..];
            if !self.extensions.iter().any(|e| e == extension) {
                return false;
            }
        }
        true
    }
}

/// The file-type -> gates mapping: repository configuration, in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GateScopeMapping {
    entries: Vec<ScopeEntry>,
}

impl GateScopeMapping {
    /// A mapping with no entries: every path is unclaimed, so every verdict
    /// is refused. Fail-closed: an absent mapping is not a blanket pass.
    pub fn empty() -> Self {
        Self::default()
    }

    /// The mapping as configured in the repository
    /// ([`GATE_SCOPE_CONFIG_PATH`]).
    pub fn repository() -> Self {
        Self::parse(REPOSITORY_CONFIG)
            .expect("config/gate-scope.yml must parse: it is checked by the tests")
    }

    /// Parse the mapping from YAML.
    ///
    /// Strict: unknown keys, a missing criterion, an empty gate list, and a
    /// duplicated file-type label are all errors — a malformed mapping must
    /// fail, not silently claim less than it says it claims.
    pub fn parse(source: &str) -> Result<Self, String> {
        let document =
            Document::from_str(source).map_err(|error| format!("gate scope config: {error}"))?;
        let root = document
            .as_mapping()
            .ok_or_else(|| "gate scope config root must be a mapping".to_string())?;
        validate_keys(&root, &["version", "scopes"], "gate scope config")?;
        let version = required_scalar(&root, "version", "gate scope config version")?;
        if version != SUPPORTED_VERSION {
            return Err(format!("unsupported gate scope config version: {version}"));
        }
        let Some(scopes_node) = root.get("scopes") else {
            return Err("gate scope config requires a `scopes` list".to_string());
        };
        let scopes = scopes_node
            .as_sequence()
            .ok_or_else(|| "gate scope config `scopes` must be a list".to_string())?;
        if scopes.is_empty() {
            return Err("gate scope config `scopes` must not be empty".to_string());
        }
        let mut entries = Vec::new();
        let mut types = BTreeSet::new();
        for (index, node) in scopes.values().enumerate() {
            let entry = parse_entry(&node, index)?;
            if !types.insert(entry.file_type.clone()) {
                return Err(format!(
                    "duplicate gate scope file type: {}",
                    entry.file_type
                ));
            }
            entries.push(entry);
        }
        Ok(Self { entries })
    }

    /// Every entry, in config order (order is priority).
    pub fn entries(&self) -> &[ScopeEntry] {
        &self.entries
    }

    /// The first entry that claims the path; config order is priority.
    pub fn entry_for(&self, path: &str) -> Option<&ScopeEntry> {
        self.entries.iter().find(|entry| entry.claims(path))
    }

    /// The file type that claims the path, if any.
    pub fn file_type_of(&self, path: &str) -> Option<&str> {
        self.entry_for(path).map(|entry| entry.file_type.as_str())
    }

    /// The gates of the entry with the given file-type label, if any.
    pub fn gates_for_type(&self, file_type: &str) -> Option<&[String]> {
        self.entries
            .iter()
            .find(|entry| entry.file_type == file_type)
            .map(|entry| entry.gates.as_slice())
    }

    /// The file types whose entry lists the gate, sorted.
    pub fn types_read_by(&self, gate: &str) -> Vec<String> {
        let mut types: Vec<String> = self
            .entries
            .iter()
            .filter(|entry| entry.gates.iter().any(|g| g.as_str() == gate))
            .map(|entry| entry.file_type.clone())
            .collect();
        types.sort();
        types.dedup();
        types
    }

    /// The gate set for a patch: the union of the gates of the file types
    /// it touches — derived from the mapping, never from memory.
    pub fn gate_set_for(&self, changed: &[String]) -> BTreeSet<String> {
        let mut gates = BTreeSet::new();
        for path in changed {
            if let Some(entry) = self.entry_for(path) {
                gates.extend(entry.gates.iter().cloned());
            }
        }
        gates
    }

    /// The changed paths no entry claims, sorted.
    pub fn unclaimed(&self, changed: &[String]) -> Vec<String> {
        changed
            .iter()
            .filter(|path| self.entry_for(path.as_str()).is_none())
            .cloned()
            .collect()
    }
}

/// One executed gate's result, recording the file types its gate reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateResult {
    /// The gate that ran, as named in the mapping.
    pub gate: String,
    /// The file types this gate reads: its scope.
    pub scope: Vec<String>,
    /// Checks the gate ran that passed.
    pub passed: u64,
    /// Checks the gate ran that failed.
    pub failed: u64,
}

impl GateResult {
    /// A result whose scope is filled from the mapping, so the recorded
    /// scope and the configuration cannot drift apart.
    pub fn recorded(mapping: &GateScopeMapping, gate: &str, passed: u64, failed: u64) -> Self {
        Self {
            gate: gate.to_string(),
            scope: mapping.types_read_by(gate),
            passed,
            failed,
        }
    }

    /// The report line. The scope is part of the fact: `678 passed` beside
    /// a TypeScript diff, unqualified, is a misrepresentation.
    pub fn line(&self) -> String {
        let scope = if self.scope.is_empty() {
            "no mapped file type".to_string()
        } else {
            self.scope.join(", ")
        };
        format!(
            "{}: {} passed, {} failed (scope: {})",
            self.gate, self.passed, self.failed, scope
        )
    }
}

/// The decision for one patch, from the mapping, the changed paths, and the
/// gates that actually ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Every changed path is read by at least one executed gate. `covered`
    /// names the file types the executed gates cover — a verdict rendered
    /// beside a diff states which file types it covers.
    Pass { covered: Vec<String> },
    /// At least one changed path is read by no gate in this run. Never a
    /// pass, however green the gates that did run were.
    Unverified { paths: Vec<String> },
}

impl Verdict {
    /// Whether this verdict may be reported as a pass.
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass { .. })
    }

    /// The report line(s). An unverified path is named exactly as the
    /// output contract spells it — one line per path, no pass marker.
    pub fn line(&self) -> String {
        match self {
            Self::Pass { covered } => {
                let list = if covered.is_empty() {
                    "none".to_string()
                } else {
                    covered.join(", ")
                };
                format!("VERIFIED (covers: {list})")
            }
            Self::Unverified { paths } => paths
                .iter()
                .map(|path| format!("UNVERIFIED: {path} is not read by any gate in this run"))
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

/// Decide the patch: every changed path is covered only when at least one
/// gate from its mapping entry ran. A path no entry claims, and an entry
/// whose gates never ran, are both unverified — never a pass.
pub fn verdict(mapping: &GateScopeMapping, changed: &[String], executed: &[GateResult]) -> Verdict {
    let executed_gates: BTreeSet<&str> = executed.iter().map(|r| r.gate.as_str()).collect();
    let mut covered = BTreeSet::new();
    let mut unverified = BTreeSet::new();
    for path in changed {
        let covered_path = mapping.entry_for(path).is_some_and(|entry| {
            entry
                .gates
                .iter()
                .any(|gate| executed_gates.contains(gate.as_str()))
        });
        if covered_path {
            if let Some(entry) = mapping.entry_for(path) {
                covered.insert(entry.file_type.clone());
            }
        } else {
            unverified.insert(path.clone());
        }
    }
    if unverified.is_empty() {
        Verdict::Pass {
            covered: covered.into_iter().collect(),
        }
    } else {
        Verdict::Unverified {
            paths: unverified.into_iter().collect(),
        }
    }
}

fn parse_entry(node: &YamlNode, index: usize) -> Result<ScopeEntry, String> {
    let label = format!("scopes[{index}]");
    let mapping = node
        .as_mapping()
        .ok_or_else(|| format!("{label} must be a mapping"))?;
    validate_keys(mapping, &["type", "match", "gates"], &label)?;
    let file_type = required_scalar(mapping, "type", &format!("{label} type"))?;
    let gates = scalar_list(mapping, "gates", &format!("{label} gates"))?;
    if gates.is_empty() {
        return Err(format!("{label} must list at least one gate"));
    }
    let Some(match_node) = mapping.get("match") else {
        return Err(format!(
            "{label} requires a `match` with `extensions` and/or `path_prefix`"
        ));
    };
    let match_mapping = match_node
        .as_mapping()
        .ok_or_else(|| format!("{label} `match` must be a mapping"))?;
    validate_keys(
        match_mapping,
        &["extensions", "path_prefix"],
        &format!("{label} match"),
    )?;
    let raw_extensions = scalar_list(
        match_mapping,
        "extensions",
        &format!("{label} match extensions"),
    )?;
    let extensions: Vec<String> = raw_extensions
        .iter()
        .map(|raw| raw.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|extension| !extension.is_empty())
        .collect();
    let path_prefix = optional_scalar(
        match_mapping,
        "path_prefix",
        &format!("{label} match path_prefix"),
    )?;
    if extensions.is_empty() && path_prefix.is_none() {
        return Err(format!(
            "{label} `match` needs `extensions` and/or `path_prefix`: an entry \
             that claims no path is a typo, not a wildcard"
        ));
    }
    Ok(ScopeEntry {
        file_type,
        extensions,
        path_prefix,
        gates,
    })
}

fn required_scalar(mapping: &Mapping, key: &str, label: &str) -> Result<String, String> {
    optional_scalar(mapping, key, label)?.ok_or_else(|| format!("{label} is required"))
}

fn optional_scalar(mapping: &Mapping, key: &str, label: &str) -> Result<Option<String>, String> {
    let Some(node) = mapping.get(key) else {
        return Ok(None);
    };
    node.as_scalar()
        .map(|scalar| scalar.as_string())
        .map(|value| {
            if value.trim().is_empty() {
                None
            } else {
                Some(value)
            }
        })
        .ok_or_else(|| format!("{label} must be a string"))
}

fn scalar_list(mapping: &Mapping, key: &str, label: &str) -> Result<Vec<String>, String> {
    let Some(node) = mapping.get(key) else {
        return Ok(Vec::new());
    };
    let sequence = node
        .as_sequence()
        .ok_or_else(|| format!("{label} must be a list"))?;
    sequence
        .values()
        .map(|item| {
            item.as_scalar()
                .map(|scalar| scalar.as_string())
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| format!("{label} entries must be non-empty strings"))
        })
        .collect()
}

fn validate_keys(mapping: &Mapping, allowed: &[&str], label: &str) -> Result<(), String> {
    let mut seen = HashSet::new();
    for key in mapping.keys() {
        let key = key
            .as_scalar()
            .map(|scalar| scalar.as_string())
            .ok_or_else(|| format!("{label} key must be a string"))?;
        if !seen.insert(key.clone()) {
            return Err(format!("duplicate {label} key: {key}"));
        }
        if !allowed.contains(&key.as_str()) {
            return Err(format!("unknown key in {label}: {key}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant 4, per-type cases: every entry of the repository mapping
    /// claims one representative path and carries its gates.
    #[test]
    fn repository_mapping_claims_each_documented_file_type() {
        let mapping = GateScopeMapping::repository();
        assert_eq!(
            mapping.file_type_of("crates/autospec-core/src/lib.rs"),
            Some("Rust")
        );
        assert_eq!(
            mapping.gates_for_type("Rust"),
            Some(
                &[
                    "cargo fmt --all --check".to_string(),
                    "cargo clippy --workspace --all-targets -- -D warnings".to_string(),
                    "cargo test --workspace".to_string(),
                ][..]
            )
        );
        assert_eq!(mapping.file_type_of("apps/web/src/App.tsx"), Some("web"));
        assert_eq!(
            mapping.gates_for_type("web"),
            Some(
                &[
                    "npm run lint".to_string(),
                    "npm run typecheck".to_string(),
                    "npm test".to_string(),
                ][..]
            )
        );
        assert_eq!(
            mapping.file_type_of("scripts/lint-implementation.sh"),
            Some("shell script")
        );
        assert_eq!(
            mapping.gates_for_type("shell script"),
            Some(&["bash -n".to_string()][..])
        );
        assert_eq!(
            mapping.file_type_of("tests/unit/gate-scope.bats"),
            Some("bats")
        );
        assert_eq!(
            mapping.gates_for_type("bats"),
            Some(&["bats".to_string()][..])
        );
        assert_eq!(
            mapping.file_type_of(".github/workflows/rust.yml"),
            Some("workflow")
        );
        assert_eq!(
            mapping.gates_for_type("workflow"),
            Some(&["workflow parse".to_string()][..])
        );
    }

    /// Invariant 4, the negative unmapped-type case: paths no entry claims
    /// stay unclaimed — no wildcard, no closest-type fallback.
    #[test]
    fn repository_mapping_leaves_unmapped_types_unclaimed() {
        let mapping = GateScopeMapping::repository();
        assert_eq!(mapping.file_type_of("assets/logo.png"), None);
        assert_eq!(mapping.file_type_of("docs/notes.md"), None);
        assert_eq!(mapping.file_type_of("web/src/App.tsx"), None);
        assert_eq!(mapping.file_type_of("scripts/ci.yml"), None);
        assert_eq!(mapping.file_type_of("Cargo.toml"), None);
    }

    /// Config order is priority: the first claiming entry wins, so a
    /// specific prefix entry can shadow a generic extension entry.
    #[test]
    fn entry_order_is_priority() {
        let mapping = GateScopeMapping::parse(
            r"
version: 1
scopes:
  - type: web
    match:
      path_prefix: apps/web/
    gates: [npm test]
  - type: TypeScript
    match:
      extensions: [ts, tsx]
    gates: [tsc]
",
        )
        .expect("custom mapping parses");
        assert_eq!(mapping.file_type_of("apps/web/src/App.tsx"), Some("web"));
        assert_eq!(mapping.file_type_of("src/lib/util.ts"), Some("TypeScript"));
        assert_eq!(mapping.file_type_of("apps/web/vendor/lib.ts"), Some("web"));
    }

    /// Invariant 1: the gate set is derived from the touched types.
    #[test]
    fn gate_set_is_derived_from_touched_types() {
        let mapping = GateScopeMapping::repository();
        let rust_only = mapping.gate_set_for(&["crates/autospec-core/src/lib.rs".to_string()]);
        assert_eq!(
            rust_only,
            [
                "cargo fmt --all --check",
                "cargo clippy --workspace --all-targets -- -D warnings",
                "cargo test --workspace",
            ]
            .into_iter()
            .map(String::from)
            .collect::<BTreeSet<_>>()
        );
        let mixed = mapping.gate_set_for(&[
            "crates/autospec-core/src/lib.rs".to_string(),
            "scripts/lint-implementation.sh".to_string(),
            "tests/unit/gate-scope.bats".to_string(),
        ]);
        assert!(mixed.contains("cargo test --workspace"));
        assert!(mixed.contains("bash -n"));
        assert!(mixed.contains("bats"));
        assert!(!mixed.contains("npm test"));
        assert!(mapping
            .gate_set_for(&["assets/logo.png".to_string()])
            .is_empty());
    }

    /// Invariant 2, the negative case: a patch touching only an unmapped
    /// type is reported unverified, never passed — however many gates ran.
    #[test]
    fn unmapped_type_is_unverified_not_pass() {
        let mapping = GateScopeMapping::repository();
        let executed = [
            GateResult::recorded(&mapping, "cargo fmt --all --check", 1, 0),
            GateResult::recorded(&mapping, "cargo test --workspace", 678, 0),
        ];
        let verdict = verdict(&mapping, &["assets/logo.png".to_string()], &executed);
        assert!(!verdict.is_pass());
        assert_eq!(
            verdict,
            Verdict::Unverified {
                paths: vec!["assets/logo.png".to_string()]
            }
        );
        assert_eq!(
            verdict.line(),
            "UNVERIFIED: assets/logo.png is not read by any gate in this run"
        );
    }

    /// Acceptance (#3793, #3928): a TypeScript-only patch is not reported
    /// as passing the Rust gate set — the green 678 is evidence about the
    /// Rust workspace, and the output says so.
    #[test]
    fn typescript_only_patch_fails_closed_under_rust_gates() {
        let mapping = GateScopeMapping::repository();
        let changed = [
            "apps/web/src/agents/AgentPage.tsx",
            "apps/web/src/agents/AgentDetail.tsx",
            "apps/web/src/api/client.ts",
            "apps/web/src/hooks/useAgentRun.ts",
            "apps/web/src/lib/runReducer.ts",
            "apps/web/src/lib/runReducer.test.ts",
            "apps/web/src/pages/RunsPage.tsx",
        ]
        .map(|path| path.to_string());
        let executed = [
            GateResult::recorded(&mapping, "cargo fmt --all --check", 1, 0),
            GateResult::recorded(
                &mapping,
                "cargo clippy --workspace --all-targets -- -D warnings",
                1,
                0,
            ),
            GateResult::recorded(&mapping, "cargo test --workspace", 678, 0),
        ];
        let verdict = verdict(&mapping, &changed, &executed);
        assert!(!verdict.is_pass());
        if let Verdict::Unverified { paths } = &verdict {
            assert_eq!(paths.len(), 7);
            assert!(paths.contains(&"apps/web/src/agents/AgentPage.tsx".to_string()));
        } else {
            panic!("expected Unverified, got {verdict:?}");
        }
        for path in &changed {
            assert!(
                verdict.line().contains(&format!(
                    "UNVERIFIED: {path} is not read by any gate in this run"
                )),
                "missing UNVERIFIED line for {path} in:\n{}",
                verdict.line()
            );
        }
    }

    /// Acceptance (#3928): a shell-only patch is not reported as passing
    /// the format gate alone.
    #[test]
    fn shell_only_patch_fails_closed_under_format_gate() {
        let mapping = GateScopeMapping::repository();
        let executed = [GateResult::recorded(
            &mapping,
            "cargo fmt --all --check",
            1,
            0,
        )];
        let verdict = verdict(&mapping, &["scripts/ci-wait.sh".to_string()], &executed);
        assert!(!verdict.is_pass());
        assert_eq!(
            verdict.line(),
            "UNVERIFIED: scripts/ci-wait.sh is not read by any gate in this run"
        );
    }

    /// An executed gate that reads no mapped type covers nothing.
    #[test]
    fn gate_outside_mapping_covers_nothing() {
        let mapping = GateScopeMapping::repository();
        let executed = [GateResult::recorded(&mapping, "schema-gen --check", 1, 0)];
        let verdict = verdict(
            &mapping,
            &["crates/autospec-core/src/lib.rs".to_string()],
            &executed,
        );
        assert!(!verdict.is_pass());
    }

    /// Invariant 3: a passing verdict names the file types it covers.
    #[test]
    fn pass_verdict_names_the_file_types_it_covers() {
        let mapping = GateScopeMapping::repository();
        let changed = [
            "crates/autospec-core/src/lib.rs".to_string(),
            "scripts/lint-implementation.sh".to_string(),
        ];
        let executed: Vec<GateResult> = mapping
            .gate_set_for(&changed)
            .into_iter()
            .map(|gate| GateResult::recorded(&mapping, &gate, 1, 0))
            .collect();
        let verdict = verdict(&mapping, &changed, &executed);
        assert!(verdict.is_pass());
        assert_eq!(
            verdict,
            Verdict::Pass {
                covered: vec!["Rust".to_string(), "shell script".to_string()]
            }
        );
        assert_eq!(verdict.line(), "VERIFIED (covers: Rust, shell script)");
    }

    /// Invariant 3: a gate result states its scope — `678 passed` alone is
    /// never what gets printed.
    #[test]
    fn gate_result_line_states_its_scope() {
        let mapping = GateScopeMapping::repository();
        let result = GateResult::recorded(&mapping, "cargo test --workspace", 678, 0);
        assert_eq!(
            result.line(),
            "cargo test --workspace: 678 passed, 0 failed (scope: Rust)"
        );
        let unmapped = GateResult::recorded(&mapping, "schema-gen --check", 1, 0);
        assert_eq!(
            unmapped.line(),
            "schema-gen --check: 1 passed, 0 failed (scope: no mapped file type)"
        );
    }

    /// An empty patch is vacuously verified and says it covers none.
    #[test]
    fn empty_patch_is_vacuously_verified() {
        let mapping = GateScopeMapping::repository();
        let verdict = verdict(&mapping, &[], &[]);
        assert!(verdict.is_pass());
        assert_eq!(verdict.line(), "VERIFIED (covers: none)");
    }

    /// Fail-closed: with no mapping at all, every path is unverified.
    #[test]
    fn empty_mapping_refuses_everything() {
        let mapping = GateScopeMapping::empty();
        let verdict = verdict(
            &mapping,
            &["crates/autospec-core/src/lib.rs".to_string()],
            &[],
        );
        assert!(!verdict.is_pass());
        assert_eq!(
            verdict,
            Verdict::Unverified {
                paths: vec!["crates/autospec-core/src/lib.rs".to_string()]
            }
        );
    }

    /// Extension matching is case-insensitive and the config may spell
    /// extensions with or without a leading dot.
    #[test]
    fn extension_matching_is_case_insensitive_and_dot_optional() {
        let mapping = GateScopeMapping::parse(
            r"
version: 1
scopes:
  - type: Rust
    match:
      extensions: [.Rs]
    gates: [cargo test]
",
        )
        .expect("mapping parses");
        assert_eq!(mapping.file_type_of("crates/x/src/lib.rs"), Some("Rust"));
        assert_eq!(mapping.file_type_of("CRATES/x/SRC/LIB.RS"), Some("Rust"));
        assert_eq!(mapping.file_type_of("crates/x/src/lib.txt"), None);
    }

    /// A workflow entry ANDs its prefix and extension: a yml elsewhere is
    /// not a workflow.
    #[test]
    fn workflow_entry_requires_prefix_and_extension() {
        let mapping = GateScopeMapping::repository();
        assert_eq!(
            mapping.file_type_of(".github/workflows/rust.yaml"),
            Some("workflow")
        );
        assert_eq!(mapping.file_type_of("deploy/workflows/ci.yml"), None);
        assert_eq!(mapping.file_type_of(".github/workflows/README.md"), None);
    }

    /// The parser is strict: every malformed mapping fails, so a broken
    /// config cannot silently claim less than it says it claims.
    #[test]
    fn parse_rejects_malformed_mappings() {
        let cases = [
            ("version: 2\nscopes:\n  - type: Rust\n    match: { extensions: [rs] }\n    gates: [cargo test]\n", "unsupported gate scope config version: 2"),
            ("version: 1\nextra: true\nscopes:\n  - type: Rust\n    match: { extensions: [rs] }\n    gates: [cargo test]\n", "unknown key in gate scope config: extra"),
            ("version: 1\nscopes: []\n", "gate scope config `scopes` must not be empty"),
            ("version: 1\nscopes:\n  - type: Rust\n    match: { extensions: [rs] }\n    gates: []\n", "scopes[0] must list at least one gate"),
            ("version: 1\nscopes:\n  - type: Rust\n    gates: [cargo test]\n", "scopes[0] requires a `match`"),
            ("version: 1\nscopes:\n  - type: Rust\n    match: {}\n    gates: [cargo test]\n", "scopes[0] `match` needs `extensions` and/or `path_prefix`"),
            ("version: 1\nscopes:\n  - type: Rust\n    match: { extensions: [rs] }\n    gates: [cargo test]\n    surprise: true\n", "unknown key in scopes[0]: surprise"),
            (
                "version: 1\nscopes:\n  - type: Rust\n    match: { extensions: [rs] }\n    gates: [cargo test]\n  - type: Rust\n    match: { extensions: [rs] }\n    gates: [other]\n",
                "duplicate gate scope file type: Rust"
            ),
        ];
        for (source, expected) in cases {
            let error = GateScopeMapping::parse(source).expect_err("must reject");
            assert!(
                error.contains(expected),
                "error {error:?} should contain {expected:?}\nfor:\n{source}"
            );
        }
    }

    /// The embedded repository mapping and the file on disk cannot drift
    /// apart: parsing the file yields the same mapping as the embedding.
    #[test]
    fn repository_config_file_parses_cleanly() {
        let from_file = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/gate-scope.yml"),
        )
        .expect("config/gate-scope.yml must exist");
        assert_eq!(
            GateScopeMapping::parse(&from_file).expect("file parses"),
            GateScopeMapping::repository()
        );
    }
}
