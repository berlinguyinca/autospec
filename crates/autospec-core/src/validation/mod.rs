pub mod affected;
pub mod catalog;
pub mod command;
pub mod external;
pub mod measurement;
pub mod options;
pub mod output_macros;
pub mod plan;
pub mod reference_pointer;
pub mod results;
pub mod runner;
pub mod structural;
pub mod structural_text;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationStatus {
    Passed,
    Failed,
    /// At least one required check never produced a measurement.
    ///
    /// Distinct from `Failed` because the two call for different responses — a failure
    /// names a defect, an unknown names a hole in the evidence — and distinct from
    /// `Passed` because a report nobody measured cannot clear a gate.
    Unknown,
}

impl ValidationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }

    /// Whether this status may be reported as success.
    ///
    /// Callers must go through this rather than comparing against `Failed`: an
    /// `== Failed` test silently treats `Unknown` as a pass, which is the exact
    /// failure #3535 records.
    pub fn is_passed(&self) -> bool {
        matches!(self, Self::Passed)
    }
}

pub use catalog::{
    CheckModes, CheckOwner, CheckReachability, StructuralCheck, ValidationCatalog, ValidationCheck,
};
pub use command::ToolCommand;
pub use external::ExternalCheck;
pub use measurement::{require_tool, resolve_tool, Measurement};
pub use options::{Jobs, ValidationOptions};
pub use plan::{PlannedValidationCheck, ValidationPlan};
pub use results::{
    CheckResult, ValidationAggregate, ValidationExecutionAggregate, ValidationExecutionReport,
    ValidationObservation, ValidationReport,
};
pub use runner::ValidationRunner;
pub use structural::StructuralValidator;
