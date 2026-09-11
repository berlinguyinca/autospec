//! Spec-outcome measurement (issue #4339).
//!
//! Twelve issues filed in one session, all `auto-implement`, all dispatched to
//! the same fleet and the same models, all merged. Six carried an explicit
//! `## Invariants` heading; six did not. Every issue carrying the heading was
//! filed **after** every issue without it — the heading became a habit partway
//! through the session. The result: 6/6 of the structured issues added a test
//! file, 4/6 of the unstructured ones did.
//!
//! That comparison is perfectly confounded with filing order, and anything
//! else that improved over those hours (phrasing, examples, which failures had
//! just been seen) is an equally good explanation. The honest reading is a
//! correlation worth testing, not a finding to act on. Reporting it as
//! "structured issues get better implementations" is exactly the overclaim this
//! repository keeps recording — a confident conclusion from a measurement that
//! cannot support it.
//!
//! Four invariants, each a primitive here:
//!
//! 1. **A claim about what makes a good spec is measured against delivered
//!    patches, not asserted.** `OutcomeRate` always carries its denominator and
//!    `MeasuredClaim::is_measured` refuses a claim with an empty group — a rate
//!    with no data is a refusal, not a zero.
//! 2. **The confound is reported with the correlation, in the same breath.**
//!    `confound` classifies how the grouping aligns with filing order, and
//!    `MeasuredClaim` cannot be rendered without its `confound` verdict —
//!    `line()` always prints both. There is no API that returns the
//!    correlation alone.
//! 3. **Where a controlled comparison is cheap, run it before adopting the
//!    practice.** `MatchedPair` and `ControlledComparison` model the matched
//!    design (the same defect written as prose and as invariants, dispatched
//!    in randomized order); `adoption_verdict` authorizes adoption only from a
//!    controlled comparison, never from the confounded observational claim.
//! 4. **Spec-outcome results are recorded where the next spec writer will see
//!    them.** `recording_verdict` marks a result beside the failure catalogue
//!    as discoverable and a result scattered across issue comments as
//!    scattered — the next spec writer will not see it.

use serde::{Deserialize, Serialize};

/// The outcome dimensions the matched design compares on. The two binary
/// dimensions that fit the `OutcomeRate` shape; the third (invariants
/// satisfied, counted against the issue's own list) is carried in the record
/// data and reported separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutcomeDim {
    /// The merged patch added a test file.
    TestFileAdded,
    /// The patch did not need a hold.
    NoHold,
}

impl OutcomeDim {
    pub fn as_str(self) -> &'static str {
        match self {
            OutcomeDim::TestFileAdded => "test file added",
            OutcomeDim::NoHold => "no hold",
        }
    }
}

/// One delivered patch, recorded with the structural property under study and
/// the outcome that actually shipped. This is the data the fleet already
/// produces — invariant 1 reads the claim from here, never from taste.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecRecord {
    pub id: String,
    /// Filing order within the session; lower means filed earlier. This is
    /// the confounding variable.
    pub filing_order: u32,
    /// Whether the issue carried an explicit `## Invariants` heading.
    pub structured: bool,
    pub test_file_added: bool,
    /// Invariants satisfied, counted against the issue's own list.
    pub invariants_satisfied: u32,
    /// The issue's own list length (the denominator for invariants satisfied).
    pub invariant_total: u32,
    pub needed_hold: bool,
}

impl SpecRecord {
    /// Whether this delivered patch shows the named binary outcome.
    pub fn shows(&self, dim: OutcomeDim) -> bool {
        match dim {
            OutcomeDim::TestFileAdded => self.test_file_added,
            OutcomeDim::NoHold => !self.needed_hold,
        }
    }
}

/// A measured rate. The denominator is never dropped: a rate with `total == 0`
/// is "no data" (a refusal), not "0%" (invariant 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeRate {
    /// Records that show the outcome.
    pub observed: u32,
    /// Records in the group.
    pub total: u32,
}

impl OutcomeRate {
    pub fn new(observed: u32, total: u32) -> Self {
        OutcomeRate { observed, total }
    }

    /// Invariant 1: a claim with no delivered patches in the group is not
    /// measured. This is the state an asserted claim lives in.
    pub fn has_data(&self) -> bool {
        self.total > 0
    }

    pub fn ratio(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.observed as f64 / self.total as f64
        }
    }

    /// `6/6` — the observed count over the denominator, never a bare
    /// percentage that hides the denominator.
    pub fn line(&self) -> String {
        format!("{}/{}", self.observed, self.total)
    }
}

/// How the grouping aligns with filing order (invariant 2). Computed over the
/// cross-group pairs of filing order, so it is a property of the data's design
/// and independent of the outcome values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfoundVerdict {
    /// Every structured record was filed after (or before) every unstructured
    /// one — the grouping is entirely explained by filing order. The incident.
    Total,
    /// The grouping partly aligns with filing order — some, not all.
    Partial,
    /// The grouping is balanced across filing order — ordering carries no
    /// information about group membership.
    None,
    /// A group is empty — fail-closed, the confound cannot be assessed.
    Insufficient,
}

impl ConfoundVerdict {
    pub fn line(self) -> &'static str {
        match self {
            ConfoundVerdict::Total => "total (grouping aligns with filing order)",
            ConfoundVerdict::Partial => "partial (grouping partly aligns with filing order)",
            ConfoundVerdict::None => "none (grouping is balanced across filing order)",
            ConfoundVerdict::Insufficient => "insufficient (a group is empty)",
        }
    }

    pub fn is_total(self) -> bool {
        matches!(self, ConfoundVerdict::Total)
    }
}

/// Classify the confound between the structured/unstructured grouping and
/// filing order (invariant 2). For every cross-group pair, count whether the
/// structured record was filed later or earlier than the unstructured one.
/// All-later (or all-earlier) is `Total`; a perfect balance is `None`; a mix
/// is `Partial`; an empty group is `Insufficient`.
pub fn confound(records: &[SpecRecord]) -> ConfoundVerdict {
    let structured: Vec<&SpecRecord> = records.iter().filter(|r| r.structured).collect();
    let unstructured: Vec<&SpecRecord> = records.iter().filter(|r| !r.structured).collect();
    if structured.is_empty() || unstructured.is_empty() {
        return ConfoundVerdict::Insufficient;
    }
    let (mut later, mut earlier) = (0u32, 0u32);
    for s in &structured {
        for u in &unstructured {
            if s.filing_order > u.filing_order {
                later += 1;
            } else if s.filing_order < u.filing_order {
                earlier += 1;
            }
        }
    }
    // All ties: filing order is a constant across the groups and provides no
    // information about membership — not a confound, not a clean balance.
    if later + earlier == 0 {
        ConfoundVerdict::None
    } else if later == 0 || earlier == 0 {
        ConfoundVerdict::Total
    } else if later == earlier {
        ConfoundVerdict::None
    } else {
        ConfoundVerdict::Partial
    }
}

/// A claim about what makes a good spec, measured against delivered patches.
/// It carries its own `confound` so it cannot be rendered without it —
/// invariant 2, made structural.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeasuredClaim {
    pub dim: OutcomeDim,
    pub structured: OutcomeRate,
    pub unstructured: OutcomeRate,
    pub confound: ConfoundVerdict,
}

impl MeasuredClaim {
    /// Invariant 1: the claim rests on delivered patches in both groups. An
    /// asserted claim (an empty denominator) is not measured.
    pub fn is_measured(&self) -> bool {
        self.structured.has_data() && self.unstructured.has_data()
    }

    /// The measured gap in the structured group's favour.
    pub fn gap(&self) -> f64 {
        self.structured.ratio() - self.unstructured.ratio()
    }

    /// Invariant 2: the correlation and its confound, in the same breath. This
    /// is the only line a claim renders; the confound is never optional.
    pub fn line(&self) -> String {
        format!(
            "{}: structured {}, unstructured {} — confound: {}",
            self.dim.as_str(),
            self.structured.line(),
            self.unstructured.line(),
            self.confound.line()
        )
    }
}

/// The observational comparison over delivered patches. The publication API is
/// `report_line`, which always carries the confound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecComparison {
    pub records: Vec<SpecRecord>,
}

impl SpecComparison {
    pub fn new(records: Vec<SpecRecord>) -> Self {
        SpecComparison { records }
    }

    /// The rate for one group (structured or not) on one dimension.
    pub fn rate(&self, dim: OutcomeDim, structured: bool) -> OutcomeRate {
        let group: Vec<&SpecRecord> = self
            .records
            .iter()
            .filter(|r| r.structured == structured)
            .collect();
        let observed = group.iter().filter(|r| r.shows(dim)).count() as u32;
        OutcomeRate::new(observed, group.len() as u32)
    }

    /// Build the measured claim for a dimension, with its confound attached
    /// (invariant 2).
    pub fn claim(&self, dim: OutcomeDim) -> MeasuredClaim {
        MeasuredClaim {
            dim,
            structured: self.rate(dim, true),
            unstructured: self.rate(dim, false),
            confound: confound(&self.records),
        }
    }

    /// The line a result is reported as — correlation and confound in the
    /// same breath (invariant 2). This is the only rendering that should reach
    /// a record or a comment.
    pub fn report_line(&self, dim: OutcomeDim) -> String {
        self.claim(dim).line()
    }

    /// The third comparison dimension: invariants satisfied, counted against
    /// each issue's own list, per group.
    pub fn invariants_line(&self) -> String {
        let (so, st) = self.invariants_totals(true);
        let (uo, ut) = self.invariants_totals(false);
        format!(
            "invariants satisfied: structured {}/{}, unstructured {}/{} — confound: {}",
            so,
            st,
            uo,
            ut,
            confound(&self.records).line()
        )
    }

    fn invariants_totals(&self, structured: bool) -> (u32, u32) {
        let mut satisfied = 0u32;
        let mut total = 0u32;
        for r in self.records.iter().filter(|r| r.structured == structured) {
            satisfied += r.invariants_satisfied;
            total += r.invariant_total;
        }
        (satisfied, total)
    }
}

/// The two forms a matched pair is written in (invariant 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpecForm {
    /// The invariants, in narrative form.
    Prose,
    /// An explicit `## Invariants` heading with numbered checkable properties.
    Invariants,
}

impl SpecForm {
    pub fn as_str(self) -> &'static str {
        match self {
            SpecForm::Prose => "prose",
            SpecForm::Invariants => "invariants",
        }
    }
}

/// One arm of a matched pair: the same defect written in one form and
/// dispatched. `dispatched_at` is the dispatch order — the variable
/// randomization shuffles so ordering cannot explain the result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Arm {
    pub defect: String,
    pub form: SpecForm,
    pub dispatched_at: u32,
    pub test_file_added: bool,
    pub invariants_satisfied: u32,
    pub invariant_total: u32,
    pub needed_hold: bool,
}

/// A matched pair: the same defect written twice, once per form, both
/// dispatched (invariant 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchedPair {
    pub id: String,
    pub a: Arm,
    pub b: Arm,
}

impl MatchedPair {
    /// Matched means the two arms carry the same defect in the two different
    /// forms. A pair that names two defects, or writes both arms in one form,
    /// is not matched and cannot be read as a controlled comparison.
    pub fn is_matched(&self) -> bool {
        self.a.defect == self.b.defect && self.a.form != self.b.form
    }

    /// Which form was dispatched first. `None` when the pair is not matched.
    /// This is what randomization shuffles across the pairs.
    pub fn first_form(&self) -> Option<SpecForm> {
        if !self.is_matched() {
            return None;
        }
        if self.a.dispatched_at <= self.b.dispatched_at {
            Some(self.a.form)
        } else {
            Some(self.b.form)
        }
    }
}

/// The controlled comparison: the matched design actually run (invariant 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlledComparison {
    pub pairs: Vec<MatchedPair>,
}

impl ControlledComparison {
    pub fn new(pairs: Vec<MatchedPair>) -> Self {
        ControlledComparison { pairs }
    }

    /// Every pair must be a proper matched pair (same defect, both forms).
    pub fn is_matched_all(&self) -> bool {
        !self.pairs.is_empty() && self.pairs.iter().all(|p| p.is_matched())
    }

    /// Ordering must not systematically favour one form: both forms must
    /// appear as the first-dispatched arm across the pairs. If every pair
    /// dispatched invariants first, filing order could still explain the
    /// result — the same confound as the observation, wearing a different hat.
    pub fn ordering_randomized(&self) -> bool {
        let firsts: Vec<SpecForm> = self.pairs.iter().filter_map(|p| p.first_form()).collect();
        firsts.iter().any(|f| *f == SpecForm::Prose)
            && firsts.iter().any(|f| *f == SpecForm::Invariants)
    }

    /// A controlled comparison has run and its design removes the confound.
    pub fn is_controlled(&self) -> bool {
        self.is_matched_all() && self.ordering_randomized()
    }

    pub fn line(&self) -> String {
        format!(
            "matched pairs: {}, matched-all: {}, ordering-randomized: {}, controlled: {}",
            self.pairs.len(),
            self.is_matched_all(),
            self.ordering_randomized(),
            self.is_controlled()
        )
    }
}

/// The decision on whether the practice (structuring specs with an invariants
/// heading) may be adopted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdoptionVerdict {
    /// A controlled comparison has run; adoption is now authorized by
    /// measurement, not by the confounded observation.
    Authorized,
    /// Refused, with the reason naming what is missing. The observational
    /// claim is not an input here — a confounded correlation cannot authorize
    /// adoption.
    Refused { reason: &'static str },
}

impl AdoptionVerdict {
    pub fn authorized(&self) -> bool {
        matches!(self, AdoptionVerdict::Authorized)
    }

    pub fn line(&self) -> String {
        match self {
            AdoptionVerdict::Authorized => {
                "adopt: authorized by a controlled comparison".to_string()
            }
            AdoptionVerdict::Refused { reason } => {
                format!("do not adopt yet: {reason}")
            }
        }
    }
}

/// Invariant 3: adoption is authorized only by a controlled comparison. The
/// observational `MeasuredClaim` is deliberately not an argument — a 6/6 vs
/// 4/6 correlation that is totally confounded with filing order cannot be
/// passed in to authorize the practice.
pub fn adoption_verdict(controlled: &ControlledComparison) -> AdoptionVerdict {
    if controlled.pairs.is_empty() {
        AdoptionVerdict::Refused {
            reason: "no controlled comparison has been run",
        }
    } else if !controlled.is_matched_all() {
        AdoptionVerdict::Refused {
            reason: "not every pair is matched (same defect, both forms)",
        }
    } else if !controlled.ordering_randomized() {
        AdoptionVerdict::Refused {
            reason: "ordering is not randomized: filing order could still explain the result",
        }
    } else {
        AdoptionVerdict::Authorized
    }
}

/// Where a spec-outcome result is recorded (invariant 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordLocation {
    /// Beside the failure catalogue — the place the next spec writer looks.
    FailureCatalogue,
    /// In an issue comment — scattered, the next spec writer will not see it.
    IssueComment,
    /// Nowhere.
    NotRecorded,
}

impl RecordLocation {
    pub fn as_str(self) -> &'static str {
        match self {
            RecordLocation::FailureCatalogue => "failure catalogue",
            RecordLocation::IssueComment => "issue comment",
            RecordLocation::NotRecorded => "nowhere",
        }
    }
}

/// The verdict on where a result was recorded (invariant 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordingVerdict {
    /// Recorded beside the failure catalogue: the next spec writer will see it.
    Discoverable,
    /// Recorded in an issue comment: scattered, the next spec writer will not
    /// see it.
    Scattered,
    /// Not recorded: the result is lost before the next spec is written.
    Missing,
}

impl RecordingVerdict {
    pub fn is_discoverable(self) -> bool {
        matches!(self, RecordingVerdict::Discoverable)
    }

    pub fn line(self) -> String {
        match self {
            RecordingVerdict::Discoverable => {
                "recorded beside the failure catalogue: the next spec writer will see it"
                    .to_string()
            }
            RecordingVerdict::Scattered => {
                "recorded in an issue comment: scattered, the next spec writer will not see it"
                    .to_string()
            }
            RecordingVerdict::Missing => {
                "not recorded: the result is lost before the next spec is written".to_string()
            }
        }
    }
}

/// Invariant 4: a result belongs beside the failure catalogue, not scattered
/// across issue comments.
pub fn recording_verdict(location: RecordLocation) -> RecordingVerdict {
    match location {
        RecordLocation::FailureCatalogue => RecordingVerdict::Discoverable,
        RecordLocation::IssueComment => RecordingVerdict::Scattered,
        RecordLocation::NotRecorded => RecordingVerdict::Missing,
    }
}
