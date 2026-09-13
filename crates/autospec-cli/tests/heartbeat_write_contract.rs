//! Regression test for issue #3502: the shell heartbeat writer must produce
//! session-sidecar directories that satisfy the **same** private-directory
//! contract the Rust claim CLI enforces, so `claim release` can retire them
//! instead of deferring with a warning and orphaning the sidecar.
//!
//! The Rust gate is
//! `crates/autospec-cli/src/commands/claim/open_beneath.rs::
//! private_heartbeat_directory_identity`, which requires the session directory
//! to be a directory, owned by the effective user, and mode exactly `0700`.
//!
//! This test is written in Rust (per the project's Rust-first direction) and
//! spawns the actual `skills/autospec-run/scripts/heartbeat-write.sh` writer,
//! then asserts — via `std::os::unix` metadata — that its output meets that
//! contract byte-for-byte. It therefore pins:
//!   - AC1: the session-sidecar directory is mode `0700`, euid-owned.
//!   - AC2: the shell writer and the Rust claim CLI resolve to the same
//!          canonical slug directory (`owner/repo` → `owner__repo`).
//!   - AC3: with AC1+AC2 holding, the Rust gate accepts the directory, so
//!          `claim release` does not defer.

#![cfg(unix)]

use std::os::unix::fs::{MetadataExt, PermissionsExt};

#[path = "support/temp_directory.rs"]
mod temp_directory;
use temp_directory::unique as temp_dir;

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root resolvable")
}

/// The session-key hex the writer derives from `--session-id` (it hex-encodes
/// the raw bytes with `od -An -tx1`).
fn session_key_hex(session_id: &str) -> String {
    session_id.bytes().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn shell_heartbeat_writer_conforms_to_the_rust_private_directory_contract() {
    let root = temp_dir("hb-write-contract");
    let script = repo_root().join("skills/autospec-run/scripts/heartbeat-write.sh");
    assert!(
        script.is_file(),
        "heartbeat-write.sh not found at {script:?}"
    );

    // A reference directory created by this process is, by definition, owned by
    // the effective user. Comparing uids against it proves euid-ownership
    // without a libc dependency.
    let reference = root.join("reference");
    std::fs::create_dir(&reference).expect("create reference dir");
    let reference_uid = std::fs::metadata(&reference)
        .expect("reference metadata")
        .uid();

    let base = root.join("heartbeats");
    let session_id = "s1";
    let output = std::process::Command::new("bash")
        .arg(&script)
        .args([
            "--issue",
            "42",
            "--branch",
            "test",
            "--step",
            "expand_start",
            "--repo",
            "owner/repo",
            "--worker-id",
            "w1",
            "--claim-id",
            "c1",
            "--session-id",
            session_id,
        ])
        .env("AUTOSPEC_HEARTBEAT_DIR", &base)
        .output()
        .expect("heartbeat-write.sh runs");
    assert!(
        output.status.success(),
        "heartbeat-write.sh failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    // ── AC2: canonical slug, matching the Rust `repo.replace('/', "__")` ──
    let rust_canonical = "owner/repo".replace('/', "__");
    assert_eq!(rust_canonical, "owner__repo");
    let slug_dir = base.join(&rust_canonical);
    assert!(
        slug_dir.is_dir(),
        "expected the canonical slug dir `{rust_canonical}` under {base:?}; \
         the shell writer must land in the same tree the Rust claim CLI uses"
    );

    // ── AC1: the session-sidecar directory is mode 0700 and euid-owned ──
    let sessions = slug_dir.join("sessions");
    let meta = std::fs::metadata(&sessions)
        .unwrap_or_else(|_| panic!("session-sidecar dir {sessions:?} was not created"));
    assert!(
        meta.is_dir(),
        "session-sidecar path {sessions:?} is not a directory"
    );
    let mode = meta.permissions().mode() & 0o7777;
    assert_eq!(
        mode, 0o700,
        "session-sidecar dir must be mode 0700 (the Rust retirement contract); \
         found {mode:o}"
    );
    assert_eq!(
        meta.uid(),
        reference_uid,
        "session-sidecar dir must be owned by the effective user"
    );

    // The slug directory itself is also created by the writer and is part of
    // the same private tree the Rust gate walks.
    let slug_meta = std::fs::metadata(&slug_dir).expect("slug dir metadata");
    assert_eq!(
        slug_meta.permissions().mode() & 0o7777,
        0o700,
        "slug dir must be mode 0700; found {:o}",
        slug_meta.permissions().mode() & 0o7777
    );

    // ── The session binding file is written private (0600) ──
    let session_file = sessions.join(format!("{}.json", session_key_hex(session_id)));
    let file_meta = std::fs::metadata(&session_file)
        .unwrap_or_else(|_| panic!("session binding {session_file:?} was not created"));
    assert_eq!(
        file_meta.permissions().mode() & 0o7777,
        0o600,
        "session binding must be mode 0600; found {:o}",
        file_meta.permissions().mode() & 0o7777
    );
}
