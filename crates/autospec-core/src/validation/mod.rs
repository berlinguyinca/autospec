pub mod affected;
pub mod catalog;
pub mod command;
pub mod external;
pub mod options;
pub mod results;
pub mod runner;
pub mod structural;

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

pub use catalog::{CheckModes, CheckOwner, StructuralCheck, ValidationCatalog, ValidationCheck};
pub use command::ToolCommand;
pub use external::ExternalCheck;
pub use options::{Jobs, ValidationOptions};
pub use results::{
    CheckResult, ValidationAggregate, ValidationExecutionAggregate, ValidationExecutionReport,
    ValidationObservation, ValidationReport,
};
pub use runner::ValidationRunner;
pub use structural::StructuralValidator;
