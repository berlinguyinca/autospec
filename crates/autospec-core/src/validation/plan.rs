use std::collections::BTreeSet;

use super::catalog::{CheckOwner, ValidationCatalog, ValidationCheck};
use super::external::ExternalCheck;
use super::options::{Jobs, ValidationOptions};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedValidationCheck {
    pub occurrence_index: usize,
    pub check: ValidationCheck,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationPlan {
    checks: Vec<PlannedValidationCheck>,
    fast: bool,
    jobs: Jobs,
    changed_base: Option<String>,
    changed_paths: Vec<String>,
}

impl ValidationPlan {
    pub fn build(catalog: &ValidationCatalog, options: &ValidationOptions) -> Result<Self, String> {
        Self::build_with_changed_paths(catalog, options, std::iter::empty::<&str>())
    }

    pub fn build_with_changed_paths(
        catalog: &ValidationCatalog,
        options: &ValidationOptions,
        changed_paths: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Result<Self, String> {
        catalog.validate()?;
        let changed_paths = changed_paths
            .into_iter()
            .map(|path| path.as_ref().replace('\\', "/"))
            .filter(|path| !path.is_empty())
            .collect::<Vec<_>>();
        let mut checks = Vec::new();
        for id in catalog.legacy_top_level_calls() {
            let check = catalog
                .checks()
                .iter()
                .find(|check| check.id == *id)
                .cloned()
                .ok_or_else(|| format!("legacy validation call has no catalog owner: {id}"))?;
            if options.fast && is_fast_only(&check) {
                continue;
            }
            let occurrence_index = checks.len();
            checks.push(PlannedValidationCheck {
                occurrence_index,
                check,
            });
        }
        Ok(Self {
            checks,
            fast: options.fast,
            jobs: options.jobs.clone(),
            changed_base: options.changed_base.clone(),
            changed_paths,
        })
    }

    pub fn from_checks(checks: Vec<ValidationCheck>, fast: bool, jobs: Jobs) -> Self {
        Self {
            checks: checks
                .into_iter()
                .enumerate()
                .map(|(occurrence_index, check)| PlannedValidationCheck {
                    occurrence_index,
                    check,
                })
                .collect(),
            fast,
            jobs,
            changed_base: None,
            changed_paths: Vec::new(),
        }
    }

    pub fn checks(&self) -> &[PlannedValidationCheck] {
        &self.checks
    }

    pub fn ids(&self) -> Vec<&str> {
        self.checks.iter().map(|entry| entry.check.id).collect()
    }

    pub fn unique_ids(&self) -> BTreeSet<&str> {
        self.ids().into_iter().collect()
    }

    pub fn fast(&self) -> bool {
        self.fast
    }

    pub fn changed_base(&self) -> Option<&str> {
        self.changed_base.as_deref()
    }

    pub fn changed_paths(&self) -> &[String] {
        &self.changed_paths
    }

    pub fn parallelism(&self) -> usize {
        match self.jobs {
            Jobs::Fixed(jobs) => jobs,
            Jobs::Auto => std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1)
                .saturating_sub(2)
                .max(1),
        }
    }
}

fn is_fast_only(check: &ValidationCheck) -> bool {
    matches!(
        check.owner,
        CheckOwner::ExternalBatch(
            ExternalCheck::PythonSuites
                | ExternalCheck::BatsSuite(_)
                | ExternalCheck::BatsDirectory(_)
                | ExternalCheck::InstallTests
        )
    )
}
