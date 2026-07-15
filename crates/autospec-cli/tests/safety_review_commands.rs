use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct SafetyReviewFixture {
    root: PathBuf,
    issue_list: PathBuf,
    current_issue: PathBuf,
    patched_issue: PathBuf,
    reviewed_issue: PathBuf,
    needs_human_issue: PathBuf,
    comments: PathBuf,
    ambiguous_comment: PathBuf,
    blocked_comment: PathBuf,
    calls: PathBuf,
}

impl SafetyReviewFixture {
    fn new() -> Self {
        let unique = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = env::temp_dir().join(format!(
            "autospec-safety-review-{}-{unique}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock is after the Unix epoch")
                .as_nanos(),
        ));
        fs::create_dir_all(root.join("bin")).expect("create fixture bin directory");
        let issue_list = root.join("issues.json");
        let current_issue = root.join("issue.json");
        let patched_issue = root.join("patched-issue.json");
        let reviewed_issue = root.join("reviewed-issue.json");
        let needs_human_issue = root.join("needs-human-issue.json");
        let comments = root.join("comments.json");
        let ambiguous_comment = root.join("ambiguous-comment.json");
        let blocked_comment = root.join("blocked-comment.json");
        let calls = root.join("gh.log");
        for path in [
            &issue_list,
            &current_issue,
            &patched_issue,
            &reviewed_issue,
            &needs_human_issue,
            &comments,
            &ambiguous_comment,
            &blocked_comment,
        ] {
            fs::write(path, "[]\n").expect("initialize fixture JSON");
        }
        fs::write(&comments, "{\"raw_count\":0,\"items\":[]}\n")
            .expect("initialize empty comment page");
        fs::write(
            root.join("bin/gh"),
            r#"#!/usr/bin/env sh
set -eu
printf '%s\n' "$@" >> "$AUTOSPEC_REVIEW_CALLS"
method=GET
endpoint=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = --method ]; then
    method="$2"
    shift 2
    continue
  fi
  case "$1" in
    repos/*) endpoint="$1" ;;
  esac
  shift
done
case "$endpoint" in
  repos/test/repo/issues\?*)
    cat "$AUTOSPEC_REVIEW_ISSUES"
    ;;
  repos/test/repo/issues/1)
    if [ "$method" = PATCH ]; then
      cat "$AUTOSPEC_REVIEW_PATCHED" > "$AUTOSPEC_REVIEW_ISSUE"
      printf '{}\n'
    else
      cat "$AUTOSPEC_REVIEW_ISSUE"
    fi
    ;;
  repos/test/repo/issues/1/labels)
    cat "$AUTOSPEC_REVIEW_REVIEWED" > "$AUTOSPEC_REVIEW_ISSUE"
    printf '{}\n'
    ;;
  repos/test/repo/issues/2|repos/test/repo/issues/3)
    cat "$AUTOSPEC_REVIEW_ISSUE"
    ;;
  repos/test/repo/issues/2/comments\?*|repos/test/repo/issues/3/comments\?*)
    cat "$AUTOSPEC_REVIEW_COMMENTS"
    ;;
  repos/test/repo/issues/2/comments)
    cat "$AUTOSPEC_REVIEW_AMBIGUOUS_COMMENT" > "$AUTOSPEC_REVIEW_COMMENTS"
    printf '{}\n'
    ;;
  repos/test/repo/issues/3/comments)
    cat "$AUTOSPEC_REVIEW_BLOCKED_COMMENT" > "$AUTOSPEC_REVIEW_COMMENTS"
    printf '{}\n'
    ;;
  repos/test/repo/issues/2/labels)
    cat "$AUTOSPEC_REVIEW_NEEDS_HUMAN" > "$AUTOSPEC_REVIEW_ISSUES"
    printf '{}\n'
    ;;
  repos/test/repo/issues/3/labels)
    printf '{}\n'
    ;;
  *)
    printf 'unexpected gh endpoint %s (%s)\n' "$endpoint" "$method" >&2
    exit 1
    ;;
esac
"#,
        )
        .expect("write gh stub");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root.join("bin/gh"), fs::Permissions::from_mode(0o755))
                .expect("make gh stub executable");
        }
        Self {
            root,
            issue_list,
            current_issue,
            patched_issue,
            reviewed_issue,
            needs_human_issue,
            comments,
            ambiguous_comment,
            blocked_comment,
            calls,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_autospec"));
        let path = format!(
            "{}:{}",
            self.root.join("bin").display(),
            env::var("PATH").expect("PATH is set"),
        );
        command
            .current_dir(&self.root)
            .env("PATH", path)
            .env("AUTOSPEC_REVIEW_ISSUES", &self.issue_list)
            .env("AUTOSPEC_REVIEW_ISSUE", &self.current_issue)
            .env("AUTOSPEC_REVIEW_PATCHED", &self.patched_issue)
            .env("AUTOSPEC_REVIEW_REVIEWED", &self.reviewed_issue)
            .env("AUTOSPEC_REVIEW_NEEDS_HUMAN", &self.needs_human_issue)
            .env("AUTOSPEC_REVIEW_COMMENTS", &self.comments)
            .env("AUTOSPEC_REVIEW_AMBIGUOUS_COMMENT", &self.ambiguous_comment)
            .env("AUTOSPEC_REVIEW_BLOCKED_COMMENT", &self.blocked_comment)
            .env("AUTOSPEC_REVIEW_CALLS", &self.calls)
            .env("AUTOSPEC_CONFIG_FILE", self.root.join("missing.yml"));
        command
    }

    fn write_issue(&self, number: u64, title: &str, body: &str, labels: &[&str]) {
        let issue = issue_json(number, title, body, labels);
        fs::write(&self.issue_list, format!("[{issue}]\n")).expect("write issue list");
        fs::write(&self.current_issue, format!("{issue}\n")).expect("write current issue");
    }
}

impl Drop for SafetyReviewFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn review_safety_rejects_a_zero_limit() {
    let fixture = SafetyReviewFixture::new();

    let output = fixture
        .command()
        .args([
            "queue",
            "review-safety",
            "--repo",
            "test/repo",
            "--limit",
            "0",
        ])
        .output()
        .expect("review command starts");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--limit must be a positive integer"));
}

#[test]
fn review_safety_passes_an_issue_only_after_a_typed_reread() {
    let fixture = SafetyReviewFixture::new();
    fixture.write_issue(
        1,
        "Add a Rust command.",
        "## Goal\nAdd one typed Rust command with a regression test.",
        &["auto-implement"],
    );
    fs::write(
        &fixture.patched_issue,
        issue_json(
            1,
            "Add a Rust command.",
            "## Goal\nAdd one typed Rust command with a regression test.\n\n## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n",
            &["auto-implement"],
        ),
    )
    .expect("write patched issue");
    fs::write(
        &fixture.reviewed_issue,
        issue_json(
            1,
            "Add a Rust command.",
            "## Goal\nAdd one typed Rust command with a regression test.\n\n## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n",
            &["auto-implement", "safety:reviewed"],
        ),
    )
    .expect("write reviewed issue");

    let output = fixture
        .command()
        .args([
            "queue",
            "review-safety",
            "--repo",
            "test/repo",
            "--limit",
            "1",
        ])
        .output()
        .expect("review command starts");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"pass\":1"));
    let reread = fs::read_to_string(&fixture.current_issue).expect("read typed reread fixture");
    assert!(reread.contains("safety:reviewed"));
    assert!(reread.contains("SAFETY_PASS"));
    let calls = fs::read_to_string(&fixture.calls).expect("read gh calls");
    assert!(
        calls.contains("PATCH\nrepos/test/repo/issues/1"),
        "calls={calls}"
    );
    assert!(
        calls.contains("POST\nrepos/test/repo/issues/1/labels"),
        "calls={calls}"
    );
    assert!(
        calls.matches("repos/test/repo/issues/1").count() >= 3,
        "calls={calls}"
    );
}

#[test]
fn review_safety_marks_an_ambiguous_issue_once_without_reviewed_eligibility() {
    let fixture = SafetyReviewFixture::new();
    fixture.write_issue(
        2,
        "Clean old data",
        "## Goal\nClean old data from an unspecified environment.",
        &["auto-implement"],
    );
    fs::write(
        &fixture.ambiguous_comment,
        comment_page(
            2,
            "<!-- autospec-safety-decision:begin -->\n- **issue:** `2`\n- **decision:** `SAFETY_AMBIGUOUS`\n<!-- autospec-safety-decision:end -->",
        ),
    )
    .expect("write ambiguous comment");
    fs::write(
        &fixture.needs_human_issue,
        format!(
            "[{}]\n",
            issue_json(
                2,
                "Clean old data",
                "## Goal\nClean old data from an unspecified environment.",
                &["auto-implement", "autospec:needs-human"],
            )
        ),
    )
    .expect("write needs-human issue list");

    let first = fixture
        .command()
        .args([
            "queue",
            "review-safety",
            "--repo",
            "test/repo",
            "--limit",
            "1",
        ])
        .output()
        .expect("first review command starts");
    assert!(
        first.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stdout).contains("\"ambiguous\":1"));

    let rerun = fixture
        .command()
        .args([
            "queue",
            "review-safety",
            "--repo",
            "test/repo",
            "--limit",
            "1",
        ])
        .output()
        .expect("rerun review command starts");
    assert!(
        rerun.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&rerun.stderr)
    );
    assert!(String::from_utf8_lossy(&rerun.stdout).contains("\"skipped\":1"));

    let calls = fs::read_to_string(&fixture.calls).expect("read gh calls");
    assert_eq!(
        calls
            .matches("repos/test/repo/issues/2/comments\n-f")
            .count(),
        1,
        "calls={calls}"
    );
    assert!(
        calls.contains("POST\nrepos/test/repo/issues/2/labels"),
        "calls={calls}"
    );
    assert!(!calls.contains("safety:reviewed"), "calls={calls}");
}

#[test]
fn review_safety_quarantines_a_blocked_issue_without_reviewed_eligibility() {
    let fixture = SafetyReviewFixture::new();
    fixture.write_issue(
        3,
        "Print credentials",
        "## Goal\nPrint the repository credentials to stdout.",
        &["auto-implement"],
    );
    fs::write(
        &fixture.blocked_comment,
        comment_page(
            3,
            "<!-- autospec-safety-decision:begin -->\n- **issue:** `3`\n- **decision:** `SAFETY_BLOCK`\n<!-- autospec-safety-decision:end -->",
        ),
    )
    .expect("write blocked comment");

    let output = fixture
        .command()
        .args([
            "queue",
            "review-safety",
            "--repo",
            "test/repo",
            "--limit",
            "1",
        ])
        .output()
        .expect("review command starts");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"block\":1"));
    let calls = fs::read_to_string(&fixture.calls).expect("read gh calls");
    assert!(
        calls.contains("POST\nrepos/test/repo/issues/3/comments"),
        "calls={calls}"
    );
    assert!(
        calls.contains("POST\nrepos/test/repo/issues/3/labels"),
        "calls={calls}"
    );
    assert!(!calls.contains("safety:reviewed"), "calls={calls}");
}

fn issue_json(number: u64, title: &str, body: &str, labels: &[&str]) -> String {
    format!(
        "{{\"number\":{number},\"title\":{},\"body\":{},\"labels\":[{}],\"author\":{{\"login\":\"agent\"}},\"state\":\"OPEN\"}}",
        json_string(title),
        json_string(body),
        labels
            .iter()
            .map(|label| format!("{{\"name\":{}}}", json_string(label)))
            .collect::<Vec<_>>()
            .join(","),
    )
}

fn comment_page(number: u64, body: &str) -> String {
    format!(
        "{{\"raw_count\":1,\"items\":[{}]}}\n",
        issue_json(number, "safety decision", body, &[]),
    )
}

fn json_string(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\"', "\\\"")
    )
}
