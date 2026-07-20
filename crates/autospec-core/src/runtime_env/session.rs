use std::collections::BTreeMap;

use getrandom::fill;

use super::{RuntimeEnvError, SessionRecord};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub process_start: String,
}

impl ProcessIdentity {
    pub fn matches(&self, observed: &Self) -> bool {
        self.pid == observed.pid && self.process_start == observed.process_start
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReleaseDecision {
    KeepActive,
    TearDown,
}

#[derive(Default)]
pub struct SessionSet {
    records: BTreeMap<String, SessionRecord>,
}

impl SessionSet {
    pub fn register(&mut self, record: SessionRecord) {
        self.records.insert(record.session_id.clone(), record);
    }

    pub fn release(&mut self, session_id: &str) -> ReleaseDecision {
        self.records.remove(session_id);
        if self.records.is_empty() {
            ReleaseDecision::TearDown
        } else {
            ReleaseDecision::KeepActive
        }
    }
}

pub fn random_session_token() -> Result<String, RuntimeEnvError> {
    let mut bytes = [0_u8; 16];
    fill(&mut bytes).map_err(|error| {
        RuntimeEnvError::new(format!("could not generate runtime session token: {error}"))
    })?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
