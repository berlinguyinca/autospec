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
    ListProjects {
        owner: String,
    },
    ListOwnerRepositories {
        owner: String,
        limit: usize,
    },
    ViewProject {
        owner: String,
        number: u64,
    },
    CreateProject {
        owner: String,
        title: String,
    },
    EditProjectMarker {
        owner: String,
        number: u64,
        readme: String,
    },
    ListProjectItems {
        owner: String,
        number: u64,
    },
    AddToProject {
        owner: String,
        project_number: u64,
        issue_url: String,
    },
}

impl GithubCommand {
    pub(crate) fn into_parts(self) -> (Vec<String>, Option<String>) {
        match self {
            Self::EnsureLabel { repository, name } => (
                vec![
                    "label".into(),
                    "create".into(),
                    name,
                    "--repo".into(),
                    repository,
                    "--color".into(),
                    "5319e7".into(),
                    "--force".into(),
                ],
                None,
            ),
            Self::ListAccountabilityIssues { repository } => (
                vec![
                    "api".into(),
                    "--paginate".into(),
                    "--slurp".into(),
                    format!(
                        "repos/{repository}/issues?state=all&labels=autospec%3Arun-accountability&per_page=100"
                    ),
                ],
                None,
            ),
            Self::ViewIssue { repository, number } => (
                vec![
                    "issue".into(),
                    "view".into(),
                    number.to_string(),
                    "--repo".into(),
                    repository,
                    "--json".into(),
                    "number,url,state,body,labels".into(),
                ],
                None,
            ),
            Self::CreateIssue {
                repository,
                title,
                body,
                labels,
            } => (
                vec![
                    "api".into(),
                    "--method".into(),
                    "POST".into(),
                    format!("repos/{repository}/issues"),
                    "--input".into(),
                    "-".into(),
                ],
                Some(serde_json::json!({"title": title, "body": body, "labels": labels}).to_string()),
            ),
            Self::EditIssue {
                repository,
                number,
                body,
            } => (
                vec![
                    "issue".into(),
                    "edit".into(),
                    number.to_string(),
                    "--repo".into(),
                    repository,
                    "--body-file".into(),
                    "-".into(),
                ],
                Some(body),
            ),
            Self::ReopenIssue { repository, number } => (
                vec![
                    "issue".into(),
                    "reopen".into(),
                    number.to_string(),
                    "--repo".into(),
                    repository,
                ],
                None,
            ),
            Self::CloseIssue { repository, number } => (
                vec![
                    "issue".into(),
                    "close".into(),
                    number.to_string(),
                    "--repo".into(),
                    repository,
                ],
                None,
            ),
            Self::ListProjects { owner } => (
                vec![
                    "api".into(),
                    "graphql".into(),
                    "--paginate".into(),
                    "--slurp".into(),
                    "-f".into(),
                    "query=query($owner:String!,$endCursor:String){repositoryOwner(login:$owner){... on Organization{projectsV2(first:100,after:$endCursor){nodes{number title}pageInfo{hasNextPage endCursor}}}... on User{projectsV2(first:100,after:$endCursor){nodes{number title}pageInfo{hasNextPage endCursor}}}}}".into(),
                    "-F".into(),
                    format!("owner={owner}"),
                ],
                None,
            ),
            Self::ListOwnerRepositories { owner, limit } => (
                vec![
                    "repo".into(),
                    "list".into(),
                    owner,
                    "--limit".into(),
                    limit.to_string(),
                    "--json".into(),
                    "nameWithOwner".into(),
                ],
                None,
            ),
            Self::ViewProject { owner, number } => (
                vec![
                    "project".into(),
                    "view".into(),
                    number.to_string(),
                    "--owner".into(),
                    owner,
                    "--format".into(),
                    "json".into(),
                ],
                None,
            ),
            Self::CreateProject { owner, title } => (
                vec![
                    "project".into(),
                    "create".into(),
                    "--owner".into(),
                    owner,
                    "--title".into(),
                    title,
                    "--format".into(),
                    "json".into(),
                ],
                None,
            ),
            Self::EditProjectMarker {
                owner,
                number,
                readme,
            } => (
                vec![
                    "project".into(),
                    "edit".into(),
                    number.to_string(),
                    "--owner".into(),
                    owner,
                    "--readme".into(),
                    readme,
                ],
                None,
            ),
            Self::ListProjectItems { owner, number } => (
                vec![
                    "project".into(),
                    "item-list".into(),
                    number.to_string(),
                    "--owner".into(),
                    owner,
                    "--format".into(),
                    "json".into(),
                    "--limit".into(),
                    "500".into(),
                ],
                None,
            ),
            Self::AddToProject {
                owner,
                project_number,
                issue_url,
            } => (
                vec![
                    "project".into(),
                    "item-add".into(),
                    project_number.to_string(),
                    "--owner".into(),
                    owner,
                    "--url".into(),
                    issue_url,
                ],
                None,
            ),
        }
    }
}

pub trait GithubTransport {
    fn execute(&mut self, command: GithubCommand) -> Result<String, GithubFailure>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GithubFailure {
    LocalExecution(String),
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
            Self::LocalExecution(message)
            | Self::Retryable(message)
            | Self::Ambiguous(message)
            | Self::Definitive(message) => formatter.write_str(message),
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
        GithubCommand::ListAccountabilityIssues { .. }
            | GithubCommand::ViewIssue { .. }
            | GithubCommand::ListProjects { .. }
            | GithubCommand::ListOwnerRepositories { .. }
            | GithubCommand::ViewProject { .. }
            | GithubCommand::ListProjectItems { .. }
    );
    let (args, stdin) = command.into_parts();
    let mut process =
        Command::new(std::env::var_os("AUTOSPEC_GH_PROGRAM").unwrap_or_else(|| "gh".into()));
    process
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin.is_some() {
        process.stdin(Stdio::piped());
    }
    let mut child = process
        .spawn()
        .map_err(|error| GithubFailure::LocalExecution(format!("cannot execute gh: {error}")))?;
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
        return Err(if definitive_gh_failure(&message) {
            GithubFailure::Definitive(message)
        } else if transient_gh_failure(&message) {
            if mutating {
                GithubFailure::Ambiguous(message)
            } else {
                GithubFailure::Retryable(message)
            }
        } else if mutating {
            GithubFailure::Ambiguous(message)
        } else {
            GithubFailure::Definitive(message)
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

fn transient_gh_failure(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        "connection refused",
        "connection reset",
        "could not resolve host",
        "failed to connect",
        "network is unreachable",
        "rate limit",
        "temporary failure",
        "timed out",
        "timeout",
        "http 429",
        "http 502",
        "http 503",
        "http 504",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub(super) fn json_error(error: serde_json::Error) -> AccountabilityError {
    AccountabilityError::new(format!("invalid GitHub response: {error}"))
}

#[allow(dead_code)]
fn _projection_type_anchor(_: &RenderedProjection) {}
