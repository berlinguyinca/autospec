use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn autospec() -> Command {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
}

#[test]
fn explore_repositories_renders_canonical_targets_and_deferred_archives() {
    let input = temp_dir("autospec-explore-repositories-input").join("repositories.json");
    std::fs::write(
        &input,
        r#"{
  "repositories": [
    {
      "name": "acme/platform-api",
      "archived": true,
      "pushed_at": "2023-01-01T00:00:00Z",
      "readme": "Archived split successor. Use acme/platform instead.",
      "module_paths": ["github.com/acme/platform/api"],
      "packages": ["acme-core-api"],
      "dependency_references": ["acme/platform"]
    },
    {
      "name": "acme/platform",
      "archived": false,
      "pushed_at": "2026-07-01T00:00:00Z",
      "readme": "Canonical home for acme-core packages.",
      "module_paths": ["github.com/acme/platform"],
      "packages": ["acme-core"],
      "dependency_references": []
    }
  ],
  "findings": [
    {"repository":"acme/platform-api","fingerprint":"fp-same","title":"stale docs","evidence":"archived evidence"},
    {"repository":"acme/platform","fingerprint":"fp-same","title":"stale docs duplicate","evidence":"canonical evidence"}
  ]
}
"#,
    )
    .unwrap();

    let output = autospec()
        .args([
            "explore",
            "repositories",
            "--input",
            input.to_str().unwrap(),
        ])
        .output()
        .expect("autospec explore repositories runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert!(stdout.contains("\"canonical_targets\""));
    assert!(stdout.contains("\"do_not_file_by_default\""));
    assert!(stdout.contains("\"repository\":\"acme/platform\""));
    assert!(stdout.contains("\"repository\":\"acme/platform-api\""));
    assert_eq!(stdout.matches("\"fingerprint\":\"fp-same\"").count(), 1);
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    path.push(format!("{name}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}

#[test]
fn explore_specialists_emits_json_and_persists_cache_for_temp_repo() {
    let repo = temp_dir("autospec-explore-specialists-repo");
    std::fs::write(repo.join("README.md"), "ccxt trading order book\n").unwrap();

    let output = autospec()
        .args([
            "explore",
            "specialists",
            "--repo-dir",
            repo.to_str().unwrap(),
            "--num-specialists",
            "1",
        ])
        .output()
        .expect("autospec explore specialists runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["suggested_specialists"].as_array().unwrap().len(), 1);
    assert_eq!(
        std::fs::read_to_string(repo.join(".autospec/explore-specialists.json")).unwrap(),
        stdout
    );
}

#[test]
fn explore_specialists_force_refreshes_existing_cache() {
    let repo = temp_dir("autospec-explore-specialists-force-repo");
    std::fs::write(repo.join("README.md"), "ccxt trading order book\n").unwrap();
    std::fs::create_dir_all(repo.join(".autospec")).unwrap();
    std::fs::write(
        repo.join(".autospec/explore-specialists.json"),
        "{\"schema_version\":1,\"domains\":[],\"suggested_specialists\":[]}",
    )
    .unwrap();

    let output = autospec()
        .args([
            "explore",
            "specialists",
            "--repo-dir",
            repo.to_str().unwrap(),
            "--num-specialists",
            "3",
            "--force",
        ])
        .output()
        .expect("autospec explore specialists --force runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("trading-specialist"));
}
