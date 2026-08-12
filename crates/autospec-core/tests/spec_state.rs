use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::state::{ParentIssueStatus, SpecLifecycle, SpecRunState, SpecStateStore};

static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(0);

struct TempProjectRoot {
    path: PathBuf,
}

impl TempProjectRoot {
    fn new() -> Self {
        let nonce = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "autospec-spec-state-{}-{timestamp}-{nonce}",
            std::process::id()
        ));

        fs::create_dir_all(&path).expect("temporary project root is created");

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

fn state_file(root: &TempProjectRoot, filename: &str) -> PathBuf {
    root.path().join(".autospec").join("state").join(filename)
}

fn write_state_file(root: &TempProjectRoot, filename: &str, document: &str) {
    let path = state_file(root, filename);
    fs::create_dir_all(path.parent().expect("state file has a parent"))
        .expect("state directory is created");
    fs::write(path, document).expect("state document is written");
}

fn assert_document_is_rejected(document: &str) {
    let root = TempProjectRoot::new();
    write_state_file(&root, "specs.json", document);

    assert!(
        SpecStateStore::load_or_default(root.path()).is_err(),
        "invalid state document must be rejected"
    );
}

#[test]
fn spec_state_allows_valid_transitions() {
    let mut lifecycle = SpecLifecycle::new("v65-spec-state-validation");

    lifecycle
        .transition_to(SpecRunState::Ready)
        .expect("planned -> ready");
    lifecycle
        .transition_to(SpecRunState::Running)
        .expect("ready -> running");
    lifecycle
        .transition_to(SpecRunState::Passed)
        .expect("running -> passed");

    assert_eq!(lifecycle.state, SpecRunState::Passed);
}

#[test]
fn spec_state_rejects_invalid_transitions() {
    let mut lifecycle = SpecLifecycle::new("v65-spec-state-validation");

    let error = lifecycle
        .transition_to(SpecRunState::Passed)
        .expect_err("planned -> passed should fail");

    assert!(error.to_string().contains("invalid transition"));
    assert_eq!(lifecycle.state, SpecRunState::Planned);
}

#[test]
fn spec_state_records_deferred_and_superseded_metadata() {
    let deferred = SpecLifecycle::new("v65-spec-state-validation")
        .deferred("waiting on V64 proof")
        .expect("deferred reason is valid");
    let superseded = SpecLifecycle::new("v65-spec-state-validation")
        .superseded_by("v66-autonomous-execution-queue")
        .expect("replacement id is valid");

    assert_eq!(deferred.state, SpecRunState::Deferred);
    assert_eq!(
        deferred.deferred_reason.as_deref(),
        Some("waiting on V64 proof")
    );
    assert_eq!(superseded.state, SpecRunState::Superseded);
    assert_eq!(
        superseded.superseded_by.as_deref(),
        Some("v66-autonomous-execution-queue")
    );
}

#[test]
fn spec_state_store_round_trips_records_in_deterministic_spec_id_order() {
    let root = TempProjectRoot::new();
    let mut store = SpecStateStore::new();

    let mut passed = SpecLifecycle::new("v101-passed");
    passed
        .transition_to(SpecRunState::Ready)
        .expect("planned -> ready");
    passed
        .transition_to(SpecRunState::Running)
        .expect("ready -> running");
    passed
        .transition_to(SpecRunState::Passed)
        .expect("running -> passed");

    store
        .insert(SpecLifecycle::new("v104-replacement"))
        .expect("replacement record is valid");
    store
        .insert(
            SpecLifecycle::new("v103-superseded")
                .superseded_by("v104-replacement")
                .expect("supersession metadata is valid"),
        )
        .expect("superseded record is valid");
    store
        .insert(
            SpecLifecycle::new("v102-deferred")
                .deferred("waiting on V64 proof")
                .expect("deferred metadata is valid"),
        )
        .expect("deferred record is valid");
    store.insert(passed).expect("passed record is valid");
    store
        .insert(SpecLifecycle::new("v100-planned"))
        .expect("planned record is valid");

    let rendered = store.to_json().expect("state document renders");
    let planned = rendered
        .find("\"spec_id\":\"v100-planned\"")
        .expect("planned record is rendered");
    let passed = rendered
        .find("\"spec_id\":\"v101-passed\"")
        .expect("passed record is rendered");
    let deferred = rendered
        .find("\"spec_id\":\"v102-deferred\"")
        .expect("deferred record is rendered");
    let superseded = rendered
        .find("\"spec_id\":\"v103-superseded\"")
        .expect("superseded record is rendered");
    let replacement = rendered
        .find("\"spec_id\":\"v104-replacement\"")
        .expect("replacement record is rendered");

    assert!(
        planned < passed && passed < deferred && deferred < superseded && superseded < replacement
    );

    store.save(root.path()).expect("state document saves");
    assert_eq!(
        fs::read_to_string(state_file(&root, "specs.json")).expect("primary document is readable"),
        rendered
    );
    assert!(
        !state_file(&root, "specs.json.tmp").exists(),
        "temporary document is promoted"
    );

    let loaded = SpecStateStore::load_or_default(root.path()).expect("saved document loads");

    assert_eq!(
        loaded
            .get("v101-passed")
            .expect("passed record is loaded")
            .state
            .as_str(),
        "passed"
    );
    assert_eq!(
        loaded
            .get("v102-deferred")
            .expect("deferred record is loaded")
            .deferred_reason
            .as_deref(),
        Some("waiting on V64 proof")
    );
    assert_eq!(
        loaded
            .get("v103-superseded")
            .expect("superseded record is loaded")
            .superseded_by
            .as_deref(),
        Some("v104-replacement")
    );
    assert_eq!(loaded.to_json().expect("loaded state renders"), rendered);
}

#[test]
fn spec_state_store_recovers_a_valid_temp_file_when_primary_is_missing() {
    let root = TempProjectRoot::new();
    let document = "{\"schema\":1,\"specs\":[{\"spec_id\":\"v110-temp-recovery\",\"state\":\"passed\",\"deferred_reason\":null,\"superseded_by\":null}]}";
    write_state_file(&root, "specs.json.tmp", document);

    let loaded = SpecStateStore::load_or_default(root.path()).expect("temporary document recovers");

    assert_eq!(
        loaded
            .get("v110-temp-recovery")
            .expect("recovered record is loaded")
            .state
            .as_str(),
        "passed"
    );
    assert_eq!(
        fs::read_to_string(state_file(&root, "specs.json"))
            .expect("temporary document is promoted"),
        document
    );
    assert!(
        !state_file(&root, "specs.json.tmp").exists(),
        "temporary document is removed after recovery"
    );
}

#[test]
fn spec_state_store_recovers_a_valid_temp_file_when_primary_is_malformed() {
    let root = TempProjectRoot::new();
    let document = "{\"schema\":1,\"specs\":[{\"spec_id\":\"v111-temp-recovery\",\"state\":\"deferred\",\"deferred_reason\":\"waiting on proof\",\"superseded_by\":null}]}";
    write_state_file(&root, "specs.json", "{not valid json");
    write_state_file(&root, "specs.json.tmp", document);

    let loaded = SpecStateStore::load_or_default(root.path()).expect("temporary document recovers");

    assert_eq!(
        loaded
            .get("v111-temp-recovery")
            .expect("recovered record is loaded")
            .deferred_reason
            .as_deref(),
        Some("waiting on proof")
    );
    assert_eq!(
        fs::read_to_string(state_file(&root, "specs.json"))
            .expect("temporary document is promoted"),
        document
    );
    assert!(
        !state_file(&root, "specs.json.tmp").exists(),
        "temporary document is removed after recovery"
    );
}

#[test]
fn spec_state_store_keeps_a_valid_primary_when_a_temporary_file_is_stale_or_malformed() {
    let root = TempProjectRoot::new();
    let primary = "{\"schema\":1,\"specs\":[{\"spec_id\":\"v112-primary\",\"state\":\"passed\",\"deferred_reason\":null,\"superseded_by\":null}]}";
    write_state_file(&root, "specs.json", primary);
    write_state_file(&root, "specs.json.tmp", "{not valid json");

    let loaded = SpecStateStore::load_or_default(root.path()).expect("primary document wins");

    assert_eq!(
        loaded
            .get("v112-primary")
            .expect("primary record is loaded")
            .state,
        SpecRunState::Passed
    );
    assert_eq!(
        fs::read_to_string(state_file(&root, "specs.json")).expect("primary stays intact"),
        primary
    );
    assert!(
        state_file(&root, "specs.json.tmp").exists(),
        "a valid primary does not reinterpret a stale recovery file"
    );
}

#[test]
fn spec_state_store_rejects_duplicate_and_invalid_spec_ids() {
    assert_document_is_rejected(
        "{\"schema\":1,\"specs\":[{\"spec_id\":\"v120-duplicate\",\"state\":\"planned\",\"deferred_reason\":null,\"superseded_by\":null},{\"spec_id\":\"v120-duplicate\",\"state\":\"ready\",\"deferred_reason\":null,\"superseded_by\":null}]}",
    );
    assert_document_is_rejected(
        "{\"schema\":1,\"specs\":[{\"spec_id\":\"invalid-spec-id\",\"state\":\"planned\",\"deferred_reason\":null,\"superseded_by\":null}]}",
    );
    assert_document_is_rejected(
        "{\"schema\":2,\"specs\":[{\"spec_id\":\"v120-unknown-schema\",\"state\":\"planned\",\"deferred_reason\":null,\"superseded_by\":null}]}",
    );
    assert_document_is_rejected("{\"schema\":1,\"specs\":[],\"unexpected\":true}");
}

#[test]
fn spec_state_store_rejects_deferred_records_without_a_reason() {
    assert_document_is_rejected(
        "{\"schema\":1,\"specs\":[{\"spec_id\":\"v121-missing-reason\",\"state\":\"deferred\",\"deferred_reason\":null,\"superseded_by\":null}]}",
    );
    assert_document_is_rejected(
        "{\"schema\":1,\"specs\":[{\"spec_id\":\"v121-conflicting-metadata\",\"state\":\"passed\",\"deferred_reason\":\"not allowed\",\"superseded_by\":null}]}",
    );
}

#[test]
fn spec_state_store_rejects_malformed_documents_without_a_recovery_file() {
    assert_document_is_rejected("{not valid json");
    assert_document_is_rejected(
        "{\"schema\":1,\"specs\":[{\"spec_id\":\"v123-invalid-unicode\",\"state\":\"deferred\",\"deferred_reason\":\"\\uD834 \\uDD1E\",\"superseded_by\":null}]}",
    );
}

#[test]
fn spec_state_store_rejects_supersession_references_to_missing_or_same_records() {
    assert_document_is_rejected(
        "{\"schema\":1,\"specs\":[{\"spec_id\":\"v122-missing-replacement\",\"state\":\"superseded\",\"deferred_reason\":null,\"superseded_by\":\"v123-not-present\"}]}",
    );
    assert_document_is_rejected(
        "{\"schema\":1,\"specs\":[{\"spec_id\":\"v124-self-superseded\",\"state\":\"superseded\",\"deferred_reason\":null,\"superseded_by\":\"v124-self-superseded\"}]}",
    );
}

#[test]
fn spec_state_store_round_trips_escaped_lifecycle_metadata() {
    let root = TempProjectRoot::new();
    let reason = "waiting for \"quoted\" path\\name\nand unicode ✓";
    let mut store = SpecStateStore::new();
    store
        .insert(
            SpecLifecycle::new("v125-escaped-metadata")
                .deferred(reason)
                .expect("escaped deferred metadata is valid"),
        )
        .expect("record is valid");

    store.save(root.path()).expect("state document saves");
    let rendered =
        fs::read_to_string(state_file(&root, "specs.json")).expect("state document is readable");
    assert!(rendered.contains("\\\"quoted\\\""));
    assert!(rendered.contains("path\\\\name\\nand unicode ✓"));

    let loaded = SpecStateStore::load_or_default(root.path()).expect("state document loads");
    assert_eq!(
        loaded
            .get("v125-escaped-metadata")
            .expect("escaped record is loaded")
            .deferred_reason
            .as_deref(),
        Some(reason)
    );
}

#[test]
fn parent_issue_closes_after_children_terminal() {
    use ParentIssueStatus::*;

    const DECOMPOSITION_COMMENT: &str = "<!-- autospec-parent-decomposition:begin -->\nParent issue #1899 was decomposed into child implementation issues:\n- #1900\n- #1901\nState: `quarantined-parent-decomposed`.\n<!-- autospec-parent-decomposition:end -->";
    const COMPLETION_SUMMARY: &str = "<!-- autospec-parent-complete:begin -->\nAll child implementation issues for parent #1899 reached a terminal state:\n- #1900\n- #1901\n\nClosing parent issue automatically.\n<!-- autospec-parent-complete:end -->";
    let mut store = SpecStateStore::new();
    let decomposition = store
        .record_parent_decomposition(1899, vec![1900, 1901], true)
        .expect("quarantined parent decomposition is recorded");

    assert_eq!(decomposition.comment_body, DECOMPOSITION_COMMENT);
    assert_eq!(
        store.parent_issue_status(1899),
        Some(QuarantinedParentDecomposed)
    );
    assert!(store
        .record_child_terminal(1900)
        .expect("first child terminal state is recorded")
        .is_empty());
    assert_eq!(
        store.parent_issue_status(1899),
        Some(QuarantinedParentDecomposed)
    );
    let completed = store
        .record_child_terminal(1901)
        .expect("last child terminal state is recorded");

    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].parent_issue, 1899);
    assert_eq!(completed[0].completion_summary, COMPLETION_SUMMARY);
    assert_eq!(store.parent_issue_status(1899), Some(CompleteButStale));
    store
        .record_parent_closed(1899)
        .expect("parent close confirmation is recorded");
    assert_eq!(store.parent_issue_status(1899), Some(Closed));
}
