//! Capability prerequisites declared by issues in a `## Requires` section.
//!
//! The dependency graph models issue-to-issue dependencies. A task can also
//! depend on a fact about the world (a running gateway, a populated database,
//! a reachable endpoint) that no issue encodes. Such prerequisites are
//! declared by name under `## Requires`; the CLI probes the world and hands
//! the planner a name-to-state map. The planner stays pure: an unmet
//! capability blocks the issue, and a satisfied one re-admits it on the next
//! plan.

use std::collections::{BTreeMap, BTreeSet};

use super::ready_queue::markdown_sections;

/// The issue-body section that declares world-state prerequisites.
pub const REQUIRES_SECTION: &str = "Requires";

/// One capability's observed state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityState {
    /// The probe succeeded; the prerequisite holds.
    Satisfied,
    /// The probe ran and failed; the prerequisite does not hold.
    Unmet,
}

/// Names declared in the issue body's `## Requires` section, in declaration
/// order with duplicates removed.
///
/// Only well-formed capability names are recognised: a single backtick-
/// optional token made of `[A-Za-z0-9._:/-]`. Free-form prose under the
/// section is ignored rather than guessed at, so a description line cannot
/// silently become a capability that blocks the issue forever.
pub fn required_capabilities(body: &str) -> Vec<String> {
    let section = markdown_sections(body, &[REQUIRES_SECTION]);
    let mut declared = Vec::new();
    let mut seen = BTreeSet::new();
    for line in section.lines() {
        let item = line.trim().trim_start_matches(['-', '*']).trim();
        let Some(name) = capability_name(item) else {
            continue;
        };
        if seen.insert(name.clone()) {
            declared.push(name);
        }
    }
    declared
}

/// Declared-but-unmet capability names, sorted for deterministic holds.
///
/// A capability with no observation fails closed: a world the planner cannot
/// see must not be assumed to hold.
pub fn unmet_capabilities(body: &str, observed: &BTreeMap<String, CapabilityState>) -> Vec<String> {
    let mut unmet: Vec<String> = required_capabilities(body)
        .into_iter()
        .filter(|name| {
            !observed
                .get(name)
                .is_some_and(|state| *state == CapabilityState::Satisfied)
        })
        .collect();
    unmet.sort();
    unmet
}

fn capability_name(item: &str) -> Option<String> {
    let token = item
        .strip_prefix('`')
        .and_then(|rest| rest.strip_suffix('`'))
        .unwrap_or(item)
        .trim();
    if token.is_empty() {
        return None;
    }
    let valid = token.bytes().all(|byte| {
        matches!(
            byte,
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'_' | b':' | b'/' | b'-'
        )
    });
    valid.then(|| token.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(requires: &str) -> String {
        format!("## Goal\nDo the thing.\n\n## Requires\n{requires}\n")
    }

    #[test]
    fn parses_single_and_multiple_names_in_order() {
        let body = body("- gateway:up\n- db:populated");
        assert_eq!(
            required_capabilities(&body),
            vec!["gateway:up".to_string(), "db:populated".to_string()]
        );
    }

    #[test]
    fn dedupes_preserving_first_occurrence() {
        let body = body("- gateway:up\n- db:populated\n- gateway:up");
        assert_eq!(
            required_capabilities(&body),
            vec!["gateway:up".to_string(), "db:populated".to_string()]
        );
    }

    #[test]
    fn ignores_prose_and_non_list_lines() {
        let body = body("a running local gateway\n- `ok/name`");
        assert_eq!(required_capabilities(&body), vec!["ok/name".to_string()]);
    }

    #[test]
    fn absent_section_has_no_capabilities() {
        assert!(required_capabilities("## Goal\nDo it.\n").is_empty());
    }

    #[test]
    fn unmet_sorts_and_fails_closed_on_missing_observations() {
        let body = body("- b/cap\n- a/cap\n- z/cap");
        let observed = BTreeMap::from([("a/cap".to_string(), CapabilityState::Satisfied)]);
        assert_eq!(
            unmet_capabilities(&body, &observed),
            vec!["b/cap".to_string(), "z/cap".to_string()]
        );
    }

    #[test]
    fn unmet_treats_failed_probes_and_absent_names_likewise() {
        let body = body("- ok\n- bad\n- absent");
        let observed = BTreeMap::from([
            ("ok".to_string(), CapabilityState::Satisfied),
            ("bad".to_string(), CapabilityState::Unmet),
        ]);
        assert_eq!(
            unmet_capabilities(&body, &observed),
            vec!["absent".to_string(), "bad".to_string()]
        );
    }

    #[test]
    fn satisfied_map_re_admits_everything() {
        let body = body("- x\n- y");
        let observed = BTreeMap::from([
            ("x".to_string(), CapabilityState::Satisfied),
            ("y".to_string(), CapabilityState::Satisfied),
        ]);
        assert!(unmet_capabilities(&body, &observed).is_empty());
    }
}
