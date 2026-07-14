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
        match &check.owner {
            CheckOwner::RustNative(owner) => Self::run_structural(check, *owner, root),
            CheckOwner::External(command) => {
                let _mode = enter_fast_validation_mode(fast);
                command.execute_in(check.id, check.required, root)
            }
            CheckOwner::ExternalBatch(batch) => {
                batch.run_with_fast(check.id, check.required, root, fast)
            }
        }
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
            Err(error) => CheckResult::completed(
                check.id,
                check.required,
                1,
                started.elapsed().as_millis(),
                0,
                0,
                error.len(),
                output_digest(&[], error.as_bytes()),
            ),
        }
    }
}
