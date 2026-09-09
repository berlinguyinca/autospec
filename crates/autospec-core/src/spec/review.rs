//! Deterministic safety-property review for spec documents.
//!
//! A spec line written in the imperative mood ("always", "never", "make sure
//! to", "be careful") states a safety property. If the same line names a
//! structural mechanism (a lock, gate, check, test, fail-closed behavior,
//! ...), the property is structurally enforced. If it does not, the property
//! rests on convention alone, and the review flags it so a reviewer can state
//! explicitly which kind of property it is.
//!
//! The check is advisory: it never fails a spec, it only reports.

/// Rule ID emitted for every finding produced by [`lint_spec_safety`].
pub const SAFETY_WITHOUT_MECHANISM_RULE_ID: &str = "SAFETY_WITHOUT_MECHANISM";

/// Imperative phrases that mark a safety property in spec prose.
const IMPERATIVE_PHRASES: &[&str] = &["always", "never", "make sure to", "be careful"];

/// Phrases whose presence on the same line indicates a structural mechanism
/// backing the safety property.
const MECHANISM_PHRASES: &[&str] = &[
    // Nouns for structural enforcers.
    "lock",
    "locks",
    "snapshot",
    "snapshots",
    "gate",
    "gates",
    "guard",
    "guards",
    "check",
    "checks",
    "test",
    "tests",
    "lint",
    "lints",
    "validator",
    "validators",
    "assertion",
    "assertions",
    "invariant",
    "invariants",
    "constraint",
    "constraints",
    "policy",
    "policies",
    "type system",
    "typed",
    // Verbs and adjectives of structural enforcement.
    "enforced",
    "enforces",
    "enforcement",
    "checked",
    "validates",
    "validated",
    "validation",
    "asserts",
    "asserted",
    "rejects",
    "rejected",
    "denies",
    "denied",
    "impossible",
    "impossibility",
    "guarantees",
    "guaranteed",
    "fails closed",
    "fail closed",
    "fail-closed",
    "failed closed",
    "mechanically",
];

/// One convention-only safety property found in a spec document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecSafetyFinding {
    /// 1-based line number in the spec source.
    pub line: usize,
    /// The imperative phrase that triggered the finding.
    pub phrase: String,
    /// The trimmed spec line the finding applies to.
    pub text: String,
}

impl SpecSafetyFinding {
    /// Stable rule identifier for the finding.
    pub const fn rule_id(&self) -> &'static str {
        SAFETY_WITHOUT_MECHANISM_RULE_ID
    }

    /// Human-readable diagnostic for the finding.
    pub fn message(&self) -> String {
        format!(
            "line {}: imperative safety property (\"{}\") names no structural mechanism — convention-only",
            self.line, self.phrase
        )
    }
}

/// Review spec prose and return every imperative safety property that names
/// no structural mechanism.
///
/// Fenced code blocks (``` or ~~~) are skipped: imperative words in code are
/// commands or comments, not spec prose. Findings are returned in line order,
/// with phrases in the order they appear in [`IMPERATIVE_PHRASES`].
pub fn lint_spec_safety(source: &str) -> Vec<SpecSafetyFinding> {
    let mut findings = Vec::new();
    let mut in_fence = false;

    for (index, raw_line) in source.lines().enumerate() {
        let trimmed = raw_line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence || trimmed.is_empty() {
            continue;
        }

        if mechanism_named(trimmed) {
            continue;
        }

        let lowered = trimmed.to_ascii_lowercase();
        for phrase in IMPERATIVE_PHRASES {
            if contains_phrase(&lowered, phrase) {
                findings.push(SpecSafetyFinding {
                    line: index + 1,
                    phrase: (*phrase).to_owned(),
                    text: trimmed.to_owned(),
                });
            }
        }
    }

    findings
}

fn mechanism_named(line: &str) -> bool {
    let lowered = line.to_ascii_lowercase();
    MECHANISM_PHRASES
        .iter()
        .any(|phrase| contains_phrase(&lowered, phrase))
}

/// Case-insensitive containment test. Multi-word phrases match by substring;
/// single words must sit on word boundaries so "nevertheless" does not match
/// "never" and "lockstep" does not match "lock".
fn contains_phrase(haystack: &str, needle: &str) -> bool {
    if needle.contains(' ') {
        return haystack.contains(needle);
    }
    haystack.match_indices(needle).any(|(index, _)| {
        let before_ok = index == 0 || !haystack.as_bytes()[index - 1].is_ascii_alphanumeric();
        let after = index + needle.len();
        let after_ok =
            after >= haystack.len() || !haystack.as_bytes()[after].is_ascii_alphanumeric();
        before_ok && after_ok
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phrases(findings: &[SpecSafetyFinding]) -> Vec<String> {
        findings.iter().map(|f| f.phrase.clone()).collect()
    }

    #[test]
    fn flags_convention_only_imperatives() {
        let source = "The pipeline never touches the live file.\n";
        let findings = lint_spec_safety(source);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 1);
        assert_eq!(findings[0].phrase, "never");
    }

    #[test]
    fn each_imperative_phrase_is_recognized() {
        for phrase in ["always", "never", "make sure to", "be careful"] {
            let source = format!("The run {phrase} the state directory.\n");
            let findings = lint_spec_safety(&source);
            assert_eq!(
                phrases(&findings),
                vec![phrase.to_owned()],
                "phrase {phrase:?} should be flagged"
            );
        }
    }

    #[test]
    fn mechanism_on_same_line_suppresses_finding() {
        let source = "The gate rejects a second writer, so the lock is never held twice.\n";
        assert!(lint_spec_safety(source).is_empty());
    }

    #[test]
    fn fail_closed_is_a_mechanism() {
        let source = "On ambiguity the loader always fails closed.\n";
        assert!(lint_spec_safety(source).is_empty());
    }

    #[test]
    fn lockstep_is_not_a_lock_mechanism() {
        let source = "We never revisit lockstep drift.\n";
        let findings = lint_spec_safety(source);
        assert_eq!(phrases(&findings), vec!["never".to_owned()]);
    }

    #[test]
    fn nevertheless_is_not_never() {
        let source = "The note is filed nevertheless.\n";
        assert!(lint_spec_safety(source).is_empty());
    }

    #[test]
    fn imperative_inside_code_fence_is_ignored() {
        let source = "```\nnever rm the state\n```\n";
        assert!(lint_spec_safety(source).is_empty());
    }

    #[test]
    fn tilde_fences_are_ignored_too() {
        let source = "~~~\nalways panic here\n~~~\n";
        assert!(lint_spec_safety(source).is_empty());
    }

    #[test]
    fn multiple_lines_report_in_line_order() {
        let source = "a\nnever rewrite it\nb\nalways trust it\n";
        let findings = lint_spec_safety(source);
        assert_eq!(
            findings
                .iter()
                .map(|f| (f.line, f.phrase.as_str()))
                .collect::<Vec<_>>(),
            vec![(2, "never"), (4, "always")]
        );
    }

    #[test]
    fn matching_is_case_insensitive() {
        let source = "NEVER push to main.\nAlways snapshot first.\n";
        let findings = lint_spec_safety(source);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 1);
        assert_eq!(findings[0].phrase, "never");
    }

    #[test]
    fn finding_message_cites_rule_and_line() {
        let findings = lint_spec_safety("Never delete the journal.\n");
        assert_eq!(findings[0].rule_id(), SAFETY_WITHOUT_MECHANISM_RULE_ID);
        assert!(findings[0].message().starts_with("line 1:"));
    }
}
