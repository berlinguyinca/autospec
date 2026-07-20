use std::path::PathBuf;

use super::super::CommandFailure;

#[derive(Debug)]
pub(super) struct RecordOptions {
    pub(super) repo: String,
    pub(super) parent: u64,
    pub(super) children: Vec<u64>,
    pub(super) quarantined: bool,
    pub(super) state_root: PathBuf,
}

#[derive(Debug)]
pub(super) struct ReconcileOptions {
    pub(super) repo: String,
    pub(super) child: u64,
    pub(super) state_root: PathBuf,
}

#[derive(Debug)]
pub(super) struct SweepOptions {
    pub(super) repo: String,
    pub(super) state_root: PathBuf,
}

pub(super) fn parse_record(args: &[String]) -> Result<RecordOptions, CommandFailure> {
    let mut repo = None;
    let mut parent = None;
    let mut children = None;
    let mut quarantined = false;
    let mut state_root = default_state_root();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--repo" => repo = Some(next_value(args, &mut index, "--repo")?),
            "--parent" => parent = Some(parse_issue(next_value(args, &mut index, "--parent")?)?),
            "--children" => {
                children = Some(parse_children(&next_value(
                    args,
                    &mut index,
                    "--children",
                )?)?)
            }
            "--state-root" => {
                state_root = PathBuf::from(next_value(args, &mut index, "--state-root")?)
            }
            "--quarantined" => quarantined = true,
            other => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec parent record option: {other}"
                )))
            }
        }
        index += 1;
    }
    Ok(RecordOptions {
        repo: required(repo, "--repo")?,
        parent: required(parent, "--parent")?,
        children: required(children, "--children")?,
        quarantined,
        state_root,
    })
}

pub(super) fn parse_reconcile(args: &[String]) -> Result<ReconcileOptions, CommandFailure> {
    let mut repo = None;
    let mut child = None;
    let mut state_root = default_state_root();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--repo" => repo = Some(next_value(args, &mut index, "--repo")?),
            "--child" => child = Some(parse_issue(next_value(args, &mut index, "--child")?)?),
            "--state-root" => {
                state_root = PathBuf::from(next_value(args, &mut index, "--state-root")?)
            }
            other => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec parent reconcile-child option: {other}"
                )))
            }
        }
        index += 1;
    }
    Ok(ReconcileOptions {
        repo: required(repo, "--repo")?,
        child: required(child, "--child")?,
        state_root,
    })
}

pub(super) fn parse_sweep(args: &[String]) -> Result<SweepOptions, CommandFailure> {
    let mut repo = None;
    let mut state_root = default_state_root();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--repo" => repo = Some(next_value(args, &mut index, "--repo")?),
            "--state-root" => {
                state_root = PathBuf::from(next_value(args, &mut index, "--state-root")?)
            }
            other => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec parent sweep option: {other}"
                )))
            }
        }
        index += 1;
    }
    Ok(SweepOptions {
        repo: required(repo, "--repo")?,
        state_root,
    })
}

fn default_state_root() -> PathBuf {
    std::env::var_os("AUTOSPEC_PARENT_STATE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn next_value(args: &[String], index: &mut usize, option: &str) -> Result<String, CommandFailure> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| CommandFailure::diagnostic(format!("{option} requires a value")))
}

fn parse_issue(value: String) -> Result<u64, CommandFailure> {
    value
        .parse::<u64>()
        .ok()
        .filter(|issue| *issue > 0)
        .ok_or_else(|| CommandFailure::diagnostic("issue number must be positive"))
}

fn parse_children(value: &str) -> Result<Vec<u64>, CommandFailure> {
    let mut children = Vec::new();
    for item in value.split(',') {
        let issue = parse_issue(item.trim().to_string())?;
        if children.contains(&issue) {
            return Err(CommandFailure::diagnostic("child issues must be unique"));
        }
        children.push(issue);
    }
    if children.is_empty() {
        return Err(CommandFailure::diagnostic(
            "--children requires at least one issue",
        ));
    }
    Ok(children)
}

fn required<T>(value: Option<T>, option: &str) -> Result<T, CommandFailure> {
    value.ok_or_else(|| CommandFailure::diagnostic(format!("{option} is required")))
}
