use std::collections::BTreeSet;

use serde::Serialize;

use super::schema::Location;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Information => "information",
            Self::Hint => "hint",
        }
    }

    /// LSP severity codes are numeric; normalize them once here so no other
    /// module carries the mapping.
    pub fn from_lsp_code(code: u64) -> Self {
        match code {
            1 => Self::Error,
            2 => Self::Warning,
            3 => Self::Information,
            _ => Self::Hint,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    pub location: Location,
    pub severity: Severity,
    pub code: Option<String>,
    pub message: String,
    pub source: Option<String>,
}

impl Diagnostic {
    pub fn new(location: Location, severity: Severity, message: impl Into<String>) -> Self {
        Self {
            location,
            severity,
            code: None,
            message: message.into(),
            source: None,
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Line-independent identity.
    ///
    /// Editing a file above a diagnostic shifts every line below it. Keying on
    /// line number would report those shifted diagnostics as new errors, so
    /// identity is file + severity + code + message instead.
    pub fn identity(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.location.path,
            self.severity.as_str(),
            self.code.as_deref().unwrap_or(""),
            self.message
        )
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// A diagnostic snapshot for one workspace at one point in the lifecycle.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DiagnosticSet {
    pub workspace: String,
    pub revision: String,
    pub diagnostics: Vec<Diagnostic>,
}

impl DiagnosticSet {
    pub fn new(
        workspace: impl Into<String>,
        revision: impl Into<String>,
        diagnostics: Vec<Diagnostic>,
    ) -> Self {
        Self {
            workspace: workspace.into(),
            revision: revision.into(),
            diagnostics,
        }
    }

    pub fn errors(&self) -> Vec<&Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .collect()
    }

    pub fn error_count(&self) -> usize {
        self.errors().len()
    }

    fn identities(&self) -> BTreeSet<String> {
        self.diagnostics
            .iter()
            .map(|diagnostic| diagnostic.identity())
            .collect()
    }
}

/// What changed between a baseline snapshot and a post-change snapshot.
///
/// Baseline errors stay visible but never fail a task on their own: the task
/// did not introduce them. Only `new_errors` gates completion.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DiagnosticDelta {
    pub workspace: String,
    pub baseline_errors: usize,
    pub current_errors: usize,
    pub new_diagnostics: Vec<Diagnostic>,
    pub resolved_diagnostics: Vec<Diagnostic>,
}

impl DiagnosticDelta {
    /// Compare two snapshots of the same workspace.
    pub fn between(baseline: &DiagnosticSet, current: &DiagnosticSet) -> Self {
        let baseline_identities = baseline.identities();
        let current_identities = current.identities();
        Self {
            workspace: current.workspace.clone(),
            baseline_errors: baseline.error_count(),
            current_errors: current.error_count(),
            new_diagnostics: select(current, &baseline_identities),
            resolved_diagnostics: select(baseline, &current_identities),
        }
    }

    /// Errors this change introduced.
    pub fn new_errors(&self) -> Vec<&Diagnostic> {
        self.new_diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .collect()
    }

    pub fn has_new_errors(&self) -> bool {
        !self.new_errors().is_empty()
    }

    /// Whether the change may complete under the `block_new_errors` policy.
    pub fn is_clean(&self, block_new_errors: bool) -> bool {
        !block_new_errors || !self.has_new_errors()
    }

    /// One-line summary for the closeout report.
    pub fn summary(&self) -> String {
        format!(
            "{}: {} baseline errors, {} current errors, {} new, {} resolved",
            self.workspace,
            self.baseline_errors,
            self.current_errors,
            self.new_diagnostics.len(),
            self.resolved_diagnostics.len()
        )
    }

    pub fn to_json_string(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|error| error.to_string())
    }
}

fn select(set: &DiagnosticSet, exclude: &BTreeSet<String>) -> Vec<Diagnostic> {
    set.diagnostics
        .iter()
        .filter(|diagnostic| !exclude.contains(&diagnostic.identity()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(path: &str, line: u32, message: &str) -> Diagnostic {
        Diagnostic::new(
            Location::point(path, line, 0),
            Severity::Error,
            message.to_string(),
        )
    }

    fn warning(path: &str, line: u32, message: &str) -> Diagnostic {
        Diagnostic::new(Location::point(path, line, 0), Severity::Warning, message)
    }

    fn set(diagnostics: Vec<Diagnostic>) -> DiagnosticSet {
        DiagnosticSet::new("issue-421", "abc123", diagnostics)
    }

    #[test]
    fn lsp_severity_codes_normalize() {
        assert_eq!(Severity::from_lsp_code(1), Severity::Error);
        assert_eq!(Severity::from_lsp_code(2), Severity::Warning);
        assert_eq!(Severity::from_lsp_code(3), Severity::Information);
        assert_eq!(Severity::from_lsp_code(4), Severity::Hint);
        assert_eq!(Severity::from_lsp_code(99), Severity::Hint);
    }

    #[test]
    fn a_new_error_is_reported_against_a_clean_baseline() {
        let delta = DiagnosticDelta::between(
            &set(Vec::new()),
            &set(vec![error("src/gateway.rs", 12, "unresolved import")]),
        );

        assert!(delta.has_new_errors());
        assert_eq!(delta.new_errors().len(), 1);
        assert!(!delta.is_clean(true));
    }

    #[test]
    fn baseline_errors_do_not_fail_a_task() {
        let baseline = set(vec![error("src/legacy.rs", 3, "pre-existing")]);
        let current = set(vec![error("src/legacy.rs", 3, "pre-existing")]);

        let delta = DiagnosticDelta::between(&baseline, &current);

        assert!(!delta.has_new_errors());
        assert!(delta.is_clean(true));
        assert_eq!(delta.baseline_errors, 1);
        assert_eq!(delta.current_errors, 1);
    }

    #[test]
    fn a_diagnostic_shifted_by_an_edit_above_it_is_not_new() {
        let baseline = set(vec![error("src/legacy.rs", 3, "pre-existing")]);
        let current = set(vec![error("src/legacy.rs", 41, "pre-existing")]);

        let delta = DiagnosticDelta::between(&baseline, &current);

        assert!(!delta.has_new_errors());
        assert!(delta.new_diagnostics.is_empty());
    }

    #[test]
    fn resolved_diagnostics_are_reported() {
        let baseline = set(vec![error("src/legacy.rs", 3, "pre-existing")]);
        let current = set(Vec::new());

        let delta = DiagnosticDelta::between(&baseline, &current);

        assert_eq!(delta.resolved_diagnostics.len(), 1);
        assert!(delta.is_clean(true));
    }

    #[test]
    fn new_warnings_never_block_completion() {
        let delta = DiagnosticDelta::between(
            &set(Vec::new()),
            &set(vec![warning("src/gateway.rs", 9, "unused variable")]),
        );

        assert_eq!(delta.new_diagnostics.len(), 1);
        assert!(!delta.has_new_errors());
        assert!(delta.is_clean(true));
    }

    #[test]
    fn disabling_the_policy_lets_new_errors_through() {
        let delta = DiagnosticDelta::between(
            &set(Vec::new()),
            &set(vec![error("src/gateway.rs", 12, "type mismatch")]),
        );

        assert!(delta.has_new_errors());
        assert!(delta.is_clean(false));
    }

    #[test]
    fn the_same_message_in_a_different_file_is_a_new_error() {
        let baseline = set(vec![error("src/a.rs", 1, "type mismatch")]);
        let current = set(vec![
            error("src/a.rs", 1, "type mismatch"),
            error("src/b.rs", 1, "type mismatch"),
        ]);

        let delta = DiagnosticDelta::between(&baseline, &current);

        assert_eq!(delta.new_errors().len(), 1);
        assert_eq!(delta.new_errors()[0].location.path, "src/b.rs");
    }

    #[test]
    fn codes_distinguish_otherwise_identical_diagnostics() {
        let baseline = set(vec![error("src/a.rs", 1, "mismatch").with_code("E0308")]);
        let current = set(vec![error("src/a.rs", 1, "mismatch").with_code("E0277")]);

        let delta = DiagnosticDelta::between(&baseline, &current);

        assert_eq!(delta.new_errors().len(), 1);
        assert_eq!(delta.resolved_diagnostics.len(), 1);
    }

    #[test]
    fn the_summary_reports_both_sides_of_the_delta() {
        let baseline = set(vec![error("src/a.rs", 1, "old")]);
        let current = set(vec![error("src/b.rs", 2, "new")]);

        let summary = DiagnosticDelta::between(&baseline, &current).summary();

        assert_eq!(
            summary,
            "issue-421: 1 baseline errors, 1 current errors, 1 new, 1 resolved"
        );
    }
}
