use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use autospec_core::autonomous::premerge::{
    evaluate_premerge, EvidenceAvailability, PremergeDecision, PremergeLaneIdentity, QaEvidence,
    SecurityAuditEvidence,
};

use super::{atomic_write, json_escape, Options, RunLayout};
use crate::commands::{claim, CommandFailure};

#[derive(Default)]
struct EvaluateOptions {
    repo: Option<String>,
    repo_dir: Option<PathBuf>,
    issue: Option<u64>,
    worker_id: Option<String>,
    claim_id: Option<String>,
    json: bool,
}

pub(super) fn run(args: &[String]) -> Result<(), CommandFailure> {
    match args {
        [command, rest @ ..] if command == "evaluate" => evaluate(rest),
        [] => Err(CommandFailure::diagnostic(
            "autospec autonomous premerge requires a subcommand",
        )),
        [command, ..] => Err(CommandFailure::diagnostic(format!(
            "unknown autospec autonomous premerge subcommand: {command}"
        ))),
    }
}

fn evaluate(args: &[String]) -> Result<(), CommandFailure> {
    let options = parse_evaluate_options(args)?;
    let repo = options.repo.as_deref().expect("validated --repo");
    let repo_dir = canonical_repo_dir(options.repo_dir.as_deref().expect("validated --repo-dir"))?;
    let branch = git_stdout(&repo_dir, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .map_err(|_| CommandFailure::diagnostic("premerge worktree is detached"))?;
    let commit = git_stdout(&repo_dir, &["rev-parse", "HEAD"])?;
    let status = git_stdout(
        &repo_dir,
        &["status", "--porcelain", "--untracked-files=no"],
    )?;
    if !status.is_empty() {
        return Err(CommandFailure::diagnostic(
            "premerge worktree has dirty tracked or staged changes",
        ));
    }

    let issue = options.issue.expect("validated --issue");
    let worker_id = options.worker_id.as_deref().expect("validated --worker-id");
    let claim_id = options.claim_id.as_deref().expect("validated --claim-id");
    let lane = PremergeLaneIdentity::new(repo, issue, worker_id, claim_id, &branch, &commit)
        .map_err(|error| CommandFailure::diagnostic(format!("invalid premerge lane: {error}")))?;
    if !claim::active_claim_generation_matches(repo, issue, worker_id, claim_id, &branch)? {
        return Err(CommandFailure::diagnostic(
            "premerge lane does not match the active claim generation",
        ));
    }

    let evidence_dir = repo_dir
        .join(".autospec/evidence/premerge")
        .join(lane.lane_digest());
    let qa = read_evidence(&evidence_dir.join("qa.json"), QaEvidence::parse)?;
    let security = read_evidence(
        &evidence_dir.join("security.json"),
        SecurityAuditEvidence::parse,
    )?;
    let decision = evaluate_premerge(&lane, qa, security);
    let document = decision_document(&decision);
    persist_decision(repo, &repo_dir, &decision, &document)?;
    emit_decision(&decision, &document, options.json)
}

fn parse_evaluate_options(args: &[String]) -> Result<EvaluateOptions, CommandFailure> {
    let mut options = EvaluateOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--repo" => set_string(&mut options.repo, args, &mut index, "--repo")?,
            "--repo-dir" => {
                if options.repo_dir.is_some() {
                    return Err(repeated("--repo-dir"));
                }
                let mut value = None;
                set_string(&mut value, args, &mut index, "--repo-dir")?;
                options.repo_dir = Some(PathBuf::from(value.expect("set string")));
            }
            "--issue" => {
                let value = take_value(args, &mut index, "--issue")?;
                if options.issue.is_some() {
                    return Err(repeated("--issue"));
                }
                options.issue = Some(value.parse().ok().filter(|issue| *issue > 0).ok_or_else(
                    || CommandFailure::diagnostic("--issue must be a positive integer"),
                )?);
            }
            "--worker-id" => set_string(&mut options.worker_id, args, &mut index, "--worker-id")?,
            "--claim-id" => set_string(&mut options.claim_id, args, &mut index, "--claim-id")?,
            "--json" => {
                if options.json {
                    return Err(repeated("--json"));
                }
                options.json = true;
                index += 1;
            }
            option => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec autonomous premerge evaluate option: {option}"
                )))
            }
        }
    }
    require(&options.repo, "--repo")?;
    require(&options.repo_dir, "--repo-dir")?;
    require(&options.issue, "--issue")?;
    require(&options.worker_id, "--worker-id")?;
    require(&options.claim_id, "--claim-id")?;
    Ok(options)
}

fn set_string(
    slot: &mut Option<String>,
    args: &[String],
    index: &mut usize,
    flag: &str,
) -> Result<(), CommandFailure> {
    if slot.is_some() {
        return Err(repeated(flag));
    }
    let value = take_value(args, index, flag)?;
    if value.is_empty() {
        return Err(CommandFailure::diagnostic(format!(
            "{flag} must not be empty"
        )));
    }
    *slot = Some(value.to_string());
    Ok(())
}

fn take_value<'a>(
    args: &'a [String],
    index: &mut usize,
    flag: &str,
) -> Result<&'a str, CommandFailure> {
    let value = args
        .get(*index + 1)
        .ok_or_else(|| CommandFailure::diagnostic(format!("{flag} requires a value")))?;
    *index += 2;
    Ok(value)
}

fn repeated(flag: &str) -> CommandFailure {
    CommandFailure::diagnostic(format!("{flag} may be specified only once"))
}

fn require<T>(value: &Option<T>, flag: &str) -> Result<(), CommandFailure> {
    value
        .as_ref()
        .map(|_| ())
        .ok_or_else(|| CommandFailure::diagnostic(format!("{flag} is required")))
}

fn canonical_repo_dir(path: &Path) -> Result<PathBuf, CommandFailure> {
    path.canonicalize().map_err(|error| {
        CommandFailure::diagnostic(format!(
            "cannot canonicalize premerge repo directory {}: {error}",
            path.display()
        ))
    })
}

fn git_stdout(repo_dir: &Path, args: &[&str]) -> Result<String, CommandFailure> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(args)
        .output()
        .map_err(|error| CommandFailure::diagnostic(format!("cannot run git: {error}")))?;
    if !output.status.success() {
        return Err(CommandFailure::diagnostic(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|_| CommandFailure::diagnostic("git output is not valid UTF-8"))
}

fn read_evidence<T>(
    path: &Path,
    parse: fn(&str) -> Result<T, String>,
) -> Result<EvidenceAvailability<T>, CommandFailure> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(EvidenceAvailability::Missing)
        }
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "cannot read fixed premerge evidence {}: {error}",
                path.display()
            )))
        }
    };
    let document = match String::from_utf8(bytes) {
        Ok(document) => document,
        Err(error) => return Ok(EvidenceAvailability::Malformed(error.to_string())),
    };
    Ok(match parse(&document) {
        Ok(evidence) => EvidenceAvailability::Present(evidence),
        Err(error) => EvidenceAvailability::Malformed(error),
    })
}

fn decision_document(decision: &PremergeDecision) -> String {
    let (name, lane, evidence_digest, reason, finding_codes) = match decision {
        PremergeDecision::Pass {
            lane,
            evidence_digest,
        } => ("pass", lane, evidence_digest, "", Vec::new()),
        PremergeDecision::Blocked {
            lane,
            reason,
            evidence_digest,
            quarantine,
        } => (
            "blocked",
            lane,
            evidence_digest,
            reason.as_str(),
            quarantine.finding_codes.clone(),
        ),
        PremergeDecision::Failed {
            lane,
            reason,
            evidence_digest,
        } => ("failed", lane, evidence_digest, reason.as_str(), Vec::new()),
    };
    let codes = finding_codes
        .iter()
        .map(|code| format!("\"{}\"", json_escape(code)))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"schema\":1,\"decision\":\"{name}\",\"repo\":\"{}\",\"issue\":{},\"worker_id\":\"{}\",\"claim_id\":\"{}\",\"branch\":\"{}\",\"commit\":\"{}\",\"lane_digest\":\"{}\",\"evidence_digest\":\"{}\",\"reason\":\"{}\",\"finding_codes\":[{codes}]}}\n",
        json_escape(&lane.repo),
        lane.issue,
        json_escape(&lane.worker_id),
        json_escape(&lane.claim_id),
        json_escape(&lane.branch),
        json_escape(&lane.commit),
        lane.lane_digest(),
        evidence_digest,
        json_escape(reason),
    )
}

fn persist_decision(
    repo: &str,
    repo_dir: &Path,
    decision: &PremergeDecision,
    document: &str,
) -> Result<(), CommandFailure> {
    let (lane, evidence_digest) = decision_identity(decision);
    let layout_options = Options {
        repo: repo.to_string(),
        repo_dir: repo_dir.display().to_string(),
        ..Options::default()
    };
    let layout = RunLayout::new(&layout_options).map_err(CommandFailure::diagnostic)?;
    let lane_dir = layout
        .state_dir
        .join("premerge/lanes")
        .join(lane.lane_digest());
    let decisions_dir = lane_dir.join("decisions");
    fs::create_dir_all(&decisions_dir).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "cannot create premerge decision directory {}: {error}",
            decisions_dir.display()
        ))
    })?;
    let decision_path = decisions_dir.join(format!("{evidence_digest}.json"));
    persist_immutable(&decision_path, document)?;
    let reread = fs::read_to_string(&decision_path).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "cannot re-read immutable premerge decision {}: {error}",
            decision_path.display()
        ))
    })?;
    if reread != document {
        return Err(CommandFailure::diagnostic(format!(
            "immutable premerge decision differs at {}",
            decision_path.display()
        )));
    }
    if matches!(decision, PremergeDecision::Blocked { .. }) {
        persist_immutable(&lane_dir.join("quarantine.json"), document)?;
    }
    let latest = format!(
        "{{\"schema\":1,\"decision_digest\":\"{evidence_digest}\",\"path\":\"decisions/{evidence_digest}.json\"}}\n"
    );
    atomic_write(&lane_dir.join("latest.json"), &latest).map_err(CommandFailure::diagnostic)
}

fn decision_identity(decision: &PremergeDecision) -> (&PremergeLaneIdentity, &str) {
    match decision {
        PremergeDecision::Pass {
            lane,
            evidence_digest,
        }
        | PremergeDecision::Blocked {
            lane,
            evidence_digest,
            ..
        }
        | PremergeDecision::Failed {
            lane,
            evidence_digest,
            ..
        } => (lane, evidence_digest),
    }
}

fn persist_immutable(path: &Path, contents: &str) -> Result<(), CommandFailure> {
    if path.exists() {
        return verify_immutable(path, contents);
    }
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("premerge-state");
    let staged = path.with_file_name(format!(".{filename}.{}.pending", std::process::id()));
    atomic_write(&staged, contents).map_err(CommandFailure::diagnostic)?;
    let linked = fs::hard_link(&staged, path);
    let _ = fs::remove_file(&staged);
    match linked {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            verify_immutable(path, contents)
        }
        Err(error) => Err(CommandFailure::diagnostic(format!(
            "cannot create immutable premerge state {}: {error}",
            path.display()
        ))),
    }
}

fn verify_immutable(path: &Path, contents: &str) -> Result<(), CommandFailure> {
    let existing = fs::read_to_string(path).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "cannot read immutable premerge state {}: {error}",
            path.display()
        ))
    })?;
    if existing == contents {
        return Ok(());
    }
    Err(CommandFailure::diagnostic(format!(
        "immutable premerge state already exists with different contents: {}",
        path.display()
    )))
}

fn emit_decision(
    decision: &PremergeDecision,
    document: &str,
    json: bool,
) -> Result<(), CommandFailure> {
    if json {
        print!("{document}");
    } else {
        println!("{}", decision_name(decision));
    }
    match decision {
        PremergeDecision::Pass { .. } => Ok(()),
        PremergeDecision::Blocked { .. } => Err(CommandFailure::status(String::new(), 20)),
        PremergeDecision::Failed { .. } => Err(CommandFailure::status(String::new(), 2)),
    }
}

fn decision_name(decision: &PremergeDecision) -> &'static str {
    match decision {
        PremergeDecision::Pass { .. } => "pass",
        PremergeDecision::Blocked { .. } => "blocked",
        PremergeDecision::Failed { .. } => "failed",
    }
}
