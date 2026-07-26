use std::fs;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};

use autospec_core::autonomous::premerge::{
    evaluate_premerge, EvidenceAvailability, EvidenceVerdict, PremergeDecision,
    PremergeLaneIdentity, QaEvidence, SecurityAuditEvidence,
};

use super::{atomic_write, json_escape, Options, RunLayout};
use crate::commands::{claim, CommandFailure};

#[cfg(test)]
static COMPLETE_PUBLICATION_FAILPOINT: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
pub(super) fn set_complete_publication_failpoint(enabled: bool) {
    COMPLETE_PUBLICATION_FAILPOINT.store(enabled, Ordering::SeqCst);
}

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
        [command, rest @ ..] if command == "produce" => produce(rest),
        [] => Err(CommandFailure::diagnostic(
            "autospec autonomous premerge requires a subcommand",
        )),
        [command, ..] => Err(CommandFailure::diagnostic(format!(
            "unknown autospec autonomous premerge subcommand: {command}"
        ))),
    }
}

fn produce(args: &[String]) -> Result<(), CommandFailure> {
    let mut kind = None;
    let mut repo = None;
    let mut repo_dir = None;
    let mut issue = None;
    let mut worker_id = None;
    let mut claim_id = None;
    let mut run_id = None;
    let mut verdict = None;
    let mut reason = None;
    let mut finding_codes = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = args
            .get(index + 1)
            .ok_or_else(|| CommandFailure::diagnostic(format!("{flag} requires a value")))?;
        match flag {
            "--kind" => set_string(&mut kind, args, &mut index, flag)?,
            "--repo" => set_string(&mut repo, args, &mut index, flag)?,
            "--repo-dir" => {
                let mut value_slot = None;
                set_string(&mut value_slot, args, &mut index, flag)?;
                repo_dir = value_slot.map(PathBuf::from);
            }
            "--issue" => {
                issue = Some(
                    value
                        .parse()
                        .map_err(|_| CommandFailure::diagnostic("--issue must be positive"))?,
                );
                index += 2;
            }
            "--worker-id" => set_string(&mut worker_id, args, &mut index, flag)?,
            "--claim-id" => set_string(&mut claim_id, args, &mut index, flag)?,
            "--run-id" => set_string(&mut run_id, args, &mut index, flag)?,
            "--verdict" => set_string(&mut verdict, args, &mut index, flag)?,
            "--reason" => set_string(&mut reason, args, &mut index, flag)?,
            "--finding-code" => {
                finding_codes.push(value.to_string());
                index += 2;
            }
            unknown => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown produce option: {unknown}"
                )));
            }
        }
    }
    let kind = kind.ok_or_else(|| CommandFailure::diagnostic("--kind is required"))?;
    let repo = repo.ok_or_else(|| CommandFailure::diagnostic("--repo is required"))?;
    let repo_dir = canonical_repo_dir(
        &repo_dir.ok_or_else(|| CommandFailure::diagnostic("--repo-dir is required"))?,
    )?;
    let issue = issue.ok_or_else(|| CommandFailure::diagnostic("--issue is required"))?;
    let worker_id =
        worker_id.ok_or_else(|| CommandFailure::diagnostic("--worker-id is required"))?;
    let claim_id = claim_id.ok_or_else(|| CommandFailure::diagnostic("--claim-id is required"))?;
    let run_id = run_id.ok_or_else(|| CommandFailure::diagnostic("--run-id is required"))?;
    let verdict_name =
        verdict.ok_or_else(|| CommandFailure::diagnostic("--verdict is required"))?;
    if verdict_name == "pass" {
        return Err(CommandFailure::diagnostic(
            "premerge Pass requires observed bridge evidence; the external producer may only record blocked or failed evidence",
        ));
    }
    let branch = git_stdout(&repo_dir, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .map_err(|_| CommandFailure::diagnostic("premerge producer worktree is detached"))?;
    let commit = git_stdout(&repo_dir, &["rev-parse", "HEAD"])?;
    let lane = PremergeLaneIdentity::new(repo.clone(), issue, worker_id, claim_id, branch, commit)
        .map_err(CommandFailure::diagnostic)?;
    let evidence_verdict = match verdict_name.as_str() {
        "pass" => EvidenceVerdict::Pass,
        "blocked" => EvidenceVerdict::Blocked { finding_codes },
        "failed" => EvidenceVerdict::Failed {
            reason: reason.unwrap_or_else(|| "producer failed".to_string()),
        },
        _ => {
            return Err(CommandFailure::diagnostic(
                "--verdict must be pass, blocked, or failed",
            ));
        }
    };
    let json = match kind.as_str() {
        "qa" => QaEvidence {
            lane: lane.clone(),
            run_id: run_id.clone(),
            completed_at: unix_now(),
            verdict: evidence_verdict.clone(),
        }
        .to_json(),
        "security" => SecurityAuditEvidence {
            lane: lane.clone(),
            run_id,
            completed_at: unix_now(),
            verdict: evidence_verdict,
        }
        .to_json(),
        _ => return Err(CommandFailure::diagnostic("--kind must be qa or security")),
    };
    let directory = repo_dir
        .join(".autospec/evidence/premerge")
        .join(lane.lane_digest());
    fs::create_dir_all(&directory).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "cannot create producer evidence directory: {error}"
        ))
    })?;
    let filename = if kind == "qa" {
        "qa.json"
    } else {
        "security.json"
    };
    atomic_write(&directory.join(filename), &format!("{json}\n"))
        .map_err(CommandFailure::diagnostic)?;
    println!(
        "{{\"kind\":\"{}\",\"lane_digest\":\"{}\",\"path\":\"{}\"}}",
        kind,
        lane.lane_digest(),
        directory.join(filename).display()
    );
    Ok(())
}

pub(crate) fn persist_observed_bridge_evidence(
    repo_dir: &Path,
    bundle: &super::executor_bridge::ObservedEvidenceBundle,
) -> Result<PremergeDecision, String> {
    let qa = bundle.qa();
    let security = bundle.security();
    if qa.lane != security.lane {
        return Err("observed QA and security evidence lanes differ".to_string());
    }
    let live_decision = evaluate_premerge(
        &qa.lane,
        EvidenceAvailability::Present(qa.clone()),
        EvidenceAvailability::Present(security.clone()),
    );
    if !matches!(live_decision, PremergeDecision::Pass { .. }) {
        return Err("observed bridge evidence is not a live passing generation".to_string());
    }
    let repo_dir = fs::canonicalize(repo_dir)
        .map_err(|error| format!("canonicalize observed evidence repository: {error}"))?;
    let branch = git_stdout(&repo_dir, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .map_err(|error| error.message)?;
    let commit = git_stdout(&repo_dir, &["rev-parse", "HEAD"]).map_err(|error| error.message)?;
    if qa.lane.branch != branch || qa.lane.commit != commit {
        return Err("observed evidence lane drifted from the exact worktree commit".to_string());
    }
    let directory = repo_dir
        .join(".autospec/evidence/premerge")
        .join(qa.lane.lane_digest());
    if bundle.artifact_root() != directory {
        return Err("observed evidence bundle root differs from the exact lane root".to_string());
    }
    ensure_private_evidence_directory(&directory)?;
    let attempt_relative = bundle
        .attempt_root()
        .strip_prefix(&directory)
        .map_err(|_| "observed evidence attempt escapes the lane root".to_string())?;
    if attempt_relative
        .components()
        .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err("observed evidence attempt path is unsafe".to_string());
    }
    let observed_path = bundle.attempt_root().join("observed.json");
    let observed = fs::read_to_string(&observed_path)
        .map_err(|error| format!("read observed evidence manifest: {error}"))?;
    if observed != bundle.manifest_body()
        || autospec_core::autonomous::waterfall::sha256_hex(observed.as_bytes())
            != bundle.manifest_digest()
    {
        return Err("observed evidence manifest changed before persistence".to_string());
    }
    let qa_path = bundle.attempt_root().join("qa.json");
    let security_path = bundle.attempt_root().join("security.json");
    let qa_body = format!("{}\n", qa.to_json());
    let security_body = format!("{}\n", security.to_json());
    let cleanup_digest = bundle.cleanup_digest()?;
    super::executor_bridge::write_private_create_once(
        &qa_path,
        qa_body.as_bytes(),
        "attempt QA evidence",
    )?;
    super::executor_bridge::write_private_create_once(
        &security_path,
        security_body.as_bytes(),
        "attempt security evidence",
    )?;
    let seal = serde_json::json!({
        "schema": 1,
        "lane_digest": qa.lane.lane_digest(),
        "intent_digest": bundle.intent_digest(),
        "manifest_digest": bundle.manifest_digest(),
        "cleanup_digest": cleanup_digest,
        "qa_digest": autospec_core::autonomous::waterfall::sha256_hex(qa_body.as_bytes()),
        "security_digest": autospec_core::autonomous::waterfall::sha256_hex(security_body.as_bytes()),
    })
    .to_string();
    super::executor_bridge::write_private_create_once(
        &bundle.attempt_root().join("seal.json"),
        seal.as_bytes(),
        "attempt evidence seal",
    )?;
    archive_current_complete(&repo_dir, &qa.lane.lane_digest(), &directory)?;
    let generation = attempt_relative
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "observed evidence attempt has no generation".to_string())?;
    let complete = serde_json::json!({
        "schema": 2,
        "lane_digest": qa.lane.lane_digest(),
        "attempt_path": attempt_relative,
        "generation": generation,
        "seal_digest": autospec_core::autonomous::waterfall::sha256_hex(seal.as_bytes()),
    })
    .to_string();
    publish_complete_marker(&directory.join("complete.json"), &complete)?;
    #[cfg(test)]
    if COMPLETE_PUBLICATION_FAILPOINT.load(Ordering::SeqCst) {
        return Err("injected failure after complete marker fsync".to_string());
    }
    let (reread_qa, reread_security) =
        read_validated_bundle(&repo_dir, &qa.lane.lane_digest()).map_err(|error| error.message)?;
    if reread_qa != EvidenceAvailability::Present(qa.clone())
        || reread_security != EvidenceAvailability::Present(security.clone())
    {
        return Err(format!(
            "observed evidence changed after atomic persistence: qa={reread_qa:?} security={reread_security:?}"
        ));
    }
    let document = decision_document(&live_decision);
    persist_decision(&qa.lane.repo, &repo_dir, &live_decision, &document)
        .map_err(|error| error.message)?;
    Ok(live_decision)
}

fn archive_current_complete(
    repo_dir: &Path,
    lane_digest: &str,
    directory: &Path,
) -> Result<(), String> {
    let complete_path = directory.join("complete.json");
    if !complete_path
        .try_exists()
        .map_err(|error| format!("inspect current complete marker: {error}"))?
    {
        return Ok(());
    }
    let (qa, security) =
        read_validated_bundle(repo_dir, lane_digest).map_err(|error| error.message)?;
    if !matches!(qa, EvidenceAvailability::Present(_))
        || !matches!(security, EvidenceAvailability::Present(_))
    {
        return Err(format!(
            "current complete generation is malformed and cannot be archived: qa={qa:?} security={security:?}"
        ));
    }
    let body = fs::read_to_string(&complete_path)
        .map_err(|error| format!("read current complete marker for archive: {error}"))?;
    let value: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| format!("parse current complete marker for archive: {error}"))?;
    let attempt = value
        .get("attempt_path")
        .and_then(serde_json::Value::as_str)
        .and_then(|path| Path::new(path).file_name())
        .and_then(|value| value.to_str())
        .ok_or_else(|| "current complete marker has no generation".to_string())?;
    let completed = directory.join("completed");
    ensure_private_evidence_directory(&completed)?;
    super::executor_bridge::write_private_create_once(
        &completed.join(format!("{attempt}.json")),
        body.as_bytes(),
        "archived complete generation",
    )
}

fn publish_complete_marker(path: &Path, contents: &str) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(format!(
                    "observed evidence path is not a regular file: {}",
                    path.display()
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o077 != 0 {
                    return Err(format!(
                        "observed evidence path is not private: {}",
                        path.display()
                    ));
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return super::executor_bridge::write_private_create_once(
                path,
                contents.as_bytes(),
                "complete generation pointer",
            );
        }
        Err(error) => {
            return Err(format!(
                "inspect observed evidence path {}: {error}",
                path.display()
            ));
        }
    }
    let parent = path
        .parent()
        .ok_or_else(|| "complete generation pointer has no parent".to_string())?;
    let temporary = path.with_file_name(format!(".complete.{}.pending", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut file = options
        .open(&temporary)
        .map_err(|error| format!("create complete generation temporary file: {error}"))?;
    let result = (|| {
        file.write_all(contents.as_bytes())
            .map_err(|error| format!("write complete generation pointer: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("sync complete generation pointer: {error}"))?;
        fs::rename(&temporary, path)
            .map_err(|error| format!("publish complete generation pointer: {error}"))?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("sync complete generation parent: {error}"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn read_validated_bundle(
    repo_dir: &Path,
    lane_digest: &str,
) -> Result<
    (
        EvidenceAvailability<QaEvidence>,
        EvidenceAvailability<SecurityAuditEvidence>,
    ),
    CommandFailure,
> {
    let Some(complete_path) = fixed_evidence_path(repo_dir, lane_digest, "complete.json")? else {
        return Ok((
            EvidenceAvailability::Malformed(
                "observed evidence bundle marker is missing".to_string(),
            ),
            EvidenceAvailability::Malformed(
                "observed evidence bundle marker is missing".to_string(),
            ),
        ));
    };
    let complete: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&complete_path).map_err(|error| {
            CommandFailure::diagnostic(format!("read complete marker: {error}"))
        })?)
        .map_err(|error| CommandFailure::diagnostic(format!("parse complete marker: {error}")))?;
    let attempt = complete
        .get("attempt_path")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CommandFailure::diagnostic("complete marker has no attempt_path"))?;
    let attempt_root = repo_dir
        .join(".autospec/evidence/premerge")
        .join(lane_digest)
        .join(attempt);
    let qa = read_evidence_file(&attempt_root.join("qa.json"), QaEvidence::parse)?;
    let security = read_evidence_file(
        &attempt_root.join("security.json"),
        SecurityAuditEvidence::parse,
    )?;
    let validation =
        validate_complete_bundle(repo_dir, lane_digest, &complete_path, &qa, &security);
    if let Err(reason) = validation {
        return Ok((
            EvidenceAvailability::Malformed(reason.clone()),
            EvidenceAvailability::Malformed(reason),
        ));
    }
    Ok((qa, security))
}

fn read_evidence_file<T>(
    path: &Path,
    parse: fn(&str) -> Result<T, String>,
) -> Result<EvidenceAvailability<T>, CommandFailure> {
    validate_private_bundle_file(path).map_err(CommandFailure::diagnostic)?;
    let body = fs::read_to_string(path).map_err(|error| {
        CommandFailure::diagnostic(format!("read attempt-local evidence: {error}"))
    })?;
    Ok(match parse(&body) {
        Ok(value) => EvidenceAvailability::Present(value),
        Err(error) => EvidenceAvailability::Malformed(error),
    })
}

fn validate_complete_bundle(
    repo_dir: &Path,
    lane_digest: &str,
    complete_path: &Path,
    qa: &EvidenceAvailability<QaEvidence>,
    security: &EvidenceAvailability<SecurityAuditEvidence>,
) -> Result<(), String> {
    let complete_body = fs::read_to_string(complete_path)
        .map_err(|error| format!("read bundle marker: {error}"))?;
    let complete: serde_json::Value = serde_json::from_str(&complete_body)
        .map_err(|error| format!("parse bundle marker: {error}"))?;
    let complete_field = |name: &str| {
        complete
            .get(name)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("bundle marker is missing {name}"))
    };
    let schema = complete.get("schema").and_then(serde_json::Value::as_u64);
    if !matches!(schema, Some(1) | Some(2)) || complete_field("lane_digest")? != lane_digest {
        return Err("bundle marker schema or lane identity is invalid".to_string());
    }
    let qa = match qa {
        EvidenceAvailability::Present(qa) => qa,
        _ => return Err("bundle QA evidence is absent or malformed".to_string()),
    };
    let security = match security {
        EvidenceAvailability::Present(security) => security,
        _ => return Err("bundle security evidence is absent or malformed".to_string()),
    };
    let directory = repo_dir
        .join(".autospec/evidence/premerge")
        .join(lane_digest);
    let attempt_relative = Path::new(complete_field("attempt_path")?);
    if attempt_relative.is_absolute()
        || attempt_relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err("bundle attempt path escapes its lane".to_string());
    }
    let components = attempt_relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if components.len() != 2
        || components[0] != "attempts"
        || components[1].len() != 24
        || !components[1]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("bundle attempt path is not attempts/<24 lowercase hex>".to_string());
    }
    if schema == Some(2) && complete_field("generation")? != components[1] {
        return Err("bundle generation does not match its attempt selector".to_string());
    }
    let attempt_root = directory.join(attempt_relative);
    let intent_body = fs::read_to_string(attempt_root.join("intent.json"))
        .map_err(|error| format!("read attempt intent: {error}"))?;
    let intent_value: serde_json::Value = serde_json::from_str(&intent_body)
        .map_err(|error| format!("parse attempt intent: {error}"))?;
    let input_digest = intent_value
        .get("input_digest")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "attempt intent is missing input_digest".to_string())?;
    if input_digest.len() < 24 || components[1] != input_digest[..24] {
        return Err("bundle attempt selector does not match intent inputs/session".to_string());
    }
    let seal_path = attempt_root.join("seal.json");
    validate_private_bundle_file(&seal_path)?;
    let seal_body =
        fs::read_to_string(&seal_path).map_err(|error| format!("read attempt seal: {error}"))?;
    if complete_field("seal_digest")?
        != autospec_core::autonomous::waterfall::sha256_hex(seal_body.as_bytes())
    {
        return Err("attempt seal digest changed".to_string());
    }
    let seal: serde_json::Value =
        serde_json::from_str(&seal_body).map_err(|error| format!("parse attempt seal: {error}"))?;
    let field = |name: &str| {
        seal.get(name)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("attempt seal is missing {name}"))
    };
    if seal.get("schema").and_then(serde_json::Value::as_u64) != Some(1)
        || field("lane_digest")? != lane_digest
    {
        return Err("attempt seal schema or lane identity is invalid".to_string());
    }
    let qa_body = format!("{}\n", qa.to_json());
    let security_body = format!("{}\n", security.to_json());
    if field("qa_digest")? != autospec_core::autonomous::waterfall::sha256_hex(qa_body.as_bytes())
        || field("security_digest")?
            != autospec_core::autonomous::waterfall::sha256_hex(security_body.as_bytes())
    {
        return Err("attempt seal evidence digest changed".to_string());
    }
    let observed_path = attempt_root.join("observed.json");
    validate_private_bundle_file(&observed_path)?;
    let observed_body = fs::read_to_string(&observed_path)
        .map_err(|error| format!("read observed bundle manifest: {error}"))?;
    if field("manifest_digest")?
        != autospec_core::autonomous::waterfall::sha256_hex(observed_body.as_bytes())
    {
        return Err("bundle observed manifest digest changed".to_string());
    }
    let observed: serde_json::Value = serde_json::from_str(&observed_body)
        .map_err(|error| format!("parse observed bundle manifest: {error}"))?;
    if observed.get("schema").and_then(serde_json::Value::as_u64) != Some(1)
        || observed
            .get("lane_digest")
            .and_then(serde_json::Value::as_str)
            != Some(lane_digest)
        || observed
            .get("intent_digest")
            .and_then(serde_json::Value::as_str)
            != Some(field("intent_digest")?)
    {
        return Err("observed bundle manifest identity is invalid".to_string());
    }
    let cleanup_path = attempt_root.join("cleanup.json");
    validate_private_bundle_file(&cleanup_path)?;
    let cleanup_body = fs::read_to_string(&cleanup_path)
        .map_err(|error| format!("read runtime cleanup receipt: {error}"))?;
    if field("cleanup_digest")?
        != autospec_core::autonomous::waterfall::sha256_hex(cleanup_body.as_bytes())
    {
        return Err("runtime cleanup receipt digest changed".to_string());
    }
    let cleanup: serde_json::Value = serde_json::from_str(&cleanup_body)
        .map_err(|error| format!("parse runtime cleanup receipt: {error}"))?;
    if cleanup.get("schema").and_then(serde_json::Value::as_u64) != Some(1)
        || cleanup.get("cleanup").and_then(serde_json::Value::as_str) != Some("verified")
        || cleanup
            .get("intent_digest")
            .and_then(serde_json::Value::as_str)
            != Some(field("intent_digest")?)
        || cleanup
            .get("manifest_digest")
            .and_then(serde_json::Value::as_str)
            != Some(field("manifest_digest")?)
    {
        return Err("runtime cleanup receipt identity is invalid".to_string());
    }
    match (
        cleanup
            .get("runtime_session_id")
            .and_then(serde_json::Value::as_str),
        cleanup
            .get("runtime_environment_dir")
            .and_then(serde_json::Value::as_str),
    ) {
        (None, None) => {}
        (Some(session_id), Some(environment_dir)) => {
            let environment_dir = Path::new(environment_dir);
            if !environment_dir.is_absolute() {
                return Err("runtime cleanup locator is not absolute".to_string());
            }
            for path in [
                environment_dir.join(format!("sessions/{session_id}.json")),
                environment_dir.join(format!("sessions/{session_id}.lock")),
                environment_dir.join("owner.json"),
                environment_dir.join("plan.json"),
                environment_dir.join("env"),
                environment_dir.join("inventory.json"),
                environment_dir.join("sessions"),
            ] {
                if path
                    .try_exists()
                    .map_err(|error| format!("inspect runtime cleanup locator: {error}"))?
                {
                    return Err(format!(
                        "runtime cleanup locator still has authoritative resource {}",
                        path.display()
                    ));
                }
            }
        }
        _ => return Err("runtime cleanup locator/session pair is incomplete".to_string()),
    }
    let artifacts = observed
        .get("artifacts")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "observed bundle artifact inventory is missing".to_string())?;
    if artifacts.is_empty() {
        return Err("observed bundle artifact inventory is empty".to_string());
    }
    for artifact in artifacts {
        let relative = artifact
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "observed bundle artifact path is missing".to_string())?;
        let relative = Path::new(relative);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err("observed bundle artifact path escapes its lane".to_string());
        }
        let path = directory.join(relative);
        validate_private_bundle_file(&path)?;
        let digest = artifact
            .get("digest")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "observed bundle artifact digest is missing".to_string())?;
        let bytes =
            fs::read(&path).map_err(|error| format!("read observed bundle artifact: {error}"))?;
        if autospec_core::autonomous::waterfall::sha256_hex(&bytes) != digest {
            return Err("observed bundle artifact digest changed".to_string());
        }
    }
    super::executor_bridge::validate_persisted_observed_manifest(
        repo_dir,
        &directory,
        &attempt_root,
        &observed,
        qa,
        security,
        field("intent_digest")?,
    )?;
    Ok(())
}

fn validate_private_bundle_file(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect observed bundle artifact: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("observed bundle artifact is not a regular file".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("observed bundle artifact is not private".to_string());
        }
    }
    Ok(())
}

fn ensure_private_evidence_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|error| {
        format!(
            "create observed evidence directory {}: {error}",
            path.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut current = path.to_path_buf();
        for _ in 0..3 {
            fs::set_permissions(&current, fs::Permissions::from_mode(0o700)).map_err(|error| {
                format!(
                    "secure observed evidence directory {}: {error}",
                    current.display()
                )
            })?;
            let Some(parent) = current.parent() else {
                break;
            };
            current = parent.to_path_buf();
        }
    }
    Ok(())
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
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

    let lane_digest = lane.lane_digest();
    let mut qa = read_evidence(&repo_dir, &lane_digest, "qa.json", QaEvidence::parse)?;
    let mut security = read_evidence(
        &repo_dir,
        &lane_digest,
        "security.json",
        SecurityAuditEvidence::parse,
    )?;
    if matches!(
        evaluate_premerge(&lane, qa.clone(), security.clone()),
        PremergeDecision::Pass { .. }
    ) {
        (qa, security) = read_validated_bundle(&repo_dir, &lane_digest)?;
        if matches!(
            evaluate_premerge(&lane, qa.clone(), security.clone()),
            PremergeDecision::Pass { .. }
        ) {
            qa = EvidenceAvailability::Malformed(
                "standalone evidence replay is diagnostic-only and cannot originate admission authority"
                    .to_string(),
            );
            security = EvidenceAvailability::Malformed(
                "standalone evidence replay is diagnostic-only and cannot originate admission authority"
                    .to_string(),
            );
        }
    }
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
                )));
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
    repo_dir: &Path,
    lane_digest: &str,
    filename: &str,
    parse: fn(&str) -> Result<T, String>,
) -> Result<EvidenceAvailability<T>, CommandFailure> {
    let Some(path) = fixed_evidence_path(repo_dir, lane_digest, filename)? else {
        return Ok(EvidenceAvailability::Missing);
    };
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(EvidenceAvailability::Missing);
        }
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "cannot read fixed premerge evidence {}: {error}",
                path.display()
            )));
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

fn fixed_evidence_path(
    repo_dir: &Path,
    lane_digest: &str,
    filename: &str,
) -> Result<Option<PathBuf>, CommandFailure> {
    let mut path = repo_dir.to_path_buf();
    for component in [".autospec", "evidence", "premerge", lane_digest] {
        path.push(component);
        let Some(metadata) = evidence_metadata(&path)? else {
            return Ok(None);
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(unsafe_evidence_path(&path));
        }
    }
    path.push(filename);
    let Some(metadata) = evidence_metadata(&path)? else {
        return Ok(None);
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(unsafe_evidence_path(&path));
    }
    Ok(Some(path))
}

fn evidence_metadata(path: &Path) -> Result<Option<fs::Metadata>, CommandFailure> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CommandFailure::diagnostic(format!(
            "cannot inspect fixed premerge evidence path {}: {error}",
            path.display()
        ))),
    }
}

fn unsafe_evidence_path(path: &Path) -> CommandFailure {
    CommandFailure::diagnostic(format!(
        "fixed premerge evidence path is a symlink or non-regular component: {}",
        path.display()
    ))
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
