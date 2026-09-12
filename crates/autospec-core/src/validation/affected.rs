use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffectedRule {
    pub check: String,
    pub reason: String,
}

impl AffectedRule {
    fn new(check: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            check: check.into(),
            reason: reason.into(),
        }
    }
}

/// The gate set a change selects: which declared gates cover the paths it
/// touches, and which paths no declared gate covers at all.
///
/// The gate set is a *function of the changed paths* (#3790). The declared
/// coverage is exactly the `is_*_input` predicates below; a path that matches
/// none of them is reported in `ungated_paths`, never folded into a default
/// gate. A TypeScript-only patch passing the Rust suite was not a weak verdict
/// — it was not a verdict at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffectedSet {
    pub changed_paths: Vec<String>,
    pub rules: Vec<AffectedRule>,
    pub ungated_paths: Vec<String>,
}

impl AffectedSet {
    pub fn from_paths(paths: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        let changed_paths = paths
            .into_iter()
            .map(|path| path.as_ref().replace('\\', "/"))
            .filter(|path| !path.is_empty())
            .collect::<Vec<_>>();

        let mut checks = BTreeSet::new();
        let mut rules = Vec::new();
        let mut ungated_paths = Vec::new();

        for path in &changed_paths {
            match Self::rule_for(path) {
                Some(rule) => {
                    push_rule(&mut checks, &mut rules, rule.check, rule.reason);
                }
                None => {
                    // No declared gate covers this path. Report it as ungated
                    // rather than letting some other toolchain's gate set
                    // stand in for it.
                    ungated_paths.push(path.clone());
                }
            }
        }

        Self {
            changed_paths,
            rules,
            ungated_paths,
        }
    }

    /// The single declared gate for a normalized path, in priority order
    /// shared > skill > rust > docs. `None` means no declared gate covers the
    /// path: it must be reported as ungated, never folded into a default.
    fn rule_for(path: &str) -> Option<AffectedRule> {
        if is_shared_input(path) {
            return Some(AffectedRule::new(
                "always-run",
                "shared validation input changed",
            ));
        }
        if let Some(skill_name) = skill_name_for_path(path) {
            return Some(AffectedRule::new(
                format!("skill:{skill_name}"),
                "skill-scoped validation input changed",
            ));
        }
        if is_rust_input(path) {
            return Some(AffectedRule::new(
                "rust:lint",
                "Rust source or manifest changed",
            ));
        }
        if is_docs_input(path) {
            return Some(AffectedRule::new("docs", "documentation input changed"));
        }
        None
    }

    pub fn checks(&self) -> Vec<&str> {
        self.rules.iter().map(|rule| rule.check.as_str()).collect()
    }

    pub fn includes_check(&self, check: &str) -> bool {
        self.rules.iter().any(|rule| rule.check == check)
    }

    /// Whether any changed path has no declared gate covering it.
    ///
    /// An ungated set may never be reported as validated: the checks that do
    /// run cover a different scope than the change made.
    pub fn has_ungated(&self) -> bool {
        !self.ungated_paths.is_empty()
    }
}

fn push_rule(
    checks: &mut BTreeSet<String>,
    rules: &mut Vec<AffectedRule>,
    check: impl Into<String>,
    reason: impl Into<String>,
) {
    let check = check.into();
    if checks.insert(check.clone()) {
        rules.push(AffectedRule::new(check, reason));
    }
}

fn is_shared_input(path: &str) -> bool {
    matches!(path, "AGENTS.md" | "scripts/expand-skill-blocks.sh")
        || path.starts_with("scripts/lib/")
        || path.starts_with("crates/autospec-core/src/validation/")
}

fn skill_name_for_path(path: &str) -> Option<&str> {
    if let Some(rest) = path.strip_prefix("skills/") {
        return rest.split('/').next().filter(|name| !name.is_empty());
    }

    path.strip_prefix("tests/fixtures/skill-goldens/")
        .and_then(|rest| rest.split('.').next())
        .filter(|name| !name.is_empty())
}

fn is_rust_input(path: &str) -> bool {
    path.ends_with(".rs")
        || path == "Cargo.toml"
        || path.ends_with("/Cargo.toml")
        || path == "Cargo.lock"
}

fn is_docs_input(path: &str) -> bool {
    path.starts_with("docs/") || (!path.contains('/') && path.ends_with(".md"))
}
