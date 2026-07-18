use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PortRegistry {
    pub schema_version: u32,
    claims: BTreeMap<u16, String>,
}

impl Default for PortRegistry {
    fn default() -> Self {
        Self {
            schema_version: 1,
            claims: BTreeMap::new(),
        }
    }
}

impl PortRegistry {
    pub fn claim(&mut self, environment_id: &str, port: u16) -> Result<PortClaim, PortClaimError> {
        if port == 0 {
            return Err(PortClaimError::new("PORT_INVALID", port, None));
        }
        match self.claims.get(&port) {
            Some(owner) if owner != environment_id => Err(PortClaimError::new(
                "PORT_ALREADY_CLAIMED",
                port,
                Some(owner.clone()),
            )),
            Some(_) => Ok(PortClaim::new(environment_id, port)),
            None => {
                self.claims.insert(port, environment_id.to_string());
                Ok(PortClaim::new(environment_id, port))
            }
        }
    }

    pub fn claim_fixed(
        &mut self,
        environment_id: &str,
        port: u16,
    ) -> Result<PortClaim, PortClaimError> {
        self.claim(environment_id, port)
    }

    pub fn owner(&self, port: u16) -> Option<&str> {
        self.claims.get(&port).map(String::as_str)
    }

    pub fn release_environment(&mut self, environment_id: &str) -> Vec<u16> {
        let ports = self
            .claims
            .iter()
            .filter_map(|(port, owner)| (owner == environment_id).then_some(*port))
            .collect::<Vec<_>>();
        for port in &ports {
            self.claims.remove(port);
        }
        ports
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortClaim {
    pub environment_id: String,
    pub port: u16,
}

impl PortClaim {
    fn new(environment_id: &str, port: u16) -> Self {
        Self {
            environment_id: environment_id.to_string(),
            port,
        }
    }

    pub fn release(self, registry: &mut PortRegistry) -> Result<(), PortClaimError> {
        match registry.claims.get(&self.port) {
            Some(owner) if owner == &self.environment_id => {
                registry.claims.remove(&self.port);
                Ok(())
            }
            owner => Err(PortClaimError::new(
                "PORT_CLAIM_OWNER_MISMATCH",
                self.port,
                owner.cloned(),
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortClaimError {
    pub code: &'static str,
    pub port: u16,
    pub owner: Option<String>,
}

impl PortClaimError {
    fn new(code: &'static str, port: u16, owner: Option<String>) -> Self {
        Self { code, port, owner }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GcOwnerSnapshot {
    pub environment_id: String,
    pub owner_key: String,
    pub recorded_generation: Option<String>,
    pub current_generation: Option<String>,
    pub worktree_exists: bool,
    pub locked_session_records: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GcInventorySnapshot {
    pub environment_id: String,
    pub docker_owner_keys: Vec<String>,
    pub live_environment_owners: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GcDecision {
    Delete,
    Retain(&'static str),
    Ambiguous(&'static str),
}

pub struct GcPolicy;

impl GcPolicy {
    pub fn evaluate(owner: &GcOwnerSnapshot, inventory: &GcInventorySnapshot) -> GcDecision {
        if inventory.environment_id != owner.environment_id
            || inventory
                .docker_owner_keys
                .iter()
                .any(|candidate| candidate != &owner.owner_key)
            || inventory
                .live_environment_owners
                .iter()
                .any(|candidate| candidate != &owner.owner_key)
        {
            return GcDecision::Ambiguous("RESOURCE_OWNER_MISMATCH");
        }
        if owner.locked_session_records > 0 {
            return GcDecision::Retain("LIVE_SESSION_RECORD");
        }
        if owner.worktree_exists
            && owner.current_generation.as_deref() == owner.recorded_generation.as_deref()
        {
            return GcDecision::Retain("WORKTREE_GENERATION_ACTIVE");
        }
        GcDecision::Delete
    }
}
