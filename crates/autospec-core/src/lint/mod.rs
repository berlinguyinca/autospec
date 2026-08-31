//! Deterministic issue-body lint rules shared with the shell autospec linter.

pub mod diff;
pub mod implementation;
pub mod pr_size;
mod text;

pub use diff::{parse_unified_diff, DiffFile, DiffHunk, DiffLine, DiffLineKind, UnifiedDiff};
pub use implementation::{
    directive_for, lint_implementation, lint_issue_implementation_contract,
    ImplementationLintContext, ImplementationLintFinding, ImplementationLintOptions,
    ImplementationLintResult, ImplementationLintRule, ImplementationLintSeverity, RepositoryIndex,
};
pub use pr_size::{
    evaluate_patch_size, PatchSize, PatchSizeDimension, PatchSizeEvaluation, PatchSizeLimits,
    DEFAULT_MAX_CHANGED_LINES, DEFAULT_MAX_LOGICAL_UNITS, DEFAULT_MAX_RAW_FILES,
    PROACTIVE_CHANGED_LINES, PROACTIVE_RAW_FILES,
};

use std::collections::BTreeSet;

use text::{
    count_sentence_terminals, first_word_match, has_concrete_goal_object, is_path_character,
    is_word_boundary,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueQualityRule {
    GoalNotOneSentence,
    GoalVague,
    GoalHedge,
    AcEmpty,
    AcProse,
    AcSubjective,
    AcTooLong,
    SmokeNotFenced,
    SmokePlaceholder,
    SmokeMultiLine,
    MissingSectionFilesToRead,
    MissingSectionImplOutline,
    MissingSectionTests,
    DepsMalformed,
    FilesTouchedMalformed,
    TooManyFiles,
    BodyTooLong,
    OutlineTooLong,
    UiSectionsIncomplete,
}

impl IssueQualityRule {
    pub fn id(self) -> &'static str {
        match self {
            Self::GoalNotOneSentence => "GOAL_NOT_ONE_SENTENCE",
            Self::GoalVague => "GOAL_VAGUE",
            Self::GoalHedge => "GOAL_HEDGE",
            Self::AcEmpty => "AC_EMPTY",
            Self::AcProse => "AC_PROSE",
            Self::AcSubjective => "AC_SUBJECTIVE",
            Self::AcTooLong => "AC_TOO_LONG",
            Self::SmokeNotFenced => "SMOKE_NOT_FENCED",
            Self::SmokePlaceholder => "SMOKE_PLACEHOLDER",
            Self::SmokeMultiLine => "SMOKE_MULTI_LINE",
            Self::MissingSectionFilesToRead => "MISSING_SECTION_FILES_TO_READ",
            Self::MissingSectionImplOutline => "MISSING_SECTION_IMPL_OUTLINE",
            Self::MissingSectionTests => "MISSING_SECTION_TESTS",
            Self::DepsMalformed => "DEPS_MALFORMED",
            Self::FilesTouchedMalformed => "FILES_TOUCHED_MALFORMED",
            Self::TooManyFiles => "TOO_MANY_FILES",
            Self::BodyTooLong => "BODY_TOO_LONG",
            Self::OutlineTooLong => "OUTLINE_TOO_LONG",
            Self::UiSectionsIncomplete => "UI_SECTIONS_INCOMPLETE",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueLintFinding {
    pub rule: IssueQualityRule,
    pub message: String,
}

impl IssueLintFinding {
    fn new(rule: IssueQualityRule, message: impl Into<String>) -> Self {
        Self {
            rule,
            message: message.into(),
        }
    }

    pub fn rule_id(&self) -> &'static str {
        self.rule.id()
    }
}

/// Lint an issue body using the same ordered policy as `scripts/lint-issue.sh`.
///
/// This function is deliberately pure: callers provide the body and receive every
/// finding in shell order, with no file-system or process dependency.
pub fn lint_issue_body(body: &str) -> Vec<IssueLintFinding> {
    let document = IssueDocument::parse(body);
    let mut findings = Vec::new();

    check_goal(&document, &mut findings);
    check_acceptance_criteria(&document, &mut findings);
    check_primary_smoke(&document, &mut findings);
    check_sections(&document, &mut findings);
    check_files_touched(&document, &mut findings);
    check_body_size(&document, &mut findings);
    check_outline_size(&document, &mut findings);
    check_ui_sections(&document, &mut findings);

    findings
}

struct IssueDocument<'a> {
    lines: Vec<&'a str>,
}

impl<'a> IssueDocument<'a> {
    fn parse(source: &'a str) -> Self {
        Self {
            lines: source.lines().collect(),
        }
    }

    /// Mirrors the shell `extract_section` helper: the heading itself must be an
    /// exact line, and content ends only at the next `## ` heading.
    fn section(&self, heading: &str) -> Option<Vec<&'a str>> {
        let start = self.lines.iter().position(|line| *line == heading)? + 1;
        let end = self.lines[start..]
            .iter()
            .position(|line| line.starts_with("## "))
            .map_or(self.lines.len(), |offset| start + offset);
        Some(self.lines[start..end].to_vec())
    }

    /// Mirrors `extract_subsection`: the requested text may occur anywhere in
    /// its heading line, and any line beginning with `##` ends the subsection.
    fn subsection(&self, heading: &str) -> Option<Vec<&'a str>> {
        let start = self.lines.iter().position(|line| line.contains(heading))? + 1;
        let end = self.lines[start..]
            .iter()
            .position(|line| line.starts_with("##"))
            .map_or(self.lines.len(), |offset| start + offset);
        Some(self.lines[start..end].to_vec())
    }

    /// Required heading checks accept trailing shell whitespace but no indent.
    fn has_heading(&self, heading: &str) -> bool {
        self.lines.iter().any(|line| {
            line.strip_prefix(heading)
                .is_some_and(|suffix| suffix.chars().all(char::is_whitespace))
        })
    }
}

fn check_goal(document: &IssueDocument<'_>, findings: &mut Vec<IssueLintFinding>) {
    let Some(goal) = document.section("## Goal") else {
        findings.push(IssueLintFinding::new(
            IssueQualityRule::GoalNotOneSentence,
            "Goal section is empty or missing",
        ));
        return;
    };
    let nonempty_lines = goal
        .iter()
        .copied()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if nonempty_lines.is_empty() {
        findings.push(IssueLintFinding::new(
            IssueQualityRule::GoalNotOneSentence,
            "Goal section is empty or missing",
        ));
        return;
    }

    let text = collapse_lines(&nonempty_lines);
    let sentence_count = count_sentence_terminals(&text);
    let word_count = text.split_whitespace().count();
    if sentence_count > 2 || word_count > 30 {
        findings.push(IssueLintFinding::new(
            IssueQualityRule::GoalNotOneSentence,
            format!(
                "Goal must be at most 2 sentences and 30 words; found {sentence_count} sentence(s) and {word_count} word(s)"
            ),
        ));
    }

    if !has_concrete_goal_object(&text) {
        if let Some(vague) = first_word_match(
            &text,
            &[
                "improve", "enhance", "optimize", "polish", "simplify", "refactor", "harden",
            ],
        ) {
            findings.push(IssueLintFinding::new(
                IssueQualityRule::GoalVague,
                format!(
                    "Bare vague verb '{vague}' used without a concrete object (path, backtick term, number, or UPPER_SNAKE label)"
                ),
            ));
        }
    }

    if let Some(hedge) = first_word_match(&text, &["should", "might", "could try", "try to"]) {
        findings.push(IssueLintFinding::new(
            IssueQualityRule::GoalHedge,
            format!("Hedging word '{hedge}' found in Goal section; state the outcome flatly"),
        ));
    }
}

fn check_acceptance_criteria(document: &IssueDocument<'_>, findings: &mut Vec<IssueLintFinding>) {
    let ac = document
        .section("## Acceptance criteria")
        .filter(|lines| !section_command_output(lines).is_empty())
        .or_else(|| document.section("## Acceptance Criteria"))
        .unwrap_or_default();
    let lines = ac
        .into_iter()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();

    if lines.is_empty() {
        findings.push(IssueLintFinding::new(
            IssueQualityRule::AcEmpty,
            "Acceptance criteria section has no checkbox items (section missing or empty)",
        ));
        return;
    }

    if lines.iter().all(|line| !starts_with_checkbox(line)) {
        findings.push(IssueLintFinding::new(
            IssueQualityRule::AcEmpty,
            "Acceptance criteria section has no '- [ ]' checkbox items",
        ));
        return;
    }

    for (index, line) in lines.iter().enumerate() {
        let line_number = index + 1;
        if !is_checkbox_with_content(line) {
            findings.push(IssueLintFinding::new(
                IssueQualityRule::AcProse,
                format!(
                    "AC line {line_number} is not a checkbox ('- [ ]' with content required): {}",
                    first_chars(line, 60)
                ),
            ));
            continue;
        }

        if let Some(subjective) = first_word_match(
            line,
            &[
                "looks",
                "feels",
                "seems",
                "nice",
                "clean",
                "elegant",
                "appropriate",
            ],
        ) {
            findings.push(IssueLintFinding::new(
                IssueQualityRule::AcSubjective,
                format!(
                    "AC item {line_number} contains subjective word '{subjective}': {}",
                    first_chars(line, 60)
                ),
            ));
        }

        let item_body = checkbox_item_body(line);
        let item_length = item_body.chars().count();
        if item_length > 120 {
            findings.push(IssueLintFinding::new(
                IssueQualityRule::AcTooLong,
                format!(
                    "AC item {line_number} is {item_length} chars (max 120): {}...",
                    first_chars(item_body, 60)
                ),
            ));
        }
    }
}

fn check_primary_smoke(document: &IssueDocument<'_>, findings: &mut Vec<IssueLintFinding>) {
    let smoke = document
        .subsection("### Primary smoke test")
        .filter(|lines| !section_command_output(lines).is_empty())
        .or_else(|| document.section("## Verification"));
    let Some(smoke) = smoke else {
        return;
    };
    if section_command_output(&smoke).is_empty() {
        return;
    }

    let Some(block) = first_fenced_block(&smoke) else {
        findings.push(IssueLintFinding::new(
            IssueQualityRule::SmokeNotFenced,
            "No fenced code block found under Primary smoke test heading",
        ));
        return;
    };

    if let Some(placeholder) = first_placeholder(&block) {
        findings.push(IssueLintFinding::new(
            IssueQualityRule::SmokePlaceholder,
            format!("Primary smoke test block contains placeholder '{placeholder}'"),
        ));
    }

    let code_line_count = block
        .iter()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .count();
    if code_line_count > 1 {
        findings.push(IssueLintFinding::new(
            IssueQualityRule::SmokeMultiLine,
            format!(
                "Primary smoke test has {code_line_count} non-blank/non-comment lines (must be exactly 1; use '&&' to chain)"
            ),
        ));
    }
}

fn check_sections(document: &IssueDocument<'_>, findings: &mut Vec<IssueLintFinding>) {
    for (heading, rule, message) in [
        (
            "## Files to read first",
            IssueQualityRule::MissingSectionFilesToRead,
            "Body has no '## Files to read first' heading (implementer reads it)",
        ),
        (
            "## Implementation outline",
            IssueQualityRule::MissingSectionImplOutline,
            "Body has no '## Implementation outline' heading (implementer reads it)",
        ),
        (
            "## Tests required",
            IssueQualityRule::MissingSectionTests,
            "Body has no '## Tests required' heading (implementer reads it)",
        ),
    ] {
        if !document.has_heading(heading) {
            findings.push(IssueLintFinding::new(rule, message));
        }
    }

    if document.has_heading("## Dependencies") {
        if let Some(dependencies) = document.section("## Dependencies") {
            if let Some(bad_dependency) = dependencies.into_iter().find(|line| {
                !line.trim().is_empty() && *line != "none" && !is_dependency_line(line)
            }) {
                findings.push(IssueLintFinding::new(
                    IssueQualityRule::DepsMalformed,
                    format!(
                        "Dependencies line must be 'Depends on issue #N' or 'none': {}",
                        first_chars(bad_dependency, 60)
                    ),
                ));
            }
        }
    }
}

fn check_files_touched(document: &IssueDocument<'_>, findings: &mut Vec<IssueLintFinding>) {
    if !document.has_heading("## Files touched") {
        return;
    }
    let Some(files) = document.section("## Files touched") else {
        return;
    };

    let mut units = BTreeSet::new();
    for line in files.into_iter().filter(|line| !line.trim().is_empty()) {
        match parse_declared_path(line) {
            Ok(path) => {
                if let Some(unit) = normalize_logical_unit(path.as_str()) {
                    units.insert(unit);
                }
            }
            Err(()) => findings.push(IssueLintFinding::new(
                IssueQualityRule::FilesTouchedMalformed,
                format!(
                    "Files touched entry must be one safe repo-relative file or trailing-slash directory: {}",
                    line.trim()
                ),
            )),
        }
    }

    if units.len() > DEFAULT_MAX_LOGICAL_UNITS {
        findings.push(IssueLintFinding::new(
            IssueQualityRule::TooManyFiles,
            format!(
                "Files touched lists {} logical units (max {}; trio members + derived goldens count as one); split the issue to stay small-LLM-sized",
                units.len(),
                DEFAULT_MAX_LOGICAL_UNITS
            ),
        ));
    }
}

/// The five `ui-feature` sections (spec §L1a,
/// docs/superpowers/specs/2026-08-04-autospec-web-ui-design.md): excluded from
/// the ≤400-word body count because Phase 3.5/3.75 append Model-fit and
/// Shared-contracts blocks after the trim, so classified UI children would
/// otherwise systematically trip `needs-quality-bar`.
const UI_SECTION_HEADINGS: [&str; 5] = [
    "## Design reference",
    "## Interaction states",
    "## UX flows",
    "## Motion & feedback",
    "## Device & viewport",
];

/// True when `line` is one of the UI section headings. Trailing whitespace is
/// tolerated, matching `has_heading`, which is what decides the section is present.
/// The two must agree: a heading accepted there but rejected here would be counted
/// against the word cap it is supposed to be exempt from — and markdown's own
/// hard-line-break convention is two trailing spaces, so this is not hypothetical.
fn is_ui_section_heading(line: &str) -> bool {
    UI_SECTION_HEADINGS.iter().any(|heading| {
        line.strip_prefix(heading)
            .is_some_and(|suffix| suffix.chars().all(char::is_whitespace))
    })
}

/// Generated-metadata marker families, mirroring the shell
/// `strip_generated_metadata` helper. Content between a family's begin and end
/// marker is written by autospec itself, not by the issue author, so it is
/// exempt from the authored word budget.
const GENERATED_MARKER_FAMILIES: [&str; 3] = [
    "autospec-classify",
    "autospec-quality",
    "autospec-shared-contracts",
];

fn generated_marker(line: &str, side: &str) -> Option<&'static str> {
    let trimmed = line.trim();
    GENERATED_MARKER_FAMILIES
        .into_iter()
        .find(|family| trimmed == format!("<!-- {family}:{side} -->"))
}

/// Mirrors the shell `strip_ui_sections | strip_generated_metadata` pipeline:
/// drop each UI section heading line plus its body (up to but excluding the next
/// `## ` heading or a generated begin marker), drop every marker-bounded
/// generated block, then count words over what remains. Line-by-line summation is
/// equivalent to splitting the whole source on whitespace, since word boundaries
/// never span a newline.
///
/// A generated block terminates a UI section exactly as a new heading does. Without
/// that, the UI skip swallows the block's opening marker -- it is not a `## ` line --
/// leaving an unmatched end marker, and the whole block is charged to the count.
fn word_count_excluding_ui_sections(document: &IssueDocument<'_>) -> usize {
    // Only well-formed pairs are exempt, matching the shell's
    // `begin && end && begin < end` guard: a half-written block must not
    // suppress counting.
    let mut exempt = vec![false; document.lines.len()];
    for family in GENERATED_MARKER_FAMILIES {
        let begin = format!("<!-- {family}:begin -->");
        let end = format!("<!-- {family}:end -->");
        let last = |needle: &str| {
            document
                .lines
                .iter()
                .rposition(|line| line.trim() == needle)
        };
        if let (Some(b), Some(e)) = (last(&begin), last(&end)) {
            if b < e {
                for slot in &mut exempt[b..=e] {
                    *slot = true;
                }
            }
        }
    }

    let mut skip = false;
    let mut count = 0usize;
    for (index, line) in document.lines.iter().enumerate() {
        if is_ui_section_heading(line) {
            skip = true;
            continue;
        }
        if skip && (line.starts_with("## ") || generated_marker(line, "begin").is_some()) {
            skip = false;
        }
        if skip || exempt[index] {
            continue;
        }
        count += line.split_whitespace().count();
    }
    count
}

fn check_body_size(document: &IssueDocument<'_>, findings: &mut Vec<IssueLintFinding>) {
    let word_count = word_count_excluding_ui_sections(document);
    if word_count > 400 {
        findings.push(IssueLintFinding::new(
            IssueQualityRule::BodyTooLong,
            format!(
                "Body is {word_count} words (max 400); a small-LLM implementer cannot hold an over-long issue"
            ),
        ));
    }
}

fn check_outline_size(document: &IssueDocument<'_>, findings: &mut Vec<IssueLintFinding>) {
    if !document.has_heading("## Implementation outline") {
        return;
    }
    let Some(outline) = document.section("## Implementation outline") else {
        return;
    };
    let line_count = outline
        .iter()
        .filter(|line| !line.trim().is_empty())
        .count();
    if line_count > 30 {
        findings.push(IssueLintFinding::new(
            IssueQualityRule::OutlineTooLong,
            format!(
                "Implementation outline has {line_count} non-blank lines (max 30); tighten or split"
            ),
        ));
    }
}

fn check_ui_sections(document: &IssueDocument<'_>, findings: &mut Vec<IssueLintFinding>) {
    let has_marker = document.lines.iter().any(|line| is_ui_feature_marker(line));
    let has_sections = UI_SECTION_HEADINGS
        .iter()
        .map(|section| document.has_heading(section))
        .collect::<Vec<_>>();
    if !has_marker && has_sections.iter().all(|present| !present) {
        return;
    }

    let missing = UI_SECTION_HEADINGS
        .iter()
        .zip(has_sections)
        .filter_map(|(section, present)| (!present).then_some(format!(" '{section}'")))
        .collect::<String>();
    if !missing.is_empty() {
        findings.push(IssueLintFinding::new(
            IssueQualityRule::UiSectionsIncomplete,
            format!(
                "UI feature detected; missing required section(s):{missing} (UI issues need Design reference + Interaction states + UX flows + Motion & feedback + Device & viewport)"
            ),
        ));
    }
}

fn collapse_lines(lines: &[&str]) -> String {
    lines
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn section_command_output(lines: &[&str]) -> String {
    lines.join("\n").trim_end_matches('\n').to_owned()
}

fn starts_with_checkbox(line: &str) -> bool {
    let Some(after_dash) = line.trim_start().strip_prefix('-') else {
        return false;
    };
    after_dash.trim_start().starts_with("[ ]")
}

fn is_checkbox_with_content(line: &str) -> bool {
    let Some(after_dash) = line.trim_start().strip_prefix('-') else {
        return false;
    };
    let Some(after_box) = after_dash.trim_start().strip_prefix("[ ]") else {
        return false;
    };
    after_box.chars().next().is_some_and(char::is_whitespace) && !after_box.trim().is_empty()
}

fn checkbox_item_body(line: &str) -> &str {
    let without_dash = line.trim_start().strip_prefix('-').unwrap_or(line);
    let without_box = without_dash
        .trim_start()
        .strip_prefix("[ ]")
        .unwrap_or(without_dash);
    without_box.trim_start()
}

fn first_fenced_block<'a>(lines: &[&'a str]) -> Option<Vec<&'a str>> {
    let open = lines.iter().position(|line| line.starts_with("```"))?;
    let rest = &lines[open + 1..];
    let close = rest
        .iter()
        .position(|line| line.starts_with("```"))
        .unwrap_or(rest.len());
    Some(rest[..close].to_vec())
}

fn first_placeholder(lines: &[&str]) -> Option<String> {
    let mut first = None;
    for line in lines {
        for placeholder in ["...", "<TODO>", "TBD", "XXX"] {
            let position = if matches!(placeholder, "TBD" | "XXX") {
                find_standalone_word(line, placeholder)
            } else {
                line.find(placeholder)
            };
            if let Some(position) = position {
                let candidate = (position, placeholder);
                if first
                    .as_ref()
                    .is_none_or(|(existing, _): &(usize, &str)| candidate.0 < *existing)
                {
                    first = Some(candidate);
                }
            }
        }
        if let Some((_, placeholder)) = first {
            return Some(placeholder.to_owned());
        }
    }
    None
}

fn find_standalone_word(text: &str, word: &str) -> Option<usize> {
    let mut offset = 0;
    while let Some(found) = text[offset..].find(word) {
        let start = offset + found;
        let end = start + word.len();
        if is_word_boundary(text, start, end) {
            return Some(start);
        }
        offset = end;
    }
    None
}

fn is_dependency_line(line: &str) -> bool {
    let Some(number) = line.strip_prefix("Depends on issue #") else {
        return false;
    };
    !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct DeclaredPath {
    path: String,
    directory: bool,
}

impl DeclaredPath {
    pub(crate) fn as_str(&self) -> &str {
        &self.path
    }

    pub(crate) fn authorizes(&self, changed_path: &str) -> bool {
        if self.directory {
            changed_path
                .strip_prefix(&self.path)
                .is_some_and(|suffix| !suffix.is_empty())
        } else {
            changed_path == self.path
        }
    }
}

pub(crate) fn parse_declared_path(line: &str) -> Result<DeclaredPath, ()> {
    let line = line.trim();
    let line = line
        .strip_prefix('-')
        .filter(|suffix| suffix.chars().next().is_some_and(char::is_whitespace))
        .map_or(line, str::trim_start);
    let path = match (line.strip_prefix('`'), line.strip_suffix('`')) {
        (Some(without_open), Some(_)) => without_open.strip_suffix('`').ok_or(())?,
        (None, None) => line,
        _ => return Err(()),
    };
    if path.is_empty()
        || path == "."
        || path == "/"
        || path.starts_with('/')
        || !path.bytes().all(is_path_character)
    {
        return Err(());
    }

    let directory = path.ends_with('/');
    let segments = path.trim_end_matches('/').split('/').collect::<Vec<_>>();
    if segments.is_empty()
        || segments
            .iter()
            .any(|segment| segment.is_empty() || matches!(*segment, "." | ".."))
    {
        return Err(());
    }

    Ok(DeclaredPath {
        path: path.to_string(),
        directory,
    })
}

pub(crate) fn normalize_logical_unit(path: &str) -> Option<String> {
    if path.starts_with("tests/fixtures/skill-goldens/") && path.ends_with(".sha256") {
        return None;
    }
    if path.starts_with("skills/")
        && (path.ends_with("/SKILL.md")
            || path.ends_with("/codex/prompt.md")
            || path.ends_with("/opencode/agent.md"))
    {
        if let Some(skill) = path.split('/').nth(1) {
            return Some(format!("skills/{skill}/<trio>"));
        }
    }
    Some(path.to_string())
}

fn is_ui_feature_marker(line: &str) -> bool {
    let mut remaining = line;
    while let Some(open) = remaining.find("<!--") {
        let marker = &remaining[open + "<!--".len()..];
        if let Some(close) = marker.find("-->") {
            if marker[..close].trim() == "ui-feature" {
                return true;
            }
        }
        // The shell regex searches each opener independently, including a
        // syntactically nested marker after an earlier malformed comment.
        remaining = marker;
    }
    false
}

fn first_chars(input: &str, max: usize) -> &str {
    input
        .char_indices()
        .nth(max)
        .map_or(input, |(index, _)| &input[..index])
}
