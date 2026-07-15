use std::process::Command;

fn lifecycle(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args(["autonomous", "lifecycle", "decide"])
        .args(args)
        .output()
        .expect("run lifecycle decision")
}

#[test]
fn lifecycle_decide_serializes_ready_tier_one() {
    let output = lifecycle(&["--repo", "test/repo", "--ready-tier", "1"]);

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"decision\":\"run\",\"tier\":\"1\"}\n"
    );
}

#[test]
fn lifecycle_decide_serializes_each_explicit_tier() {
    for tier in ["1.5", "2", "3", "4", "5", "6", "7"] {
        let output = lifecycle(&["--repo", "test/repo", "--ready-tier", tier]);
        assert!(output.status.success(), "tier={tier}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{{\"decision\":\"run\",\"tier\":\"{tier}\"}}\n")
        );
    }
}

#[test]
fn lifecycle_decide_returns_nonzero_json_for_stop_and_claim_rejection() {
    let stopped = lifecycle(&["--repo", "test/repo", "--stop", "immediate"]);
    assert_eq!(stopped.status.code(), Some(20));
    assert_eq!(
        String::from_utf8_lossy(&stopped.stdout),
        "{\"decision\":\"stop\",\"mode\":\"immediate\"}\n"
    );

    let rejected = lifecycle(&["--repo", "test/repo", "--claim-repo", "other/repo"]);
    assert_eq!(rejected.status.code(), Some(3));
    assert_eq!(
        String::from_utf8_lossy(&rejected.stdout),
        "{\"decision\":\"reject\",\"reason\":\"cross_scope_claim\"}\n"
    );
}

#[test]
fn lifecycle_decide_serializes_health_budget_stale_and_idle_gates() {
    let health = lifecycle(&["--repo", "test/repo", "--health", "wait"]);
    assert_eq!(health.status.code(), Some(20));
    assert_eq!(
        String::from_utf8_lossy(&health.stdout),
        "{\"decision\":\"park\",\"reason\":\"health_wait\"}\n"
    );

    let budget = lifecycle(&["--repo", "test/repo", "--budget", "hard"]);
    assert_eq!(budget.status.code(), Some(20));
    assert_eq!(
        String::from_utf8_lossy(&budget.stdout),
        "{\"decision\":\"park\",\"reason\":\"budget_hard_cap\"}\n"
    );

    let stale = lifecycle(&["--repo", "test/repo", "--lease-age-sec", "301"]);
    assert_eq!(stale.status.code(), Some(3));
    assert_eq!(
        String::from_utf8_lossy(&stale.stdout),
        "{\"decision\":\"reject\",\"reason\":\"stale_lease\"}\n"
    );

    let idle = lifecycle(&["--repo", "test/repo", "--ready-tier", "idle"]);
    assert_eq!(idle.status.code(), Some(20));
    assert_eq!(
        String::from_utf8_lossy(&idle.stdout),
        "{\"decision\":\"park\",\"reason\":\"idle_rescan\"}\n"
    );
}

#[test]
fn lifecycle_decide_rejects_malformed_and_repeated_flags() {
    let malformed = lifecycle(&["--repo", "test/repo", "--health", "unknown"]);
    assert_eq!(malformed.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&malformed.stderr).contains("--health"));

    let repeated = lifecycle(&[
        "--repo",
        "test/repo",
        "--ready-tier",
        "1",
        "--ready-tier",
        "2",
    ]);
    assert_eq!(repeated.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&repeated.stderr).contains("--ready-tier"));
}
