pub mod affected;
pub mod catalog;
pub mod results;

use std::collections::BTreeMap;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationEntry {
    pub name: String,
    pub command: String,
    pub cwd: String,
    pub timeout_seconds: u64,
    pub required: bool,
}

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
    CheckModes, CheckOwner, StructuralCheck, ToolCommand, ValidationCatalog, ValidationCheck,
};
pub use results::{ValidationAggregate, ValidationObservation, ValidationReport};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationResult {
    pub name: String,
    pub status: ValidationStatus,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub required: bool,
}

#[derive(Debug, Default)]
pub struct ValidationRegistry {
    entries: BTreeMap<String, ValidationEntry>,
}

impl ValidationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        name: impl Into<String>,
        command: impl Into<String>,
        cwd: impl Into<String>,
        timeout_seconds: u64,
        required: bool,
    ) {
        let entry = ValidationEntry {
            name: name.into(),
            command: command.into(),
            cwd: cwd.into(),
            timeout_seconds,
            required,
        };
        self.entries.insert(entry.name.clone(), entry);
    }

    pub fn run(&self, name: &str) -> Result<ValidationResult, String> {
        let entry = self
            .entries
            .get(name)
            .ok_or_else(|| format!("unknown validation entry: {name}"))?;
        let output = Command::new("sh")
            .arg("-c")
            .arg(&entry.command)
            .current_dir(&entry.cwd)
            .output()
            .map_err(|error| format!("failed to run validation {name}: {error}"))?;

        Ok(ValidationResult {
            name: entry.name.clone(),
            status: if output.status.success() {
                ValidationStatus::Passed
            } else {
                ValidationStatus::Failed
            },
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            required: entry.required,
        })
    }
}
