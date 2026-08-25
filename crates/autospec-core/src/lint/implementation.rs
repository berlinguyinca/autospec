//! Deterministic implementation-policy lint rules, independent of processes and I/O.

mod severity;

use std::collections::{BTreeMap, BTreeSet};

use super::diff::{DiffFile, UnifiedDiff};
use super::pr_size::{evaluate_patch_size, PatchSizeDimension, PatchSizeLimits};

const RULE_EMIT_CAP: usize = 10;
const DEFAULT_AGGREGATE_HARD_CAP: usize = 200;
const EXIT_CAP: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImplementationLintRule {
    PrSize,
    OutOfScope,
    MissingTest,
    Complexity,
    Security,
    TodoLeft,
    MockDb,
    DocOutOfSync,
    VacuousGrepInverseOrTrue,
    VacuousOrTrue,
    VacuousTautology,
    VacuousAcStub,
    VacuousEmptyTest,
    VacuousNoAssert,
    AssertionDensity,
    ReinventRepoUtil,
    NewDepUnjustified,
    NewAbstractionSingleCaller,
}

impl ImplementationLintRule {
    pub fn id(self) -> &'static str {
        match self {
            Self::PrSize => "PR_SIZE",
            Self::OutOfScope => "OUT_OF_SCOPE",
            Self::MissingTest => "MISSING_TEST",
            Self::Complexity => "COMPLEXITY",
            Self::Security => "SECURITY",
            Self::TodoLeft => "TODO_LEFT",
            Self::MockDb => "MOCK_DB",
            Self::DocOutOfSync => "DOC_OUT_OF_SYNC",
            Self::VacuousGrepInverseOrTrue => "VACUOUS_GREP_INVERSE_OR_TRUE",
            Self::VacuousOrTrue => "VACUOUS_OR_TRUE",
            Self::VacuousTautology => "VACUOUS_TAUTOLOGY",
            Self::VacuousAcStub => "VACUOUS_AC_STUB",
            Self::VacuousEmptyTest => "VACUOUS_EMPTY_TEST",
            Self::VacuousNoAssert => "VACUOUS_NO_ASSERT",
            Self::AssertionDensity => "ASSERTION_DENSITY",
            Self::ReinventRepoUtil => "REINVENT_REPO_UTIL",
            Self::NewDepUnjustified => "NEW_DEP_UNJUSTIFIED",
            Self::NewAbstractionSingleCaller => "NEW_ABSTRACTION_SINGLE_CALLER",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "PR_SIZE" => Self::PrSize,
            "OUT_OF_SCOPE" => Self::OutOfScope,
            "MISSING_TEST" => Self::MissingTest,
            "COMPLEXITY" => Self::Complexity,
            "SECURITY" => Self::Security,
            "TODO_LEFT" => Self::TodoLeft,
            "MOCK_DB" => Self::MockDb,
            "DOC_OUT_OF_SYNC" => Self::DocOutOfSync,
            "VACUOUS_GREP_INVERSE_OR_TRUE" => Self::VacuousGrepInverseOrTrue,
            "VACUOUS_OR_TRUE" => Self::VacuousOrTrue,
            "VACUOUS_TAUTOLOGY" => Self::VacuousTautology,
            "VACUOUS_AC_STUB" => Self::VacuousAcStub,
            "VACUOUS_EMPTY_TEST" => Self::VacuousEmptyTest,
            "VACUOUS_NO_ASSERT" => Self::VacuousNoAssert,
            "ASSERTION_DENSITY" => Self::AssertionDensity,
            "REINVENT_REPO_UTIL" => Self::ReinventRepoUtil,
            "NEW_DEP_UNJUSTIFIED" => Self::NewDepUnjustified,
            "NEW_ABSTRACTION_SINGLE_CALLER" => Self::NewAbstractionSingleCaller,
            _ => return None,
        })
    }
}

const HOOK_FAILURE_PREFIX: &str = "Pre-commit lint FAILED. Findings:\n";
const HOOK_FAILURE_TRAILER: &str = "\n\nRun 'lint-implementation.sh --pre-commit --directives' to get re-prompt directives, OR fix the listed RULE_IDs and re-stage.";
const MAX_HOOK_FAILURE_BYTES: usize = 16 * 1024;
const MAX_HOOK_FINDING_LINES: usize = 64;

pub fn parse_blocking_hook_failure(failure: &str) -> Result<Vec<ImplementationLintRule>, String> {
    if failure.len() > MAX_HOOK_FAILURE_BYTES {
        return Err("implementation hook failure evidence exceeds 16384 bytes".to_string());
    }
    let findings = failure
        .strip_prefix(HOOK_FAILURE_PREFIX)
        .and_then(|body| body.strip_suffix(HOOK_FAILURE_TRAILER))
        .ok_or_else(|| {
            "implementation hook failure envelope is malformed or truncated".to_string()
        })?;
    let lines = findings.lines().collect::<Vec<_>>();
    if lines.is_empty() || lines.len() > MAX_HOOK_FINDING_LINES {
        return Err("implementation hook failure has an invalid finding count".to_string());
    }
    let mut seen = BTreeSet::new();
    let mut blocking = Vec::new();
    for line in lines {
        let (informational, record) = if let Some(record) = line.strip_prefix("INFO:") {
            (true, record)
        } else if let Some(record) = line.strip_prefix("ERROR:") {
            (false, record)
        } else {
            (false, line)
        };
        let mut fields = record.splitn(4, ':');
        let id = fields.next().unwrap_or_default();
        let path = fields.next().unwrap_or_default();
        let line_number = fields.next().unwrap_or_default();
        let message = fields.next().and_then(|value| value.strip_prefix(' '));
        let rule = ImplementationLintRule::from_id(id)
            .ok_or_else(|| format!("implementation hook reported unknown rule ID {id:?}"))?;
        if path.is_empty() || line_number.is_empty() || message.is_none_or(str::is_empty) {
            return Err("implementation hook finding record is malformed".to_string());
        }
        if !informational && seen.insert(id) {
            blocking.push(rule);
        }
    }
    if blocking.is_empty() {
        return Err("implementation hook failure contains no blocking finding".to_string());
    }
    Ok(blocking)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImplementationLintSeverity {
    Error,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImplementationLintFinding {
    pub rule: ImplementationLintRule,
    pub severity: ImplementationLintSeverity,
    pub path: String,
    pub line: Option<usize>,
    pub message: String,
}

impl ImplementationLintFinding {
    pub fn rule_id(&self) -> &'static str {
        self.rule.id()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImplementationLintResult {
    pub findings: Vec<ImplementationLintFinding>,
    pub blocking_count: usize,
    pub scope_exploded: bool,
}

impl ImplementationLintResult {
    pub fn exit_code(&self) -> i32 {
        if self.scope_exploded {
            200
        } else {
            self.blocking_count.min(EXIT_CAP) as i32
        }
    }
}

/// Repository evidence supplied by the caller. The core intentionally never
/// shells out or reads the checkout to obtain this evidence.
pub trait RepositoryIndex {
    fn helper_definition(&self, _function_name: &str, _excluding_path: &str) -> Option<String> {
        None
    }

    fn external_caller_count(&self, _stem: &str, _excluding_path: &str) -> Option<usize> {
        None
    }

    fn post_change_file(&self, _path: &str) -> Option<String> {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImplementationLintOptions {
    pub patch_size_limits: PatchSizeLimits,
    pub enable_vacuous_assertions: bool,
    pub enable_assertion_density: bool,
    pub pre_commit_mode: bool,
    pub enable_reuse_lens: bool,
    /// Compatibility hook for the shell's 200-finding safety exit. The default
    /// preserves its value; callers may lower it to exercise the terminal path.
    pub aggregate_hard_cap: usize,
    pub max_file_loc: usize,
    pub max_function_loc: usize,
    pub max_cyclomatic: usize,
    pub complexity_enforced: bool,
}

impl Default for ImplementationLintOptions {
    fn default() -> Self {
        Self {
            patch_size_limits: PatchSizeLimits::default(),
            enable_vacuous_assertions: false,
            enable_assertion_density: false,
            pre_commit_mode: false,
            enable_reuse_lens: false,
            aggregate_hard_cap: DEFAULT_AGGREGATE_HARD_CAP,
            max_file_loc: 400,
            max_function_loc: 50,
            max_cyclomatic: 10,
            complexity_enforced: severity::complexity_enforced_from_env(),
        }
    }
}

pub struct ImplementationLintContext<'a> {
    pub issue_body: Option<&'a str>,
    pub repository: &'a dyn RepositoryIndex,
    pub options: ImplementationLintOptions,
}

/// Lint a supplied diff using the ordered deterministic policy from
/// `scripts/lint-implementation.sh` without inspecting the host checkout.
pub fn lint_implementation(
    diff: &UnifiedDiff,
    context: ImplementationLintContext<'_>,
) -> ImplementationLintResult {
    let skipped_rules = severity::advisory_skip_set(&context.options, context.issue_body);
    let mut collector = FindingCollector::new(skipped_rules, context.options.aggregate_hard_cap);

    detect_pr_size(
        diff,
        context.issue_body,
        context.options.patch_size_limits,
        &mut collector,
    );
    collect_issue_implementation_contract(diff, context.issue_body, false, &mut collector);
    detect_complexity(diff, context.repository, &context.options, &mut collector);
    detect_security(diff, &mut collector);
    detect_todo_left(diff, &mut collector);
    detect_mock_db(diff, &mut collector);
    detect_doc_out_of_sync(diff, &mut collector);

    if context.options.enable_vacuous_assertions || context.options.pre_commit_mode {
        detect_vacuous_assertions(diff, &mut collector);
    }
    if context.options.enable_assertion_density || context.options.pre_commit_mode {
        detect_assertion_density(diff, &mut collector);
    }
    if context.options.enable_reuse_lens {
        detect_reuse_rules(diff, context.repository, &mut collector);
    }

    collector.finish()
}

/// Check only the issue-defined path scope and required regression evidence.
pub fn lint_issue_implementation_contract(
    diff: &UnifiedDiff,
    issue_body: &str,
) -> ImplementationLintResult {
    let mut collector =
        FindingCollector::new(parse_guardian_skips(issue_body), DEFAULT_AGGREGATE_HARD_CAP);
    collect_issue_implementation_contract(diff, Some(issue_body), true, &mut collector);
    collector.finish()
}

fn collect_issue_implementation_contract(
    diff: &UnifiedDiff,
    issue_body: Option<&str>,
    fail_closed_on_missing_outline: bool,
    collector: &mut FindingCollector,
) {
    detect_out_of_scope(diff, issue_body, fail_closed_on_missing_outline, collector);
    detect_missing_test(diff, issue_body, collector);
}

pub fn directive_for(rule: ImplementationLintRule) -> &'static str {
    match rule {
        ImplementationLintRule::PrSize => {
            "Freeze the completed capped slice and move unmet acceptance criteria to ordered continuation issues; never push or merge this oversized diff."
        }
        ImplementationLintRule::OutOfScope => {
            "Restrict diff to files listed in the issue ## Implementation outline; revert or amend the issue body for any extra files."
        }
        ImplementationLintRule::MissingTest => {
            "Add a test under tests/<tier>/ or a project-native scripts/test-* regression artifact before re-pushing."
        }
        ImplementationLintRule::Complexity => {
            "Split functions >50 LOC, files >500 LOC, or nesting >4 — no copy-paste branches."
        }
        ImplementationLintRule::Security => {
            "Remove the flagged pattern: never hardcode secrets, never bypass git hooks or use destructive resets, validate all inputs."
        }
        ImplementationLintRule::TodoLeft => {
            "Remove deferred-work markers from non-test code; file a follow-up issue for genuinely deferred work."
        }
        ImplementationLintRule::MockDb => {
            "Remove DB mock/stub; use the real database per AGENTS.md ## Engineering standards."
        }
        ImplementationLintRule::DocOutOfSync => {
            "Update the doc file(s) covering the changed public surface (CLI flag/env var/export) in this same PR."
        }
        ImplementationLintRule::VacuousGrepInverseOrTrue => {
            "Replace `grep -qv \"X\" || true` with `! grep -q \"X\"` — the current form always exits 0."
        }
        ImplementationLintRule::VacuousOrTrue => {
            "Remove `|| true` from the assertion line so failures propagate correctly."
        }
        ImplementationLintRule::VacuousTautology => {
            "Replace the tautological assertion with one that checks real output from the code under test."
        }
        ImplementationLintRule::VacuousAcStub => {
            "Replace the auto-stub skip with a real assertion that exercises the acceptance criterion."
        }
        ImplementationLintRule::VacuousEmptyTest => "Add at least one assertion to the empty test body.",
        ImplementationLintRule::VacuousNoAssert => {
            "Add an assert/expect/run+grep call to the test so it can actually fail."
        }
        ImplementationLintRule::AssertionDensity => {
            "Add at least one assert/expect/run/grep call to each test block — zero-assertion tests cannot catch regressions."
        }
        ImplementationLintRule::ReinventRepoUtil => {
            "Reuse the existing helper found in scripts/ instead of re-implementing the same function."
        }
        ImplementationLintRule::NewDepUnjustified => {
            "Add a '# why: <reason>' comment in the same diff hunk justifying this new dependency."
        }
        ImplementationLintRule::NewAbstractionSingleCaller => {
            "Inline this abstraction — with only one caller, the named wrapper adds indirection without value."
        }
    }
}

#[derive(Debug, Clone)]
enum PrSizeException {
    GeneratedMigration(String),
    DependencyLockfile(String),
    LockStep(String),
}

impl PrSizeException {
    fn label(&self) -> &'static str {
        match self {
            Self::GeneratedMigration(_) => "generated migration",
            Self::DependencyLockfile(_) => "dependency-solver lockfile",
            Self::LockStep(_) => "mandatory lock-step artifacts",
        }
    }
}

fn detect_pr_size(
    diff: &UnifiedDiff,
    issue_body: Option<&str>,
    limits: PatchSizeLimits,
    collector: &mut FindingCollector,
) {
    let evaluation = evaluate_patch_size(diff, limits);
    if !evaluation.is_hard() {
        return;
    }
    let exception = issue_body.and_then(parse_pr_size_exception);
    let exempt = exception
        .as_ref()
        .is_some_and(|exception| validate_pr_size_exception(diff, exception));
    let size = evaluation.size();
    let exceeded = evaluation
        .hard_dimensions()
        .iter()
        .map(|dimension| match dimension {
            PatchSizeDimension::ChangedLines => "changed_lines",
            PatchSizeDimension::RawFiles => "raw_files",
            PatchSizeDimension::LogicalUnits => "logical_units",
            PatchSizeDimension::Binary => "binary",
        })
        .collect::<Vec<_>>()
        .join(",");
    let category = exception
        .as_ref()
        .filter(|_| exempt)
        .map(|value| format!(" category={}", value.label()))
        .unwrap_or_default();
    let message = format!(
        "changed_lines={}/{} raw_files={}/{} logical_units={}/{} binary={} exceeded={}{}",
        size.changed_lines,
        limits.max_changed_lines,
        size.raw_files,
        limits.max_raw_files,
        size.logical_units,
        limits.max_logical_units,
        size.has_binary,
        exceeded,
        category,
    );
    if exempt {
        collector.info(ImplementationLintRule::PrSize, "-", None, message);
    } else {
        collector.emit(ImplementationLintRule::PrSize, "-", None, message);
    }
}

fn parse_pr_size_exception(body: &str) -> Option<PrSizeException> {
    let reason = body
        .lines()
        .find_map(|line| line.strip_prefix("Guardian: skip-PR_SIZE # "))?;
    let (category, detail) = reason.split_once(':')?;
    if detail.trim().is_empty() {
        return None;
    }
    match category {
        "generated migration" => Some(PrSizeException::GeneratedMigration(
            detail.trim().to_ascii_lowercase(),
        )),
        "dependency-solver lockfile" => Some(PrSizeException::DependencyLockfile(
            detail.trim().to_ascii_lowercase(),
        )),
        "mandatory lock-step artifacts" => {
            Some(PrSizeException::LockStep(detail.trim().to_string()))
        }
        _ => None,
    }
}

fn validate_pr_size_exception(diff: &UnifiedDiff, exception: &PrSizeException) -> bool {
    if diff.files.iter().any(|file| file.is_binary) {
        return false;
    }
    match exception {
        PrSizeException::GeneratedMigration(generator) => diff.files.iter().all(|file| {
            file.path
                .split('/')
                .any(|part| matches!(part, "migration" | "migrations" | "migrate"))
                && file.added_lines().any(|line| {
                    let line = line.content.to_ascii_lowercase();
                    line.contains("generated") && line.contains(generator)
                })
        }),
        PrSizeException::DependencyLockfile(solver) => diff
            .files
            .iter()
            .all(|file| is_solver_lockfile(solver, &file.path)),
        PrSizeException::LockStep(identity) => validate_lock_step_exception(diff, identity),
    }
}

fn is_solver_lockfile(solver: &str, path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    match solver {
        "bundler" => name == "Gemfile.lock",
        "cargo" => name == "Cargo.lock",
        "composer" => name == "composer.lock",
        "go" => name == "go.sum",
        "gradle" => name == "gradle.lockfile",
        "npm" => matches!(name, "package-lock.json" | "npm-shrinkwrap.json"),
        "pipenv" => name == "Pipfile.lock",
        "pnpm" => name == "pnpm-lock.yaml",
        "poetry" => name == "poetry.lock",
        "yarn" => name == "yarn.lock",
        _ => false,
    }
}

fn validate_lock_step_exception(diff: &UnifiedDiff, identity: &str) -> bool {
    let mut mirrors = BTreeMap::<String, Vec<&DiffFile>>::new();
    let mut goldens = Vec::new();
    let mut manual = Vec::new();
    for file in &diff.files {
        if let Some(skill) = golden_skill(&file.path) {
            goldens.push(skill);
            continue;
        }
        if let Some(skill) = lock_step_skill(&file.path) {
            mirrors.entry(skill.to_string()).or_default().push(file);
        } else {
            manual.push(file.clone());
        }
    }
    let skills = mirrors.keys().map(String::as_str).collect::<Vec<_>>();
    if skills.is_empty()
        || !lock_step_identity_matches(identity, &skills)
        || goldens.iter().any(|golden| !mirrors.contains_key(*golden))
        || mirrors.values().any(|files| {
            files.len() != 3
                || files
                    .windows(2)
                    .any(|pair| lock_step_fingerprint(pair[0]) != lock_step_fingerprint(pair[1]))
        })
        || manual
            .iter()
            .any(|file| is_code_file(&file.path) || is_test_file(&file.path))
    {
        return false;
    }
    manual.extend(mirrors.values().map(|files| files[0].clone()));
    !evaluate_patch_size(&UnifiedDiff { files: manual }, PatchSizeLimits::default()).is_hard()
}

fn lock_step_skill(path: &str) -> Option<&str> {
    let (skill, adapter) = path.strip_prefix("skills/")?.split_once('/')?;
    ["SKILL.md", "codex/prompt.md", "opencode/agent.md"]
        .contains(&adapter)
        .then_some(skill)
}

fn golden_skill(path: &str) -> Option<&str> {
    let name = path.strip_prefix("tests/fixtures/skill-goldens/")?;
    [
        ".SKILL.md.sha256",
        ".codex.prompt.md.sha256",
        ".opencode.agent.md.sha256",
    ]
    .into_iter()
    .find_map(|suffix| name.strip_suffix(suffix))
    .filter(|skill| !skill.is_empty() && !skill.contains('/'))
}

fn lock_step_identity_matches(identity: &str, skills: &[&str]) -> bool {
    if skills.len() > 1 {
        let expected = format!(
            "{} adapter trios plus derived goldens",
            skills.join(" and ")
        );
        return identity == expected;
    }
    let skill = skills[0];
    let identity = identity.strip_suffix(" adapters").unwrap_or(identity);
    identity.strip_prefix("skills/").unwrap_or(identity) == skill
}

fn lock_step_fingerprint(file: &DiffFile) -> Vec<Vec<(u8, &str)>> {
    file.hunks
        .iter()
        .map(|hunk| {
            hunk.lines
                .iter()
                .map(|line| (line.kind as u8, line.content.as_str()))
                .collect()
        })
        .collect()
}

struct FindingCollector {
    findings: Vec<ImplementationLintFinding>,
    skipped_rules: BTreeSet<String>,
    emitted_per_rule: BTreeMap<&'static str, usize>,
    blocking_count: usize,
    aggregate_hard_cap: usize,
    scope_exploded: bool,
}

impl FindingCollector {
    fn new(skipped_rules: BTreeSet<String>, aggregate_hard_cap: usize) -> Self {
        Self {
            findings: Vec::new(),
            skipped_rules,
            emitted_per_rule: BTreeMap::new(),
            blocking_count: 0,
            aggregate_hard_cap,
            scope_exploded: false,
        }
    }

    fn stopped(&self) -> bool {
        self.scope_exploded
    }

    fn emit(
        &mut self,
        rule: ImplementationLintRule,
        path: &str,
        line: Option<usize>,
        message: impl Into<String>,
    ) {
        if self.stopped() {
            return;
        }
        let message = message.into();
        if self.skipped_rules.contains(rule.id()) {
            self.info(rule, path, line, message);
            return;
        }
        let count = self.emitted_per_rule.entry(rule.id()).or_default();
        *count += 1;
        if *count > RULE_EMIT_CAP + 1 {
            return;
        }
        let message = if *count == RULE_EMIT_CAP + 1 {
            "+ more (truncated)".to_string()
        } else {
            message
        };
        self.findings.push(ImplementationLintFinding {
            rule,
            severity: ImplementationLintSeverity::Error,
            path: path.to_string(),
            line,
            message,
        });
        self.blocking_count += 1;
        // `emit_finding` in the shell writes the triggering finding first, then
        // writes this uncounted sentinel and terminates on the cap itself.
        if self.blocking_count >= self.aggregate_hard_cap.max(1) {
            self.scope_exploded = true;
            self.findings.push(ImplementationLintFinding {
                rule: ImplementationLintRule::OutOfScope,
                severity: ImplementationLintSeverity::Error,
                path: "-".to_string(),
                line: None,
                message: "too many findings — likely scope explosion".to_string(),
            });
        }
    }

    fn info(
        &mut self,
        rule: ImplementationLintRule,
        path: &str,
        line: Option<usize>,
        message: impl Into<String>,
    ) {
        self.findings.push(ImplementationLintFinding {
            rule,
            severity: ImplementationLintSeverity::Info,
            path: path.to_string(),
            line,
            message: message.into(),
        });
    }

    fn finish(self) -> ImplementationLintResult {
        ImplementationLintResult {
            findings: self.findings,
            blocking_count: self.blocking_count,
            scope_exploded: self.scope_exploded,
        }
    }
}

fn detect_out_of_scope(
    diff: &UnifiedDiff,
    issue_body: Option<&str>,
    fail_closed_on_missing_outline: bool,
    collector: &mut FindingCollector,
) {
    let Some(issue_body) = issue_body else {
        return;
    };
    let outline = section(issue_body, &["Implementation outline"])
        .or_else(|| section(issue_body, &["Implementation scope"]));
    let mut allowed = outline.map(path_tokens).unwrap_or_default();
    // `## Files touched` is a mandatory heading (see `lint::issue`), and the
    // decomposer frequently writes the outline as prose bullets naming
    // behaviours rather than paths. Reading only the outline therefore flagged
    // issues on the very files they declare, and the documented workaround was
    // for the implementer to amend the issue body — i.e. to rewrite the scope
    // it is being measured against. Take the union instead: scope stays
    // fail-closed when *neither* section names a path.
    allowed.extend(
        section(issue_body, &["Files touched"])
            .map(path_tokens)
            .unwrap_or_default(),
    );
    if allowed.is_empty() && !fail_closed_on_missing_outline {
        return;
    }
    for file in &diff.files {
        if collector.stopped() {
            return;
        }
        let base = file.path.rsplit('/').next().unwrap_or(&file.path);
        if !allowed
            .iter()
            .any(|allowed| file.path.contains(allowed) || allowed.contains(base))
        {
            collector.emit(
                ImplementationLintRule::OutOfScope,
                &file.path,
                None,
                "file not listed in ## Implementation outline or ## Files touched",
            );
        }
    }
}

fn detect_missing_test(
    diff: &UnifiedDiff,
    issue_body: Option<&str>,
    collector: &mut FindingCollector,
) {
    let Some(tests) = issue_body.and_then(|body| section(body, &["Tests required"])) else {
        return;
    };
    let tests = tests.to_ascii_lowercase();
    for tier in ["unit", "integration", "smoke", "e2e"] {
        if tests
            .split(|c: char| !c.is_ascii_alphabetic())
            .any(|word| word == tier)
            && !diff
                .files
                .iter()
                .any(|file| is_regression_artifact(&file.path, tier))
        {
            collector.emit(
                ImplementationLintRule::MissingTest,
                &format!("tests/{tier}/"),
                None,
                format!("required test tier '{tier}' not present in diff"),
            );
        }
    }
}

fn is_regression_artifact(path: &str, tier: &str) -> bool {
    path.starts_with(&format!("tests/{tier}/")) || path.starts_with("scripts/test-")
}

fn detect_complexity(
    diff: &UnifiedDiff,
    repository: &dyn RepositoryIndex,
    options: &ImplementationLintOptions,
    collector: &mut FindingCollector,
) {
    // These are separate global passes in the shell. Do not interleave a
    // snapshot check with a diff-only check, because the shared rule cap makes
    // finding order observable.
    for file in &diff.files {
        if collector.stopped() || !is_code_file(&file.path) {
            continue;
        }
        if file.added_line_count() > 500 {
            collector.emit(
                ImplementationLintRule::Complexity,
                &file.path,
                None,
                format!(
                    "file adds {} lines (threshold: 500)",
                    file.added_line_count()
                ),
            );
        }
        scan_added_complexity(file, collector);
    }
    for file in &diff.files {
        if collector.stopped() || !file.path.ends_with(".py") {
            continue;
        }
        if let Some(contents) = repository.post_change_file(&file.path) {
            scan_python_snapshot_nesting(&file.path, &contents, collector);
        }
    }
    for file in &diff.files {
        if collector.stopped() || !is_code_file(&file.path) {
            continue;
        }
        if let Some(contents) = repository.post_change_file(&file.path) {
            scan_snapshot_file_loc(&file.path, &contents, options.max_file_loc, collector);
        }
    }
    for file in &diff.files {
        if collector.stopped() || !file.path.ends_with(".py") {
            continue;
        }
        if let Some(contents) = repository.post_change_file(&file.path) {
            scan_python_function_loc(&file.path, &contents, options.max_function_loc, collector);
        }
    }
    for file in &diff.files {
        if collector.stopped() || !file.path.ends_with(".py") {
            continue;
        }
        if let Some(contents) = repository.post_change_file(&file.path) {
            scan_python_cyclomatic(&file.path, &contents, options.max_cyclomatic, collector);
        }
    }
    let mut names = BTreeMap::<String, usize>::new();
    for file in &diff.files {
        if collector.stopped()
            || !matches!(
                file.path.rsplit('.').next(),
                Some("py" | "ts" | "js" | "go" | "sh")
            )
        {
            continue;
        }
        if let Some(contents) = repository.post_change_file(&file.path) {
            for name in snapshot_definition_names(&contents) {
                *names.entry(name).or_default() += 1;
            }
        }
    }
    for (name, count) in names {
        if count > 1 && !is_conventional_duplicate_name(&name) {
            collector.emit(
                ImplementationLintRule::Complexity,
                "-",
                None,
                format!("duplicate function name '{name}' across changed files — reuse or rename to avoid confusion"),
            );
        }
    }
}

fn scan_added_complexity(file: &DiffFile, collector: &mut FindingCollector) {
    let mut heredoc: Option<(String, bool)> = None;
    let mut shell_function: Option<(String, usize, usize)> = None;
    for line in file.added_lines() {
        if collector.stopped() {
            return;
        }
        let number = line.new_line;
        if let Some((marker, strip_tabs)) = &heredoc {
            let closer = if *strip_tabs {
                line.content.trim_start_matches('\t')
            } else {
                &line.content
            };
            if closer.trim_end() == marker {
                heredoc = None;
            }
            if let Some((_, _, loc)) = &mut shell_function {
                *loc += 1;
            }
            continue;
        }
        if let Some(marker) = heredoc_marker(&line.content) {
            heredoc = Some(marker);
        }
        let mut shell_loc_candidates = [None, None];
        if let Some(name) = shell_function_name(&line.content) {
            shell_loc_candidates[0] = shell_function.take();
            shell_function = Some((name, number.unwrap_or(0), 0));
        } else if let Some((name, start, loc)) = &mut shell_function {
            *loc += 1;
            if line.content.trim() == "}" {
                shell_loc_candidates[1] = Some((name.clone(), *start, *loc));
                shell_function = None;
            }
        }
        emit_shell_function_loc_findings(file, collector, shell_loc_candidates);
        let leading = line.content.len() - line.content.trim_start_matches(' ').len();
        if !file.path.ends_with(".py") && leading / 4 > 4 {
            collector.emit(
                ImplementationLintRule::Complexity,
                &file.path,
                number,
                format!("nesting depth ~{} (threshold: 4)", leading / 4),
            );
        }
    }
    emit_shell_function_loc_findings(file, collector, [shell_function, None]);
}

fn emit_shell_function_loc_findings(
    file: &DiffFile,
    collector: &mut FindingCollector,
    candidates: [Option<(String, usize, usize)>; 2],
) {
    for (name, start, loc) in candidates
        .into_iter()
        .flatten()
        .filter(|(_, _, loc)| *loc > 50)
    {
        collector.emit(
            ImplementationLintRule::Complexity,
            &file.path,
            Some(start),
            format!("function '{name}' is {loc} LOC (threshold: 50)"),
        );
    }
}

fn scan_snapshot_file_loc(
    path: &str,
    contents: &str,
    max_file_loc: usize,
    collector: &mut FindingCollector,
) {
    let loc = contents.bytes().filter(|byte| *byte == b'\n').count();
    if loc > max_file_loc {
        collector.emit(
            ImplementationLintRule::Complexity,
            path,
            None,
            format!(
                "file is {loc} LOC (AUTOSPEC_MAX_FILE_LOC={max_file_loc}); split into smaller modules"
            ),
        );
    }
}

fn scan_python_function_loc(
    path: &str,
    contents: &str,
    max_function_loc: usize,
    collector: &mut FindingCollector,
) {
    let lines = contents.lines().collect::<Vec<_>>();
    let starts = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| python_definition_name(line).map(|name| (index, name)))
        .collect::<Vec<_>>();
    for (index, (start, name)) in starts.iter().enumerate() {
        let end = starts.get(index + 1).map_or(lines.len(), |(next, _)| *next);
        let loc = end.saturating_sub(*start);
        if loc > max_function_loc {
            collector.emit(
                ImplementationLintRule::Complexity,
                path,
                Some(start + 1),
                format!(
                    "function '{name}' is {loc} LOC (AUTOSPEC_MAX_FUNC_LOC={max_function_loc})"
                ),
            );
        }
    }
}

fn scan_python_cyclomatic(
    path: &str,
    contents: &str,
    max_cyclomatic: usize,
    collector: &mut FindingCollector,
) {
    let decisions = contents.lines().filter(is_python_decision).count();
    if decisions > max_cyclomatic {
        collector.emit(ImplementationLintRule::Complexity, path, None, format!("keyword-proxy cyclomatic ~{decisions} (AUTOSPEC_MAX_CYCLOMATIC={max_cyclomatic}); install radon for accurate analysis"));
    }
}

/// A control-flow nesting model for a supplied post-change Python snapshot.
/// `FunctionDef`/`AsyncFunctionDef` and `If`/`elif` contribute nesting scopes
/// like the shell's AST walk, while `match` deliberately does not.
fn scan_python_snapshot_nesting(path: &str, contents: &str, collector: &mut FindingCollector) {
    let mut scopes = Vec::<usize>::new();
    let mut current_function: Option<(usize, usize, bool)> = None;
    for (index, source) in contents.lines().enumerate() {
        let indent = source.len() - source.trim_start_matches(' ').len();
        let text = source.trim_start();
        if text.starts_with("def ") || text.starts_with("async def ") {
            while scopes.last().is_some_and(|previous| *previous >= indent) {
                scopes.pop();
            }
            scopes.push(indent);
            current_function = Some((indent, index + 1, false));
            emit_python_nesting_if_needed(path, scopes.len(), &mut current_function, collector);
            continue;
        }
        if current_function
            .as_ref()
            .is_some_and(|(function_indent, _, _)| !text.is_empty() && indent <= *function_indent)
        {
            current_function = None;
        }
        let branch = python_branch_header(text);
        if branch {
            while scopes.last().is_some_and(|previous| *previous > indent) {
                scopes.pop();
            }
        } else {
            while scopes.last().is_some_and(|previous| *previous >= indent) {
                scopes.pop();
            }
        }
        if is_python_scope_header(text) {
            scopes.push(indent);
            emit_python_nesting_if_needed(path, scopes.len(), &mut current_function, collector);
        }
    }
}

fn emit_python_nesting_if_needed(
    path: &str,
    depth: usize,
    current_function: &mut Option<(usize, usize, bool)>,
    collector: &mut FindingCollector,
) {
    if depth <= 4 {
        return;
    }
    let Some((_, function_line, emitted)) = current_function.as_mut() else {
        return;
    };
    if *emitted {
        return;
    }
    collector.emit(
        ImplementationLintRule::Complexity,
        path,
        Some(*function_line),
        format!("nesting depth {depth} (threshold: 4) at line {function_line}"),
    );
    *emitted = true;
}

fn is_python_scope_header(text: &str) -> bool {
    [
        "if ",
        "elif ",
        "for ",
        "async for ",
        "while ",
        "with ",
        "async with ",
        "try:",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
}

fn python_branch_header(text: &str) -> bool {
    text.starts_with("elif ")
        || text.starts_with("else:")
        || text.starts_with("except")
        || text.starts_with("finally:")
        || text.starts_with("case ")
}

fn detect_security(diff: &UnifiedDiff, collector: &mut FindingCollector) {
    // The shell performs one full diff pass per pattern. Preserve that
    // observable rule order rather than grouping all patterns by source line.
    for (matches, description) in [
        (
            security_eval as fn(&str) -> bool,
            "eval() usage — potential code injection",
        ),
        (security_exec, "exec() usage — potential code injection"),
        (security_no_verify, "--no-verify flag — bypasses git hooks"),
        (
            security_reset_hard,
            "git reset --hard — destructive operation",
        ),
        (security_rm_rf, "rm -rf / — dangerous recursive delete"),
        (contains_aws_key, "hardcoded AWS access key (AKIA...)"),
        (contains_github_token, "hardcoded GitHub token"),
        (
            contains_private_key_header,
            "private key material in committed file",
        ),
    ] {
        for file in &diff.files {
            for line in file.added_lines() {
                if collector.stopped() {
                    return;
                }
                if !matches(&line.content) {
                    continue;
                }
                collector.emit(
                    ImplementationLintRule::Security,
                    &file.path,
                    line.new_line.map(|number| number + 1),
                    description,
                );
            }
        }
    }
}

fn detect_todo_left(diff: &UnifiedDiff, collector: &mut FindingCollector) {
    for file in &diff.files {
        if is_test_file(&file.path) {
            continue;
        }
        for line in file.added_lines() {
            if collector.stopped() {
                return;
            }
            if let Some(marker) = ["TODO", "XXX", "FIXME"]
                .into_iter()
                .find(|marker| contains_word(&line.content, marker))
            {
                collector.emit(
                    ImplementationLintRule::TodoLeft,
                    &file.path,
                    line.new_line,
                    format!("{marker} found in non-test source"),
                );
            }
        }
    }
}

fn detect_mock_db(diff: &UnifiedDiff, collector: &mut FindingCollector) {
    for file in &diff.files {
        if !is_test_file(&file.path) {
            continue;
        }
        for line in file.added_lines() {
            let lower = line.content.to_ascii_lowercase();
            if (contains_word(&lower, "mock") || contains_word(&lower, "stub"))
                && (lower.contains("db.")
                    || lower.contains("database")
                    || lower.contains("datasource")
                    || lower.contains("pg")
                    || lower.contains("mysql")
                    || lower.contains("sqlite"))
            {
                collector.emit(
                    ImplementationLintRule::MockDb,
                    &file.path,
                    line.new_line,
                    "mock/stub of DB symbol detected in test",
                );
            }
        }
    }
}

fn detect_doc_out_of_sync(diff: &UnifiedDiff, collector: &mut FindingCollector) {
    if diff.files.iter().any(|file| is_doc_file(&file.path)) {
        return;
    }
    for file in &diff.files {
        if is_test_file(&file.path) || is_doc_file(&file.path) || is_binaryish(&file.path) {
            continue;
        }
        for line in file.added_lines() {
            if introduced_long_flag(&line.content) {
                collector.emit(
                    ImplementationLintRule::DocOutOfSync,
                    &file.path,
                    line.new_line,
                    "CLI long-flag introduced without touching a doc file",
                );
                break;
            }
            if introduced_env_var(&line.content) {
                collector.emit(
                    ImplementationLintRule::DocOutOfSync,
                    &file.path,
                    line.new_line,
                    "env var introduced without touching a doc file",
                );
                break;
            }
        }
    }
}

fn detect_vacuous_assertions(diff: &UnifiedDiff, collector: &mut FindingCollector) {
    for file in &diff.files {
        if is_non_source_fixture(&file.path) {
            continue;
        }
        for line in file.added_lines() {
            let text = &line.content;
            if text.contains("grep -qv") && text.contains("|| true") {
                collector.emit(
                    ImplementationLintRule::VacuousGrepInverseOrTrue,
                    &file.path,
                    line.new_line,
                    "`grep -qv` with `|| true` is a no-op assertion. Use `! grep -q` instead.",
                );
            } else if is_test_file(&file.path) && text.trim_end().ends_with("|| true") {
                collector.emit(
                    ImplementationLintRule::VacuousOrTrue,
                    &file.path,
                    line.new_line,
                    "`|| true` at end of assertion masks failure — assertion always exits 0.",
                );
            }
            if is_tautology(text) {
                collector.emit(
                    ImplementationLintRule::VacuousTautology,
                    &file.path,
                    line.new_line,
                    "Tautological assertion — always passes regardless of code under test.",
                );
            }
            if file.path.starts_with("tests/ac/")
                && text.contains("skip")
                && text.contains("auto-stub")
            {
                collector.emit(
                    ImplementationLintRule::VacuousAcStub,
                    &file.path,
                    line.new_line,
                    "Auto-generated stub test with skip — replace with a real assertion.",
                );
            }
            if is_empty_test(text) {
                collector.emit(
                    ImplementationLintRule::VacuousEmptyTest,
                    &file.path,
                    line.new_line,
                    "Empty test body — it() callback has no assertions.",
                );
            }
        }
        if is_test_file(&file.path) {
            scan_no_assertions(file, collector);
        }
    }
}

fn detect_assertion_density(diff: &UnifiedDiff, collector: &mut FindingCollector) {
    for file in &diff.files {
        if is_test_file(&file.path) && !is_non_source_fixture(&file.path) {
            scan_assertion_density(file, collector);
        }
    }
}

fn detect_reuse_rules(
    diff: &UnifiedDiff,
    repository: &dyn RepositoryIndex,
    collector: &mut FindingCollector,
) {
    for file in &diff.files {
        detect_reinvent_repo_util(file, repository, collector);
    }
    for file in &diff.files {
        detect_new_dependency(file, repository, collector);
    }
    for file in &diff.files {
        detect_new_abstraction(file, repository, collector);
    }
}

fn detect_reinvent_repo_util(
    file: &DiffFile,
    repository: &dyn RepositoryIndex,
    collector: &mut FindingCollector,
) {
    if file.path.ends_with(".sh") || file.path.ends_with(".bash") {
        for line in file.added_lines() {
            let Some(name) = shell_function_name(&line.content) else {
                continue;
            };
            if matches!(
                name.as_str(),
                "main"
                    | "setup"
                    | "teardown"
                    | "run"
                    | "help"
                    | "usage"
                    | "init"
                    | "cleanup"
                    | "die"
                    | "err"
                    | "warn"
                    | "log"
            ) {
                continue;
            }
            if let Some(existing) = repository.helper_definition(&name, &file.path) {
                if inline_allow(
                    repository,
                    &file.path,
                    line.new_line,
                    ImplementationLintRule::ReinventRepoUtil,
                ) {
                    collector.info(
                        ImplementationLintRule::ReinventRepoUtil,
                        &file.path,
                        line.new_line,
                        format!("function '{name}' already defined in {existing}"),
                    );
                } else {
                    collector.emit(
                        ImplementationLintRule::ReinventRepoUtil,
                        &file.path,
                        line.new_line,
                        format!("function '{name}' already defined in {existing}"),
                    );
                }
            }
        }
    }
}

fn detect_new_abstraction(
    file: &DiffFile,
    repository: &dyn RepositoryIndex,
    collector: &mut FindingCollector,
) {
    if !file.is_new || !is_abstraction_path(&file.path) {
        return;
    }
    let stem = file
        .path
        .rsplit('/')
        .next()
        .unwrap_or(&file.path)
        .rsplit_once('.')
        .map_or(file.path.as_str(), |(stem, _)| stem);
    if let Some(count) = repository
        .external_caller_count(stem, &file.path)
        .filter(|count| *count <= 1)
    {
        collector.emit(ImplementationLintRule::NewAbstractionSingleCaller, &file.path, None, format!("new abstraction '{stem}' has {count} external caller(s) — consider inlining if single-use"));
    }
}

fn detect_new_dependency(
    file: &DiffFile,
    repository: &dyn RepositoryIndex,
    collector: &mut FindingCollector,
) {
    if !is_dependency_manifest(&file.path) {
        return;
    }
    for hunk in &file.hunks {
        let has_why = hunk
            .lines
            .iter()
            .filter(|line| line.kind != super::diff::DiffLineKind::Removed)
            .any(|line| line.content.contains("why:") || line.content.contains("Why:"));
        if has_why {
            continue;
        }
        let added = hunk
            .lines
            .iter()
            .filter(|line| line.kind == super::diff::DiffLineKind::Added)
            .collect::<Vec<_>>();
        for line in added {
            if !looks_like_dependency(&file.path, &line.content) {
                continue;
            }
            if inline_allow(
                repository,
                &file.path,
                line.new_line,
                ImplementationLintRule::NewDepUnjustified,
            ) {
                collector.info(
                    ImplementationLintRule::NewDepUnjustified,
                    &file.path,
                    line.new_line,
                    format!(
                        "dependency added without 'why:' justification: {}",
                        line.content
                    ),
                );
            } else {
                collector.emit(
                    ImplementationLintRule::NewDepUnjustified,
                    &file.path,
                    line.new_line,
                    format!(
                        "dependency added without 'why:' justification: {}",
                        line.content
                    ),
                );
            }
        }
    }
}

fn scan_no_assertions(file: &DiffFile, collector: &mut FindingCollector) {
    let mut block: Option<(usize, String, bool)> = None;
    for line in file.added_lines() {
        if let Some(name) = bats_test_name(&line.content) {
            flush_no_assertion(&mut block, file, collector);
            block = Some((line.new_line.unwrap_or(0), name, false));
            continue;
        }
        let Some((_, _, has_assertion)) = block.as_mut() else {
            continue;
        };
        if has_assertion_word(&line.content) {
            *has_assertion = true;
        }
        if line.content.trim() == "}" {
            flush_no_assertion(&mut block, file, collector);
        }
    }
    flush_no_assertion(&mut block, file, collector);
}

fn flush_no_assertion(
    block: &mut Option<(usize, String, bool)>,
    file: &DiffFile,
    collector: &mut FindingCollector,
) {
    if let Some((line, name, _)) = block.take().filter(|(_, _, asserted)| !*asserted) {
        collector.emit(
            ImplementationLintRule::VacuousNoAssert,
            &file.path,
            Some(line),
            format!("Test '{name}' has no assert/run/grep assertion (WARN)."),
        );
    }
}

fn scan_assertion_density(file: &DiffFile, collector: &mut FindingCollector) {
    let mut block: Option<(usize, bool, bool)> = None; // line, assertion, bats
    for line in file.added_lines() {
        let is_bats = bats_test_name(&line.content).is_some();
        let is_js = line.content.trim_start().starts_with("it(")
            || line.content.trim_start().starts_with("test(");
        let is_python = line.content.trim_start().starts_with("def test_");
        if is_bats || is_js || is_python {
            flush_density(&mut block, file, collector);
            block = Some((line.new_line.unwrap_or(0), false, is_bats));
            continue;
        }
        let Some((_, has_assertion, bats)) = block.as_mut() else {
            continue;
        };
        if has_assertion_word(&line.content) {
            *has_assertion = true;
        }
        if *bats && line.content.trim() == "}" {
            flush_density(&mut block, file, collector);
        }
    }
    flush_density(&mut block, file, collector);
}

fn flush_density(
    block: &mut Option<(usize, bool, bool)>,
    file: &DiffFile,
    collector: &mut FindingCollector,
) {
    if let Some((line, false, _)) = block.take() {
        collector.emit(
            ImplementationLintRule::AssertionDensity,
            &file.path,
            Some(line),
            "test block has no assert/expect/run/grep call — add a real assertion",
        );
    }
}

fn section<'a>(body: &'a str, names: &[&str]) -> Option<&'a str> {
    let start = body
        .lines()
        .scan(0usize, |offset, line| {
            let current = *offset;
            *offset += line.len() + 1;
            Some((current, line))
        })
        .find_map(|(offset, line)| {
            let heading = line.strip_prefix("## ")?;
            names
                .iter()
                .any(|name| heading.eq_ignore_ascii_case(name))
                .then_some(offset + line.len() + 1)
        })?;
    let rest = &body[start.min(body.len())..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    Some(&rest[..end])
}

fn path_tokens(source: &str) -> Vec<String> {
    source
        .split_whitespace()
        .filter_map(|token| {
            let token =
                token.trim_matches(|c: char| matches!(c, '`' | '(' | ')' | ',' | ';' | ':'));
            (token.contains('/') || token.contains('.')).then_some(token.to_string())
        })
        .collect()
}

fn parse_guardian_skips(body: &str) -> BTreeSet<String> {
    body.lines()
        .filter_map(|line| {
            let tail = line.strip_prefix("Guardian:")?;
            if !tail.starts_with(char::is_whitespace) {
                return None;
            }
            let (rules, reason) = tail.trim_start().split_once('#')?;
            if !reason.starts_with(char::is_whitespace) {
                return None;
            }
            let reason = reason.trim_start_matches(char::is_whitespace);
            if !rules.ends_with(char::is_whitespace) || reason.chars().count() < 2 {
                return None;
            }
            let parsed = rules
                .trim_end()
                .split(',')
                .enumerate()
                .map(|token| {
                    let token = if token.0 == 0 {
                        token.1
                    } else {
                        token.1.trim_start()
                    };
                    if token.chars().any(char::is_whitespace) {
                        return None;
                    }
                    token
                        .strip_prefix("skip-")
                        .filter(|rule| {
                            !rule.is_empty()
                                && rule.chars().all(|c| c == '_' || c.is_ascii_uppercase())
                        })
                        .map(str::to_string)
                })
                .collect::<Option<Vec<_>>>()?;
            Some(parsed)
        })
        .flatten()
        .collect()
}

fn is_test_file(path: &str) -> bool {
    path.starts_with("tests/") || path.contains("/tests/")
}
fn is_doc_file(path: &str) -> bool {
    path.starts_with("README")
        || path == "AGENTS.md"
        || path.starts_with("docs/")
        || path.ends_with("/SKILL.md")
        || path == "SKILL.md"
}
fn is_binaryish(path: &str) -> bool {
    [".diff", ".png", ".jpg", ".gif"]
        .iter()
        .any(|suffix| path.ends_with(suffix))
}
fn is_non_source_fixture(path: &str) -> bool {
    [".md", ".txt", ".diff", ".json", ".yaml", ".yml"]
        .iter()
        .any(|suffix| path.ends_with(suffix))
}
fn is_code_file(path: &str) -> bool {
    !is_non_source_fixture(path)
}
fn is_conventional_duplicate_name(name: &str) -> bool {
    matches!(
        name,
        "setUp"
            | "tearDown"
            | "setUpClass"
            | "tearDownClass"
            | "asyncSetUp"
            | "asyncTearDown"
            | "__init__"
            | "__enter__"
            | "__exit__"
            | "main"
    )
}

fn shell_function_name(line: &str) -> Option<String> {
    let line = line
        .trim_start()
        .strip_prefix("function ")
        .unwrap_or(line.trim_start());
    let name = line.split_once("()")?.0.trim();
    (!name.is_empty() && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()))
        .then_some(name.to_string())
}

fn heredoc_marker(line: &str) -> Option<(String, bool)> {
    let start = line.find("<<")?;
    let mut marker = &line[start + 2..];
    let strip_tabs = marker.starts_with('-');
    marker = marker.strip_prefix('-').unwrap_or(marker).trim_start();
    marker = marker.trim_start_matches(['\\', '\'', '"']);
    let marker = marker
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect::<String>();
    (!marker.is_empty()).then_some((marker, strip_tabs))
}

fn python_definition_name(line: &str) -> Option<String> {
    let line = line.trim_start();
    let line = line
        .strip_prefix("def ")
        .or_else(|| line.strip_prefix("class "))?;
    let name = line
        .split(|c: char| c == '(' || c == ':' || c.is_whitespace())
        .next()?;
    (!name.is_empty()).then_some(name.to_string())
}

fn is_python_decision(line: &&str) -> bool {
    line.starts_with(char::is_whitespace)
        && matches!(line.trim_start(), text if text.starts_with("if ") || text.starts_with("elif ") || text.starts_with("else:") || text.starts_with("for ") || text.starts_with("while ") || text.starts_with("case ") || text.starts_with("except") || text.starts_with("catch "))
}

fn contains_word(text: &str, word: &str) -> bool {
    let mut offset = 0;
    while let Some(index) = text[offset..].find(word) {
        let start = offset + index;
        let end = start + word.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        if before.is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_')
            && after.is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_')
        {
            return true;
        }
        offset = end;
    }
    false
}

fn contains_aws_key(text: &str) -> bool {
    text.as_bytes().windows(20).any(|window| {
        window.starts_with(b"AKIA")
            && window[4..]
                .iter()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    })
}
fn security_eval(text: &str) -> bool {
    super::text::contains_dangerous_eval_call(text)
}
fn security_exec(text: &str) -> bool {
    text.contains("exec(")
}
fn security_no_verify(text: &str) -> bool {
    text.contains("--no-verify")
}
fn security_reset_hard(text: &str) -> bool {
    text.contains("git reset --hard")
}
fn security_rm_rf(text: &str) -> bool {
    text.contains("rm -rf /")
}
fn contains_github_token(text: &str) -> bool {
    ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"]
        .iter()
        .any(|prefix| {
            text.match_indices(prefix).any(|(index, _)| {
                text[index + prefix.len()..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .count()
                    >= 36
            })
        })
}
fn contains_private_key_header(text: &str) -> bool {
    text.match_indices("-----BEGIN ").any(|(start, _)| {
        let rest = &text[start + "-----BEGIN ".len()..];
        rest.find("PRIVATE KEY-----").is_some_and(|end| {
            rest[..end]
                .chars()
                .all(|character| character.is_ascii_uppercase() || character == ' ')
        })
    })
}

fn introduced_long_flag(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.windows(2).enumerate().any(|(index, pair)| {
        if pair != b"--" || (index > 0 && !bytes[index - 1].is_ascii_whitespace()) {
            return false;
        }
        let rest = &text[index + 2..];
        if !rest
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_lowercase())
        {
            return false;
        }
        let length = rest
            .chars()
            .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
            .count();
        length >= 3
            && rest
                .chars()
                .nth(length)
                .is_some_and(|next| next.is_ascii_whitespace() || next == '=')
    })
}
fn introduced_env_var(text: &str) -> bool {
    let text = if let Some(rest) = text.strip_prefix("export") {
        if !rest.starts_with(char::is_whitespace) {
            return false;
        }
        rest.trim_start()
    } else {
        text
    };
    let Some((name, _)) = text.split_once('=') else {
        return false;
    };
    name.chars()
        .next()
        .is_some_and(|first| first.is_ascii_uppercase())
        && name
            .split_once('_')
            .is_some_and(|(_, suffix)| !suffix.is_empty())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn is_tautology(text: &str) -> bool {
    let expectation = ["true", "1"].iter().any(|value| {
        ["toBe", "toEqual", "toStrictEqual"]
            .iter()
            .any(|method| text.contains(&format!("expect({value}).{method}({value})")))
    });
    let compact = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    expectation
        || compact.contains("assert(1===1)")
        || compact.contains("assert(1==1)")
        || text.trim() == "assert True"
        || text.contains("xit(")
        || text.contains("assert.ok(true)")
        || text.contains("t.true(true)")
}
fn is_empty_test(text: &str) -> bool {
    (text.contains("it(") && text.contains("() => {}"))
        || (text.trim_start().starts_with("@test ")
            && (text.trim_end().ends_with("{}") || text.trim_end().ends_with("{ }")))
}
fn bats_test_name(text: &str) -> Option<String> {
    let text = text.trim_start();
    let rest = text.strip_prefix("@test ")?.trim_start();
    let quoted = rest.strip_prefix('"')?;
    Some(quoted.split_once('"')?.0.to_string())
}
fn has_assertion_word(text: &str) -> bool {
    ["assert", "expect", "run", "grep", "check", "verify"]
        .iter()
        .any(|word| contains_word(text, word))
}

fn is_dependency_manifest(path: &str) -> bool {
    matches!(
        path.rsplit('/').next().unwrap_or(path),
        "requirements.txt"
            | "package.json"
            | "go.mod"
            | "Cargo.toml"
            | "pyproject.toml"
            | "Gemfile"
    )
}
fn looks_like_dependency(path: &str, line: &str) -> bool {
    let line = line.trim();
    match path.rsplit('/').next().unwrap_or(path) {
        "requirements.txt" => {
            let mut chars = line.chars();
            chars
                .next()
                .is_some_and(|first| first.is_ascii_alphabetic())
                && chars.next().is_some_and(|second| {
                    second.is_ascii_alphanumeric() || matches!(second, '_' | '.' | '-')
                })
        }
        "package.json" => package_json_dependency(line),
        "go.mod" => go_mod_dependency(line),
        "Cargo.toml" => cargo_dependency(line),
        "pyproject.toml" => pyproject_dependency(line),
        "Gemfile" => line.starts_with("gem '") || line.starts_with("gem \""),
        _ => false,
    }
}

fn package_json_dependency(line: &str) -> bool {
    let Some(rest) = line.strip_prefix('"') else {
        return false;
    };
    let Some((name, version)) = rest.split_once("\": \"") else {
        return false;
    };
    name.chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '@')
        && version.chars().next().is_some_and(|first| {
            first.is_ascii_digit() || matches!(first, '^' | '~' | '*' | '>' | '=')
        })
}
fn go_mod_dependency(line: &str) -> bool {
    let line = line.strip_prefix("require ").unwrap_or(line).trim_start();
    let mut parts = line.split_whitespace();
    parts.next().is_some_and(|module| {
        module
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic())
    }) && parts.next().is_some_and(|version| {
        version.starts_with('v')
            && version[1..]
                .chars()
                .next()
                .is_some_and(|first| first.is_ascii_digit())
    })
}
fn cargo_dependency(line: &str) -> bool {
    let Some((name, value)) = line.split_once('=') else {
        return false;
    };
    let name = name.trim();
    name.chars()
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first == '_' || first == '-')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-'))
        && value.trim_start().starts_with('"')
}
fn pyproject_dependency(line: &str) -> bool {
    line.char_indices().any(|(start, character)| {
        if !character.is_ascii_alphabetic() {
            return false;
        }
        let candidate = &line[start..];
        let name_length = candidate
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
            .map(char::len_utf8)
            .sum::<usize>();
        candidate[name_length..]
            .trim_start()
            .starts_with(['>', '=', '<', '!'])
    })
}
fn inline_allow(
    repository: &dyn RepositoryIndex,
    path: &str,
    line: Option<usize>,
    rule: ImplementationLintRule,
) -> bool {
    let Some(line) = line else {
        return false;
    };
    let Some(contents) = repository.post_change_file(path) else {
        return false;
    };
    let allowed = |line: &str| {
        let marker = format!("linter:allow-{}", rule.id());
        line.find(&marker).is_some_and(|start| {
            let reason = &line[start + marker.len()..];
            reason.starts_with(char::is_whitespace) && !reason.trim().is_empty()
        })
    };
    let lines = contents.lines().collect::<Vec<_>>();
    line.checked_sub(1)
        .and_then(|index| lines.get(index))
        .is_some_and(|source| allowed(source))
        || line
            .checked_sub(2)
            .and_then(|index| lines.get(index))
            .is_some_and(|source| allowed(source))
}

fn snapshot_definition_names(contents: &str) -> impl Iterator<Item = String> + '_ {
    contents.lines().filter_map(|line| {
        let line = line.trim_start();
        let rest = ["def ", "function ", "func "]
            .iter()
            .find_map(|prefix| line.strip_prefix(prefix))?;
        let name = rest
            .split(|c: char| c == '(' || c == ':' || c.is_whitespace())
            .next()?;
        (!name.is_empty()).then_some(name.to_string())
    })
}

fn is_abstraction_path(path: &str) -> bool {
    let stem = path
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .rsplit_once('.')
        .map_or(path, |(stem, _)| stem)
        .to_ascii_lowercase();
    [
        "manager", "factory", "adapter", "wrapper", "base", "abstract",
    ]
    .iter()
    .any(|word| stem.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint::{parse_unified_diff, DiffHunk, DiffLine, DiffLineKind};

    struct EmptyRepository;
    impl RepositoryIndex for EmptyRepository {}

    fn file(path: &str, changed_lines: usize) -> DiffFile {
        let line = DiffLine {
            kind: DiffLineKind::Added,
            content: "line".to_string(),
            old_line: None,
            new_line: Some(1),
        };
        DiffFile {
            path: path.to_string(),
            is_new: false,
            is_binary: false,
            hunks: vec![DiffHunk {
                old_start: 1,
                new_start: 1,
                lines: vec![line; changed_lines],
            }],
        }
    }

    fn findings(files: Vec<DiffFile>, issue_body: Option<&str>) -> Vec<ImplementationLintFinding> {
        lint_implementation(
            &UnifiedDiff { files },
            ImplementationLintContext {
                issue_body,
                repository: &EmptyRepository,
                options: ImplementationLintOptions::default(),
            },
        )
        .findings
        .into_iter()
        .filter(|finding| finding.rule == ImplementationLintRule::PrSize)
        .collect()
    }

    fn trio(skill: &str, lines: usize) -> Vec<DiffFile> {
        ["SKILL.md", "codex/prompt.md", "opencode/agent.md"]
            .map(|adapter| file(&format!("skills/{skill}/{adapter}"), lines))
            .to_vec()
    }

    fn severity(files: Vec<DiffFile>, body: Option<&str>) -> ImplementationLintSeverity {
        findings(files, body).remove(0).severity
    }

    fn rejects(files: Vec<DiffFile>, body: Option<&str>) {
        assert_eq!(severity(files, body), ImplementationLintSeverity::Error);
    }

    fn accepts(files: Vec<DiffFile>, body: &str) {
        assert_eq!(
            severity(files, Some(body)),
            ImplementationLintSeverity::Info
        );
    }

    #[test]
    fn pr_size_enforces_every_hard_boundary_in_order() {
        let mut boundary = (0..8).map(|_| file("docs/a.md", 50)).collect::<Vec<_>>();
        boundary[1].path = "docs/b.md".to_string();
        boundary[2].path = "docs/c.md".to_string();
        assert!(findings(boundary, None).is_empty());

        let mut combined = (0..9)
            .map(|index| file(&format!("area-{index}/file.rs"), 45))
            .collect::<Vec<_>>();
        combined[0].is_binary = true;
        let findings = findings(combined, None);
        assert!(findings[0].message.contains(
            "changed_lines=405/400 raw_files=9/8 logical_units=9/3 binary=true \
             exceeded=changed_lines,raw_files,logical_units,binary"
        ));

        assert_eq!(
            directive_for(ImplementationLintRule::PrSize),
            "Freeze the completed capped slice and move unmet acceptance criteria to ordered \
             continuation issues; never push or merge this oversized diff."
        );
    }

    #[test]
    fn pr_size_validates_generated_and_lockfile_exceptions() {
        let generated = "Guardian: skip-PR_SIZE # generated migration: prisma\n";
        let mut migration = file("db/migrations/001_create.sql", 401);
        migration.hunks[0].lines[0].content = "Generated by prisma".to_string();
        let finding = findings(vec![migration.clone()], Some(generated)).remove(0);
        assert!(finding.message.contains("category=generated migration"));

        let lockfile = "Guardian: skip-PR_SIZE # dependency-solver lockfile: npm\n";
        accepts(vec![file("package-lock.json", 401)], lockfile);
        let test = file("nested/tests/manual.json", 1);
        rejects(vec![migration, test.clone()], Some(generated));
        rejects(vec![file("package-lock.json", 400), test], Some(lockfile));

        for suffix in ["", " # generated migration:", " # unknown: tool"] {
            let invalid = format!("Guardian: skip-PR_SIZE{suffix}\n");
            rejects(vec![file("src/manual.rs", 401)], Some(&invalid));
        }
    }

    #[test]
    fn pr_size_validates_lock_step_shape_and_manual_budget() {
        let reason = "Guardian: skip-PR_SIZE # mandatory lock-step artifacts: autospec adapters\n";
        let mut adapters = trio("autospec", 140);
        adapters.push(file(
            "tests/fixtures/skill-goldens/autospec.SKILL.md.sha256",
            1,
        ));
        accepts(adapters.clone(), reason);

        adapters[1].hunks[0].lines[0].content = "divergent".to_string();
        rejects(adapters, Some(reason));

        let lock_step = "Guardian: skip-PR_SIZE # mandatory lock-step artifacts: autospec\n";
        let mut mirrors = trio("autospec", 200);
        mirrors.push(file("docs/manual.md", 300));
        rejects(mirrors, Some(lock_step));

        for (extra, reason) in [
            ("tests/fixtures/skill-goldens/other.sha256", lock_step),
            (
                "tests/fixtures/skill-goldens/autospec.sha256",
                "Guardian: skip-PR_SIZE # mandatory lock-step artifacts: other\n",
            ),
        ] {
            let mut trio = trio("autospec", 140);
            trio.push(file(extra, 1));
            rejects(trio, Some(reason));
        }

        rejects(
            vec![file("Cargo.lock", 401)],
            Some("Guardian: skip-PR_SIZE # dependency-solver lockfile: npm\n"),
        );
        rejects(
            (0..4).map(|n| file(&format!("unit-{n}.md"), 1)).collect(),
            None,
        );

        let mut nested = trio("autospec", 140);
        nested.push(file("nested/tests/manual.yaml", 1));
        rejects(nested, Some(lock_step));

        let mut files = Vec::new();
        for skill in ["autospec", "autospec-run"] {
            files.extend(trio(skill, 60));
            for artifact in ["SKILL.md", "codex.prompt.md", "opencode.agent.md"] {
                let path = format!("tests/fixtures/skill-goldens/{skill}.{artifact}.sha256");
                files.push(file(&path, 1));
            }
        }
        accepts(
            files,
            "Guardian: skip-PR_SIZE # mandatory lock-step artifacts: autospec and \
             autospec-run adapter trios plus derived goldens\n",
        );
    }

    #[test]
    fn lint_issue_implementation_contract_classifies_scope_and_regression_artifacts() {
        let body = "## Implementation outline\n\n- `src/allowed.rs`\n- `scripts/test-autonomous-status-panel.mjs`\n\
                    \n## Tests required\n\n- unit: regression artifact\n";

        for path in [
            "tests/unit/allowed.rs",
            "scripts/test-autonomous-status-panel.mjs",
        ] {
            let result = lint_issue_implementation_contract(
                &UnifiedDiff {
                    files: vec![file(path, 1)],
                },
                body,
            );
            assert!(
                result
                    .findings
                    .iter()
                    .all(|finding| finding.rule != ImplementationLintRule::MissingTest),
                "{path} must satisfy the regression-artifact contract"
            );
        }

        let result = lint_issue_implementation_contract(
            &UnifiedDiff {
                files: vec![file("src/omitted.rs", 1)],
            },
            body,
        );
        assert!(result.findings.iter().any(|finding| {
            finding.rule == ImplementationLintRule::OutOfScope && finding.path == "src/omitted.rs"
        }));

        for path in ["src/allowed.rs", "scripts/helper.mjs"] {
            let result = lint_issue_implementation_contract(
                &UnifiedDiff {
                    files: vec![file(path, 1)],
                },
                body,
            );
            assert!(
                result
                    .findings
                    .iter()
                    .any(|finding| finding.rule == ImplementationLintRule::MissingTest),
                "{path} must not satisfy the regression-artifact contract"
            );
        }

        let in_scope = lint_issue_implementation_contract(
            &UnifiedDiff {
                files: vec![file("src/allowed.rs", 1)],
            },
            body,
        );
        assert!(in_scope
            .findings
            .iter()
            .all(|finding| finding.rule != ImplementationLintRule::OutOfScope));
    }

    #[test]
    fn lint_issue_implementation_contract_rejects_changed_path_without_outline() {
        let diff = parse_unified_diff(concat!(
            "diff ",
            "--git a/src/changed.rs b/src/changed.rs\n\
             --- a/src/changed.rs\n\
             +++ b/src/changed.rs\n\
             @@ -1 +1 @@\n\
             -old\n\
             +new\n"
        ))
        .expect("literal diff must parse");

        let result = lint_issue_implementation_contract(&diff, "## Goal\n\nChange behavior.\n");
        let out_of_scope = result
            .findings
            .iter()
            .filter(|finding| finding.rule == ImplementationLintRule::OutOfScope)
            .collect::<Vec<_>>();

        assert_eq!(out_of_scope.len(), 1);
        assert_eq!(out_of_scope[0].path, "src/changed.rs");
        assert_eq!(out_of_scope[0].severity, ImplementationLintSeverity::Error);
        assert_eq!(result.blocking_count, 1);
        assert_eq!(result.exit_code(), 1);
    }

    #[test]
    fn lint_issue_implementation_contract_rejects_changed_path_with_empty_outline() {
        let diff = parse_unified_diff(concat!(
            "diff ",
            "--git a/src/changed.rs b/src/changed.rs\n\
             --- a/src/changed.rs\n\
             +++ b/src/changed.rs\n\
             @@ -1 +1 @@\n\
             -old\n\
             +new\n"
        ))
        .expect("literal diff must parse");

        let result = lint_issue_implementation_contract(&diff, "## Implementation outline\n\n");
        let out_of_scope = result
            .findings
            .iter()
            .filter(|finding| finding.rule == ImplementationLintRule::OutOfScope)
            .collect::<Vec<_>>();

        assert_eq!(out_of_scope.len(), 1);
        assert_eq!(out_of_scope[0].path, "src/changed.rs");
        assert_eq!(out_of_scope[0].severity, ImplementationLintSeverity::Error);
        assert_eq!(result.blocking_count, 1);
        assert_eq!(result.exit_code(), 1);
    }

    #[test]
    fn lint_issue_implementation_contract_rejects_changed_path_with_pathless_outline() {
        let diff = parse_unified_diff(concat!(
            "diff ",
            "--git a/src/changed.rs b/src/changed.rs\n\
             --- a/src/changed.rs\n\
             +++ b/src/changed.rs\n\
             @@ -1 +1 @@\n\
             -old\n\
             +new\n"
        ))
        .expect("literal diff must parse");
        let body = "## Implementation outline\n\n- Update the shared classifier before delivery\n";

        let result = lint_issue_implementation_contract(&diff, body);
        let out_of_scope = result
            .findings
            .iter()
            .filter(|finding| finding.rule == ImplementationLintRule::OutOfScope)
            .collect::<Vec<_>>();

        assert_eq!(out_of_scope.len(), 1);
        assert_eq!(out_of_scope[0].path, "src/changed.rs");
        assert_eq!(out_of_scope[0].severity, ImplementationLintSeverity::Error);
        assert_eq!(result.blocking_count, 1);
        assert_eq!(result.exit_code(), 1);
    }

    #[test]
    fn lint_issue_implementation_contract_accepts_matching_outline_path() {
        let diff = parse_unified_diff(concat!(
            "diff ",
            "--git a/src/changed.rs b/src/changed.rs\n\
             --- a/src/changed.rs\n\
             +++ b/src/changed.rs\n\
             @@ -1 +1 @@\n\
             -old\n\
             +new\n"
        ))
        .expect("literal diff must parse");
        let body = "## Implementation outline\n\n- `src/changed.rs`\n";

        let result = lint_issue_implementation_contract(&diff, body);

        assert!(result
            .findings
            .iter()
            .all(|finding| finding.rule != ImplementationLintRule::OutOfScope));
    }

    #[test]
    fn lint_issue_implementation_contract_accepts_files_touched_path_with_prose_outline() {
        let diff = parse_unified_diff(concat!(
            "diff ",
            "--git a/src/changed.rs b/src/changed.rs\n\
             --- a/src/changed.rs\n\
             +++ b/src/changed.rs\n\
             @@ -1 +1 @@\n\
             -old\n\
             +new\n"
        ))
        .expect("literal diff must parse");
        // The decomposer routinely writes prose bullets in the outline while
        // still declaring the concrete paths under the mandatory
        // `## Files touched` heading; both sections define the scope.
        let body = "## Implementation outline\n\n\
                    - Update the shared classifier before delivery\n\n\
                    ## Files touched\n\n\
                    - `src/changed.rs`\n";

        let result = lint_issue_implementation_contract(&diff, body);

        assert!(
            result
                .findings
                .iter()
                .all(|finding| finding.rule != ImplementationLintRule::OutOfScope),
            "a path declared under ## Files touched must be in scope even when \
             the outline names no path: {:?}",
            result.findings
        );
        assert_eq!(result.blocking_count, 0);
        assert_eq!(result.exit_code(), 0);
    }

    #[test]
    fn lint_issue_implementation_contract_rejects_changed_path_when_neither_section_names_a_path() {
        let diff = parse_unified_diff(concat!(
            "diff ",
            "--git a/src/changed.rs b/src/changed.rs\n\
             --- a/src/changed.rs\n\
             +++ b/src/changed.rs\n\
             @@ -1 +1 @@\n\
             -old\n\
             +new\n"
        ))
        .expect("literal diff must parse");
        // Neither bullet contains `/` or `.`, so `path_tokens` yields nothing
        // from either section and the fail-closed branch — not the per-file
        // comparison loop — is the one under test.
        let body = "## Implementation outline\n\n\
                    - Update the shared classifier before delivery\n\n\
                    ## Files touched\n\n\
                    - To be determined\n";

        let result = lint_issue_implementation_contract(&diff, body);
        let out_of_scope = result
            .findings
            .iter()
            .filter(|finding| finding.rule == ImplementationLintRule::OutOfScope)
            .collect::<Vec<_>>();

        assert_eq!(out_of_scope.len(), 1);
        assert_eq!(out_of_scope[0].path, "src/changed.rs");
        assert_eq!(out_of_scope[0].severity, ImplementationLintSeverity::Error);
        assert_eq!(result.blocking_count, 1);
        assert_eq!(result.exit_code(), 1);
    }

    #[test]
    fn missing_test_directive_mentions_every_accepted_artifact_shape() {
        assert_eq!(
            directive_for(ImplementationLintRule::MissingTest),
            "Add a test under tests/<tier>/ or a project-native scripts/test-* regression artifact \
             before re-pushing."
        );
    }

    #[test]
    fn implementation_hook_failure_parses_only_known_blocking_rules() {
        let failure = concat!(
            "Pre-commit lint FAILED. Findings:\n",
            "INFO:MISSING_TEST:tests/unit/:-: skipped by issue contract\n",
            "COMPLEXITY:scripts/run.sh:12: nesting depth is 5\n",
            "ERROR:VACUOUS_OR_TRUE:tests/run.bats:8: assertion masks failure\n",
            "\n",
            "Run 'lint-implementation.sh --pre-commit --directives' to get re-prompt directives, OR fix the listed RULE_IDs and re-stage."
        );

        assert_eq!(
            parse_blocking_hook_failure(failure).expect("parse exact hook failure"),
            vec![
                ImplementationLintRule::Complexity,
                ImplementationLintRule::VacuousOrTrue,
            ]
        );
    }

    #[test]
    fn implementation_hook_failure_rejects_unknown_truncated_and_oversized_evidence() {
        let trailer = "\n\nRun 'lint-implementation.sh --pre-commit --directives' to get re-prompt directives, OR fix the listed RULE_IDs and re-stage.";
        for failure in [
            format!("Pre-commit lint FAILED. Findings:\nUNKNOWN:x:-: no{trailer}"),
            "Pre-commit lint FAILED. Findings:\nCOMPLEXITY:x:-: truncated".to_string(),
            format!(
                "Pre-commit lint FAILED. Findings:\nCOMPLEXITY:x:-: {}{trailer}",
                "x".repeat(16_385)
            ),
        ] {
            assert!(parse_blocking_hook_failure(&failure).is_err(), "{failure}");
        }
    }
}
