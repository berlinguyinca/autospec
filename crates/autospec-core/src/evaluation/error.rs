//! Structured errors for the evaluation subsystem.

use std::fmt;

/// Why an evaluation operation refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvaluationErrorKind {
    Invariant,
    Immutable,
    Integrity,
    Io,
    Parse,
    FailClosed,
    AccessDenied,
}

/// An evaluation failure carrying its [`EvaluationErrorKind`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvaluationError {
    pub kind: EvaluationErrorKind,
    pub message: String,
}

impl EvaluationError {
    pub fn new(kind: EvaluationErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
    pub fn invariant(m: impl Into<String>) -> Self {
        Self::new(EvaluationErrorKind::Invariant, m)
    }
    pub fn immutable(m: impl Into<String>) -> Self {
        Self::new(EvaluationErrorKind::Immutable, m)
    }
    pub fn integrity(m: impl Into<String>) -> Self {
        Self::new(EvaluationErrorKind::Integrity, m)
    }
    pub fn io(m: impl Into<String>) -> Self {
        Self::new(EvaluationErrorKind::Io, m)
    }
    pub fn parse(m: impl Into<String>) -> Self {
        Self::new(EvaluationErrorKind::Parse, m)
    }
    pub fn fail_closed(m: impl Into<String>) -> Self {
        Self::new(EvaluationErrorKind::FailClosed, m)
    }
    pub fn access_denied(m: impl Into<String>) -> Self {
        Self::new(EvaluationErrorKind::AccessDenied, m)
    }
}

impl fmt::Display for EvaluationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.kind {
            EvaluationErrorKind::Invariant => "invariant",
            EvaluationErrorKind::Immutable => "immutable",
            EvaluationErrorKind::Integrity => "integrity",
            EvaluationErrorKind::Io => "io",
            EvaluationErrorKind::Parse => "parse",
            EvaluationErrorKind::FailClosed => "fail-closed",
            EvaluationErrorKind::AccessDenied => "access-denied",
        };
        write!(f, "{kind}: {}", self.message)
    }
}

impl std::error::Error for EvaluationError {}

impl From<std::io::Error> for EvaluationError {
    fn from(e: std::io::Error) -> Self {
        Self::io(e.to_string())
    }
}

impl From<serde_json::Error> for EvaluationError {
    fn from(e: serde_json::Error) -> Self {
        Self::parse(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_the_kind() {
        assert_eq!(
            EvaluationError::fail_closed("lower_best_belief").to_string(),
            "fail-closed: lower_best_belief"
        );
        assert_eq!(
            EvaluationError::invariant("epoch went backwards").to_string(),
            "invariant: epoch went backwards"
        );
    }

    #[test]
    fn kinds_are_distinct() {
        assert_ne!(EvaluationErrorKind::Parse, EvaluationErrorKind::Io);
        assert_ne!(
            EvaluationErrorKind::Immutable,
            EvaluationErrorKind::Invariant
        );
    }
}
