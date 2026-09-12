//! Deployment drift (issue #4416): deploy from a committed ref, or make
//! divergence loud.
//!
//! The acceptance scenarios, in the order the incident produced them:
//!
//! * a worker refuses to start, or logs at ERROR, when its script differs
//!   from the tracked version at the deployed ref (and a script with no
//!   tracked version fails closed);
//! * the fleet scan reports any file under the fleet directories that
//!   differs from HEAD, and its exit code is the CI job's verdict;
//! * `.bak-*` files are reported for the sweep, with the replacement
//!   practice of a branch.

use autospec_core::deploy_drift::{
    is_backup_name, startup_action, startup_check, FleetScan, StartupAction, StartupPolicy,
};

const TRACKED: &[u8] = b"# worker v2: derive the context window from launch args\n";
const DRIFTED: &[u8] = b"# worker v1: sample /props ten seconds after launch\n";
const REF: &str = "main @ 1a2b3c4d";

fn verdict(running: &[u8], tracked: Option<&[u8]>) -> autospec_core::deploy_drift::StartupVerdict {
    startup_check(running, tracked)
}

/// AC1: a running script identical to the tracked version at the deployed
/// ref starts normally, under either policy.
#[test]
fn in_sync_script_starts() {
    let v = verdict(TRACKED, Some(TRACKED));
    assert!(matches!(
        v,
        autospec_core::deploy_drift::StartupVerdict::InSync { .. }
    ));
    assert_eq!(
        startup_action(StartupPolicy::Refuse, "worker.sh", REF, &v),
        StartupAction::Start
    );
    assert_eq!(
        startup_action(StartupPolicy::LogError, "worker.sh", REF, &v),
        StartupAction::Start
    );
}

/// AC1: a script that differs from the tracked version at the deployed ref
/// refuses to start, and the message names the path, the ref, and both
/// content hashes, so the divergence is loud in the log.
#[test]
fn drifted_script_refuses_to_start() {
    let v = verdict(DRIFTED, Some(TRACKED));
    let action = startup_action(StartupPolicy::Refuse, "worker.sh", REF, &v);
    let StartupAction::Refuse(message) = action else {
        panic!("drifted script must refuse to start, got {action:?}");
    };
    assert!(message.starts_with("ERROR:"), "message: {message}");
    assert!(message.contains("worker.sh"));
    assert!(message.contains(REF));
    let running = autospec_core::autonomous::waterfall::sha256_hex(DRIFTED);
    let tracked = autospec_core::autonomous::waterfall::sha256_hex(TRACKED);
    assert!(message.contains(&running[..12]), "message: {message}");
    assert!(message.contains(&tracked[..12]), "message: {message}");
    assert!(message.contains("refusing to start"));
}

/// AC1: under the log-at-ERROR policy the same drift logs at ERROR instead
/// of refusing, and still never logs at a quieter level.
#[test]
fn drifted_script_logs_at_error() {
    let v = verdict(DRIFTED, Some(TRACKED));
    let action = startup_action(StartupPolicy::LogError, "worker.sh", REF, &v);
    let StartupAction::LogError(message) = action else {
        panic!("drifted script must log at ERROR, got {action:?}");
    };
    assert!(message.starts_with("ERROR:"), "message: {message}");
    assert!(message.contains("worker.sh"));
    assert!(message.contains(REF));
}

/// AC1: a script with no tracked version at the deployed ref is fail-closed,
/// never assumed current: both policies produce a loud outcome, never Start.
#[test]
fn untracked_script_fails_closed() {
    let v = verdict(DRIFTED, None);
    let refuse = startup_action(StartupPolicy::Refuse, "worker.sh", REF, &v);
    assert!(matches!(refuse, StartupAction::Refuse(_)), "got {refuse:?}");
    let log_error = startup_action(StartupPolicy::LogError, "worker.sh", REF, &v);
    let StartupAction::LogError(message) = log_error else {
        panic!("untracked script must log at ERROR, got {log_error:?}");
    };
    assert!(message.starts_with("ERROR:"));
    assert!(message.contains("no tracked version"));
}

/// AC1: the verdict hashes are distinct for distinct content, so the log
/// line actually identifies which copy drifted from which.
#[test]
fn verdict_hashes_distinguish_the_copies() {
    let v = verdict(DRIFTED, Some(TRACKED));
    let (running, tracked) = match &v {
        autospec_core::deploy_drift::StartupVerdict::Drifted { running, tracked } => {
            (running.clone(), tracked.clone())
        }
        other => panic!("expected Drifted, got {other:?}"),
    };
    assert_ne!(running, tracked);
    assert_eq!(
        running,
        autospec_core::autonomous::waterfall::sha256_hex(DRIFTED)
    );
    assert_eq!(
        tracked,
        autospec_core::autonomous::waterfall::sha256_hex(TRACKED)
    );
}

/// AC2: a fleet directory where every file matches HEAD and no `.bak-*`
/// copy is present reports clean and exits 0.
#[test]
fn clean_fleet_scan_reports_clean_and_exit_zero() {
    let mut scan = FleetScan::new();
    scan.add("worker.sh", TRACKED, Some(TRACKED));
    scan.add("gateway.sh", b"# gateway\n", Some(b"# gateway\n"));
    assert!(scan.is_clean());
    let lines = scan.report();
    assert_eq!(
        lines,
        vec!["clean: 2 file(s) under the fleet directory match HEAD; no .bak-* files".to_string()]
    );
    assert_eq!(scan.exit_code(), 0);
}

/// AC2: a file that differs from HEAD is reported with both content hashes,
/// and the scan's exit code turns the CI job red.
#[test]
fn drifted_fleet_file_is_reported_and_exit_one() {
    let mut scan = FleetScan::new();
    scan.add("worker.sh", DRIFTED, Some(TRACKED));
    assert!(!scan.is_clean());
    let lines = scan.report();
    assert_eq!(lines.len(), 1);
    let running = autospec_core::autonomous::waterfall::sha256_hex(DRIFTED);
    let tracked = autospec_core::autonomous::waterfall::sha256_hex(TRACKED);
    assert!(
        lines[0].starts_with("drift: worker.sh:"),
        "line: {}",
        lines[0]
    );
    assert!(lines[0].contains(&running[..12]), "line: {}", lines[0]);
    assert!(lines[0].contains(&tracked[..12]), "line: {}", lines[0]);
    assert_eq!(scan.exit_code(), 1);
}

/// AC2: a file present in the deployment but absent from HEAD is divergence
/// too: it gets a line and a failing exit code, never a clean pass.
#[test]
fn file_absent_from_head_is_reported() {
    let mut scan = FleetScan::new();
    scan.add("worker.sh", DRIFTED, None);
    assert!(!scan.is_clean());
    let lines = scan.report();
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].starts_with("not in HEAD: worker.sh:"),
        "line: {}",
        lines[0]
    );
    assert_eq!(scan.exit_code(), 1);
}

/// AC3: `.bak-*` copies are reported for the sweep, with the replacement
/// practice of a branch and commit, even when their content matches HEAD.
#[test]
fn backup_files_are_swept_with_branch_recommendation() {
    let mut scan = FleetScan::new();
    scan.add("worker.sh", TRACKED, Some(TRACKED));
    scan.add("worker.sh.bak-20260814", TRACKED, Some(TRACKED));
    assert!(!scan.is_clean());
    let lines = scan.report();
    assert_eq!(lines.len(), 1, "lines: {lines:?}");
    assert!(
        lines[0].starts_with("backup: worker.sh.bak-20260814:"),
        "line: {}",
        lines[0]
    );
    assert!(lines[0].contains("branch"), "line: {}", lines[0]);
    assert!(lines[0].contains("sweep"), "line: {}", lines[0]);
    assert_eq!(scan.exit_code(), 1);
}

/// AC3: the backup marker is decided from the base name: `.bak-` anywhere
/// in the base name counts, the directory part does not, and ordinary
/// names do not.
#[test]
fn backup_name_detection() {
    assert!(is_backup_name("worker.sh.bak-20260814"));
    assert!(is_backup_name(".bak-worker.sh"));
    assert!(is_backup_name("fleet/node-04/worker.sh.bak-1"));
    assert!(!is_backup_name("worker.sh"));
    assert!(!is_backup_name("fleet.bak-1/worker.sh"));
    assert!(!is_backup_name("backup.sh"));
}

/// AC2: a scan with no files at all is clean and exits 0: an empty fleet
/// directory is not a failure.
#[test]
fn empty_fleet_scan_is_clean() {
    let scan = FleetScan::new();
    assert!(scan.is_clean());
    assert_eq!(
        scan.report(),
        vec!["clean: 0 file(s) under the fleet directory match HEAD; no .bak-* files".to_string()]
    );
    assert_eq!(scan.exit_code(), 0);
}
