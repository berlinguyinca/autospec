//! `autospec.protocol` v1 — the Rust side of the shared protocol contract
//! (`docs/contracts/autospec-protocol-v1.md`).
//!
//! This module fails closed. A reader never guesses: an unsupported version,
//! an unknown required extension, a missing envelope field or a malformed one
//! is an error, and no part of the message is applied.

pub mod negotiate;
pub mod sequence;

use std::fmt;

/// Protocol major versions this build can read and write.
pub const PROTOCOL_VERSIONS: &[u32] = &[1];

/// Why a message or a negotiation was refused. Codes are stable machine
/// strings shared with the TypeScript reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    /// No common protocol version, or a message version this reader lacks.
    VersionUnsupported { detail: String },
    /// `required_extensions` names an extension this reader does not understand.
    UnknownRequiredField { name: String },
    /// A required envelope field is absent.
    MissingField { field: &'static str },
    /// An envelope field is present but has the wrong type or value.
    InvalidField { field: &'static str, reason: String },
    /// A message arrived ahead of the next expected sequence for its mission.
    SequenceGap { expected: u64, got: u64 },
    /// A message arrived behind the next expected sequence under a key that was
    /// never accepted there: that position is already taken by another message.
    SequenceConflict { expected: u64, got: u64 },
    /// An idempotency key reappeared at a different sequence in the same mission.
    IdempotencyKeyReused { key: String },
    /// An idempotency key reappeared at its own sequence with a different payload.
    IdempotencyConflict { key: String },
}

impl ProtocolError {
    /// The stable machine code.
    pub fn code(&self) -> &'static str {
        match self {
            ProtocolError::VersionUnsupported { .. } => "PROTOCOL_VERSION_UNSUPPORTED",
            ProtocolError::UnknownRequiredField { .. } => "UNKNOWN_REQUIRED_FIELD",
            ProtocolError::MissingField { .. } => "MISSING_FIELD",
            ProtocolError::InvalidField { .. } => "INVALID_FIELD",
            ProtocolError::SequenceGap { .. } => "SEQUENCE_GAP",
            ProtocolError::SequenceConflict { .. } => "SEQUENCE_CONFLICT",
            ProtocolError::IdempotencyKeyReused { .. } => "IDEMPOTENCY_KEY_REUSED",
            ProtocolError::IdempotencyConflict { .. } => "IDEMPOTENCY_CONFLICT",
        }
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtocolError::VersionUnsupported { detail } => {
                write!(f, "{}: {detail}", self.code())
            }
            ProtocolError::UnknownRequiredField { name } => {
                write!(
                    f,
                    "{}: required extension `{name}` is not understood",
                    self.code()
                )
            }
            ProtocolError::MissingField { field } => {
                write!(f, "{}: `{field}`", self.code())
            }
            ProtocolError::InvalidField { field, reason } => {
                write!(f, "{}: `{field}` {reason}", self.code())
            }
            ProtocolError::SequenceGap { expected, got }
            | ProtocolError::SequenceConflict { expected, got } => {
                write!(
                    f,
                    "{}: expected sequence {expected}, got {got}",
                    self.code()
                )
            }
            ProtocolError::IdempotencyKeyReused { key } => {
                write!(
                    f,
                    "{}: key `{key}` was accepted at a different sequence",
                    self.code()
                )
            }
            ProtocolError::IdempotencyConflict { key } => {
                write!(
                    f,
                    "{}: key `{key}` was redelivered with a different payload",
                    self.code()
                )
            }
        }
    }
}

impl std::error::Error for ProtocolError {}
