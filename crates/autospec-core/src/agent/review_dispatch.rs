//! Typed construction of the Phase 5.5 broad-review harness invocation.
//!
//! Harness CLIs differ in their executable flags, but all of them must receive
//! the autospec slash command as one argument. Keeping that rule here prevents
//! shell-specific argument splitting from reintroducing the Claude failure.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessKind {
    Claude,
    Codex,
    OpenCode,
    Pi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDispatchOutcome {
    DispatchFailed,
    ZeroGapsEmitted,
    GapsEmitted,
}

/// Build argv for a broad review without splitting slash-command options.
pub fn build_review_argv(kind: HarnessKind, since: &str, gaps_file: &str) -> Vec<String> {
    let command = format!("/autospec-review --remediation --since {since} --emit-gaps {gaps_file}");
    let executable = match kind {
        HarnessKind::Claude => "claude",
        HarnessKind::Codex => "codex",
        HarnessKind::OpenCode => "opencode",
        HarnessKind::Pi => "pi",
    };

    match kind {
        HarnessKind::Codex => vec![
            executable.to_string(),
            "exec".to_string(),
            "--skip-git-repo-check".to_string(),
            command,
        ],
        HarnessKind::Claude | HarnessKind::OpenCode | HarnessKind::Pi => {
            vec![executable.to_string(), command]
        }
    }
}
