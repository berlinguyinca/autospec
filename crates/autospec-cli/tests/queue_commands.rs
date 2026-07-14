use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct QueueFixture {
    root: PathBuf,
    auto: PathBuf,
    active: PathBuf,
    pull_requests: PathBuf,
    comments: PathBuf,
    calls: PathBuf,
}

impl QueueFixture {
    fn new() -> Self {
        let suffix = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "autospec-queue-cli-{}-{suffix}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("bin")).expect("create fixture bin");
        let auto = root.join("auto.json");
        let active = root.join("active.json");
        let pull_requests = root.join("pull-requests.json");
        let comments = root.join("comments.json");
        let calls = root.join("gh.log");
        fs::write(&auto, "[]\n").expect("write empty candidates");
        fs::write(&active, "[]\n").expect("write empty active issues");
        fs::write(&pull_requests, "[]\n").expect("write empty pull requests");
        fs::write(&comments, "[]\n").expect("write empty comments");
        fs::write(
            root.join("bin/gh"),
            r#"#!/usr/bin/env sh
set -eu
printf '%s\n' "$@" >> "$AUTOSPEC_QUEUE_CALLS"
if [ "$1" = issue ] && [ "$2" = list ]; then
  label=""
  while [ "$#" -gt 0 ]; do
    if [ "$1" = --label ]; then label="$2"; fi
    shift
  done
  case "$label" in
    auto-implement) cat "$AUTOSPEC_QUEUE_AUTO" ;;
    in-progress-by-bot) cat "$AUTOSPEC_QUEUE_ACTIVE" ;;
    *) printf '[]\n' ;;
  esac
  exit 0
fi
if [ "$1" = issue ] && [ "$2" = edit ]; then
  printf '[]\n' > "$AUTOSPEC_QUEUE_ACTIVE"
  exit 0
fi
if [ "$1" = pr ] && [ "$2" = list ]; then
  cat "$AUTOSPEC_QUEUE_PRS"
  exit 0
fi
if [ "$1" = api ]; then
  case "$2" in
    repos/test/repo/issues/99/comments) cat "$AUTOSPEC_QUEUE_COMMENTS" ;;
    *) exit 0 ;;
  esac
  exit 0
fi
if [ "$1" = issue ] && [ "$2" = view ]; then
  printf '{"state":"OPEN","body":"","labels":[]}\n'
  exit 0
fi
printf 'unexpected gh invocation: %s\n' "$*" >&2
exit 1
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
            auto,
            active,
            pull_requests,
            comments,
            calls,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_autospec"));
        let path = format!(
            "{}:{}",
            self.root.join("bin").display(),
            std::env::var("PATH").expect("PATH is set")
        );
        command
            .current_dir(&self.root)
            .env("PATH", path)
            .env("AUTOSPEC_QUEUE_AUTO", &self.auto)
            .env("AUTOSPEC_QUEUE_ACTIVE", &self.active)
            .env("AUTOSPEC_QUEUE_PRS", &self.pull_requests)
            .env("AUTOSPEC_QUEUE_COMMENTS", &self.comments)
            .env("AUTOSPEC_QUEUE_CALLS", &self.calls)
            .env("AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS", "0")
            .env("AUTOSPEC_HEARTBEAT_DIR", self.root.join("heartbeats"))
            .env("AUTOSPEC_CONFIG_FILE", self.root.join("missing.yml"));
        command
    }
}

impl Drop for QueueFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn passing_issue(number: u64, path: &str) -> String {
    format!(
        r###"{{"number":{number},"title":"issue-{number}","body":"## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n## Implementation outline\n\n- edit `{path}`\n","labels":[{{"name":"auto-implement"}},{{"name":"safety:reviewed"}}],"author":{{"login":"agent"}}}}"###
    )
}

#[test]
fn queue_ready_emits_the_legacy_top_level_shape_in_number_order() {
    let fixture = QueueFixture::new();
    fs::write(
        &fixture.auto,
        format!(
            "[{},{}]",
            passing_issue(12, "src/b.rs"),
            passing_issue(11, "src/a.rs")
        ),
    )
    .expect("write candidate fixture");

    let output = fixture
        .command()
        .args(["queue", "ready", "--repo", "test/repo", "--batch-size", "2"])
        .output()
        .expect("queue command starts");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            r#"{{"ready":[{{"number":11,"title":"issue-11","body":{},"labels":[{{"name":"auto-implement"}},{{"name":"safety:reviewed"}}],"author":{{"login":"agent"}},"paths":["src/a.rs"],"non_blocking_refs":[],"serialization_reasons":[],"parallel_safe":true}},{{"number":12,"title":"issue-12","body":{},"labels":[{{"name":"auto-implement"}},{{"name":"safety:reviewed"}}],"author":{{"login":"agent"}},"paths":["src/b.rs"],"non_blocking_refs":[],"serialization_reasons":[],"parallel_safe":true}}],"blocked":[],"claimed":[],"conflicts":[],"worker_cap":{{"max_repo_workers":0,"active_count":0,"remaining":2,"reached":false}},"batch":[{{"number":11,"title":"issue-11","body":{},"labels":[{{"name":"auto-implement"}},{{"name":"safety:reviewed"}}],"author":{{"login":"agent"}},"paths":["src/a.rs"],"non_blocking_refs":[],"serialization_reasons":[],"parallel_safe":true}},{{"number":12,"title":"issue-12","body":{},"labels":[{{"name":"auto-implement"}},{{"name":"safety:reviewed"}}],"author":{{"login":"agent"}},"paths":["src/b.rs"],"non_blocking_refs":[],"serialization_reasons":[],"parallel_safe":true}}]}}"#,
            json_string("## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n## Implementation outline\n\n- edit `src/a.rs`\n"),
            json_string("## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n## Implementation outline\n\n- edit `src/b.rs`\n"),
            json_string("## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n## Implementation outline\n\n- edit `src/a.rs`\n"),
            json_string("## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n## Implementation outline\n\n- edit `src/b.rs`\n"),
        ) + "\n"
    );
}

#[test]
fn queue_ready_fails_closed_per_candidate_when_pull_request_evidence_is_malformed() {
    let fixture = QueueFixture::new();
    fs::write(
        &fixture.auto,
        format!("[{}]", passing_issue(15, "src/a.rs")),
    )
    .expect("write candidate fixture");
    fs::write(&fixture.pull_requests, "{not json\n").expect("write malformed PR fixture");

    let output = fixture
        .command()
        .args(["queue", "ready", "--repo", "test/repo"])
        .output()
        .expect("queue command starts");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout)
        .contains(r#""reason":"linked_pr_evidence_unavailable""#));
}

#[test]
fn queue_ready_recovers_an_evidenceless_stale_claim_before_counting_worker_capacity() {
    let fixture = QueueFixture::new();
    fs::write(
        &fixture.auto,
        format!("[{}]", passing_issue(20, "src/a.rs")),
    )
    .expect("write candidate fixture");
    fs::write(
        &fixture.active,
        r#"[{"number":99,"title":"stale","body":"","labels":[{"name":"in-progress-by-bot"}],"author":{"login":"agent"}}]"#,
    )
    .expect("write active fixture");
    fs::write(
        &fixture.comments,
        r#"[{"id":100,"updated_at":"2000-01-01T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"test/repo\",\"issue\":99,\"worker_id\":\"worker-a\",\"state\":\"claimed\",\"branch\":\"\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2000-01-01T00:00:00Z\",\"updated_at\":\"2000-01-01T00:00:00Z\",\"ttl_seconds\":10800}\n<!-- autospec-run-state:end -->"}]"#,
    )
    .expect("write stale state fixture");

    let output = fixture
        .command()
        .args(["queue", "ready", "--repo", "test/repo", "--batch-size", "1"])
        .output()
        .expect("queue command starts");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"active_count\":0"));
    assert!(stdout.contains("\"number\":20"));
    let calls = fs::read_to_string(&fixture.calls).expect("read gh calls");
    let labels = calls.find("issue\nedit\n99").expect("stale label release");
    let clear = calls
        .find("repos/test/repo/issues/comments/100\n-X\nDELETE")
        .expect("stale state clear");
    assert!(labels < clear);
}

fn json_string(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('\n', "\\n")
            .replace('\"', "\\\"")
    )
}
