// executor_bridge tests: verification environment declaration (#3794).
//
// The environment an agent verifies in must be the environment the merge gate runs in, or the
// difference must be declared and carried with every verdict. A test that cannot run where it
// is executed is not a failing test; it is an unrunnable one, and the two must never be
// recorded the same way.
//
// Cargo has no unrunnable status: a test that returns early is recorded as passed. The
// distinction is therefore carried in machine-readable markers in the test output that the
// recording agent and the merge gate parse before they count:
//
//   AUTOSPEC-TOOL-ENV: found=<csv> absent=<csv> git=<version>
//     One line per test binary. Declares the verification environment: which of the external
//     tools this suite's tests may need were found on PATH, which were not, and the git version
//     in force. Recorded test figures must be recorded alongside this line; figures from two
//     different environments are not comparable.
//
//   AUTOSPEC-UNRUNNABLE: <file>:<line> <tool-or-capability>: <reason>
//     One line per test that detected a required external tool or capability missing and
//     returned without executing its assertions. Consumers count these as unrunnable, never
//     as failures, and declare the corresponding tool-dependent checks out of scope for agent
//     verification on this host -- the merge gate, which runs with the full toolchain, owns
//     those.
//
// Delivery: every marker line goes to stderr (visible under `cargo test --nocapture` and in
// any runner that surfaces test output) and, when `AUTOSPEC_TOOL_ENV_FILE` names a ledger
// file, to that file as well. The ledger is the deterministic channel: cargo swallows the
// output of passing tests, so a plain `cargo test` shows no markers at all -- a recording
// agent that runs the full suite must set `AUTOSPEC_TOOL_ENV_FILE` to a run-specific path
// and carry the file's lines alongside the pass/fail figures. When the variable is unset
// the tests have no file side effects.

use crate::commands::autonomous::executor_bridge as bridge;
use core::panic::Location;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// The external tools this suite's tests may require. The inventory line records, per run,
/// which of them the verification environment provides and which it does not, so a verdict
/// travels with the environment that produced it.
const VERIFIED_TOOLS: &[&str] = &[
    "codex",
    "gitleaks",
    "license-checker",
    "semgrep",
    "shellcheck",
    "trivy",
];

/// `git merge-tree --write-tree` landed in git 2.38; older git reads `--write-tree` as a
/// revision and dies with `unknown rev`. The gate is the capability, not the binary's
/// mere presence.
const MERGE_TREE_WRITE_TREE_MINIMUM: (u64, u64) = (2, 38);

fn probe_on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path)
        .any(|directory| directory.is_absolute() && directory.join(name).is_file())
}

fn git_version() -> String {
    let Ok(output) = std::process::Command::new("git").arg("--version").output() else {
        return "absent".to_string();
    };
    if !output.status.success() {
        return "unusable".to_string();
    }
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    combined
        .split_whitespace()
        .last()
        .unwrap_or("unknown")
        .to_string()
}

fn git_version_tuple() -> Option<(u64, u64, u64)> {
    let version = git_version();
    let version = version.trim_start_matches('v');
    let mut parts = version.split('.');
    let major: u64 = parts.next()?.parse().ok()?;
    let minor: u64 = parts.next()?.parse().ok()?;
    let patch: u64 = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    Some((major, minor, patch))
}

/// The once-per-process environment declaration. Idempotent and race-free: whichever gated
/// test arrives first emits the line, the rest observe `DECLARED` and stay quiet.
pub(super) fn declare_tool_environment() {
    static DECLARED: AtomicBool = AtomicBool::new(false);
    if DECLARED.swap(true, Ordering::SeqCst) {
        return;
    }
    let line = tool_environment_line();
    eprintln!("{line}");
    append_ledger(&line);
}

/// The run's ledger path, named by the recording agent; `None` keeps the tests side-effect
/// free for a plain `cargo test`.
fn ledger_path() -> Option<PathBuf> {
    std::env::var_os("AUTOSPEC_TOOL_ENV_FILE")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

/// Append one marker line to the run's ledger, serialized per process. A failed write
/// degrades the channel, never the test: stderr still carries the line.
fn append_ledger(line: &str) {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(path) = ledger_path() else {
        return;
    };
    append_ledger_line(&path, line);
}

/// Pure form of the declaration line, so the regression test can assert its shape without
/// emitting a second copy into the marker stream.
fn tool_environment_line() -> String {
    let mut found = Vec::new();
    let mut absent = Vec::new();
    for tool in VERIFIED_TOOLS {
        if probe_on_path(tool) {
            found.push(*tool);
        } else {
            absent.push(*tool);
        }
    }
    format!(
        "AUTOSPEC-TOOL-ENV: found={} absent={} git={}",
        found.join(","),
        absent.join(","),
        git_version()
    )
}

/// Pure form of the unrunnable marker.
fn unrunnable_marker(what: &str, reason: &str, site: &Location<'static>) -> String {
    let file = Path::new(site.file())
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(site.file());
    format!(
        "AUTOSPEC-UNRUNNABLE: {file}:{} {what}: {reason}",
        site.line()
    )
}

/// Record that the calling test is unrunnable on this host and report `false` so the caller
/// returns without executing its assertions. The marker is what reclassifies the early
/// return: unrunnable, never failed.
fn record_unrunnable(what: &str, reason: &str, site: &Location<'static>) -> bool {
    declare_tool_environment();
    let line = unrunnable_marker(what, reason, site);
    eprintln!("{line}");
    append_ledger(&line);
    false
}

/// The gate decision without its side effects: would `bridge::resolve_direct_executable`
/// resolve `tool` from `worktree`?
fn direct_tool_available(worktree: &Path, tool: &str) -> bool {
    bridge::resolve_direct_executable(worktree, tool).is_ok()
}

/// The same question for `bridge::safe_executable` with an explicit environment.
fn harness_tool_available(tool: &str, env: &BTreeMap<String, OsString>) -> bool {
    bridge::safe_executable(Path::new(tool), env).is_ok()
}

/// The capability boundary, isolated so the regression test can pin it on both sides
/// without a git binary.
fn merge_tree_write_tree_capable(version: (u64, u64, u64)) -> bool {
    (version.0, version.1) >= MERGE_TREE_WRITE_TREE_MINIMUM
}

/// Gate for a test that resolves `tool` through `bridge::resolve_direct_executable` (the
/// gitleaks and semgrep regressions). Returns the resolved program when the tool is
/// present; records the calling test as unrunnable and returns `None` when it is not.
#[track_caller]
pub(super) fn require_direct_tool(worktree: &Path, tool: &str) -> Option<PathBuf> {
    let site = Location::caller();
    match bridge::resolve_direct_executable(worktree, tool) {
        Ok(resolved) => {
            declare_tool_environment();
            Some(resolved.program)
        }
        Err(reason) => {
            record_unrunnable(tool, &reason, &site);
            None
        }
    }
}

/// Gate for a test that resolves `tool` through `bridge::safe_executable` with an explicit
/// environment (the Codex regressions). Same contract as `require_direct_tool`.
#[track_caller]
pub(super) fn require_harness_tool(
    tool: &str,
    env: &BTreeMap<String, OsString>,
) -> Option<PathBuf> {
    let site = Location::caller();
    match bridge::safe_executable(Path::new(tool), env) {
        Ok(path) => {
            declare_tool_environment();
            Some(path)
        }
        Err(reason) => {
            record_unrunnable(tool, &reason, &site);
            None
        }
    }
}

/// Gate for a test that needs `git merge-tree --write-tree`. Reports `true` when the git in
/// force has the capability; records the calling test as unrunnable when it does not.
#[track_caller]
pub(super) fn git_supports_merge_tree_write_tree() -> bool {
    let site = Location::caller();
    match git_version_tuple() {
        Some(version) if merge_tree_write_tree_capable(version) => {
            declare_tool_environment();
            true
        }
        Some(version) => record_unrunnable(
            "git merge-tree --write-tree",
            &format!("git version {version:?} is older than 2.38, where --write-tree landed"),
            &site,
        ),
        None => record_unrunnable(
            "git",
            "git is not on PATH or does not report a version",
            &site,
        ),
    }
}

// AC5 regression: an agent run on a host without a required tool must report the affected
// tests as unrunnable and never as failures -- on every host, including hosts that have
// every tool installed. The probe name is guaranteed to exist on no PATH anywhere, so the
// classification is deterministic here.
#[test]
fn missing_tool_is_reported_unrunnable_not_failed() {
    let probe = format!("autospec-unrunnable-probe-{}", std::process::id());
    let env: BTreeMap<String, OsString> = std::env::vars_os()
        .filter_map(|(key, value)| key.into_string().ok().map(|key| (key, value)))
        .collect();

    // Both resolution paths classify a missing tool as unrunnable without panicking: the
    // behavior the six tool-dependent regressions now get instead of the `.expect(...)`
    // failure.
    assert!(!direct_tool_available(&std::env::temp_dir(), &probe));
    assert!(!harness_tool_available(&probe, &env));

    // The classification is what the marker records: unrunnable, never failed.
    let site = Location::caller();
    let marker = unrunnable_marker(&probe, "the tool is not installed", &site);
    assert!(marker.starts_with("AUTOSPEC-UNRUNNABLE: "));
    assert!(marker.contains(&probe));
    assert!(!marker.contains("failed"));

    // The declaration line carries both sides of the environment, so a verdict travels
    // with the environment that produced it.
    let line = tool_environment_line();
    assert!(line.starts_with("AUTOSPEC-TOOL-ENV: found="));
    assert!(line.contains(" absent="));
    assert!(line.contains(" git="));

    // The capability boundary is pinned on both sides, without a git binary.
    assert!(merge_tree_write_tree_capable((2, 38, 0)));
    assert!(merge_tree_write_tree_capable((3, 0, 0)));
    assert!(!merge_tree_write_tree_capable((2, 37, 3)));

    // The ledger is the deterministic channel: append to an explicit path and read the
    // lines back, without touching the process environment.
    let ledger = std::env::temp_dir().join(format!("autospec-ledger-probe-{}", std::process::id()));
    let _ = fs::remove_file(&ledger);
    append_ledger_line(&ledger, "AUTOSPEC-TOOL-ENV: found= absent= git=");
    append_ledger_line(
        &ledger,
        "AUTOSPEC-UNRUNNABLE: test.rs:1 missing-tool: not installed",
    );
    let recorded = fs::read_to_string(&ledger).expect("ledger round-trip");
    assert_eq!(
        recorded.lines().count(),
        2,
        "ledger keeps one line per marker: {recorded:?}"
    );
    let _ = fs::remove_file(&ledger);
}

/// Write half of the ledger channel, path-explicit so the regression test above can pin
/// it without mutating the process environment.
fn append_ledger_line(path: &Path, line: &str) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}
