//! `autospec anchor` — protected-anchor suite commands (issue #3558): register,
//! list, show (redacted per access role) and verify suites stored write-once at
//! `.autospec/evaluation/anchors/<suite-id>/v<N>.json`. Artifacts must byte-match
//! their recorded digest: a tampered case is named by id and `verify` exits 2.
//! The suite model lives here until core's `evaluation::anchor` (#3557) lands,
//! with field names verbatim from the evaluator-coevolution design spec.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use autospec_core::evaluation::digest::Digest;
use autospec_core::evaluation::ids::{AnchorCaseId, AnchorSuiteId, EvaluatorSlot};
use autospec_core::evaluation::store::journal::Journal;
use autospec_core::evaluation::store::layout::EvaluationLayout;
use autospec_core::evaluation::store::{io, EvaluationError, EvaluationErrorKind};

use super::CommandFailure;

/// Suite documents carry the core evaluation schema version.
const SUITE_SCHEMA_VERSION: u64 = autospec_core::evaluation::EVALUATION_SCHEMA_VERSION;

/// `verify` exits 2: the command ran fine, the suite did not verify.
const VERIFY_FAILED_EXIT: i32 = 2;

/// Who is asking: operator and qualification see the stored suite; mutation (the
/// default) never sees quarantine or protected-holdout labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AccessRole {
    Operator,
    Qualification,
    Mutation,
}

impl AccessRole {
    fn parse(value: &str) -> Result<Self, EvaluationError> {
        match value {
            "operator" => Ok(Self::Operator),
            "qualification" => Ok(Self::Qualification),
            "mutation" => Ok(Self::Mutation),
            other => Err(parse_err(format!("unknown access role: {other}"))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Operator => "operator",
            Self::Qualification => "qualification",
            Self::Mutation => "mutation",
        }
    }
}

/// The verdict an evaluator must reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProtectedLabel {
    Accept,
    Reject,
}

/// Cost of getting a case wrong: `critical` feeds the critical-security subset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

/// Case visibility class (design spec §Data Model).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnchorVisibility {
    Development,
    PublicRegression,
    ProtectedHoldout,
    Quarantine,
}

/// One anchor case: a pinned artifact plus the verdict an evaluator must reach.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnchorCase {
    pub case_id: AnchorCaseId,
    pub artifact_ref: String,
    pub content_digest: Digest,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_label: Option<ProtectedLabel>,
    pub severity: Severity,
    pub visibility: AnchorVisibility,
    #[serde(default)]
    pub tags: BTreeSet<String>,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<String>,
}

/// A required tag with its false-accept/false-reject ceilings and the number of
/// cases of drift tolerated against the incumbent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProtectedSubset {
    pub name: String,
    pub tag: String,
    pub max_false_accept: Option<u64>,
    pub max_false_reject: Option<u64>,
    pub regression_tolerance_cases: u64,
}

/// Human-readable registration provenance; `created_by` is stamped when empty.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Provenance {
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// An immutable suite as stored at `anchors/<suite-id>/v<N>.json`.
/// `minimum_case_count` guards against an empty suite passing as a reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnchorSuite {
    pub schema: u64,
    pub suite_id: AnchorSuiteId,
    pub version: u32,
    pub slot: EvaluatorSlot,
    #[serde(default)]
    pub cases: Vec<AnchorCase>,
    pub minimum_case_count: usize,
    #[serde(default)]
    pub required_subsets: Vec<ProtectedSubset>,
    #[serde(default)]
    pub provenance: Provenance,
}

pub(crate) fn run(args: &[String]) -> Result<(), CommandFailure> {
    let Some((command, rest)) = args.split_first() else {
        print_help();
        return Ok(());
    };
    if matches!(command.as_str(), "--help" | "-h" | "help") {
        print_help();
        return Ok(());
    }
    let arguments = Arguments::parse(rest).map_err(failure)?;
    match command.as_str() {
        "register" => register(arguments),
        "list" => list(arguments),
        "show" => render(arguments, false),
        "verify" => render(arguments, true),
        other => Err(failure(parse_err(format!(
            "unknown anchor subcommand: {other}"
        )))),
    }
}

fn print_help() {
    println!(
        "usage: autospec anchor register --file <suite.json> [--root <path>] [--json]
       autospec anchor list [--root <path>] [--json]
       autospec anchor show|verify <suite-id>@<version> [--role <r>] [--file <p>] [--root <path>]

  register  Validate, digest every artifact (all must byte-match) and store once.
  list      One line per registered suite version (empty when none).
  show      Print JSON; the mutation view (default) drops quarantined cases.
  verify    Recompute every artifact digest, report mismatches, exit 2.

Roles are operator | qualification | mutation (default). --file overrides the
stored suite path; --root is the repo root holding .autospec/ (default .)."
    );
}

struct Arguments {
    root: PathBuf,
    file: Option<PathBuf>,
    role: AccessRole,
    target: Option<(AnchorSuiteId, u32)>,
    json: bool,
}

impl Arguments {
    fn parse(args: &[String]) -> Result<Self, EvaluationError> {
        let mut parsed = Self {
            root: PathBuf::from("."),
            file: None,
            role: AccessRole::Mutation,
            target: None,
            json: false,
        };
        let mut index = 0;
        while index < args.len() {
            let flag = args[index].as_str();
            let (value, next): (&str, usize) = match flag {
                "--root" | "--file" | "--role" => flag_value(args, index)?,
                "--json" => {
                    parsed.json = true;
                    index += 1;
                    continue;
                }
                other if parsed.target.is_none() => (other, index + 1),
                other => return Err(parse_err(format!("unexpected argument: {other}"))),
            };
            match flag {
                "--root" => parsed.root = PathBuf::from(value),
                "--file" => parsed.file = Some(PathBuf::from(value)),
                "--role" => parsed.role = AccessRole::parse(value)?,
                _ => parsed.target = Some(parse_target(value)?),
            }
            index = next;
        }
        Ok(parsed)
    }

    /// The suite a command reads or writes: `--file` overrides the store path.
    fn suite_path(&self, layout: &EvaluationLayout) -> Result<PathBuf, EvaluationError> {
        if let Some(path) = &self.file {
            return Ok(path.clone());
        }
        let target = self
            .target
            .as_ref()
            .ok_or_else(|| parse_err("missing suite target <id>@<version>"))?;
        Ok(layout.anchor_file(target.0.as_str(), target.1))
    }
}

fn flag_value(args: &[String], index: usize) -> Result<(&str, usize), EvaluationError> {
    let flag = &args[index];
    args.get(index + 1)
        .map(|value| (value.as_str(), index + 2))
        .ok_or_else(|| parse_err(format!("{flag} needs a value")))
}

/// `<suite-id>@<version>`; split at the last `@` because ids may contain one.
fn parse_target(text: &str) -> Result<(AnchorSuiteId, u32), EvaluationError> {
    let (id, raw) = text
        .rsplit_once('@')
        .ok_or_else(|| parse_err(format!("suite target must be <id>@<version>: {text}")))?;
    let bad = || parse_err(format!("bad suite version: {text}"));
    let version: u32 = raw.parse().map_err(|_| bad())?;
    if version == 0 {
        return Err(parse_err(format!("suite version must be >= 1: {text}")));
    }
    let id = AnchorSuiteId::parse(id).map_err(|error| parse_err(error.message))?;
    Ok((id, version))
}

fn register(arguments: Arguments) -> Result<(), CommandFailure> {
    let Some(source) = arguments.file.clone() else {
        return Err(failure(parse_err("register needs --file <suite.json>")));
    };
    let read = std::fs::read_to_string(&source);
    let text = read.map_err(|error| failure(io_err(&source, error)))?;
    let mut suite: AnchorSuite =
        serde_json::from_str(&text).map_err(|error| failure(parse_err(error.to_string())))?;
    if suite.schema != SUITE_SCHEMA_VERSION {
        return Err(failure(invariant(format!(
            "anchor suite schema {} is not supported (expected {SUITE_SCHEMA_VERSION})",
            suite.schema
        ))));
    }
    validate(&suite).map_err(failure)?;
    // Artifacts are checked before anything is stored: a suite registered over
    // tampered fixtures would make an immutable lie of the write-once store.
    let problems = verify_artifacts(&suite, &arguments.root);
    if !problems.is_empty() {
        return Err(broken(&suite, &problems, false));
    }
    let digest = suite_digest(&suite);
    if suite.provenance.created_by.is_empty() {
        suite.provenance.created_by = whoami();
    }
    let layout = EvaluationLayout::new(&arguments.root);
    layout.ensure_directories().map_err(failure)?;
    let stored_at = layout.anchor_file(suite.suite_id.as_str(), suite.version);
    io::write_immutable_json(&stored_at, &suite).map_err(failure)?;
    journal_record(&layout, &suite).map_err(failure)?;
    let (id, version, n) = (suite.suite_id.as_str(), suite.version, suite.cases.len());
    if arguments.json {
        print_json(&serde_json::json!({
            "schema": SUITE_SCHEMA_VERSION,
            "registered": format!("{id}@{version}"),
            "slot": suite.slot.as_str(),
            "cases": n,
            "digest": digest.to_string(),
        }))?;
    } else {
        let short = digest.short();
        println!("registered {id}@{version}: {n} cases, digest {short}");
    }
    Ok(())
}

/// Loads the suite a show/verify targets: `--file` or the stored version.
fn load(arguments: &Arguments) -> Result<AnchorSuite, CommandFailure> {
    let layout = EvaluationLayout::new(&arguments.root);
    io::read_json(&arguments.suite_path(&layout).map_err(failure)?).map_err(failure)
}

/// `show` prints the role's view as JSON; `verify` names every broken case and
/// exits 2. Both read the same target, so they cannot disagree on contents.
fn render(arguments: Arguments, verify: bool) -> Result<(), CommandFailure> {
    let stored = load(&arguments)?;
    let view = redact(&stored, arguments.role);
    // Artifacts are recomputed for every stored case — quarantined ones too, so
    // a broken case is reported rather than hidden — and verify re-checks the
    // stored suite's own invariants (never the redacted view, whose labels are
    // legitimately absent).
    let mut problems = verify_artifacts(&stored, &arguments.root);
    if verify {
        problems.extend(validate(&stored).err().map(|error| error.message));
    }
    if !problems.is_empty() {
        return Err(broken(&stored, &problems, verify));
    }
    let (id, version, n) = (stored.suite_id.as_str(), stored.version, view.cases.len());
    if !verify {
        return print_json(&serde_json::json!({
            "schema": SUITE_SCHEMA_VERSION,
            "role": arguments.role.as_str(),
            "suite": view,
            "cases_visible": n,
            "cases_total": stored.cases.len(),
        }));
    }
    if arguments.json {
        print_json(&serde_json::json!({
            "schema": SUITE_SCHEMA_VERSION,
            "suite": format!("{id}@{version}"),
            "role": arguments.role.as_str(),
            "cases_verified": n,
            "ok": true,
        }))?;
    } else {
        println!("ok: {id}@{version} - {n} cases verified");
    }
    Ok(())
}

/// A broken suite is reported the same way by both commands; only `verify`
/// exits 2, because for it a red reference is the expected outcome's opposite.
fn broken(suite: &AnchorSuite, problems: &[String], verify: bool) -> CommandFailure {
    let detail = problems.join("; ");
    let (id, version) = (suite.suite_id.as_str(), suite.version);
    let text = format!("anchor suite {id}@{version}: {detail}");
    if verify {
        return CommandFailure::status(text, VERIFY_FAILED_EXIT);
    }
    failure(integrity(text))
}

/// One line per registered suite version: `id@version slot cases digest[..16]`.
fn list(arguments: Arguments) -> Result<(), CommandFailure> {
    let layout = EvaluationLayout::new(&arguments.root);
    let mut suites: Vec<AnchorSuite> = Vec::new();
    for directory in read_dir_all(&layout.anchors_dir())? {
        // Anything that is not a directory named as a suite id is not ours.
        if !directory.path().is_dir()
            || AnchorSuiteId::parse(&directory.file_name().to_string_lossy()).is_err()
        {
            continue;
        }
        for file in read_dir_all(&directory.path())? {
            let path = file.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                suites.push(io::read_json(&path).map_err(failure)?);
            }
        }
    }
    suites.sort_by_key(|suite| (suite.suite_id.as_str().to_string(), suite.version));
    if arguments.json {
        return print_json(&serde_json::json!({
            "schema": SUITE_SCHEMA_VERSION,
            "suites": suites.iter().map(list_row).collect::<Vec<_>>(),
        }));
    }
    for suite in &suites {
        let (id, version, n) = (suite.suite_id.as_str(), suite.version, suite.cases.len());
        let digest = suite_digest(suite).short().to_string();
        println!("{id}@{version} {} {n} cases {digest}", suite.slot.as_str());
    }
    Ok(())
}

fn list_row(suite: &AnchorSuite) -> serde_json::Value {
    serde_json::json!({
        "suite_id": suite.suite_id.as_str(),
        "version": suite.version,
        "slot": suite.slot.as_str(),
        "cases": suite.cases.len(),
        "digest": suite_digest(suite).to_string(),
    })
}

/// `read_dir` where a missing directory is an empty listing: a workspace with
/// no evaluation store simply has no suites, and `list` must be green.
fn read_dir_all(path: &Path) -> Result<Vec<std::fs::DirEntry>, CommandFailure> {
    match std::fs::read_dir(path) {
        Ok(entries) => entries.map(|entry| entry.map_err(failure)).collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(failure(io_err(path, error))),
    }
}

fn io_err(path: &Path, error: std::io::Error) -> EvaluationError {
    EvaluationError::io(format!("{}: {error}", path.display()))
}

/// Canonical form of one case for the suite digest. The tuple `Debug` of the
/// enums stands in for core's `anchor` canonicaliser until #3557 lands.
fn canonical_case(case: &AnchorCase) -> String {
    let tags = case.tags.iter().cloned().collect::<Vec<_>>().join(",");
    format!(
        "{}\u{0}{}\u{0}{:?}\u{0}{}\u{0}{}",
        case.case_id,
        case.artifact_ref,
        (case.expected_label, case.severity, case.visibility),
        tags,
        case.content_digest,
    )
}

/// Suite digest over NUL-joined canonical parts: the case list, in order, is
/// part of what the digest commits to.
fn suite_digest(suite: &AnchorSuite) -> Digest {
    let mut lines = vec![
        "anchor-suite".to_string(),
        SUITE_SCHEMA_VERSION.to_string(),
        suite.suite_id.as_str().to_string(),
        suite.version.to_string(),
        suite.slot.as_str().to_string(),
        suite.minimum_case_count.to_string(),
    ];
    lines.extend(suite.cases.iter().map(canonical_case));
    let parts: Vec<&[u8]> = lines.iter().map(String::as_bytes).collect();
    Digest::of_parts(&parts)
}

fn validate(suite: &AnchorSuite) -> Result<(), EvaluationError> {
    if suite.version == 0 {
        return Err(invariant("anchor suite version must be >= 1"));
    }
    // A suite that can pass on zero cases is no reference at all.
    if suite.minimum_case_count == 0 {
        return Err(invariant("minimum_case_count must be >= 1"));
    }
    if suite.cases.len() < suite.minimum_case_count {
        return Err(invariant(format!(
            "{} carries {} cases but requires {}",
            suite.suite_id,
            suite.cases.len(),
            suite.minimum_case_count
        )));
    }
    let mut seen = BTreeSet::new();
    for case in &suite.cases {
        let id = case.case_id.as_str();
        if !seen.insert(id) {
            return Err(invariant(format!("duplicate anchor case id: {id}")));
        }
        let path = Path::new(&case.artifact_ref);
        let escaped = case.artifact_ref.is_empty()
            || path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, Component::ParentDir));
        if escaped {
            return Err(invariant(format!(
                "anchor case {id} has an unsafe artifact_ref: {:?}",
                case.artifact_ref
            )));
        }
        // A case with no label cannot be graded; quarantine is the only place
        // an expected verdict may be absent.
        if case.visibility != AnchorVisibility::Quarantine && case.expected_label.is_none() {
            return Err(invariant(format!(
                "anchor case {id} needs expected_label: only quarantined cases may be unlabelled"
            )));
        }
    }
    for subset in &suite.required_subsets {
        if subset.name.is_empty() {
            return Err(invariant("required subset needs a non-empty name"));
        }
        if !suite
            .cases
            .iter()
            .any(|case| case.tags.contains(&subset.tag))
        {
            return Err(invariant(format!(
                "required subset {} references tag {:?} no case carries",
                subset.name, subset.tag
            )));
        }
    }
    Ok(())
}

/// Recompute every case digest against the working tree, naming each broken
/// case id. Never short-circuits: a reviewer needs the full damage report.
fn verify_artifacts(suite: &AnchorSuite, repo_root: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    for case in &suite.cases {
        let path = repo_root.join(&case.artifact_ref);
        match std::fs::read(&path) {
            Ok(bytes) => {
                let actual = Digest::of_bytes(&bytes);
                if actual != case.content_digest {
                    problems.push(format!(
                        "{}: digest mismatch (want {}, got {})",
                        case.case_id,
                        case.content_digest.short(),
                        actual.short()
                    ));
                }
            }
            Err(error) => problems.push(format!("{}: {}: {}", case.case_id, path.display(), error)),
        }
    }
    problems
}

/// The role's view of a suite: mutation drops quarantined cases entirely and
/// hides every protected-holdout label (design spec §Data Model).
fn redact(suite: &AnchorSuite, role: AccessRole) -> AnchorSuite {
    if role != AccessRole::Mutation {
        return suite.clone();
    }
    let mut view = suite.clone();
    view.cases
        .retain(|case| case.visibility != AnchorVisibility::Quarantine);
    for case in &mut view.cases {
        if case.visibility == AnchorVisibility::ProtectedHoldout {
            case.expected_label = None;
        }
    }
    view
}

/// One `anchor.suite.verified` event per suite version. The key is content-keyed
/// so a replayed register is an idempotent no-op rather than a duplicate.
fn journal_record(layout: &EvaluationLayout, suite: &AnchorSuite) -> Result<(), EvaluationError> {
    let digest = suite_digest(suite);
    let mut fields = BTreeMap::new();
    fields.insert("slot".to_string(), suite.slot.as_str().to_string());
    fields.insert("cases".to_string(), suite.cases.len().to_string());
    fields.insert("digest".to_string(), digest.to_string());
    let key = format!("anchor.suite.verified:{}@{}", suite.suite_id, suite.version);
    Journal::open(layout)
        .map(|mut journal| journal.append(now_seconds(), "anchor.suite.verified", &key, fields))
        .map(|_recorded| ())
}

fn print_json(value: &serde_json::Value) -> Result<(), CommandFailure> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| failure(EvaluationError::io(error.to_string())))?;
    println!("{text}");
    Ok(())
}

fn now_seconds() -> u64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    elapsed.as_secs()
}

fn whoami() -> String {
    let user = std::env::var("USER").or_else(|_| std::env::var("USERNAME"));
    user.unwrap_or_else(|_| "autospec anchor".to_string())
}

/// The `EvaluationError` kinds this command adds to core's `new`/`io` set.
fn parse_err(message: impl Into<String>) -> EvaluationError {
    EvaluationError::new(EvaluationErrorKind::Parse, message)
}

fn invariant(message: impl Into<String>) -> EvaluationError {
    EvaluationError::new(EvaluationErrorKind::Invariant, message)
}

fn integrity(message: impl Into<String>) -> EvaluationError {
    EvaluationError::new(EvaluationErrorKind::Integrity, message)
}

/// Every failure renders as `<kind>: <message>` on stderr, matching core's
/// `EvaluationError` Display.
fn failure<Error: std::fmt::Display>(error: Error) -> CommandFailure {
    CommandFailure::diagnostic(error.to_string())
}
