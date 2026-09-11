//! Work selection is part of the system, not the invocation (issue #4257).
//!
//! A loop that runs every half hour — "find the agent patches worth
//! converting" — had its selection predicate reconstructed from memory on
//! every pass. Three reconstructions, three distinct defects:
//!
//! 1. The glob spanned four projects whose issue numbers collide
//!    (`issue-14` is InferWeave, `issue-1` is the dispatcher): converting by
//!    bare number would have opened InferWeave patches as autospec pull
//!    requests.
//! 2. It counted issue *directories* rather than finished `changes.patch`
//!    files: a directory appears the moment an agent starts, so in-flight
//!    work was offered as a candidate and an entire pass was spent printing
//!    `SKIP: no patch`.
//! 3. It applied a filter from a previous run that was never re-derived and
//!    reported the backlog "drained" at 2 when it held 78.
//!
//! Each version was written in a hurry, looked right, and produced a
//! plausible number. None was reviewable, because none existed as an
//! artifact — they lived in shell history.
//!
//! The primitives here make each rule a checkable invariant:
//!
//! 1. **A predicate that decides what work to do is part of the system, not
//!    the invocation.** If it is reconstructed at each use, it will be
//!    reconstructed differently, and the difference will be invisible: the
//!    output is a list of numbers that looks equally reasonable whether it
//!    is right or wrong. [`SelectionSpec`] is the artifact: the scope and
//!    every exclusion carry a written condition and a justification, and
//!    construction refuses either an unnamed or an unjustified condition,
//!    because the comment block matters as much as the code — every one of
//!    the conditions of the conversion selector is a bug that was actually
//!    shipped, and written down they stop being rediscoverable
//!    ([`SelectionSpec::render`] is the numbered comment block a selector
//!    file must carry).
//! 2. **The selector reports its denominator.** [`SelectionReport::line`]
//!    is `considered=423 finished_patches=423 have_pr=314 closed_issue=265
//!    attempted=232 -> candidates=0`: every exclusion reports how many
//!    items it removed, and the line always leads with `considered=`.
//!    `candidates=0` alone is unfalsifiable — it cannot be told from "the
//!    query was never run" or "the filter is broken". With the denominator,
//!    a zero is *evidence the backlog is drained*, which is a different and
//!    much more useful statement ([`SelectionReport::verdict`], the same
//!    rule as #3992 applied to the selection step rather than the execution
//!    step).

use std::fmt;

/// Labels the report line owns. An exclusion named after one of them would
/// make the line unreadable: two different counts under one name.
const RESERVED_REPORT_NAMES: [&str; 2] = ["considered", "candidates"];

/// A condition of the selection predicate: what is excluded, and why.
///
/// Constructed through [`SelectionSpec::exclude`], which refuses an unnamed
/// or unjustified condition. The predicate returns `true` for an item the
/// condition *removes* from the candidates.
pub struct Exclusion<T> {
    /// Short report label: the key under which the removal count is
    /// printed in [`SelectionReport::line`] (`finished_patches`, `have_pr`).
    name: String,
    /// The written condition, rendered in the numbered comment block ("a
    /// NON-EMPTY changes.patch exists").
    condition: String,
    /// Why the condition exists: every one of these is a bug that was
    /// actually shipped, and written down they stop being rediscoverable.
    justification: String,
    predicate: Box<dyn Fn(&T) -> bool>,
}

impl<T> Exclusion<T> {
    /// The short report label.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The written condition.
    pub fn condition(&self) -> &str {
        &self.condition
    }

    /// The justification for the condition.
    pub fn justification(&self) -> &str {
        &self.justification
    }

    /// Whether this condition removes the item.
    pub fn removes(&self, item: &T) -> bool {
        (self.predicate)(item)
    }
}

/// A defect in a [`SelectionSpec`]: the predicate is not an artifact until
/// every condition of it is named and justified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecError {
    /// A condition or scope with no name: the report would print an unnamed
    /// count, and an unnamed exclusion cannot be checked against the
    /// comment block.
    EmptyName,
    /// A condition or scope with no written justification. A condition
    /// without a written reason is the predicate being reconstructed from
    /// memory.
    EmptyJustification,
    /// Two conditions with the same report label: the report would count
    /// two different removals under one number.
    DuplicateName(String),
    /// A condition named `considered` or `candidates`: those labels belong
    /// to the report line, and a collision makes the line unreadable.
    ReservedName(String),
}

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => write!(f, "a selection condition needs a name"),
            Self::EmptyJustification => {
                write!(f, "a selection condition needs a written justification")
            }
            Self::DuplicateName(name) => {
                write!(f, "two selection conditions named `{name}`")
            }
            Self::ReservedName(name) => write!(
                f,
                "`{name}` is a report-line label (considered/candidates) and \
                 cannot be an exclusion name"
            ),
        }
    }
}

impl std::error::Error for SpecError {}

fn reject_unnamed(what: &str) -> Result<(), SpecError> {
    if what.trim().is_empty() {
        Err(SpecError::EmptyName)
    } else {
        Ok(())
    }
}

fn reject_unjustified(what: &str) -> Result<(), SpecError> {
    if what.trim().is_empty() {
        Err(SpecError::EmptyJustification)
    } else {
        Ok(())
    }
}

/// The work-selection predicate as an artifact: the scope that defines the
/// input set, and every exclusion, each named and justified.
///
/// The scope is the first condition of the numbered comment block; it
/// decides what the query may see at all. The 2026-09-07 conversion pass
/// scoped its glob to four projects whose issue numbers collide — a bare
/// number addressed the wrong project, and the defect was invisible in the
/// output because the numbers looked equally reasonable.
pub struct SelectionSpec<T> {
    scope_condition: String,
    scope_justification: String,
    exclusions: Vec<Exclusion<T>>,
}

/// Manual [`fmt::Debug`] for [`SelectionSpec`]: the artifact is its written
/// conditions, not its closures.
impl<T> fmt::Debug for SelectionSpec<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SelectionSpec")
            .field("scope_condition", &self.scope_condition)
            .field("scope_justification", &self.scope_justification)
            .field(
                "exclusions",
                &self
                    .exclusions
                    .iter()
                    .map(|e| (e.name(), e.condition(), e.justification()))
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl<T> SelectionSpec<T> {
    /// Start the spec with the scope: the written condition that defines
    /// the input set, and the reason it must be that narrow.
    pub fn new(scope_condition: &str, scope_justification: &str) -> Result<Self, SpecError> {
        reject_unnamed(scope_condition)?;
        reject_unjustified(scope_justification)?;
        Ok(Self {
            scope_condition: scope_condition.to_string(),
            scope_justification: scope_justification.to_string(),
            exclusions: Vec::new(),
        })
    }

    /// Add an exclusion: the short report label, the written condition,
    /// the justification, and the predicate that removes matching items.
    ///
    /// Refuses an empty label, an empty condition, an empty justification,
    /// a duplicate label, and a label reserved by the report line — a spec
    /// that fails this check is not an artifact yet.
    pub fn exclude<F: Fn(&T) -> bool + 'static>(
        mut self,
        name: &str,
        condition: &str,
        justification: &str,
        predicate: F,
    ) -> Result<Self, SpecError> {
        reject_unnamed(name)?;
        reject_unnamed(condition)?;
        reject_unjustified(justification)?;
        if RESERVED_REPORT_NAMES.contains(&name) {
            return Err(SpecError::ReservedName(name.to_string()));
        }
        if self.exclusions.iter().any(|e| e.name == name) {
            return Err(SpecError::DuplicateName(name.to_string()));
        }
        self.exclusions.push(Exclusion {
            name: name.to_string(),
            condition: condition.to_string(),
            justification: justification.to_string(),
            predicate: Box::new(predicate),
        });
        Ok(self)
    }

    /// The scope, as (condition, justification).
    pub fn scope(&self) -> (&str, &str) {
        (&self.scope_condition, &self.scope_justification)
    }

    /// The report labels of the exclusions, in spec order.
    pub fn exclusion_names(&self) -> Vec<&str> {
        self.exclusions.iter().map(|e| e.name.as_str()).collect()
    }

    /// Run the predicate over the items the scope already selected.
    ///
    /// Every exclusion reports how many items it removed (an item removed
    /// by several conditions counts once under each); `candidates` are the
    /// items no condition removed.
    pub fn select(&self, items: &[T]) -> SelectionReport {
        let mut counts = vec![0usize; self.exclusions.len()];
        let mut candidates = 0usize;
        let mut excluded = 0usize;
        for item in items {
            let mut removed_by_any = false;
            for (exclusion, count) in self.exclusions.iter().zip(counts.iter_mut()) {
                if exclusion.removes(item) {
                    *count += 1;
                    removed_by_any = true;
                }
            }
            if removed_by_any {
                excluded += 1;
            } else {
                candidates += 1;
            }
        }
        let removals = self
            .exclusions
            .iter()
            .zip(counts)
            .map(|(e, removed)| ExclusionCount {
                name: e.name.clone(),
                removed,
            })
            .collect();
        SelectionReport {
            considered: items.len(),
            removals,
            candidates,
            excluded,
        }
    }

    /// The numbered comment block a selector file must carry: the scope
    /// first, then every exclusion, each as `# <n>. <condition>
    /// (<justification>)`.
    ///
    /// This is the artifact half of the rule: the block is what a reviewer
    /// reads instead of re-deriving the predicate from a run, and it is
    /// rendered, not hand-typed, so it cannot drift from the code.
    pub fn render(&self) -> String {
        let rows: Vec<(&str, &str)> = std::iter::once((
            self.scope_condition.as_str(),
            self.scope_justification.as_str(),
        ))
        .chain(
            self.exclusions
                .iter()
                .map(|e| (e.condition(), e.justification())),
        )
        .collect();
        let width = rows
            .iter()
            .map(|(condition, _)| condition.len())
            .max()
            .unwrap_or(0);
        let mut out = String::new();
        for (i, (condition, justification)) in rows.iter().enumerate() {
            out.push_str(&format!(
                "# {:>3}. {:<width$}  ({})\n",
                i + 1,
                condition,
                justification,
                width = width
            ));
        }
        out
    }
}

/// The number of items one exclusion removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExclusionCount {
    /// The exclusion's report label.
    pub name: String,
    /// How many items the exclusion removed. An item removed by several
    /// conditions counts once under each.
    pub removed: usize,
}

/// The outcome of a selection pass, with its denominator.
///
/// The line always leads with `considered=`: `candidates=0` without it is
/// unfalsifiable — the same zero means "backlog drained", "the query never
/// ran", and "the filter is broken", and the output cannot tell them
/// apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionReport {
    considered: usize,
    removals: Vec<ExclusionCount>,
    candidates: usize,
    excluded: usize,
}

impl SelectionReport {
    /// The denominator: how many items the scope produced.
    pub fn considered(&self) -> usize {
        self.considered
    }

    /// How many items no condition removed.
    pub fn candidates(&self) -> usize {
        self.candidates
    }

    /// How many items at least one condition removed.
    pub fn excluded(&self) -> usize {
        self.excluded
    }

    /// How many items the named exclusion removed, if the spec has it.
    pub fn removed_by(&self, name: &str) -> Option<usize> {
        self.removals
            .iter()
            .find(|r| r.name == name)
            .map(|r| r.removed)
    }

    /// The report line:
    /// `considered=423 finished_patches=423 have_pr=314 closed_issue=265
    /// attempted=232 -> candidates=0`.
    ///
    /// Every exclusion reports how many items it removed, and the line
    /// always carries the denominator, so a zero is a claim that can be
    /// checked against it.
    pub fn line(&self) -> String {
        let mut line = format!("considered={}", self.considered);
        for count in &self.removals {
            line.push(' ');
            line.push_str(&count.name);
            line.push('=');
            line.push_str(&count.removed.to_string());
        }
        line.push_str(" -> candidates=");
        line.push_str(&self.candidates.to_string());
        line
    }

    /// Whether the counts reconcile: every input is either a candidate or
    /// excluded, nothing is removed more times than items exist to remove,
    /// and the union of removals cannot exceed their sum. A report whose
    /// numbers do not reconcile is reporting a state that cannot exist.
    pub fn reconciles(&self) -> bool {
        let sum: usize = self.removals.iter().map(|r| r.removed).sum();
        self.considered == self.candidates + self.excluded
            && self.excluded <= sum
            && self.removals.iter().all(|r| r.removed <= self.excluded)
    }

    /// What the zero (or non-zero) actually says.
    pub fn verdict(&self) -> SelectionVerdict {
        if self.considered == 0 {
            SelectionVerdict::NoInputs
        } else if self.candidates == 0 {
            SelectionVerdict::Drained
        } else {
            SelectionVerdict::Work
        }
    }

    /// The statement the verdict supports, for the log.
    ///
    /// `NoInputs` is never rendered as "drained": reporting a zero without
    /// a denominator as a drained backlog is defect 3 of #4257.
    pub fn statement(&self) -> String {
        match self.verdict() {
            SelectionVerdict::NoInputs => {
                "the query never ran or the scope matched nothing — not 'drained'".to_string()
            }
            SelectionVerdict::Drained => format!(
                "evidence the backlog is drained ({} considered, every one removed by a named condition)",
                self.considered
            ),
            SelectionVerdict::Work => format!("{} candidate(s) remain", self.candidates),
        }
    }
}

/// What a selection result says, given its denominator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionVerdict {
    /// `considered == 0`: the query never ran, or the scope matched
    /// nothing. Not drained, not empty work — unfalsifiable, and it must be
    /// reported as such.
    NoInputs,
    /// `considered > 0` and `candidates == 0`: every input was removed by a
    /// named condition. With the denominator, this is *evidence the
    /// backlog is drained*.
    Drained,
    /// `candidates > 0`: work to do.
    Work,
}
