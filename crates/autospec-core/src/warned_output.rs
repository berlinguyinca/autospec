//! A tool that warns on stderr and still writes a result to stdout is a tool
//! whose exit status must be checked (issue #4396).
//!
//! The incident: `comm -23 a b` computed "patches that have not been
//! attempted". Its inputs had been produced with `sort -un` — numerically
//! sorted — and `comm` requires **lexically** sorted input. It printed a
//! diagnostic to stderr:
//!
//! ```text
//! comm: file 1 is not in sorted order
//! ```
//!
//! …and then emitted a result anyway. The result said **588 fresh patches**
//! when the true answer was **164**. Because stdout carried a plausible-looking
//! list, the number was nearly reported as the size of the backlog — a 3.6x
//! overstatement that would have driven a decision about where to spend
//! hours. The same class of error had already produced a wrong backlog figure
//! once before ("121 patches awaiting conversion", where the real number was
//! 11).
//!
//! A failed command announces itself. A command that warns and then emits a
//! result looks like a successful measurement, and piping its stdout onward
//! while ignoring its status converts a *detected* error into a confident
//! wrong answer — which is strictly worse than a crash, because nothing
//! downstream can tell.
//!
//! The primitives here make the rules checkable:
//!
//! 1. **A set operation must not depend on an ordering convention the
//!    producer does not guarantee** ([`SetOp`], [`OrderVerdict`]). Either do
//!    set operations in a language with real sets (order-independent), or
//!    sort defensively at the point of use with the exact collation the
//!    consumer requires — never relying on how the input was produced.
//! 2. **A tool that warns on stderr and still writes a result to stdout is a
//!    tool whose exit status must be checked** ([`Step`], [`StepOutcome`]).
//!    A step that warned and whose status was not checked is a
//!    `ConfidentWrongAnswer`, not a result.
//! 3. **Any pipeline step whose output feeds a reported number needs a sanity
//!    assertion on the result** ([`CountBound`]). Here, "fresh cannot exceed
//!    total patches" — 588 == 588 — was the giveaway.
//! 4. **When a spec asks for a count of outstanding work, it must name the
//!    filter** ([`BacklogQuestion`], [`Filter`]): "has a patch" (588), "has
//!    no branch or PR" (164), and "…and the issue is still open" (75) are
//!    three different numbers for the same question. Reporting the wrong one
//!    is not a rounding error — it is an 8x misstatement of the remaining
//!    work.

/// The collation a producer guarantees for its output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Collation {
    /// Lexical byte order — what plain `sort` (and `LC_ALL=C sort`)
    /// guarantees.
    Lexical,
    /// Numeric order — what `sort -n` / `sort -un` guarantee. The producer
    /// ordered by numeric value, not by bytes.
    Numeric,
}

impl Collation {
    /// The one-word label used in report lines.
    pub fn label(self) -> &'static str {
        match self {
            Self::Lexical => "lexicographic",
            Self::Numeric => "numeric",
        }
    }
}

/// A set operation that requires its inputs to be sorted in a particular
/// collation — and, when they are not, warns on stderr and still emits a
/// result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderConsumer {
    /// `comm` compares lines lexically.
    Comm,
}

impl OrderConsumer {
    /// The command name, for report lines.
    pub fn name(self) -> &'static str {
        match self {
            Self::Comm => "comm",
        }
    }

    /// The collation the consumer requires at the point of use.
    pub fn requires(self) -> Collation {
        match self {
            Self::Comm => Collation::Lexical,
        }
    }
}

/// A producer/consumer pair: input produced by a command that guarantees one
/// collation, consumed by a set operation that requires another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetOp {
    /// The producing command (`"sort -un"`).
    pub producer: String,
    /// The collation the producer guarantees.
    pub produced: Collation,
    /// The set operation the output was fed to.
    pub consumer: OrderConsumer,
}

/// What a producer/consumer pair is worth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderVerdict {
    /// The producer guarantees the collation the consumer requires.
    Safe {
        producer: String,
        consumer: String,
        collation: Collation,
    },
    /// The producer's collation is not the collation the consumer requires.
    /// The consumer warns on stderr and still emits a result, so the result
    /// is untrustworthy.
    Unordered {
        producer: String,
        produced: Collation,
        consumer: String,
        required: Collation,
    },
}

impl OrderVerdict {
    /// One-line rendering for a review, a closeout, or a pipeline audit.
    pub fn line(&self) -> String {
        match self {
            Self::Safe {
                producer,
                consumer,
                collation,
            } => format!(
                "OK: '{producer}' feeds '{consumer}' — the producer guarantees the {} order {consumer} requires",
                collation.label()
            ),
            Self::Unordered {
                producer,
                produced,
                consumer,
                required,
            } => format!(
                "FAIL: '{producer}' feeds '{consumer}' — {} order is not the {} order {consumer} requires; {consumer} warns on stderr and still emits a result: sort defensively at the point of use with the {} collation, or do the set operation in a language with real sets",
                produced.label(),
                required.label(),
                required.label()
            ),
        }
    }
}

/// Check a set operation against the collation its consumer requires.
///
/// Invariant 1: the consumer's requirement at the point of use is the
/// contract. How the input was produced is not a guarantee the consumer may
/// rely on.
pub fn check_order(op: &SetOp) -> OrderVerdict {
    let required = op.consumer.requires();
    if op.produced == required {
        OrderVerdict::Safe {
            producer: op.producer.clone(),
            consumer: op.consumer.name().to_string(),
            collation: required,
        }
    } else {
        OrderVerdict::Unordered {
            producer: op.producer.clone(),
            produced: op.produced,
            consumer: op.consumer.name().to_string(),
            required,
        }
    }
}

/// Evidence about one pipeline step whose output feeds a reported number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The command or step name that produced the result (`"comm -23"`).
    pub tool: String,
    /// Whether stderr carried a diagnostic — a warning that the emitted
    /// result may not be what it looks like.
    pub warned: bool,
    /// Whether the exit status was checked: `set -o pipefail`, an explicit
    /// `|| exit`, or `$?` inspected before the stdout was used.
    pub status_checked: bool,
}

/// What a step's emitted result is worth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    /// The step warned nothing: its result may be piped onward.
    Clean,
    /// The step warned and its status was checked: the error is detected,
    /// and its emitted result is not used.
    Detected,
    /// The step warned and its status was not checked: a detected error has
    /// been converted into a confident wrong answer — strictly worse than a
    /// crash, because nothing downstream can tell.
    ConfidentWrongAnswer,
}

impl Step {
    /// Classify the step (invariant 2).
    pub fn outcome(&self) -> StepOutcome {
        if !self.warned {
            StepOutcome::Clean
        } else if self.status_checked {
            StepOutcome::Detected
        } else {
            StepOutcome::ConfidentWrongAnswer
        }
    }

    /// One-line rendering, naming the tool.
    pub fn line(&self) -> String {
        match self.outcome() {
            StepOutcome::Clean => {
                format!("OK: '{}' warned nothing — its result may be piped onward", self.tool)
            }
            StepOutcome::Detected => format!(
                "OK: '{}' warned and its exit status was checked — the error is detected and its emitted result is not used",
                self.tool
            ),
            StepOutcome::ConfidentWrongAnswer => format!(
                "FAIL: '{}' warned on stderr and still wrote a result, and its exit status was not checked — piping that stdout onward converts a detected error into a confident wrong answer, strictly worse than a crash, because nothing downstream can tell",
                self.tool
            ),
        }
    }
}

/// A sanity assertion between a computed number and the known bound the
/// number was filtered from (invariant 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountBound {
    /// The reported number, named (`"fresh patches"`).
    pub name: String,
    /// The computed number.
    pub count: u64,
    /// The unfiltered total the number was filtered from.
    pub total: u64,
}

impl CountBound {
    /// More than the total is a state that cannot exist: the number was not
    /// measured.
    pub fn impossible(&self) -> bool {
        self.count > self.total
    }

    /// A filtered count equal to the unfiltered total: a filter that
    /// excludes anything cannot select everything, so equality is the
    /// giveaway that the filter did not run. Not provably wrong on its own —
    /// in the incident, "588 fresh" against 588 total was exactly this.
    pub fn unfiltered(&self) -> bool {
        self.count == self.total
    }

    /// One-line rendering.
    pub fn line(&self) -> String {
        if self.impossible() {
            format!(
                "FAIL: {}={} exceeds the total {} — impossible; the number was not measured",
                self.name, self.count, self.total
            )
        } else if self.unfiltered() {
            format!(
                "WARN: {}={} equals the total {} — a filter that excludes anything cannot select everything; the filter may not have run",
                self.name, self.count, self.total
            )
        } else {
            format!(
                "OK: {}={} is within its bound (total {}) and does not equal it",
                self.name, self.count, self.total
            )
        }
    }
}

/// The filters the incident's numbers correspond to, over the same corpus of
/// 588 patches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    /// "has a patch" — 588.
    HasPatch,
    /// "has no branch or PR" — 164.
    NoBranchOrPr,
    /// "has no branch or PR and the issue is still open" — 75.
    NoBranchOrPrAndOpen,
}

impl Filter {
    /// The filter as the spec must name it.
    pub fn label(self) -> &'static str {
        match self {
            Self::HasPatch => "has a patch",
            Self::NoBranchOrPr => "has no branch or PR",
            Self::NoBranchOrPrAndOpen => "has no branch or PR and the issue is still open",
        }
    }
}

/// A spec question asking for a count of outstanding work (invariant 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BacklogQuestion {
    /// The filter the count is under. `None` is a finding: without a named
    /// filter the question has no single answer.
    pub filter: Option<Filter>,
}

impl BacklogQuestion {
    /// A count of outstanding work without a named filter is a finding:
    /// "has a patch" (588), "has no branch or PR" (164), and "…and the issue
    /// is still open" (75) are three different numbers for the same question.
    /// Reporting the wrong one is not a rounding error — it is an 8x
    /// misstatement of the remaining work.
    pub fn finding(&self) -> Option<&'static str> {
        match self.filter {
            Some(_) => None,
            None => Some(
                "a count of outstanding work must name its filter — 'has a patch' (588), 'has no branch or PR' (164), and '…and the issue is still open' (75) are three different numbers for the same question",
            ),
        }
    }

    /// The line to report the number on, so the reader sees the filter the
    /// number is under: `164 outstanding (has no branch or PR)`.
    pub fn report(&self, count: u64) -> String {
        match self.filter {
            Some(f) => format!("{count} outstanding ({})", f.label()),
            None => format!("{count} outstanding (filter unnamed — not a number)"),
        }
    }
}
