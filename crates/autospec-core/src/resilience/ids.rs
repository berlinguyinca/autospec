//! Durable identities for the resilient agent runtime.
//!
//! The work protocol deliberately does **not** overload a single identifier.
//! A `WorkId` names a durable unit of requested work; an `AttemptId` names one
//! execution attempt of that work; a `ClaimId` names an ownership lease; a
//! `SessionId` names a harness/model session; a `CheckpointId` names a durable
//! continuation point; a `ReceiptId` names an acknowledgement; and an
//! `IdempotencyKey` makes a delivery/claim idempotent.
//!
//! Keeping these distinct is what lets recovery distinguish "work exists" from
//! "an attempt ran" from "a worker owned it" from "a session did work" from
//! "a checkpoint is durable".

use serde::{Deserialize, Serialize};

/// Builds a stable, collision-resistant id from a namespace prefix.
///
/// The id is a hash of the prefix plus a caller-supplied nonce so the caller
/// controls entropy (and can seed deterministically in tests). The returned
/// string is lowercase hex and safe to embed in filenames, URLs and JSON.
pub fn make_id(prefix: &str, nonce: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(prefix.as_bytes());
    hasher.update(nonce);
    let digest = hasher.finalize();
    let mut out = String::from(prefix);
    out.push('-');
    for byte in digest.iter().take(12) {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// A durable unit of requested engineering work.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkId(pub String);

impl WorkId {
    pub fn new(nonce: &[u8]) -> Self {
        Self(make_id("work", nonce))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One execution attempt of a work item. Retries create new attempts while the
/// work identity is preserved.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AttemptId(pub String);

impl AttemptId {
    pub fn new(nonce: &[u8]) -> Self {
        Self(make_id("attempt", nonce))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An execution run of an attempt. Distinguishes the logical work and attempt
/// from the concrete execution that carried them out.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExecutionId(pub String);

impl ExecutionId {
    pub fn new(nonce: &[u8]) -> Self {
        Self(make_id("exec", nonce))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An ownership lease for an attempt/work item. A claim is a lease, not a
/// permanent flag: it carries an expiry and a fencing generation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClaimId(pub String);

impl ClaimId {
    pub fn new(nonce: &[u8]) -> Self {
        Self(make_id("claim", nonce))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A harness/model session.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub String);

impl SessionId {
    pub fn new(nonce: &[u8]) -> Self {
        Self(make_id("session", nonce))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A durable structured continuation checkpoint.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CheckpointId(pub String);

impl CheckpointId {
    pub fn new(nonce: &[u8]) -> Self {
        Self(make_id("checkpoint", nonce))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A durable Repository Attention Stream.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StreamId(pub String);

impl StreamId {
    pub fn new(nonce: &[u8]) -> Self {
        Self(make_id("stream", nonce))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An acknowledgement receipt (delivered / claimed / handled).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReceiptId(pub String);

impl ReceiptId {
    pub fn new(nonce: &[u8]) -> Self {
        Self(make_id("receipt", nonce))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A Verified Engineering Learning lesson candidate.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CandidateId(pub String);

impl CandidateId {
    pub fn new(nonce: &[u8]) -> Self {
        Self(make_id("lesson", nonce))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Makes a delivery/claim idempotent. The same idempotency key must be treated
/// as the same logical operation even if the message arrives twice.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IdempotencyKey(pub String);

impl IdempotencyKey {
    pub fn new(nonce: &[u8]) -> Self {
        Self(make_id("idem", nonce))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_namespaced_and_deterministic() {
        let nonce = b"fixed-nonce";
        let a = WorkId::new(nonce);
        let b = WorkId::new(nonce);
        assert_eq!(a, b);
        assert!(a.as_str().starts_with("work-"));
        assert!(AttemptId::new(nonce).as_str().starts_with("attempt-"));
        assert!(ExecutionId::new(nonce).as_str().starts_with("exec-"));
        assert!(ClaimId::new(nonce).as_str().starts_with("claim-"));
        assert!(SessionId::new(nonce).as_str().starts_with("session-"));
        assert!(CheckpointId::new(nonce).as_str().starts_with("checkpoint-"));
        assert!(ReceiptId::new(nonce).as_str().starts_with("receipt-"));
        assert!(StreamId::new(nonce).as_str().starts_with("stream-"));
        assert!(CandidateId::new(nonce).as_str().starts_with("lesson-"));
        assert!(IdempotencyKey::new(nonce).as_str().starts_with("idem-"));
    }

    #[test]
    fn ids_from_different_nonces_differ() {
        assert_ne!(WorkId::new(b"a"), WorkId::new(b"b"));
        assert_ne!(ClaimId::new(b"a"), ClaimId::new(b"b"));
    }

    #[test]
    fn id_types_are_not_conflated() {
        // The whole point of separate identities: same nonce yields different
        // concrete id types that never compare equal across kinds.
        let w = WorkId::new(b"x");
        let a = AttemptId::new(b"x");
        assert_ne!(w.0, a.0);
    }
}
