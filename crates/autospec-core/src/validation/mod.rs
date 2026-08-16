pub mod affected;
pub mod catalog;
pub mod command;
pub mod external;
pub mod options;
pub mod output_macros;
pub mod plan;
pub mod results;
pub mod runner;
pub mod reference_pointer;
pub mod structural;
pub mod structural_text;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationStatus {
    Passed,
    Failed,
}

impl ValidationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
        }
    }
}

pub use catalog::{
    CheckModes, CheckOwner, CheckReachability, StructuralCheck, ValidationCatalog, ValidationCheck,
};
pub use command::ToolCommand;
pub use external::ExternalCheck;
pub use options::{Jobs, ValidationOptions};
pub use plan::{PlannedValidationCheck, ValidationPlan};
pub use results::{
    CheckResult, ValidationAggregate, ValidationExecutionAggregate, ValidationExecutionReport,
    ValidationObservation, ValidationReport,
};
pub use runner::ValidationRunner;
pub use structural::StructuralValidator;
