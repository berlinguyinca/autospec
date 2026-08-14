// executor_bridge/continuation.rs — the continuation checkpoint: what the bridge records when
// an implementation cannot land as one pull request.
//
// Two things make a run continuable. Its closeout report lists acceptance criteria it did not
// meet, or its diff is larger than the patch-size gate accepts — usually both, because the way
// a run outgrows the gate is by trying to satisfy every criterion at once. Neither is a
// failure: the work that did land is real and must be preserved rather than discarded and
// retried, which is what an ordinary failure path would do.
//
// What preserves it is a receipt bound to the exact head OID that produced it, holding the
// criteria met and unmet and a digest over its own content. Binding to the head is what makes
// the record safe to recover: a receipt found on disk is only trusted when its identity and
// digest match the state being resumed, so a receipt left by a different head, a different
// issue, or a truncated write is rejected rather than adopted. Writing it create-once means a
// crash between decision and record cannot produce two disagreeing receipts.
//
// The continuation events are emitted exactly once each, which takes more than an append. The
// bridge may be restarted at any point, so each event carries a binding digest over the run
// identity, the receipt content and the initiating session; a lease plus an intent/complete
// marker pair makes a repeat of the same binding a no-op, and the log is re-read before
// appending in case the marker was lost but the entry survived.
//
// Filing the child issues that carry the unmet criteria forward is the other half, and lives
// in continuation_children.rs — it talks to GitHub, this file does not. That file depends on
// this one; this one does not depend on it.
//
// Split out of executor_bridge.rs, which is over the size limit and so may not grow.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ContinuationReceipt {
    pub(super) repository: String,
    pub(super) issue: u64,
    pub(super) base_oid: String,
    pub(super) head_oid: String,
    pub(super) status: String,
    pub(super) completed: Vec<String>,
    pub(super) unmet: Vec<String>,
    pub(super) content_digest: String,
}

impl ContinuationReceipt {
    pub(super) fn digest(&self) -> String {
        let mut content = self.clone();
        content.content_digest.clear();
        sha256_hex(content.to_json().as_bytes())
    }

    fn to_json(&self) -> String {
        serde_json::json!({
            "schema": 1,
            "repository": self.repository,
            "issue": self.issue,
            "base_oid": self.base_oid,
            "head_oid": self.head_oid,
            "status": self.status,
            "completed": self.completed,
            "unmet": self.unmet,
            "content_digest": self.content_digest,
        })
        .to_string()
    }

    fn from_json(body: &str) -> Result<Self, String> {
        let value: serde_json::Value = serde_json::from_str(body)
            .map_err(|error| format!("parse executor continuation receipt: {error}"))?;
        let fields =
            "schema repository issue base_oid head_oid status completed unmet content_digest"
                .split_whitespace()
                .collect::<Vec<_>>();
        let object = strict_object(value, &fields, "executor continuation receipt")?;
        if number(&object, "schema")? != 1 {
            return Err("executor continuation receipt schema is invalid".to_string());
        }
        let strings = |field| -> Result<Vec<String>, String> {
            required(&object, field)?
                .as_array()
                .ok_or_else(|| format!("executor continuation receipt {field} is not an array"))?
                .iter()
                .map(|value| {
                    value.as_str().map(str::to_string).ok_or_else(|| {
                        format!("executor continuation receipt {field} is not a string array")
                    })
                })
                .collect()
        };
        Ok(Self {
            repository: text(&object, "repository")?,
            issue: number(&object, "issue")?,
            base_oid: text(&object, "base_oid")?,
            head_oid: text(&object, "head_oid")?,
            status: text(&object, "status")?,
            completed: strings("completed")?,
            unmet: strings("unmet")?,
            content_digest: text(&object, "content_digest")?,
        })
    }
}

pub(super) fn continuation_receipt_path(
    state_path: &Path,
    head_oid: &str,
) -> Result<PathBuf, String> {
    if !canonical_git_oid(head_oid) {
        return Err("executor continuation receipt path requires a canonical head OID".to_string());
    }
    Ok(state_path.with_extension(format!("continuation.{head_oid}.json")))
}

pub(super) fn load_continuation_receipt(
    state_path: &Path,
    state: &PersistedInvocation,
) -> Result<ContinuationReceipt, String> {
    let path =
        continuation_receipt_path(state_path, state.head_oid.as_deref().unwrap_or_default())?;
    reject_symlink_path(&path)?;
    validate_private_state_file(&path)?;
    let receipt = ContinuationReceipt::from_json(
        &fs::read_to_string(path)
            .map_err(|error| format!("read executor continuation receipt: {error}"))?,
    )?;
    if receipt.repository != state.identity.repository
        || receipt.issue != state.identity.issue
        || receipt.base_oid != state.identity.base_oid
        || receipt.head_oid.as_str() != state.head_oid.as_deref().unwrap_or_default()
        || !canonical_git_oid(&receipt.base_oid)
        || !canonical_git_oid(&receipt.head_oid)
        || !matches!(receipt.status.as_str(), "planned" | "oversized_checkpoint")
        || receipt.content_digest != receipt.digest()
    {
        return Err("executor continuation receipt identity or content mismatch".to_string());
    }
    Ok(receipt)
}

pub(super) struct ContinuationCheckpoint {
    pub(super) receipt: ContinuationReceipt,
    receipt_path: PathBuf,
    recovered: bool,
    evaluation: PatchSizeEvaluation,
    invalid_exception: bool,
}

pub(super) fn prepare_continuation_checkpoint(
    state_path: &Path,
    state: &PersistedInvocation,
    proof: &ImplementationProof,
    issue_body: &str,
) -> Result<Option<ContinuationCheckpoint>, String> {
    let Some((completed, unmet)) = parse_closeout_criteria(&proof.closeout_body)? else {
        return Ok(None);
    };
    if unmet.is_empty() {
        return Ok(None);
    }
    let evaluation = exact_patch_size(state, &proof.head_oid)?;
    let admission = evaluate_patch_size_admission(state, &proof.head_oid, issue_body);
    let rejected = admission.is_err();
    let status = if evaluation.is_hard() && rejected {
        "oversized_checkpoint"
    } else if evaluation.is_hard() || !evaluation.is_proactive() {
        return Ok(None);
    } else {
        "planned"
    };
    let mut expected = ContinuationReceipt {
        repository: state.identity.repository.clone(),
        issue: state.identity.issue,
        base_oid: state.identity.base_oid.clone(),
        head_oid: proof.head_oid.clone(),
        status: status.to_string(),
        completed,
        unmet,
        content_digest: String::new(),
    };
    expected.content_digest = expected.digest();
    let path = continuation_receipt_path(state_path, &proof.head_oid)?;
    let recovered = path.exists();
    if recovered {
        if load_continuation_receipt(state_path, state)? != expected {
            return Err("executor continuation receipt exact-head content mismatch".to_string());
        }
    } else {
        write_private_create_once(
            &path,
            format!("{}\n", expected.to_json()).as_bytes(),
            "executor continuation receipt",
        )?;
    }
    let invalid_exception =
        evaluation.is_hard() && rejected && guardian_pr_size_attempt(issue_body);
    Ok(Some(ContinuationCheckpoint {
        receipt: expected,
        receipt_path: path,
        recovered,
        evaluation,
        invalid_exception,
    }))
}

pub(super) fn continuation_event_marker_path(
    state_path: &Path,
    binding: &str,
    stage: &str,
) -> PathBuf {
    state_path.with_extension(format!("continuation-event.{binding}.{stage}.json"))
}

pub(super) fn guardian_pr_size_attempt(issue_body: &str) -> bool {
    issue_body.lines().any(|line| {
        line.strip_prefix("Guardian: ")
            .and_then(|body| body.split_once(" # "))
            .is_some_and(|(skips, reason)| {
                !reason.trim().is_empty() && skips.split(", ").any(|skip| skip == "skip-PR_SIZE")
            })
    })
}

fn normalized_event_log(path: &Path) -> Result<(PathBuf, String), String> {
    reject_symlink_path(path)?;
    let absolute = std::path::absolute(path)
        .map_err(|error| format!("normalize executor event log: {error}"))?;
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component),
        }
    }
    let text = normalized
        .to_str()
        .ok_or_else(|| "executor event log path must be UTF-8".to_string())?;
    Ok((normalized.clone(), sha256_hex(text.as_bytes())))
}

pub(super) fn acquire_continuation_event_lease(path: &Path) -> Result<File, String> {
    reject_symlink_path(path)?;
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options
        .open(path)
        .map_err(|error| format!("open executor continuation event lock: {error}"))?;
    validate_private_state_file(path)?;
    file.try_lock()
        .map_err(|error| format!("lock executor continuation event: {error}"))?;
    Ok(file)
}

fn continuation_event_logged(path: &Path, binding: &str) -> Result<bool, String> {
    for segment in [path.to_path_buf(), path.with_extension("jsonl.1")] {
        reject_symlink_path(&segment)?;
        let body = match fs::read_to_string(&segment) {
            Ok(body) => body,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("read executor continuation event log: {error}")),
        };
        for line in body.lines() {
            let event: serde_json::Value = serde_json::from_str(line)
                .map_err(|error| format!("parse executor continuation event: {error}"))?;
            if event
                .get("continuation_binding")
                .and_then(serde_json::Value::as_str)
                == Some(binding)
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn emit_continuation_event_once(
    state_path: &Path,
    event_log: &Path,
    state: &PersistedInvocation,
    checkpoint: &ContinuationCheckpoint,
    event: &str,
) -> Result<(), String> {
    let (event_log, session_digest) = normalized_event_log(event_log)?;
    let session = serde_json::json!({
        "path": event_log,
        "digest": session_digest,
    });
    write_private_create_once(
        &state_path.with_extension("continuation-session.json"),
        session.to_string().as_bytes(),
        "executor continuation initiating session",
    )?;
    let binding = sha256_hex(
        serde_json::json!([
            state.identity.repository,
            state.identity.issue,
            checkpoint.receipt.base_oid,
            checkpoint.receipt.head_oid,
            checkpoint.receipt.content_digest,
            session_digest,
            event,
        ])
        .to_string()
        .as_bytes(),
    );
    let size = checkpoint.evaluation.size();
    let details = serde_json::json!({
        "continuation_binding": binding,
        "continuation_event": event,
        "receipt_path": checkpoint.receipt_path,
        "receipt_digest": checkpoint.receipt.content_digest,
        "base_oid": checkpoint.receipt.base_oid,
        "head_oid": checkpoint.receipt.head_oid,
        "unmet": checkpoint.receipt.unmet,
        "changed_lines": size.changed_lines,
        "raw_files": size.raw_files,
        "logical_units": size.logical_units,
        "has_binary": size.has_binary,
        "initiating_session_path": event_log,
        "initiating_session_digest": session_digest,
    });
    let intent = continuation_event_marker_path(state_path, &binding, "intent");
    let complete = continuation_event_marker_path(state_path, &binding, "complete");
    write_private_create_once(
        &intent,
        details.to_string().as_bytes(),
        "executor continuation event intent",
    )?;
    let _lease = acquire_continuation_event_lease(&continuation_event_marker_path(
        state_path, &binding, "lock",
    ))?;
    reject_symlink_path(&complete)?;
    if complete.exists() {
        return validate_cleanup_record(
            &complete,
            &binding,
            "executor continuation event complete",
        );
    }
    if !continuation_event_logged(&event_log, &binding)? {
        append_executor_event(&event_log, state, event, Some(details))?;
        File::open(
            event_log
                .parent()
                .ok_or_else(|| "executor continuation event log has no parent".to_string())?,
        )
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("sync executor continuation event log parent: {error}"))?;
    }
    ensure_cleanup_record(&complete, &binding, "executor continuation event complete")
}

pub(super) fn emit_continuation_events(
    state_path: &Path,
    event_log: &Path,
    state: &PersistedInvocation,
    checkpoint: &ContinuationCheckpoint,
) -> Result<(), String> {
    let primary = match checkpoint.receipt.status.as_str() {
        "planned" => "continuation_planned",
        _ => "continuation_oversized_checkpoint",
    };
    emit_continuation_event_once(state_path, event_log, state, checkpoint, primary)?;
    if checkpoint.invalid_exception {
        emit_continuation_event_once(
            state_path,
            event_log,
            state,
            checkpoint,
            "continuation_invalid_exception",
        )?;
    }
    if checkpoint.recovered {
        emit_continuation_event_once(
            state_path,
            event_log,
            state,
            checkpoint,
            "continuation_recovered",
        )?;
    }
    Ok(())
}
