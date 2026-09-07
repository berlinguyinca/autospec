//! Process observer (spec §16, §19, §47).
//!
//! Observes AutoSpec child processes by reading their startup heartbeats from
//! a **single repository-slug directory** under `~/.autospec/process-heartbeats/`.
//! This is Phase 1 of the resource-lifecycle design (spec §47) — observation
//! only: this module never signals a process, never `kill`s, and never deletes
//! anything. It only inspects files on disk and the kernel process table.
//!
//! A process is `OwnershipClass::RunExclusive` (active) **only** while its
//! lease is unexpired *and* its PID is confirmed live. Two safety rules from
//! the spec are the point of this observer:
//!
//! * An **expired lease never grants `RunExclusive`** (spec §19: expiry is not
//!   abandonment). Expiry only adds the `lease_expired` reason.
//! * A **live PID whose lease has expired is indistinguishable from a reused
//!   PID**, so it stays `External` (spec §36: unattributable → MUST NOT be
//!   reclaimed). PID reuse therefore can never be mistaken for our own active
//!   worker.
//!
//! Scoping: the caller passes exactly one `<slug>/` directory. Reading only the
//! immediate regular files in that directory is what keeps "another slug's
//! heartbeat" out — a heartbeat belonging to a different repository lives in a
//! sibling `<slug>/` directory and is never opened here.

use super::model::{ObservedResource, OwnershipClass, ResourceType};
use crate::AutospecError;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Observe the AutoSpec child processes whose heartbeats live in
/// `heartbeat_root` (one repository-slug directory, e.g.
/// `~/.autospec/process-heartbeats/o10_autospec_r3_core/`).
///
/// Returns one `ObservedResource` per immediate heartbeat file. Subdirectories
/// (the per-slug `sessions/` tree, quarantine trees, ...) are not heartbeats
/// and are not descended into. A missing directory yields an empty list (there
/// are simply no heartbeats here), not an error.
pub fn observe_processes(heartbeat_root: &Path) -> Result<Vec<ObservedResource>, AutospecError> {
    let mut out: Vec<ObservedResource> = Vec::new();

    let entries = match std::fs::read_dir(heartbeat_root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(err) => {
            return Err(AutospecError::io(
                "read_dir",
                heartbeat_root.to_string_lossy(),
                err,
            ))
        }
    };

    let now = unix_now();

    // Only immediate regular files are heartbeats. Do NOT descend into
    // subdirectories — that is how per-slug scoping (and the exclusion of
    // nested `sessions/` files) is preserved.
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            files.push(path);
        }
    }
    files.sort();

    for path in files {
        let Some(external_id) = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_string())
        else {
            continue;
        };

        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            // Vanished between listing and read; do not report a stale row.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => {
                return Err(AutospecError::io(
                    "read_to_string",
                    path.to_string_lossy(),
                    err,
                ))
            }
        };

        let (ownership, reasons) = match serde_json::from_str::<Heartbeat>(&text) {
            Ok(hb) => classify(&hb, now),
            Err(_) => (
                OwnershipClass::External,
                vec![
                    "autospec heartbeat in repo-slug dir".to_string(),
                    "unreadable heartbeat: could not parse JSON".to_string(),
                ],
            ),
        };

        out.push(ObservedResource {
            resource_type: ResourceType::ChildProcess,
            external_id,
            ownership,
            reasons,
            // Process heartbeats do not carry a size; only Docker images do.
            size_bytes: None,
        });
    }

    Ok(out)
}

/// The lease inputs and PID this observer reads from a startup heartbeat.
/// Every other field (repo, issue, nonce, host, boot_id, ...) is intentionally
/// ignored. All three are `Option` so a partially-written or truncated
/// heartbeat degrades to `External` instead of erroring the whole observation.
#[derive(serde::Deserialize)]
struct Heartbeat {
    #[serde(default)]
    pid: Option<u32>,
    #[serde(default)]
    ts: Option<u64>,
    #[serde(default)]
    ttl_seconds: Option<u64>,
}

/// Unix seconds since the epoch. A pre-epoch clock (should be impossible) maps
/// to `0`, which expires every lease — the safe direction (never `RunExclusive`).
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Classify one parsed heartbeat into an ownership class + reasons.
///
/// Split from `observe_processes` so the lease/PID rules are unit-testable
/// with no file I/O and no `/proc` access: `pid_live` is derived from the
/// heartbeat's PID, and the rest is pure.
fn classify(hb: &Heartbeat, now: u64) -> (OwnershipClass, Vec<String>) {
    // A valid lease requires BOTH a timestamp and a TTL. Missing either means
    // we cannot confirm a live lease, so we do not grant RunExclusive — treat
    // as expired (the safe direction).
    let lease_expired = match (hb.ts, hb.ttl_seconds) {
        (Some(ts), Some(ttl)) => now > ts.saturating_add(ttl),
        _ => true,
    };

    // A PID of `0` or absent is never "live". A live PID is confirmed against
    // the kernel process table before the process is ever marked active.
    let pid_live = matches!(hb.pid, Some(pid) if pid != 0 && is_pid_live(pid));

    process_ownership(lease_expired, pid_live)
}

/// The single ownership rule for a process heartbeat. `RunExclusive` requires
/// BOTH an unexpired lease and a confirmed-live PID. Any other combination is
/// `External`:
///
/// * expired lease + live PID → possibly a *reused* PID → not ours → `External`
///   (and therefore not reclaimable).
/// * valid lease + dead PID → the worker is gone → nothing active → `External`.
/// * expired lease + dead PID → clearly nothing → `External`.
fn process_ownership(lease_expired: bool, pid_live: bool) -> (OwnershipClass, Vec<String>) {
    let mut reasons = vec!["autospec heartbeat in repo-slug dir".to_string()];
    if !lease_expired && pid_live {
        reasons.push("lease valid".to_string());
        reasons.push("pid live".to_string());
        (OwnershipClass::RunExclusive, reasons)
    } else {
        if lease_expired {
            reasons.push("lease_expired".to_string());
        }
        if !pid_live {
            reasons.push("pid_not_live".to_string());
        }
        (OwnershipClass::External, reasons)
    }
}

/// Whether `pid` names a live process. Read-only: this never sends a signal —
/// it only consults the kernel process table (Linux `/proc`), so a Phase-1
/// observer cannot accidentally signal or `kill` anything.
///
/// On platforms without a `/proc` process table (e.g. macOS) this fails safe
/// and returns `false`: an observer that cannot confirm liveness must not mark
/// a process `RunExclusive` (active). The primary deployment target is Linux,
/// where `/proc/<pid>` is authoritative for our purposes.
fn is_pid_live(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        // `/proc/<pid>` exists iff the kernel still knows the PID. A zombie
        // still has an entry; that is safe here — a zombie is not a *reused*
        // live PID, and the result is combined with the lease check anyway.
        Path::new(&format!("/proc/{pid}")).is_dir()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_root(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("autospec-proc-{name}-{nonce}"))
    }

    /// Write a heartbeat JSON file with the fields the observer reads (plus a
    /// couple of realistic extras that must be ignored).
    fn write_heartbeat(
        slug_dir: &Path,
        filename: &str,
        ts: Option<u64>,
        ttl: Option<u64>,
        pid: Option<u32>,
    ) {
        let mut map = serde_json::Map::new();
        map.insert("repo".into(), serde_json::json!("berlinguyinca/autospec"));
        map.insert("issue".into(), serde_json::json!(42));
        map.insert("worker_id".into(), serde_json::json!("worker-1"));
        if let Some(ts) = ts {
            map.insert("ts".into(), serde_json::json!(ts));
        }
        if let Some(ttl) = ttl {
            map.insert("ttl_seconds".into(), serde_json::json!(ttl));
        }
        if let Some(pid) = pid {
            map.insert("pid".into(), serde_json::json!(pid));
        }
        std::fs::write(
            slug_dir.join(filename),
            serde_json::Value::Object(map).to_string(),
        )
        .unwrap();
    }

    fn observed_ids(out: &[ObservedResource]) -> Vec<String> {
        out.iter().map(|o| o.external_id.clone()).collect()
    }

    // ── pure ownership decision matrix (cross-platform, no OS) ───────────

    #[test]
    fn run_exclusive_requires_valid_lease_and_live_pid() {
        let (ownership, reasons) = process_ownership(false, true);
        assert_eq!(ownership, OwnershipClass::RunExclusive);
        assert!(ownership.is_reclaimable());
        assert!(reasons.contains(&"lease valid".to_string()));
        assert!(reasons.contains(&"pid live".to_string()));
        assert!(!reasons.contains(&"lease_expired".to_string()));
    }

    #[test]
    fn expired_lease_with_live_pid_never_grants_run_exclusive() {
        // The key safety AC: a live PID whose lease has expired is possibly a
        // reused PID, so it must stay External (not reclaimable).
        let (ownership, reasons) = process_ownership(true, true);
        assert_eq!(ownership, OwnershipClass::External);
        assert!(!ownership.is_reclaimable());
        assert!(reasons.contains(&"lease_expired".to_string()));
        assert!(!reasons.contains(&"pid_not_live".to_string()));
    }

    #[test]
    fn valid_lease_with_dead_pid_is_external() {
        let (ownership, reasons) = process_ownership(false, false);
        assert_eq!(ownership, OwnershipClass::External);
        assert!(!ownership.is_reclaimable());
        assert!(reasons.contains(&"pid_not_live".to_string()));
        assert!(!reasons.contains(&"lease_expired".to_string()));
    }

    #[test]
    fn expired_lease_and_dead_pid_is_external_with_both_reasons() {
        let (ownership, reasons) = process_ownership(true, false);
        assert_eq!(ownership, OwnershipClass::External);
        assert!(reasons.contains(&"lease_expired".to_string()));
        assert!(reasons.contains(&"pid_not_live".to_string()));
    }

    // ── real heartbeat tree (files on disk) ───────────────────────────────

    #[test]
    fn missing_slug_directory_yields_empty_list() {
        let root = temp_root("missing");
        let out = observe_processes(&root.join("no-such-slug")).expect("no error");
        assert!(out.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn another_slugs_heartbeat_is_excluded() {
        // Two sibling slug directories; observing one must not surface the
        // other's heartbeats. This is what "repo-slug subdirectory only" means.
        let root = temp_root("slug-scope");
        let slug_a = root.join("o10_autospec_r3_core");
        let slug_b = root.join("o7_other_r4_repo");
        std::fs::create_dir_all(&slug_a).unwrap();
        std::fs::create_dir_all(&slug_b).unwrap();
        let now = unix_now();
        write_heartbeat(&slug_a, "42", Some(now), Some(3600), Some(1));
        write_heartbeat(&slug_b, "99", Some(now), Some(3600), Some(1));

        let out = observe_processes(&slug_a).expect("observe slug a");
        let ids = observed_ids(&out);
        assert_eq!(ids, vec!["42".to_string()]);
        assert!(
            !ids.contains(&"99".to_string()),
            "slug b's heartbeat leaked in"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn nested_session_files_are_not_descended_into() {
        // Only immediate regular files are heartbeats; files under the
        // per-slug `sessions/` subdirectory are not.
        let root = temp_root("no-descend");
        let slug = root.join("o10_autospec_r3_core");
        std::fs::create_dir_all(slug.join("sessions")).unwrap();
        let now = unix_now();
        write_heartbeat(&slug, "42", Some(now), Some(3600), Some(1));
        write_heartbeat(&slug.join("sessions"), "77", Some(now), Some(3600), Some(1));

        let out = observe_processes(&slug).expect("observe");
        let ids = observed_ids(&out);
        assert_eq!(ids, vec!["42".to_string()]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unreadable_heartbeat_is_external() {
        let root = temp_root("bad-json");
        let slug = root.join("o10_autospec_r3_core");
        std::fs::create_dir_all(&slug).unwrap();
        std::fs::write(slug.join("42"), "this is not json").unwrap();

        let out = observe_processes(&slug).expect("observe");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].ownership, OwnershipClass::External);
        assert!(!out[0].ownership.is_reclaimable());
        assert!(
            out[0].reasons.iter().any(|r| r.contains("unreadable")),
            "expected an unreadable reason, got {:?}",
            out[0].reasons
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn process_observations_carry_no_size_bytes() {
        let root = temp_root("no-size");
        let slug = root.join("o10_autospec_r3_core");
        std::fs::create_dir_all(&slug).unwrap();
        let now = unix_now();
        write_heartbeat(&slug, "42", Some(now), Some(3600), Some(1));

        let out = observe_processes(&slug).expect("observe");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].resource_type, ResourceType::ChildProcess);
        assert_eq!(out[0].size_bytes, None, "process heartbeats have no size");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn valid_lease_with_dead_pid_stays_external_end_to_end() {
        // A PID that has been spawned, run, and reaped is guaranteed-dead on
        // every platform, so this end-to-end case is cross-platform.
        let mut child = std::process::Command::new("true").spawn().unwrap();
        child.wait().unwrap();
        let dead_pid = child.id();
        assert!(dead_pid != 0);

        let root = temp_root("dead-pid");
        let slug = root.join("o10_autospec_r3_core");
        std::fs::create_dir_all(&slug).unwrap();
        let now = unix_now();
        write_heartbeat(&slug, "42", Some(now), Some(3600), Some(dead_pid));

        let out = observe_processes(&slug).expect("observe");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].ownership, OwnershipClass::External);
        assert!(
            out[0].reasons.contains(&"pid_not_live".to_string()),
            "expected pid_not_live, got {:?}",
            out[0].reasons
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // ── Linux-gated end-to-end (real /proc liveness) ──────────────────────

    #[cfg(target_os = "linux")]
    #[test]
    fn valid_lease_with_live_pid_is_run_exclusive_end_to_end() {
        let pid = std::process::id(); // the test process itself: live
        let root = temp_root("run-exclusive");
        let slug = root.join("o10_autospec_r3_core");
        std::fs::create_dir_all(&slug).unwrap();
        let now = unix_now();
        write_heartbeat(&slug, "42", Some(now), Some(3600), Some(pid));

        let out = observe_processes(&slug).expect("observe");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].ownership, OwnershipClass::RunExclusive);
        assert!(out[0].ownership.is_reclaimable());
        assert!(out[0].reasons.contains(&"lease valid".to_string()));
        assert!(out[0].reasons.contains(&"pid live".to_string()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn expired_lease_with_live_pid_stays_external_end_to_end() {
        // A genuinely live PID (this test process) but a long-expired lease
        // must NOT become RunExclusive / reclaimable.
        let pid = std::process::id();
        let root = temp_root("expired-live");
        let slug = root.join("o10_autospec_r3_core");
        std::fs::create_dir_all(&slug).unwrap();
        let now = unix_now();
        write_heartbeat(
            &slug,
            "42",
            Some(now.saturating_sub(10_000)),
            Some(1),
            Some(pid),
        );

        let out = observe_processes(&slug).expect("observe");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].ownership, OwnershipClass::External);
        assert!(!out[0].ownership.is_reclaimable());
        assert!(out[0].reasons.contains(&"lease_expired".to_string()));
        let _ = std::fs::remove_dir_all(&root);
    }
}
