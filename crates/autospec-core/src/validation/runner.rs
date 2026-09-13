use std::path::Path;
use std::time::Instant;

use super::catalog::{CheckOwner, ValidationCatalog, ValidationCheck};
use super::command::enter_fast_validation_mode;
use super::plan::ValidationPlan;
use super::results::{output_digest, CheckResult, ValidationExecutionReport};
use super::structural::StructuralValidator;

pub struct ValidationRunner;

impl ValidationRunner {
    pub fn run(catalog: &ValidationCatalog, root: &Path) -> ValidationExecutionReport {
        let results = catalog
            .checks()
            .iter()
            .map(|check| Self::run_check(check, root, false))
            .collect();
        ValidationExecutionReport::new(results)
    }

    pub fn run_plan(plan: &ValidationPlan, root: &Path) -> ValidationExecutionReport {
        let jobs = plan.parallelism();
        let mut completed = Vec::new();

        std::thread::scope(|scope| {
            let mut active: Vec<(usize, std::thread::ScopedJoinHandle<'_, CheckResult>)> =
                Vec::new();
            for entry in plan.checks() {
                if jobs <= 1 || !entry.check.independent {
                    for (index, handle) in active.drain(..) {
                        completed.push((
                            index,
                            handle
                                .join()
                                .expect("parallel validation check must not panic"),
                        ));
                    }
                    completed.push((
                        entry.occurrence_index,
                        Self::run_check(&entry.check, root, plan.fast()),
                    ));
                    continue;
                }

                if active.len() == jobs {
                    let (index, handle) = active.remove(0);
                    completed.push((
                        index,
                        handle
                            .join()
                            .expect("parallel validation check must not panic"),
                    ));
                }
                let check = entry.check.clone();
                let check_root = root.to_path_buf();
                active.push((
                    entry.occurrence_index,
                    scope.spawn(move || Self::run_check(&check, &check_root, plan.fast())),
                ));
            }
            for (index, handle) in active {
                completed.push((
                    index,
                    handle
                        .join()
                        .expect("parallel validation check must not panic"),
                ));
            }
        });
        completed.sort_by_key(|(index, _)| *index);
        ValidationExecutionReport::new(completed.into_iter().map(|(_, result)| result).collect())
    }

    fn run_check(check: &ValidationCheck, root: &Path, fast: bool) -> CheckResult {
        let result = match &check.owner {
            CheckOwner::RustNative(owner) => Self::run_structural(check, *owner, root),
            CheckOwner::External(command) => {
                let _mode = enter_fast_validation_mode(fast);
                command.execute_in(check.id, check.required, root)
            }
            CheckOwner::ExternalBatch(batch) => {
                batch.run_with_fast(check.id, check.required, root, fast)
            }
        };
        Self::ensure_failure_is_explained(check.id, result)
    }

    /// A failing check must say why, and if it cannot, say *that*.
    ///
    /// #3734 established this and fixed it for native checks, whose `Err(String)`
    /// names the exact divergence. Batch checks were left behind: each has its own
    /// runner function and several never populate `failure`, so `validate` printed
    /// "failed (no reason captured)" for four required checks at once. That is
    /// strictly worse than a wrong reason -- there is nothing to act on, nothing to
    /// search for, and no way to tell a real failure from a check that is itself
    /// broken.
    ///
    /// The reason is synthesised here rather than in each runner because there are
    /// dozens of runners and the next one added would reintroduce the gap. What is
    /// synthesised is deliberately modest: it does not invent a cause, it states
    /// that the check produced none and reports what little the harness does know,
    /// which is enough to tell "the suite ran and something failed" from "the suite
    /// produced no output at all".
    fn ensure_failure_is_explained(id: &str, result: CheckResult) -> CheckResult {
        if result.is_success() || result.is_unmeasured() || result.failure.is_some() {
            return result;
        }
        let produced_output = result.stdout_bytes > 0 || result.stderr_bytes > 0;
        let detail = if produced_output {
            format!(
                "{id} reported failure without a reason; it wrote {} byte(s) to stdout and \
                 {} to stderr, so the output exists and is being discarded by its runner",
                result.stdout_bytes, result.stderr_bytes
            )
        } else {
            format!(
                "{id} reported failure without a reason and produced no output at all \
                 (exit {:?}); the check did not run, or ran and said nothing",
                result.exit_code
            )
        };
        result.with_failure(detail)
    }

    fn run_structural(
        check: &ValidationCheck,
        owner: super::catalog::StructuralCheck,
        root: &Path,
    ) -> CheckResult {
        let started = Instant::now();
        match StructuralValidator::run(owner, root) {
            Ok(()) => CheckResult::completed(
                check.id,
                check.required,
                0,
                started.elapsed().as_millis(),
                0,
                0,
                0,
                output_digest(&[], &[]),
            ),
            Err(error) => {
                // Keep the message, not just its length. `error` names the exact
                // divergence; discarding it is why validate could say a check failed
                // but never why (#3734).
                let digest = output_digest(&[], error.as_bytes());
                let bytes = error.len();
                CheckResult::completed(
                    check.id,
                    check.required,
                    1,
                    started.elapsed().as_millis(),
                    0,
                    0,
                    bytes,
                    digest,
                )
                .with_failure(error)
            }
        }
    }
}

#[cfg(test)]
mod failure_explanation_tests {
    use super::*;
    use crate::validation::results::CheckResult;

    fn failed(stdout: usize, stderr: usize) -> CheckResult {
        CheckResult::completed("check_example", true, 1, 5, 1, stdout, stderr, "digest")
    }

    #[test]
    fn a_failure_that_already_explains_itself_is_left_alone() {
        let original = failed(10, 0).with_failure("not ok 2 the thing diverged");
        let out = ValidationRunner::ensure_failure_is_explained("check_example", original);
        assert_eq!(out.failure.as_deref(), Some("not ok 2 the thing diverged"));
    }

    #[test]
    fn a_failure_with_no_reason_is_given_one() {
        // The production symptom: "failed (no reason captured)", which sends a
        // reader looking for a reason the run had and threw away.
        let out = ValidationRunner::ensure_failure_is_explained("check_example", failed(0, 0));
        let reason = out.failure.expect("a failing check must carry a reason");
        assert!(reason.contains("without a reason"), "{reason}");
    }

    #[test]
    fn output_that_exists_is_distinguished_from_output_that_does_not() {
        // These need different responses: the first is a runner discarding text
        // it already has, the second is a check that never ran. Reporting them
        // identically is what made four failures undiagnosable at once.
        let silent = ValidationRunner::ensure_failure_is_explained("check_example", failed(0, 0))
            .failure
            .unwrap();
        let noisy = ValidationRunner::ensure_failure_is_explained("check_example", failed(120, 8))
            .failure
            .unwrap();
        assert!(silent.contains("no output at all"), "{silent}");
        assert!(noisy.contains("discarded by its runner"), "{noisy}");
        assert_ne!(silent, noisy);
    }

    #[test]
    fn a_passing_check_is_never_given_a_failure() {
        let ok = CheckResult::completed("check_example", true, 0, 5, 1, 0, 0, "digest");
        let out = ValidationRunner::ensure_failure_is_explained("check_example", ok);
        assert!(out.failure.is_none());
        assert!(out.is_success());
    }

    #[test]
    fn an_unmeasured_check_keeps_its_own_distinction() {
        // "no measurement happened" and "a measurement says this" are different
        // states and must not be collapsed.
        let un = CheckResult::unmeasured("check_example", true, "the tool is absent");
        let out = ValidationRunner::ensure_failure_is_explained("check_example", un);
        assert!(out.failure.is_none());
        assert!(out.is_unmeasured());
    }
}
