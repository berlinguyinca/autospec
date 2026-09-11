//! One definition of the run-status vocabulary, and verdicts derived from the
//! fields written beside them instead of next to them (issue #4206).
//!
//! ## The defect this module exists to prevent
//!
//! A verdict guard on the cluster decided an agent run's fate by matching three
//! strings — `BUILD-FAILED`, `UNKNOWN-NO-BASELINE`, `UNKNOWN-NO-FMT-BASELINE`.
//! The runner emits none of them: its build failure is spelled `BUILD-FAIL`. The
//! guard's `BUILD-FAILED` arm was dead code that read as coverage, so every build
//! failure fell through to the default branch. Meanwhile 183 of 183 runs were
//! labelled `VERIFIED` while carrying `test_rc=101` in the same record: the label
//! was written *alongside* the fields that contradict it rather than *derived from*
//! them, and a label that never varies across the population conveys nothing about
//! any individual run.
//!
//! Four invariants follow, each with a primitive here:
//!
//! 1. **A match list must be asserted against the vocabulary it matches.** The
//!    vocabulary lives in [`VOCABULARY_PATH`] and is the only place a status name
//!    is defined; [`audit_match_list`] reports the names a consumer matches that
//!    nothing emits, and the statuses nothing matches.
//! 2. **A verdict is derived from fields, not written next to them.**
//!    [`RunEvidence::derive`] computes the status from the exit codes;
//!    [`check_verdict`] compares a recorded label against that derivation and calls
//!    a contradiction what it is.
//! 3. **A constant label conveys nothing.** [`label_distribution`] counts distinct
//!    labels and [`LabelDistribution::is_degenerate`] fires when a population that
//!    should separate runs separates none of them.
//! 4. **Shared vocabulary has a shared definition.** Consumers name statuses through
//!    [`emitted`] / [`canonical_status`] instead of restating literals; the
//!    `tests/run_status.rs` source scan fails on a status literal in `crates/**/src`
//!    that this file does not declare.
//!
//! ## Relationship to its neighbours
//!
//! - [`crate::execution::status_triage`] is the vocabulary's main in-repo consumer:
//!   it canonicalises the recorded name with [`canonical_status`] and matches
//!   exhaustively over [`Status`], so a status added here without a decision stops
//!   compiling.
//! - [`crate::dispatch_guard::FAILED_RUN_STATUSES`] is a consumer whose list is
//!   asserted against this vocabulary by `tests/run_status.rs`.
//! - [`crate::autonomous::verdict_validity`] asks whether a recorded verdict is
//!   still *fresh* (has the baseline moved since it was earned). This module asks
//!   the prior question: is it supported by the fields recorded beside it.
//! - [`crate::conversion_gate`] keeps its own verdict enum for the gate's internal
//!   taxonomy; its names reach this vocabulary through the alias rows.
//!
//! Everything here is pure: no I/O, no filesystem, no child processes. The TSV is
//! compiled in with [`include_str!`], so a built binary cannot disagree with the
//! file it was built from.

use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Repo-relative path of the authoritative vocabulary file.
pub const VOCABULARY_PATH: &str = "config/run-status-vocabulary.tsv";

/// The vocabulary itself, compiled in so consumer lists cannot drift from it.
pub const VOCABULARY_TSV: &str = include_str!("../../../config/run-status-vocabulary.tsv");

/// A status in the vocabulary, after alias resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Status {
    /// build, test and fmt stages all returned 0.
    Verified,
    /// tests ran and at least one failed against the baseline.
    NewTestFailures,
    /// the test stage hit its wall clock; no test verdict exists.
    TestTimeout,
    /// the run produced no artifact at all.
    NoOutput,
    /// the run hit its wall clock before the gate.
    Timeout,
    /// `cargo fmt --check` reported unformatted files.
    FmtDirty,
    /// `cargo build` did not return 0.
    BuildFail,
    /// timed out and produced no artifact either.
    TimeoutNoOutput,
    /// gate-only: no baseline existed to attribute failures against.
    UnknownNoBaseline,
    /// gate-only: test targets did not run, the database was unreachable.
    NoTestDb,
}

impl Status {
    /// The vocabulary name of this status — always an `emitted` name, never an alias.
    /// `const` so a consumer's match list can be built from the enum rather than
    /// restating the wire strings (#4206).
    pub const fn as_str(self) -> &'static str {
        match self {
            Status::Verified => "VERIFIED",
            Status::NewTestFailures => "NEW-TEST-FAILURES",
            Status::TestTimeout => "TEST-TIMEOUT",
            Status::NoOutput => "NO-OUTPUT",
            Status::Timeout => "TIMEOUT",
            Status::FmtDirty => "FMT-DIRTY",
            Status::BuildFail => "BUILD-FAIL",
            Status::TimeoutNoOutput => "TIMEOUT-NO-OUTPUT",
            Status::UnknownNoBaseline => "UNKNOWN-NO-BASELINE",
            Status::NoTestDb => "NO-TEST-DB",
        }
    }
}

/// How a name in the vocabulary is produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// the runner writes this name into the artifact,
    Emitted,
    /// another component writes this name and it means [`Entry::canonical`],
    Alias,
    /// the conversion gate writes this name about its own verdict; the runner
    /// never emits it, so a consumer that only ever sees runner output never
    /// matches it.
    Gate,
}

/// One row of [`VOCABULARY_PATH`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// the name as written,
    pub name: &'static str,
    /// who writes it,
    pub kind: EntryKind,
    /// the status the name means. An `emitted` or `gate` name is its own canonical;
    /// `None` only for an alias whose canonical column could not be resolved, which
    /// parsing rejects.
    pub canonical: Option<Status>,
    /// what it means, verbatim from the file,
    pub meaning: &'static str,
}

/// Every row of the vocabulary, in file order.
fn table() -> &'static Vec<Entry> {
    static TABLE: OnceLock<Vec<Entry>> = OnceLock::new();
    TABLE.get_or_init(parse_vocabulary)
}

/// Parses [`VOCABULARY_TSV`], failing closed on a malformed row: an in-tree
/// vocabulary file that does not parse is corruption, not a runtime condition.
fn parse_vocabulary() -> Vec<Entry> {
    let mut rows = Vec::new();
    for (idx, line) in VOCABULARY_TSV.lines().enumerate() {
        let line = line.trim_end();
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() != 4 {
            panic!(
                "{VOCABULARY_PATH}:{}: expected 4 tab-separated columns, got {}: {line}",
                idx + 1,
                cols.len()
            );
        }
        let (name, kind, canonical, meaning) = (
            cols[0].trim(),
            cols[1].trim(),
            cols[2].trim(),
            cols[3].trim(),
        );
        // The header row names the columns; it is not itself a vocabulary entry.
        if name == "name" && kind == "kind" {
            continue;
        }
        let entry = match kind {
            "emitted" => {
                let status = status_named(name).unwrap_or_else(|| {
                    panic!(
                        "{VOCABULARY_PATH}:{}: emitted status {name} has no Status variant; add it or declare it as an alias",
                        idx + 1
                    )
                });
                Entry {
                    name: status.as_str(),
                    kind: EntryKind::Emitted,
                    canonical: Some(status),
                    meaning,
                }
            }
            "alias" => {
                let status = status_named(canonical).unwrap_or_else(|| {
                    panic!(
                        "{VOCABULARY_PATH}:{}: alias {name} points at {canonical}, which is not an emitted status",
                        idx + 1
                    )
                });
                Entry {
                    name,
                    kind: EntryKind::Alias,
                    canonical: Some(status),
                    meaning,
                }
            }
            "gate" => {
                let status = status_named(name).unwrap_or_else(|| {
                    panic!(
                        "{VOCABULARY_PATH}:{}: gate status {name} has no Status variant; add it or declare it as an alias",
                        idx + 1
                    )
                });
                Entry {
                    name,
                    kind: EntryKind::Gate,
                    canonical: Some(status),
                    meaning,
                }
            }
            other => panic!(
                "{VOCABULARY_PATH}:{}: kind must be emitted, alias or gate, got {other}",
                idx + 1
            ),
        };
        rows.push(entry);
    }
    if rows.is_empty() {
        panic!("{VOCABULARY_PATH}: no vocabulary rows parsed");
    }
    rows
}

/// The `Status` whose vocabulary name is `name`, if any.
fn status_named(name: &str) -> Option<Status> {
    [
        Status::Verified,
        Status::NewTestFailures,
        Status::TestTimeout,
        Status::NoOutput,
        Status::Timeout,
        Status::FmtDirty,
        Status::BuildFail,
        Status::TimeoutNoOutput,
        Status::UnknownNoBaseline,
        Status::NoTestDb,
    ]
    .into_iter()
    .find(|s| s.as_str() == name)
}

/// Every row of the vocabulary, in file order.
pub fn vocabulary() -> &'static [Entry] {
    table()
}

/// The names the runner emits — the set a consumer's match list must cover.
pub fn emitted() -> Vec<&'static str> {
    vocabulary()
        .iter()
        .filter(|e| e.kind == EntryKind::Emitted)
        .map(|e| e.name)
        .collect()
}

/// The names other components emit in place of an emitted status.
pub fn aliases() -> Vec<&'static str> {
    vocabulary()
        .iter()
        .filter(|e| e.kind == EntryKind::Alias)
        .map(|e| e.name)
        .collect()
}

/// The names only the conversion gate writes about its own verdict.
pub fn gate_statuses() -> Vec<&'static str> {
    vocabulary()
        .iter()
        .filter(|e| e.kind == EntryKind::Gate)
        .map(|e| e.name)
        .collect()
}

/// The row declaring `name`, if the vocabulary declares it.
pub fn entry(name: &str) -> Option<&'static Entry> {
    vocabulary().iter().find(|e| e.name == name)
}

/// Whether `name` is declared at all — emitted, alias or gate.
pub fn is_declared(name: &str) -> bool {
    entry(name).is_some()
}

/// Resolves a recorded name to its canonical status. Aliases resolve; a name the
/// vocabulary does not declare resolves to `None`, and the caller must refuse
/// rather than guess — see [`unknown_status_refusal`].
pub fn canonical_status(raw: &str) -> Option<Status> {
    entry(raw.trim()).and_then(|e| e.canonical)
}

/// Where a name came from, as a short phrase for a diagnostic.
fn origin_phrase(entry: &Entry) -> String {
    match entry.kind {
        EntryKind::Emitted => "emitted status".to_string(),
        EntryKind::Alias => match entry.canonical {
            Some(status) => format!("alias of {}", status.as_str()),
            None => "alias".to_string(),
        },
        EntryKind::Gate => "gate status, never emitted by the runner".to_string(),
    }
}

/// The refusal a consumer emits for a name the vocabulary does not declare.
///
/// The refusal names the file that would authorise the name, so the operator reads
/// the answer instead of guessing a default: a component that cannot determine a
/// safety-relevant parameter must refuse and name what would tell it.
pub fn unknown_status_refusal(consumer: &str, raw: &str) -> String {
    format!(
        "{consumer}: status '{raw}' is not declared in {VOCABULARY_PATH}; refusing to guess an action for a name the vocabulary does not contain"
    )
}

/// What a consumer's match list covers and misses (invariant 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchListAudit {
    /// the consumer that was audited, as named by the caller,
    pub consumer: String,
    /// names it matches that the runner never emits, each annotated with what it
    /// actually is,
    pub dead: Vec<String>,
    /// names it matches that the vocabulary does not declare at all,
    pub undeclared: Vec<String>,
    /// emitted statuses no entry of the list covers.
    pub uncovered: Vec<&'static str>,
}

impl MatchListAudit {
    /// Whether the list matches the vocabulary exactly.
    pub fn ok(&self) -> bool {
        self.dead.is_empty() && self.undeclared.is_empty() && self.uncovered.is_empty()
    }

    /// One-line rendering, for a log or a gate message.
    pub fn line(&self) -> String {
        if self.ok() {
            return format!(
                "vocab-audit {}: match list matches {VOCABULARY_PATH}",
                self.consumer
            );
        }
        let mut parts = Vec::new();
        if !self.dead.is_empty() {
            parts.push(format!(
                "{} matched name(s) the runner never emits: {}",
                self.dead.len(),
                self.dead.join(", ")
            ));
        }
        if !self.undeclared.is_empty() {
            parts.push(format!(
                "{} name(s) not declared in {VOCABULARY_PATH}: {}",
                self.undeclared.len(),
                self.undeclared.join(", ")
            ));
        }
        if !self.uncovered.is_empty() {
            parts.push(format!(
                "{} emitted status(es) unmatched: {}",
                self.uncovered.len(),
                self.uncovered.join(", ")
            ));
        }
        format!("vocab-audit {}: {}", self.consumer, parts.join("; "))
    }
}

/// Audits a consumer that compares recorded names verbatim — a shell `case` or a
/// chain of string comparisons — against the vocabulary.
///
/// `matched` is every status name the consumer compares against. A list that
/// matches an alias covers the alias's spelling, not the status the runner writes,
/// so an alias entry is reported as dead: matching `BUILD-FAILED` leaves
/// `BUILD-FAIL` uncovered, which is exactly how the #4206 guard behaved. A consumer
/// that wants an alias to count as coverage must resolve through
/// [`canonical_status`] first and audit itself with
/// [`audit_canonicalising_match_list`].
pub fn audit_match_list(consumer: &str, matched: &[&str]) -> MatchListAudit {
    audit_list(consumer, matched, false)
}

/// Audits a consumer that resolves every recorded name through
/// [`canonical_status`] before matching, so an alias entry covers its canonical
/// status. A name that resolves to an emitted status is not dead here; a gate-only
/// name still is, because no runner wrote it.
pub fn audit_canonicalising_match_list(consumer: &str, matched: &[&str]) -> MatchListAudit {
    audit_list(consumer, matched, true)
}

fn audit_list(consumer: &str, matched: &[&str], canonicalises: bool) -> MatchListAudit {
    let mut dead = Vec::new();
    let mut undeclared = Vec::new();
    let mut covered: Vec<&'static str> = Vec::new();
    for name in matched {
        match entry(name) {
            None => undeclared.push((*name).to_string()),
            Some(e) => {
                // What the entry actually covers, given how the consumer matches.
                let covers = if canonicalises {
                    e.canonical.filter(|s| emitted().contains(&s.as_str()))
                } else if e.kind == EntryKind::Emitted {
                    e.canonical
                } else {
                    None
                };
                if let Some(status) = covers {
                    covered.push(status.as_str());
                } else {
                    dead.push(format!("{name} ({})", origin_phrase(e)));
                }
            }
        }
    }
    let uncovered = emitted()
        .into_iter()
        .filter(|name| !covered.contains(name))
        .collect();
    MatchListAudit {
        consumer: consumer.to_string(),
        dead,
        undeclared,
        uncovered,
    }
}

/// The exit-code evidence a run records beside its verdict.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunEvidence {
    /// `cargo build` exit code, `None` when the stage did not record one.
    pub build_rc: Option<i32>,
    /// test stage exit code, `None` when the stage did not record one.
    pub test_rc: Option<i32>,
    /// `cargo fmt --check` exit code, `None` when the stage did not record one.
    pub fmt_rc: Option<i32>,
}

/// What the evidence supports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Derivation {
    /// the exit codes determine this status.
    Status(Status),
    /// the exit codes do not determine a status; the string names what is missing.
    Insufficient(&'static str),
}

impl RunEvidence {
    /// Evidence with all three stage exit codes recorded.
    pub fn new(build_rc: i32, test_rc: i32, fmt_rc: i32) -> Self {
        RunEvidence {
            build_rc: Some(build_rc),
            test_rc: Some(test_rc),
            fmt_rc: Some(fmt_rc),
        }
    }

    /// The stages whose exit code is absent, in check order.
    pub fn missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.fmt_rc.is_none() {
            missing.push("fmt_rc");
        }
        if self.build_rc.is_none() {
            missing.push("build_rc");
        }
        if self.test_rc.is_none() {
            missing.push("test_rc");
        }
        missing
    }

    /// The evidence rendered as it appears in a status record, `?` for a stage that
    /// recorded nothing.
    pub fn evidence_line(&self) -> String {
        let show = |v: Option<i32>| match v {
            Some(rc) => rc.to_string(),
            None => "?".to_string(),
        };
        format!(
            "build_rc={} test_rc={} fmt_rc={}",
            show(self.build_rc),
            show(self.test_rc),
            show(self.fmt_rc)
        )
    }

    /// Derives the status from the exit codes. Precedence is the most decisive
    /// evidence first — formatting, then build, then tests — matching the triage
    /// precedence in [`crate::execution::status_triage`].
    ///
    /// Green is only derivable when all three stages recorded zero. A record that
    /// omitted a stage says nothing about it, so it cannot support `VERIFIED`.
    /// The timeout statuses are not derivable from exit codes at all: a timed-out
    /// run records no code, which is the fact being asserted.
    pub fn derive(&self) -> Derivation {
        if matches!(self.fmt_rc, Some(rc) if rc != 0) {
            return Derivation::Status(Status::FmtDirty);
        }
        if matches!(self.build_rc, Some(rc) if rc != 0) {
            return Derivation::Status(Status::BuildFail);
        }
        if matches!(self.test_rc, Some(rc) if rc != 0) {
            return Derivation::Status(Status::NewTestFailures);
        }
        if self.fmt_rc == Some(0) && self.build_rc == Some(0) && self.test_rc == Some(0) {
            return Derivation::Status(Status::Verified);
        }
        Derivation::Insufficient("green needs build_rc, test_rc and fmt_rc all recorded and zero")
    }
}

/// A recorded verdict compared against the evidence recorded with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerdictCheck {
    /// no verdict was recorded; the caller derives one and reports it.
    NoVerdict,
    /// the recorded label is what the evidence gives.
    Consistent {
        /// the label, and the status it resolves to.
        recorded: Status,
    },
    /// the evidence determines a status and the record says something else.
    Contradiction {
        /// the label that was written,
        recorded: Status,
        /// the status the exit codes give,
        derived: Status,
        /// the fields that contradict the label.
        evidence: String,
    },
    /// a green label over evidence that does not establish green.
    Undecidable {
        /// the green label that was written,
        recorded: Status,
        /// the stages whose exit code is absent.
        missing: Vec<&'static str>,
    },
    /// the label is one the exit codes cannot confirm or refute, such as a timeout.
    Unverifiable {
        /// the label, and why the exit codes say nothing about it.
        recorded: Status,
        /// the reason.
        reason: &'static str,
    },
    /// the label is not in the vocabulary, so it resolves to nothing.
    Undeclared {
        /// the raw label as recorded.
        recorded: String,
    },
}

impl VerdictCheck {
    /// Whether this outcome means the record's verdict cannot be trusted.
    ///
    /// An undeclared label is a defect of the same order as a contradiction: in one
    /// case the verdict disagrees with the fields beside it, in the other it names
    /// nothing at all (`BUILD-FAILDED`, `UNKNOWN-NO-FMT-BASELINE`). `Unverifiable`
    /// and `NoVerdict` are not defects — they are the honest absence of evidence,
    /// which is a gap to record, not a wrong answer.
    pub fn is_defect(&self) -> bool {
        matches!(
            self,
            VerdictCheck::Contradiction { .. }
                | VerdictCheck::Undecidable { .. }
                | VerdictCheck::Undeclared { .. }
        )
    }

    /// One-line rendering, verdict first.
    pub fn line(&self) -> String {
        match self {
            VerdictCheck::NoVerdict => {
                "no verdict recorded; derive one from the fields".to_string()
            }
            VerdictCheck::Consistent { recorded } => {
                format!("verdict {} matches the evidence", recorded.as_str())
            }
            VerdictCheck::Contradiction {
                recorded,
                derived,
                evidence,
            } => format!(
                "verdict {} contradicted by its own fields: {} implies {}",
                recorded.as_str(),
                evidence,
                derived.as_str()
            ),
            VerdictCheck::Undecidable { recorded, missing } => format!(
                "verdict {} not supported by the record: green is not derivable with {} unrecorded",
                recorded.as_str(),
                missing.join(", ")
            ),
            VerdictCheck::Unverifiable { recorded, reason } => {
                format!(
                    "verdict {} not checkable from exit codes: {reason}",
                    recorded.as_str()
                )
            }
            VerdictCheck::Undeclared { recorded } => unknown_status_refusal("verdict", recorded),
        }
    }
}

/// Compares a recorded status against the exit codes recorded beside it
/// (invariant 2). A verdict is a claim about the fields; this is the check.
pub fn check_verdict(recorded: Option<&str>, evidence: &RunEvidence) -> VerdictCheck {
    let raw = match recorded {
        None => return VerdictCheck::NoVerdict,
        Some(r) => r.trim(),
    };
    let Some(recorded_status) = canonical_status(raw) else {
        return VerdictCheck::Undeclared {
            recorded: raw.to_string(),
        };
    };
    match evidence.derive() {
        Derivation::Status(derived) => {
            if derived == recorded_status {
                VerdictCheck::Consistent {
                    recorded: recorded_status,
                }
            } else {
                VerdictCheck::Contradiction {
                    recorded: recorded_status,
                    derived,
                    evidence: evidence.evidence_line(),
                }
            }
        }
        Derivation::Insufficient(_) => {
            if recorded_status == Status::Verified {
                VerdictCheck::Undecidable {
                    recorded: recorded_status,
                    missing: evidence.missing(),
                }
            } else {
                VerdictCheck::Unverifiable {
                    recorded: recorded_status,
                    reason:
                        "a negative verdict needs the failing stage's exit code to be checkable",
                }
            }
        }
    }
}

/// How a population of verdict labels is distributed (invariant 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelDistribution {
    /// labels counted, with their counts.
    pub counts: BTreeMap<String, usize>,
    /// total labels seen, including those the vocabulary does not declare.
    pub total: usize,
}

/// Counts a population of recorded status labels. Names are counted as recorded;
/// an alias counts separately from its canonical status, because a population that
/// spells the same status two ways is itself a finding.
pub fn label_distribution<'a>(labels: impl IntoIterator<Item = &'a str>) -> LabelDistribution {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0usize;
    for label in labels {
        *counts.entry(label.trim().to_string()).or_insert(0) += 1;
        total += 1;
    }
    LabelDistribution { counts, total }
}

impl LabelDistribution {
    /// Number of distinct labels observed.
    pub fn distinct(&self) -> usize {
        self.counts.len()
    }

    /// The dominant label and its count, if any.
    pub fn top(&self) -> Option<(&str, usize)> {
        self.counts
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(k, v)| (k.as_str(), *v))
    }

    pub fn count_of(&self, label: &str) -> usize {
        self.counts.get(label).copied().unwrap_or(0)
    }

    /// Whether the label separates nothing: a population large enough to vary that
    /// shows exactly one value. One record is not degenerate, it is one record.
    pub fn is_degenerate(&self) -> bool {
        self.total >= 2 && self.distinct() == 1
    }

    /// One-line rendering.
    pub fn line(&self) -> String {
        match self.top() {
            None => "label distribution: empty population".to_string(),
            Some((label, n)) if self.is_degenerate() => format!(
                "label distribution: {label} {n}/{} (100%) — the label is constant across the population: it separates nothing and is not evidence",
                self.total
            ),
            Some((label, n)) => format!(
                "label distribution: {} distinct label(s) over {} record(s), most common {label} {n}/{}",
                self.distinct(),
                self.total,
                self.total
            ),
        }
    }
}

/// One run's record: what it claimed and what it measured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRecord {
    /// the status the run wrote, verbatim.
    pub status: Option<String>,
    /// the exit codes it wrote beside it.
    pub evidence: RunEvidence,
}

impl RunRecord {
    /// A record with a status and all three exit codes.
    pub fn new(status: &str, build_rc: i32, test_rc: i32, fmt_rc: i32) -> Self {
        RunRecord {
            status: Some(status.to_string()),
            evidence: RunEvidence::new(build_rc, test_rc, fmt_rc),
        }
    }
}

/// The verdict audit of a whole population: per-record checks plus the shape of
/// the labels together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerdictAudit {
    /// records audited.
    pub total: usize,
    /// records whose verdict the evidence supports.
    pub consistent: usize,
    /// per-record outcomes for the records that are contradicted, unprovable-green
    /// or undeclared.
    pub defects: Vec<VerdictCheck>,
    /// the label shape of the population.
    pub distribution: LabelDistribution,
}

impl VerdictAudit {
    /// Whether the population's verdicts can be used as evidence at all: no record
    /// contradicted, and the label actually varies.
    pub fn trustworthy(&self) -> bool {
        self.defects.is_empty() && !self.distribution.is_degenerate()
    }

    /// One-line rendering, for the monitor log that reads as normal operation.
    pub fn line(&self) -> String {
        let mut parts = vec![format!(
            "verdict audit: {}/{} record(s) supported by their fields",
            self.consistent, self.total
        )];
        if !self.defects.is_empty() {
            parts.push(format!("{} record(s) contradicted", self.defects.len()));
        }
        if self.distribution.is_degenerate() {
            parts.push("label is constant".to_string());
        }
        format!(
            "{}: {}",
            if self.trustworthy() { "ok" } else { "WARN" },
            parts.join(", ")
        )
    }
}

/// Audits a population of run records: every verdict against its own fields, plus
/// the distribution of labels across the population.
///
/// This is the check the cluster needed: 183 runs labelled `VERIFIED`, every one of
/// them carrying a failing `test_rc` in the same record. Either half alone reads as
/// plausible — one contradicted record looks like a bad run, a constant label looks
/// like a healthy pipeline. Together they say the verdict is not evidence.
pub fn audit_population(records: &[RunRecord]) -> VerdictAudit {
    let mut defects = Vec::new();
    let mut consistent = 0usize;
    let mut labels: Vec<&str> = Vec::with_capacity(records.len());
    for record in records {
        if let Some(status) = record.status.as_deref() {
            labels.push(status);
        }
        match check_verdict(record.status.as_deref(), &record.evidence) {
            VerdictCheck::Consistent { .. } => consistent += 1,
            check @ (VerdictCheck::Contradiction { .. }
            | VerdictCheck::Undecidable { .. }
            | VerdictCheck::Undeclared { .. }) => defects.push(check),
            _ => {}
        }
    }
    VerdictAudit {
        total: records.len(),
        consistent,
        defects,
        distribution: label_distribution(labels),
    }
}
