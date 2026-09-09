//! §41 conflict detection across instruction sources and the §40
//! growth-control priority.
//!
//! Spec: `docs/specs/2026-09-08-continuous-improvement-engine.md` §40-§41.
//!
//! §41: rules are checked for conflicts across instruction sources — AGENTS.md,
//! repo instructions, global instructions, skills, tool descriptions, routing
//! rules. Conflicting rules produce a [`Conflict`] and MUST be surfaced
//! during proposal evaluation.
//!
//! Detection here is deterministic (§4.1 "deterministic first, semantic
//! second"): the unambiguous "always use X" / "never use X" class — one side
//! obligates a subject, the other forbids it, on overlapping subjects.
//! Fully semantic contradictions (synonyms, paraphrase) are a strong-model
//! job at evaluation time; a result of this module is *structural* evidence,
//! not a claim that the pair is semantically identical.

use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::error::AutospecError;

use super::schema::Proposal;

/// §41 instruction sources a rule can live in, in the §41 listing order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleSource {
    #[default]
    AgentsMd,
    RepoInstructions,
    GlobalInstructions,
    Skill,
    ToolDescription,
    RoutingRule,
}

impl RuleSource {
    /// The §41 snake_case wire name for this source.
    pub fn as_str(&self) -> &'static str {
        match self {
            RuleSource::AgentsMd => "agents_md",
            RuleSource::RepoInstructions => "repo_instructions",
            RuleSource::GlobalInstructions => "global_instructions",
            RuleSource::Skill => "skill",
            RuleSource::ToolDescription => "tool_description",
            RuleSource::RoutingRule => "routing_rule",
        }
    }
}

impl FromStr for RuleSource {
    type Err = AutospecError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let err = || {
            AutospecError::parse(
                "insights.rule_source",
                format!("unknown §41 rule source {value:?}"),
            )
        };
        Ok(match value {
            "agents_md" => RuleSource::AgentsMd,
            "repo_instructions" => RuleSource::RepoInstructions,
            "global_instructions" => RuleSource::GlobalInstructions,
            "skill" => RuleSource::Skill,
            "tool_description" => RuleSource::ToolDescription,
            "routing_rule" => RuleSource::RoutingRule,
            _ => return Err(err()),
        })
    }
}

/// A rule drawn from one instruction source, used as the left-hand side of a
/// §41 conflict check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// Which instruction source the rule comes from.
    pub source: RuleSource,
    /// The rule text (e.g. an AGENTS.md line or a skill instruction).
    pub text: String,
}

/// The kind of conflict found between an existing rule and a proposed rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    /// One side obligates a subject ("always use X") while the other
    /// forbids it ("never use X") on overlapping subjects — the §41
    /// contradictory-rule pair.
    OpposingObligation,
}

/// A §41 conflict: an existing rule from an instruction source contradicts a
/// rule the proposal would introduce. Surfaced during proposal evaluation;
/// never resolved silently by this module (§4.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    /// How the two rules contradict each other.
    pub kind: ConflictKind,
    /// The existing rule's text.
    pub existing_rule: String,
    /// Where the existing rule lives.
    pub existing_source: RuleSource,
    /// The proposed rule's text (a patch-added line).
    pub proposed_rule: String,
}

/// §41: check the rules a proposal would introduce (its patch-added lines)
/// against the existing rules of every instruction source. Returns one
/// [`Conflict`] per contradicting pair; an empty result means no structural
/// contradiction was found, not that the proposal is safe.
pub fn detect_conflicts(existing_rules: &[Rule], proposal: &Proposal) -> Vec<Conflict> {
    let mut conflicts = Vec::new();
    for proposed in proposed_rules(&proposal.patch) {
        for existing in existing_rules {
            if is_opposing_obligation(&existing.text, &proposed) {
                conflicts.push(Conflict {
                    kind: ConflictKind::OpposingObligation,
                    existing_rule: existing.text.clone(),
                    existing_source: existing.source,
                    proposed_rule: proposed.clone(),
                });
            }
        }
    }
    conflicts
}

/// §40 policy priority for the same change, weakest to strongest. Growth
/// control: prefer the strongest deterministic enforcement available and
/// fall back to prompt text only when no deterministic mechanism fits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnforcementLevel {
    /// An AGENTS.md rule — prompt text, the weakest lever (§40: deterministic
    /// enforcement is preferred over prompt text).
    AgentsMdRule,
    /// A global-skill instruction.
    GlobalSkill,
    /// A repo-local skill instruction.
    RepoLocalSkill,
    /// A quality gate that scores the output.
    QualityGate,
    /// Validation inside a tool wrapper (deterministic, per call).
    ToolValidation,
    /// Fully deterministic enforcement — lint, CI gate, pre-commit hook; the
    /// strongest lever and the §40 default target.
    Deterministic,
}

impl EnforcementLevel {
    /// §40: the strongest available enforcement level wins; prompt-text
    /// levers (skills, AGENTS.md rules) are only used when no deterministic
    /// mechanism applies.
    pub fn strongest(levels: impl IntoIterator<Item = Self>) -> Option<Self> {
        levels.into_iter().max()
    }
}

/// A rule the proposal would introduce: a non-empty line added by the
/// proposal's unified-diff `patch` text.
fn proposed_rules(patch: &str) -> Vec<String> {
    patch
        .lines()
        .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
        .map(|line| line.trim_start_matches('+').trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

/// True when one rule obligates a subject and the other forbids it on
/// overlapping subjects — the §41 "always use X" / "never use X" pair.
fn is_opposing_obligation(existing: &str, proposed: &str) -> bool {
    let (existing_modality, proposed_modality) = (modality(existing), modality(proposed));
    let opposing = (existing_modality == Modality::Obligatory
        && proposed_modality == Modality::Prohibitive)
        || (existing_modality == Modality::Prohibitive
            && proposed_modality == Modality::Obligatory);
    if !opposing {
        return false;
    }
    let existing_subject = subject_tokens(existing);
    let proposed_subject = subject_tokens(proposed);
    subject_prefix_overlap(&existing_subject, &proposed_subject)
}

/// The obligation polarity of a rule: obligatory ("always use X"),
/// prohibitive ("never use X"), or neutral (no obligation language).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Modality {
    Obligatory,
    Prohibitive,
    Neutral,
}

const OBLIGATORY_WORDS: &[&str] = &[
    "always",
    "must",
    "require",
    "requires",
    "required",
    "shall",
    "mandatory",
];
const PROHIBITIVE_WORDS: &[&str] = &[
    "never",
    "avoid",
    "forbid",
    "forbidden",
    "prohibit",
    "prohibited",
    "ban",
    "banned",
];
/// Two-word prohibitive phrases, checked after normalization ("do not",
/// "don t", ...).
const PROHIBITIVE_PHRASES: &[(&str, &str)] = &[
    ("must", "not"),
    ("do", "not"),
    ("shall", "not"),
    ("should", "not"),
    ("can", "not"),
    ("don", "t"),
];
/// The verb a rule's subject hangs off ("use X", "call X", "prefer X").
const SUBJECT_VERBS: &[&str] = &[
    "use", "uses", "using", "call", "calls", "running", "run", "prefer", "adds", "add", "remove",
    "install", "disable", "enable", "write", "writes",
];
/// Tokens that carry no subject meaning.
const SUBJECT_STOPWORDS: &[&str] = &[
    "always", "never", "must", "not", "do", "don", "t", "should", "shall", "can", "may", "in",
    "for", "when", "the", "a", "an", "of", "to", "this", "that", "on", "with", "and", "or", "your",
    "all", "any", "code",
];

/// Lowercase and collapse every non-alphanumeric character to a space so
/// punctuation and case can't hide a match.
fn normalize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.to_lowercase().chars() {
        if ch.is_alphanumeric() {
            current.push(ch);
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn contains_phrase(tokens: &[String], first: &str, second: &str) -> bool {
    tokens
        .windows(2)
        .any(|window| window[0] == first && window[1] == second)
}

fn modality(text: &str) -> Modality {
    let tokens = normalize(text);
    for word in PROHIBITIVE_WORDS {
        if tokens.iter().any(|token| token == word) {
            return Modality::Prohibitive;
        }
    }
    for (first, second) in PROHIBITIVE_PHRASES {
        if contains_phrase(&tokens, first, second) {
            return Modality::Prohibitive;
        }
    }
    for word in OBLIGATORY_WORDS {
        if tokens.iter().any(|token| token == word) {
            return Modality::Obligatory;
        }
    }
    Modality::Neutral
}

/// The rule's subject: the tokens after its verb (if any), stopwords
/// removed. "Always use helper X." -> ["helper", "x"].
fn subject_tokens(text: &str) -> Vec<String> {
    let tokens = normalize(text);
    let start = tokens
        .iter()
        .position(|token| SUBJECT_VERBS.contains(&token.as_str()))
        .map(|index| index + 1)
        .unwrap_or(0);
    tokens[start..]
        .iter()
        .filter(|token| !SUBJECT_STOPWORDS.contains(&token.as_str()))
        .cloned()
        .collect()
}

/// Overlap test: one subject is a prefix of the other (both non-empty),
/// i.e. the narrower rule scopes the broader one ("helper x" vs "helper x
/// in service code").
fn subject_prefix_overlap(first: &[String], second: &[String]) -> bool {
    if first.is_empty() || second.is_empty() {
        return false;
    }
    is_prefix(first, second) || is_prefix(second, first)
}

fn is_prefix(short: &[String], long: &[String]) -> bool {
    short.len() <= long.len() && short.iter().zip(long.iter()).all(|(a, b)| a == b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::proposals::schema::{
        EvaluationPlan, ExpectedEffect, Proposal, ProposalStatus, ProposalType,
    };

    fn proposal_with_patch(patch: &str) -> Proposal {
        Proposal {
            proposal_id: "proposal_conflict_test".to_string(),
            finding_id: "finding_001".to_string(),
            kind: ProposalType::AgentInstruction,
            title: "Rule change".to_string(),
            status: ProposalStatus::default(),
            rationale: "test".to_string(),
            evidence: vec![],
            expected_effect: ExpectedEffect::default(),
            risks: vec![],
            affected_components: vec![],
            patch: patch.to_string(),
            evaluation_plan: EvaluationPlan::default(),
            created_by_model: "qwen3-32b".to_string(),
            review_model: None,
        }
    }

    #[test]
    fn the_section_41_contradictory_rule_pair_produces_exactly_one_conflict() {
        let existing = vec![Rule {
            source: RuleSource::AgentsMd,
            text: "Always use helper X.".to_string(),
        }];
        let proposal = proposal_with_patch(
            "--- a/AGENTS.md\n+++ b/AGENTS.md\n+Never use helper X for service code.\n",
        );
        let conflicts = detect_conflicts(&existing, &proposal);
        assert_eq!(conflicts.len(), 1, "got: {conflicts:?}");
        let conflict = &conflicts[0];
        assert_eq!(conflict.kind, ConflictKind::OpposingObligation);
        assert_eq!(conflict.existing_source, RuleSource::AgentsMd);
        assert_eq!(conflict.existing_rule, "Always use helper X.");
        assert_eq!(
            conflict.proposed_rule,
            "Never use helper X for service code."
        );
    }

    #[test]
    fn the_conflict_finds_in_the_reverse_direction_too() {
        let existing = vec![Rule {
            source: RuleSource::Skill,
            text: "Never run shell commands in parallel.".to_string(),
        }];
        let proposal = proposal_with_patch(
            "--- a/AGENTS.md\n+++ b/AGENTS.md\n+You must run shell commands in parallel.\n",
        );
        assert_eq!(detect_conflicts(&existing, &proposal).len(), 1);
    }

    #[test]
    fn same_polarity_and_unrelated_pairs_do_not_conflict() {
        let existing = vec![
            Rule {
                source: RuleSource::AgentsMd,
                text: "Always use helper X.".to_string(),
            },
            Rule {
                source: RuleSource::Skill,
                text: "Never use helper Y.".to_string(),
            },
            Rule {
                source: RuleSource::GlobalInstructions,
                text: "Always prefer small diffs.".to_string(),
            },
        ];
        let proposal = proposal_with_patch(
            "--- a/AGENTS.md\n+++ b/AGENTS.md\n+Never use helper X for service code.\n\
             +Never use helper Y for batch jobs.\n\
             +Prefer small diffs.\n",
        );
        // Only the "helper X" pair contradicts: "helper Y" is forbidden on
        // both sides (same polarity, not a conflict), and "small diffs" is
        // neutral vs obligatory, not opposed.
        let conflicts = detect_conflicts(&existing, &proposal);
        assert_eq!(conflicts.len(), 1, "got: {conflicts:?}");
        assert_eq!(conflicts[0].existing_rule, "Always use helper X.");
    }

    #[test]
    fn a_different_subject_produces_no_conflict() {
        let existing = vec![Rule {
            source: RuleSource::AgentsMd,
            text: "Always use helper X.".to_string(),
        }];
        let proposal =
            proposal_with_patch("--- a/AGENTS.md\n+++ b/AGENTS.md\n+Never use helper Y.\n");
        assert!(detect_conflicts(&existing, &proposal).is_empty());
    }

    #[test]
    fn neutral_rules_produce_no_conflict() {
        let existing = vec![Rule {
            source: RuleSource::ToolDescription,
            text: "Runs the query and returns rows.".to_string(),
        }];
        let proposal = proposal_with_patch(
            "--- a/AGENTS.md\n+++ b/AGENTS.md\n+Run the query before committing.\n",
        );
        assert!(detect_conflicts(&existing, &proposal).is_empty());
    }

    #[test]
    fn diff_header_lines_and_blank_additions_are_not_rules() {
        let existing = vec![Rule {
            source: RuleSource::AgentsMd,
            text: "Always use helper X.".to_string(),
        }];
        let proposal = proposal_with_patch("--- a/AGENTS.md\n+++ b/AGENTS.md\n@@ -1 +1 @@\n+\n");
        assert!(detect_conflicts(&existing, &proposal).is_empty());
    }

    #[test]
    fn two_existing_sources_both_contradicting_yield_two_conflicts() {
        let existing = vec![
            Rule {
                source: RuleSource::AgentsMd,
                text: "Always use helper X.".to_string(),
            },
            Rule {
                source: RuleSource::RoutingRule,
                text: "Always use helper X.".to_string(),
            },
        ];
        let proposal =
            proposal_with_patch("--- a/AGENTS.md\n+++ b/AGENTS.md\n+Never use helper X.\n");
        assert_eq!(detect_conflicts(&existing, &proposal).len(), 2);
    }

    #[test]
    fn the_section_40_priority_ranks_deterministic_enforcement_above_prompt_text() {
        let ordered = [
            EnforcementLevel::Deterministic,
            EnforcementLevel::ToolValidation,
            EnforcementLevel::QualityGate,
            EnforcementLevel::RepoLocalSkill,
            EnforcementLevel::GlobalSkill,
            EnforcementLevel::AgentsMdRule,
        ];
        // Strictly descending: deterministic > tool validation > quality
        // gate > repo-local skill > global skill > AGENTS.md rule.
        for pair in ordered.windows(2) {
            assert!(pair[0] > pair[1], "{:?} !> {:?}", pair[0], pair[1]);
        }
        // §40: deterministic enforcement is preferred over prompt text.
        assert!(EnforcementLevel::Deterministic > EnforcementLevel::AgentsMdRule);
        assert!(
            EnforcementLevel::strongest([
                EnforcementLevel::AgentsMdRule,
                EnforcementLevel::Deterministic,
                EnforcementLevel::QualityGate,
            ]) == Some(EnforcementLevel::Deterministic)
        );
        assert!(EnforcementLevel::strongest([]).is_none());
    }
}
