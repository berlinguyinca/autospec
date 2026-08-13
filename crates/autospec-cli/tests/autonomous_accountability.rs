#[path = "../src/commands/autonomous/accountability.rs"]
#[allow(dead_code)]
mod accountability;

use accountability::{
    AccountabilityEvent, AccountabilityStore, EventKind, Evidence, LaunchDescriptor,
    LeaseGeneration, RecoveryManifest, RepositoryIdentity, RunIdentity, RunNonce,
};
use std::fs;
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
            "autospec-accountability-{name}-{}-{serial}",
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

fn identity(nonce: &str, generation: u64) -> RunIdentity {
    RunIdentity::derive(
        RepositoryIdentity::parse("acme/widgets").unwrap(),
        RunNonce::parse(nonce).unwrap(),
        LeaseGeneration::new(generation).unwrap(),
    )
}

fn launch(run: RunIdentity) -> LaunchDescriptor {
    LaunchDescriptor::new(
        run,
        "Ship checkout-bound autonomous accountability",
        "Operators need one durable, understandable record for each conductor generation.",
    )
    .unwrap()
}

fn event(kind: EventKind, suffix: &str) -> AccountabilityEvent {
    AccountabilityEvent::new(
        kind,
        format!("Built slice {suffix}"),
        format!("The run requires capability {suffix}"),
        vec![Evidence::outcome(format!("verified {suffix}"))],
    )
    .unwrap()
}

#[test]
fn run_identity_is_stable_and_rejects_incomplete_parts() {
    let first = identity("00112233445566778899aabbccddeeff", 7);
    let again = identity("00112233445566778899aabbccddeeff", 7);
    let successor = identity("ffeeddccbbaa99887766554433221100", 8);

    assert_eq!(first.run_id(), again.run_id());
    assert_ne!(first.run_id(), successor.run_id());
    assert_eq!(first.run_id().len(), 64);
    assert!(RepositoryIdentity::parse("widgets").is_err());
    assert!(RunNonce::parse("short").is_err());
    assert!(LeaseGeneration::new(0).is_err());
}

#[test]
fn launch_identity_can_be_adopted_but_not_replaced_in_place() {
    let fixture = Fixture::new("identity-transition");
    let mut store = AccountabilityStore::open(fixture.path()).unwrap();
    let current = launch(identity("00112233445566778899aabbccddeeff", 7));

    store.begin_launch(current.clone()).unwrap();
    store.begin_launch(current).unwrap();
    let error = store
        .begin_launch(launch(identity("ffeeddccbbaa99887766554433221100", 8)))
        .unwrap_err();

    assert!(error.to_string().contains("different run identity"));
}

#[test]
#[cfg(unix)]
fn store_publishes_private_atomic_state() {
    let fixture = Fixture::new("private-state");
    let mut store = AccountabilityStore::open(fixture.path()).unwrap();
    store
        .begin_launch(launch(identity("00112233445566778899aabbccddeeff", 7)))
        .unwrap();
    store
        .append_event(event(EventKind::RunStarted, "identity"))
        .unwrap();

    assert_eq!(
        fs::metadata(fixture.path()).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for name in [
        "accountability.json",
        "accountability-events.jsonl",
        "accountability-outbox.jsonl",
    ] {
        let path = fixture.path().join(name);
        assert!(path.is_file(), "missing {}", path.display());
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert!(fs::read_dir(fixture.path()).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains(".tmp")));
}

#[test]
#[cfg(unix)]
fn store_rejects_preexisting_public_or_symlinked_state() {
    let public_fixture = Fixture::new("public-state");
    fs::create_dir(&public_fixture.0).unwrap();
    fs::set_permissions(&public_fixture.0, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(AccountabilityStore::open(public_fixture.path()).is_err());

    let symlink_fixture = Fixture::new("symlink-state");
    fs::create_dir(&symlink_fixture.0).unwrap();
    fs::set_permissions(&symlink_fixture.0, fs::Permissions::from_mode(0o700)).unwrap();
    let outside = symlink_fixture.0.with_extension("outside");
    fs::write(&outside, "outside").unwrap();
    std::os::unix::fs::symlink(&outside, symlink_fixture.0.join("accountability.json")).unwrap();
    assert!(AccountabilityStore::open(symlink_fixture.path()).is_err());
    fs::remove_file(outside).unwrap();
}

#[test]
fn events_have_monotonic_sequences_and_content_bound_ids() {
    let fixture = Fixture::new("monotonic-events");
    let mut store = AccountabilityStore::open(fixture.path()).unwrap();
    store
        .begin_launch(launch(identity("00112233445566778899aabbccddeeff", 7)))
        .unwrap();

    let first = store
        .append_event(event(EventKind::RunStarted, "one"))
        .unwrap();
    let second = store
        .append_event(event(EventKind::IssueClaimed { issue: 42 }, "two"))
        .unwrap();

    assert_eq!((first.seq, second.seq), (1, 2));
    assert_ne!(first.event_id, second.event_id);
    assert_eq!(first.event_id.len(), 64);
    assert_eq!(store.status().event_count, 2);
}

#[test]
fn reopening_discards_only_an_unterminated_event_tail() {
    let fixture = Fixture::new("partial-tail");
    let mut store = AccountabilityStore::open(fixture.path()).unwrap();
    store
        .begin_launch(launch(identity("00112233445566778899aabbccddeeff", 7)))
        .unwrap();
    let first = store
        .append_event(event(EventKind::RunStarted, "one"))
        .unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(fixture.path().join("accountability-events.jsonl"))
        .unwrap()
        .write_all(br#"{"seq":2,"event_id":"torn""#)
        .unwrap();
    drop(store);

    let mut reopened = AccountabilityStore::open(fixture.path()).unwrap();
    assert_eq!(reopened.status().event_count, 1);
    let second = reopened
        .append_event(event(EventKind::Blocked, "recovered"))
        .unwrap();
    assert_eq!(second.seq, first.seq + 1);
    assert!(
        fs::read_to_string(fixture.path().join("accountability-events.jsonl"))
            .unwrap()
            .ends_with('\n')
    );
}

#[test]
fn projection_ack_requires_matching_revision_digest_and_high_watermark() {
    let fixture = Fixture::new("projection");
    let mut store = AccountabilityStore::open(fixture.path()).unwrap();
    store
        .begin_launch(launch(identity("00112233445566778899aabbccddeeff", 7)))
        .unwrap();
    store
        .append_event(event(EventKind::RunStarted, "one"))
        .unwrap();
    let projection = store.render().unwrap();

    assert_eq!(projection.revision, 1);
    assert_eq!(projection.desired_high_watermark, 1);
    assert_eq!(projection.digest.len(), 64);
    assert_eq!(store.status().pending_projection_count, 1);
    assert!(store
        .ack_projection(
            projection.revision,
            "wrong",
            projection.desired_high_watermark
        )
        .is_err());
    store
        .ack_projection(
            projection.revision,
            &projection.digest,
            projection.desired_high_watermark,
        )
        .unwrap();
    assert_eq!(store.status().acknowledged_high_watermark, 1);
    assert_eq!(store.status().pending_projection_count, 0);
}

#[test]
fn renderer_is_bounded_and_aggregates_after_twenty_five_work_nodes() {
    let fixture = Fixture::new("bounded-render");
    let mut store = AccountabilityStore::open(fixture.path()).unwrap();
    store
        .begin_launch(launch(identity("00112233445566778899aabbccddeeff", 7)))
        .unwrap();
    for issue in 1..=80 {
        store
            .append_event(
                AccountabilityEvent::new(
                    EventKind::IssueClaimed { issue },
                    format!("Claimed issue {issue} {}", "x".repeat(900)),
                    "It is an ordered part of the requested outcome.",
                    vec![Evidence::github_url(format!(
                        "https://github.com/acme/widgets/issues/{issue}"
                    ))
                    .unwrap()],
                )
                .unwrap(),
            )
            .unwrap();
    }

    let body = store.render().unwrap().markdown;
    assert!(body.len() <= 48 * 1024, "rendered {} bytes", body.len());
    assert!(body.contains("older work items"));
    assert_eq!(body.matches("work_").count(), 25);
}

#[test]
fn sanitizer_excludes_secret_shaped_text_paths_and_markdown_breakouts() {
    let fixture = Fixture::new("sanitizer");
    let mut store = AccountabilityStore::open(fixture.path()).unwrap();
    store
        .begin_launch(launch(identity("00112233445566778899aabbccddeeff", 7)))
        .unwrap();
    store
        .append_event(
            AccountabilityEvent::new(
                EventKind::Verified,
                "Checked `build` ] --> hacked %%{init:evil} <!-- --><script>alert(1)</script>",
            "Keep token ghp_abcdefghijklmnopqrstuvwxyzABCDEFGHIJ and /Users/alice/private private\u{0007}",
                vec![
                    Evidence::command("cargo", ["test", "--workspace", "super-secret-token"])
                        .unwrap(),
                    Evidence::repository_path("docs/spec.md").unwrap(),
                    Evidence::github_url(
                        "https://user:pass@github.com/acme/widgets/issues/1?token=x#frag",
                    )
                    .unwrap(),
                ],
            )
            .unwrap(),
        )
        .unwrap();

    assert!(Evidence::repository_path("/Users/alice/secret").is_err());
    let body = store.render().unwrap().markdown;
    for forbidden in [
        "ghp_",
        "super-secret-token",
        "user:pass",
        "?token=",
        "#frag",
        "<script>",
        "init:evil",
        "/Users/alice/private",
    ] {
        assert!(!body.contains(forbidden), "leaked {forbidden}: {body}");
    }
    assert!(body.contains("cargo test --workspace redacted"));
}

#[test]
fn comprehension_fixture_renders_two_diagrams_and_what_why_evidence_paragraphs() {
    let fixture = Fixture::new("comprehension");
    let mut store = AccountabilityStore::open(fixture.path()).unwrap();
    store
        .begin_launch(launch(identity("00112233445566778899aabbccddeeff", 7)))
        .unwrap();
    for (kind, label, proof) in [
        (
            EventKind::IssueClaimed { issue: 21 },
            "claimed runtime refresh",
            "issue 21 defines the boundary",
        ),
        (
            EventKind::Merged { pull_request: 44 },
            "merged runtime refresh",
            "PR 44 passed verification",
        ),
        (
            EventKind::Failed,
            "failed projection retry",
            "the edit remained in the outbox",
        ),
        (
            EventKind::Blocked,
            "blocked duplicate epic",
            "two exact markers were found",
        ),
    ] {
        store
            .append_event(
                AccountabilityEvent::new(
                    kind,
                    label,
                    "This records the decision a maintainer must understand.",
                    vec![Evidence::outcome(proof)],
                )
                .unwrap(),
            )
            .unwrap();
    }

    let body = store.render().unwrap().markdown;
    assert_eq!(body.matches("```mermaid").count(), 2);
    assert!(body.contains("flowchart TD"));
    assert!(body.contains("stateDiagram-v2"));
    assert!(body.contains("**What:** merged runtime refresh."));
    assert!(body.contains("**Why:** This records the decision a maintainer must understand."));
    assert!(body.contains("**Evidence:** PR 44 passed verification."));
    assert!(body.contains("failed projection retry"));
    assert!(body.contains("blocked duplicate epic"));
}

#[test]
fn recovery_manifest_is_typed_versioned_and_rejects_tampering() {
    let manifest = RecoveryManifest::new(
        identity("00112233445566778899aabbccddeeff", 7),
        3135,
        "https://github.com/acme/widgets/issues/3135",
        9,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        27,
        3,
    )
    .unwrap();
    let encoded = manifest.to_json();
    let decoded = RecoveryManifest::parse(&encoded).unwrap();

    assert_eq!(decoded, manifest);
    let wrong_schema = encoded.replace("\"schema\":1", "\"schema\":99");
    assert!(RecoveryManifest::parse(&wrong_schema).is_err());
    let wrong_repository = encoded.replace("acme/widgets", "other/widgets");
    assert!(RecoveryManifest::parse_for_repository(
        &wrong_repository,
        &RepositoryIdentity::parse("acme/widgets").unwrap()
    )
    .is_err());
}

#[test]
fn resume_reconstructs_a_chained_segment_and_records_the_source_epic() {
    let fixture = Fixture::new("remote-resume");
    let manifest = RecoveryManifest::new(
        identity("00112233445566778899aabbccddeeff", 7),
        3135,
        "https://github.com/acme/widgets/issues/3135",
        9,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        27,
        3,
    )
    .unwrap();
    let mut store = AccountabilityStore::open(fixture.path()).unwrap();

    let record = store
        .resume_from_manifest(
            manifest,
            "Resume checkout-bound autonomous accountability",
            "The local journal was unavailable and the managed epic is the recovery source.",
        )
        .unwrap();
    let status = store.status();

    assert_eq!(record.seq, 28);
    assert_eq!(record.kind, EventKind::ResumedFromEpic { epic: 3135 });
    assert_eq!(status.journal_segment, 4);
    assert_eq!(
        status.prior_remote_digest.as_deref(),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
    assert_eq!(status.acknowledged_high_watermark, 27);
    assert_eq!(status.epic_number, Some(3135));
    let line = fs::read_to_string(fixture.path().join("accountability-events.jsonl")).unwrap();
    assert!(line.contains("resumed_from_epic"));
    assert!(line.contains("\"epic\":3135"));
}

use std::io::Write;
