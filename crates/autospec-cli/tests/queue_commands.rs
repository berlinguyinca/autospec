#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct QueueFixture {
    root: PathBuf,
    auto: PathBuf,
    auto_page_two: PathBuf,
    active: PathBuf,
    pull_requests: PathBuf,
    pull_requests_page_two: PathBuf,
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
        let auto_page_two = root.join("auto-page-two.json");
        let active = root.join("active.json");
        let pull_requests = root.join("pull-requests.json");
        let pull_requests_page_two = root.join("pull-requests-page-two.json");
        let comments = root.join("comments.json");
        let calls = root.join("gh.log");
        fs::write(&auto, "[]\n").expect("write empty candidates");
        fs::write(&auto_page_two, "[]\n").expect("write empty second candidate page");
        fs::write(&active, "[]\n").expect("write empty active issues");
        fs::write(&pull_requests, pull_request_page(Vec::new(), false, None))
            .expect("write empty pull requests");
        fs::write(
            &pull_requests_page_two,
            pull_request_page(Vec::new(), false, None),
        )
        .expect("write empty second pull request page");
        fs::write(&comments, "[]\n").expect("write empty comments");
        fs::write(
            root.join("bin/gh"),
            r#"#!/usr/bin/env sh
set -eu
printf '%s\n' "$@" >> "$AUTOSPEC_QUEUE_CALLS"
if [ "$1" = api ] && [ "$2" = graphql ]; then
  cursor=""
  for value in "$@"; do
    case "$value" in endCursor=*) cursor="$value" ;; esac
  done
  case "$cursor" in
    "") cat "$AUTOSPEC_QUEUE_PRS" ;;
    *) cat "$AUTOSPEC_QUEUE_PRS_PAGE_TWO" ;;
  esac
  exit 0
fi
if [ "$1" = api ] && [ "$2" != repos/test/repo/issues/99/comments ]; then
  endpoint=""
  for value in "$@"; do
    case "$value" in repos/*/issues?*|repos/*/pulls?*) endpoint="$value" ;; esac
  done
  case "$endpoint" in
    *labels=auto-implement*\&page=1) cat "$AUTOSPEC_QUEUE_AUTO" ;;
    *labels=auto-implement*\&page=2) cat "$AUTOSPEC_QUEUE_AUTO_PAGE_TWO" ;;
    *labels=in-progress-by-bot*\&page=1) cat "$AUTOSPEC_QUEUE_ACTIVE" ;;
    *) printf '[]\n' ;;
  esac
  exit 0
fi
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
            auto_page_two,
            active,
            pull_requests,
            pull_requests_page_two,
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
            .env("AUTOSPEC_QUEUE_AUTO_PAGE_TWO", &self.auto_page_two)
            .env("AUTOSPEC_QUEUE_ACTIVE", &self.active)
            .env("AUTOSPEC_QUEUE_PRS", &self.pull_requests)
            .env("AUTOSPEC_QUEUE_PRS_PAGE_TWO", &self.pull_requests_page_two)
            .env("AUTOSPEC_QUEUE_COMMENTS", &self.comments)
            .env("AUTOSPEC_QUEUE_CALLS", &self.calls)
            .env("AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS", "0")
            .env("AUTOSPEC_HEARTBEAT_DIR", self.root.join("heartbeats"))
            .env_remove("AUTOSPEC_CONFIG_FILE");
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

fn issues_json(numbers: std::ops::RangeInclusive<u64>) -> String {
    format!(
        "[{}]",
        numbers
            .map(|number| passing_issue(number, "crates/autospec-cli/src/commands/queue.rs"))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn issue_page(raw_count: usize, numbers: std::ops::RangeInclusive<u64>) -> String {
    format!(
        "{{\"raw_count\":{raw_count},\"items\":[{}]}}",
        numbers
            .map(|number| passing_issue(number, "crates/autospec-cli/src/commands/queue.rs"))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn pull_request(number: u64, body: &str) -> String {
    pull_request_with_checks(number, body, "[]")
}

fn pull_request_with_checks(number: u64, body: &str, checks: &str) -> String {
    format!(
        "{{\"number\":{number},\"state\":\"OPEN\",\"body\":{},\"statusCheckRollup\":{checks}}}",
        json_string(body),
    )
}

fn pull_requests_json(numbers: std::ops::RangeInclusive<u64>) -> Vec<String> {
    numbers
        .map(|number| pull_request(number, "no linked issue"))
        .collect::<Vec<_>>()
}

fn pull_request_page(items: Vec<String>, has_next_page: bool, end_cursor: Option<&str>) -> String {
    format!(
        "{{\"items\":[{}],\"page_info\":{{\"has_next_page\":{has_next_page},\"end_cursor\":{}}}}}",
        items.join(","),
        end_cursor
            .map(json_string)
            .unwrap_or_else(|| "null".to_string()),
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
            r#"{{"ready":[{{"number":11,"title":"issue-11","body":{},"labels":[{{"name":"auto-implement"}},{{"name":"safety:reviewed"}}],"author":{{"login":"agent"}},"paths":["src/a.rs"],"non_blocking_refs":[],"serialization_reasons":[],"parallel_safe":true}},{{"number":12,"title":"issue-12","body":{},"labels":[{{"name":"auto-implement"}},{{"name":"safety:reviewed"}}],"author":{{"login":"agent"}},"paths":["src/b.rs"],"non_blocking_refs":[],"serialization_reasons":[],"parallel_safe":true}}],"blocked":[],"claimed":[],"conflicts":[],"gate_counts":{{"open":2,"candidate":2,"reviewed":2,"blocked":0,"dependency_blocked":0,"linked_pr_blocked":0,"path_conflicted":0,"ready":2,"claimed":0,"selected":2}},"scan_scope":"repository","worker_cap":{{"max_repo_workers":0,"active_count":0,"remaining":2,"reached":false}},"batch":[{{"number":11,"title":"issue-11","body":{},"labels":[{{"name":"auto-implement"}},{{"name":"safety:reviewed"}}],"author":{{"login":"agent"}},"paths":["src/a.rs"],"non_blocking_refs":[],"serialization_reasons":[],"parallel_safe":true}},{{"number":12,"title":"issue-12","body":{},"labels":[{{"name":"auto-implement"}},{{"name":"safety:reviewed"}}],"author":{{"login":"agent"}},"paths":["src/b.rs"],"non_blocking_refs":[],"serialization_reasons":[],"parallel_safe":true}}]}}"#,
            json_string("## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n## Implementation outline\n\n- edit `src/a.rs`\n"),
            json_string("## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n## Implementation outline\n\n- edit `src/b.rs`\n"),
            json_string("## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n## Implementation outline\n\n- edit `src/a.rs`\n"),
            json_string("## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n## Implementation outline\n\n- edit `src/b.rs`\n"),
        ) + "\n"
    );
}

#[test]
fn queue_ready_follows_every_github_page() {
    let fixture = QueueFixture::new();
    fs::write(&fixture.auto, issues_json(1..=100)).expect("write first page");
    fs::write(&fixture.auto_page_two, issues_json(101..=201)).expect("write second page");

    let output = fixture
        .command()
        .args(["queue", "ready", "--repo", "test/repo"])
        .output()
        .expect("queue command starts");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"number\":201"));
    let calls = fs::read_to_string(&fixture.calls).expect("read gh calls");
    assert!(calls
        .contains("repos/test/repo/issues?state=open&labels=auto-implement&per_page=100&page=2"));
}

#[test]
fn queue_ready_keeps_paging_after_a_raw_page_contains_a_pull_request() {
    let fixture = QueueFixture::new();
    fs::write(&fixture.auto, issue_page(100, 1..=99)).expect("write filtered first page");
    fs::write(&fixture.auto_page_two, issue_page(1, 201..=201))
        .expect("write filtered second page");

    let output = fixture
        .command()
        .args(["queue", "ready", "--repo", "test/repo"])
        .output()
        .expect("queue command starts");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"number\":201"));
    let calls = fs::read_to_string(&fixture.calls).expect("read gh calls");
    assert!(calls
        .contains("repos/test/repo/issues?state=open&labels=auto-implement&per_page=100&page=2"));
}

#[test]
fn queue_ready_scans_linked_pull_requests_beyond_the_first_page() {
    let fixture = QueueFixture::new();
    fs::write(
        &fixture.auto,
        format!("[{}]", passing_issue(201, "src/queue.rs")),
    )
    .expect("write candidate");
    fs::write(
        &fixture.pull_requests,
        pull_request_page(pull_requests_json(1..=100), true, Some("cursor-100")),
    )
    .expect("write first pull request page");
    fs::write(
        &fixture.pull_requests_page_two,
        pull_request_page(vec![pull_request(201, "Closes #201")], false, None),
    )
    .expect("write second pull request page");

    let output = fixture
        .command()
        .args(["queue", "ready", "--repo", "test/repo"])
        .output()
        .expect("queue command starts");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let calls = fs::read_to_string(&fixture.calls).expect("read gh calls");
    assert!(
        stdout.contains("\"reason\":\"linked_pr_open\""),
        "stdout={stdout}\ncalls={calls}"
    );
    assert!(calls.contains("api\ngraphql"));
    assert!(calls.contains("endCursor=cursor-100"));
}

#[test]
fn queue_ready_preserves_completed_linked_pull_request_checks() {
    let fixture = QueueFixture::new();
    fs::write(
        &fixture.auto,
        format!("[{}]", passing_issue(202, "src/queue.rs")),
    )
    .expect("write candidate");
    fs::write(
        &fixture.pull_requests,
        pull_request_page(
            vec![pull_request_with_checks(
                202,
                "Fixes #202",
                r#"[{"name":"tests","status":"COMPLETED","conclusion":"SUCCESS"}]"#,
            )],
            false,
            None,
        ),
    )
    .expect("write completed pull request page");

    let output = fixture
        .command()
        .args(["queue", "ready", "--repo", "test/repo"])
        .output()
        .expect("queue command starts");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"ready\":[{\"number\":202"),
        "stdout={stdout}"
    );
    assert!(!stdout.contains("linked_pr_open"), "stdout={stdout}");
}

#[test]
fn queue_ready_fails_closed_when_a_later_github_page_is_malformed() {
    let fixture = QueueFixture::new();
    fs::write(&fixture.auto, issues_json(1..=100)).expect("write first page");
    fs::write(&fixture.auto_page_two, "{not json\n").expect("write malformed second page");

    let output = fixture
        .command()
        .args(["queue", "ready", "--repo", "test/repo"])
        .output()
        .expect("queue command starts");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("could not fetch GitHub auto-implement issue page 2 after 3 attempts"),
        "stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn queue_ready_marks_an_allowlist_as_slice_scoped() {
    let fixture = QueueFixture::new();
    fs::write(
        &fixture.auto,
        format!(
            "[{},{}]",
            passing_issue(1, "src/one.rs"),
            passing_issue(2, "src/two.rs")
        ),
    )
    .expect("write candidates");

    let output = fixture
        .command()
        .env("AUTOSPEC_RUN_ONLY_ISSUES", "2")
        .args(["queue", "ready", "--repo", "test/repo"])
        .output()
        .expect("queue command starts");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"scan_scope\":\"slice\""));
    assert!(stdout.contains("\"open\":2,\"candidate\":1"));
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
fn queue_ready_observes_legacy_stale_claim_without_mutating_remote_state() {
    let fixture = QueueFixture::new();
    let claim_remote = fixture.root.join("claim-remote.git");
    let init = Command::new("git")
        .args(["init", "--bare", claim_remote.to_str().unwrap()])
        .output()
        .expect("initialize claim remote");
    assert!(
        init.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&init.stderr)
    );
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
        r#"[{"id":100,"updated_at":"2000-01-01T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"test/repo\",\"issue\":99,\"worker_id\":\"worker-a\",\"state\":\"claimed\",\"branch\":\"\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2000-01-01T00:00:00Z\",\"updated_at\":\"2000-01-01T00:00:00Z\",\"ttl_seconds\":10800,\"claim_id\":\"claim-generation-a\"}\n<!-- autospec-run-state:end -->"}]"#,
    )
    .expect("write stale state fixture");
    let active_before = fs::read(&fixture.active).expect("read active state before planning");
    let comments_before = fs::read(&fixture.comments).expect("read comments before planning");

    let mut command = fixture.command();
    let output = command
        .args(["queue", "ready", "--repo", "test/repo", "--batch-size", "1"])
        .env("AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS", "1")
        .env("AUTOSPEC_CLAIM_GIT_REMOTE", &claim_remote)
        .env(
            "AUTOSPEC_CLAIM_GIT_STATE_DIR",
            fixture.root.join("claim-state"),
        )
        .output()
        .expect("queue command starts");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"active_count\":1"));
    assert_eq!(
        fs::read(&fixture.active).expect("read active state after planning"),
        active_before
    );
    assert_eq!(
        fs::read(&fixture.comments).expect("read comments after planning"),
        comments_before
    );
    let calls = fs::read_to_string(&fixture.calls).expect("read gh calls");
    assert!(!calls.contains("issue\nedit\n99"));
    assert!(!calls.contains("repos/test/repo/issues/comments/100\n-X\nDELETE"));
    let refs = Command::new("git")
        .args([
            "--git-dir",
            claim_remote.to_str().unwrap(),
            "for-each-ref",
            "--format=%(refname)",
            "refs/autospec/claims/",
        ])
        .output()
        .expect("inspect claim refs");
    assert!(refs.status.success());
    assert!(refs.stdout.is_empty());
}

#[test]
fn queue_ready_applies_configured_trusted_actors_to_safety_review() {
    let fixture = QueueFixture::new();
    let policy = fixture.root.join("trusted-actors.yml");
    fs::write(
        &policy,
        "safety:\n  issue_intent_gate:\n    trusted_actors:\n      - login: release-operator\n",
    )
    .expect("write trusted-actor policy");
    fs::write(
        &fixture.auto,
        r###"[{"number":21,"title":"Reset test database","body":"## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n## Goal\n\nDelete the local test database and repopulate it from fixtures.\n\nOnly test, local, and fixture data are in scope. Production is out of scope.\n\n## Implementation outline\n\n- edit `src/reset.rs`\n","labels":[{"name":"auto-implement"},{"name":"safety:reviewed"}],"author":{"login":"release-operator"}}]"###,
    )
    .expect("write trusted candidate");

    let output = fixture
        .command()
        .env("AUTOSPEC_CONFIG_FILE", policy)
        .args(["queue", "ready", "--repo", "test/repo"])
        .output()
        .expect("queue command starts");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("\"ready\":[{\"number\":21"),
        "stdout={}",
        String::from_utf8_lossy(&output.stdout),
    );
}

#[test]
fn queue_ready_fails_closed_for_an_unsupported_custom_regex() {
    let fixture = QueueFixture::new();
    let policy = fixture.root.join("unsupported-regex.yml");
    fs::write(
        &policy,
        "safety:\n  issue_intent_gate:\n    block_patterns:\n      - id: company-secret-policy\n        patterns:\n          - \"(?i)company secret\"\n",
    )
    .expect("write unsupported policy");
    fs::write(
        &fixture.auto,
        format!("[{}]", passing_issue(22, "src/a.rs")),
    )
    .expect("write candidate fixture");

    let output = fixture
        .command()
        .env("AUTOSPEC_CONFIG_FILE", policy)
        .args(["queue", "ready", "--repo", "test/repo"])
        .output()
        .expect("queue command starts");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported custom regex"));
}

#[test]
fn queue_ready_reports_a_classification_draft_as_blocked() {
    let fixture = QueueFixture::new();
    fs::write(
        &fixture.auto,
        r###"[{"number":23,"title":"draft","body":"## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n## Implementation outline\n\n- edit `src/draft.rs`\n","labels":[{"name":"auto-implement"},{"name":"needs-classify"},{"name":"safety:reviewed"}],"author":{"login":"agent"}}]"###,
    )
    .expect("write draft fixture");

    let output = fixture
        .command()
        .args(["queue", "ready", "--repo", "test/repo"])
        .output()
        .expect("queue command starts");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"ready\":[]"), "stdout={stdout}");
    assert!(
        stdout.contains("\"reason\":\"needs_classify\""),
        "stdout={stdout}"
    );
}

#[test]
fn live_queue_view_json_uses_dispatch_table_for_optional_fields() {
    let source =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/commands/queue.rs"))
            .expect("read queue command source");
    let view_json = source
        .split("fn view_json")
        .nth(1)
        .expect("view_json function exists")
        .split("fn view_field_dispatchers")
        .next()
        .expect("view field dispatcher accessor follows view_json");

    assert!(
        view_json.contains("view_field_dispatchers"),
        "view_json should drive optional fields through a named dispatcher table"
    );
    assert!(
        view_json.matches("fields.push(").count() <= 2,
        "view_json should not reintroduce one fields.push branch per optional field"
    );
}

#[test]
fn live_source_has_no_reference_to_deleted_shell_queue_authorities() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("workspace root")
        .to_path_buf();
    let removed = [
        "skills/autospec-run/scripts/list-ready-issues.sh",
        "skills/autospec-run/scripts/issue-safety-gate.sh",
        "scripts/lint-issue-safety.sh",
    ];
    for path in removed {
        assert!(
            !workspace.join(path).exists(),
            "deleted shell authority still exists: {path}"
        );
    }

    let needles = [
        "list-ready-issues.sh",
        "issue-safety-gate.sh",
        "lint-issue-safety.sh",
    ];
    for root in ["install.sh", "scripts", "skills", "crates"] {
        for path in live_source_files(&workspace.join(root)) {
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            for needle in needles {
                assert!(
                    !content.contains(needle),
                    "live source {} still references {needle}",
                    path.strip_prefix(&workspace).unwrap_or(&path).display(),
                );
            }
        }
    }
}

fn live_source_files(path: &std::path::Path) -> Vec<PathBuf> {
    if path.is_file() {
        return vec![path.to_path_buf()];
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(path).expect("read source directory") {
        let entry = entry.expect("read source entry");
        let name = entry.file_name();
        if matches!(
            name.to_str(),
            Some("tests" | "fixtures" | "target" | "docs")
        ) {
            continue;
        }
        let entry_path = entry.path();
        if entry_path.is_dir() {
            files.extend(live_source_files(&entry_path));
        } else if entry_path.is_file() {
            files.push(entry_path);
        }
    }
    files
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
