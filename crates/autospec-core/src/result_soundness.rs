//! Result soundness: the producer must be able to produce a different
//! answer (issue #4210).
//!
//! Three findings from one investigation, each a different way of trusting a
//! result that was never produced:
//!
//! 1. **A probe whose tool is absent reports "nothing found", forever.** The
//!    architecture probe (`strings <binary> | grep …` inside apptainer
//!    images) returned empty for all three images. The unanimity was the
//!    signal, and the cause was not the images: `command -v strings` was
//!    missing inside every image, and the "binary" was a 17,888-byte
//!    wrapper. A probe that cannot produce a positive cannot report one, so
//!    its negative is a property of the probe, not of the target — and its
//!    silence is indistinguishable from a genuine negative until you check.
//! 2. **Structured data edited by two programs at once gets corrupted.**
//!    Changing one field of a TSV row, a `sed` substitution (with a
//!    malformed backreference) and an `awk` field rewrite ran in the same
//!    command, producing a row with nine fields where every other row has
//!    seven — and it still *looks* like a row. Regex-editing a table
//!    produces records that pass visual inspection and fail parsing, and a
//!    second tool run over the first's output inherits its mistakes while
//!    making the result look plausible.
//! 3. **A fix verified through a path production does not use.** The
//!    multi-GPU budget fix was tested with `pick-config.py --vram-mib <sum>`;
//!    production calls it *without* `--vram-mib`, taking the branch that
//!    derives VRAM and GPU count from `detect_gpu()`. The argument passed
//!    was the one argument that disables the code under test.
//!
//! The common thread: **a result was accepted without establishing that the
//! thing producing it could have produced a different answer.** An absent
//! tool cannot report a positive; a corrupted row cannot fail visual
//! inspection; a disabled branch cannot fail its test. In each case the
//! check that would establish that costs seconds.
//!
//! Three invariants, each a checkable primitive here:
//!
//! 1. **Validate a probe against a known-positive case before believing a
//!    negative from it.** [`ProbeCheck`] records whether the probe's tool is
//!    present in the environment it runs in and the probe's result on a
//!    known-positive input (a positive control — `qwen` on an image serving
//!    Qwen right now is the free check); a negative from a
//!    [`ProbeSoundness::ToolAbsent`] or [`ProbeSoundness::Unvalidated`]
//!    probe is not evidence, and a negative on the control breaks the probe.
//! 2. **Structured data is edited by one tool that understands the
//!    structure, and validated by schema afterward.** [`RecordSchema`] is
//!    the field-count assertion against a known-good row; [`check_record`]
//!    is the one-line validation; [`EditPass::findings`] flags a pass that
//!    touched one record with more than one tool, and the record that fails
//!    the schema.
//! 3. **Verify through the production call path, not a convenient
//!    equivalent.** [`verification_verdict`] compares a verification
//!    invocation's arguments against the production call site's arguments
//!    and names the override argument that disables the branch under test
//!    ([`PathVerdict::DisablesBranch`]); where a function branches on how it
//!    is invoked, testing one branch says nothing about the other.
//!
//! Everything here is pure in-memory state — no I/O, no clock, no
//! subprocess.

/// Invariant 1: why a probe's negative cannot (or cannot yet) be evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeSoundness {
    /// The tool is present and the probe produced a positive on a
    /// known-positive input: its negatives are evidence.
    Sound,
    /// The probe's tool is absent in the environment it runs in: it cannot
    /// report anything at all, so "nothing found" is a property of the
    /// probe, not of the target.
    ToolAbsent { tool: String },
    /// The probe returned negative on a known-positive input: its method
    /// does not work here.
    Broken { control: String },
    /// No positive control has been established (and the tool has not been
    /// confirmed absent): the negative is not yet evidence.
    Unvalidated,
}

impl ProbeSoundness {
    /// Whether the probe's negatives are evidence.
    pub fn is_evidence(self) -> bool {
        matches!(self, ProbeSoundness::Sound)
    }

    /// One line for the investigation record.
    pub fn line(&self) -> String {
        match self {
            ProbeSoundness::Sound => {
                "probe is sound: validated against a known-positive input".to_string()
            }
            ProbeSoundness::ToolAbsent { tool } => format!(
                "probe cannot report anything: tool `{tool}` is absent in the environment — \
                 its negative is a property of the probe, not of the target"
            ),
            ProbeSoundness::Broken { control } => format!(
                "probe is broken: it returned negative on the known-positive input `{control}` \
                 — its negatives are not evidence"
            ),
            ProbeSoundness::Unvalidated => {
                "probe is unvalidated: no positive control has been run — its negative is not \
                 yet evidence (run the probe on a known-positive input first)"
                    .to_string()
            }
        }
    }
}

/// Invariant 1: a probe of the world and whether it is sound — the thing
/// producing the result must be able to produce a different answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeCheck {
    name: String,
    tool: String,
    /// Whether the tool is present in the environment the probe runs in:
    /// `None` until a presence check (`tool_present`) or a positive control
    /// settles it. `None` is "not yet known", never "absent": absence is a
    /// claim a check makes, not an assumption.
    tool_present: Option<bool>,
    /// The probe's results on known-positive inputs (positive controls).
    controls: Vec<(String, bool)>,
}

impl ProbeCheck {
    /// A fresh probe check. The tool's presence is unknown until either
    /// [`tool_present`](Self::tool_present) records a presence check (e.g.
    /// `command -v`) or a positive control proves the probe can run here.
    pub fn new(name: impl Into<String>, tool: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            tool: tool.into(),
            tool_present: None,
            controls: Vec::new(),
        }
    }

    /// Record the tool-presence check for the environment the probe runs
    /// in. The incident check: `command -v strings` → missing inside every
    /// image.
    pub fn tool_present(&mut self, present: bool) -> &mut Self {
        self.tool_present = Some(present);
        self
    }

    /// Record the probe's result on a known-positive input (a positive
    /// control). A positive result proves the probe — and its tool — works
    /// in this environment; a negative one breaks the probe (the first
    /// failing input is recorded and cannot be re-attributed).
    pub fn record_control(&mut self, input: &str, returned_positive: bool) -> &mut Self {
        if returned_positive {
            self.tool_present = Some(true);
        }
        self.controls.push((input.to_string(), returned_positive));
        self
    }

    /// The soundness verdict for this probe's negatives.
    pub fn soundness(&self) -> ProbeSoundness {
        if self.tool_present == Some(false) {
            return ProbeSoundness::ToolAbsent {
                tool: self.tool.clone(),
            };
        }
        if let Some((control, _)) = self.controls.iter().find(|(_, positive)| !positive) {
            return ProbeSoundness::Broken {
                control: control.clone(),
            };
        }
        if self.controls.iter().any(|(_, positive)| *positive) {
            return ProbeSoundness::Sound;
        }
        ProbeSoundness::Unvalidated
    }

    /// Whether this probe's negative result is evidence (invariant 1).
    pub fn negative_is_evidence(&self) -> bool {
        self.soundness().is_evidence()
    }

    /// One line for the investigation record, leading with the probe's name.
    pub fn line(&self) -> String {
        format!("probe `{}`: {}", self.name, self.soundness().line())
    }
}

/// Invariant 2: the schema of a structured record — its field count,
/// derived from a known-good row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordSchema {
    pub field_count: usize,
}

impl RecordSchema {
    /// Derive the schema from a known-good row: the number of whitespace-
    /// separated fields (a tab-separated row counts tabs; a field is a
    /// maximal run of non-whitespace). An empty row names no schema.
    pub fn from_row(row: &str) -> Option<Self> {
        let field_count = row.split_whitespace().count();
        (field_count > 0).then_some(Self { field_count })
    }
}

/// Invariant 2: the outcome of validating one record against the schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordCheck {
    /// The record has the schema's field count.
    Ok,
    /// The record is empty or whitespace: there is no record to check.
    Empty,
    /// The field count differs from the schema: the record passes visual
    /// inspection and fails parsing.
    FieldCount { expected: usize, found: usize },
}

impl RecordCheck {
    /// Whether the record passes the schema.
    pub fn is_ok(&self) -> bool {
        matches!(self, RecordCheck::Ok)
    }

    /// One line for the edit record.
    pub fn line(&self) -> String {
        match self {
            RecordCheck::Ok => "record passes the schema check".to_string(),
            RecordCheck::Empty => "record is empty: no record to check".to_string(),
            RecordCheck::FieldCount { expected, found } => format!(
                "record fails the schema check: {found} fields where the schema has {expected} — \
                 it looks like a row and is not one"
            ),
        }
    }
}

/// Invariant 2: the one-line validation of a record against the schema.
pub fn check_record(record: &str, schema: RecordSchema) -> RecordCheck {
    let found = record.split_whitespace().count();
    if found == 0 {
        return RecordCheck::Empty;
    }
    if found != schema.field_count {
        return RecordCheck::FieldCount {
            expected: schema.field_count,
            found,
        };
    }
    RecordCheck::Ok
}

/// Invariant 2: the kind of tool that touched a structured record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditTool {
    /// A tool that edits text, not structure (`sed`, a regex
    /// substitution): it cannot know where the fields are.
    Text,
    /// A tool that edits by structure (an `awk` field assignment, a TSV
    /// writer): it knows where the fields are.
    Structured,
}

/// Invariant 2: a finding about an edit pass and its output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditFinding {
    /// The pass used more than one tool: the second tool ran over the
    /// first's output, inheriting its mistakes while making the result
    /// look plausible.
    MultiTool { count: usize },
    /// The record fails the schema check: it passes visual inspection and
    /// fails parsing.
    SchemaViolation { expected: usize, found: usize },
}

impl EditFinding {
    /// One line for the edit record.
    pub fn line(&self) -> String {
        match self {
            EditFinding::MultiTool { count } => format!(
                "edit pass used {count} tools on one record: a second tool inherits the first's \
                 mistakes while making the result look plausible — edit structured data with one \
                 tool that understands the structure"
            ),
            EditFinding::SchemaViolation { expected, found } => format!(
                "record fails the schema check: {found} fields where the schema has {expected}"
            ),
        }
    }
}

/// Invariant 2: every tool that touched one record in one edit pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditPass {
    tools: Vec<EditTool>,
}

impl EditPass {
    /// An edit pass with the tools that touched the record, in the order
    /// they ran.
    pub fn new(tools: Vec<EditTool>) -> Self {
        Self { tools }
    }

    /// The findings about this pass and its output: the multi-tool pass
    /// (when the record was touched by more than one tool) and the schema
    /// violation (when the output record fails the field-count check).
    /// Empty when the pass is clean: one structure-aware tool and a record
    /// that passes the schema.
    pub fn findings(&self, record: &str, schema: RecordSchema) -> Vec<EditFinding> {
        let mut out = Vec::new();
        if self.tools.len() > 1 {
            out.push(EditFinding::MultiTool {
                count: self.tools.len(),
            });
        }
        if let RecordCheck::FieldCount { expected, found } = check_record(record, schema) {
            out.push(EditFinding::SchemaViolation { expected, found });
        }
        out
    }
}

/// Invariant 3: an argument that, when passed, disables a derivation.
/// Production omits the flag and takes the branch that derives the value;
/// passing the flag substitutes a supplied value for the derivation — the
/// one argument that disables the code under test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Override {
    /// The flag (e.g. `--vram-mib`).
    pub flag: String,
    /// The branch the flag disables (e.g. `detect_gpu()`).
    pub branch: String,
}

impl Override {
    pub fn new(flag: impl Into<String>, branch: impl Into<String>) -> Self {
        Self {
            flag: flag.into(),
            branch: branch.into(),
        }
    }
}

/// Invariant 3: the verdict on a verification invocation, compared against
/// the production call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathVerdict {
    /// The verification's arguments are the production call site's
    /// arguments: it exercises the production path.
    Matches,
    /// The verification passes an argument production does not, and that
    /// argument disables the branch under test: the verification cannot
    /// fail on the code it was meant to test.
    DisablesBranch { flag: String, branch: String },
    /// The verification's arguments differ from the production call
    /// site's, and no difference is a known override: it exercises a
    /// different invocation and establishes nothing about the production
    /// path. `extra` is what the verification added (in invocation order);
    /// `missing` is what it dropped.
    Divergent {
        extra: Vec<String>,
        missing: Vec<String>,
    },
}

impl PathVerdict {
    /// Whether the verification is about the production path.
    pub fn is_sound(&self) -> bool {
        matches!(self, PathVerdict::Matches)
    }

    /// One line for the verification record.
    pub fn line(&self) -> String {
        match self {
            PathVerdict::Matches => {
                "verification matches the production call path: it exercises the production \
                 branch"
                    .to_string()
            }
            PathVerdict::DisablesBranch { flag, branch } => format!(
                "verification passes `{flag}` (production does not): it disables `{branch}`, so \
                 the verification cannot fail on the code under test"
            ),
            PathVerdict::Divergent { extra, missing } => {
                let extras = extra
                    .iter()
                    .map(|a| format!("`{a}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let missing_list = missing
                    .iter()
                    .map(|a| format!("`{a}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut parts = Vec::new();
                if !extra.is_empty() {
                    parts.push(format!("extra {extras}"));
                }
                if !missing.is_empty() {
                    parts.push(format!("missing {missing_list}"));
                }
                format!(
                    "verification diverges from the production call path ({}): it exercises a \
                     different invocation and establishes nothing about the production path",
                    parts.join("; ")
                )
            }
        }
    }
}

/// Invariant 3: compare a verification invocation's arguments against the
/// production call site's arguments (the mechanical check: find the real
/// call site, reproduce its arguments exactly).
///
/// Arguments are compared as multisets — order is not the point, what each
/// invocation *contains* is. When they differ, a verification argument
/// production does not pass that is a known [`Override`] is the strongest
/// finding ([`PathVerdict::DisablesBranch`]): the verification disabled the
/// code under test. Any other difference is [`PathVerdict::Divergent`].
pub fn verification_verdict(
    production: &[String],
    verification: &[String],
    overrides: &[Override],
) -> PathVerdict {
    let mut extra = verification.to_vec();
    for arg in production {
        extra.retain(|candidate| candidate != arg);
    }
    let mut missing = production.to_vec();
    for arg in verification {
        missing.retain(|candidate| candidate != arg);
    }
    if extra.is_empty() && missing.is_empty() {
        return PathVerdict::Matches;
    }
    for arg in &extra {
        if let Some(override_) = overrides.iter().find(|o| &o.flag == arg) {
            return PathVerdict::DisablesBranch {
                flag: override_.flag.clone(),
                branch: override_.branch.clone(),
            };
        }
    }
    PathVerdict::Divergent { extra, missing }
}
