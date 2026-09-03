use std::fmt;

/// Every failure raised by the Code Intelligence Gateway.
///
/// The gateway is a supervisor around external language servers, so failures are
/// expected during normal operation. Callers distinguish `kind` to decide whether
/// a query may degrade to a structural/textual fallback (see `fallback`) or must
/// surface as a hard error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeIntelErrorKind {
    /// Malformed configuration, unknown key, or an invalid workspace identifier.
    Config,
    /// The requested workspace is unknown, expired, or has been invalidated.
    Workspace,
    /// The backend process failed, crashed, or returned an unusable payload.
    Backend,
    /// The backend does not implement the requested capability.
    Unsupported,
    /// The backend did not answer within its deadline.
    Timeout,
    /// The workspace exists but its index is not ready to answer semantic queries.
    NotReady,
    /// A mandatory workflow gate rejected the request.
    Gate,
}

impl CodeIntelErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Workspace => "workspace",
            Self::Backend => "backend",
            Self::Unsupported => "unsupported",
            Self::Timeout => "timeout",
            Self::NotReady => "not-ready",
            Self::Gate => "gate",
        }
    }

    /// Whether a failure of this kind may silently degrade to a lower-confidence
    /// source. Config and gate failures never degrade: they are operator errors.
    pub fn is_degradable(self) -> bool {
        matches!(
            self,
            Self::Backend | Self::Unsupported | Self::Timeout | Self::NotReady
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeIntelError {
    kind: CodeIntelErrorKind,
    message: String,
}

impl CodeIntelError {
    pub fn new(kind: CodeIntelErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self::new(CodeIntelErrorKind::Config, message)
    }

    pub fn workspace(message: impl Into<String>) -> Self {
        Self::new(CodeIntelErrorKind::Workspace, message)
    }

    pub fn backend(message: impl Into<String>) -> Self {
        Self::new(CodeIntelErrorKind::Backend, message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(CodeIntelErrorKind::Unsupported, message)
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(CodeIntelErrorKind::Timeout, message)
    }

    pub fn not_ready(message: impl Into<String>) -> Self {
        Self::new(CodeIntelErrorKind::NotReady, message)
    }

    pub fn gate(message: impl Into<String>) -> Self {
        Self::new(CodeIntelErrorKind::Gate, message)
    }

    pub fn kind(&self) -> CodeIntelErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn is_degradable(&self) -> bool {
        self.kind.is_degradable()
    }
}

impl fmt::Display for CodeIntelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.kind.as_str(), self.message)
    }
}

impl std::error::Error for CodeIntelError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_and_gate_failures_never_degrade() {
        assert!(!CodeIntelError::config("bad key").is_degradable());
        assert!(!CodeIntelError::gate("impact analysis missing").is_degradable());
    }

    #[test]
    fn backend_failures_degrade_to_fallback() {
        assert!(CodeIntelError::backend("server crashed").is_degradable());
        assert!(CodeIntelError::timeout("deadline exceeded").is_degradable());
        assert!(CodeIntelError::unsupported("no call hierarchy").is_degradable());
        assert!(CodeIntelError::not_ready("indexing").is_degradable());
    }

    #[test]
    fn display_prefixes_the_kind() {
        let error = CodeIntelError::workspace("issue-421 expired");

        assert_eq!(error.to_string(), "workspace: issue-421 expired");
        assert_eq!(error.message(), "issue-421 expired");
    }
}
