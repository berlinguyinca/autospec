use autospec_core::claim::RunStateRecord;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "support/claim_process_fixture.rs"]
mod claim_process_fixture;
use claim_process_fixture::{bind_to_current_process, current_process_start};

static EXECUTABLE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn autospec() -> Command {
    let fixture = temp_dir("autospec-default-claim-git");
    let remote = fixture.join("remote.git");
    git(&fixture, &["init", "--bare", remote.to_str().unwrap()]);
    let mut command = Command::new(env!("CARGO_BIN_EXE_autospec"));
    command
        .env("AUTOSPEC_CLAIM_GIT_REMOTE", remote)
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("state"));
    command
}

fn temp_dir(prefix: &str) -> std::path::PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{prefix}-{}-{suffix}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("fixture directory");
    dir
}

fn write_executable(path: &std::path::Path, contents: &str) {
    publish_executable(path, contents.as_bytes());
}

fn publish_executable(path: &std::path::Path, contents: &[u8]) {
    let sequence = EXECUTABLE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_extension(format!("tmp-{}-{sequence}", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .expect("fake command temporary");
    file.write_all(contents).expect("fake command");
    drop(file);
    let mut permissions = std::fs::metadata(&temporary)
        .expect("fake command metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&temporary, permissions).expect("fake command permissions");
    std::fs::rename(temporary, path).expect("fake command publish");
    // Atomic rename prevents inode clashes; 30-run stress showed this ZFS host
    // still needs 10 ms after close before exec stops returning ETXTBSY.
    std::thread::sleep(std::time::Duration::from_millis(10));
}

fn path_with(bin: &std::path::Path) -> String {
    format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").expect("test PATH is set")
    )
}

fn claim_run_state(stdout: &[u8]) -> RunStateRecord {
    let text = std::str::from_utf8(stdout).expect("claim state stdout is UTF-8");
    RunStateRecord::parse_json(text.trim()).expect("claim state stdout is a run-state JSON object")
}

fn git(directory: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn claim_git_repo(fixture: &std::path::Path) -> std::path::PathBuf {
    let remote = fixture.join("claim-remote.git");
    let repo = fixture.join("claim-repo");
    git(fixture, &["init", "--bare", remote.to_str().unwrap()]);
    git(fixture, &["init", repo.to_str().unwrap()]);
    git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    repo
}

fn transition_claim_ref(repo: &std::path::Path, record: &RunStateRecord) {
    let reference = format!("refs/autospec/claims/issue-{}", record.issue);
    let remote = git(repo, &["remote", "get-url", "origin"]);
    let current = git(repo, &["ls-remote", "--refs", &remote, &reference]);
    let parent = current.split_whitespace().next().map(str::to_string);
    if parent.is_some() {
        git(repo, &["fetch", "--no-tags", &remote, &reference]);
    }
    let tree = git(repo, &["mktree"]);
    let mut command = Command::new("git");
    command
        .arg("commit-tree")
        .arg(tree)
        .current_dir(repo)
        .env("GIT_AUTHOR_NAME", "Autospec Claim Test")
        .env("GIT_AUTHOR_EMAIL", "autospec-claim-test@localhost")
        .env("GIT_COMMITTER_NAME", "Autospec Claim Test")
        .env("GIT_COMMITTER_EMAIL", "autospec-claim-test@localhost")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());
    if let Some(parent) = parent.as_deref() {
        command.args(["-p", parent]);
    }
    let mut child = command.spawn().expect("create claim commit");
    write!(
        child.stdin.take().expect("claim commit stdin"),
        "autospec-claim-ledger-v1\ngeneration=test-{}\n\n{}\n",
        EXECUTABLE_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        record.to_marked_comment()
    )
    .expect("write claim commit");
    let output = child.wait_with_output().expect("claim commit output");
    assert!(output.status.success());
    let oid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    git(repo, &["push", &remote, &format!("{oid}:{reference}")]);
}

fn claim_ref_oid(repo: &std::path::Path, issue: u64) -> String {
    let remote = git(repo, &["remote", "get-url", "origin"]);
    let reference = format!("refs/autospec/claims/issue-{issue}");
    git(repo, &["ls-remote", "--refs", &remote, &reference])
        .split_whitespace()
        .next()
        .expect("claim ref oid")
        .to_string()
}

fn claim_ref_message(repo: &std::path::Path, issue: u64) -> String {
    let remote = git(repo, &["remote", "get-url", "origin"]);
    let reference = format!("refs/autospec/claims/issue-{issue}");
    git(repo, &["fetch", "--no-tags", &remote, &reference]);
    git(repo, &["show", "-s", "--format=%B", "FETCH_HEAD"])
}

fn claim_refresh_comments(
    worker_id: &str,
    branch: &str,
    claim_id: &str,
    server_updated_at: &str,
) -> String {
    format!(
        r#"[{{"id":100,"updated_at":"{server_updated_at}","body":"<!-- autospec-run-state:begin -->\n{{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"{worker_id}\",\"state\":\"claimed\",\"branch\":\"{branch}\",\"pr\":\"11\",\"step\":\"implementing\",\"paths\":[\"src/lib.rs\"],\"claimed_at\":\"2026-07-14T00:00:00Z\",\"updated_at\":\"2026-07-14T00:00:00Z\",\"ttl_seconds\":1,\"claim_id\":\"{claim_id}\"}}\n<!-- autospec-run-state:end -->"}}]"#
    )
}

fn linked_claim_body(
    worker_id: &str,
    branch: &str,
    claim_id: &str,
    parent: u64,
    generation: &str,
) -> String {
    format!(
        "<!-- autospec-run-state-link parent={parent} generation={generation} -->\n{}",
        claim_refresh_comments(worker_id, branch, claim_id, "2030-01-01T00:00:00Z")
            .trim_start_matches("[{\"id\":100,\"updated_at\":\"2030-01-01T00:00:00Z\",\"body\":\"")
            .trim_end_matches("\"}]")
            .replace("\\n", "\n")
            .replace("\\\"", "\"")
    )
}

fn run_append_only_claim_refresh(
    fixture: &std::path::Path,
    comments: &std::path::Path,
    mode: &str,
) -> std::process::Output {
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    std::fs::write(&log, "").expect("claim log fixture");
    let repo = claim_git_repo(fixture);
    let initial = if mode == "takeover-first" {
        RunStateRecord::new(
            "testorg/testrepo",
            42,
            "worker-takeover",
            "claimed",
            "feat/takeover",
            "",
            "claimed",
            Vec::new(),
            "2026-07-14T00:00:00Z",
            "2026-07-14T00:00:00Z",
            1,
        )
        .with_claim_id("claim-takeover")
    } else {
        RunStateRecord::new(
            "testorg/testrepo",
            42,
            "worker-a",
            "claimed",
            "feat/test",
            "11",
            "implementing",
            vec!["src/lib.rs".to_string()],
            "2026-07-14T00:00:00Z",
            "2026-07-14T00:00:00Z",
            1,
        )
        .with_claim_id("claim-a")
    };
    transition_claim_ref(&repo, &initial);
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        r#"#!/bin/sh
set -eu
{
  printf 'CALL\n'
  printf '%s\n' "$@"
} >> "$AUTOSPEC_CLAIM_LOG"
if [ "$1" = api ] && [ "$2" = repos/testorg/testrepo/issues/42/comments ]; then
  cat "$AUTOSPEC_CLAIM_COMMENTS"
  exit 0
fi
if [ "$1" = issue ] && [ "$2" = comment ]; then
  body=''
  shift 2
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --body) body="$2"; shift 2 ;;
      *) shift ;;
    esac
  done
  while ! mkdir "$AUTOSPEC_CLAIM_LOCK" 2>/dev/null; do sleep 0.01; done
  trap 'rmdir "$AUTOSPEC_CLAIM_LOCK"' EXIT
  case "$AUTOSPEC_CLAIM_APPEND_MODE" in
    takeover-first)
      jq --arg takeover "$AUTOSPEC_CLAIM_TAKEOVER_BODY" --arg own "$body" \
        '. + [{id:101,updated_at:"2030-01-01T00:00:01Z",body:$takeover},{id:102,updated_at:"2030-01-01T00:00:02Z",body:$own}]' \
        "$AUTOSPEC_CLAIM_COMMENTS" > "$AUTOSPEC_CLAIM_COMMENTS.tmp"
      ;;
    renewal-first)
      jq --arg takeover "$AUTOSPEC_CLAIM_TAKEOVER_BODY" --arg own "$body" \
        '. + [{id:101,updated_at:"2030-01-01T00:00:01Z",body:$own},{id:102,updated_at:"2030-01-01T00:00:02Z",body:$takeover}]' \
        "$AUTOSPEC_CLAIM_COMMENTS" > "$AUTOSPEC_CLAIM_COMMENTS.tmp"
      ;;
    ambiguous)
      jq --arg own "$body" \
        '. + [{id:101,updated_at:"2030-01-01T00:00:01Z",body:$own}]' \
        "$AUTOSPEC_CLAIM_COMMENTS" > "$AUTOSPEC_CLAIM_COMMENTS.tmp"
      ;;
  esac
  mv "$AUTOSPEC_CLAIM_COMMENTS.tmp" "$AUTOSPEC_CLAIM_COMMENTS"
  [ "$AUTOSPEC_CLAIM_APPEND_MODE" != ambiguous ]
  exit
fi
exit 17
"#,
    );
    let takeover = linked_claim_body(
        "worker-takeover",
        "feat/takeover",
        "claim-takeover",
        100,
        "takeover-generation",
    );
    autospec()
        .args([
            "claim",
            "state",
            "refresh",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            "worker-a",
            "--branch",
            "feat/test",
            "--claim-id",
            "claim-a",
            "--step",
            "verification",
            "--pr",
            "17",
        ])
        .current_dir(repo)
        .env(
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            fixture.join("claim-remote.git"),
        )
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_COMMENTS", comments)
        .env("AUTOSPEC_CLAIM_APPEND_MODE", mode)
        .env("AUTOSPEC_CLAIM_TAKEOVER_BODY", takeover)
        .env("AUTOSPEC_CLAIM_LOCK", fixture.join("lock"))
        .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0")
        .env("AUTOSPEC_GH_API_RETRIES", "1")
        .output()
        .expect("append-only refresh starts")
}

fn run_paged_claim_refresh(
    fixture: &std::path::Path,
    first_page: &std::path::Path,
    second_page: &std::path::Path,
) -> std::process::Output {
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    std::fs::write(&log, "").expect("claim log fixture");
    let repo = claim_git_repo(fixture);
    let terminal = std::fs::read_to_string(second_page)
        .expect("second claim page")
        .contains("autospec-run-terminal:begin");
    let state = if terminal { "merged" } else { "claimed" };
    let initial = RunStateRecord::new(
        "testorg/testrepo",
        42,
        "worker-a",
        state,
        "feat/test",
        "11",
        state,
        Vec::new(),
        "2026-07-14T00:00:00Z",
        "2026-07-14T00:00:00Z",
        1,
    )
    .with_claim_id("claim-a");
    transition_claim_ref(&repo, &initial);
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        r#"#!/bin/sh
set -eu
{
  printf 'CALL\n'
  printf '%s\n' "$@"
} >> "$AUTOSPEC_CLAIM_LOG"
if [ "$1" = api ] && [ "$2" = repos/testorg/testrepo/issues/42/comments ]; then
  paginate=0
  for argument in "$@"; do [ "$argument" = --paginate ] && paginate=1; done
  if [ "$paginate" -eq 1 ]; then
    jq -s '.[0] + .[1]' "$AUTOSPEC_CLAIM_PAGE_ONE" "$AUTOSPEC_CLAIM_PAGE_TWO"
  else
    cat "$AUTOSPEC_CLAIM_PAGE_ONE"
  fi
  exit 0
fi
if [ "$1" = api ] && [ "$2" = repos/testorg/testrepo/issues/comments/100 ]; then
  body=''
  shift 2
  while [ "$#" -gt 0 ]; do
    case "$1" in
      -f) body="${2#body=}"; shift 2 ;;
      *) shift ;;
    esac
  done
  jq --arg body "$body" '.[0].body=$body' "$AUTOSPEC_CLAIM_PAGE_ONE" > "$AUTOSPEC_CLAIM_PAGE_ONE.tmp"
  mv "$AUTOSPEC_CLAIM_PAGE_ONE.tmp" "$AUTOSPEC_CLAIM_PAGE_ONE"
  exit 0
fi
if [ "$1" = issue ] && [ "$2" = comment ]; then
  exit 0
fi
exit 17
"#,
    );
    autospec()
        .args([
            "claim",
            "state",
            "refresh",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            "worker-a",
            "--branch",
            "feat/test",
            "--claim-id",
            "claim-a",
            "--step",
            "verification",
        ])
        .current_dir(repo)
        .env(
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            fixture.join("claim-remote.git"),
        )
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_PAGE_ONE", first_page)
        .env("AUTOSPEC_CLAIM_PAGE_TWO", second_page)
        .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0")
        .env("AUTOSPEC_GH_API_RETRIES", "1")
        .output()
        .expect("paged refresh starts")
}

#[test]
fn claim_refresh_loses_when_takeover_successor_gets_the_lower_comment_id() {
    // Break caught: read/PATCH/read lets stale renewal overwrite a takeover.
    let fixture = temp_dir("autospec-claim-refresh-takeover-first");
    let comments = fixture.join("comments.json");
    std::fs::write(
        &comments,
        claim_refresh_comments("worker-a", "feat/test", "claim-a", "2000-01-01T00:00:00Z"),
    )
    .expect("claim comments fixture");

    let output = run_append_only_claim_refresh(&fixture, &comments, "takeover-first");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"reason\":\"ownership_lost\""));
    let calls = std::fs::read_to_string(fixture.join("gh.log")).expect("gh log");
    assert_eq!(calls.matches("issue\ncomment\n42").count(), 0);
    assert!(!calls.contains("issues/comments/100"));
}

#[test]
fn claim_refresh_wins_when_renewal_successor_gets_the_lower_comment_id() {
    // Break caught: selecting the highest child lets a later takeover steal an already-won race.
    let fixture = temp_dir("autospec-claim-refresh-renewal-first");
    let comments = fixture.join("comments.json");
    std::fs::write(
        &comments,
        claim_refresh_comments("worker-a", "feat/test", "claim-a", "2000-01-01T00:00:00Z"),
    )
    .expect("claim comments fixture");

    let output = run_append_only_claim_refresh(&fixture, &comments, "renewal-first");

    assert!(output.status.success());
    assert_eq!(
        claim_run_state(&output.stdout).claim_id.as_deref(),
        Some("claim-a")
    );
    let calls = std::fs::read_to_string(fixture.join("gh.log")).expect("gh log");
    assert_eq!(calls.matches("issue\ncomment\n42").count(), 1);
    assert!(!calls.contains("issues/comments/100"));
}

#[test]
fn claim_refresh_recovers_an_ambiguous_post_by_rereading_without_reposting() {
    // Break caught: retrying POST after an ambiguous response creates duplicate successors.
    let fixture = temp_dir("autospec-claim-refresh-ambiguous-post");
    let comments = fixture.join("comments.json");
    std::fs::write(
        &comments,
        claim_refresh_comments("worker-a", "feat/test", "claim-a", "2000-01-01T00:00:00Z"),
    )
    .expect("claim comments fixture");

    let output = run_append_only_claim_refresh(&fixture, &comments, "ambiguous");

    assert!(output.status.success());
    let calls = std::fs::read_to_string(fixture.join("gh.log")).expect("gh log");
    assert_eq!(calls.matches("issue\ncomment\n42").count(), 1);
}

#[test]
fn claim_refresh_follows_a_successor_on_a_later_comment_page() {
    // Break caught: one-page reads renew an ancestor after a later-page takeover.
    let fixture = temp_dir("autospec-claim-refresh-paged-successor");
    let first_page = fixture.join("page-one.json");
    let second_page = fixture.join("page-two.json");
    std::fs::write(
        &first_page,
        claim_refresh_comments("worker-a", "feat/test", "claim-a", "2000-01-01T00:00:00Z"),
    )
    .expect("first comment page");
    std::fs::write(
        &second_page,
        serde_json::json!([{
            "id": 101,
            "updated_at": "2030-01-01T00:00:01Z",
            "body": linked_claim_body(
                "worker-takeover",
                "feat/takeover",
                "claim-takeover",
                100,
                "takeover-generation",
            )
        }])
        .to_string(),
    )
    .expect("second comment page");

    let output = run_paged_claim_refresh(&fixture, &first_page, &second_page);

    assert!(output.status.success());
    let calls = std::fs::read_to_string(fixture.join("gh.log")).expect("gh log");
    assert!(calls.contains("--paginate"));
    assert!(!calls.contains("issues/comments/100"));
}

#[test]
fn claim_refresh_stops_for_terminal_evidence_on_a_later_comment_page() {
    // Break caught: one-page reads can mutate a claim after terminal merge evidence.
    let fixture = temp_dir("autospec-claim-refresh-paged-terminal");
    let first_page = fixture.join("page-one.json");
    let second_page = fixture.join("page-two.json");
    std::fs::write(
        &first_page,
        claim_refresh_comments("worker-a", "feat/test", "claim-a", "2000-01-01T00:00:00Z"),
    )
    .expect("first comment page");
    std::fs::write(
        &second_page,
        r#"[{"id":201,"updated_at":"2030-01-01T00:00:01Z","body":"<!-- autospec-run-terminal:begin -->\n{ \"state\" : \"merged\" }\n<!-- autospec-run-terminal:end -->"}]"#,
    )
    .expect("second comment page");

    let output = run_paged_claim_refresh(&fixture, &first_page, &second_page);

    assert_eq!(output.status.code(), Some(2));
    let calls = std::fs::read_to_string(fixture.join("gh.log")).expect("gh log");
    assert!(!calls.contains("--paginate"));
    assert!(!calls.contains("issues/comments/100"));
    assert!(!calls.contains("issue\ncomment\n42"));
}

fn run_claim_refresh(
    fixture: &std::path::Path,
    comments: &std::path::Path,
    worker_id: &str,
    branch: &str,
    claim_id: &str,
    mode: &str,
) -> std::process::Output {
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    std::fs::write(&log, "").expect("claim log fixture");
    let repo = claim_git_repo(fixture);
    let (owner, owner_branch, owner_claim) = if mode == "takeover" {
        ("worker-takeover", "feat/takeover", "claim-takeover")
    } else if std::fs::read_to_string(comments)
        .expect("claim comments")
        .contains("claim-successor")
    {
        ("worker-a", "feat/test", "claim-successor")
    } else {
        ("worker-a", "feat/test", "claim-a")
    };
    let initial = RunStateRecord::new(
        "testorg/testrepo",
        42,
        owner,
        "claimed",
        owner_branch,
        "11",
        "implementing",
        vec!["src/lib.rs".to_string()],
        "2026-07-14T00:00:00Z",
        "2026-07-14T00:00:00Z",
        1,
    )
    .with_claim_id(owner_claim);
    transition_claim_ref(&repo, &initial);
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        r#"#!/bin/sh
set -eu
printf 'CALL\n' >> "$AUTOSPEC_CLAIM_LOG"
printf '%s\n' "$@" >> "$AUTOSPEC_CLAIM_LOG"
if [ "$1" = api ] && [ "$2" = repos/testorg/testrepo/issues/42/comments ]; then
  cat "$AUTOSPEC_CLAIM_COMMENTS"
  exit 0
fi
if [ "$1" = issue ] && [ "$2" = comment ]; then
  body=''
  shift 2
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --body) body="$2"; shift 2 ;;
      *) shift ;;
    esac
  done
  if [ "$AUTOSPEC_CLAIM_REFRESH_MODE" = takeover ]; then
    jq --arg takeover "$AUTOSPEC_CLAIM_TAKEOVER_BODY" --arg own "$body" \
      '. + [{id:101,updated_at:"2030-01-01T00:00:01Z",body:$takeover},{id:102,updated_at:"2030-01-01T00:00:02Z",body:$own}]' \
      "$AUTOSPEC_CLAIM_COMMENTS" > "$AUTOSPEC_CLAIM_COMMENTS.tmp"
  else
    jq --arg body "$body" \
      '. + [{id:101,updated_at:"2030-01-01T00:00:00Z",body:$body}]' \
      "$AUTOSPEC_CLAIM_COMMENTS" > "$AUTOSPEC_CLAIM_COMMENTS.tmp"
  fi
  mv "$AUTOSPEC_CLAIM_COMMENTS.tmp" "$AUTOSPEC_CLAIM_COMMENTS"
  exit 0
fi
exit 17
"#,
    );
    let takeover_body = linked_claim_body(
        "worker-takeover",
        "feat/takeover",
        "claim-takeover",
        100,
        "takeover-generation",
    );

    autospec()
        .args([
            "claim",
            "state",
            "refresh",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            worker_id,
            "--branch",
            branch,
            "--claim-id",
            claim_id,
            "--step",
            "verification",
            "--pr",
            "17",
        ])
        .current_dir(repo)
        .env(
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            fixture.join("claim-remote.git"),
        )
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_COMMENTS", comments)
        .env("AUTOSPEC_CLAIM_REFRESH_MODE", mode)
        .env("AUTOSPEC_CLAIM_TAKEOVER_BODY", takeover_body)
        .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0")
        .env("AUTOSPEC_GH_API_RETRIES", "1")
        .output()
        .expect("autospec claim state refresh starts")
}

#[test]
fn claim_state_refresh_updates_only_heartbeat_step_and_pr_for_the_exact_generation() {
    // Break caught: renewal reconstructing or replacing immutable claim identity fields.
    let fixture = temp_dir("autospec-claim-refresh-exact");
    let comments = fixture.join("comments.json");
    std::fs::write(
        &comments,
        claim_refresh_comments("worker-a", "feat/test", "claim-a", "2026-07-14T00:00:00Z"),
    )
    .expect("claim comments fixture");

    let output = run_claim_refresh(
        &fixture,
        &comments,
        "worker-a",
        "feat/test",
        "claim-a",
        "apply",
    );

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
    let state = claim_run_state(&output.stdout);
    assert_eq!(state.worker_id, "worker-a");
    assert_eq!(state.branch, "feat/test");
    assert_eq!(state.claim_id.as_deref(), Some("claim-a"));
    assert_eq!(state.claimed_at, "2026-07-14T00:00:00Z");
    assert_eq!(state.paths, ["src/lib.rs"]);
    assert_eq!(state.ttl_seconds, 1);
    assert_eq!(state.step, "verification");
    assert_eq!(state.pr, "17");
    assert_ne!(state.updated_at, "2026-07-14T00:00:00Z");
    let calls = std::fs::read_to_string(fixture.join("gh.log")).expect("gh call log");
    assert_eq!(calls.matches("issue\ncomment\n42").count(), 1);
    assert_eq!(
        calls
            .matches("repos/testorg/testrepo/issues/42/comments")
            .count(),
        1
    );
}

#[test]
fn claim_state_refresh_rejects_a_stale_claim_generation_before_append() {
    // Break caught: an old conductor refreshing a successor's claim by worker/branch alone.
    let fixture = temp_dir("autospec-claim-refresh-stale-generation");
    let comments = fixture.join("comments.json");
    std::fs::write(
        &comments,
        claim_refresh_comments(
            "worker-a",
            "feat/test",
            "claim-successor",
            "2026-07-14T00:00:00Z",
        ),
    )
    .expect("claim comments fixture");

    let output = run_claim_refresh(
        &fixture,
        &comments,
        "worker-a",
        "feat/test",
        "claim-a",
        "apply",
    );

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"reason\":\"ownership_lost\""));
    let calls = std::fs::read_to_string(fixture.join("gh.log")).expect("gh call log");
    assert!(!calls.contains("issue\ncomment\n42"));
}

#[test]
fn claim_state_refresh_rejects_changed_worker_branch_and_claim_id() {
    // Break caught: partial identity comparison allowing any one foreign owner field.
    for (label, worker_id, branch, claim_id) in [
        ("worker", "worker-b", "feat/test", "claim-a"),
        ("branch", "worker-a", "feat/other", "claim-a"),
        ("claim", "worker-a", "feat/test", "claim-b"),
    ] {
        let fixture = temp_dir(&format!("autospec-claim-refresh-{label}"));
        let comments = fixture.join("comments.json");
        std::fs::write(
            &comments,
            claim_refresh_comments("worker-a", "feat/test", "claim-a", "2026-07-14T00:00:00Z"),
        )
        .expect("claim comments fixture");

        let output = run_claim_refresh(&fixture, &comments, worker_id, branch, claim_id, "apply");

        assert_eq!(output.status.code(), Some(2), "{label}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("\"reason\":\"ownership_lost\""),
            "{label}"
        );
        let calls = std::fs::read_to_string(fixture.join("gh.log")).expect("gh call log");
        assert!(!calls.contains("issue\ncomment\n42"), "{label}");
    }
}

#[test]
fn claim_state_refresh_renews_an_exact_generation_after_its_ttl_elapsed() {
    // Break caught: freshness-gating the rightful owner so a slow implementation cannot renew.
    let fixture = temp_dir("autospec-claim-refresh-expired-heartbeat");
    let comments = fixture.join("comments.json");
    std::fs::write(
        &comments,
        claim_refresh_comments("worker-a", "feat/test", "claim-a", "2000-01-01T00:00:00Z"),
    )
    .expect("expired claim comments fixture");

    let output = run_claim_refresh(
        &fixture,
        &comments,
        "worker-a",
        "feat/test",
        "claim-a",
        "apply",
    );

    assert!(output.status.success());
    assert_eq!(
        claim_run_state(&output.stdout).claim_id.as_deref(),
        Some("claim-a")
    );
}

#[test]
fn claim_state_refresh_reports_takeover_when_a_lower_sibling_wins() {
    // Break caught: treating a successful POST exit as ownership proof after a concurrent takeover.
    let fixture = temp_dir("autospec-claim-refresh-takeover");
    let comments = fixture.join("comments.json");
    std::fs::write(
        &comments,
        claim_refresh_comments("worker-a", "feat/test", "claim-a", "2026-07-14T00:00:00Z"),
    )
    .expect("claim comments fixture");

    let output = run_claim_refresh(
        &fixture,
        &comments,
        "worker-a",
        "feat/test",
        "claim-a",
        "takeover",
    );

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"reason\":\"ownership_lost\""));
    let calls = std::fs::read_to_string(fixture.join("gh.log")).expect("gh call log");
    assert_eq!(calls.matches("issue\ncomment\n42").count(), 0);
}

#[test]
fn claim_state_read_selects_the_lowest_marked_github_comment() {
    let fixture = temp_dir("autospec-claim-state-read");
    let bin = fixture.join("bin");
    let repo = claim_git_repo(&fixture);
    transition_claim_ref(
        &repo,
        &RunStateRecord::new(
            "testorg/testrepo",
            42,
            "worker-a",
            "claimed",
            "feat/test",
            "",
            "claimed",
            Vec::new(),
            "2026-07-14T00:00:00Z",
            "2026-07-14T00:00:00Z",
            10_800,
        ),
    );
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nif [ \"$1\" = api ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_COMMENTS\"; else exit 19; fi\n",
    );
    let comments = r#"[
      {"id":101,"updated_at":"2026-07-14T00:01:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-b\",\"state\":\"claimed\",\"branch\":\"feat/test\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2026-07-14T00:01:00Z\",\"updated_at\":\"2026-07-14T00:01:00Z\",\"ttl_seconds\":10800}\n<!-- autospec-run-state:end -->"},
      {"id":100,"updated_at":"2026-07-14T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-a\",\"state\":\"claimed\",\"branch\":\"feat/test\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2026-07-14T00:00:00Z\",\"updated_at\":\"2026-07-14T00:00:00Z\",\"ttl_seconds\":10800}\n<!-- autospec-run-state:end -->"}
    ]"#;

    let output = autospec()
        .args([
            "claim",
            "state",
            "read",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
        ])
        .env(
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            fixture.join("claim-remote.git"),
        )
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_COMMENTS", comments)
        .output()
        .expect("autospec claim state read starts");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
    let state = claim_run_state(&output.stdout);
    assert_eq!(state.worker_id, "worker-a");
}

#[test]
fn claim_state_upsert_appends_to_the_authoritative_parent_without_deleting_history() {
    let fixture = temp_dir("autospec-claim-state-upsert");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    let comments = fixture.join("comments.json");
    let repo = claim_git_repo(&fixture);
    transition_claim_ref(
        &repo,
        &RunStateRecord::new(
            "testorg/testrepo",
            42,
            "worker-c",
            "claimed",
            "feat/test",
            "",
            "claimed",
            Vec::new(),
            "2026-07-14T00:00:00Z",
            "2026-07-14T00:00:00Z",
            10_800,
        )
        .with_claim_id("claim-c"),
    );
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then cat \"$AUTOSPEC_CLAIM_COMMENTS\"; exit 0; fi\nif [ \"$1\" = issue ] && [ \"$2\" = comment ]; then\n  body=''; shift 2\n  while [ \"$#\" -gt 0 ]; do case \"$1\" in --body) body=\"$2\"; shift 2 ;; *) shift ;; esac; done\n  jq --arg body \"$body\" '. + [{id:102,updated_at:\"2030-01-01T00:00:00Z\",body:$body}]' \"$AUTOSPEC_CLAIM_COMMENTS\" > \"$AUTOSPEC_CLAIM_COMMENTS.tmp\"\n  mv \"$AUTOSPEC_CLAIM_COMMENTS.tmp\" \"$AUTOSPEC_CLAIM_COMMENTS\"\n  exit 0\nfi\nexit 17\n",
    );
    std::fs::write(
        &comments,
        r#"[
      {"id":101,"updated_at":"2026-07-14T00:01:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-b\",\"state\":\"claimed\",\"branch\":\"feat/test\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2026-07-14T00:01:00Z\",\"updated_at\":\"2026-07-14T00:01:00Z\",\"ttl_seconds\":10800}\n<!-- autospec-run-state:end -->"},
      {"id":100,"updated_at":"2026-07-14T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-a\",\"state\":\"claimed\",\"branch\":\"feat/test\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2026-07-14T00:00:00Z\",\"updated_at\":\"2026-07-14T00:00:00Z\",\"ttl_seconds\":10800}\n<!-- autospec-run-state:end -->"}
    ]"#,
    )
    .expect("comments fixture");

    let output = autospec()
        .args([
            "claim",
            "state",
            "upsert",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            "worker-c",
            "--claim-id",
            "claim-c",
            "--branch",
            "feat/test",
            "--state",
            "worktree_ready",
            "--step",
            "worktree_ready",
            "--paths",
            "crates/autospec-core/src/claim/mod.rs,crates/autospec-cli/src/commands/claim.rs",
            "--ttl-seconds",
            "7200",
        ])
        .env(
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            fixture.join("claim-remote.git"),
        )
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_COMMENTS", &comments)
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0")
        .output()
        .expect("autospec claim state upsert starts");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
    let state = claim_run_state(&output.stdout);
    assert_eq!(state.worker_id, "worker-c");
    assert_eq!(state.ttl_seconds, 7200);
    assert_eq!(state.claimed_at, "2026-07-14T00:00:00Z");
    let calls = std::fs::read_to_string(log).expect("gh call log");
    assert_eq!(calls.matches("issue\ncomment\n42").count(), 1);
    assert!(!calls.contains("\nPATCH\n"));
    assert!(!calls.contains("\nDELETE\n"));
    let persisted = std::fs::read_to_string(comments).expect("appended comments");
    assert!(persisted.contains(
        "autospec-run-state-link parent=100 parent_generation=legacy generation=claim-ref-"
    ));
    assert!(persisted.contains("worker-b"));
}

#[test]
fn claim_state_upsert_requires_expected_claim_id() {
    let fixture = temp_dir("autospec-claim-upsert-requires-generation");
    let bin = fixture.join("bin");
    let comments = fixture.join("comments.json");
    let repo = claim_git_repo(&fixture);
    transition_claim_ref(
        &repo,
        &RunStateRecord::new(
            "testorg/testrepo",
            42,
            "worker-a",
            "claimed",
            "feat/test",
            "",
            "claimed",
            Vec::new(),
            "2999-07-25T00:00:00Z",
            "2999-07-25T00:00:00Z",
            10_800,
        )
        .with_claim_id("claim-a"),
    );
    let original_oid = claim_ref_oid(&repo, 42);
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    std::fs::write(&comments, "[]\n").expect("comments fixture");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nset -eu\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then cat \"$AUTOSPEC_CLAIM_COMMENTS\"; exit 0; fi\nif [ \"$1\" = issue ] && [ \"$2\" = comment ]; then exit 0; fi\nexit 17\n",
    );

    let output = autospec()
        .args([
            "claim",
            "state",
            "upsert",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            "worker-a",
            "--branch",
            "feat/test",
            "--state",
            "implementing",
        ])
        .current_dir(&repo)
        .env(
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            fixture.join("claim-remote.git"),
        )
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_COMMENTS", &comments)
        .output()
        .expect("autospec claim state upsert starts");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--claim-id is required"));
    assert_eq!(claim_ref_oid(&repo, 42), original_oid);
}

#[test]
fn stale_claim_generation_cannot_upsert_successor_state() {
    let fixture = temp_dir("autospec-claim-upsert-stale-generation");
    let repo = claim_git_repo(&fixture);
    transition_claim_ref(
        &repo,
        &RunStateRecord::new(
            "testorg/testrepo",
            42,
            "worker-a",
            "claimed",
            "feat/test",
            "",
            "claimed",
            Vec::new(),
            "2999-07-25T00:00:00Z",
            "2999-07-25T00:00:00Z",
            10_800,
        )
        .with_claim_id("claim-b"),
    );
    let original_oid = claim_ref_oid(&repo, 42);

    let output = autospec()
        .args([
            "claim",
            "state",
            "upsert",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            "worker-a",
            "--branch",
            "feat/test",
            "--claim-id",
            "claim-a",
            "--state",
            "blocked",
        ])
        .current_dir(&repo)
        .env(
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            fixture.join("claim-remote.git"),
        )
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
        .output()
        .expect("autospec stale upsert starts");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("claim ID ownership"));
    assert_eq!(claim_ref_oid(&repo, 42), original_oid);
    assert!(claim_ref_message(&repo, 42).contains("\"claim_id\":\"claim-b\""));
}

#[test]
fn claim_state_upsert_recovers_an_ambiguous_post_without_repeating_it() {
    let fixture = temp_dir("autospec-claim-state-retry");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    let comments = fixture.join("comments.json");
    let repo = claim_git_repo(&fixture);
    transition_claim_ref(
        &repo,
        &RunStateRecord::new(
            "testorg/testrepo",
            42,
            "worker-a",
            "claimed",
            "feat/test",
            "",
            "claimed",
            Vec::new(),
            "2026-07-14T00:00:00Z",
            "2026-07-14T00:00:00Z",
            10_800,
        )
        .with_claim_id("claim-a"),
    );
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then cat \"$AUTOSPEC_CLAIM_COMMENTS\"; exit 0; fi\nif [ \"$1\" = issue ] && [ \"$2\" = comment ]; then\n  body=''; shift 2\n  while [ \"$#\" -gt 0 ]; do case \"$1\" in --body) body=\"$2\"; shift 2 ;; *) shift ;; esac; done\n  jq --arg body \"$body\" '. + [{id:101,updated_at:\"2030-01-01T00:00:00Z\",body:$body}]' \"$AUTOSPEC_CLAIM_COMMENTS\" > \"$AUTOSPEC_CLAIM_COMMENTS.tmp\"\n  mv \"$AUTOSPEC_CLAIM_COMMENTS.tmp\" \"$AUTOSPEC_CLAIM_COMMENTS\"\n  exit 1\nfi\nexit 17\n",
    );
    std::fs::write(
        &comments,
        r#"[{"id":100,"updated_at":"2026-07-14T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-a\",\"state\":\"claimed\",\"branch\":\"\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2026-07-14T00:00:00Z\",\"updated_at\":\"2026-07-14T00:00:00Z\",\"ttl_seconds\":10800}\n<!-- autospec-run-state:end -->"}]"#,
    )
    .expect("comments fixture");

    let output = autospec()
        .args([
            "claim",
            "state",
            "upsert",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            "worker-a",
            "--claim-id",
            "claim-a",
            "--branch",
            "feat/test",
            "--state",
            "worktree_ready",
        ])
        .env(
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            fixture.join("claim-remote.git"),
        )
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_COMMENTS", &comments)
        .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0")
        .env("AUTOSPEC_GH_API_RETRIES", "1")
        .output()
        .expect("autospec claim state upsert starts");

    assert!(output.status.success());
    let calls = std::fs::read_to_string(log).expect("gh call log");
    assert_eq!(calls.matches("issue\ncomment\n42").count(), 1);
}

#[test]
fn claim_state_clear_without_exact_identity_leaves_audit_history_untouched() {
    let fixture = temp_dir("autospec-claim-state-clear");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    std::fs::write(&log, "").expect("gh log fixture");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_COMMENTS\"; fi\n",
    );
    let comments = r#"[
      {"id":101,"updated_at":"2026-07-14T00:01:00Z","body":"<!-- autospec-run-state:begin -->\n{}\n<!-- autospec-run-state:end -->"},
      {"id":100,"updated_at":"2026-07-14T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{}\n<!-- autospec-run-state:end -->"},
      {"id":99,"updated_at":"2026-07-14T00:00:00Z","body":"ordinary comment"}
    ]"#;

    let output = autospec()
        .args([
            "claim",
            "state",
            "clear",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
        ])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_COMMENTS", comments)
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0")
        .output()
        .expect("autospec claim state clear starts");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--worker-id is required"));
    let calls = std::fs::read_to_string(log).expect("gh call log");
    assert!(!calls.contains("\n-X\nDELETE"));
}

#[cfg(target_os = "linux")]
#[test]
fn claim_legacy_stale_heartbeat_recovery() {
    let fixture = temp_dir("autospec-claim-recover-stale");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_COMMENTS\"; fi\n",
    );
    let comments = r#"[{"id":100,"updated_at":"2000-01-01T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-a\",\"state\":\"claimed\",\"branch\":\"\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2000-01-01T00:00:00Z\",\"updated_at\":\"2000-01-01T00:00:00Z\",\"ttl_seconds\":10800,\"claim_id\":\"claim-a\"}\n<!-- autospec-run-state:end -->"}]"#;
    let heartbeats = fixture.join("heartbeats");
    let heartbeat_repo = heartbeats.join("o7_testorg_r8_testrepo");
    std::fs::create_dir_all(&heartbeat_repo).expect("heartbeat repository");
    std::fs::set_permissions(&heartbeats, std::fs::Permissions::from_mode(0o700))
        .expect("private heartbeat root");
    std::fs::set_permissions(&heartbeat_repo, std::fs::Permissions::from_mode(0o700))
        .expect("private heartbeat repository");
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname").expect("host identity");
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").expect("boot identity");
    std::fs::write(
        heartbeat_repo.join("42.json"),
        format!(
            "{{\"issue\":\"42\",\"branch\":\"\",\"step\":\"claimed\",\"ts\":1,\"ttl_seconds\":1,\"pid\":2147483647,\"nonce\":\"cb2fb10be6aeeaa790206bdd149beaf909af1587ff0f794c1a88d479f39f1ded\",\"host\":{:?},\"boot_id\":{:?},\"process_start\":\"1\",\"pr\":\"\",\"repo\":\"testorg/testrepo\",\"worker_id\":\"worker-a\",\"claim_id\":\"claim-a\"}}\n",
            host.trim(),
            boot.trim()
        ),
    )
    .expect("expired heartbeat");
    std::fs::set_permissions(
        heartbeat_repo.join("42.json"),
        std::fs::Permissions::from_mode(0o600),
    )
    .expect("private heartbeat");

    let output = autospec()
        .args([
            "claim",
            "state",
            "recover-stale-startup",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--timeout-seconds",
            "300",
        ])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_COMMENTS", comments)
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0")
        .env("AUTOSPEC_HEARTBEAT_DIR", &heartbeats)
        .output()
        .expect("autospec claim stale recovery starts");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"recovered\":true"));
    let calls = std::fs::read_to_string(log).expect("gh call log");
    let labels = calls
        .find("issue\nedit\n42\n--repo\ntestorg/testrepo\n--remove-label\nin-progress-by-bot\n--add-label\nauto-implement")
        .expect("label release");
    let clear = calls
        .find("repos/testorg/testrepo/issues/comments/100\n-X\nDELETE")
        .expect("state clear");
    assert!(labels < clear);
}

#[test]
fn claim_paginated_comments_flatten_pages_without_combining_slurp_and_jq() {
    // Break caught: gh rejects --slurp with --jq before returning paginated comments.
    let fixture = temp_dir("autospec-claim-paginated-comments");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        r#"#!/bin/sh
set -eu
printf '%s\n' "$@" >> "$AUTOSPEC_CLAIM_LOG"
slurp=0
jq=0
for argument in "$@"; do
  [ "$argument" = --slurp ] && slurp=1
  [ "$argument" = --jq ] && jq=1
done
if [ "$slurp" -eq 1 ] && [ "$jq" -eq 1 ]; then
  printf '%s\n' 'the `--slurp` option is not supported with `--jq` or `--template`' >&2
  exit 64
fi
if [ "$1" = api ] && [ "$2" = repos/testorg/testrepo/issues/42/comments ]; then
  printf '%s\n' "$AUTOSPEC_CLAIM_COMMENTS"
  exit 0
fi
if [ "$1" = issue ] && [ "$2" = edit ]; then exit 0; fi
if [ "$1" = api ] && [ "$2" = repos/testorg/testrepo/issues/comments/100 ]; then exit 0; fi
exit 17
"#,
    );
    let comments = r#"[
      [{"id":99,"updated_at":"2000-01-01T00:00:00Z","body":"ordinary comment","user":{"login":"operator"}}],
      [{"id":100,"updated_at":"2000-01-01T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-a\",\"state\":\"claimed\",\"branch\":\"\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2000-01-01T00:00:00Z\",\"updated_at\":\"2000-01-01T00:00:00Z\",\"ttl_seconds\":10800}\n<!-- autospec-run-state:end -->","user":{"login":"autospec"}}]
    ]"#;

    let output = autospec()
        .args([
            "claim",
            "state",
            "recover-stale-startup",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--timeout-seconds",
            "300",
        ])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_COMMENTS", comments)
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0")
        .env("AUTOSPEC_HEARTBEAT_DIR", fixture.join("heartbeats"))
        .output()
        .expect("autospec claim stale recovery starts");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"recovered\":true"));
    let calls = std::fs::read_to_string(log).expect("gh call log");
    assert!(calls.contains("--paginate\n--slurp"));
    assert!(!calls.contains("--jq"));
    assert!(calls.contains("repos/testorg/testrepo/issues/comments/100\n-X\nDELETE"));
}

#[test]
fn claim_state_recover_stale_startup_preserves_a_fresh_claim_without_label_mutation() {
    let fixture = temp_dir("autospec-claim-recover-fresh");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_COMMENTS\"; fi\n",
    );
    let comments = r#"[{"id":100,"updated_at":"2999-01-01T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-a\",\"state\":\"claimed\",\"branch\":\"\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2999-01-01T00:00:00Z\",\"updated_at\":\"2999-01-01T00:00:00Z\",\"ttl_seconds\":10800}\n<!-- autospec-run-state:end -->"}]"#;

    let output = autospec()
        .args([
            "claim",
            "state",
            "recover-stale-startup",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--timeout-seconds",
            "300",
        ])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_COMMENTS", comments)
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_HEARTBEAT_DIR", fixture.join("heartbeats"))
        .output()
        .expect("autospec claim stale recovery starts");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"recovered\":false"));
    let calls = std::fs::read_to_string(log).expect("gh call log");
    assert!(!calls.contains("issue\nedit"));
    assert!(!calls.contains("\n-X\nDELETE"));
}

#[test]
fn stale_startup_recovery_advances_ref_to_available_before_requeue() {
    let fixture = temp_dir("autospec-claim-recover-stale-ref");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    let comments = fixture.join("comments.json");
    let repo = claim_git_repo(&fixture);
    transition_claim_ref(
        &repo,
        &RunStateRecord::new(
            "testorg/testrepo",
            42,
            "worker-a",
            "claimed",
            "",
            "",
            "claimed",
            Vec::new(),
            "2000-01-01T00:00:00Z",
            "2000-01-01T00:00:00Z",
            1,
        )
        .with_claim_id("claim-a"),
    );
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    std::fs::write(&comments, "[]\n").expect("comments fixture");
    write_executable(
        &bin.join("gh"),
        r#"#!/bin/sh
set -eu
printf '%s\n' "$@" >> "$AUTOSPEC_CLAIM_LOG"
if [ "$1" = api ] && [ "$2" = repos/testorg/testrepo/issues/42/comments ]; then
  cat "$AUTOSPEC_CLAIM_COMMENTS"
  exit 0
fi
if { [ "$1" = issue ] && [ "$2" = comment ]; } || { [ "$1" = issue ] && [ "$2" = edit ]; }; then
  git --git-dir "$AUTOSPEC_CLAIM_REMOTE" show -s --format=%B refs/autospec/claims/issue-42 |
    grep -Fq '"state":"available"' || exit 45
  exit 0
fi
exit 17
"#,
    );

    let output = autospec()
        .args([
            "claim",
            "state",
            "recover-stale-startup",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--timeout-seconds",
            "300",
        ])
        .current_dir(&repo)
        .env(
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            fixture.join("claim-remote.git"),
        )
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_COMMENTS", &comments)
        .env("AUTOSPEC_CLAIM_REMOTE", fixture.join("claim-remote.git"))
        .env("AUTOSPEC_HEARTBEAT_DIR", fixture.join("heartbeats"))
        .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0")
        .output()
        .expect("autospec stale startup recovery starts");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"recovered\":true"));
    let message = claim_ref_message(&repo, 42);
    assert!(message.contains("\"state\":\"available\""));
    assert!(message.contains("\"step\":\"stale_startup_recovered\""));
    assert!(message.contains("\"claim_id\":\"claim-a\""));
}

#[cfg(target_os = "linux")]
#[test]
fn claim_stale_heartbeat_recovery() {
    use std::os::unix::fs::PermissionsExt;

    // Break caught: authoritative recovery treated every heartbeat pathname as live evidence,
    // so an exact expired heartbeat from a dead process could wedge its claim forever.
    let fixture = temp_dir("autospec-claim-stale-heartbeat-recovery");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    let repo = claim_git_repo(&fixture);
    let heartbeats = fixture.join("heartbeats");
    let heartbeat_repo = heartbeats.join("o7_testorg_r8_testrepo");
    let handoff = heartbeat_repo.join("quarantine/startup-heartbeat-handoffs");
    transition_claim_ref(
        &repo,
        &RunStateRecord::new(
            "testorg/testrepo",
            42,
            "worker-a",
            "claimed",
            "feat/test",
            "",
            "claimed",
            Vec::new(),
            "2000-01-01T00:00:00Z",
            "2000-01-01T00:00:00Z",
            1,
        )
        .with_claim_id("claim-a"),
    );
    transition_claim_ref(
        &repo,
        &RunStateRecord::new(
            "testorg/testrepo",
            43,
            "worker-b",
            "claimed",
            "feat/empty",
            "",
            "claimed",
            Vec::new(),
            "2000-01-01T00:00:00Z",
            "2000-01-01T00:00:00Z",
            1,
        )
        .with_claim_id("claim-b"),
    );
    for issue in [44, 45, 46, 47] {
        transition_claim_ref(
            &repo,
            &RunStateRecord::new(
                "testorg/testrepo",
                issue,
                "worker-new",
                "claimed",
                "feat/test",
                "",
                "heartbeat-pending:none",
                Vec::new(),
                "2000-01-01T00:00:00Z",
                "2000-01-01T00:00:00Z",
                1,
            )
            .with_claim_id("claim-new"),
        );
    }
    transition_claim_ref(
        &repo,
        &RunStateRecord::new(
            "testorg/testrepo",
            48,
            "worker-new",
            "claimed",
            "feat/test",
            "",
            "heartbeat-publishing:73657373696f6e2d63757272656e74:attempt-live",
            Vec::new(),
            "2000-01-01T00:00:00Z",
            "2000-01-01T00:00:00Z",
            1,
        )
        .with_claim_id("claim-new"),
    );
    std::fs::create_dir_all(&heartbeat_repo).expect("heartbeat repository");
    std::fs::set_permissions(&heartbeats, std::fs::Permissions::from_mode(0o700))
        .expect("private heartbeat root");
    std::fs::set_permissions(&heartbeat_repo, std::fs::Permissions::from_mode(0o700))
        .expect("private heartbeat repository");
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname").expect("host identity");
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").expect("boot identity");
    let process_start = current_process_start();
    let heartbeat = heartbeat_repo.join("42.json");
    let expired_document = format!(
        "{{\"issue\":\"42\",\"branch\":\"feat/test\",\"step\":\"claimed\",\"ts\":1,\"ttl_seconds\":1,\"pid\":2147483647,\"nonce\":\"cb2fb10be6aeeaa790206bdd149beaf909af1587ff0f794c1a88d479f39f1ded\",\"host\":{:?},\"boot_id\":{:?},\"process_start\":\"1\",\"pr\":\"\",\"repo\":\"testorg/testrepo\",\"worker_id\":\"worker-a\",\"claim_id\":\"claim-a\"}}\n",
        host.trim(),
        boot.trim()
    );
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        r#"#!/bin/sh
set -eu
printf '%s\n' "$@" >> "$AUTOSPEC_CLAIM_LOG"
if [ "$1" = api ]; then printf '[]\n'; exit 0; fi
if [ "$1 $2" = 'issue comment' ] || [ "$1 $2" = 'issue edit' ]; then
  if printf '%s\n' "$@" | grep -qx 42; then
    # linter:allow-DOC_OUT_OF_SYNC existing git flag remains test-fixture-only after issue routing
    git --git-dir "$AUTOSPEC_CLAIM_REMOTE" show -s --format=%B refs/autospec/claims/issue-42 |
      grep -Fq '"state":"available"' || exit 45
    find "$AUTOSPEC_HEARTBEAT_HANDOFF" -type f -name '*.json' -print -quit |
      grep -q . || exit 46
  fi
  exit 0
fi
exit 17
"#,
    );

    let recover = |issue| {
        autospec()
            .args([
                "claim",
                "state",
                "recover-stale-startup",
                "--issue",
                issue,
                "--repo",
                "testorg/testrepo",
                // linter:allow-DOC_OUT_OF_SYNC existing recovery flag moved within test closure
                "--timeout-seconds",
                "1",
            ])
            .current_dir(&repo)
            .env(
                "AUTOSPEC_CLAIM_GIT_REMOTE",
                fixture.join("claim-remote.git"),
            )
            .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
            .env("AUTOSPEC_HEARTBEAT_DIR", &heartbeats)
            .env("AUTOSPEC_HEARTBEAT_HANDOFF", &handoff)
            .env("AUTOSPEC_CLAIM_REMOTE", fixture.join("claim-remote.git"))
            .env("AUTOSPEC_CLAIM_LOG", &log)
            .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0")
            .env("PATH", path_with(&bin))
            .output()
            .expect("autospec stale heartbeat recovery starts")
    };
    let absent = recover("43");
    assert!(absent.status.success());
    assert!(String::from_utf8_lossy(&absent.stdout).contains("\"recovered\":true"));
    assert!(claim_ref_message(&repo, 43).contains("\"state\":\"available\""));
    let claimed = claim_ref_message(&repo, 42);
    nix::unistd::mkfifo(&heartbeat, nix::sys::stat::Mode::from_bits_truncate(0o600))
        .expect("heartbeat FIFO");
    let started = std::time::Instant::now();
    let blocked = recover("42");
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    assert!(String::from_utf8_lossy(&blocked.stdout).contains("\"recovered\":false"));
    assert_eq!(claim_ref_message(&repo, 42), claimed);
    std::fs::remove_file(&heartbeat).expect("remove FIFO");

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("wall clock")
        .as_secs();
    let live_document = format!(
        "{{\"issue\":\"42\",\"branch\":\"feat/test\",\"step\":\"claimed\",\"ts\":{now},\"ttl_seconds\":1,\"pid\":{},\"nonce\":\"cb2fb10be6aeeaa790206bdd149beaf909af1587ff0f794c1a88d479f39f1ded\",\"host\":{:?},\"boot_id\":{:?},\"process_start\":\"{process_start}\",\"pr\":\"\",\"repo\":\"testorg/testrepo\",\"worker_id\":\"worker-a\",\"claim_id\":\"claim-a\"}}\n",
        std::process::id(), host.trim(), boot.trim()
    );
    for document in ["{}\n".to_string(), live_document] {
        std::fs::write(&heartbeat, &document).expect("blocking heartbeat");
        std::fs::set_permissions(&heartbeat, std::fs::Permissions::from_mode(0o600))
            .expect("private heartbeat");
        let blocked = recover("42");
        assert!(String::from_utf8_lossy(&blocked.stdout).contains("\"recovered\":false"));
        assert_eq!(claim_ref_message(&repo, 42), claimed);
        std::fs::remove_file(&heartbeat).expect("remove blocking heartbeat");
    }

    std::fs::write(&heartbeat, &expired_document).expect("expired heartbeat");
    std::fs::set_permissions(&heartbeat, std::fs::Permissions::from_mode(0o600))
        .expect("private heartbeat");
    let displaced = fixture.join("displaced-heartbeat-repo");
    std::fs::rename(&heartbeat_repo, &displaced).expect("rename heartbeat repository");
    std::os::unix::fs::symlink(&displaced, &heartbeat_repo).expect("replace repository with link");
    assert!(!recover("42").status.success());
    assert_eq!(claim_ref_message(&repo, 42), claimed);
    std::fs::remove_file(&heartbeat_repo).expect("remove repository link");
    std::fs::rename(&displaced, &heartbeat_repo).expect("restore heartbeat repository");

    let output = recover("42");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"recovered\":true"));
    assert!(!heartbeat.exists(), "expired heartbeat remained live");
    let retained = std::fs::read_dir(&handoff)
        .expect("heartbeat handoff")
        .filter_map(Result::ok)
        .find(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .expect("retained heartbeat");
    assert_eq!(
        retained
            .metadata()
            .expect("retained metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(claim_ref_message(&repo, 42).contains("\"state\":\"available\""));

    let prior_generation_document = |repo: &str, issue: u64, ts: u64, nonce: &str| {
        format!(
            "{{\"issue\":\"{issue}\",\"branch\":\"feat/test\",\"step\":\"claimed\",\"ts\":{ts},\"ttl_seconds\":1,\"pid\":2147483647,\"nonce\":\"{nonce}\",\"host\":{:?},\"boot_id\":{:?},\"process_start\":\"1\",\"pr\":\"\",\"repo\":\"{repo}\",\"worker_id\":\"worker-old\",\"claim_id\":\"claim-old\"}}\n",
            host.trim(),
            boot.trim()
        )
    };
    let successor_heartbeat = heartbeat_repo.join("44.json");
    std::fs::write(
        &successor_heartbeat,
        prior_generation_document(
            "testorg/testrepo",
            44,
            1,
            "ec8b91f0af46c8d2aa2e0737f0fd9d3c3808e5ce539d0b26f1c2be2cdcf901e1",
        ),
    )
    .expect("expired prior-generation heartbeat");
    std::fs::set_permissions(&successor_heartbeat, std::fs::Permissions::from_mode(0o600))
        .expect("private prior-generation heartbeat");

    let successor = recover("44");

    assert!(successor.status.success());
    assert!(String::from_utf8_lossy(&successor.stdout).contains("\"recovered\":true"));
    assert!(!successor_heartbeat.exists());
    assert!(std::fs::read_dir(&handoff)
        .expect("prior-generation handoff")
        .filter_map(Result::ok)
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .any(|document| document.contains("\"issue\":\"44\"")
            && document.contains("\"claim_id\":\"claim-old\"")));
    let recovered = claim_ref_message(&repo, 44);
    assert!(recovered.contains("\"state\":\"available\""));
    assert!(recovered.contains("\"step\":\"stale_startup_recovered\""));

    let fresh_heartbeat = heartbeat_repo.join("45.json");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("wall clock")
        .as_secs();
    std::fs::write(
        &fresh_heartbeat,
        bind_to_current_process(prior_generation_document(
            "testorg/testrepo",
            45,
            now,
            "200ba5dde87a479bccd82ffab1dcc64d59b63e68d0234682c133873593363a67",
        )),
    )
    .expect("fresh prior-generation heartbeat");
    std::fs::set_permissions(&fresh_heartbeat, std::fs::Permissions::from_mode(0o600))
        .expect("private fresh heartbeat");
    let fresh_claim = claim_ref_message(&repo, 45);
    let fresh = recover("45");
    assert!(String::from_utf8_lossy(&fresh.stdout).contains("\"recovered\":false"));
    assert_eq!(claim_ref_message(&repo, 45), fresh_claim);
    assert!(fresh_heartbeat.exists());

    let cross_repo_heartbeat = heartbeat_repo.join("46.json");
    std::fs::write(
        &cross_repo_heartbeat,
        prior_generation_document(
            "otherorg/testrepo",
            46,
            1,
            "da6c29ccd42ddf083f065bfc437879d61e4cda55f4e00f0f7f26de8e2c7807dd",
        ),
    )
    .expect("cross-repository heartbeat");
    std::fs::set_permissions(
        &cross_repo_heartbeat,
        std::fs::Permissions::from_mode(0o600),
    )
    .expect("private cross-repository heartbeat");
    let cross_repo_claim = claim_ref_message(&repo, 46);
    let cross_repo = recover("46");
    assert!(String::from_utf8_lossy(&cross_repo.stdout).contains("\"recovered\":false"));
    assert_eq!(claim_ref_message(&repo, 46), cross_repo_claim);
    assert!(cross_repo_heartbeat.exists());

    let interrupted_digest = "1b9806766ed75d84434219eb6cb4e07fa0a8754f9537b060a2f3e097fc690105";
    let pending = handoff.join(format!("pending-47-{interrupted_digest}.receipt"));
    let completed = handoff.join(format!("completed-47-{interrupted_digest}.receipt"));
    let archive = handoff.join(format!("completed-47-{interrupted_digest}.json"));
    std::fs::write(&pending, b"").expect("pending prior-generation receipt");
    std::fs::set_permissions(&pending, std::fs::Permissions::from_mode(0o600))
        .expect("private pending receipt");
    std::fs::write(
        &archive,
        prior_generation_document(
            "testorg/testrepo",
            47,
            1,
            "f1837a09a0d35a8239ed726cccf717868ee651f98dd97f1337b0a986796baf4f",
        ),
    )
    .expect("retained prior-generation heartbeat");
    std::fs::set_permissions(&archive, std::fs::Permissions::from_mode(0o600))
        .expect("private retained heartbeat");

    let interrupted = recover("47");

    assert!(interrupted.status.success());
    assert!(String::from_utf8_lossy(&interrupted.stdout).contains("\"recovered\":true"));
    assert!(!pending.exists(), "pending receipt was not retired");
    assert!(completed.exists(), "completed receipt was not published");
    let resumed = claim_ref_message(&repo, 47);
    assert!(resumed.contains("\"state\":\"available\""));
    assert!(resumed.contains("\"step\":\"stale_startup_recovered\""));

    let publishing_digest = "21db6c26883b61ad67d2efa3bef926dc9e5a15892fc04bfddf13956e574f9238";
    let old_receipt = handoff.join(format!("completed-48-{publishing_digest}.receipt"));
    let old_archive = handoff.join(format!("completed-48-{publishing_digest}.json"));
    std::fs::write(&old_receipt, b"").expect("completed old-generation receipt");
    std::fs::set_permissions(&old_receipt, std::fs::Permissions::from_mode(0o600))
        .expect("private completed receipt");
    std::fs::write(
        &old_archive,
        prior_generation_document(
            "testorg/testrepo",
            48,
            1,
            "95efb38c00bc3b7d331c9c9000a5a2617cb28204ea933c9ddb907928a566b707",
        ),
    )
    .expect("retained old-generation heartbeat");
    std::fs::set_permissions(&old_archive, std::fs::Permissions::from_mode(0o600))
        .expect("private retained heartbeat");
    let sessions = heartbeat_repo.join("sessions");
    std::fs::create_dir(&sessions).expect("session heartbeat directory");
    std::fs::set_permissions(&sessions, std::fs::Permissions::from_mode(0o700))
        .expect("private session directory");
    let session = sessions.join("73657373696f6e2d63757272656e74.json");
    std::fs::write(&session, format!("{{\"issue\":\"48\",\"branch\":\"feat/test\",\"step\":\"claimed\",\"ts\":{now},\"ttl_seconds\":10800,\"pid\":{},\"nonce\":\"d7289361137ba9d5556f9e1c9ddb9367a7cb4768e6b9dcb48a8c0dcc4273e1d2\",\"host\":{:?},\"boot_id\":{:?},\"process_start\":\"{process_start}\",\"pr\":\"\",\"repo\":\"testorg/testrepo\",\"worker_id\":\"worker-new\",\"claim_id\":\"claim-new\",\"session_id\":\"session-current\"}}\n", std::process::id(), host.trim(), boot.trim())).expect("current session heartbeat");
    std::fs::set_permissions(&session, std::fs::Permissions::from_mode(0o600))
        .expect("private session heartbeat");
    let publishing_claim = claim_ref_message(&repo, 48);
    let calls = std::fs::read_to_string(&log).expect("gh calls before publishing recovery");

    let publishing = recover("48");

    assert!(String::from_utf8_lossy(&publishing.stdout).contains("\"recovered\":false"));
    assert_eq!(claim_ref_message(&repo, 48), publishing_claim);
    assert!(session.exists());
    assert_eq!(
        std::fs::read_to_string(&log).expect("gh calls after recovery"),
        calls
    );
}

#[test]
fn stale_startup_recovery_retries_a_prepared_label_transition() {
    let fixture = temp_dir("autospec-claim-recover-prepared-ref");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    let comments = fixture.join("comments.json");
    let repo = claim_git_repo(&fixture);
    transition_claim_ref(
        &repo,
        &RunStateRecord::new(
            "testorg/testrepo",
            42,
            "worker-a",
            "available",
            "",
            "",
            "stale_startup_recovered",
            Vec::new(),
            "2000-01-01T00:00:00Z",
            "2000-01-01T00:00:00Z",
            1,
        )
        .with_claim_id("claim-a"),
    );
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    std::fs::write(&comments, "[]\n").expect("comments fixture");
    write_executable(
        &bin.join("gh"),
        r#"#!/bin/sh
set -eu
printf '%s\n' "$@" >> "$AUTOSPEC_CLAIM_LOG"
if [ "$1" = api ] && [ "$2" = repos/testorg/testrepo/issues/42/comments ]; then
  cat "$AUTOSPEC_CLAIM_COMMENTS"
  exit 0
fi
if { [ "$1" = issue ] && [ "$2" = comment ]; } || { [ "$1" = issue ] && [ "$2" = edit ]; }; then
  exit 0
fi
exit 17
"#,
    );

    let output = autospec()
        .args([
            "claim",
            "state",
            "recover-stale-startup",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--timeout-seconds",
            "300",
        ])
        .current_dir(&repo)
        .env(
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            fixture.join("claim-remote.git"),
        )
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_COMMENTS", &comments)
        .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0")
        .output()
        .expect("autospec prepared stale startup recovery starts");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"recovered\":true"));
    let calls = std::fs::read_to_string(log).expect("gh call log");
    assert!(calls.contains("issue\nedit\n42"));
    assert!(claim_ref_message(&repo, 42).contains("\"state\":\"available\""));
}

#[test]
fn clear_releases_exact_claim_generation_without_deleting_ref() {
    let fixture = temp_dir("autospec-claim-clear-exact-generation");
    let bin = fixture.join("bin");
    let comments = fixture.join("comments.json");
    let repo = claim_git_repo(&fixture);
    transition_claim_ref(
        &repo,
        &RunStateRecord::new(
            "testorg/testrepo",
            42,
            "worker-a",
            "claimed",
            "feat/test",
            "",
            "claimed",
            Vec::new(),
            "2999-07-25T00:00:00Z",
            "2999-07-25T00:00:00Z",
            10_800,
        )
        .with_claim_id("claim-a"),
    );
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    std::fs::write(&comments, "[]\n").expect("comments fixture");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nset -eu\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then cat \"$AUTOSPEC_CLAIM_COMMENTS\"; exit 0; fi\nif [ \"$1\" = issue ] && [ \"$2\" = comment ]; then exit 0; fi\nexit 17\n",
    );

    let output = autospec()
        .args([
            "claim",
            "state",
            "clear",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            "worker-a",
            "--branch",
            "feat/test",
            "--claim-id",
            "claim-a",
        ])
        .current_dir(&repo)
        .env(
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            fixture.join("claim-remote.git"),
        )
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_COMMENTS", &comments)
        .output()
        .expect("autospec claim state clear starts");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let message = claim_ref_message(&repo, 42);
    assert!(message.contains("\"state\":\"released\""));
    assert!(message.contains("\"step\":\"cleared\""));
    assert!(message.contains("\"claim_id\":\"claim-a\""));
}

#[test]
fn clear_wrong_identity_leaves_authoritative_ref_unchanged() {
    for (label, worker, branch, claim_id) in [
        ("worker", "worker-b", "feat/test", "claim-a"),
        ("branch", "worker-a", "feat/other", "claim-a"),
        ("claim", "worker-a", "feat/test", "claim-b"),
    ] {
        let fixture = temp_dir(&format!("autospec-claim-clear-wrong-{label}"));
        let repo = claim_git_repo(&fixture);
        transition_claim_ref(
            &repo,
            &RunStateRecord::new(
                "testorg/testrepo",
                42,
                "worker-a",
                "claimed",
                "feat/test",
                "",
                "claimed",
                Vec::new(),
                "2999-07-25T00:00:00Z",
                "2999-07-25T00:00:00Z",
                10_800,
            )
            .with_claim_id("claim-a"),
        );
        let original_oid = claim_ref_oid(&repo, 42);

        let output = autospec()
            .args([
                "claim",
                "state",
                "clear",
                "--issue",
                "42",
                "--repo",
                "testorg/testrepo",
                "--worker-id",
                worker,
                "--branch",
                branch,
                "--claim-id",
                claim_id,
            ])
            .current_dir(&repo)
            .env(
                "AUTOSPEC_CLAIM_GIT_REMOTE",
                fixture.join("claim-remote.git"),
            )
            .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
            .output()
            .expect("autospec claim state clear starts");

        assert!(!output.status.success(), "{label}");
        assert_eq!(claim_ref_oid(&repo, 42), original_oid, "{label}");
        let message = claim_ref_message(&repo, 42);
        assert!(message.contains("\"state\":\"claimed\""), "{label}");
        assert!(message.contains("\"claim_id\":\"claim-a\""), "{label}");
    }
}

#[test]
fn claim_state_reconcile_records_a_linked_pr_before_posting_one_handoff_blocker() {
    let fixture = temp_dir("autospec-claim-state-reconcile");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    let comments = fixture.join("comments.json");
    let repo = claim_git_repo(&fixture);
    transition_claim_ref(
        &repo,
        &RunStateRecord::new(
            "testorg/testrepo",
            42,
            "worker-a",
            "claimed",
            "feat/test",
            "",
            "claimed",
            Vec::new(),
            "2026-07-14T00:00:00Z",
            "2026-07-14T00:00:00Z",
            10_800,
        )
        .with_claim_id("claim-a"),
    );
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then cat \"$AUTOSPEC_CLAIM_COMMENTS\"; exit 0; fi\nif [ \"$1\" = pr ] && [ \"$2\" = list ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_PRS\"; exit 0; fi\nif [ \"$1\" = pr ] && [ \"$2\" = checks ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_CHECKS\"; exit 0; fi\nif [ \"$1\" = issue ] && [ \"$2\" = comment ]; then\n  body=''; shift 2\n  while [ \"$#\" -gt 0 ]; do case \"$1\" in --body) body=\"$2\"; shift 2 ;; *) shift ;; esac; done\n  jq --arg body \"$body\" '. + [{id: ((map(.id)|max) + 1),updated_at:\"2030-01-01T00:00:00Z\",body:$body}]' \"$AUTOSPEC_CLAIM_COMMENTS\" > \"$AUTOSPEC_CLAIM_COMMENTS.tmp\"\n  mv \"$AUTOSPEC_CLAIM_COMMENTS.tmp\" \"$AUTOSPEC_CLAIM_COMMENTS\"\n  exit 0\nfi\nexit 17\n",
    );
    std::fs::write(
        &comments,
        r#"[{"id":100,"updated_at":"2026-07-14T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-a\",\"state\":\"claimed\",\"branch\":\"feat/test\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2026-07-14T00:00:00Z\",\"updated_at\":\"2026-07-14T00:00:00Z\",\"ttl_seconds\":10800}\n<!-- autospec-run-state:end -->"},{"id":101,"updated_at":"2026-07-14T00:01:00Z","body":"<!-- autospec-executor-result:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-a\",\"branch\":\"feat/test\",\"outcome\":\"succeeded\",\"pr\":75,\"step\":\"executor_succeeded\",\"receipt_id\":\"result-75\",\"claim_id\":\"claim-a\",\"commit\":\"7575757575757575757575757575757575757575\",\"premerge_receipt\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}\n<!-- autospec-executor-result:end -->"}]"#,
    )
    .expect("comments fixture");
    let pull_requests = r#"[
      {"number":77,"body":"Fixes #42\n\n## Closeout report\n\n## Closeout report","headRefName":"feat/other","headRefOid":"7777777777777777777777777777777777777777","isDraft":false,"baseRefName":"main"},
      {"number":75,"body":"Closes #42\n\n## Closeout report\n\n**Result** shipped.","headRefName":"feat/test","headRefOid":"7575757575757575757575757575757575757575","isDraft":false,"baseRefName":"main"}
    ]"#;

    let output = autospec()
        .args([
            "claim",
            "state",
            "reconcile-linked-pr",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            "worker-a",
            "--branch",
            "feat/test",
            "--claim-id",
            "claim-a",
        ])
        .env(
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            fixture.join("claim-remote.git"),
        )
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_COMMENTS", &comments)
        .env("AUTOSPEC_CLAIM_PRS", pull_requests)
        .env(
            "AUTOSPEC_CLAIM_CHECKS",
            r#"[{"name":"CI","state":"SUCCESS"}]"#,
        )
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0")
        .output()
        .expect("autospec claim state reconcile starts");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"reconciled\":true"));
    assert!(stdout.contains("\"pr\":\"75\""));
    let calls = std::fs::read_to_string(log).expect("gh call log");
    assert_eq!(calls.matches("issue\ncomment\n42").count(), 2);
    assert!(!calls.contains("\nPATCH\n"));
}

#[test]
fn claim_acquire_refuses_an_unreviewed_issue_before_label_mutation() {
    let fixture = temp_dir("autospec-claim-acquire-safety");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = issue ] && [ \"$2\" = view ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_ISSUE\"; fi\n",
    );
    let issue = r###"{"labels":["auto-implement"],"title":"Add Rust claim","body":"## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->","author":"agent"}"###;

    let output = autospec()
        .args([
            "claim",
            "acquire",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            "worker-a",
            "--branch",
            "feat/test",
        ])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_ISSUE", issue)
        .output()
        .expect("autospec claim acquire starts");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"reason\":\"safety_gate_failed\""));
    assert!(!std::fs::read_to_string(log)
        .expect("gh log")
        .contains("issue\nedit"));
}

#[test]
fn claim_acquire_writes_startup_evidence_then_wins_the_initial_cas_comment() {
    let fixture = temp_dir("autospec-claim-acquire");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    let comments = fixture.join("comments.json");
    let mode = fixture.join("labels.mode");
    let heartbeats = fixture.join("heartbeats");
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    std::fs::write(&comments, "[]\n").expect("comments fixture");
    std::fs::write(&mode, "ready\n").expect("label mode fixture");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = issue ] && [ \"$2\" = view ]; then\n  if [ \"$(cat \"$AUTOSPEC_CLAIM_MODE\")\" = active ]; then labels='[\"in-progress-by-bot\",\"safety:reviewed\"]'; else labels='[\"auto-implement\",\"safety:reviewed\"]'; fi\n  jq -n --argjson labels \"$labels\" --arg body \"$AUTOSPEC_CLAIM_ISSUE_BODY\" '{labels:$labels,title:\"Add Rust claim\",body:$body,author:\"agent\"}'\n  exit 0\nfi\nif [ \"$1\" = label ] && [ \"$2\" = create ]; then exit 0; fi\nif [ \"$1\" = issue ] && [ \"$2\" = edit ]; then if [ \"$(cat \"$AUTOSPEC_CLAIM_MODE\")\" = ready ]; then printf retry > \"$AUTOSPEC_CLAIM_MODE\"; exit 23; fi; printf active > \"$AUTOSPEC_CLAIM_MODE\"; exit 0; fi\nif [ \"$1\" = issue ] && [ \"$2\" = comment ]; then\n  body=''; shift 2\n  while [ \"$#\" -gt 0 ]; do case \"$1\" in --body) body=\"$2\"; shift 2 ;; *) shift ;; esac; done\n  jq --arg body \"$body\" '. + [{id:100,updated_at:\"2026-07-14T00:00:00Z\",body:$body}]' \"$AUTOSPEC_CLAIM_COMMENTS\" > \"$AUTOSPEC_CLAIM_COMMENTS.tmp\"\n  mv \"$AUTOSPEC_CLAIM_COMMENTS.tmp\" \"$AUTOSPEC_CLAIM_COMMENTS\"\n  exit 0\nfi\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then cat \"$AUTOSPEC_CLAIM_COMMENTS\"; exit 0; fi\nexit 0\n",
    );
    let body = "## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n## Goal\nAdd the Rust implementation.";

    let mut acquire = autospec();
    acquire
        .args([
            "claim",
            "acquire",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            "worker-a",
            "--branch",
            "feat/test",
            "--session-id",
            "session-real-7",
        ])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_COMMENTS", &comments)
        .env("AUTOSPEC_CLAIM_MODE", &mode)
        .env("AUTOSPEC_CLAIM_ISSUE_BODY", body)
        .env("AUTOSPEC_HEARTBEAT_DIR", &heartbeats)
        .env("AUTOSPEC_GH_API_RETRIES", "1")
        .env("AUTOSPEC_CLAIM_CONFIRM_READS", "1")
        .env("AUTOSPEC_CLAIM_SETTLE_MILLIS", "0");
    let first = acquire.output().expect("autospec claim acquire starts");
    assert_eq!(first.status.code(), Some(2));
    let ready = std::fs::read_to_string(&comments).unwrap();
    assert!(ready.contains("heartbeat-ready:"));
    let output = acquire.output().expect("autospec claim retry starts");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"claimed\":true"));
    assert!(stdout.contains("\"claim_id\":\"claim-"));
    assert!(stdout.contains("\"session_id\":\"session-real-7\""));
    assert!(heartbeats.join("o7_testorg_r8_testrepo/42.json").exists());
    let heartbeat = std::fs::read_to_string(heartbeats.join("o7_testorg_r8_testrepo/42.json"))
        .expect("claim heartbeat");
    assert!(heartbeat.contains("\"worker_id\":\"worker-a\""));
    assert!(heartbeat.contains("\"claim_id\":\"claim-"));
    assert!(heartbeat.contains("\"session_id\":\"session-real-7\""));
    let claim_id = heartbeat.split_once("\"claim_id\":\"").unwrap().1;
    let claim_id = claim_id.split_once('"').unwrap().0;
    assert!(stdout.contains(&format!("\"claim_id\":\"{claim_id}\"")));
    assert!(heartbeats
        .join("o7_testorg_r8_testrepo/sessions/73657373696f6e2d7265616c2d37.json")
        .exists());
    assert!(std::fs::read_to_string(&comments)
        .unwrap()
        .contains("\\\"step\\\":\\\"claimed\\\""));
    let calls = std::fs::read_to_string(log).expect("gh call log");
    let label_edit = calls.find("issue\nedit\n42").expect("label edit");
    let create_comment = calls.find("issue\ncomment\n42").expect("claim comment");
    assert!(create_comment < label_edit);
}

#[test]
fn claim_acquire_prelink_failure_keeps_pending_ref() {
    let fixture = temp_dir("autospec-claim-acquire-prelink");
    let bin = fixture.join("bin");
    let repo = claim_git_repo(&fixture);
    let heartbeats = fixture.join("heartbeats");
    std::fs::create_dir(&bin).unwrap();
    std::fs::write(&heartbeats, b"not-a-directory").unwrap();
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nif [ \"$1 $2\" = 'issue view' ]; then printf '%s\\n' \"$GH_ISSUE\"; elif [ \"$1\" = api ]; then printf '[]\\n'; fi\n",
    );
    let issue = r###"{"labels":["auto-implement","safety:reviewed"],"title":"claim","body":"## Safety review\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->","author":"agent"}"###;
    let output = Command::new("sh")
        .arg("-c")
        .arg("\"$AUTOSPEC_BIN\" claim acquire --issue 42 --repo testorg/testrepo --worker-id worker-a --branch feat/test --session-id session-a")
        .current_dir(&repo)
        .env(
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            fixture.join("claim-remote.git"),
        )
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
        .env("AUTOSPEC_HEARTBEAT_DIR", &heartbeats)
        .env("AUTOSPEC_BIN", env!("CARGO_BIN_EXE_autospec"))
        .env("PATH", path_with(&bin))
        .env("GH_ISSUE", issue)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let record = claim_ref_message(&repo, 42);
    assert!(record.contains("\"state\":\"claimed\""), "{record}");
    assert!(record.contains("\"step\":\"heartbeat-pending:"), "{record}");
}

#[test]
fn startup_heartbeat_claim_lifecycle() {
    let fixture = temp_dir("autospec-claim-acquire-race");
    let bin = fixture.join("bin");
    let repo = claim_git_repo(&fixture);
    let heartbeats_a = fixture.join("heartbeats-a");
    let heartbeats_b = fixture.join("heartbeats-b");
    let sessions = heartbeats_a.join("o7_testorg_r8_testrepo/sessions");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    let pending = RunStateRecord::parse_json(r#"{"schema":1,"repo":"testorg/testrepo","issue":42,"worker_id":"worker-a","state":"claimed","branch":"feat/a","pr":"","step":"heartbeat-pending:73657373696f6e2d61","paths":[],"claimed_at":"2999-07-30T00:00:00Z","updated_at":"2999-07-30T00:00:00Z","ttl_seconds":10800,"claim_id":"claim-a"}"#).unwrap();
    transition_claim_ref(&repo, &pending);
    let binding = sessions.join("73657373696f6e2d636f6e666c696374.json");
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap();
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
    let process_start = current_process_start();
    std::fs::write(&binding, format!("{{\"issue\":\"43\",\"branch\":\"feat/c\",\"step\":\"claimed\",\"ts\":1,\"ttl_seconds\":10800,\"pid\":{},\"nonce\":\"27ba109f66aa73f03cb40a405356da986874fb0ef2a279127821fdc4dc319b0c\",\"host\":{:?},\"boot_id\":{:?},\"process_start\":\"{process_start}\",\"pr\":\"\",\"repo\":\"testorg/testrepo\",\"worker_id\":\"worker-c\",\"claim_id\":\"claim-old\",\"session_id\":\"session-conflict\"}}\n", std::process::id(), host.trim(), boot.trim())).unwrap();
    std::fs::set_permissions(&binding, std::fs::Permissions::from_mode(0o600)).unwrap();
    let binding_before = std::fs::read(&binding).unwrap();
    let stale = RunStateRecord::parse_json(r#"{"schema":1,"repo":"testorg/testrepo","issue":43,"worker_id":"worker-c","state":"claimed","branch":"feat/c","pr":"","step":"heartbeat-publishing:73657373696f6e2d636f6e666c696374:attempt-old","paths":[],"claimed_at":"2000-07-30T00:00:00Z","updated_at":"2000-07-30T00:00:00Z","ttl_seconds":1,"claim_id":"claim-old"}"#).unwrap();
    transition_claim_ref(&repo, &stale);
    let fresh = RunStateRecord::parse_json(r#"{"schema":1,"repo":"testorg/testrepo","issue":44,"worker_id":"worker-d","state":"claimed","branch":"feat/d","pr":"","step":"heartbeat-publishing:73657373696f6e2d6672657368:attempt-live","paths":[],"claimed_at":"2999-07-30T00:00:00Z","updated_at":"2999-07-30T00:00:00Z","ttl_seconds":10800,"claim_id":"claim-live"}"#).unwrap();
    transition_claim_ref(&repo, &fresh);
    let fresh_oid = claim_ref_oid(&repo, 44);
    write_executable(
        &bin.join("git-wrapper"),
        "#!/bin/sh\n/usr/bin/git \"$@\"; status=$?; case \" $* \" in *' push '*) if [ $status = 0 ] && [ ! -e \"$PAUSED\" ]; then : > \"$PAUSED\"; while [ ! -e \"$RELEASE\" ]; do sleep .01; done; fi;; esac; exit $status\n",
    );
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nif [ \"$1 $2\" = 'issue view' ]; then printf '%s\\n' \"$GH_ISSUE\"; elif [ \"$1\" = api ]; then printf '[]\\n'; elif [ \"$1 $2\" = 'issue edit' ]; then exit 23; fi\n",
    );
    let issue = r###"{"labels":["auto-implement","safety:reviewed"],"title":"claim","body":"## Safety review\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->","author":"agent"}"###;
    let lifecycle = Command::new("sh")
        .arg("-c")
        .arg("AUTOSPEC_HEARTBEAT_DIR=\"$HEARTBEATS_A\" AUTOSPEC_CLAIM_GIT_BIN=\"$GIT_WRAPPER\" \"$AUTOSPEC_BIN\" claim acquire --issue 42 --repo testorg/testrepo --worker-id worker-a --branch feat/a --session-id session-a & a=$!; i=0; while [ ! -e \"$PAUSED\" ]; do i=$((i+1)); [ $i -lt 500 ] || exit 99; sleep .01; done; AUTOSPEC_HEARTBEAT_DIR=\"$HEARTBEATS_B\" \"$AUTOSPEC_BIN\" claim acquire --issue 42 --repo testorg/testrepo --worker-id worker-a --branch feat/a --session-id session-a || true; : > \"$RELEASE\"; wait $a || true; before=$(/usr/bin/git --git-dir \"$AUTOSPEC_CLAIM_GIT_REMOTE\" rev-parse refs/autospec/claims/issue-43); AUTOSPEC_HEARTBEAT_DIR=\"$HEARTBEATS_A\" \"$AUTOSPEC_BIN\" claim state recover-stale-startup --issue 43 --repo testorg/testrepo --timeout-seconds 1 || true; after=$(/usr/bin/git --git-dir \"$AUTOSPEC_CLAIM_GIT_REMOTE\" rev-parse refs/autospec/claims/issue-43); [ \"$before\" = \"$after\" ] || exit 98; for identity in 'worker-x feat/c session-conflict' 'worker-c feat/x session-conflict' 'worker-c feat/c session-x'; do set -- $identity; AUTOSPEC_HEARTBEAT_DIR=\"$HEARTBEATS_A\" \"$AUTOSPEC_BIN\" claim acquire --issue 43 --repo testorg/testrepo --worker-id \"$1\" --branch \"$2\" --session-id \"$3\" || true; done; AUTOSPEC_HEARTBEAT_DIR=\"$HEARTBEATS_A\" \"$AUTOSPEC_BIN\" claim acquire --issue 43 --repo testorg/testrepo --worker-id worker-c --branch feat/c --session-id session-conflict || true; AUTOSPEC_HEARTBEAT_DIR=\"$HEARTBEATS_A\" \"$AUTOSPEC_BIN\" claim acquire --issue 44 --repo testorg/testrepo --worker-id worker-d --branch feat/d --session-id session-fresh || true")
        .current_dir(&repo)
        .env("AUTOSPEC_BIN", env!("CARGO_BIN_EXE_autospec"))
        .env("AUTOSPEC_CLAIM_GIT_REMOTE", fixture.join("claim-remote.git"))
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
        .env("PATH", path_with(&bin))
        .env("GIT_WRAPPER", bin.join("git-wrapper"))
        .env("HEARTBEATS_A", &heartbeats_a)
        .env("HEARTBEATS_B", &heartbeats_b)
        .env("PAUSED", fixture.join("paused"))
        .env("RELEASE", fixture.join("release"))
        .env("GH_ISSUE", issue)
        .output()
        .unwrap();
    assert!(
        lifecycle.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&lifecycle.stdout),
        String::from_utf8_lossy(&lifecycle.stderr)
    );
    assert!(heartbeats_a.join("o7_testorg_r8_testrepo/42.json").exists());
    assert!(!heartbeats_b.join("o7_testorg_r8_testrepo/42.json").exists());
    assert!(!heartbeats_b
        .join("o7_testorg_r8_testrepo/sessions/73657373696f6e2d61.json")
        .exists());
    let ready = claim_ref_message(&repo, 43);
    assert!(ready.contains("\"step\":\"heartbeat-pending:"), "{ready}");
    assert_eq!(std::fs::read(&binding).unwrap(), binding_before);
    assert!(ready.contains("\"claim_id\":\"claim-old\""));
    assert_eq!(claim_ref_oid(&repo, 44), fresh_oid);
}

#[test]
fn fresh_claim_blocks_same_worker_reacquire_even_with_wait_failure_comment() {
    let fixture = temp_dir("autospec-claim-acquire-same-owner");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    let comments = fixture.join("comments.json");
    let heartbeats = fixture.join("heartbeats");
    let repo = claim_git_repo(&fixture);
    let record = RunStateRecord::new(
        "testorg/testrepo",
        42,
        "worker-a",
        "claimed",
        "feat/test",
        "",
        "claimed",
        Vec::new(),
        "2999-07-25T00:00:00Z",
        "2999-07-25T00:00:00Z",
        10_800,
    )
    .with_claim_id("claim-a");
    transition_claim_ref(&repo, &record);
    let original_oid = claim_ref_oid(&repo, 42);
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    std::fs::write(
        &comments,
        r#"[{"id":101,"updated_at":"2999-07-25T00:00:01Z","body":"<!-- autospec-executor-result:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-a\",\"branch\":\"feat/test\",\"outcome\":\"failed\",\"pr\":null,\"step\":\"implementer_wait_failed\",\"receipt_id\":\"implementer-wait-failed:claim-a:session-a\",\"claim_id\":null,\"commit\":null,\"premerge_receipt\":null}\n<!-- autospec-executor-result:end -->"}]"#,
    )
    .expect("wait-failure audit comment");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = issue ] && [ \"$2\" = view ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_ISSUE\"; exit 0; fi\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then cat \"$AUTOSPEC_CLAIM_COMMENTS\"; exit 0; fi\nexit 0\n",
    );
    let issue = r###"{"labels":["auto-implement","safety:reviewed"],"title":"Add Rust claim","body":"## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->","author":"agent"}"###;

    let output = autospec()
        .args([
            "claim",
            "acquire",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            "worker-a",
            "--branch",
            "feat/test",
        ])
        .current_dir(&repo)
        .env(
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            fixture.join("claim-remote.git"),
        )
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_ISSUE", issue)
        .env("AUTOSPEC_CLAIM_COMMENTS", &comments)
        .env("AUTOSPEC_HEARTBEAT_DIR", &heartbeats)
        .output()
        .expect("autospec claim acquire starts");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"reason\":\"claim_lost\""));
    assert_eq!(claim_ref_oid(&repo, 42), original_oid);
    let calls = std::fs::read_to_string(log).expect("gh call log");
    assert!(!calls.contains("issue\ncomment"));
    assert!(!calls.contains("issue\nedit"));
}

#[test]
fn wait_failure_recovery_advances_claim_ref_before_side_effects() {
    let fixture = temp_dir("autospec-wait-failure-ref-first");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    let comments = fixture.join("comments.json");
    let repo = claim_git_repo(&fixture);
    transition_claim_ref(
        &repo,
        &RunStateRecord::new(
            "testorg/testrepo",
            42,
            "worker-a",
            "claimed",
            "feat/test",
            "",
            "claimed",
            Vec::new(),
            "2999-07-25T00:00:00Z",
            "2999-07-25T00:00:00Z",
            10_800,
        )
        .with_claim_id("claim-a"),
    );
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    std::fs::write(&comments, "[]\n").expect("comments fixture");
    write_executable(
        &bin.join("gh"),
        r#"#!/bin/sh
set -eu
printf '%s\n' "$@" >> "$AUTOSPEC_CLAIM_LOG"
if [ "$1" = api ] && [ "$2" = repos/testorg/testrepo/issues/42/comments ]; then
  cat "$AUTOSPEC_CLAIM_COMMENTS"
  exit 0
fi
if { [ "$1" = issue ] && [ "$2" = comment ]; } || { [ "$1" = issue ] && [ "$2" = edit ]; }; then
  git --git-dir "$AUTOSPEC_CLAIM_REMOTE" show -s --format=%B refs/autospec/claims/issue-42 |
    grep -Fq '"state":"available"' || exit 45
fi
if [ "$1" = issue ] && [ "$2" = comment ]; then
  body=''
  shift 2
  while [ "$#" -gt 0 ]; do
    case "$1" in --body) body="$2"; shift 2;; *) shift;; esac
  done
  jq --arg body "$body" '. + [{id:101,updated_at:"2999-07-25T00:00:01Z",body:$body}]' \
    "$AUTOSPEC_CLAIM_COMMENTS" > "$AUTOSPEC_CLAIM_COMMENTS.tmp"
  mv "$AUTOSPEC_CLAIM_COMMENTS.tmp" "$AUTOSPEC_CLAIM_COMMENTS"
  exit 0
fi
if [ "$1" = issue ] && [ "$2" = edit ]; then exit 0; fi
exit 17
"#,
    );

    let output = autospec()
        .args([
            "autonomous",
            "implementer-wait-failed",
            "--repo",
            "testorg/testrepo",
            "--issue",
            "42",
            "--worker-id",
            "worker-a",
            "--branch",
            "feat/test",
            "--claim-id",
            "claim-a",
            "--session-id",
            "session-a",
            "--diagnostic",
            "stdin closed",
        ])
        .current_dir(&repo)
        .env(
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            fixture.join("claim-remote.git"),
        )
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_COMMENTS", &comments)
        .env("AUTOSPEC_CLAIM_REMOTE", fixture.join("claim-remote.git"))
        .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0")
        .env("AUTOSPEC_GH_API_RETRIES", "1")
        .output()
        .expect("autospec wait-failure recovery starts");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let message = claim_ref_message(&repo, 42);
    assert!(message.contains("\"state\":\"available\""));
    assert!(message.contains("\"step\":\"implementer_wait_failed\""));
    assert!(message.contains("\"claim_id\":\"claim-a\""));
}

#[test]
fn claim_acquire_ignores_a_fresh_foreign_audit_comment_without_a_claim_ref() {
    let fixture = temp_dir("autospec-claim-acquire-fresh-foreign");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    let heartbeats = fixture.join("heartbeats");
    let mode = fixture.join("labels.mode");
    std::fs::write(&mode, "ready\n").expect("label mode fixture");
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = issue ] && [ \"$2\" = view ]; then\n  if [ \"$(cat \"$AUTOSPEC_CLAIM_MODE\")\" = active ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_ISSUE\" | jq '.labels=[\"in-progress-by-bot\",\"safety:reviewed\"]'; else printf '%s\\n' \"$AUTOSPEC_CLAIM_ISSUE\"; fi\nelif [ \"$1\" = issue ] && [ \"$2\" = edit ]; then printf active > \"$AUTOSPEC_CLAIM_MODE\"\nelif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_COMMENTS\"\nfi\n",
    );
    let issue = r###"{"labels":["auto-implement","safety:reviewed"],"title":"Add Rust claim","body":"## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->","author":"agent"}"###;
    let comments = r#"[{"id":100,"updated_at":"2999-07-14T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-b\",\"state\":\"claimed\",\"branch\":\"feat/other\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2999-07-14T00:00:00Z\",\"updated_at\":\"2999-07-14T00:00:00Z\",\"ttl_seconds\":1}\n<!-- autospec-run-state:end -->"}]"#;

    let output = autospec()
        .args([
            "claim",
            "acquire",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            "worker-a",
            "--branch",
            "feat/test",
        ])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_ISSUE", issue)
        .env("AUTOSPEC_CLAIM_COMMENTS", comments)
        .env("AUTOSPEC_CLAIM_MODE", &mode)
        .env("AUTOSPEC_HEARTBEAT_DIR", &heartbeats)
        .env("AUTOSPEC_CLAIM_LEASE_SECONDS", "9999999999")
        .output()
        .expect("autospec claim acquire starts");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"claimed\":true"));
    assert!(heartbeats.join("o7_testorg_r8_testrepo/42.json").exists());
}

#[test]
fn claim_acquire_reclaims_a_stale_foreign_lowest_comment_after_confirming_the_cas_owner() {
    let fixture = temp_dir("autospec-claim-acquire-stale-foreign");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    let comments = fixture.join("comments.json");
    let mode = fixture.join("labels.mode");
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    std::fs::write(
        &comments,
        r#"[{"id":100,"updated_at":"2000-01-01T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-b\",\"state\":\"claimed\",\"branch\":\"feat/other\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2000-01-01T00:00:00Z\",\"updated_at\":\"2000-01-01T00:00:00Z\",\"ttl_seconds\":1}\n<!-- autospec-run-state:end -->"}]"#,
    )
    .expect("stale comments fixture");
    std::fs::write(&mode, "ready\n").expect("label mode fixture");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = issue ] && [ \"$2\" = view ]; then\n  if [ \"$(cat \"$AUTOSPEC_CLAIM_MODE\")\" = active ]; then labels='[\"in-progress-by-bot\",\"safety:reviewed\"]'; else labels='[\"auto-implement\",\"safety:reviewed\"]'; fi\n  jq -n --argjson labels \"$labels\" --arg body \"$AUTOSPEC_CLAIM_ISSUE_BODY\" '{labels:$labels,title:\"Add Rust claim\",body:$body,author:\"agent\"}'\n  exit 0\nfi\nif [ \"$1\" = label ] && [ \"$2\" = create ]; then exit 0; fi\nif [ \"$1\" = issue ] && [ \"$2\" = edit ]; then printf active > \"$AUTOSPEC_CLAIM_MODE\"; exit 0; fi\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then cat \"$AUTOSPEC_CLAIM_COMMENTS\"; exit 0; fi\nif [ \"$1\" = issue ] && [ \"$2\" = comment ]; then\n  body=''; shift 2\n  while [ \"$#\" -gt 0 ]; do case \"$1\" in --body) body=\"$2\"; shift 2 ;; *) shift ;; esac; done\n  jq --arg body \"$body\" '. + [{id:101,updated_at:\"2026-07-14T00:00:00Z\",body:$body}]' \"$AUTOSPEC_CLAIM_COMMENTS\" > \"$AUTOSPEC_CLAIM_COMMENTS.tmp\"\n  mv \"$AUTOSPEC_CLAIM_COMMENTS.tmp\" \"$AUTOSPEC_CLAIM_COMMENTS\"\n  exit 0\nfi\nexit 0\n",
    );
    let body = "## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n## Goal\nAdd the Rust implementation.";

    let output = autospec()
        .args([
            "claim",
            "acquire",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            "worker-a",
            "--branch",
            "feat/test",
        ])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_COMMENTS", &comments)
        .env("AUTOSPEC_CLAIM_MODE", &mode)
        .env("AUTOSPEC_CLAIM_ISSUE_BODY", body)
        .env("AUTOSPEC_HEARTBEAT_DIR", fixture.join("heartbeats"))
        .env("AUTOSPEC_CLAIM_LEASE_SECONDS", "9999999999")
        .env("AUTOSPEC_CLAIM_CONFIRM_READS", "1")
        .env("AUTOSPEC_CLAIM_SETTLE_MILLIS", "0")
        .output()
        .expect("autospec claim acquire starts");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"claimed\":true"));
    assert!(std::fs::read_to_string(&comments)
        .expect("claimed comments")
        .contains("worker-a"));
    let calls = std::fs::read_to_string(log).expect("gh log");
    assert!(calls.contains("issue\ncomment\n42"));
    assert!(!calls.contains("\nPATCH\n"));
}

#[test]
fn claim_acquire_does_not_treat_a_terminal_audit_comment_as_absorbing() {
    let fixture = temp_dir("autospec-claim-acquire-terminal");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    let heartbeats = fixture.join("heartbeats");
    let mode = fixture.join("labels.mode");
    std::fs::write(&mode, "ready\n").expect("label mode fixture");
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = issue ] && [ \"$2\" = view ]; then\n  if [ \"$(cat \"$AUTOSPEC_CLAIM_MODE\")\" = active ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_ISSUE\" | jq '.labels=[\"in-progress-by-bot\",\"safety:reviewed\"]'; else printf '%s\\n' \"$AUTOSPEC_CLAIM_ISSUE\"; fi\nelif [ \"$1\" = issue ] && [ \"$2\" = edit ]; then printf active > \"$AUTOSPEC_CLAIM_MODE\"\nelif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_COMMENTS\"\nfi\n",
    );
    let issue = r###"{"labels":["auto-implement","safety:reviewed"],"title":"Add Rust claim","body":"## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->","author":"agent"}"###;
    let comments = r#"[{"id":100,"updated_at":"2026-07-14T00:00:00Z","body":"<!-- autospec-run-terminal:begin -->\n{ \"state\" : \"merged\" }\n<!-- autospec-run-terminal:end -->"}]"#;

    let output = autospec()
        .args([
            "claim",
            "acquire",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            "worker-a",
            "--branch",
            "feat/test",
        ])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_ISSUE", issue)
        .env("AUTOSPEC_CLAIM_COMMENTS", comments)
        .env("AUTOSPEC_CLAIM_MODE", &mode)
        .env("AUTOSPEC_HEARTBEAT_DIR", &heartbeats)
        .output()
        .expect("autospec claim acquire starts");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"claimed\":true"));
    assert!(heartbeats.join("o7_testorg_r8_testrepo/42.json").exists());
}

#[test]
fn claim_option_parsers_reject_duplicates_even_when_the_value_is_empty_or_default() {
    let upsert = autospec()
        .args([
            "claim",
            "state",
            "upsert",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            "worker-a",
            "--state",
            "claimed",
            "--paths",
            "[]",
            "--paths",
            "[]",
        ])
        .output()
        .expect("autospec claim state upsert starts");
    assert!(!upsert.status.success());
    assert!(
        String::from_utf8_lossy(&upsert.stderr).contains("--paths accepts exactly one path list")
    );

    let release = autospec()
        .args([
            "claim",
            "release",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--state",
            "released",
            "--state",
            "released",
        ])
        .output()
        .expect("autospec claim release starts");
    assert!(!release.status.success());
    assert!(String::from_utf8_lossy(&release.stderr).contains("--state accepts exactly one state"));
}

// Split out of this file, which is past the size ratchet: see the module for what it owns.
#[path = "claim_commands/retirement.rs"]
mod retirement;
