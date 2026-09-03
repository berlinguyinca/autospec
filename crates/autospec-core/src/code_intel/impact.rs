use std::collections::BTreeSet;

use serde::Serialize;

use super::diagnostics::Diagnostic;
use super::schema::{Provenance, Reference, Symbol};

/// The normalized answer to `code.impact` — the gateway's primary high-level
/// operation and the evidence a plan's Impact Set is checked against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImpactSet {
    pub provenance: Provenance,
    pub target: String,
    pub definitions: Vec<Symbol>,
    pub callers: Vec<Symbol>,
    pub callees: Vec<Symbol>,
    pub implementations: Vec<Symbol>,
    pub references: Vec<Reference>,
    pub exports: Vec<Symbol>,
    pub related_tests: Vec<String>,
    pub dependent_modules: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

impl ImpactSet {
    pub fn new(provenance: Provenance, target: impl Into<String>) -> Self {
        Self {
            provenance,
            target: target.into(),
            definitions: Vec::new(),
            callers: Vec::new(),
            callees: Vec::new(),
            implementations: Vec::new(),
            references: Vec::new(),
            exports: Vec::new(),
            related_tests: Vec::new(),
            dependent_modules: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Every file the analysis touched, deduplicated and sorted.
    ///
    /// This is what a plan's `## Files touched` section is compared against, so
    /// it must include every path any leg of the analysis produced.
    pub fn affected_files(&self) -> Vec<String> {
        let mut files: BTreeSet<String> = BTreeSet::new();
        for group in [
            &self.definitions,
            &self.callers,
            &self.callees,
            &self.implementations,
            &self.exports,
        ] {
            files.extend(group.iter().map(|symbol| symbol.location.path.clone()));
        }
        files.extend(
            self.references
                .iter()
                .map(|reference| reference.location.path.clone()),
        );
        files.extend(self.related_tests.iter().cloned());
        files.extend(
            self.diagnostics
                .iter()
                .map(|diagnostic| diagnostic.location.path.clone()),
        );
        files.into_iter().collect()
    }

    /// Identities of every symbol the analysis produced.
    pub fn affected_symbols(&self) -> Vec<String> {
        let mut symbols: BTreeSet<String> = BTreeSet::new();
        for group in [
            &self.definitions,
            &self.callers,
            &self.callees,
            &self.implementations,
            &self.exports,
        ] {
            symbols.extend(group.iter().map(Symbol::identity));
        }
        symbols.into_iter().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.affected_files().is_empty()
    }

    /// Merge another repository's analysis of the same target.
    ///
    /// No single language server resolves cross-repository relationships, so a
    /// project-wide impact set is the union of independent per-workspace
    /// queries. The provenance of the receiver is kept; per-result provenance
    /// stays available on the merged sub-results.
    pub fn merge(&mut self, other: ImpactSet) {
        self.definitions.extend(other.definitions);
        self.callers.extend(other.callers);
        self.callees.extend(other.callees);
        self.implementations.extend(other.implementations);
        self.references.extend(other.references);
        self.exports.extend(other.exports);
        self.related_tests.extend(other.related_tests);
        self.dependent_modules.extend(other.dependent_modules);
        self.diagnostics.extend(other.diagnostics);
    }

    pub fn to_json_string(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|error| error.to_string())
    }
}

/// How an actual impact set compares to the one a plan declared.
///
/// The reviewer uses this to catch a change that quietly grew past its plan,
/// and the implementer uses it to catch a plan that was too narrow before
/// editing anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImpactComparison {
    /// Files the analysis found that the plan did not declare.
    pub unplanned_files: Vec<String>,
    /// Files the plan declared that the analysis did not reach.
    pub unreached_files: Vec<String>,
    /// Files present on both sides.
    pub confirmed_files: Vec<String>,
}

impl ImpactComparison {
    pub fn between(planned: &[String], actual: &[String]) -> Self {
        let planned: BTreeSet<&String> = planned.iter().collect();
        let actual: BTreeSet<&String> = actual.iter().collect();
        Self {
            unplanned_files: difference(&actual, &planned),
            unreached_files: difference(&planned, &actual),
            confirmed_files: actual
                .intersection(&planned)
                .map(|path| (*path).clone())
                .collect(),
        }
    }

    /// Whether the change stayed inside the planned Impact Set.
    pub fn within_plan(&self) -> bool {
        self.unplanned_files.is_empty()
    }

    pub fn summary(&self) -> String {
        format!(
            "{} confirmed, {} unplanned, {} unreached",
            self.confirmed_files.len(),
            self.unplanned_files.len(),
            self.unreached_files.len()
        )
    }
}

fn difference(left: &BTreeSet<&String>, right: &BTreeSet<&String>) -> Vec<String> {
    left.difference(right).map(|path| (*path).clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::super::diagnostics::Severity;
    use super::super::schema::{Location, ResultSource};
    use super::*;

    fn provenance() -> Provenance {
        Provenance::new(
            "issue-421",
            "autospec",
            "abc123",
            "agent-lsp",
            ResultSource::Lsp,
        )
    }

    fn symbol(name: &str, path: &str) -> Symbol {
        Symbol::new(name, "function", Location::point(path, 1, 0))
    }

    fn impact() -> ImpactSet {
        let mut impact = ImpactSet::new(provenance(), "Gateway::resolve");
        impact.definitions.push(symbol("resolve", "src/gateway.rs"));
        impact.callers.push(symbol("dispatch", "src/router.rs"));
        impact
            .references
            .push(Reference::read(Location::point("src/router.rs", 12, 8)));
        impact
            .related_tests
            .push("tests/gateway_test.rs".to_string());
        impact
    }

    #[test]
    fn affected_files_span_every_leg_of_the_analysis() {
        let files = impact().affected_files();

        assert_eq!(
            files,
            vec![
                "src/gateway.rs".to_string(),
                "src/router.rs".to_string(),
                "tests/gateway_test.rs".to_string(),
            ]
        );
    }

    #[test]
    fn affected_files_include_diagnostic_paths() {
        let mut impact = impact();
        impact.diagnostics.push(Diagnostic::new(
            Location::point("src/broken.rs", 4, 0),
            Severity::Error,
            "unresolved import",
        ));

        assert!(impact
            .affected_files()
            .contains(&"src/broken.rs".to_string()));
    }

    #[test]
    fn affected_symbols_are_deduplicated() {
        let mut impact = impact();
        impact.callees.push(symbol("resolve", "src/gateway.rs"));

        let symbols = impact.affected_symbols();

        assert_eq!(
            symbols,
            vec![
                "dispatch@src/router.rs".to_string(),
                "resolve@src/gateway.rs".to_string()
            ]
        );
    }

    #[test]
    fn an_empty_analysis_is_reported_as_empty() {
        assert!(ImpactSet::new(provenance(), "Missing").is_empty());
        assert!(!impact().is_empty());
    }

    #[test]
    fn merging_aggregates_a_second_repositorys_analysis() {
        let mut first = impact();
        let mut second = ImpactSet::new(provenance(), "Gateway::resolve");
        second.callers.push(symbol("call", "gui/src/main.rs"));

        first.merge(second);

        assert!(first
            .affected_files()
            .contains(&"gui/src/main.rs".to_string()));
    }

    #[test]
    fn a_change_inside_the_plan_is_within_plan() {
        let planned = vec!["src/gateway.rs".to_string(), "src/router.rs".to_string()];

        let comparison = ImpactComparison::between(&planned, &impact().affected_files());

        assert!(!comparison.within_plan());
        assert_eq!(
            comparison.unplanned_files,
            vec!["tests/gateway_test.rs".to_string()]
        );
    }

    #[test]
    fn a_complete_plan_leaves_nothing_unplanned() {
        let planned = impact().affected_files();

        let comparison = ImpactComparison::between(&planned, &impact().affected_files());

        assert!(comparison.within_plan());
        assert!(comparison.unreached_files.is_empty());
        assert_eq!(comparison.confirmed_files.len(), 3);
    }

    #[test]
    fn a_plan_wider_than_the_analysis_reports_unreached_files() {
        let planned = vec!["src/unrelated.rs".to_string()];

        let comparison = ImpactComparison::between(&planned, &["src/gateway.rs".to_string()]);

        assert_eq!(
            comparison.unreached_files,
            vec!["src/unrelated.rs".to_string()]
        );
        assert_eq!(
            comparison.summary(),
            "0 confirmed, 1 unplanned, 1 unreached"
        );
    }
}
