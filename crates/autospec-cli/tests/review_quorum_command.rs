//! `autospec review-quorum`: evaluate exit codes, plus the Herdr producer (`publish`) and
//! the idempotent re-review consumer (`request`) against a fake curl (T06).

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "support/temp_directory.rs"]
mod temp_directory;
use temp_directory::unique as temp_dir;

use autospec_core::autonomous::review_quorum::{hmac_sha256_hex, receipt_binding, sha256_hex};

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
    let binding = receipt_binding("pr:github.com/o/r#7", commit, reviewer, &digest);
    format!(
        r#"{{"reviewer":"{reviewer}","target":"pr:github.com/o/r#7","verdict":"{verdict}","commit":"{commit}","receipt_digest":"{digest}","receipt_binding":"{binding}"}}"#
    )
}

fn input(head: &str, vote_commit: &str, verdict: &str) -> String {
    format!(
        r#"{{"target":"pr:github.com/o/r#7","head":"{head}","policy":{{"required":1,"pr_author":"author"}},"votes":[{}]}}"#,
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
        r#"{"target":"pr:github.com/o/r#7","head":"abc","policy":{"required":1,"pr_author":"a"},"votes":[]}"#,
    );
    assert_eq!(code, Some(2));
    assert!(err.contains("head"));
    assert_eq!(evaluate(r#"{"target":"t","head":"x","policy":{"required":1,"pr_author":"a"},"votes":[],"extra":1}"#).0, Some(2));
    let head = "a".repeat(40);
    let body = format!(
        r#"{{"target":"pr:github.com/o/r#7","head":"{head}","policy":{{"required":1}},"votes":[]}}"#
    );
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

/// Write a marker the way `request` does (including its MAC under the state dir's nonce), so a
/// test can stage a specific claim state.
fn forge_marker(
    state: &Path,
    marker: &Path,
    head: &str,
    marker_state: &str,
    pid: u32,
    started: u64,
    failures: u32,
    next_try_at: u64,
) {
    std::fs::create_dir_all(state).unwrap();
    std::fs::set_permissions(state, std::fs::Permissions::from_mode(0o700)).unwrap();
    let nonce = state.join(".nonce");
    if !nonce.exists() {
        std::fs::write(&nonce, "n".repeat(64)).unwrap();
        std::fs::set_permissions(&nonce, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let mut body = serde_json::json!({"pr": PR, "head": head, "state": marker_state, "pid": pid,
        "started": started, "token": "t", "failures": failures, "next_try_at": next_try_at, "detail": ""});
    let message = [
        "pr",
        "head",
        "state",
        "pid",
        "started",
        "token",
        "failures",
        "next_try_at",
        "detail",
    ]
    .iter()
    .map(|n| body[*n].to_string())
    .collect::<Vec<_>>()
    .join("\u{1f}");
    body["mac"] = hmac_sha256_hex(&std::fs::read(&nonce).unwrap(), message.as_bytes()).into();
    std::fs::write(marker, format!("{body}\n")).unwrap();
    std::fs::set_permissions(marker, std::fs::Permissions::from_mode(0o600)).unwrap();
}

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
    let marker_path = std::fs::read_dir(&state)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "json"))
        .unwrap();
    let marker: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&marker_path).unwrap()).unwrap();
    assert_eq!(marker["state"], "failed");
    assert_eq!(marker["failures"], 1);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert!(
        marker["next_try_at"].as_u64().unwrap() >= now + 25,
        "backoff is stored: {marker}"
    );
    // inside the backoff window nothing is dispatched again (and it is not "done" either)
    let (code, out, _) = run();
    assert_eq!(code, Some(0));
    assert!(out.contains("request_backoff"), "{out}");
    // once the window has passed it is retried, with the failure count carried and the delay doubled
    forge_marker(&state, &marker_path, &head, "failed", 0, now, 1, now - 1);
    assert_eq!(run().0, Some(2));
    let marker: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&marker_path).unwrap()).unwrap();
    assert_eq!(marker["failures"], 2);
    assert!(
        marker["next_try_at"].as_u64().unwrap() >= now + 55,
        "{marker}"
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

// ---------------------------------------------------------------- merge gate
/// A repo root with a passing verifier report, a `bin/` holding the real `autospec` and a fake
/// `gh` whose answer is whatever `live.json` says (the PR's live head and URL).
struct Gate {
    dir: PathBuf,
    bin: PathBuf,
}

impl Gate {
    fn new(name: &str) -> Self {
        let dir = temp_dir(name);
        let reports = dir.join(".autospec/reports");
        std::fs::create_dir_all(&reports).unwrap();
        std::fs::write(
            reports.join("verifier-report.json"),
            r#"{"verdict":"pass","dimensions":[]}"#,
        )
        .unwrap();
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_autospec"), bin.join("autospec")).unwrap();
        let gate = Self { dir, bin };
        gate.live(&"a".repeat(40), 7);
        // The fake forge: `pr view` answers `live.json` (and then moves the head to `moved.json`
        // when it exists, i.e. a push right AFTER the gate looked), `api .../contents/...`
        // serves `base.yml` as the default branch's config (404 when absent), `pr merge`
        // behaves like GitHub's `--match-head-commit`, `issue edit` is recorded.
        let d = gate.dir.display();
        script(
            &gate.bin,
            "gh",
            &format!(
                r#"case "$*" in
*"api "*"contents/.autospec/autospec.yml"*)
  if [ -f {d}/api_fail ]; then echo "gh: Bad Gateway (HTTP 502)" >&2; exit 1; fi
  if [ -f {d}/api_noise ]; then echo "gh: HTTP 500 (request id 4041-404)" >&2; exit 1; fi
  if [ -f {d}/base.yml ]; then cat {d}/base.yml; else echo "gh: Not Found (HTTP 404)" >&2; exit 1; fi;;
*"api "*) if [ -f {d}/repo_fail ]; then echo "gh: Not Found (HTTP 404)" >&2; exit 1; fi; echo '{{}}';;
*"pr view"*"--json files"*) echo README.md;;
*"pr view"*"--json headRefOid --jq"*) sed -n 's/.*"headRefOid":"\([0-9a-f]*\)".*/\1/p' {d}/live.json;;
*"pr view"*"headRefOid,url"*) if [ -f {d}/pr_fail ]; then echo "gh: HTTP 502" >&2; exit 1; fi; cat {d}/live.json; if [ -f {d}/moved.json ]; then cp {d}/moved.json {d}/live.json; fi;;
*"pr view"*) cat {d}/live.json;;
*"pr merge"*)
  echo "$*" >> {d}/merge.log
  want=$(sed -n 's/.*"headRefOid":"\([0-9a-f]*\)".*/\1/p' {d}/live.json)
  case "$*" in *"--match-head-commit $want"*) echo merged >> {d}/merged;; *"--match-head-commit"*) echo "head moved" >&2; exit 1;; *) echo merged >> {d}/merged;; esac;;
*"issue edit"*) echo "$*" >> {d}/labels.log;;
*) echo "unexpected gh $*" >&2; exit 1;;
esac"#
            ),
        );
        gate
    }

    fn live(&self, head: &str, number: u32) {
        std::fs::write(
            self.dir.join("live.json"),
            format!(r#"{{"headRefOid":"{head}","url":"https://github.com/o/r/pull/{number}"}}"#),
        )
        .unwrap();
    }

    fn report(&self) -> PathBuf {
        self.dir.join(".autospec/reports/review-quorum.json")
    }

    fn require(&self) {
        std::fs::write(
            self.dir.join(".autospec/autospec.yml"),
            "version: 1\nreview_quorum:\n  required: true # head-bound\n",
        )
        .unwrap();
    }

    /// Run the evaluator with `--write-report`; returns its exit code.
    fn evaluate(&self, body: &str) -> Option<i32> {
        let file = self.dir.join("in.json");
        std::fs::write(&file, body).unwrap();
        autospec(&[
            "review-quorum",
            "--input",
            file.to_str().unwrap(),
            "--write-report",
            self.report().to_str().unwrap(),
            "--repo-root",
            self.dir.to_str().unwrap(),
            "--operator-config",
            self.dir.join("no-operator-config.yml").to_str().unwrap(),
        ])
        .0
    }

    /// Run the unchanged promotion script; returns (exit code, plan json text).
    fn promote(&self, pr: &str) -> (Option<i32>, String) {
        let path = format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let out = Command::new("bash")
            .env("PATH", path)
            .args([
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../scripts/autospec-promote-pr.sh"
                ),
                "--dry-run",
                "--repo-root",
                self.dir.to_str().unwrap(),
                "--repo",
                "o/r",
                "--pr",
                pr,
            ])
            .output()
            .unwrap();
        let plan = std::fs::read_to_string(self.dir.join(".autospec/reports/promotion-plan.json"))
            .unwrap_or_default();
        (out.status.code(), plan)
    }
}

fn gate_input(head: &str, verdict: &str, author: &str) -> String {
    format!(
        r#"{{"target":"{PR}","head":"{head}","policy":{{"required":1,"pr_author":"{author}"}},"votes":[{}]}}"#,
        vote("r1", verdict, head)
    )
}

#[test]
fn the_gate_promotes_only_a_pass_at_the_live_head() {
    let gate = Gate::new("rq-gate-live");
    gate.require();
    let a = "a".repeat(40);
    assert_eq!(gate.evaluate(&gate_input(&a, "lgtm", "author")), Some(0));
    assert_eq!(
        gate.promote("7").0,
        Some(0),
        "pass at the live head promotes"
    );

    // The PR moves to head B; the old pass at A must not carry over.
    gate.live(&"b".repeat(40), 7);
    let (code, plan) = gate.promote("7");
    assert_eq!(code, Some(1), "{plan}");
    assert!(plan.contains("is not the live PR head"), "{plan}");

    // A report about another PR cannot promote this one.
    gate.live(&a, 8);
    let (code, plan) = gate.promote("8");
    assert_eq!(code, Some(1), "{plan}");
    assert!(plan.contains("is not pr:github.com/o/r#8"), "{plan}");
}

#[test]
fn a_failed_evaluation_overwrites_an_older_pass_and_blocks() {
    let gate = Gate::new("rq-gate-error");
    gate.require();
    let a = "a".repeat(40);
    assert_eq!(gate.evaluate(&gate_input(&a, "lgtm", "author")), Some(0));
    assert_eq!(gate.promote("7").0, Some(0));
    // Same head, but now the evaluation errors (empty author): exit 2 AND a non-pass report.
    assert_eq!(gate.evaluate(&gate_input(&a, "lgtm", "")), Some(2));
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(gate.report()).unwrap()).unwrap();
    assert_eq!(report["verdict"], "invalid_input");
    assert_eq!(report["promotion_ready"], false);
    assert_eq!(
        report["head"],
        a.as_str(),
        "head is kept when it can be read"
    );
    let (code, plan) = gate.promote("7");
    assert_eq!(code, Some(1), "{plan}");
    // Unreadable input files overwrite too.
    assert_eq!(gate.evaluate("not json"), Some(2));
    assert_eq!(gate.promote("7").0, Some(1));
}

#[test]
fn a_required_quorum_blocks_when_the_report_is_missing_empty_or_malformed() {
    let gate = Gate::new("rq-gate-missing");
    // Not required, no report: the old behaviour (nothing to gate on) is kept.
    assert_eq!(gate.promote("7").0, Some(0));
    gate.require();
    let (code, plan) = gate.promote("7");
    assert_eq!(code, Some(1), "{plan}");
    assert!(plan.contains("review-quorum.json is missing"), "{plan}");
    for body in ["{}", "[]", "not json", "", "null"] {
        std::fs::write(gate.report(), body).unwrap();
        assert_eq!(gate.promote("7").0, Some(1), "report {body:?} must block");
    }
    // Even when the quorum is NOT required, an empty or malformed report that exists blocks.
    std::fs::remove_file(gate.dir.join(".autospec/autospec.yml")).unwrap();
    for body in ["{}", "not json"] {
        std::fs::write(gate.report(), body).unwrap();
        assert_eq!(gate.promote("7").0, Some(1), "report {body:?} must block");
    }
}

#[test]
fn a_required_quorum_fails_closed_when_the_live_head_cannot_be_read() {
    let gate = Gate::new("rq-gate-gh");
    gate.require();
    let a = "a".repeat(40);
    assert_eq!(gate.evaluate(&gate_input(&a, "lgtm", "author")), Some(0));
    script(&gate.bin, "gh", "echo boom >&2; exit 1");
    let (code, plan) = gate.promote("7");
    assert_eq!(code, Some(1), "{plan}");
    assert!(plan.contains("cannot verify the live PR head"), "{plan}");
    // A blocked report blocks too, and so does a head-bound report that is not a pass.
    script(
        &gate.bin,
        "gh",
        &format!("cat {}", gate.dir.join("live.json").display()),
    );
    assert_eq!(gate.evaluate(&gate_input(&a, "blocked", "author")), Some(3));
    assert_eq!(gate.promote("7").0, Some(1));
}

#[test]
fn a_legacy_python_report_keeps_its_meaning_unless_the_quorum_is_required() {
    let gate = Gate::new("rq-gate-legacy");
    std::fs::write(
        gate.report(),
        r#"{"schema":1,"verdict":"pass","blocking_findings":[]}"#,
    )
    .unwrap();
    assert_eq!(gate.promote("7").0, Some(0));
    std::fs::write(
        gate.report(),
        r#"{"schema":1,"verdict":"blocked","blocking_findings":[{"specialist":"x"}]}"#,
    )
    .unwrap();
    assert_eq!(gate.promote("7").0, Some(1));
    gate.require();
    std::fs::write(
        gate.report(),
        r#"{"schema":1,"verdict":"pass","blocking_findings":[]}"#,
    )
    .unwrap();
    let (code, plan) = gate.promote("7");
    assert_eq!(code, Some(1), "required means head-bound only: {plan}");
}

#[test]
fn verify_report_can_be_run_directly() {
    let gate = Gate::new("rq-verify");
    let a = "a".repeat(40);
    assert_eq!(gate.evaluate(&gate_input(&a, "lgtm", "author")), Some(0));
    let path = format!(
        "{}:{}",
        gate.bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let run = |extra: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_autospec"))
            .env("PATH", &path)
            .args([
                "review-quorum",
                "verify-report",
                "--repo-root",
                gate.dir.to_str().unwrap(),
                "--operator-config",
                gate.dir.join("no-operator-config.yml").to_str().unwrap(),
            ])
            .args(extra)
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };
    // without --repo the default branch cannot be asked: required, and the reason says why
    let (code, out) = run(&["--pr", "7"]);
    assert_eq!(code, Some(1), "{out}");
    assert!(out.contains("pass --repo OWNER/REPO"), "{out}");
    assert_eq!(run(&["--pr", "7", "--repo", "o/r"]).0, Some(0));
    assert_eq!(
        run(&[]).0,
        Some(1),
        "a head-bound report cannot be trusted without a PR"
    );
    assert_eq!(run(&["--pr", "7;rm"]).0, Some(2));
    assert_eq!(run(&["--pr", "7", "--repo", "o/r; x"]).0, Some(2));
}

#[test]
fn a_non_ascii_target_is_refused_not_case_folded() {
    let head = "a".repeat(40);
    let body = format!(
        r#"{{"target":"pr:github.com/o/r#7K","head":"{head}","policy":{{"required":1,"pr_author":"a"}},"votes":[]}}"#
    );
    let (code, _, err) = evaluate(&body);
    assert_eq!(code, Some(2));
    assert!(err.contains("ASCII"), "{err}");
}

// ---------------------------------------------------------------- request: lock, alert, safety
struct Request {
    dir: PathBuf,
    cfg: PathBuf,
    state: PathBuf,
    calls: PathBuf,
    reply: PathBuf,
}

impl Request {
    fn new(name: &str, dispatch_body: &str) -> Self {
        let dir = temp_dir(name);
        let calls = dir.join("calls.log");
        let dispatch = script(
            &dir,
            "dispatch",
            &format!("echo \"$@\" >> {}\n{dispatch_body}", calls.display()),
        );
        let reply = dir.join("reply.json");
        let curl = script(
            &dir,
            "curl",
            &format!("cat {}; printf '\\n200'", reply.display()),
        );
        let cfg = config(&dir, true, &curl, Some(&dispatch));
        let state = dir.join("state");
        let r = Self {
            dir,
            cfg,
            state,
            calls,
            reply,
        };
        r.set("insufficient", &"a".repeat(40), true);
        r
    }

    fn set(&self, status: &str, head: &str, required: bool) {
        std::fs::write(
            &self.reply,
            format!(r#"{{"status":"{status}","head":"{head}","rereview_required":{required}}}"#),
        )
        .unwrap();
    }

    fn args(&self) -> Vec<String> {
        [
            "review-quorum",
            "request",
            "--config",
            self.cfg.to_str().unwrap(),
            "--pr-node",
            PR,
            "--state-dir",
            self.state.to_str().unwrap(),
            "--scratch-dir",
            self.dir.join("scratch").to_str().unwrap(),
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    fn run(&self) -> (Option<i32>, String, String) {
        let a = self.args();
        autospec(&a.iter().map(String::as_str).collect::<Vec<_>>())
    }

    fn dispatches(&self) -> usize {
        std::fs::read_to_string(&self.calls).map_or(0, |t| t.lines().count())
    }

    fn marker(&self, head: &str) -> PathBuf {
        self.state
            .join(format!("{}-{head}.json", &sha256_hex(PR.as_bytes())[..16]))
    }
}

#[test]
fn concurrent_requests_dispatch_exactly_once() {
    let r = Request::new("rq-concurrent", "sleep 1");
    let handles: Vec<_> = (0..6)
        .map(|_| {
            let args = r.args();
            std::thread::spawn(move || {
                autospec(&args.iter().map(String::as_str).collect::<Vec<_>>())
            })
        })
        .collect();
    let outs: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(r.dispatches(), 1, "{outs:?}");
    assert!(outs.iter().all(|o| o.0 == Some(0)), "{outs:?}");
    let kinds: Vec<&str> = outs.iter().map(|o| o.1.as_str()).collect();
    assert_eq!(
        kinds
            .iter()
            .filter(|k| k.contains("\"outcome\":\"requested\""))
            .count(),
        1
    );
    assert!(
        kinds
            .iter()
            .all(|k| k.contains("requested") || k.contains("request_in_progress")),
        "{kinds:?}"
    );
    assert!(r.run().1.contains("already_requested"));
    assert_eq!(r.dispatches(), 1);
}

#[test]
fn a_pending_marker_with_a_live_owner_blocks_and_a_stale_one_expires() {
    let r = Request::new("rq-pending", "true");
    let head = "a".repeat(40);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let write = |pid: u32, started: u64| {
        forge_marker(
            &r.state,
            &r.marker(&head),
            &head,
            "pending",
            pid,
            started,
            0,
            0,
        );
    };
    // Live owner (this test process), fresh: blocks.
    write(std::process::id(), now);
    assert!(r.run().1.contains("request_in_progress"));
    assert_eq!(r.dispatches(), 0);
    // Live owner but past the timeout: expires and is re-dispatched.
    write(std::process::id(), now - 100_000);
    assert!(r.run().1.contains("\"requested\""));
    assert_eq!(r.dispatches(), 1);
    // Dead owner, fresh: also reclaimed (no waiting for the timeout).
    let dead = Command::new("true")
        .spawn()
        .and_then(|mut c| c.wait().map(|_| c.id()))
        .unwrap();
    write(dead, now);
    assert!(r.run().1.contains("\"requested\""));
    assert_eq!(r.dispatches(), 2);
}

#[test]
fn a_partial_marker_is_pending_not_dispatchable_until_it_expires() {
    let r = Request::new("rq-partial", "true");
    let head = "a".repeat(40);
    std::fs::create_dir_all(&r.state).unwrap();
    std::fs::set_permissions(&r.state, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(r.marker(&head), "{\"state\":\"pend").unwrap();
    assert!(
        r.run().1.contains("request_in_progress"),
        "a half-written marker must not re-dispatch"
    );
    assert_eq!(r.dispatches(), 0);
    // Once it is older than the timeout it is stale and the request goes out.
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(100_000);
    std::fs::File::options()
        .write(true)
        .open(r.marker(&head))
        .unwrap()
        .set_modified(old)
        .unwrap();
    assert!(r.run().1.contains("\"requested\""));
    assert_eq!(r.dispatches(), 1);
    let marker: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(r.marker(&head)).unwrap()).unwrap();
    assert_eq!(
        marker["state"], "dispatched",
        "markers are written whole (temp + rename)"
    );
}

#[test]
fn an_unverifiable_head_is_an_alert_not_a_silent_not_needed() {
    let r = Request::new("rq-alert", "true");
    for status in ["head_unverified", "unknown_head", "unknown_author"] {
        r.set(status, &"a".repeat(40), false);
        let (code, out, err) = r.run();
        assert_eq!(code, Some(4), "{status}");
        assert!(
            out.contains("\"alert\":true") && out.contains(&format!("alert:{status}")),
            "{out}"
        );
        assert!(err.contains("ALERT"), "{err}");
    }
    assert_eq!(r.dispatches(), 0);
    r.set("satisfied", &"a".repeat(40), false);
    assert_eq!(r.run().0, Some(0));
}

#[test]
fn dispatch_receives_the_pr_after_a_double_dash_and_a_bad_pr_node_is_refused() {
    let r = Request::new("rq-dashdash", "true");
    assert!(r.run().1.contains("\"requested\""));
    assert_eq!(
        std::fs::read_to_string(&r.calls).unwrap().trim(),
        format!("-- {PR} {}", "a".repeat(40))
    );
    for bad in [
        "--evil",
        "pr:-x#1",
        "pr:github.com/o/r#7;rm",
        "pr:github.com/o/r",
        "pr:\u{fb01}#1",
        "",
    ] {
        let mut a = r.args();
        let at = a.iter().position(|x| x == "--pr-node").unwrap() + 1;
        a[at] = bad.to_string();
        let (code, _, err) = autospec(&a.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(code, Some(2), "{bad:?}: {err}");
    }
    assert_eq!(r.dispatches(), 1);
}

// ---------------------------------------------------------------- key material
#[test]
fn the_key_file_lives_in_a_private_dir_that_is_always_removed() {
    let dir = temp_dir("rq-secret");
    let seen = dir.join("seen.txt");
    let reply = dir.join("reply.json");
    std::fs::write(&reply, r#"{"status":"satisfied","head":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","rereview_required":false}"#).unwrap();
    let curl = script(
        &dir,
        "curl",
        &format!(
            "for a in \"$@\"; do case \"$a\" in @*) f=${{a#@}};; esac; done\nstat -c '%a %n' \"$f\" \"$(dirname \"$f\")\" > {}\ncat {}; printf '\\n200'",
            seen.display(), reply.display()
        ),
    );
    let cfg = config(&dir, true, &curl, None);
    let scratch = dir.join("scratch");
    let run = |cfg: &Path| {
        autospec(&[
            "review-quorum",
            "request",
            "--config",
            cfg.to_str().unwrap(),
            "--pr-node",
            PR,
            "--state-dir",
            dir.join("st").to_str().unwrap(),
            "--scratch-dir",
            scratch.to_str().unwrap(),
        ])
    };
    assert_eq!(run(&cfg).0, Some(0));
    let seen_text = std::fs::read_to_string(&seen).unwrap();
    let modes: Vec<&str> = seen_text
        .lines()
        .map(|l| l.split(' ').next().unwrap())
        .collect();
    assert_eq!(modes, ["600", "700"], "{seen_text}");
    let leftovers = |p: &Path| std::fs::read_dir(p).map(|d| d.count()).unwrap_or(0);
    assert_eq!(
        leftovers(&scratch),
        0,
        "scratch dir must be empty after a call"
    );
    // curl that cannot even be spawned: still nothing left behind, and the error is reported.
    let broken = temp_dir("rq-secret-broken");
    let cfg2 = config(&broken, true, &broken.join("no-such-curl"), None);
    let scratch2 = broken.join("scratch");
    let (code, _, err) = autospec(&[
        "review-quorum",
        "request",
        "--config",
        cfg2.to_str().unwrap(),
        "--pr-node",
        PR,
        "--state-dir",
        broken.join("st").to_str().unwrap(),
        "--scratch-dir",
        scratch2.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(2));
    assert!(err.contains("run curl"), "{err}");
    assert_eq!(leftovers(&scratch2), 0);
}

#[test]
fn herdr_url_is_parsed_not_prefix_matched() {
    let dir = temp_dir("rq-url");
    let log = dir.join("curl.log");
    let curl = script(
        &dir,
        "curl",
        &format!("echo called >> {}; printf '{{}}\\n200'", log.display()),
    );
    let ok_urls = [
        "https://herdr.example.test",
        "http://127.0.0.1:9",
        "http://localhost:9/p",
        "http://[::1]:9",
    ];
    let bad_urls = [
        "http://example.test",
        "http://127.0.0.1.evil.test",
        "http://localhost.evil.test",
        "http://127.0.0.1@evil.test",
        "https://user:pw@herdr.example.test",
        "ftp://127.0.0.1",
        "https://herdr.example.test/?x=1",
        "https://herdr.example.test/#f",
        "http://[::2]",
        "not a url",
        "http://10.0.0.1",
    ];
    let run = |url: &str| {
        let cfg = config(&dir, true, &curl, None);
        let text = std::fs::read_to_string(&cfg)
            .unwrap()
            .replace("http://127.0.0.1:9", url);
        std::fs::write(&cfg, text).unwrap();
        autospec(&[
            "review-quorum",
            "request",
            "--config",
            cfg.to_str().unwrap(),
            "--pr-node",
            PR,
            "--state-dir",
            dir.join("st").to_str().unwrap(),
            "--scratch-dir",
            dir.join("sc").to_str().unwrap(),
        ])
    };
    for url in bad_urls {
        let (code, _, err) = run(url);
        assert_eq!(code, Some(2), "{url}: {err}");
    }
    assert!(!log.exists(), "curl must not run for a refused url");
    for url in ok_urls {
        // curl answers `{}` which is a non-quorum reply: the request is accepted as far as the
        // URL is concerned (the failure, if any, is about the reply, not the address).
        let (_, _, err) = run(url);
        assert!(!err.contains("herdr.url"), "{url}: {err}");
    }
}

// ---------------------------------------------------------------- retract + receipt validation
#[test]
fn a_mistaken_verdict_is_retracted_by_the_same_reviewer_at_the_same_head() {
    let dir = temp_dir("rq-retract");
    let curl = script(&dir, "curl", "exit 9");
    let head = "c".repeat(40);
    let (request, verdict) = verdict_files(&dir, &head, "blocked");
    let cfg = config(&dir, true, &curl, None);
    let go = |extra: &[&str]| {
        let mut a = vec![
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
            "--dry-run",
        ];
        a.extend_from_slice(extra);
        autospec(&a)
    };
    let (_, publish, _) = go(&[]);
    let (code, retract, _) = go(&["--retract"]);
    assert_eq!(code, Some(0), "{retract}");
    let (p, r): (serde_json::Value, serde_json::Value) = (
        serde_json::from_str(&publish).unwrap(),
        serde_json::from_str(&retract).unwrap(),
    );
    let add = &p["bodies"][1];
    let remove = &r["bodies"][0];
    assert_eq!(remove["event_type"], "edge.remove");
    assert_eq!(
        remove["source"], add["source"],
        "ordered after the add on the same source"
    );
    assert_eq!(remove["source_sequence"], 2);
    assert_eq!(remove["payload"]["from_id"], add["payload"]["from_id"]);
    assert_eq!(remove["payload"]["to_id"], add["payload"]["to_id"]);
    assert_eq!(remove["payload"]["attrs"]["commit"], head.as_str());
    assert!(remove["payload"]["reason"].as_str().unwrap().len() > 3);
    assert_eq!(
        p["reviewer"], r["reviewer"],
        "sent with the reviewer's own key"
    );
    // A retraction of a verdict file for a different commit is refused like a publish.
    std::fs::write(
        &verdict,
        format!(r#"{{"commit":"{}","verdict":"blocked"}}"#, "d".repeat(40)),
    )
    .unwrap();
    assert_eq!(go(&["--retract"]).0, Some(2));
}

#[test]
fn receipt_fields_are_validated_before_anything_is_built() {
    let dir = temp_dir("rq-receipt");
    let curl = script(&dir, "curl", "exit 9");
    let head = "c".repeat(40);
    let cfg = config(&dir, true, &curl, None);
    let (request, verdict) = verdict_files(&dir, &head, "lgtm");
    let go = || {
        autospec(&[
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
            "--dry-run",
        ])
    };
    assert_eq!(go().0, Some(0));
    let cases: [(&Path, String); 7] = [
        (
            &verdict,
            format!(r#"{{"commit":"{head}","verdict":"maybe"}}"#),
        ),
        (&verdict, format!(r#"{{"commit":"{head}"}}"#)),
        (
            &verdict,
            format!(r#"{{"commit":"abc1234","verdict":"lgtm"}}"#),
        ),
        (&verdict, format!(r#"["{head}"]"#)),
        (
            &request,
            format!(r#"{{"commit":"{head}","pr":"pr:github.com/o/r#8"}}"#),
        ),
        (&request, r#"{"commit":"abc1234"}"#.to_string()),
        (&request, "[]".to_string()),
    ];
    for (path, body) in cases {
        let original = std::fs::read(path).unwrap();
        std::fs::write(path, &body).unwrap();
        assert_eq!(go().0, Some(2), "{body}");
        std::fs::write(path, original).unwrap();
    }
    assert_eq!(go().0, Some(0));
    // A request that names this very PR is fine, in any case.
    std::fs::write(
        &request,
        format!(r#"{{"commit":"{head}","pr":"PR:GitHub.com/o/r#7"}}"#),
    )
    .unwrap();
    assert_eq!(go().0, Some(0));
}

// ---------------------------------------------------------------- trusted sources, attestation, live
fn run_git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

impl Gate {
    /// Make the repo a git checkout whose default branch (`origin/main`) carries `yaml` as
    /// `.autospec/autospec.yml`; the working tree (the "PR") may then change it freely.
    fn base(&self, yaml: &str) {
        std::fs::write(self.dir.join("base.yml"), yaml).unwrap();
        run_git(&self.dir, &["init", "-q", "-b", "main"]);
        std::fs::write(self.dir.join(".autospec/autospec.yml"), yaml).unwrap();
        run_git(&self.dir, &["add", ".autospec/autospec.yml"]);
        run_git(&self.dir, &["commit", "-q", "-m", "base"]);
        run_git(
            &self.dir,
            &["update-ref", "refs/remotes/origin/main", "HEAD"],
        );
    }

    fn operator(&self, yaml: &str) -> PathBuf {
        self.operator_named("operator.yml", yaml)
    }

    fn operator_named(&self, name: &str, yaml: &str) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::write(&path, yaml).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        path
    }

    fn evaluate_with(&self, body: &str, operator: &Path) -> Option<i32> {
        let file = self.dir.join("in.json");
        std::fs::write(&file, body).unwrap();
        autospec(&[
            "review-quorum",
            "--input",
            file.to_str().unwrap(),
            "--write-report",
            self.report().to_str().unwrap(),
            "--repo-root",
            self.dir.to_str().unwrap(),
            "--operator-config",
            operator.to_str().unwrap(),
        ])
        .0
    }

    fn verify(&self, operator: &Path, extra: &[&str]) -> (Option<i32>, serde_json::Value) {
        let path = format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let out = Command::new(env!("CARGO_BIN_EXE_autospec"))
            .env("PATH", path)
            .args([
                "review-quorum",
                "verify-report",
                "--repo-root",
                self.dir.to_str().unwrap(),
                "--operator-config",
                operator.to_str().unwrap(),
                "--pr",
                "7",
                "--repo",
                "o/r",
            ])
            .args(extra)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        (
            out.status.code(),
            serde_json::from_str(&text).unwrap_or(serde_json::Value::Null),
        )
    }

    fn rewrite_report(&self, edit: impl FnOnce(&mut serde_json::Value)) {
        let mut v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(self.report()).unwrap()).unwrap();
        edit(&mut v);
        std::fs::write(self.report(), v.to_string()).unwrap();
    }
}

fn reasons(v: &serde_json::Value) -> String {
    v["reasons"].to_string()
}

fn key_file(name: &str) -> PathBuf {
    let dir = temp_dir(name);
    let path = dir.join("attest.key");
    std::fs::write(
        &path,
        format!("{}\n", sha256_hex(name.as_bytes()).repeat(2)),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    path
}

#[test]
fn a_pr_cannot_remove_the_requirement_by_editing_its_own_checkout() {
    let gate = Gate::new("rq-trusted-base");
    gate.base("version: 1\nreview_quorum:\n  required: true\n");
    // the PR strips the block and has no report: still blocked, by the default branch's config
    std::fs::write(gate.dir.join(".autospec/autospec.yml"), "version: 1\n").unwrap();
    let (code, plan) = gate.promote("7");
    assert_eq!(code, Some(1), "{plan}");
    assert!(plan.contains("review-quorum.json is missing"), "{plan}");
    // a real pass promotes; the PR then deleting the report is blocked again
    let a = "a".repeat(40);
    assert_eq!(gate.evaluate(&gate_input(&a, "lgtm", "author")), Some(0));
    let (code, plan) = gate.promote("7");
    assert_eq!(code, Some(0), "{plan}");
    let plan_json: serde_json::Value = serde_json::from_str(&plan).unwrap();
    assert_eq!(
        plan_json["review_quorum_head"],
        a.as_str(),
        "the verified head is recorded"
    );
    std::fs::remove_file(gate.report()).unwrap();
    assert_eq!(gate.promote("7").0, Some(1));
    // and `required: false` in the PR's own file changes nothing either
    std::fs::write(
        gate.dir.join(".autospec/autospec.yml"),
        "review_quorum:\n  required: false\n",
    )
    .unwrap();
    assert_eq!(gate.promote("7").0, Some(1));
}

#[test]
fn the_operators_own_config_requires_the_quorum_and_an_unreadable_one_fails_closed() {
    let gate = Gate::new("rq-trusted-operator");
    let none = gate.dir.join("none.yml");
    assert_eq!(
        gate.verify(&none, &[]).0,
        Some(0),
        "no trusted source, no report: not adopted"
    );
    let op = gate.operator("review_quorum:\n  required: true\n");
    let (code, v) = gate.verify(&op, &[]);
    assert_eq!(code, Some(1), "{v}");
    assert!(reasons(&v).contains("missing"), "{v}");
    // a trusted source that exists but cannot be parsed / read means required
    for bad in [
        "review_quorum: [",
        "- just\n- a list\n",
        "x: [\nreview_quorum: false\n",
    ] {
        let op = gate.operator(bad);
        let (code, v) = gate.verify(&op, &[]);
        assert_eq!(code, Some(1), "{bad:?}: {v}");
        assert!(v["required"] == true, "{v}");
    }
    let dir_as_config = gate.dir.join("config-dir");
    std::fs::create_dir(&dir_as_config).unwrap();
    assert_eq!(gate.verify(&dir_as_config, &[]).0, Some(1));
    // an explicit false in the operator's config is the only way to opt out
    let op = gate.operator("review_quorum:\n  required: false\n");
    assert_eq!(gate.verify(&op, &[]).0, Some(0));
}

#[test]
fn an_attested_report_cannot_be_forged_or_edited() {
    let gate = Gate::new("rq-attest");
    let key = key_file("rq-attest-key");
    let op = gate.operator(&format!(
        "review_quorum:\n  required: true\n  attestation_key_file: {}\n",
        key.display()
    ));
    let a = "a".repeat(40);
    assert_eq!(
        gate.evaluate_with(&gate_input(&a, "lgtm", "author"), &op),
        Some(0)
    );
    let (code, v) = gate.verify(&op, &[]);
    assert_eq!(code, Some(0), "{v}");
    assert_eq!(v["origin"], "attested");

    // a hand-written report with every field right but no key material
    let genuine: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(gate.report()).unwrap()).unwrap();
    gate.rewrite_report(|v| {
        v.as_object_mut().unwrap().remove("hmac");
    });
    let (code, v) = gate.verify(&op, &[]);
    assert_eq!(code, Some(1));
    assert!(reasons(&v).contains("hmac"), "{v}");
    // a wrong MAC, and each covered field edited
    for field in [
        "target_id",
        "head",
        "evaluated_at",
        "votes_digest",
        "verdict",
    ] {
        std::fs::write(gate.report(), genuine.to_string()).unwrap();
        gate.rewrite_report(|v| {
            v[field] = match field {
                "evaluated_at" => serde_json::json!(genuine["evaluated_at"].as_u64().unwrap() + 1),
                "head" => serde_json::json!("b".repeat(40)),
                "target_id" => serde_json::json!("pr:github.com/o/r#8"),
                "verdict" => serde_json::json!("pass_with_warnings"),
                _ => serde_json::json!("0".repeat(64)),
            };
        });
        assert_eq!(
            gate.verify(&op, &[]).0,
            Some(1),
            "editing {field} must invalidate the report"
        );
    }
    // signed with another key
    let other = key_file("rq-attest-other");
    let op_other = gate.operator_named(
        "other-operator.yml",
        &format!(
            "review_quorum:\n  required: true\n  attestation_key_file: {}\n",
            other.display()
        ),
    );
    assert_eq!(
        gate.evaluate_with(&gate_input(&a, "lgtm", "author"), &op_other),
        Some(0)
    );
    assert_eq!(gate.verify(&op, &[]).0, Some(1));
    // without a key anywhere the origin is unproven and says so
    let plain = gate.operator("review_quorum:\n  required: true\n");
    assert_eq!(
        gate.evaluate_with(&gate_input(&a, "lgtm", "author"), &plain),
        Some(0)
    );
    let (code, v) = gate.verify(&plain, &[]);
    assert_eq!(code, Some(0), "{v}");
    assert_eq!(v["origin"], "unattested");
    assert!(v["warnings"].to_string().contains("unattested"), "{v}");
}

#[test]
fn an_attestation_key_must_be_private_absolute_and_outside_the_repo() {
    let gate = Gate::new("rq-attest-rules");
    let a = "a".repeat(40);
    let fail = |path: &Path, why: &str| {
        let op = gate.operator(&format!(
            "review_quorum:\n  required: true\n  attestation_key_file: {}\n",
            path.display()
        ));
        assert_eq!(
            gate.evaluate_with(&gate_input(&a, "lgtm", "author"), &op),
            Some(2),
            "{why}"
        );
        let report: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(gate.report()).unwrap()).unwrap();
        assert_eq!(
            report["verdict"], "invalid_input",
            "{why}: a bad key overwrites the report"
        );
        assert_eq!(gate.verify(&op, &[]).0, Some(1), "{why}");
    };
    let good = key_file("rq-attest-good");
    // world-readable
    let open = key_file("rq-attest-open");
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o644)).unwrap();
    fail(&open, "mode 0644");
    // relative
    fail(Path::new("attest.key"), "relative path");
    // symlink
    let link = good.parent().unwrap().join("link.key");
    std::os::unix::fs::symlink(&good, &link).unwrap();
    fail(&link, "symlink");
    // inside the repository
    let inside = gate.dir.join("inside.key");
    std::fs::copy(&good, &inside).unwrap();
    std::fs::set_permissions(&inside, std::fs::Permissions::from_mode(0o600)).unwrap();
    fail(&inside, "inside the repo");
    // too short
    let short = good.parent().unwrap().join("short.key");
    std::fs::write(&short, "tiny").unwrap();
    std::fs::set_permissions(&short, std::fs::Permissions::from_mode(0o600)).unwrap();
    fail(&short, "short key");
    // missing
    fail(&good.parent().unwrap().join("nope.key"), "missing");
}

#[test]
fn a_pass_has_an_age_limit() {
    let gate = Gate::new("rq-age");
    let a = "a".repeat(40);
    let op = gate.operator("review_quorum:\n  required: true\n  max_report_age_seconds: 60\n");
    assert_eq!(
        gate.evaluate_with(&gate_input(&a, "lgtm", "author"), &op),
        Some(0)
    );
    assert_eq!(gate.verify(&op, &[]).0, Some(0));
    let at =
        serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(gate.report()).unwrap())
            .unwrap()["evaluated_at"]
            .as_u64()
            .unwrap();
    gate.rewrite_report(|v| v["evaluated_at"] = serde_json::json!(at - 120));
    let (code, v) = gate.verify(&op, &[]);
    assert_eq!(code, Some(1));
    assert!(
        reasons(&v).contains("older than max_report_age_seconds (60)"),
        "{v}"
    );
    gate.rewrite_report(|v| v["evaluated_at"] = serde_json::json!(at + 100_000));
    assert!(reasons(&gate.verify(&op, &[]).1).contains("future"));
    gate.rewrite_report(|v| {
        v.as_object_mut().unwrap().remove("evaluated_at");
    });
    assert!(reasons(&gate.verify(&op, &[]).1).contains("no evaluated_at"));
    // the default bound is 15 minutes
    let default = gate.operator("review_quorum:\n  required: true\n");
    gate.rewrite_report(|v| v["evaluated_at"] = serde_json::json!(at - 1000));
    assert!(reasons(&gate.verify(&default, &[]).1).contains("(900)"));
}

#[test]
fn the_gate_asks_herdr_at_verify_time_and_a_later_block_or_an_outage_blocks() {
    let gate = Gate::new("rq-live");
    let a = "a".repeat(40);
    let reply = gate.dir.join("reply.json");
    let curl = script(
        &gate.dir,
        "curl",
        &format!("cat {}; printf '\\n200'", reply.display()),
    );
    let op = gate.operator(&format!(
        "review_quorum:\n  required: true\nherdr:\n  enabled: true\n  url: http://127.0.0.1:9\n  ingest_key: {}\n  curl: {}\n",
        sha256_hex(b"k"), curl.display()
    ));
    assert_eq!(
        gate.evaluate_with(&gate_input(&a, "lgtm", "author"), &op),
        Some(0)
    );
    let scratch = gate.dir.join("scratch");
    let live = |extra: &[&str]| {
        let mut args = vec!["--scratch-dir", scratch.to_str().unwrap()];
        args.extend_from_slice(extra);
        gate.verify(&op, &args)
    };
    std::fs::write(&reply, format!(r#"{{"status":"satisfied","head":"{a}"}}"#)).unwrap();
    let (code, v) = live(&[]);
    assert_eq!(code, Some(0), "{v}");
    assert_eq!(v["live"], "checked");
    // a `blocked` filed after the report was written is seen now
    std::fs::write(&reply, format!(r#"{{"status":"blocked","head":"{a}"}}"#)).unwrap();
    let (code, v) = live(&[]);
    assert_eq!(code, Some(1));
    assert!(
        reasons(&v).contains("Herdr says the quorum is blocked"),
        "{v}"
    );
    // Herdr at another head, or not satisfied: not a pass
    std::fs::write(
        &reply,
        format!(r#"{{"status":"satisfied","head":"{}"}}"#, "b".repeat(40)),
    )
    .unwrap();
    assert_eq!(live(&[]).0, Some(1));
    // an outage blocks (curl fails), and so does an HTTP error
    script(&gate.dir, "curl", "echo down >&2; exit 7");
    let (code, v) = live(&[]);
    assert_eq!(code, Some(1), "{v}");
    assert_eq!(v["live"], "error");
    script(&gate.dir, "curl", "printf '{}\\n503'");
    assert_eq!(live(&[]).0, Some(1));
    // the operator can switch the live check off for one run
    assert_eq!(live(&["--no-live"]).0, Some(0));
    // a Herdr config that the operator left readable by others cannot be used: blocked, not skipped
    std::fs::set_permissions(&op, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(live(&[]).0, Some(1));
}

#[test]
fn a_symlinked_or_escaping_report_is_refused() {
    let gate = Gate::new("rq-symlink");
    gate.require();
    let none = gate.dir.join("none.yml");
    let a = "a".repeat(40);
    assert_eq!(gate.evaluate(&gate_input(&a, "lgtm", "author")), Some(0));
    assert_eq!(gate.verify(&none, &[]).0, Some(0));
    let outside = temp_dir("rq-symlink-out");
    let moved = outside.join("review-quorum.json");
    std::fs::rename(gate.report(), &moved).unwrap();
    std::os::unix::fs::symlink(&moved, gate.report()).unwrap();
    let (code, v) = gate.verify(&none, &[]);
    assert_eq!(code, Some(1));
    assert!(reasons(&v).contains("regular file"), "{v}");
    // a symlinked directory in the path resolves outside the repo
    std::fs::remove_file(gate.report()).unwrap();
    std::fs::remove_dir(gate.dir.join(".autospec/reports")).ok();
    let moved_dir = outside.join("reports");
    std::fs::create_dir_all(&moved_dir).unwrap();
    std::fs::write(
        moved_dir.join("review-quorum.json"),
        std::fs::read(&moved).unwrap(),
    )
    .unwrap();
    std::fs::remove_dir_all(gate.dir.join(".autospec/reports")).unwrap();
    std::os::unix::fs::symlink(&moved_dir, gate.dir.join(".autospec/reports")).unwrap();
    let (code, v) = gate.verify(&none, &[]);
    assert_eq!(code, Some(1));
    assert!(reasons(&v).contains("inside the repository"), "{v}");
}

#[test]
fn the_expected_head_must_still_be_live() {
    let gate = Gate::new("rq-expect-head");
    gate.require();
    let none = gate.dir.join("none.yml");
    let a = "a".repeat(40);
    assert_eq!(gate.evaluate(&gate_input(&a, "lgtm", "author")), Some(0));
    assert_eq!(gate.verify(&none, &["--head", &a]).0, Some(0));
    gate.live(&"b".repeat(40), 7);
    let (code, v) = gate.verify(&none, &["--head", &a]);
    assert_eq!(code, Some(1));
    assert!(reasons(&v).contains("is not the live PR head"), "{v}");
}

#[test]
fn forged_linked_or_loose_request_state_is_not_trusted() {
    let r = Request::new("rq-state", "true");
    let head = "a".repeat(40);
    std::fs::create_dir_all(&r.state).unwrap();
    std::fs::set_permissions(&r.state, std::fs::Permissions::from_mode(0o700)).unwrap();
    // a hand-written "dispatched" marker (no MAC) suppresses nothing: it counts as an unknown claim
    std::fs::write(r.marker(&head), r#"{"state":"dispatched"}"#).unwrap();
    std::fs::set_permissions(r.marker(&head), std::fs::Permissions::from_mode(0o600)).unwrap();
    let out = r.run().1;
    assert!(
        out.contains("request_in_progress") && !out.contains("already_requested"),
        "{out}"
    );
    // a symlinked marker is never followed
    std::fs::remove_file(r.marker(&head)).unwrap();
    let target = r.dir.join("elsewhere.json");
    std::fs::write(&target, r#"{"state":"dispatched"}"#).unwrap();
    std::os::unix::fs::symlink(&target, r.marker(&head)).unwrap();
    assert!(r.run().1.contains("request_in_progress"));
    assert_eq!(r.dispatches(), 0);
    // a marker with a valid MAC but a different MAC key (another install) is not ours either
    std::fs::remove_file(r.marker(&head)).unwrap();
    forge_marker(&r.state, &r.marker(&head), &head, "dispatched", 0, 1, 0, 0);
    std::fs::write(r.state.join(".nonce"), "z".repeat(64)).unwrap();
    assert!(r.run().1.contains("request_in_progress"));
    // a group-writable state dir is refused outright, and so is a symlinked one
    std::fs::set_permissions(&r.state, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert_eq!(r.run().0, Some(2));
    std::fs::set_permissions(&r.state, std::fs::Permissions::from_mode(0o700)).unwrap();
    let link = r.dir.join("state-link");
    std::os::unix::fs::symlink(&r.state, &link).unwrap();
    let mut a = r.args();
    let at = a.iter().position(|x| x == "--state-dir").unwrap() + 1;
    a[at] = link.to_str().unwrap().to_string();
    assert_eq!(
        autospec(&a.iter().map(String::as_str).collect::<Vec<_>>()).0,
        Some(2)
    );
}

#[test]
fn a_dispatch_that_outlives_its_timeout_is_killed_and_backed_off() {
    let dir = temp_dir("rq-timeout");
    let pids = dir.join("pids");
    let r = Request::new(
        "rq-timeout-run",
        &format!(
            "echo $$ >> {p}\nsleep 30 &\necho $! >> {p}\nwait",
            p = pids.display()
        ),
    );
    let text = std::fs::read_to_string(&r.cfg).unwrap() + "  dispatch_timeout_seconds: 1\n";
    std::fs::write(&r.cfg, text).unwrap();
    let started = std::time::Instant::now();
    let (code, _, err) = r.run();
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("killed"), "{err}");
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    let listed = std::fs::read_to_string(&pids).unwrap();
    let alive = |pid: &str| Path::new(&format!("/proc/{}", pid.trim())).exists();
    for _ in 0..40 {
        if !listed.lines().any(alive) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(
        !listed.lines().any(alive),
        "the whole process group must be dead: {listed}"
    );
    let marker: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(r.marker(&"a".repeat(40))).unwrap()).unwrap();
    assert_eq!(marker["state"], "failed");
    assert!(
        marker["detail"].as_str().unwrap().contains("exceeded"),
        "{marker}"
    );
    assert!(r.run().1.contains("request_backoff"));
    assert_eq!(r.dispatches(), 1);
}

// ---------------------------------------------------------------- T16: the merge path
impl Gate {
    /// A fake `$HOME` whose `.autospec/autospec.yml` is a copy of `operator` (or absent): the
    /// wrapper takes no `--operator-config`, so the harness states the operator policy the way
    /// the product does, through the home directory (honored by debug builds only).
    fn home_with(&self, operator: Option<&Path>) -> PathBuf {
        let name = operator
            .map(|o| o.file_name().unwrap().to_string_lossy().into_owned())
            .unwrap_or_else(|| "none".into());
        let home = self.dir.join(format!("home-{name}"));
        std::fs::create_dir_all(home.join(".autospec")).unwrap();
        if let Some(op) = operator.filter(|o| o.exists()) {
            std::fs::copy(op, home.join(".autospec/autospec.yml")).unwrap();
        }
        home
    }

    /// Run the REAL `autospec-guarded-merge.sh` (fake forge, real `autospec`). `bin` is the
    /// directory searched first for `gh`/`autospec`.
    fn guarded_merge(&self, bin: &Path, operator: Option<&Path>) -> (Option<i32>, String) {
        let path = format!("{}:/usr/bin:/bin", bin.display());
        let mut cmd = Command::new("bash");
        cmd.env("PATH", path)
            .env("HOME", self.home_with(operator))
            .current_dir(&self.dir)
            .args([
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../scripts/autospec-guarded-merge.sh"
                ),
                "--pr",
                "7",
                "--repo",
                "o/r",
                "--no-require-checks",
                "--no-require-local-build",
            ]);
        let out = cmd.output().unwrap();
        (
            out.status.code(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
    }

    fn merged(&self) -> bool {
        self.dir.join("merged").exists()
    }

    fn merge_log(&self) -> String {
        std::fs::read_to_string(self.dir.join("merge.log")).unwrap_or_default()
    }

    fn labels_log(&self) -> String {
        std::fs::read_to_string(self.dir.join("labels.log")).unwrap_or_default()
    }

    fn merge_gate(&self, operator: &Path) -> (Option<i32>, serde_json::Value) {
        let path = format!("{}:/usr/bin:/bin", self.bin.display());
        let out = Command::new(env!("CARGO_BIN_EXE_autospec"))
            .env("PATH", path)
            .args([
                "review-quorum",
                "merge-gate",
                "--repo-root",
                self.dir.to_str().unwrap(),
                "--operator-config",
                operator.to_str().unwrap(),
                "--pr",
                "7",
                "--repo",
                "o/r",
                "--script-version",
                "2",
            ])
            .output()
            .unwrap();
        (
            out.status.code(),
            serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null),
        )
    }
}

/// A PR that is allowed to merge: the operator requires the quorum, attests with a key, and a
/// genuine pass for the live head `a` exists.
fn attested_pass(gate: &Gate, name: &str) -> PathBuf {
    let key = key_file(name);
    let op = gate.operator(&format!(
        "review_quorum:\n  required: true\n  attestation_key_file: {}\n",
        key.display()
    ));
    let a = "a".repeat(40);
    assert_eq!(
        gate.evaluate_with(&gate_input(&a, "lgtm", "author"), &op),
        Some(0)
    );
    op
}

#[test]
fn h4_a_worker_moving_the_local_base_ref_cannot_remove_the_requirement() {
    let gate = Gate::new("t16-h4-update-ref");
    // the forge's default branch requires the quorum; there is NO report
    gate.base("version: 1\nreview_quorum:\n  required: true\n");
    // the worker commits the deletion of the config, then points every local "base" ref at it
    // and repoints the remote, exactly as `p4-base-ref.sh` P4b does
    std::fs::remove_file(gate.dir.join(".autospec/autospec.yml")).unwrap();
    run_git(&gate.dir, &["add", "-A", ".autospec"]);
    run_git(&gate.dir, &["commit", "-q", "-m", "drop the gate"]);
    for r in ["refs/remotes/origin/main", "refs/remotes/origin/master"] {
        run_git(&gate.dir, &["update-ref", r, "HEAD"]);
    }
    run_git(
        &gate.dir,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    );
    run_git(
        &gate.dir,
        &["config", "remote.origin.url", "/nonexistent/attacker.git"],
    );
    let (code, plan) = gate.promote("7");
    assert_eq!(code, Some(1), "promotion must still be blocked: {plan}");
    assert!(plan.contains("review-quorum.json is missing"), "{plan}");
    let none = gate.dir.join("no-operator.yml");
    let (code, v) = gate.merge_gate(&none);
    assert_eq!(code, Some(3), "{v}");
    assert!(v["merge_head"].is_null(), "{v}");
    let (code, out) = gate.guarded_merge(&gate.bin, Some(&none));
    assert_ne!(code, Some(0), "{out}");
    assert!(!gate.merged(), "the guarded merge must not merge: {out}");
}

#[test]
fn h4_a_stale_clone_or_an_unusual_default_branch_does_not_hide_the_requirement() {
    let gate = Gate::new("t16-h4-stale");
    gate.base("review_quorum:\n  required: true\n");
    // the local clone has never fetched and knows no origin/* at all (default branch 'develop')
    run_git(&gate.dir, &["checkout", "-q", "-b", "develop"]);
    run_git(&gate.dir, &["update-ref", "-d", "refs/remotes/origin/main"]);
    std::fs::remove_file(gate.dir.join(".autospec/autospec.yml")).unwrap();
    assert_eq!(gate.promote("7").0, Some(1));
}

#[test]
fn h4_an_unreadable_forge_means_required_and_a_404_is_absent_only_for_a_readable_repo() {
    let gate = Gate::new("t16-h4-forge");
    let none = gate.dir.join("no-operator.yml");
    // no base file and the repo readable: a 404 is "absent" -> verify-report (not strict) passes
    assert_eq!(gate.verify(&none, &[]).0, Some(0));
    // any non-404 failure is "required", and the report is missing
    std::fs::write(gate.dir.join("api_fail"), "").unwrap();
    let (code, v) = gate.verify(&none, &[]);
    assert_eq!(code, Some(1), "{v}");
    assert_eq!(v["required"], true, "{v}");
    assert!(reasons(&v).contains("502"), "{v}");
    // a 404 on the file whose repository itself cannot be read (token without access) is not
    // proof that the file is absent
    std::fs::remove_file(gate.dir.join("api_fail")).unwrap();
    std::fs::write(gate.dir.join("repo_fail"), "").unwrap();
    let (code, v) = gate.verify(&none, &[]);
    assert_eq!(code, Some(1), "{v}");
    assert!(reasons(&v).contains("not readable"), "{v}");
    // and a bogus --repo cannot be turned into a path
    assert_eq!(gate.verify(&none, &["--repo", "o/r/x/y"]).0, Some(2));
}

#[test]
fn h3_the_merge_path_has_no_default_that_merges_without_a_review() {
    let gate = Gate::new("t16-h3-default");
    // no operator config, no base config, no report: "no policy stated" is required
    let none = gate.dir.join("no-operator.yml");
    let (code, v) = gate.merge_gate(&none);
    assert_eq!(code, Some(3), "{v}");
    assert!(reasons(&v).contains("states a review_quorum policy"), "{v}");
    let (code, out) = gate.guarded_merge(&gate.bin, Some(&none));
    assert_eq!(
        code,
        Some(3),
        "a quorum refusal has its own exit code: {out}"
    );
    assert!(out.contains("blocked review_quorum_not_verified"), "{out}");
    assert!(!gate.merged(), "{out}");
    // a refused merge also clears the stale marks
    assert!(
        gate.labels_log()
            .contains("--remove-label autospec:verified"),
        "{}",
        gate.labels_log()
    );
    // the explicit operator opt-out is the only way back, and the merge is still pinned
    let off = gate.operator("review_quorum:\n  required: false\n");
    let (code, v) = gate.merge_gate(&off);
    assert_eq!(code, Some(0), "{v}");
    assert_eq!(v["merge_head"], "a".repeat(40).as_str(), "{v}");
    let (code, out) = gate.guarded_merge(&gate.bin, Some(&off));
    assert_eq!(code, Some(0), "{out}");
    assert!(
        gate.merge_log()
            .contains(&format!("--match-head-commit {}", "a".repeat(40))),
        "{}",
        gate.merge_log()
    );
    // ... but not by the base alone being silent about a PR that adds a requirement itself
    let gate = Gate::new("t16-h3-checkout-adds");
    gate.require();
    assert_eq!(gate.merge_gate(&off).0, Some(3));
}

#[test]
fn h3_a_required_quorum_needs_a_report_origin_proof_and_the_live_head() {
    let gate = Gate::new("t16-h3-required");
    // required but no attestation key and no Herdr: no origin proof, refused even with a report
    let op = gate.operator("review_quorum:\n  required: true\n");
    let a = "a".repeat(40);
    assert_eq!(
        gate.evaluate_with(&gate_input(&a, "lgtm", "author"), &op),
        Some(0)
    );
    let (code, v) = gate.merge_gate(&op);
    assert_eq!(code, Some(3), "{v}");
    assert!(reasons(&v).contains("origin proof"), "{v}");
    // attested + fresh + head matches: merges, pinned to that head
    let gate = Gate::new("t16-h3-attested");
    let op = attested_pass(&gate, "t16-h3-attested");
    let (code, v) = gate.merge_gate(&op);
    assert_eq!(code, Some(0), "{v}");
    assert_eq!(v["origin"], "attested", "{v}");
    assert_eq!(v["merge_head"], a.as_str(), "{v}");
    let (code, out) = gate.guarded_merge(&gate.bin, Some(&op));
    assert_eq!(code, Some(0), "{out}");
    assert!(gate.merged(), "{out}");
    // the head moves BEFORE the gate: the old pass no longer holds
    let gate = Gate::new("t16-h3-moved-before");
    let op = attested_pass(&gate, "t16-h3-moved-before");
    gate.live(&"b".repeat(40), 7);
    let (code, out) = gate.guarded_merge(&gate.bin, Some(&op));
    assert_eq!(code, Some(3), "{out}");
    assert!(!gate.merged(), "{out}");
    assert!(gate.labels_log().contains("autospec:needs-human-review"));
}

#[test]
fn h3_a_head_pushed_after_verification_cannot_merge() {
    let gate = Gate::new("t16-h3-race");
    let op = attested_pass(&gate, "t16-h3-race");
    // the fake forge moves the PR to head B right after it answered the gate's head query
    std::fs::write(
        gate.dir.join("moved.json"),
        format!(
            r#"{{"headRefOid":"{}","url":"https://github.com/o/r/pull/7"}}"#,
            "b".repeat(40)
        ),
    )
    .unwrap();
    let (code, out) = gate.guarded_merge(&gate.bin, Some(&op));
    assert_ne!(code, Some(0), "{out}");
    assert!(
        gate.merge_log()
            .contains(&format!("--match-head-commit {}", "a".repeat(40))),
        "the merge must be pinned to the verified head: {}",
        gate.merge_log()
    );
    assert!(
        !gate.merged(),
        "a head moved after verification must not merge: {out}"
    );
}

#[test]
fn h3_a_missing_or_too_old_autospec_binary_refuses_instead_of_falling_back() {
    let gate = Gate::new("t16-h3-binary");
    let op = attested_pass(&gate, "t16-h3-binary");
    // PATH without autospec
    let only_gh = temp_dir("t16-h3-only-gh");
    std::fs::copy(gate.bin.join("gh"), only_gh.join("gh")).unwrap();
    let (code, out) = gate.guarded_merge(&only_gh, Some(&op));
    assert_eq!(code, Some(2), "{out}");
    assert!(out.contains("autospec binary not found"), "{out}");
    assert!(!gate.merged(), "{out}");
    // an autospec that predates `merge-gate` (answers "unknown command" and exits 2)
    let old = temp_dir("t16-h3-old");
    std::fs::copy(gate.bin.join("gh"), old.join("gh")).unwrap();
    script(
        &old,
        "autospec",
        "echo 'unknown autospec command' >&2; exit 2",
    );
    let (code, out) = gate.guarded_merge(&old, Some(&op));
    assert_eq!(code, Some(3), "{out}");
    assert!(!gate.merged(), "{out}");
    // promote-pr has no legacy fallback either
    let (code, plan) = {
        let out = Command::new("bash")
            .env("PATH", format!("{}:/usr/bin:/bin", only_gh.display()))
            .args([
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../scripts/autospec-promote-pr.sh"
                ),
                "--dry-run",
                "--repo-root",
                gate.dir.to_str().unwrap(),
                "--repo",
                "o/r",
                "--pr",
                "7",
            ])
            .output()
            .unwrap();
        (
            out.status.code(),
            std::fs::read_to_string(gate.dir.join(".autospec/reports/promotion-plan.json"))
                .unwrap_or_default(),
        )
    };
    assert_eq!(code, Some(1), "{plan}");
    assert!(plan.contains("autospec binary unavailable"), "{plan}");
}

#[test]
fn h3_a_moved_head_invalidates_the_verified_label_at_the_next_promotion() {
    let gate = Gate::new("t16-h3-labels");
    gate.require();
    let a = "a".repeat(40);
    assert_eq!(gate.evaluate(&gate_input(&a, "lgtm", "author")), Some(0));
    let confirm = |gate: &Gate| {
        let out = Command::new("bash")
            .env("PATH", format!("{}:/usr/bin:/bin", gate.bin.display()))
            .args([
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../scripts/autospec-promote-pr.sh"
                ),
                "--confirm",
                "--repo-root",
                gate.dir.to_str().unwrap(),
                "--repo",
                "o/r",
                "--pr",
                "7",
            ])
            .output()
            .unwrap();
        out.status.code()
    };
    assert_eq!(confirm(&gate), Some(0));
    assert!(gate.labels_log().contains("--add-label autospec:verified"));
    assert!(!gate
        .labels_log()
        .contains("--remove-label autospec:verified"));
    // the PR head moves; the stale report no longer holds and the mark is removed
    gate.live(&"b".repeat(40), 7);
    assert_eq!(confirm(&gate), Some(1));
    assert!(
        gate.labels_log()
            .contains("--remove-label autospec:verified"),
        "{}",
        gate.labels_log()
    );
}

// ---------------------------------------------------------------- T16 review round 2
impl Gate {
    fn guarded_merge_with(&self, merge_args: &str, operator: &Path) -> (Option<i32>, String) {
        let out = Command::new("bash")
            .env("PATH", format!("{}:/usr/bin:/bin", self.bin.display()))
            .env("HOME", self.home_with(Some(operator)))
            .current_dir(&self.dir)
            .args([
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../scripts/autospec-guarded-merge.sh"
                ),
                "--pr",
                "7",
                "--repo",
                "o/r",
                "--no-require-checks",
                "--no-require-local-build",
                "--merge-args",
                merge_args,
            ])
            .output()
            .unwrap();
        (
            out.status.code(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
    }
}

#[test]
fn the_merge_is_pinned_to_the_callers_head_and_never_silently_overridden() {
    let gate = Gate::new("t16-r2-pin");
    let off = gate.operator("review_quorum:\n  required: false\n");
    // the caller pinned head B, the PR is at A: refused with the quorum code, nothing merges
    let b = "b".repeat(40);
    let (code, out) = gate.guarded_merge_with(
        &format!("--admin --squash --delete-branch --match-head-commit {b}"),
        &off,
    );
    assert_eq!(code, Some(3), "{out}");
    assert!(out.contains("not the expected head"), "{out}");
    assert!(!gate.merged(), "{out}");
    // the caller pinned the head it saw: merges, and the pin is still that head
    let a = "a".repeat(40);
    let (code, out) = gate.guarded_merge_with(
        &format!("--admin --squash --delete-branch --match-head-commit {a}"),
        &off,
    );
    assert_eq!(code, Some(0), "{out}");
    assert!(gate
        .merge_log()
        .contains(&format!("--match-head-commit {a}")));
}

#[test]
fn a_stale_caller_script_is_refused_with_the_update_hint() {
    let gate = Gate::new("t16-r2-stale");
    let off = gate.operator("review_quorum:\n  required: false\n");
    let out = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .env("PATH", format!("{}:/usr/bin:/bin", gate.bin.display()))
        .args([
            "review-quorum",
            "merge-gate",
            "--repo-root",
            gate.dir.to_str().unwrap(),
            "--operator-config",
            off.to_str().unwrap(),
            "--pr",
            "7",
            "--repo",
            "o/r",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("install.sh --update"));
}

#[test]
fn read_and_trim_traps_do_not_turn_into_an_opt_out() {
    let gate = Gate::new("t16-r2-yaml");
    gate.base("version: 1\n");
    // NBSP after `false` is not `false`
    let nbsp = gate.operator("review_quorum:\n  required: false\u{a0}\n");
    let (code, v) = gate.verify(&nbsp, &[]);
    assert_eq!(code, Some(1), "{v}");
    assert_eq!(v["required"], true, "{v}");
    // a blank line after `herdr:` does not hide an enabled block
    let herdr = gate.operator(
        "review_quorum:\n  required: false\nherdr:\n\n  enabled: true\n  url: https://h.example\n  ingest_key: k\n",
    );
    let (_, v) = gate.verify(&herdr, &[]);
    assert_eq!(
        v["required"], true,
        "an enabled herdr block requires the quorum: {v}"
    );
    // NBSP-padded `true` is an unreadable herdr block: fail closed, not "disabled"
    let odd = gate.operator("review_quorum:\n  required: false\nherdr:\n  enabled: true\u{a0}\n");
    assert_eq!(gate.verify(&odd, &[]).1["required"], true);
}

#[test]
fn a_404_is_only_a_404_and_a_stray_report_cannot_override_an_opt_out() {
    let gate = Gate::new("t16-r2-404");
    let none = gate.dir.join("no-operator.yml");
    // an error that merely contains the digits 404 is not "file absent"
    std::fs::write(gate.dir.join("api_noise"), "").unwrap();
    let (code, v) = gate.verify(&none, &[]);
    assert_eq!(code, Some(1), "{v}");
    assert_eq!(v["required"], true, "{v}");
    // an opted-out repository ignores a stray (even failing) head-bound report
    let gate = Gate::new("t16-r2-stray");
    let off = gate.operator("review_quorum:\n  required: false\n");
    gate.live(&"a".repeat(40), 7);
    std::fs::write(
        gate.report(),
        r#"{"source":"head-bound-review-quorum","verdict":"blocked","promotion_ready":false,"head":"cccccccccccccccccccccccccccccccccccccccc"}"#,
    )
    .unwrap();
    let (code, v) = gate.merge_gate(&off);
    assert_eq!(code, Some(0), "{v}");
}

#[test]
fn an_untracked_key_in_an_enclosing_repo_is_fine_a_tracked_one_is_not() {
    let gate = Gate::new("t16-r2-key");
    let home = temp_dir("t16-r2-dotfiles");
    run_git(&home, &["init", "-q", "-b", "main"]);
    let key = home.join("attest.key");
    std::fs::copy(key_file("t16-r2-key-src"), &key).unwrap();
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    let op = gate.operator(&format!(
        "review_quorum:\n  required: true\n  attestation_key_file: {}\n",
        key.display()
    ));
    let a = "a".repeat(40);
    assert_eq!(
        gate.evaluate_with(&gate_input(&a, "lgtm", "author"), &op),
        Some(0)
    );
    run_git(&home, &["add", "-f", "attest.key"]);
    assert_eq!(
        gate.evaluate_with(&gate_input(&a, "lgtm", "author"), &op),
        Some(2)
    );
}

#[test]
fn surviving_mutants_a_failed_head_read_and_the_pass_checks() {
    // not required (explicit opt-out) but the live head cannot be read: nothing to pin, refuse
    let gate = Gate::new("t16-r2-m1");
    let off = gate.operator("review_quorum:\n  required: false\n");
    std::fs::write(gate.dir.join("pr_fail"), "").unwrap();
    let (code, v) = gate.merge_gate(&off);
    assert_eq!(
        code,
        Some(4),
        "a failed read is retryable, not a policy refusal: {v}"
    );
    assert!(reasons(&v).contains("live PR head"), "{v}");
    assert!(v["merge_head"].is_null(), "{v}");

    // required, no key: a report whose verdict is pass but promotion_ready is false is refused
    let gate = Gate::new("t16-r2-m2");
    gate.require();
    let a = "a".repeat(40);
    assert_eq!(gate.evaluate(&gate_input(&a, "lgtm", "author")), Some(0));
    gate.rewrite_report(|v| v["promotion_ready"] = serde_json::json!(false));
    let (code, plan) = gate.promote("7");
    assert_eq!(code, Some(1), "{plan}");
    assert!(plan.contains("not a head-bound pass"), "{plan}");
    // ... and pass_with_warnings is not a head-bound pass either
    assert_eq!(gate.evaluate(&gate_input(&a, "lgtm", "author")), Some(0));
    gate.rewrite_report(|v| v["verdict"] = serde_json::json!("pass_with_warnings"));
    let (code, plan) = gate.promote("7");
    assert_eq!(code, Some(1), "{plan}");
    assert!(plan.contains("not a head-bound pass"), "{plan}");

    // NOT required and no stated policy (legacy): a report from our evaluator still has to be
    // a head-bound pass (the `|| ours` branch)
    let gate = Gate::new("t16-r2-m3");
    assert_eq!(gate.evaluate(&gate_input(&a, "lgtm", "author")), Some(0));
    gate.rewrite_report(|v| v["promotion_ready"] = serde_json::json!(false));
    let (code, plan) = gate.promote("7");
    assert_eq!(code, Some(1), "{plan}");
    assert!(plan.contains("not a head-bound pass"), "{plan}");
}

// ---- Round 3 (re-review of 1045dcd5) ---------------------------------------------------

impl Gate {
    fn merge_gate_env(&self, operator: &Path, env: &[(&str, &str)]) -> std::process::Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_autospec"));
        cmd.env("PATH", format!("{}:/usr/bin:/bin", self.bin.display()));
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.args([
            "review-quorum",
            "merge-gate",
            "--repo-root",
            self.dir.to_str().unwrap(),
            "--operator-config",
            operator.to_str().unwrap(),
            "--pr",
            "7",
            "--repo",
            "o/r",
            "--script-version",
            "2",
        ])
        .output()
        .unwrap()
    }
}

#[test]
fn m6_a_trusted_config_the_parser_might_half_read_is_required_not_opted_out() {
    let gate = Gate::new("t16-r3-m6");
    let control = gate.operator_named("control.yml", "review_quorum:\n  required: false\n");
    assert_eq!(gate.merge_gate(&control).0, Some(0), "the control opts out");
    for (name, yaml) in [
        ("trailing.yml", "review_quorum:\n  required: false\n}}}\n"),
        (
            "tab-line.yml",
            "review_quorum:\n  required: false\n\tfoo: 1\n",
        ),
        ("tab-sep.yml", "review_quorum:\n  required:\tfalse\n"),
        ("open-flow.yml", "review_quorum: {required: false\n"),
        (
            "open-list.yml",
            "review_quorum:\n  required: false\nother: [1, 2\n",
        ),
    ] {
        let op = gate.operator_named(name, yaml);
        let (code, body) = gate.merge_gate(&op);
        assert_eq!(code, Some(3), "{name} must not opt out: {body}");
        assert!(body.is_null() || body["ok"] != true, "{name}");
    }
}

#[test]
fn m7_could_not_determine_is_exit_4_and_a_policy_refusal_is_exit_3() {
    let gate = Gate::new("t16-r3-m7");
    let off = gate.operator_named("off.yml", "review_quorum:\n  required: false\n");
    std::fs::write(gate.dir.join("pr_fail"), "").unwrap();
    let out = gate.merge_gate_env(&off, &[]);
    assert_eq!(out.status.code(), Some(4), "{out:?}");
    let body: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(body["retry"], true, "{body}");
    std::fs::remove_file(gate.dir.join("pr_fail")).unwrap();
    // a policy refusal (a required quorum with no report) is not retryable
    let req = gate.operator_named("req.yml", "review_quorum:\n  required: true\n");
    let out = gate.merge_gate_env(&req, &[]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let body: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(body["retry"], false, "{body}");
    // a mixed failure (a transport error AND a policy reason) is a refusal
    std::fs::write(gate.dir.join("pr_fail"), "").unwrap();
    let out = gate.merge_gate_env(&req, &[]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
}

#[test]
fn m7_the_wrapper_passes_exit_4_and_3_through_and_merges_nothing() {
    let gate = Gate::new("t16-r3-m7-wrap");
    let off = gate.operator_named("off.yml", "review_quorum:\n  required: false\n");
    // the wrapper's own head read succeeds; only the gate's PR view fails
    std::fs::write(gate.dir.join("pr_fail"), "").unwrap();
    let (code, out) = gate.guarded_merge(&gate.bin, Some(&off));
    assert!(!gate.merged(), "{out}");
    assert!(
        matches!(code, Some(4) | Some(3) | Some(2)),
        "{code:?} {out}"
    );
    std::fs::remove_file(gate.dir.join("pr_fail")).unwrap();
    let (code, out) = gate.guarded_merge(&gate.bin, None);
    assert_eq!(code, Some(3), "no policy at all is a refusal: {out}");
    assert!(!gate.merged());
}

#[test]
fn config_check_warns_loudly_with_the_exact_line_and_never_writes() {
    let gate = Gate::new("t16-r3-cfgcheck");
    let run = |op: &Path| {
        Command::new(env!("CARGO_BIN_EXE_autospec"))
            .args([
                "review-quorum",
                "config-check",
                "--operator-config",
                op.to_str().unwrap(),
            ])
            .output()
            .unwrap()
    };
    let missing = gate.dir.join("absent.yml");
    let out = run(&missing);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("WARNING") && err.contains("required: false"),
        "{err}"
    );
    assert!(!missing.exists(), "the check must not seed a policy");
    let stated = gate.operator_named("stated.yml", "review_quorum:\n  required: true\n");
    assert_eq!(run(&stated).status.code(), Some(0));
    let broken = gate.operator_named("broken.yml", "review_quorum:\n  required: false\n}}}\n");
    assert_eq!(
        run(&broken).status.code(),
        Some(1),
        "unparseable is not a policy"
    );
}

#[test]
fn m5_both_spellings_of_the_caller_pin_are_one_pin() {
    let gate = Gate::new("t16-r3-pin-eq");
    let off = gate.operator_named("off.yml", "review_quorum:\n  required: false\n");
    let a = "a".repeat(40);
    let (code, out) = gate.guarded_merge_with(
        &format!("--admin --squash --delete-branch --match-head-commit={a}"),
        &off,
    );
    assert_eq!(code, Some(0), "{out}");
    let log = gate.merge_log();
    assert_eq!(log.matches("--match-head-commit").count(), 1, "{log}");
    assert!(log.contains(&format!("--match-head-commit {a}")), "{log}");
    // a differing caller pin (= form) is refused and nothing merges
    let gate = Gate::new("t16-r3-pin-eq2");
    let off = gate.operator_named("off.yml", "review_quorum:\n  required: false\n");
    let b = "b".repeat(40);
    let (code, out) =
        gate.guarded_merge_with(&format!("--admin --squash --match-head-commit={b}"), &off);
    assert_eq!(code, Some(3), "{out}");
    assert!(!gate.merged());
}

#[test]
fn g30_the_gate_runs_gh_from_root_with_the_ambient_gh_environment_scrubbed() {
    let gate = Gate::new("t16-r3-env");
    // a recording gh in front of the fake forge
    let rec = gate.dir.join("rec");
    std::fs::create_dir_all(&rec).unwrap();
    let real = gate.bin.join("gh");
    let log = gate.dir.join("ghenv.log");
    script(
        &rec,
        "gh",
        &format!(
            "pwd >> {l}\nenv | sort >> {l}\nexec {r} \"$@\"",
            l = log.display(),
            r = real.display()
        ),
    );
    std::os::unix::fs::symlink(gate.bin.join("autospec"), rec.join("autospec")).unwrap();
    let off = gate.operator_named("off.yml", "review_quorum:\n  required: false\n");
    let out = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .env("PATH", format!("{}:/usr/bin:/bin", rec.display()))
        .env("GH_HOST", "evil.example")
        .env("GH_REPO", "evil/x")
        .env("GH_CONFIG_DIR", "/nonexistent-gh")
        .env("GH_ENTERPRISE_TOKEN", "x")
        .env("GITHUB_ENTERPRISE_TOKEN", "x")
        .env("GIT_DIR", "/nonexistent-git")
        .env("GIT_WORK_TREE", "/nonexistent-wt")
        .args([
            "review-quorum",
            "merge-gate",
            "--repo-root",
            gate.dir.to_str().unwrap(),
            "--operator-config",
            off.to_str().unwrap(),
            "--pr",
            "7",
            "--repo",
            "o/r",
            "--script-version",
            "2",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let seen = std::fs::read_to_string(&log).unwrap();
    for name in [
        "GH_HOST",
        "GH_REPO",
        "GH_CONFIG_DIR",
        "GH_ENTERPRISE_TOKEN",
        "GITHUB_ENTERPRISE_TOKEN",
        "GIT_DIR",
        "GIT_WORK_TREE",
    ] {
        assert!(
            !seen.lines().any(|l| l.starts_with(&format!("{name}="))),
            "{name} leaked into gh: {seen}"
        );
    }
    let cwds: Vec<&str> = seen
        .lines()
        .filter(|l| l.starts_with('/') && !l.contains('='))
        .collect();
    assert!(
        !cwds.is_empty() && cwds.iter().all(|c| *c == "/"),
        "gh must run from /: {cwds:?}"
    );
}

#[cfg(not(debug_assertions))]
#[test]
fn m4_a_release_build_refuses_a_caller_chosen_trust_anchor() {
    let gate = Gate::new("t16-r3-m4");
    let off = gate.operator_named("off.yml", "review_quorum:\n  required: false\n");
    let out = gate.merge_gate_env(&off, &[]);
    assert_ne!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stderr).contains("does not accept"));
}

#[test]
fn m4_the_wrapper_has_no_operator_config_or_no_live_flag() {
    let gate = Gate::new("t16-r3-m4w");
    for flag in ["--operator-config", "--no-live"] {
        let out = Command::new("bash")
            .env("PATH", format!("{}:/usr/bin:/bin", gate.bin.display()))
            .current_dir(&gate.dir)
            .args([
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../scripts/autospec-guarded-merge.sh"
                ),
                "--pr",
                "7",
                "--repo",
                "o/r",
                flag,
                "/tmp/x",
            ])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{flag}");
        assert!(!gate.merged());
    }
}

fn merge_gate_raw(gate: &Gate, extra: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
        .env("PATH", format!("{}:/usr/bin:/bin", gate.bin.display()))
        .args([
            "review-quorum",
            "merge-gate",
            "--repo-root",
            gate.dir.to_str().unwrap(),
            "--script-version",
            "2",
        ])
        .args(extra)
        .output()
        .unwrap()
}

#[test]
fn r6_merge_gate_requires_both_pr_and_repo() {
    let gate = Gate::new("t16-r3-r6");
    let off = gate.operator_named("off.yml", "review_quorum:\n  required: false\n");
    let op = off.to_str().unwrap();
    let ok = merge_gate_raw(
        &gate,
        &["--operator-config", op, "--pr", "7", "--repo", "o/r"],
    );
    assert_eq!(ok.status.code(), Some(0), "control: {ok:?}");
    for extra in [
        vec!["--operator-config", op, "--pr", "7"],
        vec!["--operator-config", op, "--repo", "o/r"],
    ] {
        let out = merge_gate_raw(&gate, &extra);
        assert_eq!(out.status.code(), Some(2), "{extra:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("requires --pr and --repo"));
        assert!(!String::from_utf8_lossy(&out.stdout).contains("merge_head"));
    }
}

#[test]
fn r7_the_head_pin_must_be_a_canonical_full_sha() {
    let gate = Gate::new("t16-r3-r7");
    let off = gate.operator_named("off.yml", "review_quorum:\n  required: false\n");
    let op = off.to_str().unwrap();
    let base = [
        "--operator-config",
        op,
        "--pr",
        "7",
        "--repo",
        "o/r",
        "--head",
    ];
    for bad in ["abc123", "main", &"g".repeat(40), &"a".repeat(41)] {
        let mut args = base.to_vec();
        args.push(bad);
        let out = merge_gate_raw(&gate, &args);
        assert_eq!(out.status.code(), Some(2), "{bad}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("full commit sha"),
            "{bad}"
        );
    }
    // an upper-case or padded sha is canonicalised, then compared with the live head
    let upper = format!(" {} ", "A".repeat(40));
    let mut args = base.to_vec();
    args.push(&upper);
    let out = merge_gate_raw(&gate, &args);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    // a canonical sha that is not the live head is a policy refusal
    let other = "b".repeat(40);
    let mut args = base.to_vec();
    args.push(&other);
    assert_eq!(merge_gate_raw(&gate, &args).status.code(), Some(3));
}

#[test]
fn the_default_branch_cannot_opt_out_on_its_own() {
    // No operator policy; the default branch's file says `required: false`. Only the
    // operator may waive the quorum, so the merge gate still refuses (exit 3).
    let gate = Gate::new("t16-base-optout");
    gate.base("version: 1\nreview_quorum:\n  required: false\n");
    let none = gate.dir.join("no-operator.yml");
    gate.live(&"a".repeat(40), 7);
    let (code, v) = gate.merge_gate(&none);
    assert_eq!(code, Some(3), "{v}");
    assert!(v.get("merge_head").is_none_or(|m| m.is_null()), "{v}");
}
