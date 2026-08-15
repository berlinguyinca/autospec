use super::{sha256_hex, ForegroundFixture};
use std::fs;
use std::path::Path;

#[path = "../../src/commands/autonomous/accountability.rs"]
#[allow(dead_code)]
mod accountability_contract;

pub(super) fn write_git_exclude(repo_dir: &Path) {
    let info = repo_dir.join(".git/info");
    fs::create_dir_all(&info).expect("create Git info directory");
    fs::write(info.join("exclude"), ".autospec/\n").expect("ignore bridge evidence artifacts");
}

#[test]
fn active_remote_epic_adopts_the_released_predecessor_generation_after_real_acquisition() {
    use accountability_contract::{
        LeaseGeneration, RecoveryManifest, RepositoryIdentity, RunIdentity, RunNonce,
    };

    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    let identity = RunIdentity::derive(
        RepositoryIdentity::parse("test/repo").unwrap(),
        RunNonce::parse("00112233445566778899aabbccddeeff").unwrap(),
        LeaseGeneration::new(7).unwrap(),
    );
    let projection = "# Existing active run";
    let manifest = RecoveryManifest::new(
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

    let output = fixture
        .detached_command("start")
        .args(["--detach", "--epic", "999", "--branch", "main"])
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .env("AUTOSPEC_FOREGROUND_BLOCK_GH", "1")
        .output()
        .expect("start from active remote epic");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let launch = fs::read_to_string(fixture.scoped_dir().join("launch.json")).unwrap();
    assert!(launch.contains(identity.run_id()));
    fixture.terminate_recorded_conductor();
}
