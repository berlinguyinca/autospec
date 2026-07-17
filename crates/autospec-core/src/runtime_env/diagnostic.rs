use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IsolationDiagnostic {
    pub schema_version: u32,
    pub code: String,
    pub environment_id: String,
    pub resource: String,
    pub evidence: String,
    pub recovery_command: String,
}
