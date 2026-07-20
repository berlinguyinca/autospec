use std::fs::{self, File, OpenOptions};
use std::path::Path;
use std::process::Command;

use super::super::CommandFailure;

pub(super) struct ParentStateLock {
    _file: File,
}

impl ParentStateLock {
    pub(super) fn acquire(root: &Path) -> Result<Self, CommandFailure> {
        let directory = root.join(".autospec/state");
        fs::create_dir_all(&directory).map_err(|error| {
            CommandFailure::diagnostic(format!(
                "could not create parent state directory {}: {error}",
                directory.display()
            ))
        })?;
        let path = directory.join("parent.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| {
                CommandFailure::diagnostic(format!(
                    "could not open parent state lock {}: {error}",
                    path.display()
                ))
            })?;
        file.lock().map_err(|error| {
            CommandFailure::diagnostic(format!(
                "could not lock parent state {}: {error}",
                path.display()
            ))
        })?;
        Ok(Self { _file: file })
    }
}

pub(super) fn issue_field(repo: &str, issue: u64, field: &str) -> Result<String, CommandFailure> {
    gh_output(
        &[
            "issue".into(),
            "view".into(),
            issue.to_string(),
            "--repo".into(),
            repo.to_string(),
            "--json".into(),
            field.to_string(),
            "--jq".into(),
            format!(".{field}"),
        ],
        "read issue field",
    )
}

pub(super) fn find_trusted_marked_comment(
    repo: &str,
    issue: u64,
    marker: &str,
) -> Result<String, CommandFailure> {
    let actors = trusted_actors()?;
    let mut predicates = Vec::new();
    for actor in actors {
        predicates.push(format!(".author.login == \"{actor}\""));
    }
    if predicates.is_empty() {
        return Err(CommandFailure::diagnostic(
            "issue safety policy requires at least one trusted actor",
        ));
    }
    let marker = jq_string(marker);
    gh_output(
        &[
            "issue".into(),
            "view".into(),
            issue.to_string(),
            "--repo".into(),
            repo.to_string(),
            "--json".into(),
            "comments".into(),
            "--jq".into(),
            format!(
                "[.comments[] | select(({}) and (.body | contains({marker})))] | last | .body // \"\"",
                predicates.join(" or ")
            ),
        ],
        "read trusted marked issue comment",
    )
}

pub(super) fn issue_comment(repo: &str, issue: u64, body: &str) -> Result<(), CommandFailure> {
    require_trusted_writer()?;
    run_gh(
        &[
            "issue".into(),
            "comment".into(),
            issue.to_string(),
            "--repo".into(),
            repo.to_string(),
            "--body".into(),
            body.to_string(),
        ],
        "post parent lifecycle comment",
    )
}

pub(super) fn issue_close(repo: &str, issue: u64) -> Result<(), CommandFailure> {
    require_trusted_writer()?;
    run_gh(
        &[
            "issue".into(),
            "close".into(),
            issue.to_string(),
            "--repo".into(),
            repo.to_string(),
        ],
        "close completed parent issue",
    )
}

pub(super) fn issue_reopen(repo: &str, issue: u64) -> Result<(), CommandFailure> {
    require_trusted_writer()?;
    run_gh(
        &[
            "issue".into(),
            "reopen".into(),
            issue.to_string(),
            "--repo".into(),
            repo.to_string(),
        ],
        "reopen parent issue after child state changed",
    )
}

fn require_trusted_writer() -> Result<(), CommandFailure> {
    let actors = trusted_actors()?;
    let login = gh_output(
        &["api".into(), "user".into(), "--jq".into(), ".login".into()],
        "read authenticated GitHub actor",
    )?;
    if actors.iter().any(|actor| actor == &login) {
        Ok(())
    } else {
        Err(CommandFailure::diagnostic(format!(
            "authenticated GitHub actor {login} is not trusted for parent lifecycle comments"
        )))
    }
}

fn trusted_actors() -> Result<Vec<String>, CommandFailure> {
    let actors = super::super::lint::configured_safety_trusted_actors()?;
    for actor in &actors {
        if !is_github_login(actor) {
            return Err(CommandFailure::diagnostic(format!(
                "invalid trusted actor login in issue safety policy: {actor}"
            )));
        }
    }
    if actors.is_empty() {
        return Err(CommandFailure::diagnostic(
            "issue safety policy requires at least one trusted actor",
        ));
    }
    Ok(actors)
}

fn run_gh(arguments: &[String], action: &str) -> Result<(), CommandFailure> {
    gh_output(arguments, action).map(|_| ())
}

fn gh_output(arguments: &[String], action: &str) -> Result<String, CommandFailure> {
    let output = Command::new("gh")
        .args(arguments)
        .output()
        .map_err(|error| CommandFailure::diagnostic(format!("could not {action}: {error}")))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(CommandFailure::diagnostic(if detail.is_empty() {
            format!("could not {action}: gh exited with {}", output.status)
        } else {
            format!("could not {action}: {detail}")
        }))
    }
}

fn is_github_login(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 39
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn jq_string(value: &str) -> String {
    let mut rendered = String::from("\"");
    for character in value.chars() {
        match character {
            '\\' => rendered.push_str("\\\\"),
            '"' => rendered.push_str("\\\""),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            other => rendered.push(other),
        }
    }
    rendered.push('"');
    rendered
}
