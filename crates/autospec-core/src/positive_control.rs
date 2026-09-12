//! A negative search result requires a positive control (issue #4422).
//!
//! The incident: the analyst reported "zero issues in this repository declare
//! dependencies" and filed it upstream as a process defect. **133 issues
//! declare dependencies, with 287 edges.** They use a `## Dependencies`
//! heading followed by a list; the analyst searched for `Depends on #N`,
//! `Dependencies:`, `Blocked by`, and `Requires` as inline text, and none of
//! those patterns matched a heading. A wrong issue was filed against another
//! team's repository, a working tool was declared broken, and a plan was made
//! to replace a working dependency graph with a title-parsing heuristic. The
//! frontier computation had been returning the right answer — *nothing is
//! ready* — the whole time.
//!
//! The difference from tool coverage ([`super::negative_evidence`], issue
//! #4344): the tool *could* see the corpus. The failure was the **query**. A
//! search returning zero is evidence about two things at once — the corpus,
//! and the query — and only one of them had been checked. The query's
//! correctness is checked by the positive control: run the same query against
//! a case you independently know contains the data, and see whether it finds
//! it. If it cannot find a known-present case, it is the query that is broken,
//! and the zero over the real corpus is void. The confirming detail was
//! already in the data: issue #216 states it built its DAG "from the declared
//! `## Dependencies` blocks", so the format was discoverable before the
//! absence was asserted.
//!
//! The primitives here make the invariants checkable:
//!
//! 1. **A negative result requires a positive control**
//!    ([`PositiveControl`]). Before reporting "X does not appear", find one
//!    instance of something the query *should* match. If the query cannot find
//!    a known-present case, it is the query that is broken
//!    ([`Verdict::QueryIsTheFinding`]); if no control was run at all, the
//!    negative is not usable as evidence of absence
//!    ([`Verdict::NoPositiveControl`]).
//! 2. **Establish the actual format before asserting its absence**
//!    ([`FormatBasis`]). Read two or three real examples first. A negative
//!    asserted on a guessed schema reports on the guess
//!    ([`Verdict::FormatNotEstablished`]).
//! 3. **Scale scepticism to the strength of the claim.** "Zero out of 183" is
//!    an extraordinary claim about a working system, and it requires a second,
//!    independent method before it is filed, not after
//!    ([`Verdict::NeedsSecondMethod`]).
//!
//! For specs and for tooling: an analysis that can produce "none found" must
//! state how it distinguishes *absent* from *not-matched*. In practice that
//! means shipping a known-positive fixture alongside the real corpus and
//! asserting that the query finds it: a parser that cannot find the example in
//! its own test data must **fail rather than report zero**. The checkable
//! form of that rule is [`Verdict::holds`] — a negative may be reported as
//! absence only when it holds; every other verdict is a failure of the
//! analysis and must be reported as such, not as "0 found".

use std::fmt;

/// How the search format was established before the absence was asserted
/// (invariant 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatBasis {
    /// The format was established by reading real examples of the data first —
    /// the `## Dependencies` heading was read in issue #216 before it was
    /// searched for.
    Established,
    /// The format was guessed: the schema was assumed, then searched for. A
    /// negative asserted on this basis reports on the guess, not the corpus.
    Guessed,
}

impl FormatBasis {
    /// The one-word label used in report lines.
    pub fn label(self) -> &'static str {
        match self {
            Self::Established => "established",
            Self::Guessed => "guessed",
        }
    }
}

/// A known-present fixture of the data: one instance the query *should* match
/// if it is correct (invariant 1).
///
/// The fixture is, by definition, known to contain the data independently of
/// the query — you read the raw issue and saw the `## Dependencies` heading.
/// Running the query against it is the only thing that can separate "the
/// corpus is empty of X" from "the query does not match X".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositiveControl {
    /// What the fixture is and why it is known-present
    /// (`"issue #216, which declares dependencies under a '## Dependencies'
    /// heading"`).
    pub label: String,
    /// Whether the query found the fixture. A `false` here is the finding:
    /// the query cannot find a known-present case, so it is the query that is
    /// broken, not that the data is absent.
    pub matched: bool,
}

impl PositiveControl {
    /// Construct a control. The label is mandatory: a fixture that names
    /// nothing cannot be read back into an investigation.
    pub fn new(label: &str, matched: bool) -> Option<Self> {
        if label.trim().is_empty() {
            return None;
        }
        Some(Self {
            label: label.to_string(),
            matched,
        })
    }

    /// The line to record the control under. A control that did not match is
    /// itself the finding and says so.
    pub fn line(&self) -> String {
        if self.matched {
            format!(
                "positive control OK: the query found the known-present fixture '{0}'",
                self.label
            )
        } else {
            format!(
                "FAIL: the query did not find the known-present fixture '{}' — the query is the finding; the zero is not-matched, not absent",
                self.label
            )
        }
    }
}

/// How strong the negative claim is (invariant 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimStrength {
    /// The claim does not carry extraordinary weight: a small or unverified
    /// population, or a claim that does not contradict a system believed to
    /// work.
    Ordinary,
    /// "Zero out of N" over a working system with N items — an extraordinary
    /// claim that should trigger a second method before it is filed.
    Extraordinary,
}

impl ClaimStrength {
    /// The one-word label used in report lines.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ordinary => "ordinary",
            Self::Extraordinary => "extraordinary",
        }
    }
}

/// The negative claim under test: "zero of N declare X".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegativeClaim {
    /// The tool or query that produced the zero.
    pub query: String,
    /// The size of the corpus the zero was asserted over ("183 issues").
    pub population: usize,
    /// How the search format was established.
    pub format: FormatBasis,
    /// How strong the claim is.
    pub strength: ClaimStrength,
    /// Whether an independent second method was used to produce the claim, in
    /// addition to the query.
    pub second_method: bool,
}

impl NegativeClaim {
    /// Construct a claim. The query is mandatory: a zero with no named source
    /// names nothing.
    pub fn new(
        query: &str,
        population: usize,
        format: FormatBasis,
        strength: ClaimStrength,
        second_method: bool,
    ) -> Option<Self> {
        if query.trim().is_empty() {
            return None;
        }
        Some(Self {
            query: query.to_string(),
            population,
            format,
            strength,
            second_method,
        })
    }
}

/// What a negative claim, checked against its positive control, is worth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The negative may be reported as evidence of absence: the query found
    /// its known-present fixture, the format was established from real
    /// examples, and the claim's strength is matched by the method(s) used.
    Holds { query: String, population: usize },
    /// Invariant 1, unmet: no positive control was run. The negative is not
    /// usable as evidence of absence; the investigation must not terminate on
    /// it.
    NoPositiveControl { query: String, population: usize },
    /// Invariant 1, violated: a positive control was run and the query did
    /// not find a known-present case. It is the query that is broken, and the
    /// zero over the corpus is void — not-matched, not absent.
    QueryIsTheFinding {
        query: String,
        population: usize,
        /// The known-present fixture the query failed to find.
        control: String,
    },
    /// Invariant 2, violated: the format was guessed, not established from
    /// real examples. The negative reports on the guess.
    FormatNotEstablished { query: String, population: usize },
    /// Invariant 3, violated: the claim is extraordinary and no independent
    /// second method was used. It must not be filed yet.
    NeedsSecondMethod { query: String, population: usize },
}

impl Verdict {
    /// Whether the negative may be reported as evidence of absence. This is
    /// the tooling gate: a `false` here is a failure of the analysis and must
    /// be reported as such, never as "0 found".
    pub fn holds(&self) -> bool {
        matches!(self, Self::Holds { .. })
    }

    /// The inverse of [`Self::holds`]: is this negative untrusted as evidence
    /// of absence?
    pub fn untrusted(&self) -> bool {
        !self.holds()
    }

    /// One-line rendering for a review, a closeout, or an investigation
    /// record.
    pub fn line(&self) -> String {
        match self {
            Self::Holds {
                query,
                population,
            } => format!(
                "OK: 0 of {population} for '{query}' is usable as absence — the query found its known-present fixture, the format was established, and the claim's strength is matched by its method(s)"
            ),
            Self::NoPositiveControl {
                query,
                population,
            } => format!(
                "WARN: 0 of {population} for '{query}' has no positive control — not evidence of absence; run the query against a known-present case before reporting the zero"
            ),
            Self::QueryIsTheFinding {
                query,
                population,
                control,
            } => format!(
                "FAIL: '{query}' did not find the known-present fixture '{control}' — the query is the finding, and 0 of {population} is not-matched, not absent"
            ),
            Self::FormatNotEstablished {
                query,
                population,
            } => format!(
                "WARN: 0 of {population} for '{query}' was asserted on a guessed format — it reports on the guess, not the corpus; read two or three real examples first"
            ),
            Self::NeedsSecondMethod {
                query,
                population,
            } => format!(
                "WARN: 0 of {population} for '{query}' is an extraordinary claim about a working system with no second method — run an independent method before filing, not after"
            ),
        }
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.line())
    }
}

/// Check a negative claim against its positive control (invariants 1-3).
///
/// The positive control is the gate: without it, or with it failing, the
/// negative is void, and the other invariants are not reached — a broken
/// query makes every downstream conclusion moot. Once the query is validated
/// by a control it does find, the format must have been established from real
/// examples, and an extraordinary claim must be backed by a second method.
///
/// The priority order is meaningful, not arbitrary: invariant 1 (the control)
/// is what the issue is named after and what the tooling contract reduces to,
/// so it dominates. Invariant 2 then 3 are the requirements for the negative
/// to *hold* once the query is trustworthy.
pub fn verdict(claim: &NegativeClaim, control: Option<&PositiveControl>) -> Verdict {
    let query = claim.query.clone();
    let population = claim.population;

    match control {
        None => Verdict::NoPositiveControl { query, population },
        Some(control) if !control.matched => Verdict::QueryIsTheFinding {
            query,
            population,
            control: control.label.clone(),
        },
        Some(_) => {
            if claim.format == FormatBasis::Guessed {
                Verdict::FormatNotEstablished { query, population }
            } else if claim.strength == ClaimStrength::Extraordinary && !claim.second_method {
                Verdict::NeedsSecondMethod { query, population }
            } else {
                Verdict::Holds { query, population }
            }
        }
    }
}
