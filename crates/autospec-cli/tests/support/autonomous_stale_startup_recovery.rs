#![cfg(target_os = "linux")]

use super::*;

#[cfg(target_os = "linux")]
#[test]
fn foreground_scan_recovers_stale_pending_startup_heartbeat_pending_before_acquire() {
    // Break caught: foreground acquisition skipped stale-startup recovery, replaced the
    // stranded claim generation, and then failed to publish over its expired heartbeat.
    let fixture = ForegroundFixture::new();
    fixture.initialize_empty_local_remote();
    let branch = "feat/autonomous-issue-42";
    seed_preserved_issue_branch(&fixture, branch);
    let branch_oid = git_fixture(&fixture.repo_dir, &["rev-parse", branch]);
    let stale = RunStateRecord::new(
        "test/repo",
        42,
        "successor-worker",
        "claimed",
        branch,
        "",
        "heartbeat-pending:none",
        Vec::new(),
        "2000-01-01T00:00:00Z",
        "2000-01-01T00:00:00Z",
        1,
    )
    .with_claim_id("successor-claim");
    fixture.transition_claim_ref(&stale);
    fixture.seed_expired_claim_heartbeat("prior-worker", branch, "prior-claim");
    fs::write(&fixture.mode, "reviewed\n").expect("seed reviewed issue");

    let output = fixture.run_foreground();

    assert!(
        output.status.success(),
        "stdout={} stderr={} calls={} claim={:?} heartbeat={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        fs::read_to_string(&fixture.calls).unwrap_or_default(),
        fixture.claim_record(),
        fs::read_to_string(fixture.heartbeats.join("o4_test_r4_repo/42.json")).unwrap_or_default(),
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("heartbeat_write_failed"),
        "foreground attempted acquisition before stale-startup recovery"
    );
    let claim_history = git_fixture(
        &fixture.root,
        &[
            "--git-dir",
            fixture.claim_remote.to_str().expect("claim remote"),
            "log",
            "--format=%B",
            "refs/autospec/claims/issue-42",
        ],
    );
    assert_eq!(
        claim_history
            .matches("\"step\":\"stale_startup_recovered\"")
            .count(),
        1,
        "stale startup must advance through recovery exactly once"
    );
    assert!(
        fs::read_to_string(&fixture.accountability)
            .expect("accountability evidence")
            .contains("Claimed issue 42"),
        "Scan path must reach IssueClaimed after recovery"
    );
    let journal = fs::read_to_string(
        fixture
            .scoped_dir()
            .join("accountability/accountability-events.jsonl"),
    )
    .expect("recovery accountability journal");
    assert_eq!(journal.matches("startup_claim_recovered").count(), 1);
    let recovered = journal.find("startup_claim_recovered").unwrap();
    let claimed = journal[recovered..].find("issue_claimed").unwrap() + recovered;
    assert!(
        recovered < claimed,
        "recovery must precede the successor claim event"
    );
    let projection = fs::read_to_string(&fixture.accountability).unwrap();
    assert_eq!(projection.matches("autospec:run-epic").count(), 1);
    assert!(projection.contains("Startup claim recovered"));
    let acquired = fixture.claim_record();
    assert!(acquired.worker_id.starts_with("rust-foreground-conductor-"));
    assert_ne!(acquired.claim_id.as_deref(), Some("successor-claim"));
    assert_eq!(
        git_fixture(&fixture.repo_dir, &["rev-parse", branch]),
        branch_oid
    );
    let heartbeat = fs::read_to_string(fixture.heartbeats.join("o4_test_r4_repo/42.json"))
        .expect("fresh foreground heartbeat");
    assert!(heartbeat.contains(&format!("\"worker_id\":{:?}", acquired.worker_id)));
    assert!(heartbeat.contains(&format!(
        "\"claim_id\":{:?}",
        acquired.claim_id.expect("fresh claim ID")
    )));
    assert!(fs::read_dir(
        fixture
            .heartbeats
            .join("o4_test_r4_repo/quarantine/startup-heartbeat-handoffs")
    )
    .expect("prior heartbeat handoff")
    .filter_map(Result::ok)
    .filter_map(|entry| fs::read_to_string(entry.path()).ok())
    .any(|document| document.contains("\"claim_id\":\"prior-claim\"")));

    for (case, updated_at, heartbeat_worker, heartbeat_branch, heartbeat_claim, live) in [
        (
            "fresh-claim",
            fresh_iso_timestamp(),
            "prior-worker",
            branch,
            "prior-claim",
            false,
        ),
        (
            "current-generation",
            "2000-01-01T00:00:00Z".to_string(),
            "blocked-worker",
            branch,
            "blocked-claim",
            false,
        ),
        (
            "live-prior",
            "2000-01-01T00:00:00Z".to_string(),
            "prior-worker",
            branch,
            "prior-claim",
            true,
        ),
        (
            "wrong-branch",
            "2000-01-01T00:00:00Z".to_string(),
            "prior-worker",
            "feat/foreign",
            "prior-claim",
            false,
        ),
    ] {
        let fixture = ForegroundFixture::new();
        fixture.initialize_empty_local_remote();
        seed_preserved_issue_branch(&fixture, branch);
        let blocked_branch_oid = git_fixture(&fixture.repo_dir, &["rev-parse", branch]);
        let blocked = RunStateRecord::new(
            "test/repo",
            42,
            "blocked-worker",
            "claimed",
            branch,
            "",
            "heartbeat-pending:none",
            Vec::new(),
            &updated_at,
            &updated_at,
            1,
        )
        .with_claim_id("blocked-claim");
        fixture.transition_claim_ref(&blocked);
        if live {
            fixture.seed_claim_heartbeat(heartbeat_worker, heartbeat_branch, heartbeat_claim);
        } else {
            fixture.seed_expired_claim_heartbeat(
                heartbeat_worker,
                heartbeat_branch,
                heartbeat_claim,
            );
        }
        fs::write(&fixture.mode, "reviewed\n").expect("seed reviewed issue");
        let heartbeat_path = fixture.heartbeats.join("o4_test_r4_repo/42.json");
        let heartbeat_before = fs::read_to_string(&heartbeat_path).expect("blocked heartbeat");
        let claim_before = fixture.claim_record();
        let claim_document_before = git_fixture(
            &fixture.root,
            &[
                "--git-dir",
                fixture.claim_remote.to_str().expect("claim remote"),
                "show",
                "-s",
                "--format=%B",
                "refs/autospec/claims/issue-42",
            ],
        );

        let output = fixture.run_foreground();

        assert!(
            !output.status.success(),
            "{case}: acquisition unexpectedly won"
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("\"reason\":\"claim_lost\""),
            "{case}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fixture.claim_record(),
            claim_before,
            "{case}: claim mutated"
        );
        assert_eq!(
            git_fixture(
                &fixture.root,
                &[
                    "--git-dir",
                    fixture.claim_remote.to_str().expect("claim remote"),
                    "show",
                    "-s",
                    "--format=%B",
                    "refs/autospec/claims/issue-42",
                ],
            ),
            claim_document_before,
            "{case}: claim document mutated"
        );
        assert_eq!(
            fs::read_to_string(&heartbeat_path).expect("preserved blocked heartbeat"),
            heartbeat_before,
            "{case}: heartbeat mutated"
        );
        assert_eq!(
            git_fixture(&fixture.repo_dir, &["rev-parse", branch]),
            blocked_branch_oid,
            "{case}: branch mutated"
        );
    }
}
