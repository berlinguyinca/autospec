#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainlineHealthOutcome {
    Healthy,
    Blocked,
}

impl MainlineHealthOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Blocked => "blocked",
        }
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Blocked)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainlineHealthDiagnosticReason {
    BranchNotFound,
    DefaultBranchUnavailable,
    RequiredCheckPending,
    RequiredCheckFailed,
    GithubUnavailable,
}

impl MainlineHealthDiagnosticReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BranchNotFound => "branch-not-found",
            Self::DefaultBranchUnavailable => "default-branch-unavailable",
            Self::RequiredCheckPending => "required-check-pending",
            Self::RequiredCheckFailed => "required-check-failed",
            Self::GithubUnavailable => "github-unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainlineHealthCheckEvidence {
    pub name: String,
    pub status: String,
    pub conclusion: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainlineHealth {
    pub branch: String,
    pub checks: Vec<MainlineHealthCheckEvidence>,
    pub outcome: MainlineHealthOutcome,
    pub diagnostic_reason: Option<MainlineHealthDiagnosticReason>,
}

impl MainlineHealth {
    pub fn blocked(
        branch: impl Into<String>,
        checks: Vec<MainlineHealthCheckEvidence>,
        reason: MainlineHealthDiagnosticReason,
    ) -> Self {
        Self {
            branch: branch.into(),
            checks,
            outcome: MainlineHealthOutcome::Blocked,
            diagnostic_reason: Some(reason),
        }
    }

    pub fn healthy(branch: impl Into<String>, checks: Vec<MainlineHealthCheckEvidence>) -> Self {
        Self {
            branch: branch.into(),
            checks,
            outcome: MainlineHealthOutcome::Healthy,
            diagnostic_reason: None,
        }
    }
}

pub fn resolve_health_branch(
    explicit: Option<&str>,
    default_branch: Option<&str>,
) -> Result<String, MainlineHealthDiagnosticReason> {
    if let Some(branch) = explicit.map(str::trim).filter(|branch| !branch.is_empty()) {
        return Ok(branch.to_string());
    }
    if let Some(branch) = default_branch
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
    {
        return Ok(branch.to_string());
    }
    Err(MainlineHealthDiagnosticReason::DefaultBranchUnavailable)
}

pub fn classify_required_checks(
    branch: impl Into<String>,
    checks: Vec<MainlineHealthCheckEvidence>,
) -> MainlineHealth {
    let branch = branch.into();
    if checks.iter().any(|check| check.status != "completed") {
        return MainlineHealth::blocked(
            branch,
            checks,
            MainlineHealthDiagnosticReason::RequiredCheckPending,
        );
    }
    if checks.iter().any(|check| {
        !matches!(
            check.conclusion.as_str(),
            "" | "success" | "neutral" | "skipped"
        )
    }) {
        return MainlineHealth::blocked(
            branch,
            checks,
            MainlineHealthDiagnosticReason::RequiredCheckFailed,
        );
    }
    MainlineHealth::healthy(branch, checks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_branch_takes_precedence_over_default_branch() {
        assert_eq!(
            resolve_health_branch(Some("release"), Some("main")).unwrap(),
            "release"
        );
    }

    #[test]
    fn default_branch_is_used_when_override_is_absent() {
        assert_eq!(resolve_health_branch(None, Some("trunk")).unwrap(), "trunk");
    }

    #[test]
    fn missing_branch_records_branch_not_found_not_wait() {
        let health = MainlineHealth::blocked(
            "release",
            Vec::new(),
            MainlineHealthDiagnosticReason::BranchNotFound,
        );

        assert_eq!(health.outcome, MainlineHealthOutcome::Blocked);
        assert_eq!(
            health.diagnostic_reason,
            Some(MainlineHealthDiagnosticReason::BranchNotFound)
        );
    }

    #[test]
    fn pending_required_check_blocks_mainline_health() {
        let health = classify_required_checks(
            "main",
            vec![MainlineHealthCheckEvidence {
                name: "ci".to_string(),
                status: "in_progress".to_string(),
                conclusion: String::new(),
            }],
        );

        assert_eq!(
            health.diagnostic_reason,
            Some(MainlineHealthDiagnosticReason::RequiredCheckPending)
        );
    }

    #[test]
    fn failed_required_check_blocks_mainline_health() {
        let health = classify_required_checks(
            "main",
            vec![MainlineHealthCheckEvidence {
                name: "ci".to_string(),
                status: "completed".to_string(),
                conclusion: "failure".to_string(),
            }],
        );

        assert_eq!(
            health.diagnostic_reason,
            Some(MainlineHealthDiagnosticReason::RequiredCheckFailed)
        );
    }
}
