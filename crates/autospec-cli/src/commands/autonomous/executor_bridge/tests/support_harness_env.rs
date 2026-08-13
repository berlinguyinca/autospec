// executor_bridge tests: ambient harness-environment isolation.
//
// Split out of support_base.rs to keep that file under AUTOSPEC_MAX_FILE_LOC.

use std::ffi::OsString;

/// The harness variables `HarnessConfig::resolve` infers the executor kind from.
///
/// These leak in from whatever shell ran `cargo test`, because
/// `run_executor_bridge_with_codex_probe` builds its environment from
/// `std::env::vars_os()`. A developer running the suite inside Claude Code exports
/// `CLAUDECODE`, so `resolve` returned `Claude` for fixtures whose alias table only
/// defines `codex`, and they died with `executor harness alias missing: claude`
/// before the Codex probe was reachable — on that host only, passing in CI and in a
/// plain shell (#2991). The suite must describe the harness it is testing, never the
/// one the developer happens to be sitting in.
const HARNESS_ENV_VARS: &[&str] = &[
    "AUTOSPEC_HANDOFF_DISPATCHER_KIND",
    "CODEX_THREAD_ID",
    "CODEX_CI",
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "OPENCODE",
    "OPENCODE_SESSION_ID",
];

/// What `restore` needs to put every scrubbed variable back exactly as it was.
pub(super) type HarnessEnvRestore = Vec<(&'static str, Option<OsString>)>;

/// Remove the ambient harness variables, returning what `restore` must put back.
///
/// Only ever called while holding the test-ordering mutex: these are process-wide,
/// so the same lock that sequences failpoint arming has to sequence this too.
pub(super) fn scrub() -> HarnessEnvRestore {
    HARNESS_ENV_VARS
        .iter()
        .map(|key| {
            let previous = std::env::var_os(key);
            if previous.is_some() {
                std::env::remove_var(key);
            }
            (*key, previous)
        })
        .collect()
}

/// Put back exactly what `scrub` removed. Runs from `Drop`, including on unwind, so
/// a panicking test cannot leak a scrubbed environment into whatever runs next.
pub(super) fn restore(saved: &mut HarnessEnvRestore) {
    for (key, previous) in saved.drain(..) {
        match previous {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}
