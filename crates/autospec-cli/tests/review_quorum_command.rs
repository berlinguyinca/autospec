//! `autospec review-quorum`: evaluate exit codes, plus the Herdr producer (`publish`) and
//! the idempotent re-review consumer (`request`) against a fake curl (T06).

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "support/temp_directory.rs"]
mod temp_directory;
use temp_directory::unique as temp_dir;

use autospec_core::autonomous::review_quorum::{receipt_binding, sha256_hex};

fn autospec(args: &[&str]) -> (Option<i32>, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args(args)
        .output()
        .unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn evaluate(input: &str) -> (Option<i32>, String, String) {
    let dir = temp_dir("review-quorum");
    let file = dir.join("input.json");
    std::fs::write(&file, input).unwrap();
    autospec(&["review-quorum", "--input", file.to_str().unwrap(), "--json"])
}

fn vote(reviewer: &str, verdict: &str, commit: &str) -> String {
    let digest = "d".repeat(64);
    let binding = receipt_binding("o/r#1", commit, reviewer, &digest);
    format!(
        r#"{{"reviewer":"{reviewer}","target":"o/r#1","verdict":"{verdict}","commit":"{commit}","receipt_digest":"{digest}","receipt_binding":"{binding}"}}"#
    )
}

fn input(head: &str, vote_commit: &str, verdict: &str) -> String {
    format!(
        r#"{{"target":"o/r#1","head":"{head}","policy":{{"required":1,"pr_author":"author"}},"votes":[{}]}}"#,
        vote("r1", verdict, vote_commit)
    )
}

#[test]
fn satisfied_exits_zero_stale_exits_one_blocked_exits_three() {
    let (a, b) = ("a".repeat(40), "b".repeat(40));
    let (code, out, _) = evaluate(&input(&a, &a, "lgtm"));
    assert_eq!(code, Some(0));
    assert!(out.contains("\"status\":\"satisfied\""));
    let (code, out, _) = evaluate(&input(&b, &a, "lgtm"));
    assert_eq!(code, Some(1));
    assert!(out.contains("\"rereview_required\":true"));
    assert_eq!(evaluate(&input(&a, &a, "blocked")).0, Some(3));
    assert_eq!(evaluate(&input(&a, &a, "BLOCKED")).0, Some(3));
}

#[test]
fn malformed_input_is_a_diagnostic_not_a_pass() {
    let (code, _, err) = evaluate(
        r#"{"target":"o/r#1","head":"abc","policy":{"required":1,"pr_author":"a"},"votes":[]}"#,
    );
    assert_eq!(code, Some(2));
    assert!(err.contains("head"));
    assert_eq!(evaluate(r#"{"target":"t","head":"x","policy":{"required":1,"pr_author":"a"},"votes":[],"extra":1}"#).0, Some(2));
    let head = "a".repeat(40);
    let body =
        format!(r#"{{"target":"o/r#1","head":"{head}","policy":{{"required":1}},"votes":[]}}"#);
    assert_eq!(evaluate(&body).0, Some(2));
}

// ---------------------------------------------------------------- Herdr producer / consumer
fn config(dir: &Path, enabled: bool, curl: &Path, dispatch: Option<&Path>) -> PathBuf {
    let path = dir.join("herdr.yml");
    let mut body = format!(
        "herdr:\n  enabled: {enabled}\n  url: http://127.0.0.1:9\n  ingest_key: {}\n  curl: {}\n",
        sha256_hex(b"fixture-key-material"),
        curl.display()
    );
    if let Some(d) = dispatch {
        body.push_str(&format!("  dispatch_command: {}\n", d.display()));
    }
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    path
}

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn verdict_files(dir: &Path, commit: &str, verdict: &str) -> (PathBuf, PathBuf) {
    let request = dir.join("request.json");
    let verdict_path = dir.join("verdict.json");
    std::fs::write(&request, format!(r#"{{"schema":1,"commit":"{commit}"}}"#)).unwrap();
    std::fs::write(
        &verdict_path,
        format!(r#"{{"schema":1,"commit":"{commit}","verdict":"{verdict}"}}"#),
    )
    .unwrap();
    (request, verdict_path)
}

const PR: &str = "pr:github.com/o/r#7";

#[test]
fn publish_is_off_by_default_and_builds_bound_idempotent_envelopes() {
    let dir = temp_dir("rq-publish");
    let curl = script(&dir, "curl", "exit 9");
    let head = "c".repeat(40);
    let (request, verdict) = verdict_files(&dir, &head, "lgtm");
    let off = config(&dir, false, &curl, None);
    let args = |cfg: &Path, extra: &'static [&'static str]| {
        let mut v: Vec<String> = [
            "review-quorum",
            "publish",
            "--config",
            cfg.to_str().unwrap(),
            "--request",
            request.to_str().unwrap(),
            "--verdict",
            verdict.to_str().unwrap(),
            "--pr-node",
            PR,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        v.extend(extra.iter().map(|s| s.to_string()));
        v
    };
    let run = |a: Vec<String>| autospec(&a.iter().map(String::as_str).collect::<Vec<_>>());
    let (code, out, _) = run(args(&off, &[]));
    assert_eq!(
        (code, out.contains("skipped")),
        (Some(0), true),
        "disabled config never touches the network"
    );

    let on = config(&dir, true, &curl, None);
    let (code, first, _) = run(args(&on, &["--dry-run"]));
    assert_eq!(code, Some(0));
    let (_, second, _) = run(args(&on, &["--dry-run"]));
    assert_eq!(first, second, "same verdict -> same event ids");
    let body: serde_json::Value = serde_json::from_str(&first).unwrap();
    let edge = &body["bodies"][1]["payload"]["attrs"];
    let reviewer = body["reviewer"].as_str().unwrap();
    assert!(
        reviewer.starts_with("key:") && !first.contains(&sha256_hex(b"fixture-key-material")),
        "the key never leaves"
    );
    assert_eq!(edge["commit"], head.as_str());
    assert_eq!(edge["verdict"], "lgtm");
    let digest = edge["receipt_digest"].as_str().unwrap();
    assert_eq!(
        edge["receipt_binding"],
        receipt_binding(PR, &head, reviewer, digest).as_str()
    );

    // A verdict for a different commit than the request is refused.
    std::fs::write(
        &verdict,
        format!(r#"{{"commit":"{}","verdict":"lgtm"}}"#, "d".repeat(40)),
    )
    .unwrap();
    assert_eq!(run(args(&on, &["--dry-run"])).0, Some(2));
}

#[test]
fn publish_refuses_a_world_readable_config_and_a_non_https_url() {
    let dir = temp_dir("rq-perm");
    let curl = script(&dir, "curl", "exit 0");
    let cfg = config(&dir, true, &curl, None);
    std::fs::set_permissions(&cfg, std::fs::Permissions::from_mode(0o644)).unwrap();
    let head = "c".repeat(40);
    let (request, verdict) = verdict_files(&dir, &head, "lgtm");
    let a = [
        "review-quorum",
        "publish",
        "--config",
        cfg.to_str().unwrap(),
        "--request",
        request.to_str().unwrap(),
        "--verdict",
        verdict.to_str().unwrap(),
        "--pr-node",
        PR,
    ];
    let (code, _, err) = autospec(&a);
    assert_eq!(code, Some(2));
    assert!(err.contains("chmod 600"));
    std::fs::set_permissions(&cfg, std::fs::Permissions::from_mode(0o600)).unwrap();
    let text = std::fs::read_to_string(&cfg)
        .unwrap()
        .replace("http://127.0.0.1:9", "http://example.test");
    std::fs::write(&cfg, text).unwrap();
    assert_eq!(autospec(&a).0, Some(2));
}

#[test]
fn request_dispatches_once_per_pr_and_head_and_again_after_the_head_moves() {
    let dir = temp_dir("rq-request");
    let calls = dir.join("calls.log");
    let dispatch = script(
        &dir,
        "dispatch",
        &format!("echo \"$@\" >> {}", calls.display()),
    );
    let reply = dir.join("reply.json");
    let curl = script(
        &dir,
        "curl",
        &format!("cat {}; printf '\\n200'", reply.display()),
    );
    let cfg = config(&dir, true, &curl, Some(&dispatch));
    let state = dir.join("state");
    let run = || {
        autospec(&[
            "review-quorum",
            "request",
            "--config",
            cfg.to_str().unwrap(),
            "--pr-node",
            PR,
            "--state-dir",
            state.to_str().unwrap(),
            "--scratch-dir",
            dir.join("scratch").to_str().unwrap(),
        ])
    };
    let (a, b) = ("a".repeat(40), "b".repeat(40));
    let set = |head: &str, required: bool| {
        std::fs::write(
            &reply,
            format!(
                r#"{{"status":"insufficient","head":"{head}","rereview_required":{required}}}"#
            ),
        )
        .unwrap();
    };
    set(&a, true);
    assert!(run().1.contains("\"requested\""));
    assert!(run().1.contains("already_requested"));
    assert_eq!(std::fs::read_to_string(&calls).unwrap().lines().count(), 1);
    set(&b, true);
    assert!(run().1.contains("\"requested\""));
    assert_eq!(std::fs::read_to_string(&calls).unwrap().lines().count(), 2);
    set(&b, false);
    assert!(run().1.contains("not_needed"));
    assert_eq!(std::fs::read_to_string(&calls).unwrap().lines().count(), 2);
    // arguments reach the command as argv, in order: pr node then head
    assert!(std::fs::read_to_string(&calls)
        .unwrap()
        .contains(&format!("{PR} {a}")));
}

#[test]
fn a_failed_dispatch_is_retried_not_remembered_as_done() {
    let dir = temp_dir("rq-retry");
    let dispatch = script(&dir, "dispatch", "exit 1");
    let reply = dir.join("reply.json");
    let head = "a".repeat(40);
    std::fs::write(
        &reply,
        format!(r#"{{"status":"insufficient","head":"{head}","rereview_required":true}}"#),
    )
    .unwrap();
    let curl = script(
        &dir,
        "curl",
        &format!("cat {}; printf '\\n200'", reply.display()),
    );
    let cfg = config(&dir, true, &curl, Some(&dispatch));
    let state = dir.join("state");
    let run = || {
        autospec(&[
            "review-quorum",
            "request",
            "--config",
            cfg.to_str().unwrap(),
            "--pr-node",
            PR,
            "--state-dir",
            state.to_str().unwrap(),
            "--scratch-dir",
            dir.join("scratch").to_str().unwrap(),
        ])
    };
    assert_eq!(run().0, Some(2));
    assert_eq!(
        run().0,
        Some(2),
        "still pending: it dispatches again instead of reporting already_requested"
    );
}

#[test]
fn request_reports_herdr_errors_instead_of_assuming_nothing_is_needed() {
    let dir = temp_dir("rq-err");
    let curl = script(&dir, "curl", "printf '{}\\n500'");
    let cfg = config(&dir, true, &curl, None);
    let (code, _, err) = autospec(&[
        "review-quorum",
        "request",
        "--config",
        cfg.to_str().unwrap(),
        "--pr-node",
        PR,
        "--state-dir",
        dir.join("s").to_str().unwrap(),
        "--scratch-dir",
        dir.join("x").to_str().unwrap(),
    ]);
    assert_eq!(code, Some(2));
    assert!(err.contains("HTTP 500"));
}

#[test]
fn the_report_is_what_promote_pr_already_gates_on() {
    let dir = temp_dir("rq-report");
    let reports = dir.join(".autospec/reports");
    std::fs::create_dir_all(&reports).unwrap();
    std::fs::write(
        reports.join("verifier-report.json"),
        r#"{"verdict":"pass","dimensions":[]}"#,
    )
    .unwrap();
    let head = "a".repeat(40);
    let input_file = dir.join("in.json");
    let promote = |verdict: &str| {
        std::fs::write(&input_file, input(&head, &head, verdict)).unwrap();
        let report = reports.join("review-quorum.json");
        let (code, _, _) = autospec(&[
            "review-quorum",
            "--input",
            input_file.to_str().unwrap(),
            "--write-report",
            report.to_str().unwrap(),
        ]);
        let out = Command::new("bash")
            .args([
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../scripts/autospec-promote-pr.sh"
                ),
                "--dry-run",
                "--repo-root",
                dir.to_str().unwrap(),
                "--pr",
                "7",
            ])
            .output()
            .unwrap();
        (
            code,
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };
    assert_eq!(promote("lgtm").0, Some(0));
    let (_, promo, text) = promote("lgtm");
    assert_eq!(promo, Some(0), "{text}");
    let (code, promo, text) = promote("blocked");
    assert_eq!((code, promo), (Some(3), Some(1)), "{text}");
}
