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
            1_720_000_000,
        )],
        vec![".autospec/evidence/run-v68/schema.json".to_string()],
    );

    let json = bundle.to_json();

    assert!(json.contains("\"run_id\":\"run-v68\""));
    assert!(json.contains("\"schema\":1"));
    assert!(json.contains("\"exit_code\":0"));
    assert!(json.contains("\"captured_at\":1720000000"));
    assert!(json.contains(".autospec/evidence/run-v68/schema.json"));
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
            1_720_000_001,
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
fn evidence_bundle_round_trips_a_negative_command_exit_code() {
    let root = TempProjectRoot::new();
    let bundle = EvidenceBundle::new(
        "run-failure",
        vec![EvidenceCommand::new(
            "cargo test --workspace",
            -1,
            ".autospec/evidence/run-failure/stdout.log",
            ".autospec/evidence/run-failure/stderr.log",
            1_720_000_002,
        )],
        vec![".autospec/evidence/run-failure/summary.md".to_string()],
    );

    bundle.save(root.path()).expect("bundle saves");
    let loaded = EvidenceBundle::load_named(root.path(), "run-failure")
        .expect("bundle loads")
        .expect("bundle exists");

    assert_eq!(loaded, bundle);
}

#[test]
fn evidence_bundle_reports_operational_read_errors_without_masking_them_as_corruption() {
    let root = TempProjectRoot::new();
    let bundle_path = root
        .path()
        .join(".autospec/evidence/run-read-error/bundle.json");
    fs::create_dir_all(&bundle_path).expect("bundle path is a directory");

    let error = EvidenceBundle::load_named(root.path(), "run-read-error")
        .expect_err("directory cannot be read as a bundle");

    assert!(error.contains("failed to read evidence bundle"));
}

#[test]
fn evidence_bundle_recovers_a_valid_temporary_bundle_after_primary_corruption() {
    let root = TempProjectRoot::new();
    let bundle = EvidenceBundle::new(
        "run-recovery",
        vec![EvidenceCommand::new(
            "cargo test --workspace",
            0,
            ".autospec/evidence/run-recovery/stdout.log",
            ".autospec/evidence/run-recovery/stderr.log",
            1_720_000_003,
        )],
        vec![".autospec/evidence/run-recovery/summary.md".to_string()],
    );
    let directory = root.path().join(".autospec/evidence/run-recovery");
    fs::create_dir_all(&directory).expect("evidence directory");
    fs::write(directory.join("bundle.json"), "not JSON").expect("corrupt primary");
    fs::write(directory.join("bundle.json.tmp"), bundle.to_json()).expect("temporary bundle");

    let loaded = EvidenceBundle::load_named(root.path(), "run-recovery")
        .expect("bundle recovers")
        .expect("bundle exists");

    assert_eq!(loaded, bundle);
    assert_eq!(
        fs::read_to_string(directory.join("bundle.json")).expect("promoted primary"),
        bundle.to_json()
    );
    assert!(!directory.join("bundle.json.tmp").exists());
}

#[test]
fn evidence_bundle_recovers_from_a_primary_with_the_wrong_run_id() {
    let root = TempProjectRoot::new();
    let primary = EvidenceBundle::new(
        "run-other",
        Vec::new(),
        vec![".autospec/evidence/run-other/summary.md".to_string()],
    );
    let recovery = EvidenceBundle::new(
        "run-requested",
        Vec::new(),
        vec![".autospec/evidence/run-requested/summary.md".to_string()],
    );
    let directory = root.path().join(".autospec/evidence/run-requested");
    fs::create_dir_all(&directory).expect("evidence directory");
    fs::write(directory.join("bundle.json"), primary.to_json()).expect("wrong primary");
    fs::write(directory.join("bundle.json.tmp"), recovery.to_json()).expect("recovery bundle");

    let loaded = EvidenceBundle::load_named(root.path(), "run-requested")
        .expect("recovery bundle loads")
        .expect("bundle exists");

    assert_eq!(loaded, recovery);
    assert_eq!(
        fs::read_to_string(directory.join("bundle.json")).expect("promoted primary"),
        recovery.to_json()
    );
}

#[test]
fn evidence_bundle_keeps_a_valid_primary_over_temporary_recovery_data() {
    let root = TempProjectRoot::new();
    let primary = EvidenceBundle::new(
        "run-primary",
        vec![EvidenceCommand::new(
            "cargo test --workspace",
            0,
            ".autospec/evidence/run-primary/stdout.log",
            ".autospec/evidence/run-primary/stderr.log",
            1_720_000_004,
        )],
        vec![".autospec/evidence/run-primary/summary.md".to_string()],
    );
    let recovery = EvidenceBundle::new(
        "run-primary",
        vec![EvidenceCommand::new(
            "cargo test --workspace --all-targets",
            1,
            ".autospec/evidence/run-primary/recovery-stdout.log",
            ".autospec/evidence/run-primary/recovery-stderr.log",
            1_720_000_005,
        )],
        vec![".autospec/evidence/run-primary/recovery.md".to_string()],
    );
    primary.save(root.path()).expect("primary saves");
    let temporary = root
        .path()
        .join(".autospec/evidence/run-primary/bundle.json.tmp");
    fs::write(&temporary, recovery.to_json()).expect("stale temporary bundle");

    let loaded = EvidenceBundle::load_named(root.path(), "run-primary")
        .expect("primary loads")
        .expect("bundle exists");

    assert_eq!(loaded, primary);
    assert!(
        temporary.exists(),
        "valid primary must win without promotion"
    );
}

#[test]
fn evidence_bundle_rejects_path_traversal_and_duplicate_artifacts() {
    let root = TempProjectRoot::new();
    let traversal = EvidenceBundle::new(
        "run-paths",
        vec![EvidenceCommand::new(
            "cargo test",
            0,
            ".autospec/evidence/run-paths/../escaped.log",
            ".autospec/evidence/run-paths/stderr.log",
            1_720_000_006,
        )],
        vec![".autospec/evidence/run-paths/summary.md".to_string()],
    );
    let duplicate = EvidenceBundle::new(
        "run-duplicates",
        Vec::new(),
        vec![
            ".autospec/evidence/run-duplicates/summary.md".to_string(),
            ".autospec/evidence/run-duplicates/summary.md".to_string(),
        ],
    );
    let backslash_traversal = EvidenceBundle::new(
        "run-backslash",
        vec![EvidenceCommand::new(
            "cargo test",
            0,
            ".autospec/evidence/run-backslash/..\\escaped.log",
            ".autospec/evidence/run-backslash/stderr.log",
            1_720_000_008,
        )],
        vec![".autospec/evidence/run-backslash/summary.md".to_string()],
    );

    assert!(traversal
        .save(root.path())
        .expect_err("traversal is rejected")
        .contains("escapes bundle directory"));
    assert!(duplicate
        .save(root.path())
        .expect_err("duplicates are rejected")
        .contains("duplicate evidence artifact"));
    assert!(backslash_traversal
        .save(root.path())
        .expect_err("backslash traversal is rejected")
        .contains("escapes bundle directory"));
}

#[test]
fn evidence_bundle_rejects_a_document_stored_under_the_wrong_run_id() {
    let root = TempProjectRoot::new();
    let bundle = EvidenceBundle::new(
        "run-actual",
        Vec::new(),
        vec![".autospec/evidence/run-actual/summary.md".to_string()],
    );
    let directory = root.path().join(".autospec/evidence/run-requested");
    fs::create_dir_all(&directory).expect("evidence directory");
    fs::write(directory.join("bundle.json"), bundle.to_json()).expect("mismatched bundle");

    let error = EvidenceBundle::load_named(root.path(), "run-requested")
        .expect_err("mismatched run id is rejected");

    assert!(error.contains("run id does not match path"));
}

#[test]
fn evidence_bundle_round_trips_escaped_control_characters() {
    let root = TempProjectRoot::new();
    let bundle = EvidenceBundle::new(
        "run-escape",
        vec![EvidenceCommand::new(
            "validate\u{1}\nnext",
            0,
            ".autospec/evidence/run-escape/stdout.log",
            ".autospec/evidence/run-escape/stderr.log",
            1_720_000_007,
        )],
        vec![".autospec/evidence/run-escape/summary.md".to_string()],
    );

    bundle.save(root.path()).expect("bundle saves");
    let loaded = EvidenceBundle::load_named(root.path(), "run-escape")
        .expect("bundle loads")
        .expect("bundle exists");

    assert_eq!(loaded, bundle);
    assert!(bundle.to_json().contains("\\u0001"));
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
