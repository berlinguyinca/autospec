use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

pub struct SafetyReviewFixture {
    root: PathBuf,
    issue_list: PathBuf,
    pub current_issue: PathBuf,
    pub patched_issue: PathBuf,
    pub reviewed_issue: PathBuf,
    pub needs_human_issue: PathBuf,
    pub comments: PathBuf,
    pub ambiguous_comment: PathBuf,
    pub blocked_comment: PathBuf,
    pub calls: PathBuf,
}

impl SafetyReviewFixture {
    pub fn new() -> Self {
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

    pub fn command(&self) -> Command {
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

    pub fn write_issue(&self, number: u64, title: &str, body: &str, labels: &[&str]) {
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

pub fn issue_json(number: u64, title: &str, body: &str, labels: &[&str]) -> String {
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

#[allow(dead_code)] // Shared fixture API is used by distinct integration-test crates.
pub fn comment_page(number: u64, body: &str) -> String {
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
