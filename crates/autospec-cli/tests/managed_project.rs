#[path = "../src/commands/managed_project.rs"]
mod managed_project;

use autospec_core::managed_project::{
    ProductKey, RelationshipEdge, RelationshipEvidence, RelationshipKind, RelationshipState,
    RepositoryRecord,
};
use managed_project::ManagedProjectStore;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let serial = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "autospec-managed-project-{name}-{}-{serial}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn state_path(&self, name: &str) -> PathBuf {
        self.0.join("projects").join("autospec").join(name)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn key(value: &str) -> ProductKey {
    ProductKey::new(value).unwrap()
}

fn repository(repository: &str, entry_kind: &str) -> RepositoryRecord {
    RepositoryRecord {
        repository: repository.to_owned(),
        entry_kind: entry_kind.to_owned(),
    }
}

fn edge() -> RelationshipEdge {
    RelationshipEdge {
        product_key: key("autospec"),
        kind: RelationshipKind::DependsOn,
        source: "berlinguyinca/autospec".to_owned(),
        target: "berlinguyinca/autospec-node".to_owned(),
        evidence: RelationshipEvidence {
            kind: "manifest-dependency".to_owned(),
            location: "Cargo.toml".to_owned(),
            discovered_at: "2026-08-27T00:00:00Z".to_owned(),
            confidence: 100,
        },
        state: RelationshipState::Active,
    }
}

fn add_item(issue_url: &str) -> String {
    format!("project:item-add:PV_123:{issue_url}")
}

#[test]
fn store_reopens_repository_edge_and_pending_projection_from_journal() {
    let fixture = Fixture::new("reopen");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    store
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    store.record_edge(edge()).unwrap();
    store
        .enqueue_projection(add_item(
            "https://github.com/berlinguyinca/autospec/issues/42",
        ))
        .unwrap();
    drop(store);

    let reopened = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    assert_eq!(reopened.snapshot().repositories.len(), 1);
    assert_eq!(reopened.snapshot().relationships.len(), 1);
    assert_eq!(reopened.snapshot().pending_projections.len(), 1);
}

#[test]
fn store_duplicate_event_keys_are_no_ops() {
    let fixture = Fixture::new("dedupe");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    let repository = repository("berlinguyinca/autospec", "explicit-seed");
    let edge = edge();
    let projection = add_item("https://github.com/berlinguyinca/autospec/issues/42");

    store.record_repository(repository.clone()).unwrap();
    store.record_repository(repository).unwrap();
    store.record_edge(edge.clone()).unwrap();
    store.record_edge(edge).unwrap();
    store.enqueue_projection(projection.clone()).unwrap();
    store.enqueue_projection(projection).unwrap();

    assert_eq!(store.snapshot().repositories.len(), 1);
    assert_eq!(store.snapshot().relationships.len(), 1);
    assert_eq!(store.snapshot().pending_projections.len(), 1);
    assert_eq!(
        fs::read_to_string(fixture.state_path("events.jsonl"))
            .unwrap()
            .lines()
            .count(),
        3
    );
}

#[test]
fn store_two_writers_refresh_under_lock_without_losing_events() {
    let fixture = Fixture::new("two-writers");
    let mut first = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    let mut second = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();

    first
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    second
        .record_repository(repository("berlinguyinca/autospec-node", "explicit-seed"))
        .unwrap();
    drop((first, second));

    let reopened = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    assert_eq!(reopened.snapshot().repositories.len(), 2);
    assert_eq!(
        fs::read_to_string(fixture.state_path("events.jsonl"))
            .unwrap()
            .lines()
            .count(),
        2
    );
}

#[test]
fn store_partial_append_failure_rolls_back_before_same_instance_retry() {
    let fixture = Fixture::new("append-rollback");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    store
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    let journal_path = fixture.state_path("events.jsonl");
    let length_before = fs::metadata(&journal_path).unwrap().len();
    store.fail_next_append_after(17);

    assert!(store
        .record_repository(repository("berlinguyinca/autospec-node", "explicit-seed"))
        .is_err());
    assert_eq!(fs::metadata(&journal_path).unwrap().len(), length_before);
    store
        .record_repository(repository("berlinguyinca/autospec-node", "explicit-seed"))
        .unwrap();
    drop(store);

    let reopened = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    assert_eq!(reopened.snapshot().repositories.len(), 2);
    assert_eq!(fs::read_to_string(journal_path).unwrap().lines().count(), 2);
}

#[test]
fn store_ack_projection_is_retryable_but_unknown_keys_fail_closed() {
    let fixture = Fixture::new("ack");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    let projection = add_item("https://github.com/berlinguyinca/autospec/issues/42");
    store.enqueue_projection(projection.clone()).unwrap();

    store.ack_projection(&projection).unwrap();
    store.ack_projection(&projection).unwrap();
    assert!(store
        .ack_projection("project:item-add:PV_123:missing")
        .is_err());
    assert!(store.snapshot().pending_projections.is_empty());
}

#[test]
fn store_discards_only_a_truncated_jsonl_tail_and_rebuilds_the_snapshot() {
    let fixture = Fixture::new("truncated-tail");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    store
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    drop(store);
    fs::OpenOptions::new()
        .append(true)
        .open(fixture.state_path("events.jsonl"))
        .unwrap()
        .write_all(br#"{"sequence":2,"kind":"projection-enqueued""#)
        .unwrap();
    fs::remove_file(fixture.state_path("binding.json")).unwrap();

    let reopened = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    assert_eq!(reopened.snapshot().repositories.len(), 1);
    assert!(fs::read_to_string(fixture.state_path("events.jsonl"))
        .unwrap()
        .ends_with('\n'));
    assert!(fixture.state_path("binding.json").is_file());
}

#[test]
fn store_rejects_empty_interior_journal_lines() {
    let fixture = Fixture::new("empty-journal-line");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    store
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    drop(store);
    fs::OpenOptions::new()
        .append(true)
        .open(fixture.state_path("events.jsonl"))
        .unwrap()
        .write_all(b"\n")
        .unwrap();

    assert!(ManagedProjectStore::open(fixture.path(), &key("autospec")).is_err());
}

#[test]
fn store_rebuilds_a_stale_snapshot_from_the_journal() {
    let fixture = Fixture::new("stale-snapshot");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    store
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    drop(store);
    let binding_path = fixture.state_path("binding.json");
    let mut binding: serde_json::Value =
        serde_json::from_slice(&fs::read(&binding_path).unwrap()).unwrap();
    binding["repositories"] = serde_json::json!([]);
    fs::write(&binding_path, serde_json::to_vec(&binding).unwrap()).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&binding_path, fs::Permissions::from_mode(0o600)).unwrap();

    let reopened = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    assert_eq!(reopened.snapshot().repositories.len(), 1);
}

#[test]
fn store_missing_journal_fails_closed_without_overwriting_a_nonempty_binding() {
    let fixture = Fixture::new("missing-journal");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    store
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    drop(store);
    let binding_path = fixture.state_path("binding.json");
    let binding_before = fs::read(&binding_path).unwrap();
    fs::remove_file(fixture.state_path("events.jsonl")).unwrap();

    assert!(ManagedProjectStore::open(fixture.path(), &key("autospec")).is_err());
    assert_eq!(fs::read(binding_path).unwrap(), binding_before);
    assert!(!fixture.state_path("events.jsonl").exists());
}

#[test]
fn store_zero_length_journal_fails_closed_without_modifying_state() {
    let fixture = Fixture::new("empty-journal");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    store
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    drop(store);
    let binding_path = fixture.state_path("binding.json");
    let journal_path = fixture.state_path("events.jsonl");
    let binding_before = fs::read(&binding_path).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&journal_path)
        .unwrap();

    assert!(ManagedProjectStore::open(fixture.path(), &key("autospec")).is_err());
    assert_eq!(fs::read(binding_path).unwrap(), binding_before);
    assert!(fs::read(journal_path).unwrap().is_empty());
}

#[test]
fn store_valid_journal_prefix_behind_snapshot_fails_closed_without_modifying_state() {
    let fixture = Fixture::new("journal-prefix");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    store
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    store
        .record_repository(repository("berlinguyinca/autospec-node", "explicit-seed"))
        .unwrap();
    drop(store);
    let binding_path = fixture.state_path("binding.json");
    let journal_path = fixture.state_path("events.jsonl");
    let binding_before = fs::read(&binding_path).unwrap();
    let journal = fs::read_to_string(&journal_path).unwrap();
    let first_line = format!("{}\n", journal.lines().next().unwrap());
    fs::write(&journal_path, first_line.as_bytes()).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&journal_path, fs::Permissions::from_mode(0o600)).unwrap();

    assert!(ManagedProjectStore::open(fixture.path(), &key("autospec")).is_err());
    assert_eq!(fs::read(binding_path).unwrap(), binding_before);
    assert_eq!(fs::read(journal_path).unwrap(), first_line.as_bytes());
}

#[test]
fn store_rejects_mismatched_binding_and_edge_product_keys() {
    let fixture = Fixture::new("mismatched-key");
    let store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    drop(store);
    let binding_path = fixture.state_path("binding.json");
    let mut binding: serde_json::Value =
        serde_json::from_slice(&fs::read(&binding_path).unwrap()).unwrap();
    binding["product_key"] = serde_json::json!("other");
    fs::write(&binding_path, serde_json::to_vec(&binding).unwrap()).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&binding_path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(ManagedProjectStore::open(fixture.path(), &key("autospec")).is_err());

    let other = Fixture::new("mismatched-edge");
    let mut store = ManagedProjectStore::open(other.path(), &key("autospec")).unwrap();
    let mut wrong_edge = edge();
    wrong_edge.product_key = key("other");
    assert!(store.record_edge(wrong_edge).is_err());
}

#[test]
#[cfg(unix)]
fn store_uses_private_state_and_rejects_public_binding_files() {
    let fixture = Fixture::new("private-state");
    let store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    drop(store);
    let project_dir = fixture
        .state_path("binding.json")
        .parent()
        .unwrap()
        .to_path_buf();
    assert_eq!(
        fs::metadata(&project_dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for name in ["binding.json", "events.jsonl", "binding.lock"] {
        assert_eq!(
            fs::metadata(fixture.state_path(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    fs::set_permissions(
        fixture.state_path("binding.json"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(ManagedProjectStore::open(fixture.path(), &key("autospec")).is_err());
}

#[test]
#[cfg(unix)]
fn store_rejects_a_public_product_lock_file() {
    let fixture = Fixture::new("public-lock");
    let store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    drop(store);
    fs::set_permissions(
        fixture.state_path("binding.lock"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();

    assert!(ManagedProjectStore::open(fixture.path(), &key("autospec")).is_err());
}

#[test]
#[cfg(unix)]
fn store_rejects_symlinked_product_state_directories() {
    let fixture = Fixture::new("symlink-state");
    fs::create_dir(&fixture.0).unwrap();
    fs::set_permissions(&fixture.0, fs::Permissions::from_mode(0o700)).unwrap();
    let projects = fixture.0.join("projects");
    fs::create_dir(&projects).unwrap();
    fs::set_permissions(&projects, fs::Permissions::from_mode(0o700)).unwrap();
    let outside = fixture.0.join("outside");
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, projects.join("autospec")).unwrap();

    assert!(ManagedProjectStore::open(fixture.path(), &key("autospec")).is_err());
}
