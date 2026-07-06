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
