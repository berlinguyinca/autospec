use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use autospec_core::evidence::{EvidenceBundle, EvidenceCommand, ReleaseReport};
use autospec_core::state::{SpecLifecycle, SpecRunState};

static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(0);

struct TempProjectRoot {
    path: PathBuf,
}

impl TempProjectRoot {
    fn new() -> Self {
        let nonce = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("autospec-evidence-{nonce}-{}", std::process::id()));
        fs::create_dir_all(&path).expect("temporary root is created");
        Self { path }
    }
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempProjectRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn evidence_bundle_captures_command_artifacts() {
    let bundle = EvidenceBundle::new(
        "run-v68",
        vec![EvidenceCommand::new(
            "cargo test --all evidence",
            0,
            ".autospec/evidence/run-v68/stdout.log",
            ".autospec/evidence/run-v68/stderr.log",
        )],
        vec!["schemas/autospec-evidence-bundle.schema.json".to_string()],
    );

    let json = bundle.to_json();

    assert!(json.contains("\"run_id\":\"run-v68\""));
    assert!(json.contains("\"exit_code\":0"));
    assert!(json.contains("schemas/autospec-evidence-bundle.schema.json"));
}

#[test]
fn evidence_bundle_round_trips_through_its_run_directory() {
    let root = TempProjectRoot::new();
    let bundle = EvidenceBundle::new(
        "run-v68",
        vec![EvidenceCommand::new(
            "cargo test --workspace",
            0,
            ".autospec/evidence/run-v68/stdout.log",
            ".autospec/evidence/run-v68/stderr.log",
        )],
        vec![".autospec/evidence/run-v68/summary.md".to_string()],
    );

    bundle.save(root.path()).expect("bundle saves");
    let loaded = EvidenceBundle::load_named(root.path(), "run-v68")
        .expect("bundle loads")
        .expect("bundle exists");

    assert_eq!(loaded, bundle);
}

#[test]
fn release_report_fails_unknown_spec_state() {
    let states = vec![SpecLifecycle::new("v68-evidence-release-reporting")];

    let error = ReleaseReport::from_states("V68", &states).expect_err("planned is not final");

    assert!(error.contains("unknown or unfinished state"));
}

#[test]
fn release_report_renders_markdown_and_json_for_final_states() {
    let mut passed = SpecLifecycle::new("v68-evidence-release-reporting");
    passed.transition_to(SpecRunState::Ready).unwrap();
    passed.transition_to(SpecRunState::Running).unwrap();
    passed.transition_to(SpecRunState::Passed).unwrap();

    let report = ReleaseReport::from_states("V68", &[passed]).expect("final states are valid");

    assert!(report
        .to_markdown()
        .contains("# AutoSpec Release Report V68"));
    assert!(report.to_json().contains("\"version\":\"V68\""));
    assert_eq!(report.passed, 1);
}
