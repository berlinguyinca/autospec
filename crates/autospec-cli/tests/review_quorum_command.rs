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
        script(
            &gate.bin,
            "gh",
            &format!("cat {}", gate.dir.join("live.json").display()),
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
            ])
            .args(extra)
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };
    assert_eq!(run(&["--pr", "7"]).0, Some(0));
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
    std::fs::create_dir_all(&r.state).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let write = |pid: u32, started: u64| {
        std::fs::write(
            r.marker(&head),
            format!(r#"{{"state":"pending","pid":{pid},"started":{started},"token":"t"}}"#),
        )
        .unwrap();
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
