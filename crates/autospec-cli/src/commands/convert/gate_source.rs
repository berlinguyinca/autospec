//! Resolving what the pass may judge (issue #4556): the recorded gate for the
//! repository the pass names, and the pass's coverage of its pipeline glob.
//!
//! The gate set was a constant in the conversion code, which is how a pass
//! could silently gate a repository's patches with a gate that was never
//! established for that repository. The set is now data — a per-repository
//! registry ([`autospec_core::gate_registry`]) — and a repository with no
//! recorded entry is refused rather than guessed.
//!
//! Coverage is the mirror image on the input side: a pass handed one
//! pipeline's root must say how many patches the pipelines it never opened
//! hold, or a count of zero from a directory that was never examined is
//! reportable as "nothing to convert" ([`autospec_core::pipeline_coverage`]).

use std::path::Path;

use autospec_core::gate_registry;
use autospec_core::pipeline_coverage;

use crate::commands::CommandFailure;

/// Resolve the recorded gate for the repository the pass will judge.
///
/// The registry file is found in this order: the explicit `--gate-registry`
/// path, `$AUTOSPEC_GATE_REGISTRY`, then the in-repository default
/// `data/convert-gate-registry.json` under the checkout the pass runs in.
///
/// Every failure refuses the pass before any patch is judged: a missing
/// file, a file that fails to parse, and a repository the registry does not
/// name all mean the gate is not established, and a gate that is not
/// established is not one the pass may run. The refusal names the file the
/// operator must record the gate in.
pub(super) fn resolve_gate(explicit: Option<&Path>, repo: &str) -> Result<gate_registry::GateSet, CommandFailure> {
    let cwd = std::env::current_dir().map_err(|error| {
        CommandFailure::diagnostic(format!("cannot resolve the gate registry location: {error}"))
    })?;
    let path = gate_registry::resolve_registry_path(
        explicit,
        std::env::var(gate_registry::REGISTRY_ENV).ok().as_deref(),
        &cwd,
    );
    let registry = gate_registry::GateRegistry::load(&path).map_err(|error| {
        CommandFailure::status(
            format!(
                "no gate established for {repo}: {error} — record the repository's gate \
                 (base branch, toolchain, full gate argv) in {} before gating its patches; \
                 the pass refuses to guess a gate (#4556)",
                path.display()
            ),
            2,
        )
    })?;
    match registry.lookup(repo) {
        Some(gate) => Ok(gate.clone()),
        None => Err(CommandFailure::status(
            format!(
                "no gate recorded for {repo} in {} — record it before gating its patches; \
                 the pass refuses to guess a gate (#4556)",
                path.display()
            ),
            2,
        )),
    }
}

/// The pass's coverage of its pipeline glob for the root it was handed, or
/// `None` when there is no coverage question (at most one pipeline exists).
/// `shared` mirrors `--shared-llm-root`: the operator declares whether the
/// root is the shared parent of all pipelines or one pipeline's directory.
pub(super) fn plan_coverage(
    root: &Path,
    shared: bool,
) -> Option<pipeline_coverage::CoverageReport> {
    pipeline_coverage::build(root, shared)
}

/// Finish a pass with its coverage: print the gap warnings, and exit
/// incomplete when the run reached only part of its glob. A run that
/// converts 1 of 4 pipelines must not report success; the work it did is
/// still reported (the counters printed above it are true), but the exit
/// says the pass was not whole.
pub(super) fn finish_with_coverage(
    coverage: Option<&pipeline_coverage::CoverageReport>,
) -> Result<(), CommandFailure> {
    let Some(coverage) = coverage else {
        return Ok(());
    };
    // The gaps are named per pipeline. They are diagnostics, not the plan:
    // they go to stderr so a `--json` run's stdout stays one clean document.
    for warning in coverage.warnings() {
        eprintln!("{warning}");
    }
    match coverage.exit_status() {
        Some(code) => Err(CommandFailure::status(coverage.status_line(), code)),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repository_without_a_recorded_gate_is_refused_named() {
        let dir = std::env::temp_dir().join(format!(
            "autospec-gate-source-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("dir");
        let registry = dir.join("registry.json");
        std::fs::write(
            &registry,
            r#"{"schema":1,"repos":{"someone/else":{"base_ref":"main","stages":[["test"]]}}}"#,
        )
        .expect("write");
        let failure = resolve_gate(Some(&registry), "berlinguyinca/autospec").unwrap_err();
        assert_eq!(failure.exit_code, 2);
        assert!(
            failure.message.contains("no gate recorded for berlinguyinca/autospec"),
            "{}",
            failure.message
        );
        assert!(failure.message.contains("refuses to guess"), "{}", failure.message);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_registry_refuses_before_any_judging() {
        let failure =
            resolve_gate(Some(Path::new("/nonexistent/registry.json")), "a/b").unwrap_err();
        assert_eq!(failure.exit_code, 2);
        assert!(failure.message.contains("no gate established"), "{}", failure.message);
    }

    #[test]
    fn a_recorded_gate_resolves_to_its_stages() {
        let dir = std::env::temp_dir().join(format!(
            "autospec-gate-source-ok-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("dir");
        let registry = dir.join("registry.json");
        std::fs::write(
            &registry,
            r#"{"schema":1,"repos":{"a/b":{"base_ref":"main","stages":[["fmt","--check"],["test","--no-fail-fast","@scope"]]}}}"#,
        )
        .expect("write");
        let gate = resolve_gate(Some(&registry), "a/b").expect("resolves");
        assert_eq!(gate.stages.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
