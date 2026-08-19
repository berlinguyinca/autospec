use super::*;

#[path = "../../src/commands/autonomous/accountability.rs"]
#[allow(dead_code)]
mod accountability_contract;

#[test]
fn foreground_recovery_accountability_records_authoritative_successor_once() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    let identity = accountability_contract::RunIdentity::derive(
        accountability_contract::RepositoryIdentity::parse("test/repo").unwrap(),
        accountability_contract::RunNonce::parse("00112233445566778899aabbccddeeff").unwrap(),
        accountability_contract::LeaseGeneration::new(7).unwrap(),
    );
    let projection = "Existing accountable autonomous run";
    let manifest = accountability_contract::RecoveryManifest::new(
        identity.clone(),
        999,
        "https://github.com/test/repo/issues/999",
        1,
        sha256_hex(format!("{projection}\n").as_bytes()),
        0,
        1,
    )
    .unwrap();
    let marker = format!(
        "<!-- autospec:run-epic repo=test/repo run_id={} -->",
        identity.run_id()
    );
    fs::write(
        &fixture.accountability,
        accountability_contract::github::compose_managed_body(&marker, projection, &manifest, ""),
    )
    .unwrap();
    let lifecycle = fixture.resilience_state_path();
    fs::create_dir_all(lifecycle.parent().unwrap()).unwrap();
    fs::write(
        lifecycle,
        "{\"repo\":\"test/repo\",\"slug\":\"test__repo\",\"status\":\"released\",\"host\":null,\"session\":null,\"heartbeat_at\":null,\"lock_pid\":null,\"lock_host\":null,\"lock_session\":null,\"lock_acquired_at\":null,\"lease_token\":null,\"lease_generation\":7}\n",
    )
    .unwrap();
    let branch = "feat/autonomous-issue-42";
    git_fixture(&fixture.repo_dir, &["branch", branch]);
    let stale = RunStateRecord::new(
        "test/repo",
        42,
        "stranded-worker",
        "claimed",
        branch,
        "",
        "heartbeat-pending:none",
        Vec::new(),
        "2000-01-01T00:00:00Z",
        "2000-01-01T00:00:00Z",
        1,
    )
    .with_claim_id("stranded-claim");
    fixture.transition_claim_ref(&stale);
    fs::write(&fixture.mode, "reviewed\n").expect("seed reviewed issue");

    let output = fixture
        .configured_command()
        .args([
            "autonomous",
            "start",
            "--repo",
            "test/repo",
            "--repo-dir",
            fixture.repo_dir.to_str().unwrap(),
            "--foreground",
            "--epic",
            "999",
            "--branch",
            "main",
            "--max-cycles",
            "1",
        ])
        .output()
        .expect("run foreground recovery");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let journal = fs::read_to_string(
        fixture
            .scoped_dir()
            .join("accountability-resumes/epic-999/accountability-events.jsonl"),
    )
    .expect("recovery accountability journal");
    assert_eq!(journal.matches("startup_claim_recovered").count(), 1);
    assert_eq!(journal.matches("issue_claimed").count(), 1);
    let recovered = journal.find("startup_claim_recovered").unwrap();
    let claimed = journal.find("issue_claimed").unwrap();
    assert!(
        recovered < claimed,
        "recovery must precede the successor claim event"
    );
    let projection = fs::read_to_string(&fixture.accountability).unwrap();
    assert_eq!(projection.matches("autospec:run-epic").count(), 1);
    assert!(projection.contains("Startup claim recovered"));
    assert_recovery_accountability_targets_existing_epic(&fixture);
}

#[test]
fn foreground_recovery_accountability_integrity_failure_does_not_undo_claim_handoff() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    seed_recovery_accountability_epic(&fixture, "50112233445566778899aabbccddeeff");
    let branch = "feat/autonomous-issue-42";
    git_fixture(&fixture.repo_dir, &["branch", branch]);
    let stale = RunStateRecord::new(
        "test/repo",
        42,
        "stranded-worker",
        "claimed",
        branch,
        "",
        "heartbeat-pending:none",
        Vec::new(),
        "2000-01-01T00:00:00Z",
        "2000-01-01T00:00:00Z",
        1,
    )
    .with_claim_id("stranded-claim");
    fixture.transition_claim_ref(&stale);
    fs::write(&fixture.mode, "reviewed\n").expect("seed reviewed issue");
    let view_counter = fixture.root.join("accountability-view-count");

    let output = fixture
        .configured_command()
        .env(
            "AUTOSPEC_FOREGROUND_ACCOUNTABILITY_VIEW_COUNTER",
            &view_counter,
        )
        .env("AUTOSPEC_FOREGROUND_ACCOUNTABILITY_TAMPER_AT_VIEW", "3")
        .args([
            "autonomous",
            "start",
            "--repo",
            "test/repo",
            "--repo-dir",
            fixture.repo_dir.to_str().unwrap(),
            "--foreground",
            "--epic",
            "999",
            "--branch",
            "main",
            "--max-cycles",
            "1",
        ])
        .output()
        .expect("run recovery with degraded accountability projection");

    assert!(
        output.status.success(),
        "stdout={} stderr={} calls={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        fs::read_to_string(&fixture.calls).unwrap_or_default()
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("recovery accountability degraded"));
    let journal = fs::read_to_string(
        fixture
            .scoped_dir()
            .join("accountability-resumes/epic-999/accountability-events.jsonl"),
    )
    .expect("recovery accountability journal");
    assert!(journal.contains("startup_claim_recovered"));
    assert!(journal.contains("issue_claimed"));
    assert_ne!(
        fixture.claim_record().claim_id.as_deref(),
        Some("stranded-claim")
    );
}

#[test]
fn foreground_recovery_accountability_records_deferred_then_recovered_once() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    let identity = accountability_contract::RunIdentity::derive(
        accountability_contract::RepositoryIdentity::parse("test/repo").unwrap(),
        accountability_contract::RunNonce::parse("10112233445566778899aabbccddeeff").unwrap(),
        accountability_contract::LeaseGeneration::new(7).unwrap(),
    );
    let projection = "Existing accountable autonomous run";
    let manifest = accountability_contract::RecoveryManifest::new(
        identity.clone(),
        999,
        "https://github.com/test/repo/issues/999",
        1,
        sha256_hex(format!("{projection}\n").as_bytes()),
        0,
        1,
    )
    .unwrap();
    let marker = format!(
        "<!-- autospec:run-epic repo=test/repo run_id={} -->",
        identity.run_id()
    );
    fs::write(
        &fixture.accountability,
        accountability_contract::github::compose_managed_body(&marker, projection, &manifest, ""),
    )
    .unwrap();
    let lifecycle = fixture.resilience_state_path();
    fs::create_dir_all(lifecycle.parent().unwrap()).unwrap();
    fs::write(
        lifecycle,
        "{\"repo\":\"test/repo\",\"slug\":\"test__repo\",\"status\":\"released\",\"host\":null,\"session\":null,\"heartbeat_at\":null,\"lock_pid\":null,\"lock_host\":null,\"lock_session\":null,\"lock_acquired_at\":null,\"lease_token\":null,\"lease_generation\":7}\n",
    )
    .unwrap();
    let branch = "feat/autonomous-issue-42";
    git_fixture(&fixture.repo_dir, &["branch", branch]);
    fs::write(&fixture.mode, "reviewed\n").expect("seed reviewed issue");
    fs::write(&fixture.heartbeats, "block heartbeat root\n")
        .expect("block first heartbeat publication");

    let mut command = fixture.configured_command();
    command.args([
        "autonomous",
        "start",
        "--repo",
        "test/repo",
        "--repo-dir",
        fixture.repo_dir.to_str().unwrap(),
        "--foreground",
        "--epic",
        "999",
        "--branch",
        "main",
        "--max-cycles",
        "2",
        "--poll-interval-sec",
        "1",
    ]);
    let child = command.spawn().expect("spawn recovery conductor");
    let journal_path = fixture
        .scoped_dir()
        .join("accountability-resumes/epic-999/accountability-events.jsonl");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if fs::read_to_string(&journal_path)
            .is_ok_and(|journal| journal.contains("heartbeat_publication_deferred"))
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let deferred = fs::read_to_string(&journal_path).expect("deferred event journal");
    assert!(deferred.contains("heartbeat_publication_deferred"));
    let pending = fixture.claim_record();
    let stale = RunStateRecord::new(
        "test/repo",
        42,
        &pending.worker_id,
        "claimed",
        branch,
        "",
        "heartbeat-pending:none",
        Vec::new(),
        "2000-01-01T00:00:00Z",
        "2000-01-01T00:00:00Z",
        1,
    )
    .with_claim_id(pending.claim_id.as_deref().unwrap());
    fixture.transition_claim_ref(&stale);
    fs::remove_file(&fixture.heartbeats).expect("remove heartbeat publication blocker");
    fs::create_dir(&fixture.heartbeats).expect("restore heartbeat root");
    fs::set_permissions(&fixture.heartbeats, fs::Permissions::from_mode(0o700)).unwrap();

    let output = child
        .wait_with_output()
        .expect("wait for recovery conductor");
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let journal = fs::read_to_string(&journal_path).unwrap();
    assert_eq!(journal.matches("heartbeat_publication_deferred").count(), 1);
    assert_eq!(journal.matches("startup_claim_recovered").count(), 1);
    assert_eq!(journal.matches("issue_claimed").count(), 1);
    let deferred = journal.find("heartbeat_publication_deferred").unwrap();
    let recovered = journal.find("startup_claim_recovered").unwrap();
    let claimed = journal.find("issue_claimed").unwrap();
    assert!(deferred < recovered && recovered < claimed);
    assert!(journal
        .find("stopped")
        .is_none_or(|stopped| stopped > claimed));
    let projection = fs::read_to_string(&fixture.accountability).unwrap();
    assert_eq!(projection.matches("autospec:run-epic").count(), 1);
    assert!(projection.lines().any(|line| {
        let line = line.trim();
        line.starts_with("deferred_42_") && line.contains(" --> recovered_42_")
    }));
    assert_recovery_accountability_targets_existing_epic(&fixture);
}

#[test]
fn foreground_recovery_accountability_dry_run_emits_no_event() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    seed_recovery_accountability_epic(&fixture, "30112233445566778899aabbccddeeff");
    let branch = "feat/autonomous-issue-42";
    git_fixture(&fixture.repo_dir, &["branch", branch]);
    let stale = RunStateRecord::new(
        "test/repo",
        42,
        "stranded-worker",
        "claimed",
        branch,
        "",
        "heartbeat-pending:none",
        Vec::new(),
        "2000-01-01T00:00:00Z",
        "2000-01-01T00:00:00Z",
        1,
    )
    .with_claim_id("dry-run-stranded-claim");
    fixture.transition_claim_ref(&stale);
    let epic_before = fs::read(&fixture.accountability).unwrap();
    let before = snapshot_tree(&fixture.root);

    let output = fixture
        .configured_command()
        .args([
            "autonomous",
            "start",
            "--repo",
            "test/repo",
            "--repo-dir",
            fixture.repo_dir.to_str().unwrap(),
            "--foreground",
            "--epic",
            "999",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("preview recovery conductor");

    assert!(output.status.success());
    assert_eq!(snapshot_tree(&fixture.root), before);
    assert!(!fixture.scoped_dir().join("accountability").exists());
    assert!(!fixture
        .scoped_dir()
        .join("accountability-resumes/epic-999/accountability-events.jsonl")
        .exists());
    assert_eq!(fs::read(&fixture.accountability).unwrap(), epic_before);
    assert!(!fixture.calls.exists(), "dry-run must not call GitHub");
}

#[test]
fn foreground_recovery_accountability_suppresses_deferral_after_rollback_loss() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    seed_recovery_accountability_epic(&fixture, "20112233445566778899aabbccddeeff");
    let branch = "feat/autonomous-issue-42";
    git_fixture(&fixture.repo_dir, &["branch", branch]);
    fs::write(&fixture.mode, "reviewed\n").expect("seed reviewed issue");
    fs::write(&fixture.heartbeats, "block heartbeat root\n").expect("block heartbeat publication");
    let push_count = fixture.root.join("claim-push-count");
    let git_wrapper = fixture.bin.join("claim-git-race");
    write_executable(
        &git_wrapper,
        r####"#!/bin/sh
set -eu
if [ "${1:-}" = push ]; then
  count=0
  if [ -f "$AUTOSPEC_CLAIM_RACE_COUNT" ]; then count=$(cat "$AUTOSPEC_CLAIM_RACE_COUNT"); fi
  count=$((count + 1))
  printf '%s\n' "$count" > "$AUTOSPEC_CLAIM_RACE_COUNT"
  if [ "$count" -eq 3 ]; then
    reference=refs/autospec/claims/issue-42
    current=$(/usr/bin/git --git-dir "$AUTOSPEC_CLAIM_GIT_REMOTE" rev-parse "$reference")
    tree=$(/usr/bin/git --git-dir "$AUTOSPEC_CLAIM_GIT_REMOTE" mktree </dev/null)
    oid=$(printf '%s\n%s\n\n%s\n' 'autospec-claim-ledger-v1' 'generation=winning-generation' '<!-- autospec-run-state:begin -->
{"schema":1,"repo":"test/repo","issue":42,"worker_id":"winning-worker","state":"claimed","branch":"feat/winning","pr":"","step":"claimed","paths":[],"claimed_at":"2030-01-01T00:00:00Z","updated_at":"2030-01-01T00:00:00Z","ttl_seconds":10800,"claim_id":"winning-claim"}
<!-- autospec-run-state:end -->' | \
      GIT_AUTHOR_NAME='Autospec Claim Test' GIT_AUTHOR_EMAIL='autospec-claim-test@localhost' \
      GIT_COMMITTER_NAME='Autospec Claim Test' GIT_COMMITTER_EMAIL='autospec-claim-test@localhost' \
      /usr/bin/git --git-dir "$AUTOSPEC_CLAIM_GIT_REMOTE" commit-tree "$tree" -p "$current")
    /usr/bin/git --git-dir "$AUTOSPEC_CLAIM_GIT_REMOTE" update-ref "$reference" "$oid" "$current"
  fi
fi
exec /usr/bin/git "$@"
"####,
    );

    let output = fixture
        .configured_command()
        .env("AUTOSPEC_CLAIM_GIT_BIN", &git_wrapper)
        .env("AUTOSPEC_CLAIM_RACE_COUNT", &push_count)
        .args([
            "autonomous",
            "start",
            "--repo",
            "test/repo",
            "--repo-dir",
            fixture.repo_dir.to_str().unwrap(),
            "--foreground",
            "--epic",
            "999",
            "--branch",
            "main",
            "--max-cycles",
            "1",
        ])
        .output()
        .expect("run heartbeat rollback race");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"reason\":\"claim_lost\""));
    let journal = fs::read_to_string(
        fixture
            .scoped_dir()
            .join("accountability-resumes/epic-999/accountability-events.jsonl"),
    )
    .unwrap();
    assert!(!journal.contains("heartbeat_publication_deferred"));
    assert!(!fs::read_to_string(&fixture.accountability)
        .unwrap()
        .contains("authoritative claim remains pending"));
    assert_eq!(
        fixture.claim_record().claim_id.as_deref(),
        Some("winning-claim")
    );
}

fn seed_recovery_accountability_epic(fixture: &ForegroundFixture, nonce: &str) {
    let identity = accountability_contract::RunIdentity::derive(
        accountability_contract::RepositoryIdentity::parse("test/repo").unwrap(),
        accountability_contract::RunNonce::parse(nonce).unwrap(),
        accountability_contract::LeaseGeneration::new(7).unwrap(),
    );
    let projection = "Existing accountable autonomous run";
    let manifest = accountability_contract::RecoveryManifest::new(
        identity.clone(),
        999,
        "https://github.com/test/repo/issues/999",
        1,
        sha256_hex(format!("{projection}\n").as_bytes()),
        0,
        1,
    )
    .unwrap();
    let marker = format!(
        "<!-- autospec:run-epic repo=test/repo run_id={} -->",
        identity.run_id()
    );
    fs::write(
        &fixture.accountability,
        accountability_contract::github::compose_managed_body(&marker, projection, &manifest, ""),
    )
    .unwrap();
    let lifecycle = fixture.resilience_state_path();
    fs::create_dir_all(lifecycle.parent().unwrap()).unwrap();
    fs::write(
        lifecycle,
        "{\"repo\":\"test/repo\",\"slug\":\"test__repo\",\"status\":\"released\",\"host\":null,\"session\":null,\"heartbeat_at\":null,\"lock_pid\":null,\"lock_host\":null,\"lock_session\":null,\"lock_acquired_at\":null,\"lease_token\":null,\"lease_generation\":7}\n",
    )
    .unwrap();
}

fn assert_recovery_accountability_targets_existing_epic(fixture: &ForegroundFixture) {
    let calls = fs::read_to_string(&fixture.calls).unwrap();
    assert!(!calls.contains("issue\ncreate"));
    assert!(!calls.contains("api\n--method\nPOST\nrepos/test/repo/issues\n"));
    assert!(calls.contains("issue\nedit\n999"));
    assert!(!calls.contains("issue\ncomment\n999"));
}
