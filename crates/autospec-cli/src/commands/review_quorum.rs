//! `autospec review-quorum --input FILE [--json]` evaluates a head-bound post-PR
//! review quorum (T06). The input is `{"target", "head", "policy", "votes"}`; the votes are
//! built from validated independent-review receipts (`review_commit` ->
//! `commit`, `verdict_digest` -> `receipt_digest`). Exit 0 satisfied, 1
//! insufficient (fresh review needed), 3 blocked, 2 invalid input.
//!
//! `verify-report` is the merge gate's check of the report `--write-report` produced: it
//! binds the report to the LIVE head and target of the PR. `publish` / `request` are the
//! Herdr producer and consumer (see `herdr.rs`); `request` exits 4 when Herdr cannot vouch
//! for the head or author (an operator must look).

use std::fs;
use std::io::Read;
use std::path::PathBuf;

mod herdr;

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
        Some("verify-report") => return verify_report(&args[1..]),
        _ => {}
    }
    let mut input = None;
    let mut json = false;
    let mut report = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--input" => input = Some(iter.next().ok_or("--input requires a file")?.clone()),
            "--json" => json = true,
            "--write-report" => {
                report = Some(iter.next().ok_or("--write-report requires a path")?.clone())
            }
            other => return Err(format!("unknown review-quorum argument: {other}")),
        }
    }
    let path = input.ok_or("review-quorum requires --input FILE")?;
    let (target, outcome) = match evaluate_file(&path) {
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
        write_promotion_report(&report, &target, &outcome)?;
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

fn evaluate_file(path: &str) -> Result<(String, QuorumOutcome), String> {
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
    Ok((parsed.target, outcome))
}

/// Write the report `autospec-promote-pr.sh` already consumes (`review-quorum.json`), so the
/// merge path gates on THIS evaluator without any change to the shell: a non-pass verdict or
/// any blocking finding blocks promotion. Run it after the feature-scoped Python quorum, which
/// writes the same file.
fn write_promotion_report(path: &str, target: &str, outcome: &QuorumOutcome) -> Result<(), String> {
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
    let body = serde_json::json!({"schema": 1, "source": REPORT_SOURCE, "target_id": target,
        "head": outcome.head, "verdict": verdict, "promotion_ready": ready, "blocking_findings": blocking,
        "counted": outcome.counted, "rereview_required": outcome.rereview_required});
    write_atomic(path, &body)
}

const REPORT_SOURCE: &str = "head-bound-review-quorum";

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

/// Does `.autospec/autospec.yml` require the head-bound quorum? (`review_quorum:` block with
/// `required: true`.) A missing or unreadable file means "not required".
fn required_by_config(root: &std::path::Path) -> bool {
    let Ok(text) = fs::read_to_string(root.join(".autospec/autospec.yml")) else {
        return false;
    };
    let mut in_block = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !line.starts_with([' ', '\t']) {
            in_block = trimmed.split('#').next().unwrap_or("").trim() == "review_quorum:";
        } else if in_block {
            let value = trimmed
                .strip_prefix("required:")
                .map(|v| v.split('#').next().unwrap_or("").trim());
            if value == Some("true") {
                return true;
            }
        }
    }
    false
}

/// The live head and target of a PR, from `gh` (a fake `gh` on PATH in tests).
fn live_pr(
    root: &std::path::Path,
    pr: &str,
    repo: Option<&str>,
) -> Result<(String, String), String> {
    let mut cmd = std::process::Command::new("gh");
    cmd.current_dir(root)
        .args(["pr", "view", pr, "--json", "headRefOid,url"]);
    if let Some(repo) = repo {
        cmd.args(["--repo", repo]);
    }
    let out = cmd.output().map_err(|e| format!("cannot run gh: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "gh pr view failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("gh answered invalid JSON: {e}"))?;
    let head = value["headRefOid"]
        .as_str()
        .and_then(autospec_core::autonomous::review_quorum::canonical_commit)
        .ok_or("gh did not return a full head sha")?;
    let url = url::Url::parse(value["url"].as_str().unwrap_or(""))
        .map_err(|e| format!("gh returned no PR url: {e}"))?;
    let segments: Vec<&str> = url
        .path_segments()
        .map(Iterator::collect)
        .unwrap_or_default();
    let (Some(host), [owner, name, "pull", number, ..]) = (url.host_str(), segments.as_slice())
    else {
        return Err("gh returned an unexpected PR url".to_string());
    };
    Ok((
        head,
        format!("pr:{host}/{owner}/{name}#{number}").to_ascii_lowercase(),
    ))
}

/// `verify-report`: may the merge path trust `review-quorum.json`? Exit 0 yes, 1 no (reasons
/// printed as JSON). Fail closed: when the quorum is required (config or `--required`) a
/// missing, empty, malformed, failing, or other-head report blocks, and so does any report
/// this tool wrote (`source: head-bound-review-quorum`) that is not a current pass for THIS PR.
/// Older reports from the feature-scoped Python quorum keep their old meaning when the
/// quorum is not required.
fn verify_report(args: &[String]) -> Result<i32, String> {
    let f = flags(args, &["repo-root", "report", "pr", "repo", "required"])?;
    let root = PathBuf::from(f.get("repo-root").map_or(".", String::as_str));
    let report_path = f.get("report").map_or_else(
        || root.join(".autospec/reports/review-quorum.json"),
        PathBuf::from,
    );
    let required = f.contains_key("required") || required_by_config(&root);
    let pr = f.get("pr").map_or("", String::as_str);
    let repo = f.get("repo").map(String::as_str);
    if !pr.is_empty() && !(pr.len() <= 9 && pr.bytes().all(|b| b.is_ascii_digit())) {
        return Err("--pr must be a PR number".to_string());
    }
    if let Some(repo) = repo {
        let ok = repo.split_once('/').is_some_and(|(o, n)| {
            [o, n].iter().all(|p| {
                !p.is_empty()
                    && p.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            })
        });
        if !ok {
            return Err("--repo must be OWNER/REPO".to_string());
        }
    }
    let mut reasons: Vec<String> = Vec::new();
    let report: Option<serde_json::Value> = match fs::read_to_string(&report_path) {
        Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(v) if v.as_object().is_some_and(|o| !o.is_empty()) => Some(v),
            Ok(_) => {
                reasons.push("review-quorum.json is empty or not an object".to_string());
                None
            }
            Err(e) => {
                reasons.push(format!("review-quorum.json is malformed: {e}"));
                None
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if required {
                reasons.push(
                    "review quorum is required but review-quorum.json is missing".to_string(),
                );
            }
            None
        }
        Err(e) => {
            reasons.push(format!("review-quorum.json is unreadable: {e}"));
            None
        }
    };
    if let Some(report) = &report {
        let verdict = report["verdict"].as_str().unwrap_or("");
        let ours = report["source"].as_str() == Some(REPORT_SOURCE);
        if !matches!(verdict, "pass" | "pass_with_warnings") {
            reasons.push(format!(
                "review quorum verdict is {}",
                if verdict.is_empty() {
                    "missing"
                } else {
                    verdict
                }
            ));
        }
        if report["blocking_findings"]
            .as_array()
            .is_some_and(|b| !b.is_empty())
        {
            reasons.push("blocking specialist finding".to_string());
        }
        if required || ours {
            if !ours {
                reasons.push(
                    "required review quorum report is not from the head-bound evaluator"
                        .to_string(),
                );
            }
            if report["promotion_ready"].as_bool() != Some(true) || verdict != "pass" {
                reasons.push("report is not a head-bound pass".to_string());
            }
            if pr.is_empty() {
                reasons.push("no --pr: cannot bind the report to a PR head".to_string());
            } else {
                match live_pr(&root, pr, repo) {
                    Err(e) => reasons.push(format!("cannot verify the live PR head: {e}")),
                    Ok((live_head, live_target)) => {
                        let head = report["head"]
                            .as_str()
                            .and_then(autospec_core::autonomous::review_quorum::canonical_commit);
                        if head.as_deref() != Some(live_head.as_str()) {
                            reasons.push(format!(
                                "report head {} is not the live PR head {live_head}",
                                head.as_deref().unwrap_or("<none>")
                            ));
                        }
                        let target = report["target_id"]
                            .as_str()
                            .and_then(autospec_core::autonomous::review_quorum::canonical_target);
                        if target.as_deref() != Some(live_target.as_str()) {
                            reasons.push(format!(
                                "report target {} is not {live_target}",
                                target.as_deref().unwrap_or("<none>")
                            ));
                        }
                    }
                }
            }
        }
    }
    println!(
        "{}",
        serde_json::json!({"ok": reasons.is_empty(), "required": required, "reasons": reasons})
    );
    Ok(i32::from(!reasons.is_empty()))
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
        let value = if matches!(name, "dry-run" | "retract" | "required") {
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

fn scratch(base: &str) -> Result<PathBuf, String> {
    let dir = PathBuf::from(base);
    fs::create_dir_all(&dir).map_err(|e| format!("create scratch dir: {e}"))?;
    Ok(dir)
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
    let dir = scratch(
        f.get("scratch-dir")
            .map(String::as_str)
            .unwrap_or(".autospec/state/herdr-scratch"),
    )?;
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
    let dir = scratch(
        f.get("scratch-dir")
            .map(String::as_str)
            .unwrap_or(".autospec/state/herdr-scratch"),
    )?;
    let state = PathBuf::from(
        f.get("state-dir")
            .map(String::as_str)
            .unwrap_or(".autospec/state/review-requests"),
    );
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
