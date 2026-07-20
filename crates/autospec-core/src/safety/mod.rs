use std::fs;
use std::path::Path;

mod issue_promotion;

pub use issue_promotion::{
    evaluate_issue_promotion, evaluate_issue_promotion_with_trusted_actors, IssuePromotionDecision,
    IssuePromotionPayload, IssuePromotionSafetyDecision,
};

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
    let tokens = input.split_whitespace().collect::<Vec<_>>();
    let mut redacted = Vec::with_capacity(tokens.len());
    let mut index = 0;
    while index < tokens.len() {
        if tokens[index].eq_ignore_ascii_case("-----BEGIN") {
            let mut end = index + 1;
            while end < tokens.len() && !tokens[end].eq_ignore_ascii_case("-----END") {
                end += 1;
            }
            if end < tokens.len() {
                while end < tokens.len() && !tokens[end].ends_with("KEY-----") {
                    end += 1;
                }
                redacted.push("[REDACTED_PRIVATE_KEY]".to_string());
                index = (end + 1).min(tokens.len());
                continue;
            }
        }
        if tokens[index]
            .trim_end_matches(':')
            .eq_ignore_ascii_case("authorization")
            && tokens
                .get(index + 1)
                .is_some_and(|value| value.eq_ignore_ascii_case("bearer"))
            && index + 2 < tokens.len()
        {
            redacted.push(tokens[index].to_string());
            redacted.push(tokens[index + 1].to_string());
            redacted.push("[REDACTED]".to_string());
            index += 3;
            continue;
        }
        if tokens[index].eq_ignore_ascii_case("bearer") && index + 1 < tokens.len() {
            redacted.push(tokens[index].to_string());
            redacted.push("[REDACTED]".to_string());
            index += 2;
            continue;
        }
        redacted.push(redact_token(tokens[index]));
        index += 1;
    }
    redacted.join(" ")
}

fn redact_token(token: &str) -> String {
    if is_github_token(token) {
        "[REDACTED_GITHUB_TOKEN]".to_string()
    } else if is_aws_key(token) {
        "[REDACTED_AWS_KEY]".to_string()
    } else if let Some((key, _)) = token.split_once('=').or_else(|| token.split_once(':')) {
        if is_sensitive_key(key) {
            return format!("{key}=[REDACTED]");
        }
        token.to_string()
    } else {
        token.to_string()
    }
}

fn is_github_token(token: &str) -> bool {
    let prefixes = ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"];
    (prefixes.iter().any(|prefix| token.starts_with(prefix)) && token.len() >= 40)
        || (token.starts_with("github_pat_") && token.len() > "github_pat_".len())
}

fn is_aws_key(token: &str) -> bool {
    token.len() == 20
        && token.starts_with("AKIA")
        && token
            .chars()
            .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
}

fn is_sensitive_key(key: &str) -> bool {
    matches!(
        key.trim_matches(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .to_ascii_lowercase()
            .as_str(),
        "token"
            | "password"
            | "secret"
            | "api_key"
            | "apikey"
            | "authorization"
            | "credentials"
            | "user"
            | "username"
    )
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
