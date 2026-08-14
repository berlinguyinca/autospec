#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write fake command");
    let mut permissions = fs::metadata(path)
        .expect("read fake command metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("make fake command executable");
}

#[test]
fn claim_acquire_uses_the_configured_trusted_actor_policy() {
    let fixture = std::env::temp_dir().join(format!(
        "autospec-claim-policy-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos(),
    ));
    let bin = fixture.join("bin");
    let comments = fixture.join("comments.json");
    let mode = fixture.join("labels.mode");
    let policy = fixture.join("trusted-actors.yml");
    let claim_remote = fixture.join("claim-remote.git");
    fs::create_dir_all(&bin).expect("create fixture bin directory");
    assert!(Command::new("git")
        .args(["init", "--bare"])
        .arg(&claim_remote)
        .output()
        .expect("initialize claim remote")
        .status
        .success());
    fs::write(&comments, "[]\n").expect("write empty comments");
    fs::write(&mode, "ready\n").expect("write initial label mode");
    fs::write(
        &policy,
        "safety:\n  issue_intent_gate:\n    trusted_actors:\n      - login: release-operator\n",
    )
    .expect("write trusted actor policy");
    write_executable(
        &bin.join("gh"),
        r#"#!/usr/bin/env sh
set -eu
if [ "$1" = issue ] && [ "$2" = view ]; then
  if [ "$(cat "$AUTOSPEC_CLAIM_POLICY_MODE")" = active ]; then labels='["in-progress-by-bot","safety:reviewed"]'; else labels='["auto-implement","safety:reviewed"]'; fi
  jq -n --argjson labels "$labels" --arg body "$AUTOSPEC_CLAIM_POLICY_BODY" '{labels:$labels,title:"Reset test database",body:$body,author:"release-operator"}'
  exit 0
fi
if [ "$1" = label ] && [ "$2" = create ]; then exit 0; fi
if [ "$1" = issue ] && [ "$2" = edit ]; then printf active > "$AUTOSPEC_CLAIM_POLICY_MODE"; exit 0; fi
if [ "$1" = issue ] && [ "$2" = comment ]; then
  body=''; shift 2
  while [ "$#" -gt 0 ]; do case "$1" in --body) body="$2"; shift 2 ;; *) shift ;; esac; done
  jq --arg body "$body" '. + [{id:100,updated_at:"2026-07-14T00:00:00Z",body:$body}]' "$AUTOSPEC_CLAIM_POLICY_COMMENTS" > "$AUTOSPEC_CLAIM_POLICY_COMMENTS.tmp"
  mv "$AUTOSPEC_CLAIM_POLICY_COMMENTS.tmp" "$AUTOSPEC_CLAIM_POLICY_COMMENTS"
  exit 0
fi
if [ "$1" = api ] && [ "$2" = repos/testorg/testrepo/issues/42/comments ]; then cat "$AUTOSPEC_CLAIM_POLICY_COMMENTS"; exit 0; fi
exit 0
"#,
    );
    let body = "## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n## Goal\n\nDelete the local test database and repopulate it from fixtures.\n\nOnly test, local, and fixture data are in scope. Production is out of scope.";
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").expect("PATH is set")
    );

    let output = Command::new(env!("CARGO_BIN_EXE_autospec"))
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
        .env("PATH", path)
        .env("AUTOSPEC_CLAIM_POLICY_COMMENTS", &comments)
        .env("AUTOSPEC_CLAIM_POLICY_MODE", &mode)
        .env("AUTOSPEC_CLAIM_POLICY_BODY", body)
        .env("AUTOSPEC_CONFIG_FILE", &policy)
        .env("AUTOSPEC_CLAIM_GIT_REMOTE", &claim_remote)
        .env("AUTOSPEC_HEARTBEAT_DIR", fixture.join("heartbeats"))
        .env("AUTOSPEC_CLAIM_CONFIRM_READS", "1")
        .env("AUTOSPEC_CLAIM_SETTLE_MILLIS", "0")
        .output()
        .expect("autospec claim acquire starts");

    assert!(
        output.status.success(),
        "stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"claimed\":true"));
    let _ = fs::remove_dir_all(fixture);
}
