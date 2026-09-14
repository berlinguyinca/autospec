//! The per-repository gate registry (issue #4556, ask 2).
//!
//! The gate set — the repo's base branch, its toolchain, and the exact full
//! gate argv — was a constant in the conversion code. That is how a pass
//! pointed at a repository with a different layout could silently gate its
//! patches with a gate that was never established for that repository: a
//! conversion that gates with the wrong command produces exactly the
//! merged-on-numbers-that-do-not-describe-it failure #4532 was filed for.
//!
//! The gate set is now data: one recorded entry per repository, loaded from a
//! JSON registry, and a repository with no recorded entry is refused rather
//! than guessed. Recording a new repository's gate is an operator act (the
//! entry names the base branch, the toolchain, and every stage's argv);
//! establishing what InferWeave's gate actually is is a research task that
//! must finish before an entry for it is written — until then the pass
//! refuses its patches, which is the correct state for a gate nobody has
//! established.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The token in a stage argv that expands to the pass's affected-scope tokens
/// (the packages the patch touched). A stage may contain it at most once; a
/// stage without it runs the same argv whatever the patch touches.
pub const SCOPE_PLACEHOLDER: &str = "@scope";

/// The registry file's schema version.
pub const REGISTRY_SCHEMA: u64 = 1;

/// The default registry location, relative to the repository checkout the
/// pass runs in: `data/convert-gate-registry.json`.
pub const REGISTRY_FILE_NAME: &str = "convert-gate-registry.json";

/// The environment variable that names an explicit registry file, below an
/// explicit `--gate-registry` flag and above the in-repository default.
pub const REGISTRY_ENV: &str = "AUTOSPEC_GATE_REGISTRY";

/// One repository's recorded gate: its base branch, its toolchain, and the
/// exact stage argvs, in the order the gate runs them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateSet {
    /// The branch the pass branches off and measures the base against.
    pub base_ref: String,
    /// The toolchain the gate pins to, recorded for the report. The pin
    /// itself is the repository's own `rust-toolchain.toml` (or the wrapper
    /// host's toolchain); this field is the record of what was established,
    /// not a second pin that could drift from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toolchain: Option<String>,
    /// The gate stages in order. Each is the exact argv after `cargo` (the
    /// gate always runs through `cargo`, as before); `@scope` expands to the
    /// pass's affected-scope tokens.
    pub stages: Vec<Vec<String>>,
}

/// The registry: one entry per repository, keyed by `owner/name`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GateRegistry {
    pub schema: u64,
    pub repos: BTreeMap<String, GateSet>,
}

/// The registry path for a pass: an explicit path (the `--gate-registry`
/// flag), else `$AUTOSPEC_GATE_REGISTRY`, else the in-repository default
/// under the checkout the pass runs in.
pub fn resolve_registry_path(explicit: Option<&Path>, env: Option<&str>, cwd: &Path) -> PathBuf {
    if let Some(path) = explicit {
        return path.to_path_buf();
    }
    if let Some(raw) = env {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    cwd.join("data").join(REGISTRY_FILE_NAME)
}

impl GateRegistry {
    /// Load and validate a registry file. Every failure is a named error:
    /// the pass refuses a gate it cannot prove was recorded, and a registry
    /// that fails to parse is not evidence that any repository's gate was
    /// established.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("no gate registry at {}: {error}", path.display()))?;
        let registry: GateRegistry = serde_json::from_str(&text).map_err(|error| {
            format!(
                "gate registry {} is not valid JSON: {error}",
                path.display()
            )
        })?;
        if registry.schema != REGISTRY_SCHEMA {
            return Err(format!(
                "gate registry {} has schema {}, expected {REGISTRY_SCHEMA}",
                path.display(),
                registry.schema
            ));
        }
        if registry.repos.is_empty() {
            return Err(format!(
                "gate registry {} records no repositories",
                path.display()
            ));
        }
        for (repo, gate) in &registry.repos {
            validate_repo_key(repo)?;
            validate_gate(gate)?;
        }
        Ok(registry)
    }

    /// The recorded gate for a repository, or `None` when the registry does
    /// not name it. `None` is the refusal: the pass does not guess a gate.
    pub fn lookup(&self, repo: &str) -> Option<&GateSet> {
        self.repos.get(repo)
    }
}

fn validate_repo_key(repo: &str) -> Result<(), String> {
    let mut parts = repo.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    if owner.is_empty() || name.is_empty() || parts.next().is_some() {
        return Err(format!(
            "gate registry records '{repo}': repository keys are 'owner/name'"
        ));
    }
    Ok(())
}

fn validate_gate(gate: &GateSet) -> Result<(), String> {
    if gate.base_ref.trim().is_empty() {
        return Err("a recorded gate must name its base branch".to_string());
    }
    if gate.stages.is_empty() {
        return Err("a recorded gate must name at least one stage".to_string());
    }
    for stage in &gate.stages {
        let Some(program) = stage.first() else {
            return Err("a recorded gate stage must be a non-empty argv".to_string());
        };
        if program.is_empty() || program.starts_with('-') {
            return Err(format!(
                "a recorded gate stage must begin with its command, got {program:?}"
            ));
        }
        if stage.iter().filter(|arg| *arg == SCOPE_PLACEHOLDER).count() > 1 {
            return Err(format!(
                "a recorded gate stage may contain {SCOPE_PLACEHOLDER} at most once"
            ));
        }
    }
    Ok(())
}

/// Expand a stage's recorded argv against the pass's affected scope:
/// `@scope` becomes the scope tokens (the packages the patch touched). A
/// stage without the placeholder is returned unchanged.
pub fn expand_scope(template: &[String], packages: &[String]) -> Vec<String> {
    if !template.iter().any(|arg| arg == SCOPE_PLACEHOLDER) {
        return template.to_vec();
    }
    let mut out: Vec<String> = Vec::with_capacity(template.len() + packages.len());
    for arg in template {
        if arg == SCOPE_PLACEHOLDER {
            out.extend_from_slice(packages);
        } else {
            out.push(arg.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry_json() -> String {
        r#"{
            "schema": 1,
            "repos": {
                "berlinguyinca/autospec": {
                    "base_ref": "main",
                    "toolchain": "pinned-by-repository",
                    "stages": [
                        ["fmt", "--check"],
                        ["build", "@scope"],
                        ["clippy", "--all-targets", "@scope"],
                        ["test", "--no-fail-fast", "@scope"]
                    ]
                }
            }
        }"#
        .to_string()
    }

    #[test]
    fn a_recorded_registry_loads_and_looks_up_its_repository() {
        let dir = std::env::temp_dir().join(format!(
            "autospec-gate-registry-load-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("registry.json");
        std::fs::write(&path, registry_json()).expect("write");
        let registry = GateRegistry::load(&path).expect("loads");
        let gate = registry.lookup("berlinguyinca/autospec").expect("entry");
        assert_eq!(gate.base_ref, "main");
        assert_eq!(gate.stages.len(), 4);
        assert!(registry.lookup("someone/else").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_registry_is_a_named_error() {
        let error = GateRegistry::load(Path::new("/nonexistent/registry.json")).unwrap_err();
        assert!(error.starts_with("no gate registry at"), "{error}");
    }

    #[test]
    fn scope_expansion_substitutes_the_tokens() {
        let template: Vec<String> = vec!["build".into(), SCOPE_PLACEHOLDER.into()];
        let packages: Vec<String> = vec!["-p".into(), "autospec-core".into()];
        assert_eq!(
            expand_scope(&template, &packages),
            vec!["build", "-p", "autospec-core"]
        );
    }

    #[test]
    fn a_stage_without_scope_is_unchanged() {
        let template: Vec<String> = vec!["fmt".into(), "--check".into()];
        let packages: Vec<String> = vec!["-p".into(), "x".into()];
        assert_eq!(expand_scope(&template, &packages), vec!["fmt", "--check"]);
    }

    #[test]
    fn an_empty_scope_expands_to_nothing_not_a_placeholder() {
        let template: Vec<String> = vec!["build".into(), SCOPE_PLACEHOLDER.into()];
        assert_eq!(expand_scope(&template, &[]), vec!["build"]);
    }

    #[test]
    fn validation_refuses_malformed_entries() {
        let dir =
            std::env::temp_dir().join(format!("autospec-gate-registry-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");

        let bad_key =
            r#"{"schema":1,"repos":{"not-a-repo":{"base_ref":"main","stages":[["test"]]}}}"#;
        let path = dir.join("key.json");
        std::fs::write(&path, bad_key).expect("write");
        assert!(GateRegistry::load(&path).is_err(), "owner/name required");

        let no_stages = r#"{"schema":1,"repos":{"a/b":{"base_ref":"main","stages":[]}}}"#;
        let path = dir.join("nostages.json");
        std::fs::write(&path, no_stages).expect("write");
        assert!(GateRegistry::load(&path).is_err(), "at least one stage");

        let flag_first = r#"{"schema":1,"repos":{"a/b":{"base_ref":"main","stages":[["--all"]]}}}"#;
        let path = dir.join("flag.json");
        std::fs::write(&path, flag_first).expect("write");
        assert!(
            GateRegistry::load(&path).is_err(),
            "stage begins with a command"
        );

        let double_scope = r#"{"schema":1,"repos":{"a/b":{"base_ref":"main","stages":[["@scope","x","@scope"]]}}}"#;
        let path = dir.join("twice.json");
        std::fs::write(&path, double_scope).expect("write");
        assert!(GateRegistry::load(&path).is_err(), "one @scope per stage");

        let wrong_schema =
            r#"{"schema":2,"repos":{"a/b":{"base_ref":"main","stages":[["test"]]}}}"#;
        let path = dir.join("schema.json");
        std::fs::write(&path, wrong_schema).expect("write");
        assert!(GateRegistry::load(&path).is_err(), "schema must match");

        let empty = r#"{"schema":1,"repos":{}}"#;
        let path = dir.join("empty.json");
        std::fs::write(&path, empty).expect("write");
        assert!(
            GateRegistry::load(&path).is_err(),
            "at least one repository"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_registry_path_prefers_flag_then_env_then_default() {
        let cwd = Path::new("/checkout");
        assert_eq!(
            resolve_registry_path(Some(Path::new("/flag.json")), Some("/env.json"), cwd),
            PathBuf::from("/flag.json")
        );
        assert_eq!(
            resolve_registry_path(None, Some("/env.json"), cwd),
            PathBuf::from("/env.json")
        );
        assert_eq!(
            resolve_registry_path(None, Some("  "), cwd),
            cwd.join("data").join(REGISTRY_FILE_NAME)
        );
        assert_eq!(
            resolve_registry_path(None, None, cwd),
            cwd.join("data").join(REGISTRY_FILE_NAME)
        );
    }
}
