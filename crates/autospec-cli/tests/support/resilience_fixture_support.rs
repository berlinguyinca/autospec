use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "../../src/commands/autonomous/accountability.rs"]
#[allow(dead_code)]
mod accountability_contract;

pub(super) const ACCOUNTABILITY_ONLY_GH: &str = r#"#!/bin/sh
set -eu
. "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY_HANDLER"
printf 'unexpected gh invocation: %s\n' "$*" >&2
exit 1
"#;

pub(super) fn seed_bound_accountability(
    operator_root: &Path,
    remote_body: &Path,
    lease_generation: u64,
) {
    use accountability_contract::{
        AccountabilityStore, LaunchDescriptor, LeaseGeneration, RecoveryManifest,
        RepositoryIdentity, RunIdentity, RunNonce,
    };

    let identity = RunIdentity::derive(
        RepositoryIdentity::parse("owner/repo").expect("accountability repository"),
        RunNonce::parse("00112233445566778899aabbccddeeff").expect("accountability nonce"),
        LeaseGeneration::new(lease_generation).expect("accountability lease generation"),
    );
    let root = operator_root.join("owner_repo/accountability");
    let mut store = AccountabilityStore::open(&root).expect("open accountability store");
    store
        .begin_launch(
            LaunchDescriptor::new(
                identity.clone(),
                "Exercise the resilience foreground contract",
                "The fixture models the parent-owned accountability run",
            )
            .expect("accountability launch"),
        )
        .expect("begin accountability launch");
    store
        .mark_create_attempted()
        .expect("record accountability creation intent");
    store
        .bind_epic(999, "https://github.com/owner/repo/issues/999")
        .expect("bind accountability epic");
    let projection = store.render().expect("render accountability projection");
    let manifest = RecoveryManifest::new(
        identity.clone(),
        999,
        "https://github.com/owner/repo/issues/999",
        projection.revision,
        projection.digest.clone(),
        projection.desired_high_watermark,
        store.status().journal_segment,
    )
    .expect("accountability recovery manifest");
    let marker = format!(
        "<!-- autospec:run-epic repo=owner/repo run_id={} -->",
        identity.run_id()
    );
    fs::write(
        remote_body,
        accountability_contract::github::compose_managed_body(
            &marker,
            &projection.markdown,
            &manifest,
            "",
        ),
    )
    .expect("write remote accountability fixture");
    store
        .ack_projection(
            projection.revision,
            &projection.digest,
            projection.desired_high_watermark,
        )
        .expect("ack accountability projection");
    store.mark_spawned().expect("mark accountability spawned");
    write_file(
        &operator_root.join("owner_repo/launch.json"),
        &format!(
            "{{\"accountability\":{{\"run_id\":\"{}\",\"epic_number\":999,\"epic_url\":\"https://github.com/owner/repo/issues/999\"}}}}\n",
            identity.run_id()
        ),
    );
}

pub(super) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("current time")
        .as_secs()
}

pub(super) fn dead_child_pid() -> u32 {
    let mut child = Command::new("true")
        .spawn()
        .expect("start dead PID fixture");
    let pid = child.id();
    assert!(child.wait().expect("reap dead PID fixture").success());
    pid
}

pub(super) fn write_file(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().expect("parent directory")).expect("create parent directory");
    fs::write(path, contents).expect("write fixture state");
}

pub(super) fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write fake executable");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("make fake executable");
}

pub(super) fn path_with(bin: &Path) -> String {
    format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").expect("PATH is set")
    )
}

pub(super) fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub(super) fn git_fixture(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("run fixture git command");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

pub(super) fn fixture_root(sequence: u64) -> PathBuf {
    std::env::temp_dir().join(format!(
        "autospec-resilience-test-{}-{sequence}",
        std::process::id()
    ))
}

pub(super) fn wait_for_path(path: &Path, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while !path.exists() && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    path.exists()
}
