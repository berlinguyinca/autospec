//! The Context Package and its token budget (spec sections 18, 19, 20).
//!
//! The package is a structure, not a text blob: the caller needs to tell
//! required evidence from supporting evidence, see contradictions rather than a
//! resolved answer, and know what the retrieval could not settle. Section 18's
//! `token_budget.actual` is the honest part — the builder reports what it
//! actually spent, including when compression forced it to drop items.

use crate::rag::budget::StopReason;
use crate::rag::compression::{CompressionLevel, estimate_tokens};
use crate::rag::contradiction::ContradictionSet;
use crate::rag::evidence::{Evidence, Privacy};
use crate::rag::injection::{self, InjectionFinding};
use crate::rag::policy::AgentRole;

/// A retrieval result shaped for one agent's working memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextPackage {
    id: String,
    task_id: String,
    role: AgentRole,
    summary: Vec<String>,
    required_evidence: Vec<Evidence>,
    supporting_evidence: Vec<Evidence>,
    omitted_evidence: Vec<String>,
    quarantined: Vec<(String, InjectionFinding)>,
    contradictions: ContradictionSet,
    unresolved_questions: Vec<String>,
    suggested_next_actions: Vec<String>,
    stop_reason: StopReason,
    requested_tokens: u32,
    actual_tokens: u32,
    privacy: Privacy,
}

impl ContextPackage {
    /// Package identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Task the retrieval served.
    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    /// Role the package is shaped for.
    pub fn role(&self) -> AgentRole {
        self.role
    }

    /// Short factual statements, each backed by required evidence.
    pub fn summary(&self) -> &[String] {
        &self.summary
    }

    /// Evidence the answer depends on.
    pub fn required_evidence(&self) -> &[Evidence] {
        &self.required_evidence
    }

    /// Evidence that corroborates but is not load-bearing.
    pub fn supporting_evidence(&self) -> &[Evidence] {
        &self.supporting_evidence
    }

    /// Evidence ids the token budget forced out.
    ///
    /// Reported rather than dropped silently: an agent that knows retrieval
    /// found more than it could carry can ask for the rest (section 18).
    pub fn omitted_evidence(&self) -> &[String] {
        &self.omitted_evidence
    }

    /// Evidence withheld as a likely prompt injection, with the finding.
    pub fn quarantined(&self) -> &[(String, InjectionFinding)] {
        &self.quarantined
    }

    /// Contradictions surfaced rather than resolved.
    pub fn contradictions(&self) -> &ContradictionSet {
        &self.contradictions
    }

    /// Questions the retrieval could not settle.
    pub fn unresolved_questions(&self) -> &[String] {
        &self.unresolved_questions
    }

    /// Suggested follow-up retrievals or inspections.
    pub fn suggested_next_actions(&self) -> &[String] {
        &self.suggested_next_actions
    }

    /// Why retrieval stopped (spec section 40).
    pub fn stop_reason(&self) -> &StopReason {
        &self.stop_reason
    }

    /// The token ceiling the builder was given.
    pub fn requested_tokens(&self) -> u32 {
        self.requested_tokens
    }

    /// Tokens the rendered package actually occupies.
    pub fn actual_tokens(&self) -> u32 {
        self.actual_tokens
    }

    /// Strictest privacy class of any included evidence (spec section 53).
    pub fn privacy(&self) -> Privacy {
        self.privacy
    }

    /// Every included evidence item, required first.
    pub fn all_evidence(&self) -> Vec<&Evidence> {
        self.required_evidence
            .iter()
            .chain(self.supporting_evidence.iter())
            .collect()
    }

    /// Return `true` when a blocking contradiction should stop autonomous work.
    pub fn blocks_autonomous_implementation(&self) -> bool {
        self.contradictions.blocking().is_some()
    }

    /// Render the package as fenced prompt text (spec section 29).
    pub fn render(&self) -> String {
        let mut rendered = String::new();
        rendered.push_str(&format!(
            "# Context package {} (task {}, role {})\n",
            self.id,
            self.task_id,
            self.role.as_str()
        ));
        if !self.summary.is_empty() {
            rendered.push_str("\n## Summary\n");
            for line in &self.summary {
                rendered.push_str(&format!("- {line}\n"));
            }
        }
        for (heading, evidence) in [
            ("Required evidence", &self.required_evidence),
            ("Supporting evidence", &self.supporting_evidence),
        ] {
            if evidence.is_empty() {
                continue;
            }
            rendered.push_str(&format!("\n## {heading}\n"));
            for item in evidence {
                rendered.push_str(&injection::fence(item));
                rendered.push('\n');
            }
        }
        if !self.contradictions.is_empty() {
            rendered.push_str("\n## Contradictions (unresolved by design)\n");
            for contradiction in self.contradictions.records() {
                rendered.push_str(&format!("- {}\n", contradiction.summary()));
            }
        }
        if !self.unresolved_questions.is_empty() {
            rendered.push_str("\n## Unresolved questions\n");
            for question in &self.unresolved_questions {
                rendered.push_str(&format!("- {question}\n"));
            }
        }
        if !self.omitted_evidence.is_empty() {
            rendered.push_str(&format!(
                "\n## Omitted for budget\n{} item(s) were retrieved but did not fit: {}\n",
                self.omitted_evidence.len(),
                self.omitted_evidence.join(", ")
            ));
        }
        rendered.push_str(&format!(
            "\n## Retrieval\nstopped: {}\ntokens: {} of {}\n",
            self.stop_reason.describe(),
            self.actual_tokens,
            self.requested_tokens
        ));
        rendered
    }
}

/// Assembles a [`ContextPackage`] under a token ceiling.
#[derive(Debug, Clone)]
pub struct ContextPackageBuilder {
    id: String,
    task_id: String,
    role: AgentRole,
    summary: Vec<String>,
    required: Vec<Evidence>,
    supporting: Vec<Evidence>,
    contradictions: ContradictionSet,
    unresolved_questions: Vec<String>,
    suggested_next_actions: Vec<String>,
    max_tokens: u32,
    compression: CompressionLevel,
}

impl ContextPackageBuilder {
    /// Start a package for a task and role.
    pub fn new(
        id: impl Into<String>,
        task_id: impl Into<String>,
        role: AgentRole,
        max_tokens: u32,
    ) -> Self {
        Self {
            id: id.into(),
            task_id: task_id.into(),
            role,
            summary: Vec::new(),
            required: Vec::new(),
            supporting: Vec::new(),
            contradictions: ContradictionSet::new(),
            unresolved_questions: Vec::new(),
            suggested_next_actions: Vec::new(),
            max_tokens,
            compression: CompressionLevel::Raw,
        }
    }

    /// Add a summary line.
    pub fn summary_line(mut self, line: impl Into<String>) -> Self {
        self.summary.push(line.into());
        self
    }

    /// Add evidence the answer depends on.
    pub fn required(mut self, evidence: Evidence) -> Self {
        self.required.push(evidence);
        self
    }

    /// Add corroborating evidence.
    pub fn supporting(mut self, evidence: Evidence) -> Self {
        self.supporting.push(evidence);
        self
    }

    /// Attach the contradictions found during retrieval.
    pub fn contradictions(mut self, contradictions: ContradictionSet) -> Self {
        self.contradictions = contradictions;
        self
    }

    /// Add a question retrieval could not settle.
    pub fn unresolved(mut self, question: impl Into<String>) -> Self {
        self.unresolved_questions.push(question.into());
        self
    }

    /// Add a suggested next action.
    pub fn next_action(mut self, action: impl Into<String>) -> Self {
        self.suggested_next_actions.push(action.into());
        self
    }

    /// Set the compression level applied when evidence overflows the budget.
    pub fn compression(mut self, level: CompressionLevel) -> Self {
        self.compression = level;
        self
    }

    /// Assemble the package.
    ///
    /// Required evidence is placed first and never dropped: a package that
    /// silently omits a fact the answer rests on is worse than one that admits
    /// it overflowed. Supporting evidence is dropped from the end until the
    /// package fits, and every dropped id is reported.
    pub fn build(self, stop_reason: StopReason) -> Result<ContextPackage, String> {
        let (safe_required, quarantined_required) = injection::partition(&self.required);
        let (safe_supporting, quarantined_supporting) = injection::partition(&self.supporting);
        let quarantined = quarantined_required
            .into_iter()
            .chain(quarantined_supporting)
            .map(|(evidence, finding)| (evidence.id().to_string(), finding))
            .collect::<Vec<_>>();

        let overhead = estimate_tokens(&self.summary.join("\n"))
            + estimate_tokens(&self.unresolved_questions.join("\n"))
            + PACKAGE_FRAME_TOKENS;
        let required_tokens: u32 = safe_required.iter().map(evidence_tokens).sum();
        if overhead + required_tokens > self.max_tokens {
            return Err(format!(
                "required evidence needs {} tokens, above the {} ceiling for role {}; \
                 compress the evidence or raise the role budget",
                overhead + required_tokens,
                self.max_tokens,
                self.role.as_str()
            ));
        }

        let mut spent = overhead + required_tokens;
        let mut supporting = Vec::new();
        let mut omitted = Vec::new();
        for item in safe_supporting {
            let cost = evidence_tokens(&item);
            if spent + cost > self.max_tokens {
                omitted.push(item.id().to_string());
                continue;
            }
            spent += cost;
            supporting.push(item);
        }

        let privacy = safe_required
            .iter()
            .chain(supporting.iter())
            .map(Evidence::privacy)
            .fold(Privacy::Public, Privacy::strictest);

        Ok(ContextPackage {
            id: self.id,
            task_id: self.task_id,
            role: self.role,
            summary: self.summary,
            required_evidence: safe_required,
            supporting_evidence: supporting,
            omitted_evidence: omitted,
            quarantined,
            contradictions: self.contradictions,
            unresolved_questions: self.unresolved_questions,
            suggested_next_actions: self.suggested_next_actions,
            stop_reason,
            requested_tokens: self.max_tokens,
            actual_tokens: spent,
            privacy,
        })
    }
}

/// Tokens the package structure costs before any evidence: headings, the
/// retrieval footer, and the fence markers around the first item.
const PACKAGE_FRAME_TOKENS: u32 = 64;

fn evidence_tokens(evidence: &Evidence) -> u32 {
    // The citation line is part of the cost: provenance is not free, and a
    // budget that ignored it would overshoot on many small snippets.
    estimate_tokens(evidence.content()) + estimate_tokens(&evidence.citation()) + 8
}
