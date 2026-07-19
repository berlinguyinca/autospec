use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnsafeOperation {
    DestructiveGit,
    CredentialAccess,
    ProductionMutation,
    FilesystemDeletion,
    NetworkPublication,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetyError {
    pub operation: UnsafeOperation,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetyPolicy {
    pub safe_mode: bool,
}

impl Default for SafetyPolicy {
    fn default() -> Self {
        Self { safe_mode: true }
    }
}

impl SafetyPolicy {
    pub fn check(&self, command: &str) -> Result<(), SafetyError> {
        if !self.safe_mode {
            return Ok(());
        }
        let command = command.to_ascii_lowercase();
        let checks = [
            (
                UnsafeOperation::DestructiveGit,
                ["git reset --hard", "git push --force"].as_slice(),
            ),
            (
                UnsafeOperation::CredentialAccess,
                ["private key", "github_token", "aws_secret"].as_slice(),
            ),
            (
                UnsafeOperation::ProductionMutation,
                ["prod database", "production"].as_slice(),
            ),
            (
                UnsafeOperation::FilesystemDeletion,
                ["rm -rf", "unlink "].as_slice(),
            ),
            (
                UnsafeOperation::NetworkPublication,
                ["gh pr merge", "gh release upload"].as_slice(),
            ),
        ];

        for (operation, patterns) in checks {
            if patterns.iter().any(|pattern| command.contains(pattern)) {
                return Err(SafetyError {
                    operation,
                    message: "safe mode blocked unsafe operation".to_string(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStartGitExcludeOutcome {
    AlreadyPresent,
    Created,
    SkippedMissingInfoDir { debug_reason: String },
}

pub fn prepare_session_start_git_exclude(
    repo_root: &Path,
) -> std::io::Result<SessionStartGitExcludeOutcome> {
    let info_dir = repo_root.join(".git/info");
    if !info_dir.is_dir() {
        return Ok(SessionStartGitExcludeOutcome::SkippedMissingInfoDir {
            debug_reason: format!(
                "SessionStart hook skipped git exclude setup: {} is missing",
                info_dir.display()
            ),
        });
    }

    let exclude = info_dir.join("exclude");
    if exclude.exists() {
        return Ok(SessionStartGitExcludeOutcome::AlreadyPresent);
    }

    fs::File::create(exclude)?;
    Ok(SessionStartGitExcludeOutcome::Created)
}

pub fn redact_secrets(input: &str) -> String {
    input
        .split_whitespace()
        .map(redact_token)
        .collect::<Vec<_>>()
        .join(" ")
}

fn redact_token(token: &str) -> String {
    if is_github_token(token) {
        "[REDACTED_GITHUB_TOKEN]".to_string()
    } else if is_aws_key(token) {
        "[REDACTED_AWS_KEY]".to_string()
    } else {
        token.to_string()
    }
}

fn is_github_token(token: &str) -> bool {
    let prefixes = ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"];
    prefixes.iter().any(|prefix| token.starts_with(prefix)) && token.len() >= 40
}

fn is_aws_key(token: &str) -> bool {
    token.len() == 20
        && token.starts_with("AKIA")
        && token
            .chars()
            .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExploreVerifierOutcomeKind {
    NotRun,
    Failed,
    Verified,
}

impl ExploreVerifierOutcomeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotRun => "NotRun",
            Self::Failed => "Failed",
            Self::Verified => "Verified",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExploreVerifierOutcome {
    pub kind: ExploreVerifierOutcomeKind,
    pub reason: String,
    pub tier: String,
    pub cycle: u64,
    pub artifact_path: String,
    pub sealed: bool,
    pub dry: bool,
    pub may_mutate_github: bool,
}

impl ExploreVerifierOutcome {
    pub fn not_run(tier: impl Into<String>, cycle: u64, artifact_path: impl Into<String>) -> Self {
        Self::sealed(
            ExploreVerifierOutcomeKind::NotRun,
            "missing_AUTOSPEC_EXPLORE_VERIFY_CMD",
            tier,
            cycle,
            artifact_path,
        )
    }

    pub fn failed(
        reason: impl Into<String>,
        tier: impl Into<String>,
        cycle: u64,
        artifact_path: impl Into<String>,
    ) -> Self {
        Self::sealed(
            ExploreVerifierOutcomeKind::Failed,
            reason,
            tier,
            cycle,
            artifact_path,
        )
    }

    pub fn verified(tier: impl Into<String>, cycle: u64, artifact_path: impl Into<String>) -> Self {
        Self {
            kind: ExploreVerifierOutcomeKind::Verified,
            reason: "verified".to_string(),
            tier: tier.into(),
            cycle,
            artifact_path: artifact_path.into(),
            sealed: false,
            dry: false,
            may_mutate_github: true,
        }
    }

    fn sealed(
        kind: ExploreVerifierOutcomeKind,
        reason: impl Into<String>,
        tier: impl Into<String>,
        cycle: u64,
        artifact_path: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            reason: reason.into(),
            tier: tier.into(),
            cycle,
            artifact_path: artifact_path.into(),
            sealed: true,
            dry: false,
            may_mutate_github: false,
        }
    }
}

pub fn classify_explore_verifier_outcome(
    verify_command: Option<&str>,
    command_succeeded: Option<bool>,
    tier: impl Into<String>,
    cycle: u64,
    artifact_path: impl Into<String>,
) -> ExploreVerifierOutcome {
    let tier = tier.into();
    let artifact_path = artifact_path.into();
    let command = verify_command.unwrap_or_default().trim();
    if command.is_empty() {
        return ExploreVerifierOutcome::not_run(tier, cycle, artifact_path);
    }
    match command_succeeded {
        Some(true) => ExploreVerifierOutcome::verified(tier, cycle, artifact_path),
        Some(false) => ExploreVerifierOutcome::failed(
            "AUTOSPEC_EXPLORE_VERIFY_CMD_failed",
            tier,
            cycle,
            artifact_path,
        ),
        None => ExploreVerifierOutcome::failed(
            "AUTOSPEC_EXPLORE_VERIFY_CMD_not_executed",
            tier,
            cycle,
            artifact_path,
        ),
    }
}
