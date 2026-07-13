use std::path::Path;
use std::time::Instant;

use super::catalog::{CheckOwner, ValidationCatalog, ValidationCheck};
use super::results::{output_digest, CheckResult, ValidationExecutionReport};
use super::structural::StructuralValidator;

pub struct ValidationRunner;

impl ValidationRunner {
    pub fn run(catalog: &ValidationCatalog, root: &Path) -> ValidationExecutionReport {
        let results = catalog
            .checks()
            .iter()
            .map(|check| Self::run_check(check, root))
            .collect();
        ValidationExecutionReport::new(results)
    }

    fn run_check(check: &ValidationCheck, root: &Path) -> CheckResult {
        match &check.owner {
            CheckOwner::RustNative(owner) => Self::run_structural(check, *owner, root),
            CheckOwner::External(command) => command.execute(check.id, check.required),
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
