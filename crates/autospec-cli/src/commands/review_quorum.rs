//! `autospec review-quorum --input FILE [--json]` evaluates a head-bound post-PR
//! review quorum (T06). The input is `{"target", "head", "policy", "votes"}`; the votes are
//! built from validated independent-review receipts (`review_commit` ->
//! `commit`, `verdict_digest` -> `receipt_digest`). Exit 0 satisfied, 1
//! insufficient (fresh review needed), 3 blocked, 2 invalid input.

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
    let (target, outcome) = evaluate_file(&path)?;
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
    let body = serde_json::json!({"schema": 1, "source": "head-bound-review-quorum", "target_id": target,
        "head": outcome.head, "verdict": verdict, "promotion_ready": ready, "blocking_findings": blocking,
        "counted": outcome.counted, "rereview_required": outcome.rereview_required});
    if let Some(parent) = std::path::Path::new(path).parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create report dir: {e}"))?;
    }
    fs::write(path, format!("{body}\n")).map_err(|e| format!("write report: {e}"))
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
        let value = if name == "dry-run" {
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
    let commit = verdict["commit"].as_str().ok_or("verdict has no commit")?;
    if request["commit"].as_str().map(str::to_ascii_lowercase) != Some(commit.to_ascii_lowercase())
    {
        return Err("verdict commit does not match the review request".to_string());
    }
    let reviewer = herdr::principal_id(&cfg.ingest_key);
    let pubn = herdr::build_publication(
        need(&f, "pr-node")?,
        commit,
        verdict["verdict"]
            .as_str()
            .ok_or("verdict has no verdict")?,
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
        serde_json::json!({"outcome": "published", "review_node": pubn.review_node, "acks": acks})
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
    println!(
        "{}",
        serde_json::json!({"outcome": outcome, "status": quorum["status"], "head": quorum["head"]})
    );
    Ok(0)
}
