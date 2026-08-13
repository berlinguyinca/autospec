use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn autonomous_start_names_the_stop_sentinel_that_blocks_it() {
    let temp = temp_dir("autospec-autonomous-start-blocked-by-stop");
    let operator_dir = temp.join("operator");
    let repo_dir = temp.join("repo");
    std::fs::create_dir_all(&repo_dir).expect("repo dir");
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo_dir)
        .status()
        .expect("git init runs")
        .success());

    let scope = operator_dir.join("berlinguyinca_autospec");
    std::fs::create_dir_all(&scope).expect("scope");
    std::fs::write(scope.join("stop.flag"), "immediate\n").expect("stop sentinel");

    let start = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_STATE_DIR", temp.join("state"))
        .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", temp.join("spend"))
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", temp.join("logs"))
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .output()
        .expect("autospec autonomous start runs");

    let stdout = String::from_utf8_lossy(&start.stdout);
    let stderr = String::from_utf8_lossy(&start.stderr);
    assert!(stdout.contains("\"decision\":\"stop\""), "stdout={stdout}");
    assert!(stderr.contains("stop.flag"), "stderr={stderr}");
    assert!(stderr.contains("restart"), "stderr={stderr}");
}

fn temp_dir(prefix: &str) -> std::path::PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "{prefix}-{}-{stamp}-{sequence}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}
