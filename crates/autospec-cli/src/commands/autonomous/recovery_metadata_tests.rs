use super::*;

const REPO: &str = "test/repo";
const SCOPE: &str = "test_repo";

fn metadata_with_identity(pid: u32, start_time_ticks: u64) -> String {
    format!(
        r#"{{"pid":{pid},"repo":"{REPO}","scope":"{SCOPE}","pgid":{pid},"start_time_ticks":{start_time_ticks}}}"#
    )
}

fn metadata_layout(label: &str) -> (PathBuf, RunLayout) {
    let root = std::env::temp_dir().join(format!(
        "autospec-autonomous-metadata-{label}-{}-{}",
        std::process::id(),
        ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).expect("create metadata fixture");
    let layout = RunLayout {
        state_dir: root.clone(),
        log_dir: root.join("logs"),
        scope: SCOPE.to_string(),
        repo: REPO.to_string(),
    };
    (root, layout)
}

#[test]
fn autonomous_process_observation_error_is_ambiguous() {
    let (root, layout) = metadata_layout("observation-error");
    let pid = std::process::id();
    fs::write(
        layout.state_dir.join("conductor.pid"),
        metadata_with_identity(pid, 1),
    )
    .expect("write unit metadata");

    let unit = read_unit_with_process_observer("conductor", &layout, |_| {
        Err("injected native observation error".to_string())
    });

    assert_eq!(unit.metadata_state, UnitMetadataState::Ambiguous);
    assert!(!unit.running);
    fs::remove_dir_all(root).expect("remove metadata fixture");
}

#[test]
fn autonomous_termination_refuses_ambiguous_birth_observation() {
    let (root, layout) = metadata_layout("termination-observation-error");
    let pid = std::process::id();
    let unit = UnitStatus {
        pid: pid.to_string(),
        running: true,
        stale_pid: false,
        metadata_only: false,
        metadata_state: UnitMetadataState::Live,
        recorded_identity: Some(ProcessIdentity {
            pgid: i32::try_from(pid).expect("test pid fits i32"),
            start_time_ticks: 1,
        }),
        identity_mismatch: false,
        pid_file: layout.state_dir.join("conductor.pid"),
        logpath: String::new(),
        logpath_file: layout.state_dir.join("conductor.logpath"),
    };

    let error = terminate_unit_with_process_observer("conductor", &unit, |_| {
        Err("injected native observation error".to_string())
    })
    .expect_err("ambiguous observation must refuse termination");

    assert!(error.contains("injected native observation error"));
    fs::remove_dir_all(root).expect("remove metadata fixture");
}
