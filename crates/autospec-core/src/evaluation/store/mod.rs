//! Repo-local evaluation store under `.autospec/evaluation/`.
//!
//! This module owns the on-disk layout ([`layout`]) and the crash-safe write
//! helpers ([`io`]) that every later store component (journal, epoch
//! transition) builds on. Torn writes are the threat being designed out:
//!
//! - replaceable documents are written tmp + `sync_all` + `rename` +
//!   parent-directory `sync_all`;
//! - immutable documents are opened with `create_new`, so a second write to
//!   the same version is an [`EvaluationErrorKind::Immutable`] error naming
//!   the path;
//! - appended lines record their pre-append length and roll back with
//!   `set_len` when the write is incomplete (or an injected fault fires).
//!
//! Layout table and error kinds:
//! `docs/specs/2026-09-05-evaluator-coevolution-design.md` (Architecture /
//! Interfaces sections).

pub mod io;
pub mod journal;
pub mod layout;

use std::fmt;

/// Failure class for every evaluation-store operation.
///
/// Renders as the lowercase kebab-case token used on the CLI diagnostic
/// line (`<kind>: <message>`): `invariant`, `immutable`, `integrity`, `io`,
/// `parse`, `fail-closed`, `access-denied`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EvaluationErrorKind {
    /// A structural precondition of the store was violated (bad schema,
    /// version ordering, missing required field).
    Invariant,
    /// A write raced or repeated an immutable document.
    Immutable,
    /// A digest, chain, or checkpoint verification failed.
    Integrity,
    /// An underlying filesystem operation failed.
    Io,
    /// A stored document failed to parse as the expected JSON shape.
    Parse,
    /// Evidence is incomplete and the transition must not proceed.
    FailClosed,
    /// The caller's access role may not see this data.
    AccessDenied,
}

impl EvaluationErrorKind {
    /// The CLI diagnostic token for this kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Invariant => "invariant",
            Self::Immutable => "immutable",
            Self::Integrity => "integrity",
            Self::Io => "io",
            Self::Parse => "parse",
            Self::FailClosed => "fail-closed",
            Self::AccessDenied => "access-denied",
        }
    }
}

impl fmt::Display for EvaluationErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A typed evaluation-store failure. `kind` drives exit codes and the
/// diagnostic prefix; `message` names the path or field involved.
#[derive(Debug, Clone, PartialEq, Eq)]
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

    pub fn io(message: impl Into<String>) -> Self {
        Self::new(EvaluationErrorKind::Io, message)
    }

    pub fn fail_closed(message: impl Into<String>) -> Self {
        Self::new(EvaluationErrorKind::FailClosed, message)
    }
}

impl fmt::Display for EvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for EvaluationError {}

impl From<std::io::Error> for EvaluationError {
    fn from(error: std::io::Error) -> Self {
        Self::io(error.to_string())
    }
}

/// Convenience alias used throughout the store.
pub type Result<T> = std::result::Result<T, EvaluationError>;
