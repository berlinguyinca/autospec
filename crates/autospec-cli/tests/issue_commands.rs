use std::fs;
use std::os::unix::fs::PermissionsExt;
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
    fs::write(path, contents).expect("write executable");
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
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
