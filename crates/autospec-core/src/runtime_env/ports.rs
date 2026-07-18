use std::collections::BTreeMap;
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

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

pub struct PortReservation {
    claim: PortClaim,
    listener: Option<TcpListener>,
}

impl PortReservation {
    pub fn port(&self) -> u16 {
        self.claim.port
    }

    pub fn release_for_launch(&mut self) -> u16 {
        self.listener.take();
        self.port()
    }

    pub fn release_claim(self, registry: &mut PortRegistry) -> Result<(), PortClaimError> {
        self.claim.release(registry)
    }
}

pub fn reserve_loopback_port(
    registry: &mut PortRegistry,
    environment_id: &str,
    requested: Option<u16>,
    attempts: usize,
) -> Result<PortReservation, PortClaimError> {
    if let Some(port) = requested {
        return reserve_one(registry, environment_id, port);
    }
    for _ in 0..attempts {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|_| PortClaimError::new("PORT_BIND_UNAVAILABLE", 0, None))?;
        let port = listener
            .local_addr()
            .map_err(|_| PortClaimError::new("PORT_BIND_UNAVAILABLE", 0, None))?
            .port();
        if let Ok(claim) = registry.claim(environment_id, port) {
            return Ok(PortReservation {
                claim,
                listener: Some(listener),
            });
        }
    }
    Err(PortClaimError::new(
        "PORT_ALLOCATION_RETRIES_EXHAUSTED",
        0,
        None,
    ))
}

fn reserve_one(
    registry: &mut PortRegistry,
    environment_id: &str,
    port: u16,
) -> Result<PortReservation, PortClaimError> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|_| PortClaimError::new("PORT_BIND_UNAVAILABLE", port, None))?;
    let claim = registry.claim_fixed(environment_id, port)?;
    Ok(PortReservation {
        claim,
        listener: Some(listener),
    })
}

pub fn wait_for_loopback_bind(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
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

#[cfg(test)]
mod tests {
    use std::process::{Command, Stdio};

    use super::*;

    #[test]
    fn reservation_holds_loopback_until_the_launch_boundary() {
        let mut registry = PortRegistry::default();
        let mut reservation = reserve_loopback_port(&mut registry, "env-a", None, 5).unwrap();

        assert!(!python_bind(reservation.port()));
        let port = reservation.release_for_launch();
        assert!(python_bind(port));
    }

    fn python_bind(port: u16) -> bool {
        Command::new("python3")
            .args([
                "-c",
                "import socket,sys; s=socket.socket(); s.bind(('127.0.0.1', int(sys.argv[1])))",
                &port.to_string(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    }
}
