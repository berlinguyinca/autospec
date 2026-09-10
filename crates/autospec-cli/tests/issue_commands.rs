use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

const SAFE_BODY: &str = "## Goal\nAdd a typed issue promotion command.\n";
const RESET_BODY: &str = "## Goal\n\nDelete the local test database and repopulate it from fixtures.\n\nOnly test, local, and fixture data are in scope. Production is out of scope.\n";

struct PromotionFixture {
    root: PathBuf,
    bin: PathBuf,
    state: PathBuf,
    calls: PathBuf,
}

impl PromotionFixture {
    fn new(body: &str, author: &str, labels: &[&str]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "autospec-issue-promote-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed),
        ));
        let _ = fs::remove_dir_all(&root);
        let bin = root.join("bin");
        let state = root.join("issue.json");
        let calls = root.join("gh.calls");
        fs::create_dir_all(&bin).expect("create fixture bin");
        fs::write(&calls, "").expect("create call log");
        fs::write(
            &state,
            serde_json::to_vec(&serde_json::json!({
                "number": 1890,
                "title": "Add a typed issue promotion command",
                "body": body,
                "labels": labels,
                "author": {"login": author},
                "state": "OPEN"
            }))
            .unwrap(),
        )
        .expect("write issue state");
        write_executable(&bin.join("gh"), GH_FIXTURE);
        Self {
            root,
            bin,
            state,
            calls,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_autospec"));
        let inherited = std::env::var_os("PATH").unwrap_or_default();
        let path = std::env::join_paths(
            std::iter::once(self.bin.clone()).chain(std::env::split_paths(&inherited)),
        )
        .unwrap();
        command
            .args([
                "issue",
                "promote",
                "--repo",
                "test/repo",
                "--number",
                "1890",
                "--json",
            ])
            .env("PATH", path)
            .env("AUTOSPEC_PROMOTE_STATE", &self.state)
            .env("AUTOSPEC_PROMOTE_CALLS", &self.calls);
        command
    }

    fn issue(&self) -> serde_json::Value {
        serde_json::from_slice(&fs::read(&self.state).expect("read issue state")).unwrap()
    }

    fn calls(&self) -> String {
        fs::read_to_string(&self.calls).expect("read call log")
    }

    fn policy(&self, body: &str) -> PathBuf {
        let path = self.root.join("autospec.yml");
        fs::write(&path, body).expect("write policy");
        path
    }
}

#[test]
fn issue_promote_labels_rereads_and_then_adds_auto_implement_without_editing_body() {
    let fixture = PromotionFixture::new(SAFE_BODY, "berlinguyinca", &["ctx:32k"]);

    let output = fixture.command().output().expect("promotion command runs");

    assert_success(&output);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["safety"]["decision"], "pass");
    assert_eq!(report["auto-implement"], true);
    assert_eq!(report["eligible"], true);
    assert_eq!(report["changed"], true);
    assert!(report.get("drainable").is_none());
    let issue = fixture.issue();
    assert_eq!(issue["body"].as_str().unwrap(), SAFE_BODY);
    assert!(labels(&issue).contains(&"safety:reviewed"));
    assert!(labels(&issue).contains(&"auto-implement"));
    let calls = fixture.calls();
    assert!(!calls.contains("--method PATCH"));
    assert_before(
        &calls,
        "labels[]=safety:reviewed",
        "labels[]=auto-implement",
    );
}

#[test]
fn issue_promote_preserves_an_edit_at_the_safety_mutation_boundary() {
    let fixture = PromotionFixture::new(SAFE_BODY, "berlinguyinca", &["ctx:32k"]);

    let output = fixture
        .command()
        .env("AUTOSPEC_PROMOTE_RACE", "before-safety-mutation")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("ISSUE_PROMOTION_CONFLICT"));
    let issue = fixture.issue();
    assert!(issue["body"]
        .as_str()
        .unwrap()
        .contains("Concurrent human edit."));
    assert!(!labels(&issue).contains(&"safety:reviewed"));
    assert!(!labels(&issue).contains(&"auto-implement"));
}

#[test]
fn issue_promote_admits_a_groomed_issue_while_removing_its_template_hold() {
    let fixture = PromotionFixture::new(
        SAFE_BODY,
        "berlinguyinca",
        &["ctx:32k", "needs-autospec-template"],
    );

    let output = fixture
        .command()
        .args(["--remove-label", "needs-autospec-template"])
        .output()
        .unwrap();

    assert_success(&output);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["auto-implement"], true);
    assert_eq!(report["eligible"], true);
    let issue = fixture.issue();
    assert!(labels(&issue).contains(&"auto-implement"));
    assert!(!labels(&issue).contains(&"needs-autospec-template"));
}

#[test]
fn issue_promote_is_idempotent_after_authoritative_admission() {
    let fixture = PromotionFixture::new(SAFE_BODY, "berlinguyinca", &["ctx:32k"]);
    assert_success(&fixture.command().output().unwrap());
    fs::write(&fixture.calls, "").unwrap();

    let output = fixture.command().output().unwrap();

    assert_success(&output);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["auto-implement"], true);
    assert_eq!(report["changed"], false);
    let calls = fixture.calls();
    assert_eq!(calls.matches("--method GET").count(), 1);
    assert!(!calls.contains("--method POST"));
    assert!(!calls.contains("--method PATCH"));
    assert!(!calls.contains("--method DELETE"));
}

#[test]
fn issue_promote_finishes_owned_label_cleanup_for_an_existing_admission() {
    let fixture = PromotionFixture::new(SAFE_BODY, "berlinguyinca", &["ctx:32k"]);
    assert_success(&fixture.command().output().unwrap());
    let mut issue = fixture.issue();
    issue["labels"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!("needs-autospec-template"));
    fs::write(&fixture.state, serde_json::to_vec(&issue).unwrap()).unwrap();
    fs::write(&fixture.calls, "").unwrap();

    let output = fixture
        .command()
        .args(["--remove-label", "needs-autospec-template"])
        .output()
        .unwrap();

    assert_success(&output);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["auto-implement"], true);
    assert_eq!(report["eligible"], true);
    assert_eq!(report["changed"], true);
    assert!(!labels(&fixture.issue()).contains(&"needs-autospec-template"));
}

#[test]
fn issue_promote_restores_existing_admission_cleanup_when_verification_read_fails() {
    let fixture = PromotionFixture::new(
        SAFE_BODY,
        "berlinguyinca",
        &[
            "ctx:32k",
            "safety:reviewed",
            "auto-implement",
            "needs-autospec-template",
        ],
    );

    let output = fixture
        .command()
        .args(["--remove-label", "needs-autospec-template"])
        .env("AUTOSPEC_PROMOTE_FAILURE", "cleanup-get")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("gh issue read failed"));
    assert!(labels(&fixture.issue()).contains(&"needs-autospec-template"));
    let calls = fixture.calls();
    assert_before(
        &calls,
        "labels/needs-autospec-template",
        "labels[]=needs-autospec-template",
    );
    assert!(
        calls.rfind("--method GET").unwrap()
            > calls.rfind("labels[]=needs-autospec-template").unwrap()
    );
}

#[test]
fn issue_promote_reports_existing_admission_cleanup_restore_failure_after_drift() {
    let fixture = PromotionFixture::new(
        SAFE_BODY,
        "berlinguyinca",
        &[
            "ctx:32k",
            "safety:reviewed",
            "auto-implement",
            "needs-autospec-template",
        ],
    );

    let output = fixture
        .command()
        .args(["--remove-label", "needs-autospec-template"])
        .env("AUTOSPEC_PROMOTE_RACE", "cleanup-drift")
        .env("AUTOSPEC_PROMOTE_FAILURE", "cleanup-restore")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("ISSUE_PROMOTION_ROLLBACK_FAILED"));
    assert!(!labels(&fixture.issue()).contains(&"needs-autospec-template"));
}

#[test]
fn issue_promote_fails_closed_when_issue_changes_before_safety_write() {
    let fixture = PromotionFixture::new(SAFE_BODY, "berlinguyinca", &["ctx:32k"]);

    let output = fixture
        .command()
        .env("AUTOSPEC_PROMOTE_RACE", "before-safety")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("ISSUE_PROMOTION_CONFLICT"));
    let calls = fixture.calls();
    assert!(!calls.contains("--method PATCH"));
    assert!(!calls.contains("--method POST"));
}

#[test]
fn issue_promote_rejects_malformed_canonical_safety_state_without_mutation() {
    let body = "## Goal\nAdd a typed issue promotion command.\n\n## Safety review\n<!-- autospec:safety-review:start -->\nSAFETY_PASS\n<!-- autospec:safety-review:end -->\n<!-- autospec:safety-review:start -->\nSAFETY_PASS\n<!-- autospec:safety-review:end -->\n";
    let fixture = PromotionFixture::new(body, "berlinguyinca", &["ctx:32k"]);

    let output = fixture.command().output().unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("ISSUE_PROMOTION_CONFLICT"));
    let calls = fixture.calls();
    assert!(!calls.contains("--method PATCH"));
    assert!(!calls.contains("--method POST"));
}

#[test]
fn issue_promote_rolls_back_auto_implement_when_post_write_state_drifts() {
    let fixture = PromotionFixture::new(SAFE_BODY, "berlinguyinca", &["ctx:32k"]);
    let output = fixture
        .command()
        .env("AUTOSPEC_PROMOTE_RACE", "after-auto")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("ISSUE_PROMOTION_CONFLICT"));
    assert!(!labels(&fixture.issue()).contains(&"auto-implement"));
    let calls = fixture.calls();
    assert_before(&calls, "labels[]=auto-implement", "labels/auto-implement");
}

#[test]
fn issue_promote_rolls_back_auto_implement_when_final_read_fails() {
    let fixture = PromotionFixture::new(SAFE_BODY, "berlinguyinca", &["ctx:32k"]);

    let output = fixture
        .command()
        .env("AUTOSPEC_PROMOTE_FAILURE", "final-get")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("gh issue read failed"));
    assert!(!labels(&fixture.issue()).contains(&"auto-implement"));
    let calls = fixture.calls();
    assert_before(&calls, "labels[]=auto-implement", "labels/auto-implement");
    assert!(calls.rfind("--method GET").unwrap() > calls.rfind("labels/auto-implement").unwrap());
}

#[test]
fn issue_promote_rolls_back_safety_review_when_its_verification_read_fails() {
    let fixture = PromotionFixture::new(SAFE_BODY, "berlinguyinca", &["ctx:32k"]);

    let output = fixture
        .command()
        .env("AUTOSPEC_PROMOTE_FAILURE", "safety-get")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("gh issue read failed"));
    let issue = fixture.issue();
    assert!(!labels(&issue).contains(&"safety:reviewed"));
    assert!(!labels(&issue).contains(&"auto-implement"));
}

#[test]
fn issue_promote_surfaces_verified_rollback_failure_when_auto_label_cannot_be_removed() {
    let fixture = PromotionFixture::new(SAFE_BODY, "berlinguyinca", &["ctx:32k"]);

    let output = fixture
        .command()
        .env("AUTOSPEC_PROMOTE_RACE", "after-auto")
        .env("AUTOSPEC_PROMOTE_FAILURE", "rollback-delete")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("ISSUE_PROMOTION_ROLLBACK_FAILED"));
    assert!(labels(&fixture.issue()).contains(&"auto-implement"));
    let calls = fixture.calls();
    assert_before(&calls, "labels[]=auto-implement", "labels/auto-implement");
    assert!(calls.rfind("--method GET").unwrap() > calls.rfind("labels/auto-implement").unwrap());
}

#[test]
fn issue_promote_fails_closed_for_unsupported_repository_policy() {
    let fixture = PromotionFixture::new(SAFE_BODY, "berlinguyinca", &["ctx:32k"]);
    let policy = fixture.policy(
        "safety:\n  issue_intent_gate:\n    block_patterns:\n      - id: company-secret-policy\n        patterns:\n          - \"(?i)company secret\"\n",
    );

    let output = fixture
        .command()
        .env("AUTOSPEC_CONFIG_FILE", policy)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported custom regex"));
    assert!(!fixture.calls().contains("--method POST"));
    assert!(!labels(&fixture.issue()).contains(&"auto-implement"));
}

#[test]
fn issue_promote_fails_closed_when_explicit_policy_is_missing() {
    let fixture = PromotionFixture::new(SAFE_BODY, "berlinguyinca", &["ctx:32k"]);

    let output = fixture
        .command()
        .env("AUTOSPEC_CONFIG_FILE", fixture.root.join("missing.yml"))
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("could not read issue safety policy"));
    assert!(fixture.calls().is_empty());
}

#[test]
fn issue_promote_fails_closed_when_explicit_policy_is_unreadable() {
    let fixture = PromotionFixture::new(SAFE_BODY, "berlinguyinca", &["ctx:32k"]);

    let output = fixture
        .command()
        .env("AUTOSPEC_CONFIG_FILE", &fixture.root)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("could not read issue safety policy"));
    assert!(fixture.calls().is_empty());
}

#[test]
fn issue_promote_fails_closed_when_explicit_policy_is_malformed() {
    let fixture = PromotionFixture::new(SAFE_BODY, "berlinguyinca", &["ctx:32k"]);
    let policy = fixture.policy("safety: [\n");

    let output = fixture
        .command()
        .env("AUTOSPEC_CONFIG_FILE", policy)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("could not parse issue safety policy"));
    assert!(fixture.calls().is_empty());
}

#[test]
fn issue_promote_uses_configured_trusted_actor_policy() {
    let fixture = PromotionFixture::new(RESET_BODY, "release-operator", &["ctx:32k"]);
    let policy = fixture.policy(
        "safety:\n  issue_intent_gate:\n    trusted_actors:\n      - login: release-operator\n",
    );

    let output = fixture
        .command()
        .env("AUTOSPEC_CONFIG_FILE", policy)
        .output()
        .unwrap();

    assert_success(&output);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["safety"]["decision"], "pass");
    assert_eq!(report["auto-implement"], true);
}

#[test]
fn issue_promote_reports_observed_labels_when_queue_policy_withholds_admission() {
    let fixture = PromotionFixture::new(SAFE_BODY, "berlinguyinca", &["ctx:32k", "needs-classify"]);

    let output = fixture.command().output().unwrap();

    assert_success(&output);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["safety"]["decision"], "pass");
    assert_eq!(report["eligible"], false);
    assert_eq!(report["auto-implement"], false);
    assert_eq!(
        report["final_labels"],
        serde_json::json!(["ctx:32k", "needs-classify", "safety:reviewed"])
    );
    let issue = fixture.issue();
    assert!(!labels(&issue).contains(&"auto-implement"));
    assert_eq!(report["final_labels"], issue["labels"]);
}

fn labels(issue: &serde_json::Value) -> Vec<&str> {
    issue["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|label| label.as_str().unwrap())
        .collect()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
}

fn assert_before(haystack: &str, first: &str, second: &str) {
    let first = haystack
        .find(first)
        .unwrap_or_else(|| panic!("missing {first}: {haystack}"));
    let second = haystack
        .rfind(second)
        .unwrap_or_else(|| panic!("missing {second}: {haystack}"));
    assert!(
        first < second,
        "expected {first} before {second}: {haystack}"
    );
}

fn write_executable(path: &Path, contents: &str) {
    autospec_core::test_support::write_executable(path, contents);
}

const GH_FIXTURE: &str = r###"#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >> "$AUTOSPEC_PROMOTE_CALLS"
state="$AUTOSPEC_PROMOTE_STATE"
method=GET
endpoint=''
field=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    --method) method="$2"; shift 2 ;;
    -f) field="$2"; shift 2 ;;
    repos/*) endpoint="$1"; shift ;;
    *) shift ;;
  esac
done
case "$method:$endpoint" in
  GET:repos/test/repo/issues/1890)
    if [ "${AUTOSPEC_PROMOTE_RACE:-}" = before-safety ] && [ "$(grep -c -- '--method GET' "$AUTOSPEC_PROMOTE_CALLS")" -eq 2 ]; then
      jq '.title += " (changed)"' "$state" > "$state.tmp"
      mv "$state.tmp" "$state"
    fi
    if [ "${AUTOSPEC_PROMOTE_FAILURE:-}" = final-get ] \
      && jq -e '.labels | index("auto-implement")' "$state" >/dev/null \
      && [ ! -e "$state.final-get-failed" ]; then
      : > "$state.final-get-failed"
      printf 'simulated final read failure\n' >&2
      exit 42
    fi
    if [ "${AUTOSPEC_PROMOTE_FAILURE:-}" = safety-get ] \
      && jq -e '.labels | index("safety:reviewed")' "$state" >/dev/null \
      && ! jq -e '.labels | index("auto-implement")' "$state" >/dev/null \
      && [ ! -e "$state.safety-get-failed" ]; then
      : > "$state.safety-get-failed"
      printf 'simulated safety verification read failure\n' >&2
      exit 44
    fi
    if [ "${AUTOSPEC_PROMOTE_FAILURE:-}" = cleanup-get ] \
      && jq -e '.labels | index("auto-implement")' "$state" >/dev/null \
      && ! jq -e '.labels | index("needs-autospec-template")' "$state" >/dev/null \
      && [ ! -e "$state.cleanup-get-failed" ]; then
      : > "$state.cleanup-get-failed"
      printf 'simulated cleanup verification read failure\n' >&2
      exit 45
    fi
    cat "$state"
    ;;
  PATCH:repos/test/repo/issues/1890)
    if [ "${AUTOSPEC_PROMOTE_RACE:-}" = before-safety-mutation ] && [ ! -e "$state.safety-raced" ]; then
      : > "$state.safety-raced"
      jq '.body += "\nConcurrent human edit."' "$state" > "$state.tmp"
      mv "$state.tmp" "$state"
    fi
    body="${field#body=}"
    jq --arg body "$body" '.body=$body' "$state" > "$state.tmp"
    mv "$state.tmp" "$state"
    ;;
  POST:repos/test/repo/issues/1890/labels)
    label="${field#labels[]=}"
    if [ "${AUTOSPEC_PROMOTE_FAILURE:-}" = cleanup-restore ] \
      && [ "$label" = needs-autospec-template ]; then
      printf 'simulated cleanup restore failure\n' >&2
      exit 46
    fi
    if [ "${AUTOSPEC_PROMOTE_RACE:-}" = before-safety-mutation ] \
      && [ "$label" = safety:reviewed ] \
      && [ ! -e "$state.safety-raced" ]; then
      : > "$state.safety-raced"
      jq '.body += "\nConcurrent human edit."' "$state" > "$state.tmp"
      mv "$state.tmp" "$state"
    fi
    jq --arg label "$label" '.labels = ((.labels + [$label]) | unique)' "$state" > "$state.tmp"
    mv "$state.tmp" "$state"
    if [ "${AUTOSPEC_PROMOTE_RACE:-}" = after-auto ] && [ "$label" = auto-implement ]; then
      jq '.body += "\nDelete production data."' "$state" > "$state.tmp"
      mv "$state.tmp" "$state"
    fi
    ;;
  DELETE:repos/test/repo/issues/1890/labels/*)
    label="${endpoint##*/}"
    if [ "${AUTOSPEC_PROMOTE_FAILURE:-}" = rollback-delete ] && [ "$label" = auto-implement ]; then
      printf 'simulated rollback delete failure\n' >&2
      exit 43
    fi
    jq --arg label "$label" '.labels = [.labels[] | select(. != $label)]' "$state" > "$state.tmp"
    mv "$state.tmp" "$state"
    if [ "${AUTOSPEC_PROMOTE_RACE:-}" = cleanup-drift ] \
      && [ "$label" = needs-autospec-template ]; then
      jq '.title += " (changed during cleanup)"' "$state" > "$state.tmp"
      mv "$state.tmp" "$state"
    fi
    ;;
  *)
    printf 'unexpected gh call: %s %s\n' "$method" "$endpoint" >&2
    exit 41
    ;;
esac
"###;

// ---------------------------------------------------------------------------
// autospec handoff probe (issue #3440)
// ---------------------------------------------------------------------------

const HANDOFF_AUTOSPEC_FIXTURE: &str = r###"#!/bin/sh
printf '%s\n' "$*" >> "${HANDOFF_PROBE_LOG:?}"
if [ "$1" != "handoff" ] || [ "$2" != "capabilities" ]; then
  exit 2
fi
case "${HANDOFF_FAKE_MODE:-healthy}" in
  healthy)
    printf '{"schema":"autospec.handoff-capabilities.v1","routes":["run","start","split_then_run","recover","none"]}\n'
    exit 0
    ;;
  exit2)
    printf 'unknown autospec command: handoff\n'
    exit 2
    ;;
  wrong-schema)
    printf '{"schema":"autospec.handoff-capabilities.v9","routes":["run"]}\n'
    exit 0
    ;;
  transient)
    printf 'boom\n' >&2
    exit 3
    ;;
  partial-run)
    printf '{"schema":"autospec.handoff-capabilities.v1","routes":["run"]}\n'
    exit 0
    ;;
  *)
    exit 127
    ;;
esac
"###;

const HANDOFF_GH_FIXTURE: &str = r###"#!/bin/sh
printf 'gh %s\n' "$*" >> "${HANDOFF_GH_LOG:?}"
exit 0
"###;

struct HandoffFixture {
    root: PathBuf,
    bin: PathBuf,
    /// A PATH entry holding only the `gh` spy (no `autospec`), used to
    /// simulate a host where the autospec CLI is missing entirely.
    bare: PathBuf,
    repo_dir: PathBuf,
    probe_log: PathBuf,
    gh_log: PathBuf,
}

impl HandoffFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "autospec-handoff-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed),
        ));
        let _ = fs::remove_dir_all(&root);
        let bin = root.join("bin");
        let bare = root.join("bin-bare");
        let repo_dir = root.join("repo");
        let probe_log = root.join("probe.log");
        let gh_log = root.join("gh.calls");
        fs::create_dir_all(&bin).expect("create fixture bin");
        fs::create_dir_all(&bare).expect("create bare bin");
        fs::create_dir_all(&repo_dir).expect("create repo dir");
        fs::write(&probe_log, "").expect("create probe log");
        fs::write(&gh_log, "").expect("create gh log");
        write_executable(&bin.join("autospec"), HANDOFF_AUTOSPEC_FIXTURE);
        write_executable(&bin.join("gh"), HANDOFF_GH_FIXTURE);
        write_executable(&bare.join("gh"), HANDOFF_GH_FIXTURE);
        Self {
            root,
            bin,
            bare,
            repo_dir,
            probe_log,
            gh_log,
        }
    }

    /// Build a command that runs the real autospec binary with the fixture
    /// bin prepended to PATH (the fake `autospec` shadows any installed one)
    /// and optional extra PATH entries. `path_only_fixture=true` restricts
    /// PATH to the fixture bin so the probe cannot resolve any `autospec`.
    fn command(&self, mode: &str, path_only_fixture: bool, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_autospec"));
        let inherited = std::env::var_os("PATH").unwrap_or_default();
        let first = if path_only_fixture {
            self.bare.clone()
        } else {
            self.bin.clone()
        };
        let mut entries = vec![first];
        if !path_only_fixture {
            entries.extend(std::env::split_paths(&inherited));
        }
        let path = std::env::join_paths(entries).expect("join PATH");
        command
            .args(["handoff", "probe"])
            .args(args)
            .env("PATH", path)
            .env("HANDOFF_PROBE_LOG", &self.probe_log)
            .env("HANDOFF_GH_LOG", &self.gh_log)
            .env("HANDOFF_FAKE_MODE", mode);
        command
    }

    fn run_json(
        &self,
        mode: &str,
        path_only_fixture: bool,
        args: &[&str],
    ) -> (Output, serde_json::Value) {
        let output = self
            .command(mode, path_only_fixture, args)
            .output()
            .expect("run autospec handoff probe");
        let body: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&output.stdout))
                .expect("parse handoff JSON");
        (output, body)
    }

    fn write_run_state(&self, value: &str) {
        let dir = self.repo_dir.join(".autospec/state/handoff");
        fs::create_dir_all(&dir).expect("create run state dir");
        fs::write(dir.join("runs.json"), value).expect("write runs.json");
    }

    fn tree_listing(&self, dir: &Path) -> Vec<String> {
        let mut out = vec![];
        fn walk(dir: &Path, out: &mut Vec<String>) {
            if dir.is_dir() {
                let mut entries: Vec<_> = fs::read_dir(dir)
                    .expect("read dir")
                    .map(|e| e.unwrap().path())
                    .collect();
                entries.sort();
                for entry in entries {
                    out.push(entry.to_string_lossy().into_owned());
                    walk(&entry, out);
                }
            }
        }
        walk(dir, &mut out);
        out
    }

    fn probe_calls(&self) -> u64 {
        fs::read_to_string(&self.probe_log)
            .expect("read probe log")
            .lines()
            .count() as u64
    }

    fn gh_calls(&self) -> String {
        fs::read_to_string(&self.gh_log).expect("read gh log")
    }
}

const HANDOFF_BASE_ARGS: &[&str] = &["--repo", "test/repo", "--intent", "ship the feature"];

#[test]
fn handoff_capabilities_lists_all_five_routes() {
    let output = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args(["handoff", "capabilities"])
        .output()
        .expect("run autospec handoff capabilities");
    assert!(
        output.status.success(),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let body: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))
        .expect("parse capabilities JSON");
    assert_eq!(body["schema"], "autospec.handoff-capabilities.v1");
    let routes: Vec<String> = body["routes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    for expected in ["run", "start", "split_then_run", "recover", "none"] {
        assert!(
            routes.contains(&expected.to_string()),
            "missing route {expected}"
        );
    }
}

#[test]
fn handoff_probe_routes_start_without_artifact_and_is_deterministic() {
    let fixture = HandoffFixture::new();
    let (output1, body1) = fixture.run_json("healthy", false, HANDOFF_BASE_ARGS);
    assert!(
        output1.status.success(),
        "stdout: {}",
        String::from_utf8_lossy(&output1.stdout)
    );
    assert_eq!(body1["schema"], "autospec.implementation-handoff.v1");
    assert_eq!(body1["availability"], "available");
    assert_eq!(body1["unavailable_reason"], serde_json::Value::Null);
    assert_eq!(body1["route"], "start");
    assert_eq!(body1["artifact"], serde_json::Value::Null);
    assert_eq!(body1["run"]["entry_point"], "autospec");
    assert_eq!(body1["run"]["follow_up"], serde_json::Value::Null);
    assert_eq!(body1["run"]["status"], "proposed");
    assert!(body1["run"]["run_id"].as_str().unwrap().starts_with("run-"));
    assert_eq!(body1["project"]["key"], "product.test__repo");
    assert_eq!(body1["project"]["state"], "planned");
    assert!(body1["stream"]["stream_id"]
        .as_str()
        .unwrap()
        .starts_with("stream-"));
    assert!(body1["cancellation"]["token"]
        .as_str()
        .unwrap()
        .starts_with("cancel-"));
    assert_eq!(fixture.probe_calls(), 1);
    assert_eq!(fixture.gh_calls(), "");

    // A second identical request derives identical identities.
    let (_output2, body2) = fixture.run_json("healthy", false, HANDOFF_BASE_ARGS);
    assert_eq!(
        body1, body2,
        "identical requests must derive identical identities"
    );
}

#[test]
fn handoff_probe_routes_run_for_issue_artifact() {
    let fixture = HandoffFixture::new();
    let (output, body) = fixture.run_json(
        "healthy",
        false,
        &[
            "--repo",
            "test/repo",
            "--intent",
            "fix the bug",
            "--artifact",
            "issue:42",
        ],
    );
    assert!(output.status.success());
    assert_eq!(body["route"], "run");
    assert_eq!(body["run"]["entry_point"], "autospec-run");
    assert_eq!(body["artifact"]["kind"], "issue");
    assert_eq!(body["artifact"]["ref"], "issue:42");
    assert_eq!(body["run"]["status"], "proposed");
}

#[test]
fn handoff_probe_routes_split_then_run_for_spec_artifact() {
    let fixture = HandoffFixture::new();
    let (output, body) = fixture.run_json(
        "healthy",
        false,
        &[
            "--repo",
            "test/repo",
            "--intent",
            "implement the spec",
            "--artifact",
            "spec:docs/specs/2026-08-31-design.md",
        ],
    );
    assert!(output.status.success());
    assert_eq!(body["route"], "split_then_run");
    assert_eq!(body["run"]["entry_point"], "autospec-split");
    assert_eq!(body["run"]["follow_up"], "autospec-run");
    assert_eq!(body["artifact"]["kind"], "spec");
}

#[test]
fn handoff_probe_routes_recover_for_interrupted_run() {
    let fixture = HandoffFixture::new();
    fixture.write_run_state(
        "{\"schema\":\"autospec.handoff-run-state.v1\",\"runs\":[\n\
         {\"run_id\":\"run-finished\",\"correlation_id\":\"other\",\"status\":\"completed\"},\n\
         {\"run_id\":\"run-existing-1\",\"correlation_id\":\"corr-x\",\"status\":\"interrupted\"}\n\
         ]}",
    );
    let repo_dir = fixture.repo_dir.to_string_lossy().into_owned();
    let (output, body) = fixture.run_json(
        "healthy",
        false,
        &[
            "--repo",
            "test/repo",
            "--intent",
            "ship the feature",
            "--repo-dir",
            &repo_dir,
        ],
    );
    assert!(output.status.success());
    assert_eq!(body["route"], "recover");
    assert_eq!(body["availability"], "available");
    assert_eq!(
        body["run"]["run_id"], "run-existing-1",
        "recovery keeps the interrupted run's identity"
    );
    assert_eq!(body["run"]["entry_point"], "autospec-resume");
    assert_eq!(body["run"]["status"], "recovered");
    assert_eq!(body["project"]["state"], "recovered");
}

#[test]
fn handoff_probe_routes_none_for_read_only_intents() {
    let fixture = HandoffFixture::new();
    for kind in ["explain", "plan"] {
        let (output, body) = fixture.run_json(
            "healthy",
            false,
            &[
                "--repo",
                "test/repo",
                "--intent",
                "what is this",
                "--intent-kind",
                kind,
            ],
        );
        assert!(output.status.success());
        assert_eq!(body["route"], "none");
        assert_eq!(body["availability"], "available");
        assert_eq!(
            body["run"],
            serde_json::Value::Null,
            "{kind}: read-only dispatches no run"
        );
        assert_eq!(
            body["stream"],
            serde_json::Value::Null,
            "{kind}: read-only dispatches no stream"
        );
        assert_eq!(
            body["cancellation"],
            serde_json::Value::Null,
            "{kind}: read-only dispatches no cancellation"
        );
    }
}

#[test]
fn handoff_probe_read_only_wins_over_interrupted_run() {
    let fixture = HandoffFixture::new();
    fixture.write_run_state(
        "{\"schema\":\"autospec.handoff-run-state.v1\",\"runs\":[\n\
         {\"run_id\":\"run-existing-1\",\"correlation_id\":\"corr-x\",\"status\":\"interrupted\"}\n\
         ]}",
    );
    let repo_dir = fixture.repo_dir.to_string_lossy().into_owned();
    let (output, body) = fixture.run_json(
        "healthy",
        false,
        &[
            "--repo",
            "test/repo",
            "--intent",
            "what is this",
            "--intent-kind",
            "explain",
            "--repo-dir",
            &repo_dir,
        ],
    );
    assert!(output.status.success());
    assert_eq!(
        body["route"], "none",
        "read-only intent outranks an interrupted run"
    );
}

fn assert_unavailable(body: &serde_json::Value, reason: &str) {
    assert_eq!(body["availability"], "unavailable");
    assert_eq!(body["unavailable_reason"], reason);
    assert_eq!(body["route"], serde_json::Value::Null);
    assert_eq!(body["run"], serde_json::Value::Null);
    assert_eq!(body["stream"], serde_json::Value::Null);
    assert_eq!(body["cancellation"], serde_json::Value::Null);
    assert!(
        body["guidance"]
            .as_str()
            .is_some_and(|g| g.contains("No mutating fallback")),
        "guidance must state there is no mutating fallback: {}",
        body["guidance"]
    );
}

#[test]
fn handoff_probe_unavailable_when_cli_missing() {
    let fixture = HandoffFixture::new();
    let (output, body) = fixture.run_json("healthy", true, HANDOFF_BASE_ARGS);
    assert!(
        output.status.success(),
        "well-formed request still exits 0 when blocked"
    );
    assert_unavailable(&body, "cli_missing");
    assert_eq!(fixture.probe_calls(), 0, "no probe call was possible");
    assert_eq!(fixture.gh_calls(), "");
}

#[test]
fn handoff_probe_unavailable_on_partial_install() {
    let fixture = HandoffFixture::new();
    let (output, body) = fixture.run_json("exit2", false, HANDOFF_BASE_ARGS);
    assert!(output.status.success());
    assert_unavailable(&body, "workflow_surface_missing");
    assert_eq!(fixture.probe_calls(), 1);
    assert_eq!(fixture.gh_calls(), "");
}

#[test]
fn handoff_probe_unavailable_on_incompatible_surface() {
    let fixture = HandoffFixture::new();
    let (output, body) = fixture.run_json("wrong-schema", false, HANDOFF_BASE_ARGS);
    assert!(output.status.success());
    assert_unavailable(&body, "workflow_surface_incompatible");
    assert_eq!(fixture.gh_calls(), "");
}

#[test]
fn handoff_probe_unavailable_when_required_route_missing() {
    let fixture = HandoffFixture::new();
    // Surface offers only `run`: a no-artifact request needs `start`.
    let (output, body) = fixture.run_json("partial-run", false, HANDOFF_BASE_ARGS);
    assert!(output.status.success());
    assert_unavailable(&body, "workflow_surface_missing");
    // But an issue artifact needs `run`, which the surface offers.
    let (output2, body2) = fixture.run_json(
        "partial-run",
        false,
        &[
            "--repo",
            "test/repo",
            "--intent",
            "fix",
            "--artifact",
            "issue:7",
        ],
    );
    assert!(output2.status.success());
    assert_eq!(body2["availability"], "available");
    assert_eq!(body2["route"], "run");
}

#[test]
fn handoff_probe_fails_closed_to_recovery_on_transient_probe() {
    let fixture = HandoffFixture::new();
    let (output, body) = fixture.run_json("transient", false, HANDOFF_BASE_ARGS);
    assert!(output.status.success());
    assert_eq!(body["availability"], "unknown");
    assert_eq!(body["unavailable_reason"], "probe_transient");
    assert_eq!(body["route"], "recover");
    assert_eq!(body["run"]["entry_point"], "autospec-resume");
    assert_eq!(
        fixture.gh_calls(),
        "",
        "recovery must not dispatch directly"
    );
}

#[test]
fn handoff_probe_fails_closed_on_ambiguous_run_state() {
    let fixture = HandoffFixture::new();
    fixture.write_run_state("this is not json");
    let repo_dir = fixture.repo_dir.to_string_lossy().into_owned();
    let (output, body) = fixture.run_json(
        "healthy",
        false,
        &[
            "--repo",
            "test/repo",
            "--intent",
            "ship the feature",
            "--repo-dir",
            &repo_dir,
        ],
    );
    assert!(output.status.success());
    assert_eq!(body["availability"], "unknown");
    assert_eq!(body["unavailable_reason"], "run_state_ambiguous");
    assert_eq!(body["route"], "recover");
    assert_eq!(fixture.gh_calls(), "");
}

#[test]
fn handoff_probe_unavailable_never_mutates() {
    for mode in ["exit2", "wrong-schema", "partial-run"] {
        let fixture = HandoffFixture::new();
        let before = fixture.tree_listing(&fixture.root);
        let (output, body) = fixture.run_json(mode, false, HANDOFF_BASE_ARGS);
        assert!(output.status.success());
        assert_eq!(body["availability"], "unavailable");
        let after = fixture.tree_listing(&fixture.root);
        assert_eq!(
            before, after,
            "mode {mode}: unavailable handoff must not create or change any file"
        );
        assert_eq!(
            fixture.gh_calls(),
            "",
            "mode {mode}: zero gh calls permitted"
        );
    }
    // cli_missing mode too (PATH restricted to the fixture bin).
    let fixture = HandoffFixture::new();
    let before = fixture.tree_listing(&fixture.root);
    let (output, body) = fixture.run_json("healthy", true, HANDOFF_BASE_ARGS);
    assert!(output.status.success());
    assert_eq!(body["availability"], "unavailable");
    assert_eq!(before, fixture.tree_listing(&fixture.root));
    assert_eq!(fixture.gh_calls(), "");
}

#[test]
fn handoff_probe_usage_errors_exit_2() {
    let fixture = HandoffFixture::new();
    let missing_repo = fixture
        .command("healthy", false, &["--intent", "x"])
        .output()
        .expect("run");
    assert_eq!(
        missing_repo.status.code(),
        Some(2),
        "missing --repo must exit 2"
    );
    assert!(
        String::from_utf8_lossy(&missing_repo.stderr).contains("missing required --repo"),
        "stderr: {}",
        String::from_utf8_lossy(&missing_repo.stderr)
    );

    let bad_artifact = fixture
        .command("healthy", false, &HANDOFF_BASE_ARGS)
        .arg("--artifact")
        .arg("issue:")
        .output()
        .expect("run");
    assert_eq!(
        bad_artifact.status.code(),
        Some(2),
        "empty issue number must exit 2"
    );

    let bad_repo = fixture
        .command("healthy", false, &["--repo", "a/b/c", "--intent", "x"])
        .output()
        .expect("run");
    assert_eq!(
        bad_repo.status.code(),
        Some(2),
        "malformed --repo must exit 2"
    );
}

#[test]
fn handoff_probe_help_exits_0() {
    let fixture = HandoffFixture::new();
    let output = fixture
        .command("healthy", false, &["--help"])
        .output()
        .expect("run");
    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("autospec handoff probe --repo OWNER/NAME"),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
