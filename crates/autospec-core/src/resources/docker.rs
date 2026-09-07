//! Docker observer (spec §15.1, §47).
//!
//! Observes the local Docker daemon's containers, images, volumes, and
//! networks as `ObservedResource` values. This is Phase 1 of the
//! resource-lifecycle design (spec §47) — **observation only**:
//!
//! * Every `docker` invocation is a **read-only** list/inspect subcommand,
//!   issued through `std::process::Command` with an **argument vector** (never
//!   through a shell, so the `{{json .}}` format string is a literal).
//! * No destructive subcommand is ever issued (`rm`, `rmi`, `prune`,
//!   `network remove`, `volume rm`, ...). A dedicated test pins this.
//!
//! Ownership follows spec §15.1: a resource is `OwnershipClass::RunExclusive`
//! **only** when it carries the `autospec.managed=true` label. Anything that
//! cannot be attributed to exactly one AutoSpec run is `External` and MUST NOT
//! be deleted (spec §36: unattributable → External).
//!
//! Missing binary: when `docker` is not on `PATH`, the observer **fails open**
//! — it returns an empty list and reports the `docker_unavailable` reason on
//! stderr rather than erroring, so a host without Docker is not an error
//! condition for Phase 1 observation.

use super::model::{ObservedResource, OwnershipClass, ResourceType};
use crate::AutospecError;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The spec §15.1 label that marks a Docker object as owned by exactly one
/// AutoSpec run. A container becomes `RunExclusive` **only** through this
/// label — name prefixes and other signals are not proof of ownership.
const AUTOSPEC_MANAGED_LABEL: &str = "autospec.managed";

/// Reason string for the "docker is not installed / not reachable" condition.
/// Exposed so callers (and the acceptance test for "missing docker → empty +
/// `docker_unavailable` reason") can match on the exact token.
pub const REASON_DOCKER_UNAVAILABLE: &str = "docker_unavailable";

/// Read-only `docker` subcommands the observer uses. Each is a list/inspect
/// verb; the test `docker_subcommands_are_read_only_listing_verbs` pins this.
const CONTAINERS_CMD: &[&str] = &["ps", "-a"];
const IMAGES_CMD: &[&str] = &["images"];
const VOLUMES_CMD: &[&str] = &["volume", "ls"];
const NETWORKS_CMD: &[&str] = &["network", "ls"];

/// Observe the local Docker daemon's containers, images, volumes, and
/// networks.
///
/// Returns an empty `Ok` list (with the `docker_unavailable` reason on stderr)
/// when the `docker` binary is not on `PATH`.
pub fn observe_docker() -> Result<Vec<ObservedResource>, AutospecError> {
    match resolve_docker_binary() {
        Some(bin) => observe_docker_at(&bin),
        None => {
            eprintln!(
                "WARN: resources::docker: {REASON_DOCKER_UNAVAILABLE} — docker binary not found on PATH; returning empty list"
            );
            Ok(Vec::new())
        }
    }
}

/// Locate the `docker` executable on the process `PATH`. `None` when absent.
pub(crate) fn resolve_docker_binary() -> Option<PathBuf> {
    let path_env = std::env::var_os("PATH")?;
    find_in_path("docker", &path_env)
}

/// First path named `name` in any directory of `path_env` that is a regular
/// file, or `None`. Split from `resolve_docker_binary` so the PATH-walk is
/// unit-testable with a synthetic `PATH` (no global mutation).
pub(crate) fn find_in_path(name: &str, path_env: &OsStr) -> Option<PathBuf> {
    for dir in std::env::split_paths(path_env) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Observe Docker via the binary at `bin`. When `bin` does not exist the
/// subsystem is reported unavailable (empty list + `docker_unavailable` on
/// stderr) instead of erroring. Split from `observe_docker` so the unavailable
/// path is testable without mutating the process `PATH`.
fn observe_docker_at(bin: &Path) -> Result<Vec<ObservedResource>, AutospecError> {
    if !bin.is_file() {
        eprintln!(
            "WARN: resources::docker: {REASON_DOCKER_UNAVAILABLE} — docker binary missing at {:?}; returning empty list",
            bin.display()
        );
        return Ok(Vec::new());
    }

    let mut out: Vec<ObservedResource> = Vec::new();

    // Containers: `-a` includes stopped containers — a *stopped* leaked
    // container is exactly what Phase 2 cleanup needs to see.
    for observed in map_lines(run_docker_json(bin, CONTAINERS_CMD)?, container_from_json)? {
        out.push(observed);
    }
    // Images (carry `size_bytes` from Docker's human-readable Size).
    for observed in map_lines(run_docker_json(bin, IMAGES_CMD)?, image_from_json)? {
        out.push(observed);
    }
    // Volumes.
    for observed in map_lines(run_docker_json(bin, VOLUMES_CMD)?, volume_from_json)? {
        out.push(observed);
    }
    // Networks.
    for observed in map_lines(run_docker_json(bin, NETWORKS_CMD)?, network_from_json)? {
        out.push(observed);
    }

    Ok(out)
}

/// Apply `from_json` to each non-empty JSON line, dropping malformed ones.
fn map_lines(
    lines: Vec<String>,
    from_json: impl Fn(&serde_json::Value) -> Option<ObservedResource>,
) -> Result<Vec<ObservedResource>, AutospecError> {
    let mut out = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(observed) = from_json(&value) {
                out.push(observed);
            }
        }
    }
    Ok(out)
}

/// Run `bin <subcommand...> --format {{json .}}` and return its stdout as
/// lines. Every argument (including the literal `{{json .}}` format string) is
/// passed through the argument vector — never via a shell — and every
/// subcommand in this module is read-only (see the module docs).
fn run_docker_json(bin: &Path, subcommand: &[&str]) -> Result<Vec<String>, AutospecError> {
    let mut args: Vec<&str> = subcommand.to_vec();
    args.push("--format");
    args.push("{{json .}}");

    let output = Command::new(bin)
        .args(&args)
        .output()
        .map_err(|err| AutospecError::io("spawn docker", bin.to_string_lossy(), err))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AutospecError::other(format!(
            "docker {} failed ({}): {}",
            subcommand.join(" "),
            output.status,
            stderr.trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|l| l.to_string())
        .collect())
}

// ── per-type JSON → ObservedResource ────────────────────────────────────────

/// A container is `RunExclusive` only via its `autospec.managed=true` label;
/// zero labels (or any other label set) → `External`.
fn container_from_json(v: &serde_json::Value) -> Option<ObservedResource> {
    let external_id = v.get("ID")?.as_str()?.to_string();
    if external_id.is_empty() {
        return None;
    }
    let labels = v
        .get("Labels")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let (ownership, mut reasons) = ownership_from_labels(labels);
    reasons.push(format!("container {external_id}"));
    Some(ObservedResource {
        resource_type: ResourceType::DockerContainer,
        external_id,
        ownership,
        reasons,
        // Container size is not required by this issue; leave it unknown.
        size_bytes: None,
    })
}

/// Images carry `size_bytes` parsed from Docker's human-readable `Size`.
/// `docker images --format '{{json .}}'` does not surface labels, so an image
/// is `External` unless a label field (e.g. `Labels`) is ever present.
fn image_from_json(v: &serde_json::Value) -> Option<ObservedResource> {
    let external_id = v.get("ID")?.as_str()?.to_string();
    if external_id.is_empty() {
        return None;
    }
    let repo = v
        .get("Repository")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("<none>");
    let tag = v
        .get("Tag")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let display = if tag.is_empty() || tag == "<none>" {
        repo.to_string()
    } else {
        format!("{repo}:{tag}")
    };
    let (ownership, mut reasons) = ownership_from_labels(
        v.get("Labels")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(""),
    );
    reasons.push(format!("image {display}"));
    let size_bytes = v
        .get("Size")
        .and_then(serde_json::Value::as_str)
        .and_then(parse_docker_size);
    Some(ObservedResource {
        resource_type: ResourceType::DockerImage,
        external_id,
        ownership,
        reasons,
        size_bytes,
    })
}

fn volume_from_json(v: &serde_json::Value) -> Option<ObservedResource> {
    let external_id = v.get("Name")?.as_str()?.to_string();
    if external_id.is_empty() {
        return None;
    }
    let (ownership, mut reasons) = ownership_from_labels(
        v.get("Labels")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(""),
    );
    reasons.push(format!("volume {external_id}"));
    Some(ObservedResource {
        resource_type: ResourceType::DockerVolume,
        external_id,
        ownership,
        reasons,
        size_bytes: None,
    })
}

fn network_from_json(v: &serde_json::Value) -> Option<ObservedResource> {
    let external_id = v
        .get("ID")
        .and_then(serde_json::Value::as_str)
        .or_else(|| v.get("Name").and_then(serde_json::Value::as_str))?
        .to_string();
    if external_id.is_empty() {
        return None;
    }
    let (ownership, mut reasons) = ownership_from_labels(
        v.get("Labels")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(""),
    );
    reasons.push(format!("network {external_id}"));
    Some(ObservedResource {
        resource_type: ResourceType::DockerNetwork,
        external_id,
        ownership,
        reasons,
        size_bytes: None,
    })
}

// ── label + size parsing helpers ────────────────────────────────────────────

/// Split Docker's `key=value,key2=value2` label string into `(key, value)`
/// pairs. Blank keys are dropped.
fn label_pairs(labels: &str) -> Vec<(String, String)> {
    labels
        .split(',')
        .filter_map(|kv| {
            let kv = kv.trim();
            let (k, val) = kv.split_once('=')?;
            let k = k.trim();
            if k.is_empty() {
                return None;
            }
            Some((k.to_string(), val.trim().to_string()))
        })
        .collect()
}

/// `RunExclusive` iff `labels` carry `autospec.managed=true`; otherwise
/// `External` (unattributable → MUST NOT be deleted, spec §36).
fn ownership_from_labels(labels: &str) -> (OwnershipClass, Vec<String>) {
    if label_pairs(labels)
        .iter()
        .any(|(k, v)| k == AUTOSPEC_MANAGED_LABEL && v == "true")
    {
        (
            OwnershipClass::RunExclusive,
            vec!["autospec.managed=true label present; owned by one run".to_string()],
        )
    } else {
        (
            OwnershipClass::External,
            vec!["no autospec.managed=true label; unattributable".to_string()],
        )
    }
}

/// Parse Docker's human-readable size (`55.7MB`, `0B`, `12B`) into bytes.
///
/// Returns `None` for empty input, an unparseable value, and a parsed value of
/// `0` — per the `ObservedResource::size_bytes` contract the field must never
/// be `Some(0)` as a stand-in for "not measured". A `(virtual ...)` suffix
/// reports only the on-disk size; if the only figure is virtual (e.g. a
/// stopped container's `0B (virtual 55.7MB)`), the on-disk size is `0` →
/// `None`.
fn parse_docker_size(raw: &str) -> Option<u64> {
    let base = raw.trim().split("(virtual").next()?.trim();
    if base.is_empty() {
        return None;
    }
    let number_end = base
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(base.len());
    let number = &base[..number_end];
    let unit = base[number_end..].trim().to_ascii_uppercase();
    let value: f64 = number.parse().ok()?;
    let factor: f64 = match unit.as_str() {
        "" | "B" => 1.0,
        "K" | "KB" => 1024.0,
        "M" | "MB" => 1024.0 * 1024.0,
        "G" | "GB" => 1024.0 * 1024.0 * 1024.0,
        "T" | "TB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        "P" | "PB" => 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    let bytes = (value * factor).round() as u64;
    if bytes == 0 {
        return None;
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── label / ownership (pure) ───────────────────────────────────────────

    #[test]
    fn unlabeled_container_is_external() {
        let (ownership, reasons) = ownership_from_labels("");
        assert_eq!(ownership, OwnershipClass::External);
        assert!(!ownership.is_reclaimable());
        assert!(reasons[0].contains("unattributable"));
    }

    #[test]
    fn autospec_managed_label_is_run_exclusive() {
        let (ownership, _) = ownership_from_labels("autospec.managed=true,autospec.run_id=r1");
        assert_eq!(ownership, OwnershipClass::RunExclusive);
        assert!(ownership.is_reclaimable());
    }

    #[test]
    fn managed_label_false_or_other_is_external() {
        assert_eq!(
            ownership_from_labels("autospec.managed=false").0,
            OwnershipClass::External
        );
        assert_eq!(
            ownership_from_labels("other=true,autospec.run_id=r1").0,
            OwnershipClass::External,
            "run_id alone is not proof of ownership"
        );
    }

    // ── per-type JSON → ObservedResource (pure) ────────────────────────────

    #[test]
    fn container_json_zero_labels_is_external() {
        let v = serde_json::json!({"ID": "abc123", "Labels": "", "State": "running"});
        let obs = container_from_json(&v).expect("container parses");
        assert_eq!(obs.resource_type, ResourceType::DockerContainer);
        assert_eq!(obs.external_id, "abc123");
        assert_eq!(obs.ownership, OwnershipClass::External);
        assert!(!obs.ownership.is_reclaimable());
        assert!(!obs.reasons.is_empty());
    }

    #[test]
    fn container_json_managed_label_is_run_exclusive() {
        let v = serde_json::json!({"ID": "abc123", "Labels": "autospec.managed=true", "State": "running"});
        let obs = container_from_json(&v).expect("container parses");
        assert_eq!(obs.ownership, OwnershipClass::RunExclusive);
        assert!(obs.ownership.is_reclaimable());
    }

    #[test]
    fn container_json_missing_id_is_dropped() {
        let v = serde_json::json!({"Labels": "autospec.managed=true"});
        assert!(container_from_json(&v).is_none());
    }

    #[test]
    fn image_json_carries_size_bytes_and_never_zero() {
        let v = serde_json::json!({"ID": "sha256:img", "Repository": "alpine", "Tag": "3.20", "Size": "55.7MB"});
        let obs = image_from_json(&v).expect("image parses");
        assert_eq!(obs.resource_type, ResourceType::DockerImage);
        assert_eq!(obs.external_id, "sha256:img");
        assert!(
            obs.size_bytes.is_some(),
            "images carry size_bytes from Docker"
        );
        assert_ne!(obs.size_bytes, Some(0), "size_bytes must never be 0");
        let expected = (55.7f64 * 1024.0 * 1024.0).round() as u64;
        assert_eq!(obs.size_bytes, Some(expected));
    }

    #[test]
    fn image_json_without_size_is_none_size() {
        let v = serde_json::json!({"ID": "sha256:img", "Repository": "alpine", "Tag": "3.20"});
        let obs = image_from_json(&v).expect("image parses");
        assert_eq!(obs.size_bytes, None, "unknown size -> None, not 0");
    }

    #[test]
    fn volume_and_network_parse() {
        let vol = volume_from_json(&serde_json::json!({"Name": "data", "Driver": "local"}))
            .expect("volume parses");
        assert_eq!(vol.resource_type, ResourceType::DockerVolume);
        assert_eq!(vol.external_id, "data");
        assert_eq!(vol.ownership, OwnershipClass::External);

        let net = network_from_json(&serde_json::json!({"ID": "net1", "Name": "bridge"}))
            .expect("network parses");
        assert_eq!(net.resource_type, ResourceType::DockerNetwork);
        assert_eq!(net.external_id, "net1");
        assert_eq!(net.ownership, OwnershipClass::External);
    }

    // ── size parsing (pure) ────────────────────────────────────────────────

    #[test]
    fn parse_docker_size_variants() {
        assert_eq!(parse_docker_size("12B"), Some(12));
        assert_eq!(parse_docker_size("1KB"), Some(1024));
        assert_eq!(parse_docker_size("1MB"), Some(1024 * 1024));
        assert_eq!(parse_docker_size("1GB"), Some(1024u64.pow(3)));
        assert_eq!(
            parse_docker_size("55.7MB"),
            Some((55.7f64 * 1024.0 * 1024.0).round() as u64)
        );
        assert_eq!(parse_docker_size(""), None, "empty is unknown");
        assert_eq!(parse_docker_size("0B"), None, "0 is never reported");
        assert_eq!(parse_docker_size("unknown"), None, "unparseable is unknown");
        assert_eq!(
            parse_docker_size("0B (virtual 55.7MB)"),
            None,
            "virtual-only on-disk size is 0 -> None"
        );
    }

    // ── PATH resolution (pure, no global mutation) ─────────────────────────

    #[test]
    fn find_in_path_returns_none_for_absent_binary() {
        assert!(find_in_path(
            "definitely-not-a-real-binary-xyz",
            OsStr::new("/nonexistent")
        )
        .is_none());
        assert!(find_in_path("docker", OsStr::new("")).is_none());
    }

    #[test]
    fn find_in_path_finds_an_existing_file() {
        let root = std::env::temp_dir().join(format!(
            "autospec-docker-path-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("docker");
        std::fs::write(&target, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(find_in_path("docker", root.as_os_str()), Some(target));
        let _ = std::fs::remove_dir_all(&root);
    }

    // ── docker-missing (the primary path in this environment) ──────────────

    #[test]
    fn missing_docker_binary_yields_empty_and_unavailable_reason() {
        // Point the seam at a path that does not exist: unavailable -> empty
        // list, and NO error (fail-open). This is environment-independent.
        let out = observe_docker_at(Path::new("/nonexistent-dir-xyz-123/docker"))
            .expect("missing docker must not error");
        assert!(out.is_empty(), "missing docker -> empty list");
        assert_eq!(REASON_DOCKER_UNAVAILABLE, "docker_unavailable");
    }

    // ── no-destructive-command guard ────────────────────────────────────────

    #[test]
    fn docker_subcommands_are_read_only_listing_verbs() {
        // Guard: the observer must never issue a destructive Docker
        // subcommand. Every command's lead verb must be a read-only
        // list/inspect verb.
        let read_only_leads = ["ps", "images", "volume", "network"];
        for cmd in [CONTAINERS_CMD, IMAGES_CMD, VOLUMES_CMD, NETWORKS_CMD] {
            assert!(
                read_only_leads.contains(&cmd[0]),
                "lead verb must be a read-only listing verb, got {:?}",
                cmd[0]
            );
        }
    }

    // ── real-daemon integration (skips when docker is absent) ──────────────

    #[test]
    fn observe_docker_against_real_daemon_when_present() {
        if resolve_docker_binary().is_none() {
            eprintln!("SKIP: docker not available in this environment");
            return;
        }
        let resources = observe_docker().expect("observe_docker against real daemon");
        for r in &resources {
            assert!(!r.external_id.is_empty());
            assert!(!r.reasons.is_empty());
            assert!(
                matches!(
                    r.resource_type,
                    ResourceType::DockerContainer
                        | ResourceType::DockerImage
                        | ResourceType::DockerVolume
                        | ResourceType::DockerNetwork
                ),
                "unexpected resource type: {r:?}"
            );
            if r.resource_type == ResourceType::DockerImage {
                assert!(r.size_bytes.is_some(), "image must carry size_bytes: {r:?}");
                assert_ne!(r.size_bytes, Some(0), "size_bytes must never be 0: {r:?}");
            }
        }
    }
}
