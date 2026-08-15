use super::*;
use super::{publication::*, retirement::*};
use crate::commands::claim::ClaimMutationIdentity;
use std::sync::atomic::{AtomicU64, Ordering};

#[test]
fn windows_rename_buffer_includes_the_complete_abi_header_and_utf16_name() {
    let root_directory = 0x1234usize as *mut std::ffi::c_void;
    let destination = "42.json".encode_utf16().collect::<Vec<_>>();

    let buffer = build_windows_file_rename_info_buffer(root_directory, &destination)
        .expect("rename information buffer");

    assert_eq!(
        buffer.len(),
        std::mem::size_of::<WindowsFileRenameInfo>()
            + destination.len() * std::mem::size_of::<u16>()
            + std::mem::size_of::<u16>()
    );
    let info = buffer.as_ptr().cast::<WindowsFileRenameInfo>();
    // SAFETY: the builder returned an initialized header followed by the encoded name.
    unsafe {
        assert_eq!((*info).flags, 0);
        assert_eq!((*info).root_directory, root_directory);
        assert_eq!(
            (*info).file_name_length,
            (destination.len() * std::mem::size_of::<u16>()) as u32
        );
        assert_eq!(
            std::slice::from_raw_parts((*info).file_name.as_ptr(), destination.len()),
            destination
        );
        assert_eq!(*(*info).file_name.as_ptr().add(destination.len()), 0);
    }
}

pub(super) static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

pub(super) struct Fixture {
    pub(super) root: std::path::PathBuf,
}

impl Fixture {
    pub(super) fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "autospec-heartbeat-portable-{name}-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).expect("private heartbeat root");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
                .expect("private root permissions");
        }
        Self { root }
    }

    pub(super) fn document(&self, claim_id: &str, session_id: Option<&str>) -> Vec<u8> {
        let session =
            session_id.map_or_else(String::new, |value| format!(r#","session_id":"{value}""#));
        format!(
            r#"{{"repo":"owner/repo","issue":"42","worker_id":"worker-a","branch":"feat/worker","pr":"","claim_id":"{claim_id}","step":"claimed","ts":1,"ttl_seconds":10,"pid":7,"nonce":"nonce-{claim_id}","host":"host-a","boot_id":"boot-a","process_start":"9"{session}}}"#
        )
        .into_bytes()
    }

    pub(super) fn issue_path(&self) -> std::path::PathBuf {
        self.root
            .join(crate::commands::autonomous::drain::repository_progress_key(
                "owner/repo",
            ))
            .join("42.json")
    }

    pub(super) fn repo_path(&self) -> std::path::PathBuf {
        self.root.join(repository_progress_key("owner/repo"))
    }

    pub(super) fn staging_paths(&self) -> Vec<std::path::PathBuf> {
        std::fs::read_dir(self.repo_path())
            .expect("repository entries")
            .filter_map(Result::ok)
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.starts_with(".autospec-heartbeat-") && name.ends_with(".stage")
            })
            .map(|entry| entry.path())
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn publication_is_idempotent_but_rejects_another_generation() {
    let fixture = Fixture::new("generation");
    let first = fixture.document("claim-a", Some("session-a"));
    publish(&fixture.root, "owner/repo", 42, Some("session-a"), &first)
        .expect("initial publication");
    publish(&fixture.root, "owner/repo", 42, Some("session-a"), &first).expect("idempotent replay");

    let conflict = fixture.document("claim-b", Some("session-b"));
    let error = publish(
        &fixture.root,
        "owner/repo",
        42,
        Some("session-b"),
        &conflict,
    )
    .expect_err("generation conflict");

    assert_eq!(error.message, "heartbeat publication target conflicts");
}

#[cfg(target_os = "freebsd")]
#[test]
fn freebsd_atomic_publication_rejects_destination_collision() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new("freebsd-publication-collision");
    let repo = fixture.repo_path();
    std::fs::create_dir(&repo).expect("repository directory");
    std::fs::set_permissions(&repo, std::fs::Permissions::from_mode(0o700))
        .expect("repository permissions");
    let directory = open_existing_private_directory(&repo)
        .expect("open repository")
        .expect("repository exists");
    let source =
        create_private_file_relative(&directory, ".source.stage").expect("create source stage");
    std::fs::write(repo.join("42.json"), b"destination").expect("destination");
    std::fs::set_permissions(repo.join("42.json"), std::fs::Permissions::from_mode(0o600))
        .expect("destination permissions");

    let error = atomic_rename_exclusive(&directory, ".source.stage", "42.json", &source)
        .expect_err("destination collision");

    assert!(error.message.contains("heartbeat atomic publish"));
    assert_eq!(std::fs::read(repo.join("42.json")).unwrap(), b"destination");
    assert!(repo.join(".source.stage").is_file());
}

#[cfg(target_os = "freebsd")]
#[test]
fn freebsd_publication_resumes_after_crash_between_link_and_stage_cleanup() {
    let fixture = Fixture::new("freebsd-publication-crash-after-link");
    let document = fixture.document("claim-a", None);
    *FREEBSD_CRASH_AFTER_LINK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(fixture.issue_path());

    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = publish(&fixture.root, "owner/repo", 42, None, &document);
    }));
    assert!(interrupted.is_err(), "publication did not stop after link");
    assert!(fixture.issue_path().is_file(), "destination was not linked");
    let stages = fixture.staging_paths();
    assert_eq!(stages.len(), 1, "linked stage alias was not retained");
    use std::os::unix::fs::MetadataExt;
    let linked_identity = std::fs::metadata(&stages[0]).expect("linked stage metadata");
    let destination_identity =
        std::fs::metadata(fixture.issue_path()).expect("linked destination metadata");
    assert_eq!(linked_identity.dev(), destination_identity.dev());
    assert_eq!(linked_identity.ino(), destination_identity.ino());
    assert_eq!(linked_identity.nlink(), 2);

    publish(&fixture.root, "owner/repo", 42, None, &document).expect("resume exact publication");

    assert_eq!(std::fs::read(&fixture.issue_path()).unwrap(), document);
    assert!(fixture.staging_paths().is_empty());
    let recovered_identity = std::fs::metadata(fixture.issue_path()).unwrap();
    assert_eq!(recovered_identity.dev(), destination_identity.dev());
    assert_eq!(recovered_identity.ino(), destination_identity.ino());
    assert_eq!(recovered_identity.nlink(), 1);
}

mod platform;
mod publication;
mod retirement;
