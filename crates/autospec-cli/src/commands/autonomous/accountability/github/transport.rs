use super::{AccountabilityError, RenderedProjection};
use std::fmt;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GithubCommand {
    EnsureLabel {
        repository: String,
        name: String,
    },
    ListAccountabilityIssues {
        repository: String,
    },
    ViewIssue {
        repository: String,
        number: u64,
    },
    CreateIssue {
        repository: String,
        title: String,
        body: String,
        labels: Vec<String>,
    },
    EditIssue {
        repository: String,
        number: u64,
        body: String,
    },
    ReopenIssue {
        repository: String,
        number: u64,
    },
    CloseIssue {
        repository: String,
        number: u64,
    },
    AddToProject {
        repository: String,
        project_number: u64,
        issue_url: String,
    },
}

pub trait GithubTransport {
    fn execute(&mut self, command: GithubCommand) -> Result<String, GithubFailure>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GithubFailure {
    Retryable(String),
    RetryAfter { message: String, delay: Duration },
    Ambiguous(String),
    Definitive(String),
}

impl GithubFailure {
    pub(super) fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Retryable(_) | Self::RetryAfter { .. } | Self::Ambiguous(_)
        )
    }

    pub(super) fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RetryAfter { delay, .. } => Some(*delay),
            _ => None,
        }
    }
}

impl fmt::Display for GithubFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Retryable(message) | Self::Ambiguous(message) | Self::Definitive(message) => {
                formatter.write_str(message)
            }
            Self::RetryAfter { message, .. } => formatter.write_str(message),
        }
    }
}

pub struct GhCli;

impl GithubTransport for GhCli {
    fn execute(&mut self, command: GithubCommand) -> Result<String, GithubFailure> {
        execute_gh(command)
    }
}

fn execute_gh(command: GithubCommand) -> Result<String, GithubFailure> {
    let mutating = !matches!(
        command,
        GithubCommand::ListAccountabilityIssues { .. } | GithubCommand::ViewIssue { .. }
    );
    let (args, stdin) = match command {
        GithubCommand::EnsureLabel { repository, name } => (
            vec![
                "label".to_string(),
                "create".to_string(),
                name,
                "--repo".to_string(),
                repository,
                "--color".to_string(),
                "5319e7".to_string(),
                "--force".to_string(),
            ],
            None,
        ),
        GithubCommand::ListAccountabilityIssues { repository } => (
            vec![
                "api".to_string(),
                "--paginate".to_string(),
                "--slurp".to_string(),
                format!(
                    "repos/{repository}/issues?state=all&labels=autospec%3Arun-accountability&per_page=100"
                ),
            ],
            None,
        ),
        GithubCommand::ViewIssue { repository, number } => (
            vec![
                "issue".to_string(),
                "view".to_string(),
                number.to_string(),
                "--repo".to_string(),
                repository,
                "--json".to_string(),
                "number,url,state,body,labels".to_string(),
            ],
            None,
        ),
        GithubCommand::CreateIssue {
            repository,
            title,
            body,
            labels,
        } => {
            let mut args = vec![
                "issue".to_string(),
                "create".to_string(),
                "--repo".to_string(),
                repository,
                "--title".to_string(),
                title,
                "--body-file".to_string(),
                "-".to_string(),
            ];
            for label in labels {
                args.push("--label".to_string());
                args.push(label);
            }
            (args, Some(body))
        }
        GithubCommand::EditIssue {
            repository,
            number,
            body,
        } => (
            vec![
                "issue".to_string(),
                "edit".to_string(),
                number.to_string(),
                "--repo".to_string(),
                repository,
                "--body-file".to_string(),
                "-".to_string(),
            ],
            Some(body),
        ),
        GithubCommand::ReopenIssue { repository, number } => (
            vec![
                "issue".to_string(),
                "reopen".to_string(),
                number.to_string(),
                "--repo".to_string(),
                repository,
            ],
            None,
        ),
        GithubCommand::CloseIssue { repository, number } => (
            vec![
                "issue".to_string(),
                "close".to_string(),
                number.to_string(),
                "--repo".to_string(),
                repository,
            ],
            None,
        ),
        GithubCommand::AddToProject {
            repository,
            project_number,
            issue_url,
        } => {
            let owner = repository.split('/').next().unwrap_or_default().to_owned();
            (
                vec![
                    "project".to_string(),
                    "item-add".to_string(),
                    project_number.to_string(),
                    "--owner".to_string(),
                    owner,
                    "--url".to_string(),
                    issue_url,
                ],
                None,
            )
        }
    };
    let mut process = Command::new("gh");
    process
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin.is_some() {
        process.stdin(Stdio::piped());
    }
    let mut child = process
        .spawn()
        .map_err(|error| GithubFailure::Retryable(format!("cannot execute gh: {error}")))?;
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .ok_or_else(|| GithubFailure::Ambiguous("cannot open gh stdin".to_string()))?
            .write_all(input.as_bytes())
            .map_err(|error| GithubFailure::Ambiguous(format!("cannot write gh stdin: {error}")))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| GithubFailure::Ambiguous(format!("cannot wait for gh: {error}")))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if let Some(delay) = parse_retry_after(&message) {
            return Err(GithubFailure::RetryAfter { message, delay });
        }
        return Err(if mutating {
            if definitive_gh_failure(&message) {
                GithubFailure::Definitive(message)
            } else {
                GithubFailure::Ambiguous(message)
            }
        } else {
            GithubFailure::Retryable(message)
        });
    }
    String::from_utf8(output.stdout)
        .map_err(|error| GithubFailure::Definitive(format!("gh returned invalid UTF-8: {error}")))
}

fn parse_retry_after(message: &str) -> Option<Duration> {
    message.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("retry-after")
            .then(|| value.trim().parse::<u64>().ok().map(Duration::from_secs))?
    })
}

fn definitive_gh_failure(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        "validation failed",
        "authentication",
        "forbidden",
        "http 400",
        "http 401",
        "http 403",
        "http 404",
        "unprocessable entity",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub(super) fn json_error(error: serde_json::Error) -> AccountabilityError {
    AccountabilityError::new(format!("invalid GitHub response: {error}"))
}

#[allow(dead_code)]
fn _projection_type_anchor(_: &RenderedProjection) {}
