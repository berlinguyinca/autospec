//! The process-wide environment boundary the claim tests share.
//!
//! Split from support.rs, which the file-size ratchet caps at 600 lines.

use super::support::{lock_heartbeat_env, STARTUP_HEARTBEAT_ENV};

/// No call site may take the boundary directly.
///
/// The poison-tolerance test above cannot catch a regression on its own, because
/// whether the offending test runs after the poisoner is a matter of scheduling.
/// This reads the sources instead, which is order-independent.
#[test]
fn every_boundary_user_goes_through_the_poison_tolerant_helper() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/commands/claim/tests");
    let mut offenders = Vec::new();
    for entry in std::fs::read_dir(&directory).expect("read claim tests directory") {
        let path = entry.expect("claim tests entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
            continue;
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        // support.rs owns the helper itself.
        if name == "support.rs" {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("read claim test source");
        let collapsed: String = source.split_whitespace().collect::<Vec<_>>().join(" ");
        // Built at run time, not written as a literal: spelled out, the needle would
        // appear in this file and the guard would report itself. Excluding this file
        // instead would leave a hole a real offender could hide in.
        let needle = format!("{}{}", "STARTUP_HEARTBEAT", "_ENV.lock()");
        let spaced = needle.replace("_ENV.lock()", "_ENV .lock()");
        if collapsed.contains(&needle) || collapsed.contains(&spaced) {
            offenders.push(name);
        }
    }
    assert!(
        offenders.is_empty(),
        "take the boundary through lock_heartbeat_env() so one panic cannot poison it \
         for every later test; direct .lock() found in {offenders:?}"
    );
}

/// A panicking holder must not disable the boundary for every later test.
///
/// This poisons the process-wide mutex on purpose, so the panic it prints is
/// expected output, not a failure. Poisoning it here is safe because every other
/// user goes through `lock_heartbeat_env`. That property is enforced by
/// `every_boundary_user_goes_through_the_poison_tolerant_helper` rather than by
/// this test: whether a regressed call site observes the poison depends on test
/// order, which is not deterministic, so ordering cannot be the guard.
#[test]
fn heartbeat_env_boundary_survives_a_poisoned_holder() {
    let poisoner = std::thread::spawn(|| {
        let _held = lock_heartbeat_env();
        panic!("poisoning the heartbeat boundary on purpose");
    });
    assert!(
        poisoner.join().is_err(),
        "the holder thread was supposed to panic"
    );
    assert!(
        STARTUP_HEARTBEAT_ENV.is_poisoned(),
        "a panicking holder should have poisoned the boundary"
    );
    // The point of the fix: this must not panic.
    let _guard = lock_heartbeat_env();
}
