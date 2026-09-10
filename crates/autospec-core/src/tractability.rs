//! Tractability at pre-dispatch validation (#4095).
//!
//! The tree observation that proves an issue is real — "no code of this
//! kind exists in the tree" — predicts the work will not fit a
//! single-agent budget: no structure to extend, no interface to
//! satisfy, no adjacent file to imitate. Genuineness and tractability
//! are different questions answered in opposite directions by the same
//! evidence, so pre-dispatch validation records both.
//!
//! - The **estimate** ([`PreflightRecord`], [`Tractability`]) sits
//!   alongside the genuineness check: work predicted to create new
//!   top-level packages or binaries is flagged as exceeding a
//!   single-agent budget.
//! - **Flagged work is not dispatched as-is** ([`decide`]): it is
//!   returned for decomposition — the package boundary and its first
//!   consumer as separate issues.
//! - The **residue of a budget-terminated run** ([`BudgetResidue`])
//!   records files touched, directories created, and buildability, so
//!   "timed out" and "timed out having half-built a subsystem" are
//!   distinguishable without opening the worktree.
//! - "No code of this kind exists" is a **tractability signal, not only
//!   a genuineness one** ([`ExistingStructure::Absent`]).
//!
//! Everything here is pure: the caller scans the tree and worktree, and acts on the results.

use std::collections::BTreeSet;

/// The pre-dispatch tree observation shared by the genuineness check and
/// the tractability estimate: does code of this kind exist in the tree?
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExistingStructure {
    /// Code of this kind exists in the tree: the work has something to extend.
    Present {
        /// The evidence the scan found (a path or a query + hit count).
        evidence: String,
    },
    /// No code of this kind exists in the tree: the strongest evidence
    /// the work is genuinely unaddressed, and — a tractability signal,
    /// not only a genuineness one (#4095) — the strongest predictor the
    /// work will not fit a single-agent budget: no boundary to extend,
    /// no precedent to imitate.
    Absent {
        /// The evidence the scan is based on (the query run and what it
        /// did not find).
        evidence: String,
    },
}

impl ExistingStructure {
    /// Record that code of this kind exists in the tree.
    pub fn present(evidence: &str) -> Result<Self, String> {
        if evidence.trim().is_empty() {
            return Err(
                "evidence must be non-empty: name the path or the query that found it".to_string(),
            );
        }
        Ok(Self::Present {
            evidence: evidence.trim().to_string(),
        })
    }

    /// Record that no code of this kind exists in the tree.
    pub fn absent(evidence: &str) -> Result<Self, String> {
        if evidence.trim().is_empty() {
            return Err(
                "evidence must be non-empty: name the query run and what it missed".to_string(),
            );
        }
        Ok(Self::Absent {
            evidence: evidence.trim().to_string(),
        })
    }
}

/// The tractability estimate recorded alongside the genuineness check:
/// does the work extend existing structure, or create new top-level
/// packages/binaries?
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tractability {
    /// The work adds files inside packages and binaries that already
    /// exist: it fits a single-agent budget on tractability grounds.
    ExtendsExisting,
    /// The work creates top-level structure that did not exist before:
    /// new packages and/or new binaries.
    CreatesNewStructure {
        /// Top-level packages the work would create (e.g. `internal/audit`).
        new_packages: Vec<String>,
        /// Top-level binaries the work would create (e.g. `cmd/auditread`).
        new_binaries: Vec<String>,
    },
}

impl Tractability {
    /// Whether the work is flagged as exceeding a single-agent budget:
    /// work predicted to create a new top-level package or binary does
    /// not fit a single run (#4095).
    pub fn exceeds_single_agent_budget(&self) -> bool {
        match self {
            Self::ExtendsExisting => false,
            Self::CreatesNewStructure {
                new_packages,
                new_binaries,
            } => !new_packages.is_empty() || !new_binaries.is_empty(),
        }
    }
}

/// The pre-dispatch validation record for one issue: the genuineness
/// verdict and the tractability estimate side by side, built from the
/// same tree observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightRecord {
    /// The genuineness verdict: the work is genuinely unaddressed — it
    /// has not already landed.
    pub work_unaddressed: bool,
    /// The shared tree observation: the strongest evidence for
    /// `work_unaddressed` and, when absent, the strongest predictor that
    /// the work will not fit a single-agent budget.
    pub existing_structure: ExistingStructure,
    /// Top-level packages the implementation would create, by path.
    pub new_packages: Vec<String>,
    /// Top-level binaries the implementation would create, by path.
    pub new_binaries: Vec<String>,
}

impl PreflightRecord {
    /// The tractability estimate recorded alongside the genuineness
    /// check: extends existing structure, or creates new top-level
    /// packages/binaries.
    pub fn tractability(&self) -> Tractability {
        if self.new_packages.is_empty() && self.new_binaries.is_empty() {
            Tractability::ExtendsExisting
        } else {
            Tractability::CreatesNewStructure {
                new_packages: self.new_packages.clone(),
                new_binaries: self.new_binaries.clone(),
            }
        }
    }
}

/// The dispatch decision: dispatch as-is, or return the issue for decomposition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchDecision {
    /// The work fits a single-agent budget: dispatch as-is.
    Dispatch,
    /// The work is flagged as exceeding a single-agent budget: it is not
    /// dispatched as-is, but returned for decomposition — the package
    /// boundary and its first consumer as separate issues.
    ReturnForDecomposition {
        /// The new package boundaries to split on.
        new_packages: Vec<String>,
        /// The new binaries to split on.
        new_binaries: Vec<String>,
    },
}

impl DispatchDecision {
    /// Whether the issue is dispatched as-is.
    pub fn dispatches(&self) -> bool {
        matches!(self, Self::Dispatch)
    }
}

/// Decide, from the pre-dispatch validation record, whether the issue is
/// dispatched as-is or returned for decomposition. The gate is the
/// tractability flag, not the genuineness verdict: real work that is
/// predicted to create new top-level packages or binaries is returned
/// for decomposition — the same treatment component-sized work already
/// receives (#4095, InferWeave#272).
pub fn decide(record: &PreflightRecord) -> DispatchDecision {
    match record.tractability() {
        Tractability::ExtendsExisting => DispatchDecision::Dispatch,
        Tractability::CreatesNewStructure {
            new_packages,
            new_binaries,
        } => DispatchDecision::ReturnForDecomposition {
            new_packages,
            new_binaries,
        },
    }
}

/// What a budget-terminated run left behind in the worktree (#4095).
///
/// "Timed out" and "timed out having half-built a subsystem" must be
/// distinguishable without opening the worktree: a run that dies at the
/// budget records its residue instead of a bare timeout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetResidue {
    /// Files the run touched (created or modified), by path.
    pub files_touched: Vec<String>,
    /// Directories the run created that did not exist before it.
    pub new_directories: Vec<String>,
    /// Whether the tree still builds after the run.
    pub buildable: bool,
}

impl BudgetResidue {
    /// A run that left nothing behind and a tree that still builds.
    pub fn clean() -> Self {
        Self {
            files_touched: Vec::new(),
            new_directories: Vec::new(),
            buildable: true,
        }
    }

    /// Whether the run left no residue at all.
    pub fn is_empty(&self) -> bool {
        self.files_touched.is_empty() && self.new_directories.is_empty()
    }

    /// The recorded outcome a monitor can report without opening the
    /// worktree: a single `TIMEOUT after <n>s: no residue (...)` line
    /// for a clean run, or a header line with the counts and build state
    /// followed by one line per touched file and new directory.
    pub fn report(&self, budget_secs: u64) -> String {
        if self.is_empty() {
            return format!(
                "TIMEOUT after {budget_secs}s: no residue (tree {})",
                build_state(self.buildable)
            );
        }
        let files = count_noun(self.files_touched.len(), "file", "files");
        let dirs = count_noun(
            self.new_directories.len(),
            "new directory",
            "new directories",
        );
        let mut lines = vec![format!(
            "TIMEOUT after {budget_secs}s: residue ({files} touched, {dirs}, tree {})",
            build_state(self.buildable)
        )];
        for file in &dedup(&self.files_touched) {
            lines.push(format!("  file: {file}"));
        }
        for dir in &dedup(&self.new_directories) {
            lines.push(format!("  new directory: {dir}"));
        }
        lines.join("\n")
    }
}

fn build_state(buildable: bool) -> &'static str {
    if buildable {
        "buildable"
    } else {
        "not buildable"
    }
}

fn count_noun(count: usize, singular: &str, plural: &str) -> String {
    format!("{count} {}", if count == 1 { singular } else { plural })
}

fn dedup(paths: &[String]) -> Vec<String> {
    let mut set = BTreeSet::new();
    for path in paths {
        set.insert(path);
    }
    set.into_iter().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(
        unaddressed: bool,
        structure: ExistingStructure,
        packages: &[&str],
        binaries: &[&str],
    ) -> PreflightRecord {
        PreflightRecord {
            work_unaddressed: unaddressed,
            existing_structure: structure,
            new_packages: packages.iter().map(|s| s.to_string()).collect(),
            new_binaries: binaries.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn evidence_is_required_for_both_observations() {
        assert!(ExistingStructure::present("").is_err());
        assert!(ExistingStructure::present("   ").is_err());
        assert!(ExistingStructure::absent("").is_err());
        assert_eq!(
            ExistingStructure::present("internal/audit/writer.go exists").unwrap(),
            ExistingStructure::Present {
                evidence: "internal/audit/writer.go exists".to_string()
            }
        );
        assert_eq!(
            ExistingStructure::absent("grep -r 'auditread' .: no hits").unwrap(),
            ExistingStructure::Absent {
                evidence: "grep -r 'auditread' .: no hits".to_string()
            }
        );
    }

    #[test]
    fn new_top_level_package_is_flagged_and_not_dispatched() {
        // The #11-shaped issue: no audit code in the tree, and the
        // implementation would create a new package and a new binary.
        let absent = ExistingStructure::absent("grep -r 'audit' .: no hits").unwrap();
        // A new binary alone flags too.
        let rec_bin = record(true, absent.clone(), &[], &["cmd/auditread"]);
        assert!(rec_bin.tractability().exceeds_single_agent_budget());
        assert!(!decide(&rec_bin).dispatches());
        let rec = record(true, absent, &["internal/audit"], &["cmd/auditread"]);
        assert!(rec.tractability().exceeds_single_agent_budget());
        assert!(!decide(&rec).dispatches());
        assert_eq!(
            decide(&rec),
            DispatchDecision::ReturnForDecomposition {
                new_packages: vec!["internal/audit".to_string()],
                new_binaries: vec!["cmd/auditread".to_string()],
            }
        );
    }

    #[test]
    fn extending_an_existing_package_is_dispatched() {
        // The #54-shaped issue: code of this kind exists, and the work
        // adds files inside an existing package.
        let present = ExistingStructure::present("internal/audit/writer.go exists").unwrap();
        let rec = record(true, present, &[], &[]);
        assert_eq!(rec.tractability(), Tractability::ExtendsExisting);
        assert!(!rec.tractability().exceeds_single_agent_budget());
        assert_eq!(decide(&rec), DispatchDecision::Dispatch);
    }

    #[test]
    fn the_absent_observation_is_recorded_on_the_tractability_record() {
        // AC4: "no code of this kind exists" is a tractability signal
        // recorded on the pre-dispatch record, not only a genuineness one.
        let absent = ExistingStructure::absent("grep -r 'auditread' .: no hits").unwrap();
        let rec = record(true, absent.clone(), &["internal/audit"], &[]);
        assert_eq!(rec.existing_structure, absent);
        assert!(rec.work_unaddressed);
        assert!(rec.tractability().exceeds_single_agent_budget());
    }

    #[test]
    fn budget_terminated_run_reports_its_residue() {
        let residue = BudgetResidue {
            files_touched: vec![
                "go.mod".to_string(),
                "internal/audit/writer.go".to_string(),
                "cmd/auditread/main.go".to_string(),
            ],
            new_directories: vec!["cmd/auditread".to_string(), "internal/audit".to_string()],
            buildable: false,
        };
        assert!(!residue.is_empty());
        let report = residue.report(5400);
        assert_eq!(
            report,
            "TIMEOUT after 5400s: residue (3 files touched, 2 new directories, tree not buildable)\n  file: cmd/auditread/main.go\n  file: go.mod\n  file: internal/audit/writer.go\n  new directory: cmd/auditread\n  new directory: internal/audit"
        );
    }

    #[test]
    fn a_clean_timeout_is_distinguishable_from_a_half_built_subsystem() {
        let clean = BudgetResidue::clean();
        assert!(clean.is_empty());
        let clean_report = clean.report(5400);
        assert_eq!(
            clean_report,
            "TIMEOUT after 5400s: no residue (tree buildable)"
        );

        let half_built = BudgetResidue {
            files_touched: vec!["internal/audit/writer.go".to_string()],
            new_directories: vec!["internal/audit".to_string()],
            buildable: false,
        };
        let half_built_report = half_built.report(5400);
        assert_ne!(clean_report, half_built_report);
        assert!(half_built_report.contains("tree not buildable"));
        assert!(half_built_report.contains("new directory: internal/audit"));
    }

    #[test]
    fn residue_with_a_buildable_tree_is_reported_as_buildable() {
        let residue = BudgetResidue {
            files_touched: vec!["internal/audit/writer.go".to_string()],
            new_directories: Vec::new(),
            buildable: true,
        };
        let report = residue.report(2700);
        assert!(report.starts_with(
            "TIMEOUT after 2700s: residue (1 file touched, 0 new directories, tree buildable)"
        ));
    }
}
