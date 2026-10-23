//! `autospec review-quorum --input FILE [--json]` evaluates a head-bound post-PR
//! review quorum (T06). The input is `{"target", "head", "policy", "votes"}`; the votes are
//! built from validated independent-review receipts (`review_commit` ->
//! `commit`, `verdict_digest` -> `receipt_digest`). Exit 0 satisfied, 1
//! insufficient (fresh review needed), 3 blocked, 2 invalid input.
//!
//! `merge-gate` is `verify-report` in strict mode, for the step that actually merges: it needs
//! `--pr` and `--repo`, treats "no policy stated" as required, demands origin proof for a required
//! quorum, and prints `merge_head` (the live head the gate verified, only on success) for the
//! caller to pass to `gh pr merge --match-head-commit`. A failure also clears the stale
//! `autospec:verified` / `autospec:needs-human-review` labels.
//!
//! `verify-report` is the merge gate's check of the report `--write-report` produced: it
//! binds the report to the LIVE head and target of the PR. `publish` / `request` are the
//! Herdr producer and consumer (see `herdr.rs`); `request` exits 4 when Herdr cannot vouch
//! for the head or author (an operator must look).

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;

pub(crate) mod gate;
mod herdr;

use gate::REPORT_SOURCE;

use autospec_core::autonomous::review_quorum::{
    evaluate_quorum, QuorumOutcome, QuorumPolicy, QuorumStatus, QuorumVote,
};
use serde::Deserialize;

const MAX_INPUT_BYTES: u64 = 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QuorumInput {
    target: String,
    head: String,
    policy: QuorumPolicy,
    votes: Vec<QuorumVote>,
}

pub fn run(args: &[String]) -> Result<i32, String> {
    match args.first().map(String::as_str) {
        Some("publish") => return publish(&args[1..]),
        Some("request") => return request(&args[1..]),
        Some("verify-report") => return verify_report(&args[1..], false),
        Some("merge-gate") => return verify_report(&args[1..], true),
        _ => {}
    }
    let mut input = None;
    let mut json = false;
    let mut report = None;
    let mut operator_config = None;
    let mut repo_root = None;
    let mut repo = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--input" => input = Some(iter.next().ok_or("--input requires a file")?.clone()),
            "--json" => json = true,
            "--write-report" => {
                report = Some(iter.next().ok_or("--write-report requires a path")?.clone())
            }
            "--operator-config" => {
                operator_config = Some(PathBuf::from(
                    iter.next().ok_or("--operator-config requires a path")?,
                ))
            }
            "--repo-root" => {
                repo_root = Some(PathBuf::from(
                    iter.next().ok_or("--repo-root requires a directory")?,
                ))
            }
            "--repo" => repo = Some(iter.next().ok_or("--repo requires OWNER/REPO")?.clone()),
            other => return Err(format!("unknown review-quorum argument: {other}")),
        }
    }
    let path = input.ok_or("review-quorum requires --input FILE")?;
    // The attestation key (if the trusted config names one) is resolved before evaluating, so
    // a misconfigured key also overwrites the old report with a non-pass one.
    let key = if report.is_some() {
        match attestation_key(
            repo_root.as_deref(),
            operator_config.as_deref(),
            repo.as_deref(),
        ) {
            Ok(key) => key,
            Err(error) => {
                if let Some(report) = &report {
                    write_error_report(report, &path, &error).map_err(|e| {
                        format!("{error}; also could not write the failure report: {e}")
                    })?;
                }
                return Err(error);
            }
        }
    } else {
        None
    };
    let (target, outcome, votes_digest) = match evaluate_file(&path) {
        Ok(ok) => ok,
        Err(error) => {
            // The gate must never keep trusting an OLDER pass because this evaluation failed:
            // overwrite the report with a non-pass one before reporting the error.
            if let Some(report) = &report {
                write_error_report(report, &path, &error).map_err(|e| {
                    format!("{error}; also could not write the failure report: {e}")
                })?;
            }
            return Err(error);
        }
    };
    if let Some(report) = report {
        write_promotion_report(&report, &target, &outcome, &votes_digest, key.as_deref())?;
    }
    if json {
        println!(
            "{}",
            serde_json::to_string(&outcome).map_err(|e| e.to_string())?
        );
    } else {
        println!(
            "review quorum {:?} at {}: {}/{} counted, {} blocking, {} stale",
            outcome.status,
            outcome.head,
            outcome.counted.len(),
            outcome.required,
            outcome.blocking.len(),
            outcome.stale_reviewers.len()
        );
    }
    Ok(match outcome.status {
        QuorumStatus::Satisfied => 0,
        QuorumStatus::Insufficient => 1,
        QuorumStatus::Blocked => 3,
    })
}

/// The attestation key named by the TRUSTED config (operator file or the default branch's),
/// never by the checkout under review.
fn attestation_key(
    repo_root: Option<&std::path::Path>,
    operator: Option<&std::path::Path>,
    repo: Option<&str>,
) -> Result<Option<Vec<u8>>, String> {
    let root = repo_root.map_or_else(|| PathBuf::from("."), PathBuf::from);
    // Without --repo there is no default branch to ask; the operator's config still decides.
    let base = repo.map_or(gate::Base::None, |r| gate::base_for(Some(r), &gate::Gh::default()));
    let trusted = gate::trusted(&root, operator, &base);
    match trusted.key_file {
        None => Ok(None),
        Some(file) => gate::load_attestation_key(&file, Some(&root)).map(Some),
    }
}

fn evaluate_file(path: &str) -> Result<(String, QuorumOutcome, String), String> {
    let file = fs::File::open(path).map_err(|e| format!("open review quorum input: {e}"))?;
    let mut raw = String::new();
    file.take(MAX_INPUT_BYTES + 1)
        .read_to_string(&mut raw)
        .map_err(|e| format!("read review quorum input: {e}"))?;
    if raw.len() as u64 > MAX_INPUT_BYTES {
        return Err("review quorum input is too large".to_string());
    }
    let parsed: QuorumInput =
        serde_json::from_str(&raw).map_err(|e| format!("parse review quorum input: {e}"))?;
    let outcome = evaluate_quorum(&parsed.target, &parsed.head, &parsed.votes, &parsed.policy)?;
    // sorted-key JSON of the votes as given: what the attestation binds the verdict to
    let votes: serde_json::Value = serde_json::from_str::<serde_json::Value>(&raw)
        .map_err(|e| e.to_string())?["votes"]
        .clone();
    let digest = autospec_core::autonomous::review_quorum::sha256_hex(votes.to_string().as_bytes());
    Ok((parsed.target, outcome, digest))
}

/// Write the report `autospec-promote-pr.sh` already consumes (`review-quorum.json`), so the
/// merge path gates on THIS evaluator without any change to the shell: a non-pass verdict or
/// any blocking finding blocks promotion. Run it after the feature-scoped Python quorum, which
/// writes the same file.
fn write_promotion_report(
    path: &str,
    target: &str,
    outcome: &QuorumOutcome,
    votes_digest: &str,
    key: Option<&[u8]>,
) -> Result<(), String> {
    let (verdict, ready) = match outcome.status {
        QuorumStatus::Satisfied => ("pass", true),
        QuorumStatus::Blocked => ("blocked", false),
        QuorumStatus::Insufficient => ("insufficient_reviews", false),
    };
    let blocking: Vec<_> = outcome
        .blocking
        .iter()
        .map(|who| serde_json::json!({"specialist": who, "verdict": "blocked"}))
        .collect();
    let target = autospec_core::autonomous::review_quorum::canonical_target(target)
        .unwrap_or_else(|| target.to_string());
    let evaluated_at = gate::evaluated_now();
    let mut body = serde_json::json!({"schema": 1, "source": REPORT_SOURCE, "target_id": target,
        "head": outcome.head, "verdict": verdict, "promotion_ready": ready, "blocking_findings": blocking,
        "counted": outcome.counted, "rereview_required": outcome.rereview_required,
        "evaluated_at": evaluated_at, "votes_digest": votes_digest});
    if let Some(key) = key {
        let message = autospec_core::autonomous::review_quorum::report_attestation_message(
            &target,
            &outcome.head,
            verdict,
            evaluated_at,
            votes_digest,
        );
        body["hmac"] = serde_json::Value::String(
            autospec_core::autonomous::review_quorum::hmac_sha256_hex(key, &message),
        );
    }
    write_atomic(path, &body)
}

/// Temp file + rename, so a reader never sees half a report and a crash leaves the old one.
fn write_atomic(path: &str, body: &serde_json::Value) -> Result<(), String> {
    let path = std::path::Path::new(path);
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| format!("create report dir: {e}"))?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&tmp, format!("{body}\n")).map_err(|e| format!("write report: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("commit report: {e}"))
}

/// The report for a failed evaluation: never a pass, carries the head/target when the input
/// names them, so `verify-report` can still say which PR state it describes.
fn write_error_report(path: &str, input: &str, error: &str) -> Result<(), String> {
    let named: Option<serde_json::Value> = fs::read_to_string(input)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok());
    let field = |name: &str| named.as_ref().and_then(|v| v[name].as_str());
    let head = field("head").and_then(autospec_core::autonomous::review_quorum::canonical_commit);
    let target =
        field("target").and_then(autospec_core::autonomous::review_quorum::canonical_target);
    let body = serde_json::json!({"schema": 1, "source": REPORT_SOURCE, "target_id": target,
        "head": head, "verdict": "invalid_input", "promotion_ready": false,
        "blocking_findings": [{"specialist": "review-quorum-evaluator", "verdict": "error", "detail": error}],
        "error": error});
    write_atomic(path, &body)
}

/// `verify-report` / `merge-gate`: may the merge path trust `review-quorum.json`? See
/// `gate.rs`. Exit 0 yes, 1 no (reasons as JSON), 2 usage.
fn verify_report(args: &[String], strict: bool) -> Result<i32, String> {
    let f = flags(
        args,
        &[
            "repo-root",
            "report",
            "pr",
            "repo",
            "head",
            "required",
            "no-live",
            "operator-config",
            "scratch-dir",
            "script-version",
        ],
    )?;
    // The merge path refuses a caller script that predates this gate's contract.
    if strict && f.get("script-version").and_then(|v| v.parse::<u32>().ok()) < Some(2) {
        return Err("merge-gate needs --script-version 2 or newer: the guarded-merge script is stale; run `bash <autospec-repo>/install.sh --update`".to_string());
    }
    let pr = f.get("pr").cloned().unwrap_or_default();
    if !pr.is_empty() && !(pr.len() <= 9 && pr.bytes().all(|b| b.is_ascii_digit())) {
        return Err("--pr must be a PR number".to_string());
    }
    if let Some(repo) = f.get("repo") {
        gate::parse_repo(repo)?;
    }
    if strict && (pr.is_empty() || !f.contains_key("repo")) {
        return Err("merge-gate requires --pr and --repo".to_string());
    }
    if let Some(head) = f.get("head") {
        if autospec_core::autonomous::review_quorum::canonical_commit(head).is_none() {
            return Err("--head must be a full commit sha".to_string());
        }
    }
    let (ok, body) = gate::verify(&gate::VerifyArgs {
        root: PathBuf::from(f.get("repo-root").map_or(".", String::as_str)),
        report: f.get("report").map(PathBuf::from),
        pr,
        repo: f.get("repo").cloned(),
        expect_head: f.get("head").cloned(),
        force_required: f.contains_key("required"),
        no_live: f.contains_key("no-live"),
        operator_config: f.get("operator-config").map(PathBuf::from),
        scratch: f.get("scratch-dir").map(PathBuf::from),
        strict,
        gh: gate::Gh::default(),
    });
    let mut body = body;
    if strict && !ok {
        body["labels_cleared"] = clear_labels(&f).into();
    }
    println!("{body}");
    // 3 = the quorum refused (1 stays the blast-radius fence's code in the guarded merge)
    Ok(if ok { 0 } else if strict { 3 } else { 1 })
}

/// A refused merge must not leave a stale "verified" mark on the PR: remove the labels that
/// `autospec-promote-pr.sh --confirm` added. Best effort; `true` only if every removal worked.
fn clear_labels(f: &std::collections::BTreeMap<String, String>) -> bool {
    ["autospec:verified", "autospec:needs-human-review"]
        .iter()
        .map(|label| {
            gate::gh_cmd(&gate::Gh::default())
                .current_dir("/")
                .args(["issue", "edit", f["pr"].as_str(), "--repo"])
                .arg(&f["repo"])
                .args(["--remove-label", label])
                .output()
                .is_ok_and(|o| o.status.success())
        })
        .fold(true, |all, ok| all && ok)
}

fn flags(
    args: &[String],
    allowed: &[&str],
) -> Result<std::collections::BTreeMap<String, String>, String> {
    let mut out = std::collections::BTreeMap::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let name = arg.trim_start_matches("--");
        if !arg.starts_with("--") || !allowed.contains(&name) {
            return Err(format!("unknown review-quorum argument: {arg}"));
        }
        let value = if matches!(name, "dry-run" | "retract" | "required" | "no-live") {
            "true".to_string()
        } else {
            iter.next()
                .ok_or(format!("{arg} requires a value"))?
                .clone()
        };
        out.insert(name.to_string(), value);
    }
    Ok(out)
}

fn need<'a>(
    map: &'a std::collections::BTreeMap<String, String>,
    key: &str,
) -> Result<&'a String, String> {
    map.get(key)
        .ok_or(format!("review-quorum requires --{key}"))
}

/// The scratch dir for key-bearing temp files: `--scratch-dir`, else the operator's state dir
/// (never inside a repository).
fn scratch(explicit: Option<&String>) -> Result<PathBuf, String> {
    match explicit {
        Some(dir) => herdr::private_dir(&PathBuf::from(dir)),
        None => herdr::private_dir(&gate::default_state_root()?.join("herdr-scratch")),
    }
}

/// Fully validate the two receipt files before anything is signed or sent: both are JSON
/// objects, both name the SAME full-length commit, the verdict is exactly lgtm or blocked, and
/// a request that names its PR names THIS one. Returns (commit, verdict).
fn validate_receipt(
    request: &serde_json::Value,
    verdict: &serde_json::Value,
    pr_node: &str,
) -> Result<(String, String), String> {
    use autospec_core::autonomous::review_quorum::{canonical_commit, canonical_target};
    if !request.is_object() || !verdict.is_object() {
        return Err("request and verdict files must each be a JSON object".to_string());
    }
    let commit_of = |value: &serde_json::Value, what: &str| {
        value["commit"]
            .as_str()
            .and_then(canonical_commit)
            .ok_or_else(|| format!("{what} commit must be a full 40 or 64 hex character head"))
    };
    let commit = commit_of(verdict, "verdict")?;
    if commit_of(request, "request")? != commit {
        return Err("verdict commit does not match the review request".to_string());
    }
    let word = verdict["verdict"]
        .as_str()
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| v == "lgtm" || v == "blocked")
        .ok_or("verdict.verdict must be lgtm or blocked")?;
    let wanted = canonical_target(pr_node);
    for key in ["pr", "pr_node", "target"] {
        if let Some(named) = request.get(key) {
            if named.as_str().and_then(canonical_target) != wanted {
                return Err(format!("review request {key} does not match --pr-node"));
            }
        }
    }
    Ok((commit, word))
}

/// `publish`: one validated review verdict -> `reviewed` edge in Herdr. Off by default.
fn publish(args: &[String]) -> Result<i32, String> {
    let f = flags(
        args,
        &[
            "config",
            "request",
            "verdict",
            "pr-node",
            "scratch-dir",
            "dry-run",
            "retract",
        ],
    )?;
    let cfg = herdr::load_config(std::path::Path::new(need(&f, "config")?))?;
    if !cfg.enabled {
        println!("{{\"outcome\":\"skipped\",\"reason\":\"herdr.enabled is false\"}}");
        return Ok(0);
    }
    let read = |name: &str| -> Result<(Vec<u8>, serde_json::Value), String> {
        let raw = fs::read(need(&f, name)?).map_err(|e| format!("read {name}: {e}"))?;
        let value = serde_json::from_slice(&raw).map_err(|e| format!("parse {name}: {e}"))?;
        Ok((raw, value))
    };
    let (_, request) = read("request")?;
    let (verdict_raw, verdict) = read("verdict")?;
    let (commit, verdict_word) = validate_receipt(&request, &verdict, need(&f, "pr-node")?)?;
    let reviewer = herdr::principal_id(&cfg.ingest_key);
    let retract = f.contains_key("retract");
    let build = if retract {
        herdr::build_retraction
    } else {
        herdr::build_publication
    };
    let pubn = build(
        need(&f, "pr-node")?,
        &commit,
        &verdict_word,
        &verdict_raw,
        &reviewer,
    )?;
    if f.contains_key("dry-run") {
        println!(
            "{}",
            serde_json::json!({"outcome": "dry_run", "reviewer": reviewer, "bodies": pubn.bodies})
        );
        return Ok(0);
    }
    let dir = scratch(f.get("scratch-dir"))?;
    let acks = herdr::publish(&cfg, &dir, &pubn)?;
    println!(
        "{}",
        serde_json::json!({"outcome": if retract { "retracted" } else { "published" },
                           "review_node": pubn.review_node, "acks": acks})
    );
    Ok(0)
}

/// `request`: re-review once per (PR, head) when Herdr says the current head needs it.
fn request(args: &[String]) -> Result<i32, String> {
    let f = flags(
        args,
        &["config", "pr-node", "state-dir", "scratch-dir", "dry-run"],
    )?;
    let cfg = herdr::load_config(std::path::Path::new(need(&f, "config")?))?;
    if !cfg.enabled {
        println!("{{\"outcome\":\"skipped\",\"reason\":\"herdr.enabled is false\"}}");
        return Ok(0);
    }
    let dir = scratch(f.get("scratch-dir"))?;
    let state = match f.get("state-dir") {
        Some(dir) => PathBuf::from(dir),
        None => gate::default_state_root()?.join("review-requests"),
    };
    let (outcome, quorum) = herdr::request_rereview(
        &cfg,
        &dir,
        &state,
        need(&f, "pr-node")?,
        f.contains_key("dry-run"),
    )?;
    let alert = outcome.starts_with("alert:");
    println!(
        "{}",
        serde_json::json!({"outcome": outcome, "alert": alert, "status": quorum["status"],
                           "head": quorum["head"], "head_unverified": quorum["head_unverified"]})
    );
    if alert {
        eprintln!(
            "review-quorum request: ALERT {outcome} for {}: Herdr cannot vouch for the head or author; an operator must re-sync it",
            need(&f, "pr-node")?
        );
        return Ok(4);
    }
    Ok(0)
}
