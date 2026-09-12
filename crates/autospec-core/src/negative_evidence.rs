//! Negative evidence and tool coverage: a confident zero from a tool whose
//! coverage you have not verified is not evidence of absence (issue #4344).
//!
//! The incident: determining which of two architecture programs owned a
//! component, the investigator asked GitHub code search what the candidate
//! repository contained:
//!
//! ```text
//! 'v1/models'        -> 0 hits
//! 'chat/completions' -> 0 hits
//! 'axum'             -> 0 hits
//! 'TcpListener'      -> 0 hits
//! ```
//!
//! Four zeros, consistent with each other, and about to justify building a
//! second gateway beside one that already existed. The repository contains
//! `openai_api.rs`, `proxy.rs`, `daemon.rs`, a `[[bin]]`, and a live HTTP
//! server. GitHub does not index private repositories for code search and
//! **returns zero rather than an error**: an unavailable index and an empty
//! repository produce the same answer, so a failed measurement and an empty
//! set are indistinguishable in the output.
//!
//! A failed command announces itself. A search that returns no results looks
//! like a successful measurement of an empty set, and the shape of the
//! output is identical either way — the same defect as an empty CI log while
//! the API explains it is refusing to print, or an empty `cron-*.log`
//! because the writer logs elsewhere. The most expensive kind of wrong answer
//! is the one that *terminates the investigation*, and a confident negative
//! is exactly that.
//!
//! The primitives here make the four invariants checkable:
//!
//! 1. **Before trusting a negative, confirm the tool can produce a positive
//!    over that source** ([`Control`]). Search for something certain to be
//!    present; if that also returns zero, the index is the finding
//!    ([`GroupVerdict::ToolIsTheFinding`]) and every zero from that tool
//!    over that source is void.
//! 2. **Prefer enumeration over search when the question is "does X exist
//!    here"** ([`Method::Enumeration`]). A directory listing cannot be
//!    partially indexed: it is present, or it errors.
//! 3. **Mutually consistent zeros from one tool are one observation, not
//!    four** ([`NegativeReport::weight`]). They share a failure mode, so
//!    agreement between them carries no independent weight.
//! 4. **Record the coverage limits of investigative tools where they will be
//!    read** ([`CoverageLimit`]). "GitHub code search does not index private
//!    repositories and returns zero, not an error" is a fact worth stating
//!    once rather than rediscovering.

/// How the question is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// A search over an index. Returns zero both when the set is empty and
    /// when the index does not cover the source; the two are
    /// indistinguishable in the output.
    Search,
    /// A direct enumeration of the source — a directory listing, a manifest,
    /// a database listing. It is present, or it errors; it cannot be
    /// partially indexed.
    Enumeration,
}

impl Method {
    /// The one-word label used in report lines.
    pub fn label(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Enumeration => "enumeration",
        }
    }
}

/// A single negative observation: a query that returned zero over a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// The tool that produced the result (`"github code search"`).
    pub tool: String,
    /// The source searched (a repository, a directory, a log file).
    pub source: String,
    /// What was looked for (`'v1/models'`).
    pub query: String,
    /// How the question was answered.
    pub method: Method,
}

/// A control query: something certain to be present in the source, run
/// through the same tool over the same source.
///
/// The control is the only thing that can separate "the source is empty"
/// from "the tool does not cover the source": it is the positive the
/// negative depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Control {
    /// The tool the control was run through.
    pub tool: String,
    /// The source the control was run over.
    pub source: String,
    /// A term certain to be present in the source.
    pub query: String,
    /// How many hits the control returned. Zero means the tool cannot
    /// produce a positive over that source: the index is the finding.
    pub hits: usize,
}

/// What a group of zeros over one `(tool, source)` pair is worth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupVerdict {
    /// The negative holds.
    Holds {
        tool: String,
        source: String,
        observations: usize,
        basis: HoldsBasis,
    },
    /// The tool's own control returned zero: the index does not cover the
    /// source, and every zero from that tool over that source is void.
    /// "The repository has no HTTP server" is not the finding — "the tool
    /// cannot see this repository" is.
    ToolIsTheFinding {
        tool: String,
        source: String,
        /// The control query that failed to produce a positive.
        control: String,
        /// Zeros voided by the finding.
        observations: usize,
    },
    /// Zeros from a search over a source whose coverage no control has
    /// verified. Not evidence of absence; the investigation must not
    /// terminate on them.
    CoverageUnverified {
        tool: String,
        source: String,
        observations: usize,
    },
}

/// Why a negative holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldsBasis {
    /// The source was enumerated directly; a listing cannot be partially
    /// indexed.
    Enumerated,
    /// The source was searched, and a control through the same tool over the
    /// same source returned a positive.
    Controlled,
}

impl GroupVerdict {
    /// One-line rendering for a review, a closeout, or an investigation
    /// record.
    pub fn line(&self) -> String {
        match self {
            Self::Holds {
                tool,
                source,
                observations,
                basis,
            } => match basis {
                HoldsBasis::Enumerated => format!(
                    "OK: {observations} zero(s) from '{tool}' over '{source}' hold — the source was enumerated directly, and a listing is present or it errors"
                ),
                HoldsBasis::Controlled => format!(
                    "OK: {observations} zero(s) from '{tool}' over '{source}' hold — the tool's control returned a positive over that source"
                ),
            },
            Self::ToolIsTheFinding {
                tool,
                source,
                control,
                observations,
            } => format!(
                "FAIL: the control '{control}' also returned zero — '{tool}' cannot produce a positive over '{source}', so the index is the finding and {observations} zero(s) are void (they are one observation, not {observations})"
            ),
            Self::CoverageUnverified {
                tool,
                source,
                observations,
            } => format!(
                "WARN: {observations} zero(s) from '{tool}' over '{source}' have no positive control — not evidence of absence; confirm the tool can produce a positive over that source, or enumerate it directly"
            ),
        }
    }
}

/// The verdicts for every `(tool, source)` pair the zeros come from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NegativeReport {
    groups: Vec<GroupVerdict>,
}

impl NegativeReport {
    /// The verdict per group, in first-seen order.
    pub fn groups(&self) -> &[GroupVerdict] {
        &self.groups
    }

    /// One line per group.
    pub fn lines(&self) -> Vec<String> {
        self.groups.iter().map(GroupVerdict::line).collect()
    }

    /// Whether any negative is not usable as evidence of absence.
    pub fn any_untrusted(&self) -> bool {
        self.groups
            .iter()
            .any(|g| !matches!(g, GroupVerdict::Holds { .. }))
    }

    /// How many *independent* observations the zeros are.
    ///
    /// Invariant 3: zeros from one tool over one source share a failure
    /// mode, so their mutual consistency carries no weight. Four queries to
    /// one search index over one repository are one observation, however
    /// consistent they look.
    pub fn weight(&self) -> usize {
        self.groups.len()
    }
}

/// Classify negative observations against their controls (invariants 1-3).
///
/// Observations are grouped by `(tool, source)`: that is the unit a
/// coverage question is asked over, and the unit a failure mode is shared
/// by. A zero control for a group voids the group — even the enumerations,
/// because a tool that cannot see what it is told is in a source has proven
/// nothing about that source. A search group with a positive control holds;
/// a search group without any control is `CoverageUnverified`; an
/// enumeration-only group holds on its own, because a listing is present or
/// it errors.
pub fn classify(observations: &[Observation], controls: &[Control]) -> NegativeReport {
    // First-seen order of (tool, source) pairs.
    let mut order: Vec<(String, String)> = Vec::new();
    for obs in observations {
        if !order
            .iter()
            .any(|(t, s)| t == &obs.tool && s == &obs.source)
        {
            order.push((obs.tool.clone(), obs.source.clone()));
        }
    }

    let mut groups = Vec::with_capacity(order.len());
    for (tool, source) in &order {
        let obs: Vec<&Observation> = observations
            .iter()
            .filter(|o| &o.tool == tool && &o.source == source)
            .collect();
        let searches: Vec<&Observation> = obs
            .iter()
            .filter(|o| o.method == Method::Search)
            .copied()
            .collect();
        let has_enumeration = obs.iter().any(|o| o.method == Method::Enumeration);
        let group_controls: Vec<&Control> = controls
            .iter()
            .filter(|c| &c.tool == tool && &c.source == source)
            .collect();
        let n = obs.len();

        if let Some(zero) = group_controls.iter().find(|c| c.hits == 0) {
            groups.push(GroupVerdict::ToolIsTheFinding {
                tool: tool.clone(),
                source: source.clone(),
                control: zero.query.clone(),
                observations: n,
            });
        } else if searches.is_empty() && has_enumeration {
            groups.push(GroupVerdict::Holds {
                tool: tool.clone(),
                source: source.clone(),
                observations: n,
                basis: HoldsBasis::Enumerated,
            });
        } else if group_controls.iter().any(|c| c.hits > 0) {
            groups.push(GroupVerdict::Holds {
                tool: tool.clone(),
                source: source.clone(),
                observations: n,
                basis: HoldsBasis::Controlled,
            });
        } else {
            groups.push(GroupVerdict::CoverageUnverified {
                tool: tool.clone(),
                source: source.clone(),
                observations: n,
            });
        }
    }

    NegativeReport { groups }
}

/// A recorded coverage limit of an investigative tool (invariant 4).
///
/// Stated once, where the tool will be read, it is a fact. Rediscovered, it
/// costs an investigation and nearly a duplicate build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageLimit {
    /// The tool the limit belongs to.
    pub tool: String,
    /// The limit, in the terms that matter for the decision
    /// (`"does not index private repositories; returns zero, not an
    /// error"`).
    pub limit: String,
}

impl CoverageLimit {
    /// Construct a limit. Both fields are mandatory: a limit without a tool
    /// names nothing, and a tool without a stated limit is a placeholder,
    /// not a record.
    pub fn new(tool: &str, limit: &str) -> Option<Self> {
        if tool.trim().is_empty() || limit.trim().is_empty() {
            return None;
        }
        Some(Self {
            tool: tool.to_string(),
            limit: limit.to_string(),
        })
    }

    /// The line to record it under: `limit: <tool> — <limit>`.
    pub fn line(&self) -> String {
        format!("coverage limit: {} — {}", self.tool, self.limit)
    }
}
