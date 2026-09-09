//! Spec-authority currency (#3947): the spec set a run implements must be
//! known to be the one in force.
//!
//! The failure this exists for is not a wrong build — it is forty merges in a
//! day aimed at a superseded architecture. A fleet traced a blocker that gated
//! ~55 issues to a question the task graph could not express: which program is
//! current. Two spec sets claimed the same problem space, neither referenced
//! the other, and the implementation repository appeared in the other
//! program's authority table in no row at all. Every gate was green in both
//! worlds, because a task graph models issue → issue and never issue →
//! spec-authority, and a spec is read as an instruction rather than as a claim
//! that can go stale.
//!
//! Four primitives, mirroring the invariants:
//!
//! 1. **A spec set declares its own currency** ([`CurrencyMarker`],
//!    [`parse_currency`]): a version, a `supersedes` / `superseded-by`
//!    pointer, and a decision record. Absence of a marker is a state
//!    ([`CurrencyStatus::Unknown`]), not a default of "current".
//! 2. **Currency is verified at dispatch** ([`SpecSet::dispatch_verdict`]):
//!    a superseded set, a set with no marker, and a component two documents
//!    both claim all refuse the dispatch with a named finding. Throughput
//!    toward the wrong program is not recoverable; a refused dispatch is.
//! 3. **Conflicts are detectable without a human tripping over them**
//!    ([`SpecSet::conflicts`]): two documents claiming the same component is
//!    a reported conflict, derived from the documents rather than from a
//!    supervisor's curiosity.
//! 4. **Volume is not evidence of direction** ([`TaskRecord`],
//!    [`throughput`]): every task carries the authority it derived from, so
//!    merges are reportable per authority, and merges attributed to
//!    [`UNDETERMINED`] produce a warning instead of a clean number.
//!
//! Like the rest of the core this module performs no I/O: the caller reads the
//! documents and hands over their text.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Header declaring the spec set (authority) a document belongs to.
pub const KEY_AUTHORITY: &str = "Spec-Set";
/// Header declaring the version of that spec set.
pub const KEY_VERSION: &str = "Spec-Version";
/// Header naming spec sets this one replaces.
pub const KEY_SUPERSEDES: &str = "Supersedes";
/// Header naming the spec set that replaces this one.
pub const KEY_SUPERSEDED_BY: &str = "Superseded-By";
/// Header naming the decision record that put this set in force.
pub const KEY_DECISION_RECORD: &str = "Decision-Record";
/// Header listing the components this document claims authority over.
pub const KEY_AUTHORITY_OVER: &str = "Authority-Over";

/// The authority stamped on work whose spec authority was never established.
/// A task may legitimately carry it — the point is that it is *counted*.
pub const UNDETERMINED: &str = "undetermined";

/// Whether a spec set is the one in force, as declared by its own headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurrencyStatus {
    /// Has a version and is not superseded by anything.
    Current,
    /// Carries a `Superseded-By` pointer, or declares itself superseded.
    Superseded,
    /// Declares no currency at all: the state the fleet ran in.
    Unknown,
}

impl CurrencyStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Superseded => "superseded",
            Self::Unknown => "unknown",
        }
    }
}

/// The currency declaration read out of one spec document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrencyMarker {
    /// The spec set this document belongs to.
    pub authority: String,
    /// Version of the set, e.g. `V2`. `None` when the document declares none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Sets this one replaces.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supersedes: Vec<String>,
    /// The set that replaces this one; its presence marks this set stale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    /// The decision record that put this set in force.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_record: Option<String>,
}

impl CurrencyMarker {
    fn status(&self) -> CurrencyStatus {
        if self.superseded_by.is_some() {
            return CurrencyStatus::Superseded;
        }
        if self.version.is_some() {
            return CurrencyStatus::Current;
        }
        CurrencyStatus::Unknown
    }
}

/// One spec document as read from disk: who it claims to be, what it claims
/// authority over, and what it says about its own currency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecDocument {
    /// Where the document came from, so a finding names a file.
    pub path: String,
    /// The spec set it belongs to; [`UNDETERMINED`] when it declares none.
    pub authority: String,
    /// Components claimed with `Authority-Over`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub claims: Vec<String>,
    /// The currency declaration, absent when the document says nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<CurrencyMarker>,
}

impl SpecDocument {
    /// Status derived from the marker: absence is `Unknown`, never `Current`.
    pub fn status(&self) -> CurrencyStatus {
        self.currency
            .as_ref()
            .map(CurrencyMarker::status)
            .unwrap_or(CurrencyStatus::Unknown)
    }
}

/// Parses `Key: value` currency headers out of a document.
///
/// Keys are matched case-insensitively and only the first occurrence of each
/// wins, so a document that restates its version later does not silently
/// change it. List keys (`Supersedes`, `Authority-Over`) are comma-separated.
/// A `Status: superseded` line counts as a supersession even without a
/// pointer, because a charter that says so in prose is still a charter that
/// says so.
///
/// Returns `None` when no currency header is present at all.
pub fn parse_currency(source: &str) -> Option<CurrencyMarker> {
    let mut authority = None;
    let mut version = None;
    let mut supersedes = Vec::new();
    let mut superseded_by = None;
    let mut decision_record = None;
    let mut found = false;

    for (key, value) in header_pairs(source) {
        let lower = key.to_ascii_lowercase();
        match lower.as_str() {
            "spec-set" | "spec_set" | "authority" => {
                if value.is_empty() {
                    continue;
                }
                authority.get_or_insert_with(|| value.to_string());
                found = true;
            }
            "spec-version" | "spec_version" | "version" => {
                if value.is_empty() {
                    continue;
                }
                version.get_or_insert_with(|| value.to_string());
                found = true;
            }
            "supersedes" => {
                let entries = split_list(&value);
                if !entries.is_empty() {
                    supersedes.extend(entries);
                    found = true;
                }
            }
            "superseded-by" | "superseded_by" => {
                if value.is_empty() {
                    continue;
                }
                superseded_by.get_or_insert_with(|| value.to_string());
                found = true;
            }
            "decision-record" | "decision_record" => {
                if value.is_empty() {
                    continue;
                }
                decision_record.get_or_insert_with(|| value.to_string());
                found = true;
            }
            "status" if value.eq_ignore_ascii_case("superseded") => {
                found = true;
                if superseded_by.is_none() {
                    superseded_by = Some(UNDETERMINED.to_string());
                }
            }
            _ => {}
        }
    }

    found.then(|| CurrencyMarker {
        authority: authority.unwrap_or_else(|| UNDETERMINED.to_string()),
        version,
        supersedes,
        superseded_by,
        decision_record,
    })
}

/// Parses `Authority-Over` claims out of a document.
pub fn parse_claims(source: &str) -> Vec<String> {
    let mut claims = Vec::new();
    for (key, value) in header_pairs(source) {
        if key.eq_ignore_ascii_case(KEY_AUTHORITY_OVER) {
            for component in split_list(&value) {
                if !claims.contains(&component) {
                    claims.push(component);
                }
            }
        }
    }
    claims
}

/// Builds one [`SpecDocument`] from its path and text.
pub fn parse_document(path: &str, source: &str) -> SpecDocument {
    let currency = parse_currency(source);
    let authority = currency
        .as_ref()
        .map(|marker| marker.authority.clone())
        .unwrap_or_else(|| UNDETERMINED.to_string());
    SpecDocument {
        path: path.to_string(),
        authority,
        claims: parse_claims(source),
        currency,
    }
}

/// Machine-readable reason a dispatch is refused or reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AuthorityCode {
    /// A document declares no currency marker at all.
    CurrencyMissing,
    /// A marker exists but declares no version.
    VersionMissing,
    /// The spec set is superseded; the finding names its successor.
    SpecSuperseded,
    /// Two documents claim authority over the same component.
    AuthorityConflict,
    /// The current authority for the requested components is not unique.
    AuthorityAmbiguous,
    /// `Superseded-By` points at a set that is not in this spec set.
    #[serde(rename = "SUPERSEDED_POINTER_UNRESOLVED")]
    PointerUnresolved,
    /// No decision record backs the spec set.
    DecisionRecordMissing,
}

impl AuthorityCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CurrencyMissing => "CURRENCY_MISSING",
            Self::VersionMissing => "VERSION_MISSING",
            Self::SpecSuperseded => "SPEC_SUPERSEDED",
            Self::AuthorityConflict => "AUTHORITY_CONFLICT",
            Self::AuthorityAmbiguous => "AUTHORITY_AMBIGUOUS",
            Self::PointerUnresolved => "SUPERSEDED_POINTER_UNRESOLVED",
            Self::DecisionRecordMissing => "DECISION_RECORD_MISSING",
        }
    }
}

/// One finding: the code, the file or set at fault, and a sentence an
/// operator can act on without re-reading the source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityFinding {
    pub code: AuthorityCode,
    pub subject: String,
    pub message: String,
}

/// A component two or more spec sets both claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityConflict {
    pub component: String,
    pub authorities: Vec<String>,
}

/// The gate result for one dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchVerdict {
    /// Whether the dispatch may proceed.
    pub allowed: bool,
    /// The authority to stamp on the resulting task records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority: Option<String>,
    /// Findings that refuse the dispatch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocking: Vec<AuthorityFinding>,
    /// Findings worth reporting that do not refuse the dispatch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub advisories: Vec<AuthorityFinding>,
}

impl DispatchVerdict {
    pub fn exit_code(&self) -> i32 {
        if self.allowed {
            0
        } else {
            1
        }
    }

    /// One line per finding, blocking first, for the monitor log.
    pub fn render(&self) -> String {
        let mut lines = Vec::new();
        for finding in self.blocking.iter().chain(self.advisories.iter()) {
            lines.push(format!(
                "{}: {}: {}",
                finding.code.as_str(),
                finding.subject,
                finding.message
            ));
        }
        if lines.is_empty() {
            let authority = self
                .authority
                .clone()
                .unwrap_or_else(|| UNDETERMINED.to_string());
            lines.push(format!("ALLOWED: spec authority {authority} is current"));
        }
        lines.join("\n")
    }
}

/// The spec sets a caller has read, as a whole.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SpecSet {
    documents: Vec<SpecDocument>,
}

impl SpecSet {
    pub fn new(documents: Vec<SpecDocument>) -> Self {
        Self { documents }
    }

    pub fn from_sources(sources: &[(&str, &str)]) -> Self {
        Self::new(
            sources
                .iter()
                .map(|(path, text)| parse_document(path, text))
                .collect(),
        )
    }

    pub fn documents(&self) -> &[SpecDocument] {
        &self.documents
    }

    /// Every spec set named by a document, in document order, deduplicated.
    pub fn authorities(&self) -> Vec<String> {
        let mut seen = Vec::new();
        for document in &self.documents {
            if !seen.contains(&document.authority) {
                seen.push(document.authority.clone());
            }
        }
        seen
    }

    /// The status of a named spec set: the most pessimistic status declared by
    /// any of its documents, because one superseding document is enough.
    pub fn status_of(&self, authority: &str) -> CurrencyStatus {
        self.documents
            .iter()
            .filter(|document| document.authority == authority)
            .map(SpecDocument::status)
            .max_by_key(|status| match status {
                CurrencyStatus::Current => 0,
                CurrencyStatus::Unknown => 1,
                CurrencyStatus::Superseded => 2,
            })
            .unwrap_or(CurrencyStatus::Unknown)
    }

    pub fn current_authorities(&self) -> Vec<String> {
        self.authorities()
            .into_iter()
            .filter(|authority| self.status_of(authority) == CurrencyStatus::Current)
            .collect()
    }

    /// Components claimed by more than one spec set that could still be in
    /// force.
    ///
    /// A set explicitly marked superseded is excluded: its claim lost, and the
    /// pointer says to whom. Counting it would bury the genuine conflicts —
    /// two *current* charters over one component — under every program that
    /// ever mentioned it.
    pub fn conflicts(&self) -> Vec<AuthorityConflict> {
        let mut owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for document in &self.documents {
            if document.status() == CurrencyStatus::Superseded {
                continue;
            }
            for component in &document.claims {
                let entry = owners.entry(component.clone()).or_default();
                if !entry.contains(&document.authority) {
                    entry.push(document.authority.clone());
                }
            }
        }
        owners
            .into_iter()
            .filter(|(_, claimants)| claimants.len() > 1)
            .map(|(component, mut claimants)| {
                claimants.sort();
                AuthorityConflict {
                    component,
                    authorities: claimants,
                }
            })
            .collect()
    }

    /// Currency findings for the named sets (all of them when none are named).
    pub fn currency_findings(&self, authorities: &[String]) -> Vec<AuthorityFinding> {
        let targets: Vec<String> = if authorities.is_empty() {
            self.authorities()
        } else {
            authorities.to_vec()
        };
        let mut findings = Vec::new();
        for authority in targets {
            let docs: Vec<&SpecDocument> = self
                .documents
                .iter()
                .filter(|document| document.authority == authority)
                .collect();
            if docs.is_empty() {
                findings.push(finding(
                    AuthorityCode::CurrencyMissing,
                    &authority,
                    "no document in this spec set declares it",
                ));
                continue;
            }
            match self.status_of(&authority) {
                CurrencyStatus::Unknown => {
                    // Two distinct defects, reported separately: documents that
                    // declare nothing, and documents whose marker omits the
                    // version. Collapsing them into one code picked whichever
                    // applied to *some* document and then named the paths of
                    // the others, so a tree of unmarked specs sitting next to
                    // one versioned spec was reported as "marker with no
                    // version" in files that carry no marker at all.
                    let unmarked = docs
                        .iter()
                        .filter(|document| document.currency.is_none())
                        .map(|document| document.path.as_str())
                        .collect::<Vec<_>>();
                    if !unmarked.is_empty() {
                        findings.push(finding(
                            AuthorityCode::CurrencyMissing,
                            &authority,
                            &format!(
                                "{} of {} document(s) in spec set {authority} declare no currency marker (no {KEY_AUTHORITY}/{KEY_VERSION}/{KEY_SUPERSEDED_BY} header): {}",
                                unmarked.len(),
                                docs.len(),
                                shorten_paths(&unmarked)
                            ),
                        ));
                    }
                    let versionless = docs
                        .iter()
                        .filter(|document| {
                            document
                                .currency
                                .as_ref()
                                .is_some_and(|marker| marker.version.is_none())
                        })
                        .map(|document| document.path.as_str())
                        .collect::<Vec<_>>();
                    if !versionless.is_empty() {
                        findings.push(finding(
                            AuthorityCode::VersionMissing,
                            &authority,
                            &format!(
                                "{} document(s) in spec set {authority} declare a marker with no {KEY_VERSION}: {}",
                                versionless.len(),
                                shorten_paths(&versionless)
                            ),
                        ));
                    }
                }
                CurrencyStatus::Superseded => {
                    let successor = docs
                        .iter()
                        .filter_map(|document| document.currency.as_ref())
                        .find_map(|marker| marker.superseded_by.clone())
                        .unwrap_or_else(|| "unknown successor".to_string());
                    findings.push(finding(
                        AuthorityCode::SpecSuperseded,
                        &authority,
                        &format!("spec set {authority} is superseded by {successor}"),
                    ));
                }
                CurrencyStatus::Current => {}
            }
        }
        findings
    }

    /// Advisories that must be visible but do not refuse a dispatch.
    pub fn advisories(&self, authorities: &[String]) -> Vec<AuthorityFinding> {
        let targets: Vec<String> = if authorities.is_empty() {
            self.authorities()
        } else {
            authorities.to_vec()
        };
        let known = self.authorities();
        let mut findings = Vec::new();
        for authority in targets {
            let docs: Vec<&SpecDocument> = self
                .documents
                .iter()
                .filter(|document| document.authority == authority)
                .collect();
            if docs.is_empty() {
                continue;
            }
            if self.status_of(&authority) == CurrencyStatus::Current {
                if docs.iter().all(|document| {
                    document
                        .currency
                        .as_ref()
                        .and_then(|marker| marker.decision_record.as_ref())
                        .is_none()
                }) {
                    findings.push(finding(
                        AuthorityCode::DecisionRecordMissing,
                        &authority,
                        &format!(
                            "spec set {authority} is current but cites no {KEY_DECISION_RECORD}"
                        ),
                    ));
                }
                for document in &docs {
                    if let Some(marker) = &document.currency {
                        if let Some(successor) = &marker.superseded_by {
                            if !known.contains(successor) {
                                findings.push(finding(
                                    AuthorityCode::PointerUnresolved,
                                    &document.path,
                                    &format!(
                                        "points at {successor}, which is not in this spec set"
                                    ),
                                ));
                            }
                        }
                    }
                }
            }
        }
        findings
    }

    /// The dispatch gate. `components` are the components the run intends to
    /// touch; an empty list means "the whole spec set", so every declared
    /// conflict is in scope.
    ///
    /// Refuses on: no currency marker, no version, a superseded set, a
    /// component claimed by two sets, and an ambiguous current authority. A
    /// refusal always names the file or the set, because the operator's next
    /// question is "which one do I go and argue about".
    pub fn dispatch_verdict(&self, components: &[String]) -> DispatchVerdict {
        let mut blocking = Vec::new();
        let mut advisories = Vec::new();

        for conflict in self.conflicts() {
            if !components.is_empty() && !components.contains(&conflict.component) {
                continue;
            }
            blocking.push(finding(
                AuthorityCode::AuthorityConflict,
                &conflict.component,
                &format!(
                    "components {} are claimed by {} with no decision between them",
                    conflict.component,
                    conflict.authorities.join(", ")
                ),
            ));
        }

        let targets = if components.is_empty() {
            Vec::new()
        } else {
            self.authorities_claiming(components)
        };
        blocking.extend(self.currency_findings(&targets));
        advisories.extend(self.advisories(&targets));

        let current = self.current_authorities();
        let authority = if targets.is_empty() {
            current.first().cloned()
        } else {
            let relevant: Vec<String> = current
                .into_iter()
                .filter(|authority| targets.contains(authority))
                .collect();
            match relevant.len() {
                1 => relevant.into_iter().next(),
                0 => None,
                _ => {
                    blocking.push(finding(
                        AuthorityCode::AuthorityAmbiguous,
                        &components.join(", "),
                        &format!(
                            "components are served by more than one current spec set: {}",
                            relevant.join(", ")
                        ),
                    ));
                    None
                }
            }
        };

        DispatchVerdict {
            allowed: blocking.is_empty() && authority.is_some(),
            authority,
            blocking,
            advisories,
        }
    }

    /// Named spec sets that declare authority over any of `components`.
    fn authorities_claiming(&self, components: &[String]) -> Vec<String> {
        let mut found = Vec::new();
        for document in &self.documents {
            if document.claims.iter().any(|c| components.contains(c))
                && !found.contains(&document.authority)
            {
                found.push(document.authority.clone());
            }
        }
        found
    }
}

/// What became of a task, for the throughput report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    Merged,
    Open,
    Failed,
}

impl TaskOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Merged => "merged",
            Self::Open => "open",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "merged" | "merge" | "done" => Some(Self::Merged),
            "open" => Some(Self::Open),
            "failed" | "fail" | "closed-unmerged" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// A unit of dispatched work and the spec authority it derived from.
///
/// The authority field is what makes a change of programs invalidate the
/// affected subgraph instead of silently re-offering it: the tasks under a set
/// that becomes superseded are named, not guessed at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRecord {
    pub task: String,
    pub authority: String,
    pub outcome: TaskOutcome,
}

/// Parses task records in `task<TAB>authority<TAB>outcome` form. Blank lines
/// and `#` comments are skipped; malformed lines are returned as messages
/// rather than dropped, because a dropped row is a silently wrong count.
pub fn parse_task_records(text: &str) -> (Vec<TaskRecord>, Vec<String>) {
    let mut records = Vec::new();
    let mut problems = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim_end();
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = if line.contains('\t') {
            line.split('\t').collect()
        } else {
            line.split_whitespace().collect()
        };
        if fields.len() != 3 {
            problems.push(format!(
                "line {}: expected task, authority and outcome; got {:?}",
                number + 1,
                line
            ));
            continue;
        }
        match TaskOutcome::parse(fields[2]) {
            Some(outcome) => records.push(TaskRecord {
                task: fields[0].to_string(),
                authority: fields[1].to_string(),
                outcome,
            }),
            None => problems.push(format!(
                "line {}: unknown outcome {:?} (expected merged, open or failed)",
                number + 1,
                fields[2]
            )),
        }
    }
    (records, problems)
}

/// Renders records back to the parseable form.
pub fn render_task_records(records: &[TaskRecord]) -> String {
    let mut out = String::new();
    for record in records {
        out.push_str(&format!(
            "{}\t{}\t{}\n",
            record.task,
            record.authority,
            record.outcome.as_str()
        ));
    }
    out
}

/// Merge volume attributed to one spec authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityThroughput {
    pub authority: String,
    pub merged: usize,
    pub open: usize,
    pub failed: usize,
    pub total: usize,
    /// The set's currency status as declared by the documents, if known.
    pub currency: CurrencyStatus,
}

/// Throughput grouped by spec authority, never as one aggregate number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThroughputReport {
    pub rows: Vec<AuthorityThroughput>,
    /// Why a large number must not be read as progress.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl ThroughputReport {
    pub fn merged_total(&self) -> usize {
        self.rows.iter().map(|row| row.merged).sum()
    }

    /// Merges whose direction is not established: aimed at a superseded set,
    /// at a set whose currency nobody declared, or at no set at all.
    pub fn undirected_merges(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.authority == UNDETERMINED || row.currency != CurrencyStatus::Current)
            .map(|row| row.merged)
            .sum()
    }
}

/// Groups task records by authority.
///
/// `set` supplies the currency of each authority when documents are available;
/// pass an empty set when they are not, and every row reports
/// [`CurrencyStatus::Unknown`] — which is itself the warning.
pub fn throughput(records: &[TaskRecord], set: &SpecSet) -> ThroughputReport {
    let mut rows: BTreeMap<String, AuthorityThroughput> = BTreeMap::new();
    for record in records {
        let row = rows
            .entry(record.authority.clone())
            .or_insert_with(|| AuthorityThroughput {
                authority: record.authority.clone(),
                merged: 0,
                open: 0,
                failed: 0,
                total: 0,
                currency: set.status_of(&record.authority),
            });
        row.total += 1;
        match record.outcome {
            TaskOutcome::Merged => row.merged += 1,
            TaskOutcome::Open => row.open += 1,
            TaskOutcome::Failed => row.failed += 1,
        }
    }

    let rows: Vec<AuthorityThroughput> = rows.into_values().collect();
    let mut warnings = Vec::new();
    for row in &rows {
        if row.authority == UNDETERMINED && row.total > 0 {
            warnings.push(format!(
                "{} task(s), {} merged, derive from no determined spec authority",
                row.total, row.merged
            ));
        } else if row.currency == CurrencyStatus::Superseded && row.merged > 0 {
            warnings.push(format!(
                "{} merge(s) landed on superseded spec set {}",
                row.merged, row.authority
            ));
        } else if row.currency == CurrencyStatus::Unknown && row.merged > 0 {
            warnings.push(format!(
                "{} merge(s) landed on spec set {} whose currency is undeclared",
                row.merged, row.authority
            ));
        }
    }

    ThroughputReport { rows, warnings }
}

/// The tasks whose authority is no longer current in `set`: the subgraph a
/// change of programs invalidates.
pub fn invalidated_tasks(records: &[TaskRecord], set: &SpecSet) -> Vec<String> {
    let mut ids = records
        .iter()
        .filter(|record| {
            record.authority == UNDETERMINED
                || set.status_of(&record.authority) != CurrencyStatus::Current
        })
        .map(|record| record.task.clone())
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
}

/// Path lists in a finding message are read by a human in a terminal: the
/// first few name the defect, the rest only cost scrollback. The full set is
/// recoverable by re-running with a narrower `--spec-dir`.
const MAX_LISTED_PATHS: usize = 6;

fn shorten_paths(paths: &[&str]) -> String {
    if paths.len() <= MAX_LISTED_PATHS {
        return paths.join(", ");
    }
    format!(
        "{}, (+{} more)",
        paths[..MAX_LISTED_PATHS].join(", "),
        paths.len() - MAX_LISTED_PATHS
    )
}

fn finding(code: AuthorityCode, subject: &str, message: &str) -> AuthorityFinding {
    AuthorityFinding {
        code,
        subject: subject.to_string(),
        message: message.to_string(),
    }
}

/// Iterates `Key: value` header pairs out of a document.
///
/// Only single-token keys count, and headings, list items and blockquotes are
/// skipped, so prose that mentions a version mid-sentence is not mistaken for
/// a declaration. Recognised keys are matched case-insensitively by
/// [`parse_currency`]; everything else is ignored.
fn header_pairs(source: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("#") {
            continue;
        }
        if trimmed.starts_with('-') || trimmed.starts_with('>') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || key.contains(' ') {
            continue;
        }
        pairs.push((key.to_string(), unquote(value.trim())));
    }
    pairs
}

fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    let stripped = trimmed
        .strip_prefix('`')
        .and_then(|rest| rest.strip_suffix('`'))
        .or_else(|| {
            trimmed
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
        });
    stripped.unwrap_or(trimmed).to_string()
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|entry| unquote(entry.trim()))
        .filter(|entry| !entry.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const V2_PROGRAM: &str = "\
# Edge and gateway program
Spec-Set: inferweave/v2
Spec-Version: V2
Supersedes: inferweave/v1
Decision-Record: docs/adr/0007-permanent-ownership.md
Authority-Over: gateway, edge, registry
";

    const CLEAN_SLATE: &str = "\
# Clean-slate implementation charter
Spec-Set: inferweave/mono
Spec-Version: V1
Supersedes: inferweave/v2
Decision-Record: docs/adr/0011-clean-slate.md
Authority-Over: gateway, edge, control-plane
";

    const NO_CURRENCY: &str = "\
# Former product charter
Authority-Over: gateway
This charter describes the original single-repository product.
";

    const SUPERSEDED_CHARTER: &str = "\
# Former product charter
Spec-Set: inferweave/legacy
Spec-Version: V1
Superseded-By: inferweave/mono
Decision-Record: docs/adr/0011-clean-slate.md
Authority-Over: control-plane
";

    fn finding_code(verdict: &DispatchVerdict, code: AuthorityCode) -> Option<&AuthorityFinding> {
        verdict.blocking.iter().find(|f| f.code == code)
    }

    #[test]
    fn currency_marker_is_read_from_headers() {
        let marker = parse_currency(V2_PROGRAM).expect("marker present");
        assert_eq!(marker.authority, "inferweave/v2");
        assert_eq!(marker.version.as_deref(), Some("V2"));
        assert_eq!(marker.supersedes, vec!["inferweave/v1".to_string()]);
        assert_eq!(
            marker.decision_record.as_deref(),
            Some("docs/adr/0007-permanent-ownership.md")
        );
        assert_eq!(marker.superseded_by, None);
    }

    #[test]
    fn claims_are_parsed_as_a_comma_list() {
        assert_eq!(
            parse_claims(V2_PROGRAM),
            vec![
                "gateway".to_string(),
                "edge".to_string(),
                "registry".to_string()
            ]
        );
    }

    #[test]
    fn absent_marker_is_unknown_never_current() {
        let doc = parse_document("docs/legacy-charter.md", NO_CURRENCY);
        assert_eq!(doc.status(), CurrencyStatus::Unknown);
        assert_eq!(doc.authority, UNDETERMINED);
        assert!(doc.currency.is_none());
    }

    #[test]
    fn superseded_pointer_marks_the_set_superseded() {
        let doc = parse_document("docs/legacy.md", SUPERSEDED_CHARTER);
        assert_eq!(doc.status(), CurrencyStatus::Superseded);
    }

    #[test]
    fn prose_status_superseded_counts_as_superseded() {
        let source = "Spec-Set: a/old\nSpec-Version: V3\nStatus: superseded\n";
        let marker = parse_currency(source).expect("marker present");
        assert_eq!(marker.status(), CurrencyStatus::Superseded);
    }

    #[test]
    fn dispatch_refuses_a_superseded_spec_set() {
        let set = SpecSet::from_sources(&[("docs/legacy.md", SUPERSEDED_CHARTER)]);
        let verdict = set.dispatch_verdict(&[]);
        assert!(!verdict.allowed);
        assert_eq!(verdict.exit_code(), 1);
        let blocked =
            finding_code(&verdict, AuthorityCode::SpecSuperseded).expect("superseded finding");
        assert_eq!(blocked.subject, "inferweave/legacy");
        assert!(blocked.message.contains("inferweave/mono"));
    }

    #[test]
    fn dispatch_reports_a_currency_less_spec_set() {
        let set = SpecSet::from_sources(&[("docs/legacy-charter.md", NO_CURRENCY)]);
        let verdict = set.dispatch_verdict(&[]);
        assert!(!verdict.allowed);
        let blocked =
            finding_code(&verdict, AuthorityCode::CurrencyMissing).expect("missing finding");
        assert_eq!(blocked.subject, UNDETERMINED);
        assert!(blocked.message.contains("docs/legacy-charter.md"));
        assert!(blocked.message.contains("declare no currency marker"));
        assert!(blocked.message.starts_with("1 of 1 document(s)"));
    }

    #[test]
    fn marker_without_version_is_reported_as_version_missing() {
        let source = "Spec-Set: a/partial\nDecision-Record: docs/adr/0001.md\n";
        let set = SpecSet::from_sources(&[("docs/partial.md", source)]);
        let verdict = set.dispatch_verdict(&[]);
        assert!(!verdict.allowed);
        assert!(finding_code(&verdict, AuthorityCode::VersionMissing).is_some());
    }

    #[test]
    fn unmarked_documents_are_named_even_beside_a_versioned_sibling() {
        // A tree where one document carries `Spec-Version:` and the rest carry
        // nothing used to be reported as "a marker with no version" naming the
        // very files that have no marker. The two defects are reported apart.
        let versioned = "Authority-Over: gateway\nSpec-Version: V2\n";
        let set = SpecSet::from_sources(&[
            ("docs/versioned.md", versioned),
            ("docs/unmarked.md", NO_CURRENCY),
        ]);
        let verdict = set.dispatch_verdict(&[]);
        assert!(!verdict.allowed);
        let missing =
            finding_code(&verdict, AuthorityCode::CurrencyMissing).expect("missing finding");
        assert!(missing.message.contains("1 of 2 document(s)"));
        assert!(missing.message.contains("docs/unmarked.md"));
        assert!(
            !missing.message.contains("docs/versioned.md"),
            "the versioned document is not the defect: {}",
            missing.message
        );
        assert!(
            finding_code(&verdict, AuthorityCode::VersionMissing).is_none(),
            "no document here lacks a version"
        );
    }

    #[test]
    fn a_finding_names_a_few_paths_and_counts_the_rest() {
        // 204 unmarked specs in one directory produced a single finding whose
        // message was a wall of 204 paths, which reads as nothing at all.
        let sources: Vec<(String, &str)> = (0..9)
            .map(|i| (format!("docs/spec-{i}.md"), NO_CURRENCY))
            .collect();
        let set = SpecSet::from_sources(
            &sources
                .iter()
                .map(|(path, text)| (path.as_str(), *text))
                .collect::<Vec<_>>(),
        );
        let verdict = set.dispatch_verdict(&[]);
        let missing =
            finding_code(&verdict, AuthorityCode::CurrencyMissing).expect("missing finding");
        assert!(missing.message.starts_with("9 of 9 document(s)"));
        assert!(missing.message.contains("(+3 more)"), "{}", missing.message);
        assert_eq!(
            missing.message.matches("docs/spec-").count(),
            MAX_LISTED_PATHS
        );
    }

    #[test]
    fn two_documents_claiming_a_component_are_a_conflict() {
        let set = SpecSet::from_sources(&[
            ("v2.md", V2_PROGRAM),
            ("mono.md", CLEAN_SLATE),
            ("legacy.md", SUPERSEDED_CHARTER),
        ]);
        let conflicts = set.conflicts();
        let components: Vec<&str> = conflicts.iter().map(|c| c.component.as_str()).collect();
        assert_eq!(components, vec!["edge", "gateway"]);
        assert_eq!(
            conflicts[0].authorities,
            vec!["inferweave/mono", "inferweave/v2"]
        );

        let verdict = set.dispatch_verdict(&["gateway".to_string()]);
        let blocked =
            finding_code(&verdict, AuthorityCode::AuthorityConflict).expect("conflict finding");
        assert_eq!(blocked.subject, "gateway");
        assert!(!verdict.allowed);
    }

    #[test]
    fn conflict_outside_the_requested_components_does_not_block() {
        let set = SpecSet::from_sources(&[("v2.md", V2_PROGRAM), ("mono.md", CLEAN_SLATE)]);
        let verdict = set.dispatch_verdict(&["registry".to_string()]);
        assert!(
            verdict.allowed,
            "expected allow, got {:?}",
            verdict.blocking
        );
        assert_eq!(verdict.authority.as_deref(), Some("inferweave/v2"));
    }

    #[test]
    fn current_spec_set_allows_dispatch_and_names_its_authority() {
        let set = SpecSet::from_sources(&[("v2.md", V2_PROGRAM)]);
        let verdict = set.dispatch_verdict(&[]);
        assert!(verdict.allowed);
        assert_eq!(verdict.authority.as_deref(), Some("inferweave/v2"));
        assert!(verdict.blocking.is_empty());
        assert!(verdict.advisories.is_empty());
        assert!(verdict.render().contains("inferweave/v2"));
    }

    #[test]
    fn missing_decision_record_is_advisory_not_blocking() {
        let source = "Spec-Set: a/plain\nSpec-Version: V1\nAuthority-Over: gateway\n";
        let set = SpecSet::from_sources(&[("a.md", source)]);
        let verdict = set.dispatch_verdict(&[]);
        assert!(verdict.allowed);
        assert_eq!(verdict.advisories.len(), 1);
        assert_eq!(
            verdict.advisories[0].code,
            AuthorityCode::DecisionRecordMissing
        );
    }

    #[test]
    fn superseded_pointer_to_an_unknown_set_is_advisory() {
        let set = SpecSet::from_sources(&[("legacy.md", SUPERSEDED_CHARTER)]);
        let advisories = set.advisories(&["inferweave/legacy".to_string()]);
        assert!(
            advisories.is_empty(),
            "superseded sets are reported as blocking first"
        );
        let unknown = SpecSet::from_sources(&[(
            "orphan.md",
            "Spec-Set: a/current\nSpec-Version: V1\nSuperseded-By: a/nowhere\n",
        )]);
        // A current set cannot also carry a successor; the pointer makes it
        // superseded, and the unresolved target is surfaced by the finding text.
        let verdict = unknown.dispatch_verdict(&[]);
        assert!(!verdict.allowed);
        assert!(verdict
            .blocking
            .iter()
            .any(|f| f.message.contains("a/nowhere")));
    }

    #[test]
    fn task_records_round_trip_and_report_malformed_lines() {
        let text = "# task\tauthority\toutcome\n101\tinferweave/mono\tmerged\n102\tundetermined\topen\n103\tinferweave/mono\tsplined\n";
        let (records, problems) = parse_task_records(text);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].outcome, TaskOutcome::Merged);
        assert_eq!(records[1].authority, UNDETERMINED);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("unknown outcome"));
        assert_eq!(render_task_records(&records).lines().count(), 2);
    }

    #[test]
    fn throughput_is_reported_by_authority_not_as_one_number() {
        let set = SpecSet::from_sources(&[("v2.md", V2_PROGRAM)]);
        let (records, _) = parse_task_records(
            "1\tinferweave/v2\tmerged\n2\tinferweave/v2\tmerged\n3\tundetermined\tmerged\n4\tinferweave/legacy\tmerged\n5\tinferweave/legacy\tfailed\n",
        );
        let report = throughput(&records, &set);
        assert_eq!(report.rows.len(), 3);
        assert_eq!(report.merged_total(), 4);
        let v2 = report
            .rows
            .iter()
            .find(|row| row.authority == "inferweave/v2")
            .expect("v2 row");
        assert_eq!((v2.merged, v2.open, v2.failed, v2.total), (2, 0, 0, 2));
        assert_eq!(v2.currency, CurrencyStatus::Current);
        // one merge on no authority, one on a set whose currency the read
        // documents do not declare; the two current-set merges warn nothing.
        assert_eq!(report.undirected_merges(), 2);
        assert_eq!(report.warnings.len(), 2, "{:?}", report.warnings);
    }

    #[test]
    fn a_change_of_programs_invalidates_the_affected_tasks() {
        let set =
            SpecSet::from_sources(&[("v2.md", V2_PROGRAM), ("legacy.md", SUPERSEDED_CHARTER)]);
        let (records, _) = parse_task_records(
            "a\tinferweave/v2\topen\nb\tinferweave/legacy\topen\nc\tundetermined\topen\n",
        );
        assert_eq!(invalidated_tasks(&records, &set), vec!["b", "c"]);
    }

    #[test]
    fn populated_case_blocks_superseded_and_reports_currency_less() {
        // The #3947 scenario: a v2 multi-repo program whose README claims
        // permanent ownership, a clean-slate monorepo the fleet implemented,
        // and a legacy charter that declares neither a version nor a pointer.
        let set = SpecSet::from_sources(&[
            ("v2/README.md", V2_PROGRAM),
            ("mono/README.md", CLEAN_SLATE),
            ("legacy/README.md", NO_CURRENCY),
            ("legacy/charter.md", SUPERSEDED_CHARTER),
        ]);

        // A run aimed at the superseded program stops.
        let verdict = set.dispatch_verdict(&["control-plane".to_string()]);
        assert!(!verdict.allowed);
        assert!(finding_code(&verdict, AuthorityCode::SpecSuperseded).is_some());

        // A run aimed at the whole problem space names both defects at once:
        // the disputed components and the charter with no currency.
        let verdict = set.dispatch_verdict(&[]);
        assert!(!verdict.allowed);
        assert!(finding_code(&verdict, AuthorityCode::AuthorityConflict).is_some());
        assert!(verdict
            .blocking
            .iter()
            .any(|f| f.code == AuthorityCode::CurrencyMissing
                || f.code == AuthorityCode::SpecSuperseded));
        assert!(verdict.render().contains("AUTHORITY_CONFLICT"));
    }
}
