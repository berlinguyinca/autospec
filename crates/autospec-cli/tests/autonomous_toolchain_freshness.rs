#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn autospec() -> Command {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
}

fn temp_dir(prefix: &str) -> std::path::PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{prefix}-{}-{suffix}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn make_git_repo(repo_dir: &Path) {
    std::fs::create_dir_all(repo_dir).expect("repo dir");
    assert!(Command::new("git")
        .arg("init")
        .current_dir(repo_dir)
        .output()
        .expect("git init")
        .status
        .success());
}

fn install_accountability_fixture(command: &mut Command, temp: &Path) {
    let bin = temp.join("bin");
    std::fs::create_dir_all(&bin).expect("fixture bin");
    let gh = bin.join("gh");
    std::fs::write(
        &gh,
        "#!/bin/sh\n. \"$AUTOSPEC_FOREGROUND_ACCOUNTABILITY_HANDLER\"\nexit 1\n",
    )
    .expect("fixture gh");
    #[cfg(unix)]
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).expect("executable gh");
    command
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").expect("PATH")),
        )
        .env(
            "AUTOSPEC_FOREGROUND_ACCOUNTABILITY",
            temp.join("accountability.md"),
        )
        .env(
            "AUTOSPEC_FOREGROUND_ACCOUNTABILITY_REPO",
            "berlinguyinca/autospec",
        )
        .env(
            "AUTOSPEC_FOREGROUND_ACCOUNTABILITY_HANDLER",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/foreground_accountability_gh.sh"
            ),
        );
}

fn cleanup_pids(scope: &Path) {
    for name in ["conductor", "monitor", "supervisor"] {
        let Ok(metadata) = std::fs::read_to_string(scope.join(format!("{name}.pid"))) else {
            continue;
        };
        let pid = metadata
            .trim()
            .parse::<u32>()
            .ok()
            .or_else(|| {
                serde_json::from_str::<serde_json::Value>(&metadata)
                    .ok()?
                    .get("pid")?
                    .as_u64()
                    .and_then(|pid| u32::try_from(pid).ok())
            })
            .filter(|pid| *pid > 0);
        if let Some(pid) = pid {
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{pid}")])
                .output();
            let _ = Command::new("kill")
                .args(["-KILL", "--", &pid.to_string()])
                .output();
        }
    }
}

#[test]
fn autonomous_status_json_reports_toolchain_freshness_and_last_failure() {
    let temp = temp_dir("autospec-autonomous-toolchain-status");
    let home = temp.join("home");
    let autospec_home = home.join(".autospec");
    std::fs::create_dir_all(&autospec_home).expect("autospec home");
    std::fs::write(autospec_home.join("installed-version"), "oldsha1\n").expect("installed");
    std::fs::write(autospec_home.join("remote-version"), "newsha9\n").expect("remote");
    std::fs::write(autospec_home.join("last-update-failure.json"), "{}\n").expect("failure");

    let output = autospec()
        .args([
            "autonomous",
            "status",
            "--repo",
            "berlinguyinca/autospec",
            "--json",
        ])
        .env("HOME", &home)
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", temp.join("operator"))
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", temp.join("logs"))
        .output()
        .expect("autospec autonomous status runs");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("status json");
    let toolchain = &value["toolchain"];

    assert!(output.status.success());
    assert_eq!(toolchain["installed_version"], "oldsha1");
    assert_eq!(toolchain["remote_version"], "newsha9");
    assert!(toolchain["installed_age_secs"].is_number());
    assert_eq!(toolchain["last_update_failed"], true);
    assert!(toolchain["last_update_failure_path"]
        .as_str()
        .is_some_and(|path| path.ends_with("/.autospec/last-update-failure.json")));
}

#[test]
fn autonomous_list_json_reports_toolchain_freshness_without_running_scopes() {
    let temp = temp_dir("autospec-autonomous-toolchain-list");
    let home = temp.join("home");
    let autospec_home = home.join(".autospec");
    std::fs::create_dir_all(&autospec_home).expect("autospec home");
    std::fs::write(autospec_home.join("installed-version"), "same123\n").expect("installed");
    std::fs::write(autospec_home.join("remote-version"), "same123\n").expect("remote");

    let output = autospec()
        .args(["autonomous", "list", "--json"])
        .env("HOME", &home)
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", temp.join("operator"))
        .output()
        .expect("autospec autonomous list runs");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("list json");
    let toolchain = &value["toolchain"];

    assert!(output.status.success());
    assert_eq!(toolchain["installed_version"], "same123");
    assert_eq!(toolchain["remote_version"], "same123");
    assert!(toolchain["installed_age_secs"].is_number());
    assert_eq!(toolchain["last_update_failed"], false);
    assert!(toolchain["last_update_failure_path"].is_null());
}

#[test]
fn autonomous_foreground_warns_about_persisted_update_failure_without_blocking_entry() {
    let temp = temp_dir("autospec-autonomous-toolchain-warning");
    let home = temp.join("home");
    let autospec_home = home.join(".autospec");
    let repo_dir = temp.join("repo");
    std::fs::create_dir_all(&autospec_home).expect("autospec home");
    make_git_repo(&repo_dir);
    let failure_record = autospec_home.join("last-update-failure.json");
    std::fs::write(&failure_record, "{}\n").expect("failure record");

    let mut command = autospec();
    install_accountability_fixture(&mut command, &temp);
    let output = command
        .args([
            "autonomous",
            "run-foreground",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
        ])
        .env("HOME", &home)
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", temp.join("operator"))
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", temp.join("logs"))
        .output()
        .expect("autospec autonomous foreground enters");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("WARN: autospec self-update failed"),
        "status: {}; stdout: {}; stderr: {stderr}",
        output.status,
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(stderr.contains(failure_record.to_str().unwrap()));
}

#[test]
fn autonomous_start_warns_invoking_operator_about_persisted_update_failure() {
    let temp = temp_dir("autospec-autonomous-start-toolchain-warning");
    let home = temp.join("home");
    let autospec_home = home.join(".autospec");
    let repo_dir = temp.join("repo");
    let operator_dir = temp.join("operator");
    std::fs::create_dir_all(&autospec_home).expect("autospec home");
    make_git_repo(&repo_dir);
    let failure_record = autospec_home.join("last-update-failure.json");
    std::fs::write(&failure_record, "{}\n").expect("failure record");

    let mut command = autospec();
    install_accountability_fixture(&mut command, &temp);
    let output = command
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
        ])
        .env("HOME", &home)
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", temp.join("logs"))
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .output()
        .expect("autospec autonomous start runs");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stderr.contains("WARN: autospec self-update failed"));
    assert!(stderr.contains(failure_record.to_str().unwrap()));
    cleanup_pids(&operator_dir.join("berlinguyinca_autospec"));
}
