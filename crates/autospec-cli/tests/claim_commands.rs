use autospec_core::claim::RunStateRecord;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn autospec() -> Command {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
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
    std::fs::write(path, contents).expect("fake command");
    let mut permissions = std::fs::metadata(path)
        .expect("fake command metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("fake command permissions");
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

/// One `gh api <endpoint> [-X <method>]` invocation recovered from the call
/// log the fake `gh` script appends (one CLI argument per line, invocations
/// concatenated with no delimiter between them).
#[derive(Debug, PartialEq, Eq)]
struct GhApiCall {
    endpoint: String,
    method: Option<String>,
}

/// Parses the raw call log into the `gh api <endpoint> [-X <method>]`
/// invocations it contains. Replaces matching on raw newline-joined
/// substrings (fragile to one endpoint's logged args being a prefix/suffix
/// of another's) with a parsed, structurally-compared call list.
fn gh_api_calls(log: &str) -> Vec<GhApiCall> {
    let lines: Vec<&str> = log.lines().collect();
    let mut calls = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i] == "api" && i + 1 < lines.len() {
            let endpoint = lines[i + 1].to_string();
            let method = if lines.get(i + 2) == Some(&"-X") {
                lines.get(i + 3).map(|m| (*m).to_string())
            } else {
                None
            };
            let advance = if method.is_some() { 4 } else { 2 };
            calls.push(GhApiCall { endpoint, method });
            i += advance;
        } else {
            i += 1;
        }
    }
    calls
}

#[test]
fn claim_state_read_selects_the_lowest_marked_github_comment() {
    let fixture = temp_dir("autospec-claim-state-read");
    let bin = fixture.join("bin");
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
fn claim_state_upsert_patches_the_lowest_comment_and_deletes_higher_duplicates() {
    let fixture = temp_dir("autospec-claim-state-upsert");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_COMMENTS\"; fi\n",
    );
    let comments = r#"[
      {"id":101,"updated_at":"2026-07-14T00:01:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-b\",\"state\":\"claimed\",\"branch\":\"feat/test\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2026-07-14T00:01:00Z\",\"updated_at\":\"2026-07-14T00:01:00Z\",\"ttl_seconds\":10800}\n<!-- autospec-run-state:end -->"},
      {"id":100,"updated_at":"2026-07-14T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-a\",\"state\":\"claimed\",\"branch\":\"feat/test\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2026-07-14T00:00:00Z\",\"updated_at\":\"2026-07-14T00:00:00Z\",\"ttl_seconds\":10800}\n<!-- autospec-run-state:end -->"}
    ]"#;

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
            "--state",
            "worktree_ready",
            "--step",
            "worktree_ready",
            "--paths",
            "crates/autospec-core/src/claim/mod.rs,crates/autospec-cli/src/commands/claim.rs",
            "--ttl-seconds",
            "7200",
        ])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_COMMENTS", comments)
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
    let api_calls = gh_api_calls(&calls);
    assert!(api_calls.contains(&GhApiCall {
        endpoint: "repos/testorg/testrepo/issues/comments/100".to_string(),
        method: Some("PATCH".to_string()),
    }));
    assert!(api_calls.contains(&GhApiCall {
        endpoint: "repos/testorg/testrepo/issues/comments/101".to_string(),
        method: Some("DELETE".to_string()),
    }));
}

#[test]
fn claim_state_upsert_retries_a_transient_patch_failure() {
    let fixture = temp_dir("autospec-claim-state-retry");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    let failed_once = fixture.join("failed-once");
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_COMMENTS\"; exit 0; fi\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/comments/100 ]; then\n  if [ ! -f \"$AUTOSPEC_CLAIM_FAILED_ONCE\" ]; then touch \"$AUTOSPEC_CLAIM_FAILED_ONCE\"; exit 1; fi\n  exit 0\nfi\nexit 0\n",
    );
    let comments = r#"[{"id":100,"updated_at":"2026-07-14T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-a\",\"state\":\"claimed\",\"branch\":\"\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2026-07-14T00:00:00Z\",\"updated_at\":\"2026-07-14T00:00:00Z\",\"ttl_seconds\":10800}\n<!-- autospec-run-state:end -->"}]"#;

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
            "--state",
            "worktree_ready",
        ])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_COMMENTS", comments)
        .env("AUTOSPEC_CLAIM_FAILED_ONCE", &failed_once)
        .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0")
        .output()
        .expect("autospec claim state upsert starts");

    assert!(output.status.success());
    assert!(failed_once.exists());
    let calls = std::fs::read_to_string(log).expect("gh call log");
    assert_eq!(
        calls
            .matches("repos/testorg/testrepo/issues/comments/100")
            .count(),
        2
    );
}

#[test]
fn claim_state_clear_deletes_every_marked_comment_without_touching_unmarked_history() {
    let fixture = temp_dir("autospec-claim-state-clear");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    std::fs::create_dir_all(&bin).expect("fake bin directory");
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

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
    let calls = std::fs::read_to_string(log).expect("gh call log");
    assert!(calls.contains("repos/testorg/testrepo/issues/comments/100\n-X\nDELETE"));
    assert!(calls.contains("repos/testorg/testrepo/issues/comments/101\n-X\nDELETE"));
    assert!(!calls.contains("repos/testorg/testrepo/issues/comments/99\n-X\nDELETE"));
}

#[test]
fn claim_state_recover_stale_startup_releases_only_an_old_evidenceless_claim() {
    let fixture = temp_dir("autospec-claim-recover-stale");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_COMMENTS\"; fi\n",
    );
    let comments = r#"[{"id":100,"updated_at":"2000-01-01T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-a\",\"state\":\"claimed\",\"branch\":\"\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2000-01-01T00:00:00Z\",\"updated_at\":\"2000-01-01T00:00:00Z\",\"ttl_seconds\":10800}\n<!-- autospec-run-state:end -->"}]"#;

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
fn claim_state_reconcile_records_a_linked_pr_before_posting_one_handoff_blocker() {
    let fixture = temp_dir("autospec-claim-state-reconcile");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_COMMENTS\"; elif [ \"$1\" = pr ] && [ \"$2\" = list ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_PRS\"; fi\n",
    );
    let comments = r#"[{"id":100,"updated_at":"2026-07-14T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-a\",\"state\":\"claimed\",\"branch\":\"feat/test\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2026-07-14T00:00:00Z\",\"updated_at\":\"2026-07-14T00:00:00Z\",\"ttl_seconds\":10800}\n<!-- autospec-run-state:end -->"}]"#;
    let pull_requests = r#"[
      {"number":77,"body":"Fixes #42\n\n## Closeout report\n\n## Closeout report"},
      {"number":75,"body":"Closes #42\n\n## Closeout report\n\n**Result** shipped."}
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
        ])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_COMMENTS", comments)
        .env("AUTOSPEC_CLAIM_PRS", pull_requests)
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
    assert!(calls.contains("repos/testorg/testrepo/issues/comments/100\n-X\nPATCH"));
    assert!(calls.contains("issue\ncomment\n42\n--repo\ntestorg/testrepo\n--body"));
}

#[test]
fn claim_release_records_terminal_merge_before_removing_the_active_label() {
    let fixture = temp_dir("autospec-claim-release");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_COMMENTS\"; fi\n",
    );
    let comments = r#"[{"id":100,"updated_at":"2026-07-14T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-a\",\"state\":\"claimed\",\"branch\":\"feat/test\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2026-07-14T00:00:00Z\",\"updated_at\":\"2026-07-14T00:00:00Z\",\"ttl_seconds\":10800}\n<!-- autospec-run-state:end -->"}]"#;

    let output = autospec()
        .args([
            "claim",
            "release",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            "worker-a",
            "--state",
            "merged",
            "--branch",
            "feat/test",
            "--pr",
            "99",
        ])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_COMMENTS", comments)
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0")
        .output()
        .expect("autospec claim release starts");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"released\":true"));
    assert!(stdout.contains("\"state\":\"merged\""));
    let calls = std::fs::read_to_string(log).expect("gh call log");
    let patch = calls
        .find("repos/testorg/testrepo/issues/comments/100\n-X\nPATCH")
        .unwrap();
    let terminal = calls
        .find("issue\ncomment\n42\n--repo\ntestorg/testrepo\n--body")
        .unwrap();
    let labels = calls
        .find("issue\nedit\n42\n--repo\ntestorg/testrepo\n--remove-label\nin-progress-by-bot")
        .unwrap();
    assert!(terminal < patch && patch < labels);
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
        "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = issue ] && [ \"$2\" = view ]; then\n  if [ \"$(cat \"$AUTOSPEC_CLAIM_MODE\")\" = active ]; then labels='[\"in-progress-by-bot\",\"safety:reviewed\"]'; else labels='[\"auto-implement\",\"safety:reviewed\"]'; fi\n  jq -n --argjson labels \"$labels\" --arg body \"$AUTOSPEC_CLAIM_ISSUE_BODY\" '{labels:$labels,title:\"Add Rust claim\",body:$body,author:\"agent\"}'\n  exit 0\nfi\nif [ \"$1\" = label ] && [ \"$2\" = create ]; then exit 0; fi\nif [ \"$1\" = issue ] && [ \"$2\" = edit ]; then printf active > \"$AUTOSPEC_CLAIM_MODE\"; exit 0; fi\nif [ \"$1\" = issue ] && [ \"$2\" = comment ]; then\n  body=''; shift 2\n  while [ \"$#\" -gt 0 ]; do case \"$1\" in --body) body=\"$2\"; shift 2 ;; *) shift ;; esac; done\n  jq --arg body \"$body\" '. + [{id:100,updated_at:\"2026-07-14T00:00:00Z\",body:$body}]' \"$AUTOSPEC_CLAIM_COMMENTS\" > \"$AUTOSPEC_CLAIM_COMMENTS.tmp\"\n  mv \"$AUTOSPEC_CLAIM_COMMENTS.tmp\" \"$AUTOSPEC_CLAIM_COMMENTS\"\n  exit 0\nfi\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then cat \"$AUTOSPEC_CLAIM_COMMENTS\"; exit 0; fi\nexit 0\n",
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
        .env("AUTOSPEC_HEARTBEAT_DIR", &heartbeats)
        .env("AUTOSPEC_CLAIM_CONFIRM_READS", "1")
        .env("AUTOSPEC_CLAIM_SETTLE_MILLIS", "0")
        .output()
        .expect("autospec claim acquire starts");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"claimed\":true"));
    assert!(heartbeats.join("o7_testorg_r8_testrepo/42.json").exists());
    assert!(std::fs::read_to_string(&comments)
        .expect("claim comments")
        .contains("worker-a"));
    let calls = std::fs::read_to_string(log).expect("gh call log");
    let label_edit = calls.find("issue\nedit\n42").expect("label edit");
    let create_comment = calls.find("issue\ncomment\n42").expect("claim comment");
    assert!(label_edit < create_comment);
}

#[test]
fn claim_acquire_never_reclaims_a_fresh_foreign_lowest_comment() {
    let fixture = temp_dir("autospec-claim-acquire-fresh-foreign");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    let heartbeats = fixture.join("heartbeats");
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = issue ] && [ \"$2\" = view ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_ISSUE\"; elif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_COMMENTS\"; fi\n",
    );
    let issue = r###"{"labels":["auto-implement","safety:reviewed"],"title":"Add Rust claim","body":"## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->","author":"agent"}"###;
    let comments = r#"[{"id":100,"updated_at":"2026-07-14T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-b\",\"state\":\"claimed\",\"branch\":\"feat/other\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2026-07-14T00:00:00Z\",\"updated_at\":\"2026-07-14T00:00:00Z\",\"ttl_seconds\":10800}\n<!-- autospec-run-state:end -->"}]"#;

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
        .env("AUTOSPEC_HEARTBEAT_DIR", &heartbeats)
        .env("AUTOSPEC_CLAIM_LEASE_SECONDS", "9999999999")
        .output()
        .expect("autospec claim acquire starts");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"reason\":\"claim_lost\""));
    assert!(stdout.contains("\"observed_owner\":\"worker-b\""));
    assert!(!heartbeats.join("testorg__testrepo/42.json").exists());
    assert!(!std::fs::read_to_string(log)
        .expect("gh log")
        .contains("issue\ncomment\n42"));
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
        "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = issue ] && [ \"$2\" = view ]; then\n  if [ \"$(cat \"$AUTOSPEC_CLAIM_MODE\")\" = active ]; then labels='[\"in-progress-by-bot\",\"safety:reviewed\"]'; else labels='[\"auto-implement\",\"safety:reviewed\"]'; fi\n  jq -n --argjson labels \"$labels\" --arg body \"$AUTOSPEC_CLAIM_ISSUE_BODY\" '{labels:$labels,title:\"Add Rust claim\",body:$body,author:\"agent\"}'\n  exit 0\nfi\nif [ \"$1\" = label ] && [ \"$2\" = create ]; then exit 0; fi\nif [ \"$1\" = issue ] && [ \"$2\" = edit ]; then printf active > \"$AUTOSPEC_CLAIM_MODE\"; exit 0; fi\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then cat \"$AUTOSPEC_CLAIM_COMMENTS\"; exit 0; fi\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/comments/100 ]; then\n  body=''; shift 2\n  while [ \"$#\" -gt 0 ]; do case \"$1\" in -f) body=\"${2#body=}\"; shift 2 ;; *) shift ;; esac; done\n  jq --arg body \"$body\" '.[0].body=$body | .[0].updated_at=\"2026-07-14T00:00:00Z\"' \"$AUTOSPEC_CLAIM_COMMENTS\" > \"$AUTOSPEC_CLAIM_COMMENTS.tmp\"\n  mv \"$AUTOSPEC_CLAIM_COMMENTS.tmp\" \"$AUTOSPEC_CLAIM_COMMENTS\"\n  exit 0\nfi\nexit 0\n",
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
        .env("AUTOSPEC_CLAIM_LEASE_SECONDS", "1")
        .env("AUTOSPEC_CLAIM_CONFIRM_READS", "1")
        .env("AUTOSPEC_CLAIM_SETTLE_MILLIS", "0")
        .output()
        .expect("autospec claim acquire starts");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"claimed\":true"));
    assert!(std::fs::read_to_string(&comments)
        .expect("claimed comments")
        .contains("worker-a"));
    assert!(std::fs::read_to_string(log)
        .expect("gh log")
        .contains("repos/testorg/testrepo/issues/comments/100\n-X\nPATCH"));
}

#[test]
fn claim_acquire_refuses_a_valid_whitespace_formatted_terminal_merge() {
    let fixture = temp_dir("autospec-claim-acquire-terminal");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    let heartbeats = fixture.join("heartbeats");
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = issue ] && [ \"$2\" = view ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_ISSUE\"; elif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then printf '%s\\n' \"$AUTOSPEC_CLAIM_COMMENTS\"; fi\n",
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
        .env("AUTOSPEC_HEARTBEAT_DIR", &heartbeats)
        .output()
        .expect("autospec claim acquire starts");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"reason\":\"already_merged\""));
    assert!(!heartbeats.join("testorg__testrepo/42.json").exists());
    let calls = std::fs::read_to_string(log).expect("gh log");
    assert!(calls.contains("--remove-label\nin-progress-by-bot"));
    assert!(!calls.contains("issue\ncomment\n42"));
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
