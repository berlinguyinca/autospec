use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

const DEAD_END_BODY: &str =
    "## Goal\nShip the thing.\n\n## Acceptance criteria\n- [x] kept\n- [x] documented\n";

const PROPOSAL_JSON: &str = r#"{
  "shape": "ambiguous",
  "reading": "The smoke gate means the CLI prints one JSON line on success.",
  "criteria": [
    {
      "requirement": "`cargo build --workspace && cargo fmt --check && echo SMOKE_OK` prints SMOKE_OK",
      "check_command": "cargo build --workspace && cargo fmt --check && echo SMOKE_OK",
      "current_exit_status": 1
    }
  ],
  "question": "Should the smoke gate run fmt, or only build?",
  "checked": ["read `docs/cli-reference.md` - no smoke gate is defined"]
}"#;

/// Fake `gh`: keeps issue state and comments in temp files so a test can drive
/// propose/check end to end without network access.
const GH_FIXTURE: &str = r###"#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >> "$AUTOSPEC_REPAIR_CALLS"
state="$AUTOSPEC_REPAIR_ISSUE"
comments="$AUTOSPEC_REPAIR_COMMENTS"
method=GET
endpoint=''
field=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    --method) method="$2"; shift 2 ;;
    -f) field="$2"; shift 2 ;;
    --jq) shift ;;
    repos/*) endpoint="$1"; shift ;;
    *) shift ;;
  esac
done
issue_path="repos/test/repo/issues/3541"
case "$endpoint" in
  "$issue_path")
    [ "$method" = GET ]
    cat "$state"
    ;;
  "$issue_path/comments")
    case "$method" in
      GET)
        jq -s '.' "$comments"
        ;;
      POST)
        body="${field#body=}"
        jq -cn --arg body "$body" '{author: "autospec-agent", body: $body}' >> "$comments"
        ;;
    esac
    ;;
  "$issue_path/labels")
    [ "$method" = POST ]
    label="${field#labels[]=}"
    # `label` is a jq keyword, so the jq variable must not be named label.
    if jq -e --arg name "$label" '.labels | index($name)' "$state" > /dev/null; then
      cat "$state" > "$state.tmp"
    else
      jq --arg name "$label" '.labels = (.labels + [$name])' "$state" > "$state.tmp"
    fi
    mv "$state.tmp" "$state"
    ;;
  "$issue_path/labels/"*)
    [ "$method" = DELETE ]
    label="${endpoint#"$issue_path/labels/"}"
    jq --arg name "$label" '.labels = [.labels[] | select(. != $name)]' "$state" > "$state.tmp"
    mv "$state.tmp" "$state"
    ;;
  *)
    printf 'unexpected endpoint: %s\n' "$endpoint" >&2
    exit 64
    ;;
esac
"###;

struct RepairFixture {
    root: PathBuf,
    bin: PathBuf,
    issue_state: PathBuf,
    comments: PathBuf,
    calls: PathBuf,
    ledger: PathBuf,
    pr_body: PathBuf,
}

impl RepairFixture {
    fn new(body: &str, labels: &[&str]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "autospec-spec-repair-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed),
        ));
        let _ = fs::remove_dir_all(&root);
        let bin = root.join("bin");
        let issue_state = root.join("issue.json");
        let comments = root.join("comments.jsonl");
        let calls = root.join("gh.calls");
        let ledger = root.join("ledger").join("spec-repair-ledger.jsonl");
        let pr_body = root.join("pr-body.md");
        fs::create_dir_all(&bin).expect("create fixture bin");
        fs::write(&calls, "").expect("create call log");
        fs::write(&comments, "").expect("create empty comment log");
        fs::write(&pr_body, "# PR body\n\nExisting description.\n").expect("create PR body");
        fs::write(
            &issue_state,
            serde_json::to_vec(&serde_json::json!({
                "number": 3541,
                "title": "Spec repair loop",
                "body": body,
                "labels": labels,
                "author": {"login": "octo-author"},
                "state": "OPEN"
            }))
            .unwrap(),
        )
        .expect("write issue state");
        write_executable(&bin.join("gh"), GH_FIXTURE);
        Self {
            root,
            bin,
            issue_state,
            comments,
            calls,
            ledger,
            pr_body,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_autospec"));
        let inherited = std::env::var_os("PATH").unwrap_or_default();
        let path = std::env::join_paths(
            std::iter::once(self.bin.clone()).chain(std::env::split_paths(&inherited)),
        )
        .unwrap();
        command
            .args(["spec-repair"])
            .args(args)
            .env("PATH", path)
            .env("AUTOSPEC_REPAIR_ISSUE", &self.issue_state)
            .env("AUTOSPEC_REPAIR_COMMENTS", &self.comments)
            .env("AUTOSPEC_REPAIR_CALLS", &self.calls);
        command
    }

    fn labels(&self) -> Vec<String> {
        let issue: serde_json::Value =
            serde_json::from_slice(&fs::read(&self.issue_state).expect("read issue state"))
                .unwrap();
        issue["labels"]
            .as_array()
            .expect("labels array")
            .iter()
            .map(|label| label.as_str().expect("string label").to_string())
            .collect()
    }

    fn comments(&self) -> Vec<serde_json::Value> {
        fs::read_to_string(&self.comments)
            .expect("read comment log")
            .lines()
            .map(|line| serde_json::from_str(line).expect("comment line is JSON"))
            .collect()
    }

    fn write_proposal_file(&self) -> PathBuf {
        let path = self.root.join("proposal.json");
        fs::write(&path, PROPOSAL_JSON).expect("write proposal json");
        path
    }

    fn seed_ledger(&self, entries: usize) {
        if let Some(parent) = self.ledger.parent() {
            fs::create_dir_all(parent).expect("create ledger dir");
        }
        let mut body = String::new();
        for _ in 0..entries {
            body.push_str(
                &serde_json::json!({
                    "recorded_at": 1_700_000_000,
                    "repo": "test/repo",
                    "issue": 3541,
                    "shape": "ambiguous",
                    "origin_template": "issue-template",
                    "origin_command": "autospec decompose",
                    "origin_author": "octo-author"
                })
                .to_string(),
            );
            body.push('\n');
        }
        fs::write(&self.ledger, body).expect("seed ledger");
    }
}

impl Drop for RepairFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write fixture script");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("make fixture executable");
}

#[test]
fn propose_posts_comment_adds_label_and_records_a_ledger_event() {
    let fixture = RepairFixture::new(DEAD_END_BODY, &[]);
    let proposal = fixture.write_proposal_file();
    let output = fixture
        .command(&[
            "propose",
            "--repo",
            "test/repo",
            "--issue",
            "3541",
            "--proposal-file",
            proposal.to_str().unwrap(),
            "--ledger-file",
            fixture.ledger.to_str().unwrap(),
        ])
        .output()
        .expect("run propose");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let report = String::from_utf8_lossy(&output.stdout);
    assert!(
        report.contains("\"outcome\":\"posted\""),
        "report: {report}"
    );
    assert!(report.contains("\"label\":\"needs-spec-clarification\""));
    assert!(report.contains("\"recorded_event\":true"));
    assert_eq!(
        fixture.labels(),
        vec!["needs-spec-clarification".to_string()]
    );

    let comments = fixture.comments();
    assert_eq!(comments.len(), 1, "exactly one proposal comment");
    let body = comments[0]["body"].as_str().unwrap();
    assert!(body.contains("autospec:spec-repair-proposal"));
    assert!(body.contains("a proposal, not a decision"));
    assert!(body.contains("Should the smoke gate run fmt, or only build?"));

    let ledger = fs::read_to_string(&fixture.ledger).expect("ledger written");
    let event: serde_json::Value = serde_json::from_str(ledger.trim()).expect("ledger event");
    assert_eq!(event["repo"], "test/repo");
    assert_eq!(event["issue"], 3541);
    assert_eq!(event["shape"], "ambiguous");
    assert_eq!(
        event["origin_author"], "octo-author",
        "author defaults from the issue"
    );
    assert_eq!(event["origin_template"], "(unknown)");
    assert!(report.contains("\"proposal_count\":1"));
}

#[test]
fn propose_refuses_a_workable_issue_and_proposes_when_judged_unusable() {
    let open_body = "## Goal\nShip it.\n\n## Acceptance criteria\n- [ ] not done\n";
    let fixture = RepairFixture::new(open_body, &[]);
    let proposal = fixture.write_proposal_file();
    let output = fixture
        .command(&[
            "propose",
            "--repo",
            "test/repo",
            "--issue",
            "3541",
            "--proposal-file",
            proposal.to_str().unwrap(),
        ])
        .output()
        .expect("run propose");
    assert_eq!(output.status.code(), Some(2), "diagnostic refusal exits 2");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("not in the mechanical dead-end state")
    );
    assert!(
        fixture.comments().is_empty(),
        "no comment on a refused propose"
    );

    let output = fixture
        .command(&[
            "propose",
            "--repo",
            "test/repo",
            "--issue",
            "3541",
            "--proposal-file",
            proposal.to_str().unwrap(),
            "--judged-unusable",
        ])
        .output()
        .expect("run propose with judged-unusable");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"outcome\":\"posted\""));
}

#[test]
fn second_proposal_is_refused_and_the_third_escalates() {
    let fixture = RepairFixture::new(DEAD_END_BODY, &[]);
    let proposal = fixture.write_proposal_file();
    let base = [
        "propose",
        "--repo",
        "test/repo",
        "--issue",
        "3541",
        "--proposal-file",
        proposal.to_str().unwrap(),
        "--ledger-file",
        fixture.ledger.to_str().unwrap(),
    ];
    let first = fixture.command(&base).output().expect("first propose");
    assert!(first.status.success());
    assert!(String::from_utf8_lossy(&first.stdout).contains("\"outcome\":\"posted\""));

    let second = fixture.command(&base).output().expect("second propose");
    assert!(
        second.status.success(),
        "an open proposal is a classified outcome, not an error"
    );
    assert!(String::from_utf8_lossy(&second.stdout).contains("\"outcome\":\"already-proposed\""));
    assert_eq!(fixture.comments().len(), 1);

    fixture.seed_ledger(2);
    let third = fixture.command(&base).output().expect("third propose");
    assert!(
        third.status.success(),
        "escalation is also a classified outcome"
    );
    let comments = fixture.comments();
    assert_eq!(comments.len(), 2, "one proposal plus one escalation notice");
    assert!(comments[1]["body"]
        .as_str()
        .unwrap()
        .contains("autospec:spec-repair-escalation"));
    assert!(String::from_utf8_lossy(&third.stdout).contains("\"outcome\":\"escalated-to-human\""));
}

#[test]
fn check_approval_removes_the_label_and_allows_redispatch() {
    let fixture = RepairFixture::new(DEAD_END_BODY, &["needs-spec-clarification"]);
    // A proposal, a bystander remark, then the maintainer approval.
    fs::write(
        &fixture.comments,
        concat!(
            "{\"author\": \"autospec-agent\", \"body\": \"<!-- autospec:spec-repair-proposal -->\\n### 1. The defect\\n\"}\n",
            "{\"author\": \"octo-bystander\", \"body\": \"watching this one\"}\n",
            "{\"author\": \"octo-maintainer\", \"body\": \"spec-repair: approved\\n\"}\n"
        ),
    )
    .expect("seed comments");
    let output = fixture
        .command(&["check", "--repo", "test/repo", "--issue", "3541"])
        .output()
        .expect("run check");
    assert!(output.status.success(), "every classified outcome exits 0");
    let report = String::from_utf8_lossy(&output.stdout);
    assert!(
        report.contains("\"outcome\":\"approved\""),
        "report: {report}"
    );
    assert!(report.contains("\"label_removed\":true"));
    assert!(report.contains("\"redispatch\":\"allowed\""));
    assert!(
        fixture.labels().is_empty(),
        "approval removes needs-spec-clarification"
    );
}

#[test]
fn check_rejection_keeps_the_label_and_blocks_redispatch() {
    let fixture = RepairFixture::new(DEAD_END_BODY, &["needs-spec-clarification"]);
    fs::write(
        &fixture.comments,
        concat!(
            "{\"author\": \"autospec-agent\", \"body\": \"<!-- autospec:spec-repair-proposal -->\\n\"}\n",
            "{\"author\": \"octo-maintainer\", \"body\": \"spec-repair: rejected: the criteria test the wrong command\\n\"}\n"
        ),
    )
    .expect("seed comments");
    let output = fixture
        .command(&["check", "--repo", "test/repo", "--issue", "3541"])
        .output()
        .expect("run check");
    assert!(output.status.success());
    let report = String::from_utf8_lossy(&output.stdout);
    assert!(
        report.contains("\"outcome\":\"rejected\""),
        "report: {report}"
    );
    assert!(report.contains("\"label_removed\":false"));
    assert!(report.contains("\"redispatch\":\"blocked\""));
    assert_eq!(
        fixture.labels(),
        vec!["needs-spec-clarification".to_string()]
    );
}

#[test]
fn check_reports_no_proposal_and_awaiting_maintainer() {
    let fixture = RepairFixture::new(DEAD_END_BODY, &["needs-spec-clarification"]);
    let output = fixture
        .command(&["check", "--repo", "test/repo", "--issue", "3541"])
        .output()
        .expect("run check");
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"outcome\":\"no-proposal\""));

    fs::write(
        &fixture.comments,
        "{\"author\": \"autospec-agent\", \"body\": \"<!-- autospec:spec-repair-proposal -->\\n\"}\n",
    )
    .expect("seed proposal comment");
    let output = fixture
        .command(&["check", "--repo", "test/repo", "--issue", "3541"])
        .output()
        .expect("run check");
    let report = String::from_utf8_lossy(&output.stdout);
    assert!(
        report.contains("\"outcome\":\"awaiting-maintainer\""),
        "report: {report}"
    );
    assert!(
        report.contains("\"label_removed\":false"),
        "nothing removed while waiting"
    );
}

#[test]
fn high_stakes_assumption_blocks_with_exit_2_and_untouched_pr_body() {
    let fixture = RepairFixture::new(DEAD_END_BODY, &[]);
    let before = fs::read_to_string(&fixture.pr_body).unwrap();
    let output = fixture
        .command(&[
            "assumption",
            "--statement",
            "Rotate the credential in the deploy pipeline without asking",
            "--pr-body-file",
            fixture.pr_body.to_str().unwrap(),
        ])
        .output()
        .expect("run assumption");
    assert_eq!(output.status.code(), Some(2), "high stakes must block");
    let report = String::from_utf8_lossy(&output.stdout);
    assert!(report.contains("\"stakes\":\"high\""), "report: {report}");
    assert_eq!(
        fs::read_to_string(&fixture.pr_body).unwrap(),
        before,
        "PR body untouched"
    );
}

#[test]
fn low_stakes_assumption_appends_a_reviewable_section_once() {
    let fixture = RepairFixture::new(DEAD_END_BODY, &[]);
    let pr = fixture.pr_body.to_str().unwrap().to_string();
    let args = [
        "assumption",
        "--statement",
        "The report format stays JSON; JSONL is only a display concern.",
        "--alternative",
        "JSONL as the stored format",
        "--rollback",
        "Revert the PR; the format lives in one serializer module.",
        "--pr-body-file",
        pr.as_str(),
    ];
    let first = fixture.command(&args).output().expect("first assumption");
    assert!(
        first.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let report = String::from_utf8_lossy(&first.stdout);
    assert!(report.contains("\"stakes\":\"low\""), "report: {report}");
    assert!(report.contains("\"recorded\":true"));

    let body = fs::read_to_string(&fixture.pr_body).unwrap();
    assert_eq!(body.matches("## Stated assumption (reviewable)").count(), 1);
    assert!(body.contains("The report format stays JSON"));
    assert!(body.contains("JSONL as the stored format"));
    assert!(body.contains("Revert the PR"));

    let second = fixture.command(&args).output().expect("second assumption");
    assert!(second.status.success());
    assert!(String::from_utf8_lossy(&second.stdout).contains("\"already_present\":true"));
    let body = fs::read_to_string(&fixture.pr_body).unwrap();
    assert_eq!(
        body.matches("## Stated assumption (reviewable)").count(),
        1,
        "idempotent append"
    );
}

#[test]
fn stall_reports_the_threshold_and_ledger_filters() {
    let fixture = RepairFixture::new(DEAD_END_BODY, &[]);
    let output = fixture
        .command(&["stall", "--consecutive-runs", "2", "--issue", "3541"])
        .output()
        .expect("run stall");
    let report = String::from_utf8_lossy(&output.stdout);
    assert!(
        report.contains("\"spec_review_required\":true"),
        "report: {report}"
    );
    assert!(report.contains("\"threshold\":2"));
    assert!(report.contains("\"issue\":3541"));

    let output = fixture
        .command(&["stall", "--consecutive-runs", "1"])
        .output()
        .expect("run stall");
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"spec_review_required\":false"));

    fixture.seed_ledger(2);
    let output = fixture
        .command(&[
            "ledger",
            "--file",
            fixture.ledger.to_str().unwrap(),
            "--issue",
            "3541",
        ])
        .output()
        .expect("run ledger");
    let report = String::from_utf8_lossy(&output.stdout);
    assert!(report.contains("\"total\":2"), "report: {report}");
    assert!(report.contains("\"ambiguous\":2"));

    let output = fixture
        .command(&[
            "ledger",
            "--file",
            fixture.ledger.to_str().unwrap(),
            "--shape",
            "over_scoped",
        ])
        .output()
        .expect("run ledger with shape filter");
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"total\":0"));
}
