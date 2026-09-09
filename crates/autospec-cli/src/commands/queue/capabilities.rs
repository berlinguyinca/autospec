//! World-state capability probing for the ready frontier.
//!
//! Issues declare probeable world-state prerequisites (a running gateway, a
//! populated database) by name under a `## Requires` section. The frontier
//! refuses to offer a task whose prerequisites are not currently satisfied
//! and names the missing capabilities in the hold view; a capability that
//! becomes satisfied re-admits the issue on the next plan without any issue
//! edit.
//!
//! The pure policy lives in `autospec_core::coordination::capabilities`.
//! This module performs the I/O: it reads the probe configuration and runs
//! the probes, handing the planner a name-to-state map.
//!
//! Probe configuration lives in `.autospec/capabilities.json` (override with
//! `$AUTOSPEC_CAPABILITIES_FILE`):
//!
//! ```json
//! {
//!   "gateway:up": {
//!     "probe": ["curl", "--fail", "--silent", "--max-time", "5", "http://127.0.0.1:8080/health"]
//!   }
//! }
//! ```
//!
//! A probe exits `0` when the capability is satisfied. A declared capability
//! with no configured probe fails closed (treated as unmet): the frontier
//! must never guess that the world is ready. Probes are expected to be
//! self-limiting (e.g. `curl --max-time`); there is no harness-side timeout.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use autospec_core::coordination::{required_capabilities, CapabilityState};

use super::*;

fn config_path() -> PathBuf {
    match std::env::var("AUTOSPEC_CAPABILITIES_FILE") {
        Ok(path) if !path.is_empty() => PathBuf::from(path),
        _ => PathBuf::from(".autospec/capabilities.json"),
    }
}

/// Capability names mapped to their probe argv, from the config file.
fn load_probes_at(path: &Path) -> BTreeMap<String, Vec<String>> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        eprintln!(
            "WARN: capability config {} is not valid JSON; failing closed",
            path.display()
        );
        return BTreeMap::new();
    };
    let mut probes = BTreeMap::new();
    if let serde_json::Value::Object(entries) = value {
        for (name, spec) in entries {
            let Some(serde_json::Value::Array(argv_values)) = spec.get("probe") else {
                continue;
            };
            let Some(argv) = argv_values
                .iter()
                .map(serde_json::Value::as_str)
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            if !argv.is_empty() {
                probes.insert(name, argv.iter().map(ToString::to_string).collect());
            }
        }
    }
    probes
}

fn probe_is_satisfied(argv: &[String]) -> bool {
    let Ok(output) = Command::new(&argv[0]).args(&argv[1..]).output() else {
        return false;
    };
    output.status.success()
}

/// Observes every capability declared by the queue candidates.
///
/// The map contains only declared capabilities; the planner treats any
/// declared capability absent from this map as unmet.
pub(super) fn observe_capabilities(
    candidates: &[RemoteIssue],
) -> BTreeMap<String, CapabilityState> {
    observe_capabilities_at(&config_path(), candidates)
}

pub(super) fn observe_capabilities_at(
    path: &Path,
    candidates: &[RemoteIssue],
) -> BTreeMap<String, CapabilityState> {
    let mut declared: BTreeSet<String> = BTreeSet::new();
    for issue in candidates {
        for capability in required_capabilities(&issue.body) {
            declared.insert(capability);
        }
    }
    let probes = load_probes_at(path);
    declared
        .into_iter()
        .map(|capability| {
            let state = match probes.get(&capability) {
                Some(argv) if probe_is_satisfied(argv) => CapabilityState::Satisfied,
                _ => CapabilityState::Unmet,
            };
            (capability, state)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "autospec-capabilities-{tag}-{}.json",
            std::process::id()
        ))
    }

    fn issue_with_requires(capabilities: &[&str]) -> RemoteIssue {
        let mut body = String::from("## Goal\nDo the thing.\n\n## Requires\n");
        for capability in capabilities {
            body.push_str(&format!("- {capability}\n"));
        }
        RemoteIssue {
            number: 1,
            title: "Task".to_string(),
            body,
            labels: Vec::new(),
            author: "tester".to_string(),
            closed: false,
        }
    }

    fn only_capability(states: BTreeMap<String, CapabilityState>) -> Option<CapabilityState> {
        states.into_iter().next().map(|(_, state)| state)
    }

    #[test]
    fn satisfied_probe_marks_the_capability_satisfied() {
        let path = temp_config("satisfied");
        std::fs::write(&path, r#"{"gateway:up": {"probe": ["true"]}}"#).unwrap();
        let candidates = vec![issue_with_requires(&["gateway:up"])];
        let states = observe_capabilities_at(&path, &candidates);
        assert_eq!(only_capability(states), Some(CapabilityState::Satisfied));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn failing_probe_marks_the_capability_unmet() {
        let path = temp_config("failing");
        std::fs::write(&path, r#"{"gateway:up": {"probe": ["false"]}}"#).unwrap();
        let candidates = vec![issue_with_requires(&["gateway:up"])];
        let states = observe_capabilities_at(&path, &candidates);
        assert_eq!(only_capability(states), Some(CapabilityState::Unmet));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_probe_binary_fails_closed() {
        let path = temp_config("missing-binary");
        std::fs::write(
            &path,
            r#"{"gateway:up": {"probe": ["definitely-not-a-real-binary-3908"]}}"#,
        )
        .unwrap();
        let candidates = vec![issue_with_requires(&["gateway:up"])];
        let states = observe_capabilities_at(&path, &candidates);
        assert_eq!(only_capability(states), Some(CapabilityState::Unmet));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn declared_capability_without_probe_fails_closed() {
        let path = temp_config("no-probe");
        std::fs::write(&path, r#"{"other:cap": {"probe": ["true"]}}"#).unwrap();
        let candidates = vec![issue_with_requires(&["gateway:up"])];
        let states = observe_capabilities_at(&path, &candidates);
        assert_eq!(only_capability(states), Some(CapabilityState::Unmet));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_config_file_fails_closed_for_every_capability() {
        let path = temp_config("missing-config");
        let candidates = vec![issue_with_requires(&["gateway:up", "db:populated"])];
        let states = observe_capabilities_at(&path, &candidates);
        assert_eq!(states.len(), 2);
        assert_eq!(states.get("gateway:up"), Some(&CapabilityState::Unmet));
        assert_eq!(states.get("db:populated"), Some(&CapabilityState::Unmet));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn only_declared_capabilities_are_observed() {
        let path = temp_config("only-declared");
        std::fs::write(
            &path,
            r#"{"gateway:up": {"probe": ["true"]}, "unused:cap": {"probe": ["true"]}}"#,
        )
        .unwrap();
        let candidates = vec![issue_with_requires(&["gateway:up"])];
        let states = observe_capabilities_at(&path, &candidates);
        assert_eq!(states.len(), 1);
        assert_eq!(states.get("gateway:up"), Some(&CapabilityState::Satisfied));
        assert!(!states.contains_key("unused:cap"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn corrupt_config_fails_closed_without_panic() {
        let path = temp_config("corrupt");
        std::fs::write(&path, "{not json").unwrap();
        let candidates = vec![issue_with_requires(&["gateway:up"])];
        let states = observe_capabilities_at(&path, &candidates);
        assert_eq!(only_capability(states), Some(CapabilityState::Unmet));
        let _ = std::fs::remove_file(&path);
    }
}
