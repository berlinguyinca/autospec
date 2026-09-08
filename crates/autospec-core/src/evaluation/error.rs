//! Typed errors for the evaluation module. Fail-closed semantics: callers
//! treat every `EvaluationError` as "not qualified", never as "assume ok".
use std::fmt;

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
