#[path = "../src/commands/autonomous/accountability.rs"]
#[allow(dead_code)]
mod accountability;

use accountability::{
    AccountabilityEvent, AccountabilityStore, EventKind, Evidence, LaunchDescriptor,
    LeaseGeneration, RepositoryIdentity, RunIdentity, RunNonce,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let serial = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "autospec-accountability-events-{}-{serial}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn store() -> (Fixture, AccountabilityStore) {
    let root = Fixture::new();
    let identity = RunIdentity::derive(
        RepositoryIdentity::parse("acme/widgets").unwrap(),
        RunNonce::parse("00112233445566778899aabbccddeeff").unwrap(),
        LeaseGeneration::new(1).unwrap(),
    );
    let mut store = AccountabilityStore::open(root.path()).unwrap();
    store
        .begin_launch(
            LaunchDescriptor::new(identity, "Ship the run", "Keep one accountable record").unwrap(),
        )
        .unwrap();
    (root, store)
}

fn event(kind: EventKind, what: &str) -> AccountabilityEvent {
    AccountabilityEvent::new(
        kind,
        what,
        "The lifecycle boundary must be durable before the next mutation",
        vec![Evidence::outcome("local journal fsync completed")],
    )
    .unwrap()
}

#[test]
fn lifecycle_events_are_local_first_and_projection_degradation_is_visible() {
    let (root, mut store) = store();
    store
        .append_event(event(
            EventKind::WorkSelected { issue: Some(42) },
            "Selected issue 42",
        ))
        .unwrap();
    store
        .append_event(event(
            EventKind::IssueClaimed { issue: 42 },
            "Claimed issue 42",
        ))
        .unwrap();
    let projection = store.render().unwrap();

    let journal = fs::read_to_string(root.path().join("accountability-events.jsonl")).unwrap();
    assert!(journal.contains("work_selected"));
    assert!(journal.contains("issue_claimed"));
    assert_eq!(store.status().pending_projection_count, 1);
    assert_eq!(store.status().desired_high_watermark, 2);
    assert_eq!(projection.desired_high_watermark, 2);
}

#[test]
fn only_a_merged_event_describes_a_deliverable_as_implemented() {
    let (_root, mut store) = store();
    store
        .append_event(event(
            EventKind::PullRequestOpened { pull_request: 9 },
            "Opened PR 9",
        ))
        .unwrap();
    let open = store.render().unwrap().markdown;
    assert!(open.contains("PR #9: opened, not yet implemented"));
    assert!(!open.contains("PR #9: implemented"));

    store
        .append_event(event(EventKind::Merged { pull_request: 9 }, "Merged PR 9"))
        .unwrap();
    let merged = store.render().unwrap().markdown;
    assert!(merged.contains("PR #9: merged into the target branch"));
}

#[test]
fn typed_terminal_and_recovery_boundaries_round_trip() {
    let (root, mut store) = store();
    for (kind, what) in [
        (
            EventKind::ReviewStarted { pull_request: 9 },
            "Review started",
        ),
        (EventKind::Verified, "Verification passed"),
        (EventKind::Quarantined { issue: 42 }, "Issue quarantined"),
        (EventKind::Parked, "Run parked"),
        (EventKind::Stopped, "Run stopped"),
        (EventKind::Completed, "Run completed"),
    ] {
        store.append_event(event(kind, what)).unwrap();
    }
    drop(store);

    let reopened = AccountabilityStore::open(root.path()).unwrap();
    assert_eq!(reopened.status().event_count, 6);
}
