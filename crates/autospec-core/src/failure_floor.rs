//! A tolerated-failure floor makes count-based attribution unsafe (issue #4301).
//!
//! Two converters, two different definitions of "this patch broke a test".
//! `convpass.sh` attributes failures by **name**: it extracts the failing
//! test ids from the run and diffs them against the set that also fails on
//! `main`; when the diff is empty it records
//! `all N failing test(s) also fail on main; not attributed to this patch`
//! and merges — #4065 converted with exactly that note. `iwconv.sh`
//! attributes by **count**: it compares the failing-test total against the
//! total on `main`. A count check answers "did we lose tests?"; it does not
//! answer "did this patch break a test?".
//!
//! Before the floor existed the two definitions agreed in practice, because
//! `failures != 0` was itself the signal: any failure in the run was new by
//! definition. Then `main` carried a **permanent floor of 2 tolerated
//! failures** (#4291), and the definitions diverged in exactly the new
//! situation: a patch that breaks two tests reconciles the totals against a
//! floor of two, and the patch passes. The only thing separating "the two
//! known failures" from "two different failures" is the names, and one of
//! the two converters never looks at them.
//!
//! The asymmetry is invisible from either file alone — both report
//! confidently, both look principled — so invariant 4 makes the *comparison
//! between the siblings* the checkable object, and invariant 3 makes the
//! shared attribution the single implementation both must use (the
//! `conflict-resolve.sh` pattern from #3741, applied to attribution).
//!
//! Invariants, each encoded as a checkable primitive:
//!
//! 1. **Failure attribution is by test identity, never by count** —
//!    [`attribute`] is the one attribution, and
//!    [`count_vs_name_divergence`] is the finding that fires the moment a
//!    count gate reconciles while name attribution shows new failures.
//! 2. **A tolerated-failure baseline and a count-based gate are mutually
//!    exclusive** — [`floor_gate_conflict`] fails any gate still comparing
//!    totals the moment the floor carries a single entry, so admitting the
//!    first known-failing test to the baseline is what kills the second.
//! 3. **Behaviour shared by two converters belongs in one place** —
//!    [`conclusion_for`] derives the recorded conclusion from the one shared
//!    [`attribute`]; a converter that re-derives the rule (by count) carries
//!    a different conclusion for the same input.
//! 4. **When one sibling is fixed, check the other in the same change** —
//!    [`sibling_drift`] compares what the two converters recorded for the
//!    same failure set; that comparison, not either file alone, is the view
//!    that can see the divergence.

use std::collections::BTreeSet;

/// How a gate decides "did this patch break a test?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateStyle {
    /// Compare the failing test names against the tolerated baseline
    /// (`convpass.sh`: extract names, diff against the `main` set).
    Names,
    /// Compare the failing test total against the `main` total (`iwconv.sh`:
    /// `ran N tests where main runs M`).
    Counts,
}

impl GateStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            GateStyle::Names => "names",
            GateStyle::Counts => "counts",
        }
    }
}

/// The tolerated-failure baseline recorded on `main` (#4291).
///
/// Membership is by test identity: the floor is a *set of names*, and that
/// is the whole reason it exists — the names are what separate "the known
/// failures" from "the same number of different failures".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FailureFloor {
    names: BTreeSet<String>,
}

impl FailureFloor {
    /// Build a floor from test names (order-insensitive, duplicates
    /// collapsed — identity, not occurrences).
    pub fn new<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            names: names.into_iter().map(Into::into).collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    /// Admit one more known-failing test to the baseline — the act invariant
    /// 2 is about: this single addition is what invalidates any gate still
    /// comparing totals.
    pub fn admit(&mut self, name: &str) {
        self.names.insert(name.to_string());
    }
}

/// Invariant 1: the attribution, by test identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attribution {
    /// Every failing name in the run is on the floor: the patch broke
    /// nothing that the baseline does not already carry.
    Tolerated { count: usize },
    /// These names failed in the run and are not on the floor: the patch
    /// broke them.
    NewFailures { names: Vec<String> },
}

impl Attribution {
    /// The patch is clean of failures.
    pub fn is_clean(&self) -> bool {
        matches!(self, Attribution::Tolerated { .. })
    }

    /// The line the conversion pass records in its log for this verdict.
    ///
    /// The `Tolerated` arm is the note #4065 merged under:
    /// `all 2 failing test(s) also fail on main; not attributed to this
    /// patch`.
    pub fn line(&self, n: &str) -> String {
        match self {
            Attribution::Tolerated { count } => format!(
                "{n} note: all {count} failing test(s) also fail on main; not attributed to \
                 this patch"
            ),
            Attribution::NewFailures { names } => format!(
                "{n} HELD: this patch broke failing test(s) not on main's tolerated-failure \
                 floor: {}",
                names.join(", ")
            ),
        }
    }
}

/// The one shared attribution (invariant 3): both converters call this.
///
/// Input is the set of test names that failed in the run; output is a
/// statement about *which of them* the patch is responsible for. A
/// count-based consumer never calls this, which is exactly the gap.
pub fn attribute(floor: &FailureFloor, failing: &[String]) -> Attribution {
    let mut new_failures: Vec<String> = Vec::new();
    let mut distinct: BTreeSet<&str> = BTreeSet::new();
    for name in failing {
        if distinct.insert(name) && !floor.contains(name) && !new_failures.contains(name) {
            new_failures.push(name.clone());
        }
    }
    if new_failures.is_empty() {
        // Count the distinct names that are on the floor, not the raw
        // occurrences: the note states how many *failing tests* the run
        // carried.
        let count = distinct.iter().filter(|name| floor.contains(*name)).count();
        Attribution::Tolerated { count }
    } else {
        new_failures.sort();
        Attribution::NewFailures {
            names: new_failures,
        }
    }
}

/// What the count-based gate reports: the totals reconcile.
///
/// This is `iwconv.sh`'s check, and it answers a different question —
/// "did we lose tests?" — from the one the gate needs to answer.
pub fn count_reconciles(floor: &FailureFloor, failing: &[String]) -> bool {
    let mut distinct: BTreeSet<&str> = BTreeSet::new();
    for name in failing {
        distinct.insert(name);
    }
    distinct.len() == floor.len()
}

/// Invariant 1 as a check: the finding when the count gate reconciles while
/// attribution by name shows new failures.
///
/// This is the moment the count check is unsafe: `main` carries a
/// tolerated-failure floor, the patch breaks that many tests with *different*
/// names, and the totals reconcile — so the count gate merges and the name
/// gate holds, for the same input.
pub fn count_vs_name_divergence(floor: &FailureFloor, failing: &[String]) -> Vec<String> {
    let attribution = attribute(floor, failing);
    if !count_reconciles(floor, failing) || attribution.is_clean() {
        return Vec::new();
    }
    let Attribution::NewFailures { names } = &attribution else {
        return Vec::new();
    };
    vec![format!(
        "COUNT_ATTRIBUTION_UNSAFE: the totals reconcile ({} == {}) but attribution by name \
         attributes {} new failure(s) to this patch ({}) — a count check is unsafe the moment \
         main carries a tolerated-failure floor, because the names are the only thing \
         separating the known failures from the same number of different ones",
        floor.len(),
        failing.len(),
        names.len(),
        names.join(", ")
    )]
}

/// Invariant 2 as a check: a tolerated-failure floor and a count-based gate
/// are mutually exclusive.
///
/// The floor empty, a count-based gate is still safe — `failures != 0` was
/// itself the signal, because any failure in the run was new by definition.
/// The moment the floor carries one entry the gate can no longer tell the
/// known failures from new ones, and admitting that first entry must fail
/// the gate: this returns a finding on `!floor.is_empty() && gate ==
/// Counts`, so the conflict is detected at the admission, not at the first
/// silent merge it caused.
pub fn floor_gate_conflict(floor: &FailureFloor, gate: GateStyle) -> Vec<String> {
    if floor.is_empty() || gate == GateStyle::Names {
        return Vec::new();
    }
    vec![format!(
        "FLOOR_AND_COUNT_GATE: main carries a tolerated-failure floor of {} known-failing \
         test(s); the gate comparing totals ({}) can no longer tell the known failures from \
         new ones and must attribute by name",
        floor.len(),
        gate.as_str()
    )]
}

/// The conclusion one converter recorded for its input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conclusion {
    /// `all N failing test(s) also fail on main; not attributed to this
    /// patch` — clean, merged.
    NotAttributed,
    /// Failures attributed to the patch — held, not merged.
    Attributed,
}

/// Invariant 3: the conclusion both converters must carry for an input.
///
/// Derived from the one shared [`attribute`], never re-derived: a
/// count-based re-derivation produces a different conclusion for the same
/// input the moment the floor is non-empty, and that difference is what
/// [`sibling_drift`] catches.
pub fn conclusion_for(floor: &FailureFloor, failing: &[String]) -> Conclusion {
    if attribute(floor, failing).is_clean() {
        Conclusion::NotAttributed
    } else {
        Conclusion::Attributed
    }
}

/// One converter's record of one attribution: who, what it was handed, and
/// what it concluded (invariants 3 and 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConverterRecord {
    /// The converter's name as it appears in its log (`convpass`, `iwconv`).
    pub converter: String,
    /// The failing test names the run produced — identity, the only input
    /// attribution is allowed to be built on (invariant 1).
    pub failing: Vec<String>,
    /// What the converter recorded for this input.
    pub conclusion: Conclusion,
}

impl ConverterRecord {
    pub fn new(converter: &str, failing: Vec<String>, conclusion: Conclusion) -> Self {
        Self {
            converter: converter.to_string(),
            failing,
            conclusion,
        }
    }
}

/// The same failure set, up to order and duplication.
fn same_failure_set(a: &[String], b: &[String]) -> bool {
    let sa: BTreeSet<&str> = a.iter().map(|s| s.as_str()).collect();
    let sb: BTreeSet<&str> = b.iter().map(|s| s.as_str()).collect();
    sa == sb
}

/// Invariant 4 (with 3): the finding when two converters reached different
/// attribution conclusions for the same failing-test set.
///
/// The asymmetry is invisible from either file alone — each looks
/// principled in isolation, and both report confidently. Only the comparison
/// between the siblings' records sees the divergence, which is why the
/// comparison, not the files, is the checkable object here.
pub fn sibling_drift(a: &ConverterRecord, b: &ConverterRecord) -> Vec<String> {
    if a.converter == b.converter || !same_failure_set(&a.failing, &b.failing) {
        return Vec::new();
    }
    if a.conclusion == b.conclusion {
        return Vec::new();
    }
    let mut ordered = [a, b].to_vec();
    ordered.sort_by(|x, y| x.converter.cmp(&y.converter));
    vec![format!(
        "SIBLING_ATTRIBUTION_DRIFT: {} and {} reached different attribution conclusions \
         ({} vs {}) for the same failing-test set — the shared rule has drifted between the \
         siblings; both must carry the conclusion the shared attribution derives \
         (conclusion_for), not a re-derived one",
        ordered[0].converter,
        ordered[1].converter,
        ordered[0].conclusion.as_str(),
        ordered[1].conclusion.as_str()
    )]
}

impl Conclusion {
    pub fn as_str(self) -> &'static str {
        match self {
            Conclusion::NotAttributed => "not-attributed",
            Conclusion::Attributed => "attributed",
        }
    }
}
