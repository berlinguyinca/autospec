//! `autospec review-quorum --input FILE [--json]` evaluates a head-bound post-PR
//! review quorum (T06). The input is `{"target", "head", "policy", "votes"}`; the votes are
//! built from validated independent-review receipts (`review_commit` ->
//! `commit`, `verdict_digest` -> `receipt_digest`). Exit 0 satisfied, 1
//! insufficient (fresh review needed), 3 blocked, 2 invalid input.

use std::fs;
use std::io::Read;

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
    let mut input = None;
    let mut json = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--input" => input = Some(iter.next().ok_or("--input requires a file")?.clone()),
            "--json" => json = true,
            other => return Err(format!("unknown review-quorum argument: {other}")),
        }
    }
    let path = input.ok_or("review-quorum requires --input FILE")?;
    let outcome = evaluate_file(&path)?;
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

fn evaluate_file(path: &str) -> Result<QuorumOutcome, String> {
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
    evaluate_quorum(&parsed.target, &parsed.head, &parsed.votes, &parsed.policy)
}
