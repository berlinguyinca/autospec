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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffectedSet {
    pub changed_paths: Vec<String>,
    pub rules: Vec<AffectedRule>,
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

        for path in &changed_paths {
            if is_shared_input(path) {
                push_rule(
                    &mut checks,
                    &mut rules,
                    "always-run",
                    "shared validation input changed",
                );
                continue;
            }

            if let Some(skill_name) = skill_name_for_path(path) {
                push_rule(
                    &mut checks,
                    &mut rules,
                    format!("skill:{skill_name}"),
                    "skill-scoped validation input changed",
                );
            }

            if is_rust_input(path) {
                push_rule(
                    &mut checks,
                    &mut rules,
                    "rust:lint",
                    "Rust source or manifest changed",
                );
            } else if is_docs_input(path) {
                push_rule(
                    &mut checks,
                    &mut rules,
                    "docs",
                    "documentation input changed",
                );
            }
        }

        if rules.is_empty() && !changed_paths.is_empty() {
            push_rule(
                &mut checks,
                &mut rules,
                "global:default",
                "unmapped input defaults to validation",
            );
        }

        Self {
            changed_paths,
            rules,
        }
    }

    pub fn checks(&self) -> Vec<&str> {
        self.rules.iter().map(|rule| rule.check.as_str()).collect()
    }

    pub fn includes_check(&self, check: &str) -> bool {
        self.rules.iter().any(|rule| rule.check == check)
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
