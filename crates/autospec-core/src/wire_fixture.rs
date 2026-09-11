//! Wire-fixture provenance guard for tests that parse external systems
//! (issue #4243).
//!
//! The test asserted that `GET /slots` returns an **object** with a
//! `slots` field, because the test built its own JSON blob and wrote it
//! that way: `{"n_slots": 3, "slots": [ ... ]}`. llama.cpp returns a
//! **bare array**: `[ { "id": 0, "n_ctx": 65536, "is_processing":
//! false }, ... ]`. The test passed forever — it was never wrong, it was
//! checking the parser against the test's own belief about the wire
//! format. The parser agreed, because both were written from the same
//! imagination, and the two wrong beliefs could never disagree. The
//! failure only appeared in production, on the first real response.
//!
//! A self-authored fixture is not evidence about an external system: it
//! is a restatement of the assumption under test. Bytes captured from the
//! running dependency are evidence, because the dependency was
//! unrepresented in the room when they were written and it disagreed.
//!
//! The four rules this module makes checkable:
//!
//! 1. **The spec names how a real sample will be obtained**
//!    ([`parse_sample_source`]). A wire-format parse site whose spec does
//!    not name the sample source is refused, naming the three accepted
//!    forms — a recorded fixture path, a contract-test base URL, or a
//!    published schema URL.
//! 2. **The sample is committed as data, not inlined**
//!    ([`fixture_is_data_file`], [`scan_wire_literals`]). The detection
//!    sweep finds test sources that build a third-party response out of a
//!    string literal while naming no recorded fixture anywhere in the
//!    file: those suites can only agree with themselves
//!    ([`sweep_authored_payloads`], [`coverage_verdict`]).
//! 3. **The fixture asserts its shape** ([`assert_fixture_shape`]). A
//!    fixture committed as a bare array guards a regression that needs a
//!    bare array. If the file stops being one, the test that "guards" it
//!    is guarding a different bug, and that is a failure, not a pass.
//! 4. **The fix is proven in both directions** ([`fix_evidence`]). Old
//!    code must fail on the captured bytes — that proves the bug was real
//!    and that the bytes reproduce it — and new code must pass on the same
//!    bytes. Running either half against authored bytes proves nothing
//!    ([`divergence_report`]).
//!
//! The rule in one line: a test that parses an external system's response
//! must assert against bytes that system actually sent, and the bytes must
//! be in the repo, not in the test author's head.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

/// Directories a recorded fixture is committed under. A "fixture" outside
/// these directories is not committed data; it is a source file that
/// happens to believe something.
pub const FIXTURE_DATA_DIRS: [&str; 4] = ["testdata", "fixtures", "test_fixtures", "test-fixtures"];

/// Extensions a recorded fixture may carry. A fixture with a source-code
/// extension is an inlined fixture wearing a data directory's clothes: the
/// bytes are still compiled into the test.
pub const FIXTURE_DATA_EXTENSIONS: [&str; 8] = [
    "json",
    "xml",
    "yaml",
    "yml",
    "txt",
    "textproto",
    "pb",
    "bin",
];

/// The wire format a string literal appears to describe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireFormat {
    /// A JSON document (`{...}` or `[...]` carrying at least one `"key":`).
    Json,
    /// An XML/HTML document (a `<tag>` with a `</tag>` or `<tag/>`).
    Xml,
    /// protobuf text format (`field: value`, two or more lines).
    ProtobufText,
}

impl WireFormat {
    /// Label used in report lines.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Json => "JSON",
            Self::Xml => "XML",
            Self::ProtobufText => "protobuf text",
        }
    }
}

/// Where the bytes a test asserts against came from. This is the whole
/// distinction the issue is about: [`FixtureProvenance::Captured`] bytes
/// carry information from outside the test, [`FixtureProvenance::Authored`]
/// bytes carry only the test's own expectation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum FixtureProvenance {
    /// Bytes recorded from the running dependency. `source` names where
    /// they came from (URL, host, revision) so a reviewer can re-take them.
    Captured { source: String },
    /// Bytes written by the test author, in a literal or in a file they
    /// typed. Never evidence about an external system.
    Authored,
}

impl FixtureProvenance {
    /// True when the bytes came from the dependency itself.
    pub fn captured(&self) -> bool {
        matches!(self, Self::Captured { .. })
    }

    /// Short phrase naming the provenance for report lines.
    pub fn label(&self) -> String {
        match self {
            Self::Captured { source } => format!("captured from {source}"),
            Self::Authored => "authored by the test".to_string(),
        }
    }
}

/// A wire-format payload built inside a test source file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct InlinePayload {
    /// Repo-relative path of the scanned file.
    pub path: String,
    /// 1-based line where the literal starts.
    pub line: usize,
    /// Format the literal was classified as.
    pub format: WireFormat,
    /// Flattened, truncated leading bytes of the literal.
    pub excerpt: String,
}

/// A test file handed to the sweep.
#[derive(Debug, Clone, Copy)]
pub struct TestFile<'a> {
    /// Repo-relative path.
    pub path: &'a str,
    /// Full text of the file.
    pub source: &'a str,
}

/// A test file whose inline payload has no recorded fixture.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SweepFinding {
    /// Repo-relative path of the test file.
    pub path: String,
    /// 1-based line of the offending literal.
    pub line: usize,
    /// Format the literal was classified as.
    pub format: WireFormat,
    /// Flattened, truncated leading bytes of the literal.
    pub excerpt: String,
}

impl SweepFinding {
    /// One-line finding, in `<path>:<line>: <message>` form.
    pub fn report(&self) -> String {
        format!(
            "{}:{}: {} payload built inside the test with no recorded fixture named in this \
             file: the suite can only agree with itself (#4243) — commit the bytes as \
             testdata data and assert against them [{}]",
            self.path,
            self.line,
            self.format.label(),
            self.excerpt
        )
    }
}

/// One assertion a test makes about an external system's response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireAssertion {
    /// The endpoint or message being asserted about, e.g. `GET /slots`.
    pub endpoint: String,
    /// Where the bytes under the assertion came from.
    pub provenance: FixtureProvenance,
}

/// How well a suite's wire-format assertions are backed by real bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Coverage {
    /// Nothing asserted about any external response. Not a defect: there is
    /// no external belief to check.
    NoAssertions,
    /// Assertions exist and **none** of them is backed by captured bytes:
    /// every one checks the parser against the test's own idea of the wire
    /// format. This is the #4243 state.
    SelfAgreeing { authored: usize },
    /// At least one assertion is backed by captured bytes.
    RecordedBacked { captured: usize, authored: usize },
}

impl Coverage {
    /// True when this coverage state must block a merge.
    ///
    /// Only [`Coverage::SelfAgreeing`] blocks. `NoAssertions` says nothing
    /// was claimed about an external format; `RecordedBacked` has at least
    /// one assertion the dependency itself authored.
    pub fn blocks(&self) -> bool {
        matches!(self, Self::SelfAgreeing { .. })
    }

    /// One-line verdict for a report or PR body.
    pub fn line(&self) -> String {
        match self {
            Self::NoAssertions => {
                "wire coverage: no assertions about an external response (nothing to check)"
                    .to_string()
            }
            Self::SelfAgreeing { authored } => format!(
                "wire coverage: 0 captured, {} authored — every assertion checks the parser \
                 against the test's own belief about the wire format (#4243)",
                authored
            ),
            Self::RecordedBacked { captured, authored } => format!(
                "wire coverage: {} captured, {} authored — at least one assertion is checked \
                 against bytes the dependency itself sent",
                captured, authored
            ),
        }
    }
}

/// Verdict on a suite's wire assertions: are any of them backed by bytes the
/// dependency itself sent?
///
/// The counting is deliberately coarse — one captured assertion is enough to
/// leave `SelfAgreeing`. The #4243 failure is not "too few captured fixtures",
/// it is "zero", and zero is the only state in which every single assertion in
/// the suite agrees with itself by construction.
pub fn coverage_verdict(assertions: &[WireAssertion]) -> Coverage {
    if assertions.is_empty() {
        return Coverage::NoAssertions;
    }
    let captured = assertions
        .iter()
        .filter(|a| a.provenance.captured())
        .count();
    let authored = assertions.len() - captured;
    if captured == 0 {
        Coverage::SelfAgreeing { authored }
    } else {
        Coverage::RecordedBacked { captured, authored }
    }
}

/// Classify a test file by path.
///
/// A file under a fixture data directory is committed data, never a test:
/// scanning it would flag every recorded fixture as an inline payload.
/// Beyond that, a test is a file named `*_test.*`, `test_*`, `*.test.*` or
/// `*.spec.*`, or any file under a `test`/`tests`/`spec` directory.
pub fn is_test_source(path: &str) -> bool {
    if has_fixture_dir_component(path) {
        return false;
    }
    let name = file_name(path);
    if name.contains("_test.") || name.contains(".test.") || name.contains(".spec.") {
        return true;
    }
    if name.starts_with("test_") {
        return true;
    }
    path.split('/')
        .any(|part| part == "test" || part == "tests" || part == "spec")
}

/// True when the file both reads as a test by path and carries Rust's
/// in-file test attribute.
///
/// A `.rs` file under `src/` may hold a `#[cfg(test)] mod tests`; the path
/// rule alone would let its inline payloads through.
pub fn is_test_file(file: &TestFile<'_>) -> bool {
    is_test_source(file.path) || file.source.contains("#[cfg(test)]")
}

/// True when the file names a recorded fixture.
///
/// This is the textual half of "a corresponding recorded fixture exists":
/// the test points at `testdata/`, `fixtures/`, `test_fixtures/` or
/// `test-fixtures/` — as a path (`testdata/slots.json`), as a quoted
/// directory (`filepath.Join("testdata", "slots.json")`), or in backticks.
/// A file that never names a fixture directory cannot be loading recorded
/// bytes, so any payload in it is the test's own.
pub fn references_recorded_fixture(source: &str) -> bool {
    FIXTURE_DATA_DIRS.iter().any(|dir| {
        source.contains(&format!("{dir}/"))
            || source.contains(&format!("\"{dir}\""))
            || source.contains(&format!("`{dir}`"))
    })
}

/// True when `path` is committed fixture data: under a fixture directory
/// **and** carrying a data extension.
///
/// Both halves matter. `internal/proxy/slots_fixture.go` is a Go file that
/// believes something; `testdata/slots.go` is the same belief parked in the
/// data directory — it still compiles into the test, so the bytes are still
/// authored at build time and still cannot disagree with the parser.
pub fn fixture_is_data_file(path: &str) -> bool {
    has_fixture_dir_component(path)
        && FIXTURE_DATA_EXTENSIONS
            .iter()
            .any(|ext| *ext == extension(path))
}

/// Find every wire-format payload a source file builds out of a string
/// literal.
///
/// Recognises Rust quoted (`"…"`, with `\` escapes) and raw (`r"…"`,
/// `r#"…"#`, `r##"…"##`) literals, Go/JS backtick raw literals and quoted
/// strings, and treats `//` line comments plus `/* … */` blocks as
/// non-code (`#` comments too for shell/Python/YAML-ish files, so a JSON
/// example in a comment is never a payload). A literal is a payload when
/// its content looks like a serialised external document: JSON (starts with
/// `{`/`[` and carries a `"key":`), XML/HTML (a `<tag>` with a closing
/// marker), or protobuf text (two or more `field: value` lines). First
/// match wins, so a JSON document is never also reported as protobuf text.
///
/// Line numbers are 1-based and point at the **start** of the literal, so a
/// multi-line raw literal is reported where the author wrote it. Unterminated
/// literals end at end of line (quoted) or end of file (raw), which keeps the
/// scanner total over truncated dumps.
pub fn scan_wire_literals(path: &str, source: &str) -> Vec<InlinePayload> {
    let chars: Vec<char> = source.chars().collect();
    let hash_comments = hash_comment_path(path);
    let mut payloads = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut code_on_line = false;

    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            code_on_line = false;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        // `//` line comment, `#` line comment (script-ish files, only where
        // the line has no code before it), `/* … */` block comment.
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if hash_comments && !code_on_line && c == '#' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                if chars[i] == '\n' {
                    line += 1;
                    code_on_line = false;
                }
                i += 1;
            }
            i = (i + 2).min(chars.len());
            continue;
        }
        code_on_line = true;

        let span = if c == 'r' {
            raw_string_bounds(&chars, i)
        } else if c == '"' {
            quoted_string_bounds(&chars, i)
        } else if c == '`' {
            backtick_bounds(&chars, i)
        } else {
            None
        };

        let Some((content_start, content_end, next)) = span else {
            i += 1;
            continue;
        };

        let content: String = chars[content_start..content_end].iter().collect();
        if let Some(format) = classify_payload(&content) {
            payloads.push(InlinePayload {
                path: path.to_string(),
                line,
                format,
                excerpt: excerpt(&content),
            });
        }
        for idx in i..next {
            if chars[idx] == '\n' {
                line += 1;
                code_on_line = false;
            }
        }
        i = next;
    }

    payloads
}

/// Find every inline wire-format payload in test files that name no
/// recorded fixture.
///
/// This is the detection sweep from the issue: a test that builds a
/// third-party response out of a JSON/XML/protobuf literal **and** never
/// names a fixture directory has no way to be wrong about the wire format,
/// which is exactly the defect. Files that read recorded bytes are skipped
/// wholesale — an authored literal next to a captured fixture is a
/// complementary case (a malformed-response test, an empty-list test), not
/// the bug.
///
/// Findings are sorted by `(path, line)` so output is deterministic
/// regardless of how the caller enumerated the files.
pub fn sweep_authored_payloads(files: &[TestFile<'_>]) -> Vec<SweepFinding> {
    let mut findings = Vec::new();
    for file in files {
        if !is_test_file(file) || references_recorded_fixture(file.source) {
            continue;
        }
        for payload in scan_wire_literals(file.path, file.source) {
            findings.push(SweepFinding {
                path: payload.path,
                line: payload.line,
                format: payload.format,
                excerpt: payload.excerpt,
            });
        }
    }
    findings.sort();
    findings
}

/// One-line summary of a sweep. Zero findings and zero files scanned are
/// stated differently: "nothing to check" must never read as "checked and
/// clean".
pub fn sweep_summary(findings: &[SweepFinding]) -> String {
    if findings.is_empty() {
        return "wire-fixture sweep: 0 findings (no authored wire payload without a recorded fixture)"
            .to_string();
    }
    let files: BTreeSet<&str> = findings.iter().map(|f| f.path.as_str()).collect();
    format!(
        "wire-fixture sweep: {} authored payload(s) across {} test file(s) with no recorded \
         fixture (#4243)",
        findings.len(),
        files.len()
    )
}

/// The JSON shape of a fixture document.
///
/// The distinction that cost a production incident is between
/// [`JsonShape::BareArray`] and [`JsonShape::Object`]: `[ {...} ]` and
/// `{"slots": [ {...} ]}` both contain slots, and only one of them decodes
/// into a parser written for the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JsonShape {
    /// `[ … ]` at the top level. An empty array is still a bare array: the
    /// shape of the document is what is asserted, not its content.
    BareArray,
    /// `{ … }` at the top level.
    Object,
    /// A string, number or `null` document.
    Scalar,
    /// No document at all (empty or whitespace-only bytes). Distinct from
    /// [`JsonShape::Malformed`]: empty bytes carry no shape, malformed bytes
    /// claim one and break on it.
    Empty,
    /// Present but not parseable JSON.
    Malformed,
}

impl JsonShape {
    /// Phrase used in report lines.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BareArray => "a bare array",
            Self::Object => "an object",
            Self::Scalar => "a scalar",
            Self::Empty => "no document",
            Self::Malformed => "bytes that do not parse as JSON",
        }
    }
}

/// The shape of `bytes` as a JSON document.
pub fn json_shape(bytes: &str) -> JsonShape {
    use serde_json::Value;

    let trimmed = bytes.trim();
    if trimmed.is_empty() {
        return JsonShape::Empty;
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Array(_)) => JsonShape::BareArray,
        Ok(Value::Object(_)) => JsonShape::Object,
        Ok(_) => JsonShape::Scalar,
        Err(_) => JsonShape::Malformed,
    }
}

/// A fixture whose bytes no longer have the shape the test asserts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapeDrift {
    /// The fixture the bytes were read from.
    pub fixture: String,
    /// Shape the test expects.
    pub expected: JsonShape,
    /// Shape the bytes actually have.
    pub found: JsonShape,
    /// Stable reason phrase, matched by tests.
    pub reason: &'static str,
}

impl ShapeDrift {
    /// Report line naming the fixture and both shapes.
    pub fn line(&self) -> String {
        format!(
            "{}: {} (found {})",
            self.fixture,
            self.reason,
            self.found.as_str()
        )
    }
}

impl fmt::Display for ShapeDrift {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.line())
    }
}

impl std::error::Error for ShapeDrift {}

/// Assert that a fixture still has the shape the regression needs.
///
/// A fixture committed because "the response is a bare array" is only
/// guarding that fact while it is one. Replace the file with an object — a
/// friendly re-recording after an upstream "cleanup", a hand-written
/// stand-in — and the test that reads it either fails confusingly or, worse,
/// gets its expectation edited to match, which is the original sin committed
/// a second time, now with a fixture file to point at. Failing on shape
/// change keeps the fixture's provenance meaningful.
///
/// An `expected` of [`JsonShape::Malformed`] is itself a defect and is
/// refused: a test that expects unparseable bytes passes on any corruption.
pub fn assert_fixture_shape(
    fixture: &str,
    expected: JsonShape,
    bytes: &str,
) -> Result<JsonShape, ShapeDrift> {
    let found = json_shape(bytes);
    if expected == JsonShape::Malformed {
        return Err(ShapeDrift {
            fixture: fixture.to_string(),
            expected,
            found,
            reason: "a fixture's expected shape cannot be malformed: the assertion would pass \
                     on any corruption",
        });
    }
    if found == expected {
        return Ok(found);
    }
    Err(ShapeDrift {
        fixture: fixture.to_string(),
        expected,
        found,
        reason: if found == JsonShape::Malformed {
            "fixture is no longer readable: it does not parse as JSON, so the regression it \
             guards cannot be reproduced"
        } else if found == JsonShape::Empty {
            "fixture is empty: there are no bytes to assert against"
        } else {
            "fixture is no longer the shape the regression needs; the regression it guards has \
             changed shape"
        },
    })
}

/// What one implementation did with a set of bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeOutcome {
    /// How many records the code extracted from the bytes.
    pub parsed: usize,
    /// The error the code reported, if any.
    pub error: Option<String>,
}

impl ProbeOutcome {
    /// A run that extracted `parsed` records without error.
    pub fn parsed(parsed: usize) -> Self {
        Self {
            parsed,
            error: None,
        }
    }

    /// A run that failed. `parsed` records were extracted before the failure
    /// (usually zero).
    pub fn failed(parsed: usize, error: &str) -> Self {
        Self {
            parsed,
            error: Some(error.to_string()),
        }
    }

    /// True when the code did not consume the bytes as the format they claim
    /// to be: either it reported an error or it extracted nothing at all.
    ///
    /// Zero records with no error counts as a failure: a parser that silently
    /// yields nothing is the more dangerous version of the bug, because a
    /// fleet with zero slots looks idle rather than broken.
    pub fn failed_to_parse(&self) -> bool {
        self.error.is_some() || self.parsed == 0
    }

    /// Short description for report lines.
    pub fn describe(&self) -> String {
        match &self.error {
            Some(err) if self.parsed > 0 => {
                format!("parsed {} then failed: {}", self.parsed, err)
            }
            Some(err) => format!("failed to parse: {}", err),
            None => format!("parsed {}", self.parsed),
        }
    }
}

/// The two-directional proof that a wire-format fix was needed and works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DivergenceProof {
    /// The endpoint the bytes came from, e.g. `GET /slots`.
    pub endpoint: String,
    /// Where the bytes came from. The proof is only a proof over captured
    /// bytes.
    pub provenance: FixtureProvenance,
    /// Pre-fix code run against those bytes.
    pub old_code: ProbeOutcome,
    /// Post-fix code run against the same bytes.
    pub new_code: ProbeOutcome,
}

/// Whether a [`DivergenceProof`] proves anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixEvidence {
    /// The proof ran over bytes the test author wrote. Neither outcome is
    /// evidence: authored bytes cannot testify about a dependency.
    RunOnAuthoredBytes,
    /// The old code handled the bytes fine, so the bytes do not reproduce the
    /// bug. "Passes after the fix" is not evidence the fix was needed.
    BugNotReproduced,
    /// The new code still fails on the captured bytes: the fix is incomplete.
    FixIncomplete,
    /// Old code failed on the captured bytes and new code passes on them: the
    /// bug was real, the bytes reproduce it, the fix resolves it.
    NeededAndFixed,
}

impl FixEvidence {
    /// True only for [`FixEvidence::NeededAndFixed`].
    pub fn is_evidence(&self) -> bool {
        matches!(self, Self::NeededAndFixed)
    }

    /// Stable reason phrase for the verdict.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::RunOnAuthoredBytes => {
                "the proof ran over bytes the test wrote, not bytes the dependency sent"
            }
            Self::BugNotReproduced => {
                "the pre-fix code already handled these bytes, so they do not reproduce the bug"
            }
            Self::FixIncomplete => "the post-fix code still fails on these bytes",
            Self::NeededAndFixed => {
                "pre-fix code fails and post-fix code passes on the same captured bytes"
            }
        }
    }
}

/// Judge a [`DivergenceProof`].
///
/// Order matters. Provenance is checked **first**: authored bytes that make
/// the old code fail look like the strongest evidence of all ("see, it broke!")
/// while proving only that the author's invention differs from the author's
/// parser. Then the old code's outcome, because a bug the bytes do not
/// reproduce cannot be fixed by anything run on them.
pub fn fix_evidence(proof: &DivergenceProof) -> FixEvidence {
    if !proof.provenance.captured() {
        return FixEvidence::RunOnAuthoredBytes;
    }
    if !proof.old_code.failed_to_parse() {
        return FixEvidence::BugNotReproduced;
    }
    if proof.new_code.failed_to_parse() {
        return FixEvidence::FixIncomplete;
    }
    FixEvidence::NeededAndFixed
}

/// Full report line for a divergence proof: the verdict, its reason, and the
/// two outcomes side by side so a reader can check the judgement rather than
/// take it.
pub fn divergence_report(proof: &DivergenceProof) -> String {
    let evidence = fix_evidence(proof);
    format!(
        "{} ({}): {} — {} | old: {} | new: {}",
        proof.endpoint,
        proof.provenance.label(),
        if evidence.is_evidence() {
            "verified both directions"
        } else {
            "NOT EVIDENCE"
        },
        evidence.reason(),
        proof.old_code.describe(),
        proof.new_code.describe(),
    )
}

/// How a spec says a real sample of an external response will be obtained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SampleSource {
    /// A recorded fixture committed under a data directory.
    RecordedFixture { path: String },
    /// A test that runs against the live dependency and records its answer.
    ContractTest { base_url: String },
    /// A schema the dependency publishes (OpenAPI, `.proto`, XSD).
    PublishedSchema { url: String },
}

impl SampleSource {
    /// Report line naming the declared source.
    pub fn line(&self) -> String {
        match self {
            Self::RecordedFixture { path } => {
                format!("sample source: recorded fixture {}", path)
            }
            Self::ContractTest { base_url } => {
                format!("sample source: contract test against {}", base_url)
            }
            Self::PublishedSchema { url } => format!("sample source: published schema {}", url),
        }
    }
}

/// Why a sample-source declaration was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SampleSourceError {
    /// No declaration at all.
    Empty,
    /// A declaration that does not name any of the accepted sources.
    Unspecified {
        /// The text that was given.
        decl: String,
    },
    /// A `fixture:` whose path is not committed data.
    FixtureNotDataFile {
        /// The path that was given.
        path: String,
    },
    /// A `contract:`/`schema:` whose value is not an `http(s)` URL.
    UrlMissing {
        /// The value that was given.
        value: String,
    },
}

impl SampleSourceError {
    /// The refusal, naming what would be accepted.
    pub fn message(&self) -> String {
        match self {
            Self::Empty => "the spec names no sample source; write one of \
                            `fixture: <testdata path>`, `contract: <http(s) base URL>`, \
                            `schema: <http(s) URL>`"
                .to_string(),
            Self::Unspecified { decl } => format!(
                "sample source {:?} names no accepted source; write one of \
                 `fixture: <testdata path>`, `contract: <http(s) base URL>`, \
                 `schema: <http(s) URL>`",
                decl
            ),
            Self::FixtureNotDataFile { path } => format!(
                "fixture {:?} is not committed data: it must live under one of {:?} and carry \
                 a data extension ({:?})",
                path, FIXTURE_DATA_DIRS, FIXTURE_DATA_EXTENSIONS
            ),
            Self::UrlMissing { value } => format!(
                "{:?} is not an http(s) URL: a live sample source must name the endpoint the \
                 bytes will be taken from",
                value
            ),
        }
    }
}

impl fmt::Display for SampleSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message())
    }
}

impl std::error::Error for SampleSourceError {}

/// Parse a spec's sample-source declaration.
///
/// Accepted forms (`keyword: value`, keyword case-insensitive):
/// `fixture:` / `recorded:` / `recorded-fixture:` with a path that passes
/// [`fixture_is_data_file`]; `contract:` / `live:` with an `http(s)` URL;
/// `schema:` / `openapi:` with an `http(s)` URL.
///
/// Anything else is refused, and the refusal names the three accepted forms
/// — a spec that cannot say where a real response will come from has not
/// decided how the parse site will ever be tested, which is the moment the
/// #4243 bug is born. Guessing a plausible sample is what the code already
/// did.
pub fn parse_sample_source(decl: &str) -> Result<SampleSource, SampleSourceError> {
    let trimmed = decl.trim();
    if trimmed.is_empty() {
        return Err(SampleSourceError::Empty);
    }
    let Some((keyword, value)) = trimmed.split_once(':') else {
        return Err(SampleSourceError::Unspecified {
            decl: trimmed.to_string(),
        });
    };
    let value = value.trim();
    match keyword.trim().to_ascii_lowercase().as_str() {
        "fixture" | "recorded" | "recorded-fixture" | "recorded_fixture" => {
            if value.is_empty() || !fixture_is_data_file(value) {
                return Err(SampleSourceError::FixtureNotDataFile {
                    path: value.to_string(),
                });
            }
            Ok(SampleSource::RecordedFixture {
                path: value.to_string(),
            })
        }
        "contract" | "live" | "contract-test" | "contract_test" => {
            if !is_http_url(value) {
                return Err(SampleSourceError::UrlMissing {
                    value: value.to_string(),
                });
            }
            Ok(SampleSource::ContractTest {
                base_url: value.to_string(),
            })
        }
        "schema" | "openapi" => {
            if !is_http_url(value) {
                return Err(SampleSourceError::UrlMissing {
                    value: value.to_string(),
                });
            }
            Ok(SampleSource::PublishedSchema {
                url: value.to_string(),
            })
        }
        _ => Err(SampleSourceError::Unspecified {
            decl: trimmed.to_string(),
        }),
    }
}

/// True for an `http://` or `https://` URL.
fn is_http_url(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// True when any path component is a fixture data directory.
fn has_fixture_dir_component(path: &str) -> bool {
    Path::new(path).components().any(|component| {
        FIXTURE_DATA_DIRS.contains(&component.as_os_str().to_string_lossy().as_ref())
    })
}

/// Lowercased file name of `path`.
fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Lowercased extension of `path` (empty when there is none).
fn extension(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// True for file types whose comments start with `#`, so a JSON example in a
/// shell or Python comment is never mistaken for a payload.
fn hash_comment_path(path: &str) -> bool {
    matches!(
        extension(path).as_str(),
        "sh" | "bash" | "zsh" | "dash" | "py" | "rb" | "yaml" | "yml" | "toml" | "tf"
    )
}

/// Classify a literal's content as an external wire payload.
fn classify_payload(content: &str) -> Option<WireFormat> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    // JSON: a document that starts like one and carries at least one
    // `"key":`. An array of bare scalars is not claimed as a payload — that
    // is a test table, not a third-party response, and flagging it would bury
    // the finding that matters.
    if (trimmed.starts_with('{') || trimmed.starts_with('[')) && trimmed.contains("\":") {
        return Some(WireFormat::Json);
    }
    if looks_like_xml(trimmed) {
        return Some(WireFormat::Xml);
    }
    if looks_like_protobuf_text(trimmed) {
        return Some(WireFormat::ProtobufText);
    }
    None
}

/// True for a document with an opening tag and a closing marker.
fn looks_like_xml(text: &str) -> bool {
    let Some(index) = text.find('<') else {
        return false;
    };
    // `<` is one byte, so `index + 1` is a char boundary.
    let after = text[index + 1..].chars().next();
    let opened = matches!(after, Some(c) if c.is_ascii_alphabetic() || c == '/' || c == '?');
    opened && (text.contains("</") || text.contains("/>"))
}

/// True for two or more protobuf text-format `field: value` lines.
fn looks_like_protobuf_text(text: &str) -> bool {
    if text.contains('<') {
        return false;
    }
    text.lines()
        .filter(|line| protobuf_field_line(line.trim()))
        .count()
        >= 2
}

/// True for a single `field: value` line (identifier, colon, non-empty value).
fn protobuf_field_line(line: &str) -> bool {
    if line.is_empty() || line.starts_with('"') {
        return false;
    }
    let Some(colon) = line.find(':') else {
        return false;
    };
    let name = &line[..colon];
    let value = line[colon + 1..].trim_start();
    !name.is_empty() && !value.is_empty() && is_protobuf_ident(name)
}

/// True for a protobuf field name: `[A-Za-z_][A-Za-z0-9_]*`.
fn is_protobuf_ident(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Bounds of a Rust raw string starting at `start`: a `content` span and the
/// index just past the literal. Unterminated raws run to end of input.
fn raw_string_bounds(chars: &[char], start: usize) -> Option<(usize, usize, usize)> {
    if chars.get(start) != Some(&'r') {
        return None;
    }
    let mut cursor = start + 1;
    let mut hashes = 0usize;
    while chars.get(cursor) == Some(&'#') {
        hashes += 1;
        cursor += 1;
    }
    if chars.get(cursor) != Some(&'"') {
        return None;
    }
    let content_start = cursor + 1;
    let mut cursor = content_start;
    loop {
        match chars.get(cursor) {
            None => return Some((content_start, chars.len(), chars.len())),
            Some('"') => {
                let mut matched = 0usize;
                while matched < hashes && chars.get(cursor + 1 + matched) == Some(&'#') {
                    matched += 1;
                }
                if matched == hashes {
                    return Some((content_start, cursor, cursor + 1 + hashes));
                }
                cursor += 1;
            }
            Some(_) => cursor += 1,
        }
    }
}

/// Bounds of a quoted string starting at `start`. `\` escapes consume the next
/// character. An unterminated quoted string ends at the newline, which is left
/// unconsumed so the line counter still sees it.
fn quoted_string_bounds(chars: &[char], start: usize) -> Option<(usize, usize, usize)> {
    if chars.get(start) != Some(&'"') {
        return None;
    }
    let content_start = start + 1;
    let mut cursor = content_start;
    while let Some(c) = chars.get(cursor) {
        match c {
            '\\' => cursor += 2,
            '"' => return Some((content_start, cursor, cursor + 1)),
            '\n' => return Some((content_start, cursor, cursor)),
            _ => cursor += 1,
        }
    }
    Some((content_start, chars.len(), chars.len()))
}

/// Bounds of a backtick raw string (Go, JS template literal).
fn backtick_bounds(chars: &[char], start: usize) -> Option<(usize, usize, usize)> {
    if chars.get(start) != Some(&'`') {
        return None;
    }
    let content_start = start + 1;
    let mut cursor = content_start;
    while let Some(c) = chars.get(cursor) {
        if *c == '`' {
            return Some((content_start, cursor, cursor + 1));
        }
        cursor += 1;
    }
    Some((content_start, chars.len(), chars.len()))
}

/// Flatten a payload's leading bytes into a single truncated line for the
/// finding text.
fn excerpt(content: &str) -> String {
    const MAX_CHARS: usize = 60;
    let flat = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= MAX_CHARS {
        return flat;
    }
    let head: String = flat.chars().take(MAX_CHARS).collect();
    format!("{head}…")
}
