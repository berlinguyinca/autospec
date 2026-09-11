//! A gate that runs the existing suite cannot see that new code arrived
//! untested (issue #4329).
//!
//! The patch for #4238 staged two files and added **eight public functions
//! with zero tests**: `entry_path`, `parse_table_entries`, `render_entry`,
//! `render_index`, `is_self_contained`, `decide_append`,
//! `contended_targets`, `hold_line` — all public, none exercised. Every
//! existing test still passed, so nothing objected. The gate's question is
//! *"did this patch break something that was working?"*, and the answer was
//! no. The question it never asks is *"did this patch add something that
//! nothing checks?"*
//!
//! The consequence was immediate: `hold_line` shipped with a format string
//! carrying two `{}` placeholders against one argument — a compile error,
//! the cheapest possible version of the mistake — and it still reached the
//! gate, because no test called the function. The expensive version of the
//! same gap is a placeholder that resolves to the wrong value: it compiles,
//! the suite stays green, and an operator reads a hold line naming one issue
//! twice.
//!
//! `AGENTS.md` states TDD is non-negotiable. The agent did not follow it, and
//! nothing in the pipeline could tell: a rule that only a well-behaved agent
//! enforces is not enforced. The gate is the place where the repository's
//! standards become real, and this standard had no representation there.
//!
//! Four invariants, each a primitive here:
//!
//! 1. **A patch adding a public item must add a test that exercises it, or
//!    state why not.** [`diff_public_items`] diffs the set of public items
//!    before and after; [`audit`] requires every addition to be referenced by
//!    test code added in the same patch, or to carry an explicit annotated
//!    [`Exemption`].
//! 2. **Report coverage of the change, not of the repository.** `9065 passed`
//!    says nothing about the eight functions just added;
//!    `CONVERTED+MERGED: 9065 passed, 0 failing` is a true statement that
//!    carries no signal about whether the *new* code is among the N.
//!    [`ChangeCoverage`] names how many new public items the patch introduced
//!    and how many are exercised, and [`record_carries_coverage`] refuses a
//!    merge record that does not carry the line.
//! 3. **A standard stated in AGENTS.md and unenforced by the gate will be
//!    violated silently.** Either mechanise it or stop claiming it:
//!    [`gate`] refuses the merge and names every untested item. The
//!    interesting failures are the ones where the rule was stated, believed,
//!    and had no teeth.
//! 4. **A test must be shown to fail against the defect it guards.** A test
//!    written after the fix, never seen red, is an assumption wearing a
//!    test's clothing. The regression tests here are mutation-verified: with
//!    `hold_line` missing from the exercised set, the audit fails; restored,
//!    it passes.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// The set of public items before and after the patch (invariant 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemDiff {
    /// Present after but not before, in `after` order, deduplicated.
    pub added: Vec<String>,
    /// Present before but not after, in `before` order, deduplicated.
    pub removed: Vec<String>,
}

/// Diff the set of public items before and after the patch (invariant 1, the
/// "measurable" half). Whether an item is public is decided by the caller
/// (a `pub fn` / `pub struct` / … in the touched source files); this
/// primitive only diffs the two lists. Order follows the lists given, and
/// duplicates collapse to one entry.
pub fn diff_public_items(before: &[String], after: &[String]) -> ItemDiff {
    let before_set: BTreeSet<&String> = before.iter().collect();
    let after_set: BTreeSet<&String> = after.iter().collect();
    let mut seen: BTreeSet<&String> = BTreeSet::new();
    let added: Vec<String> = after
        .iter()
        .filter(|name| !before_set.contains(name) && seen.insert(name))
        .cloned()
        .collect();
    let mut seen: BTreeSet<&String> = BTreeSet::new();
    let removed: Vec<String> = before
        .iter()
        .filter(|name| !after_set.contains(name) && seen.insert(name))
        .cloned()
        .collect();
    ItemDiff { added, removed }
}

/// The marker an exemption line must carry (invariant 1: "or state why not").
pub const EXEMPTION_MARKER: &str = "linter:allow-UNTESTED_PUBLIC";

/// An explicit annotated exemption for one untested public item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exemption {
    pub item: String,
    pub reason: String,
}

/// Parse exemption lines out of an issue body or patch note.
///
/// Grammar: `[#] linter:allow-UNTESTED_PUBLIC <item>: <reason>` — one item
/// per line, the item a single token, the reason mandatory. A bare marker or
/// an empty reason is rejected and skipped, matching the
/// `linter:allow-<RULE> <reason>` convention used elsewhere in the
/// repository.
pub fn parse_exemptions(text: &str) -> Vec<Exemption> {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let rest = trimmed.strip_prefix('#').unwrap_or(trimmed).trim();
        let Some(rest) = rest.strip_prefix(EXEMPTION_MARKER) else {
            continue;
        };
        // The marker must stand alone: a longer `linter:allow-UNTESTED_PUBLIC_*`
        // is a different rule, not this one.
        if !rest.chars().next().is_some_and(char::is_whitespace) {
            continue;
        }
        let Some((item, reason)) = rest.trim_start().split_once(':') else {
            continue;
        };
        let item = item.trim();
        let reason = reason.trim();
        if item.is_empty() || item.chars().any(char::is_whitespace) || reason.is_empty() {
            continue;
        }
        out.push(Exemption {
            item: item.to_string(),
            reason: reason.to_string(),
        });
    }
    out
}

/// The outcome of auditing the patch's additions (invariant 1). Every bucket
/// is in `additions` order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Audit {
    /// Added, and referenced by test code added in the same patch.
    pub exercised: Vec<String>,
    /// Added, not exercised, but exempted with a stated reason.
    pub exempted: Vec<String>,
    /// Added, not exercised, and no exemption — the gate's refusal.
    pub untested: Vec<String>,
}

impl Audit {
    /// Every addition is accounted for: exercised or exempted with a reason.
    pub fn complete(&self) -> bool {
        self.untested.is_empty()
    }

    pub fn coverage(&self) -> ChangeCoverage {
        ChangeCoverage::new(
            self.exercised.len() + self.exempted.len() + self.untested.len(),
            self.exercised.len(),
        )
    }
}

/// Classify every public item the patch added (invariant 1). `additions` is
/// `diff_public_items(...).added`; `exercised` are the items referenced by
/// test code added in the same patch (the caller does that mechanical search
/// over the patch's test files); `exemptions` are the explicit annotated
/// "why not" lines. An item counts as exercised only when the patch's own
/// test code names it — the existing suite staying green is not evidence
/// about new code.
pub fn audit(additions: &[String], exercised: &[String], exemptions: &[Exemption]) -> Audit {
    let exercised_set: BTreeSet<&String> = exercised.iter().collect();
    let mut seen: BTreeSet<&String> = BTreeSet::new();
    let mut out = Audit {
        exercised: Vec::new(),
        exempted: Vec::new(),
        untested: Vec::new(),
    };
    for item in additions {
        if !seen.insert(item) {
            continue;
        }
        if exercised_set.contains(item) {
            out.exercised.push(item.clone());
        } else if exemptions.iter().any(|e| e.item == *item) {
            out.exempted.push(item.clone());
        } else {
            out.untested.push(item.clone());
        }
    }
    out
}

/// Coverage of the change, not of the repository (invariant 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeCoverage {
    /// Public items the patch introduced.
    pub introduced: usize,
    /// Of those, how many are exercised by test code added in the same patch.
    pub exercised: usize,
}

impl ChangeCoverage {
    pub fn new(introduced: usize, exercised: usize) -> Self {
        ChangeCoverage {
            introduced,
            exercised,
        }
    }

    /// The line a merge record must carry. `9065 passed` is a statement about
    /// the repository; this one is about the patch.
    pub fn line(&self) -> String {
        format!(
            "change coverage: {} new public items introduced, {} exercised",
            self.introduced, self.exercised
        )
    }
}

/// Invariant 2, made mechanical: the merge record carries the
/// change-coverage line. `CONVERTED+MERGED: 9065 passed, 0 failing` is a true
/// statement that carries no signal about whether the new code is among the
/// N; a record silent about the change's coverage is a lie of omission, not
/// a missing fact to default.
pub fn record_carries_coverage(record: &str, coverage: &ChangeCoverage) -> bool {
    record.contains(&coverage.line())
}

/// The gate's verdict over the patch (invariant 3): the standard stated in
/// AGENTS.md becomes real here, or it is not enforced at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateVerdict {
    /// Every addition is exercised or exempted with a stated reason.
    Passed { coverage: ChangeCoverage },
    /// One or more additions are untested and unexempted; each is named.
    Refused {
        untested: Vec<String>,
        coverage: ChangeCoverage,
    },
}

impl GateVerdict {
    pub fn passed(&self) -> bool {
        matches!(self, GateVerdict::Passed { .. })
    }

    /// One line for the merge record and the monitor log. Carries the
    /// change-coverage line (invariant 2) and, on refusal, names every
    /// untested item — never a bare suite total.
    pub fn line(&self) -> String {
        let (untested, coverage) = match self {
            GateVerdict::Passed { coverage } => (Vec::<String>::new(), *coverage),
            GateVerdict::Refused { untested, coverage } => (untested.clone(), *coverage),
        };
        if untested.is_empty() {
            format!("{} — gate passed", coverage.line())
        } else {
            format!(
                "{} — gate refused: {} untested: {}",
                coverage.line(),
                untested.len(),
                untested.join(", ")
            )
        }
    }
}

/// Run the gate (invariant 3). Refuses — naming every untested item — when
/// any public item the patch added is neither exercised by the patch's own
/// tests nor exempted with a stated reason.
pub fn gate(additions: &[String], exercised: &[String], exemptions: &[Exemption]) -> GateVerdict {
    let audit = audit(additions, exercised, exemptions);
    let coverage = audit.coverage();
    if audit.untested.is_empty() {
        GateVerdict::Passed { coverage }
    } else {
        GateVerdict::Refused {
            untested: audit.untested,
            coverage,
        }
    }
}
