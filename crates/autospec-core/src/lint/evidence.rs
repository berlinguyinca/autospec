//! Evidence-preservation lint for shell failure paths.
//!
//! Source: GitHub issue #3882 "A failure must not name evidence it destroys",
//! extending the value-printing distinction from #3866 to the artifacts behind
//! the values. Two shapes of the same defect:
//!
//! - A failure message points at a command's output (`see \`<cmd>\``) but the
//!   script never captures that output anywhere, so the named evidence does not
//!   exist when the run finishes.
//! - A `trap`-based cleanup handler removes a file (or the directory holding
//!   it) that a failure message points at, so the evidence is gone by the time
//!   anyone acts on the message.
//!
//! Every predicate here is pure and scans linearly (no regex): authored scripts
//! cannot trigger regex denial of service, and no shell is executed. Matching is
//! *textual* — a `$LOG_DIR/run.log` in a message is connected to
//! `rm -rf "$LOG_DIR"` in a trap by string comparison, so a reference and its
//! removal written in different spellings are not connected. That limitation is
//! intrinsic to not executing the shell; the finding messages stay factual.

use std::collections::BTreeSet;

/// The two evidence-preservation rules with stable ids (dashboard-facing; do
/// not renumber).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceRule {
    /// AS-EVD-001: a failure message references a command's output
    /// (`see \`<cmd>\``) but the script never captures that command's output.
    UncapturedCommandReference,
    /// AS-EVD-002: a `trap`-based cleanup handler removes a file that a
    /// failure message references.
    TrapDestroysReferencedFile,
}

impl EvidenceRule {
    /// Stable rule id, `AS-EVD-001` or `AS-EVD-002`.
    pub fn id(self) -> &'static str {
        match self {
            Self::UncapturedCommandReference => "AS-EVD-001",
            Self::TrapDestroysReferencedFile => "AS-EVD-002",
        }
    }

    /// Short rule name for log lines and reports.
    pub fn name(self) -> &'static str {
        match self {
            Self::UncapturedCommandReference => "uncaptured-command-reference",
            Self::TrapDestroysReferencedFile => "trap-destroys-referenced-file",
        }
    }
}

/// One finding: a failure path that names evidence it destroys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceFinding {
    /// The rule that fired.
    pub rule: EvidenceRule,
    /// 1-based line of the failure message that names the evidence.
    pub line: usize,
    /// The evidence named by the message: the command or file reference.
    pub subject: String,
    /// Human-readable explanation.
    pub message: String,
}

/// First-token words that print a message.
const MESSAGE_VERBS: [&str; 12] = [
    "echo",
    "printf",
    "die",
    "fail",
    "error",
    "err",
    "warn",
    "warning",
    "log_error",
    "log_fail",
    "log_warn",
    "log_err",
];

/// File-like suffixes: a backtick span ending in one of these is a file
/// reference even when it contains spaces.
const FILE_SUFFIXES: [&str; 10] = [
    ".md", ".txt", ".json", ".jsonl", ".yaml", ".yml", ".toml", ".log", ".csv", ".html",
];

/// Signals whose traps run on the way out of the script and can destroy
/// evidence a failure message still points at.
const CLEANUP_SIGNALS: [&str; 4] = ["EXIT", "INT", "TERM", "HUP"];

/// `AS-EVD-001` + `AS-EVD-002`: scan a shell script and return every failure
/// message that names evidence the script itself destroys or never captures.
///
/// Findings are ordered by (line, subject). The same subject referenced by two
/// different messages yields two findings — each message is its own failure
/// path.
pub fn lint_failure_evidence(script: &str) -> Vec<EvidenceFinding> {
    let removals = trap_removal_targets(script);
    let mut findings = Vec::new();

    for (index, line) in script.lines().enumerate() {
        if !is_failure_message_line(script, index) {
            continue;
        }
        for reference in references_in(line) {
            if is_command_like(&reference) {
                if !command_output_captured(script, &reference) {
                    findings.push(EvidenceFinding {
                        rule: EvidenceRule::UncapturedCommandReference,
                        line: index + 1,
                        subject: reference.clone(),
                        message: format!(
                            "failure message on line {} references the output of `{}` but the script never captures that output; the named evidence does not exist when the run finishes",
                            index + 1,
                            reference
                        ),
                    });
                }
            } else if removals.iter().any(|target| removes(target, &reference)) {
                findings.push(EvidenceFinding {
                    rule: EvidenceRule::TrapDestroysReferencedFile,
                    line: index + 1,
                    subject: reference.clone(),
                    message: format!(
                        "failure message on line {} references `{}` but a trap cleanup handler removes it; the evidence is gone by the time the message is acted on",
                        index + 1,
                        reference
                    ),
                });
            }
        }
    }

    findings.sort_by(|a, b| (a.line, a.subject.as_str()).cmp(&(b.line, b.subject.as_str())));
    findings
}

/// True when a line is a failure message: it prints (first token is a message
/// verb), it references something (a backtick span or a `$var` token), and it
/// sits in a failure context — it prints to stderr, or one of the next two
/// non-blank, non-comment lines exits/returns.
fn is_failure_message_line(script: &str, index: usize) -> bool {
    let line = script.lines().nth(index).unwrap_or_default();
    let verb = first_token(line);
    if !MESSAGE_VERBS.contains(&verb) {
        return false;
    }
    if backtick_spans(line).is_empty() && dollar_references(line).is_empty() {
        return false;
    }
    has_failure_context(script, index)
}

/// Failure context: stderr on this line, or `exit`/`return` within the next
/// two non-blank, non-comment lines (a plain `exit` inherits the failed
/// status, so it counts).
fn has_failure_context(script: &str, index: usize) -> bool {
    let line = script.lines().nth(index).unwrap_or_default();
    if line.contains(">&2") || line.contains("1>&2") {
        return true;
    }
    let mut seen = 0usize;
    for following in script.lines().skip(index + 1) {
        let trimmed = following.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        seen += 1;
        if seen > 2 {
            break;
        }
        let token = first_token(trimmed);
        if token == "exit" || token == "return" {
            // `exit=...` / `return=...` would be an assignment in another
            // context; in bash `exit =` is a syntax error, but an `EXIT=1`
            // style assignment starts with the same word, so skip `=`.
            if !trimmed[token.len()..].trim_start().starts_with('=') {
                return true;
            }
        }
    }
    false
}

/// Every reference a failure message makes: backtick spans plus `$var` and
/// `${var}` tokens (possibly with a `/path` suffix). De-duplicated, in order.
fn references_in(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    for span in backtick_spans(line) {
        if !out.contains(&span) {
            out.push(span);
        }
    }
    for reference in dollar_references(line) {
        if !out.contains(&reference) {
            out.push(reference);
        }
    }
    out
}

/// All backtick-delimited spans with non-empty content, in order.
fn backtick_spans(line: &str) -> Vec<String> {
    let mut spans = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('`') {
        let after_open = &rest[open + 1..];
        match after_open.find('`') {
            Some(close) if close > 0 => {
                spans.push(after_open[..close].to_string());
                rest = &after_open[close + 1..];
            }
            _ => break,
        }
    }
    spans
}

/// All `$var` and `${var}` tokens with an optional `/suffix`, in order.
/// Single-quoted regions are skipped: a `$` there is literal text. Command
/// substitutions `$(...)` are not variable references and are skipped.
fn dollar_references(line: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0usize;
    let mut in_single = false;
    while i < chars.len() {
        match chars[i] {
            '\'' => in_single = !in_single,
            '$' if !in_single => {
                if i + 1 < chars.len() && chars[i + 1] == '{' {
                    if let Some(close) = chars[i + 2..].iter().position(|&c| c == '}') {
                        let past = i + close + 2;
                        let name: String = chars[i + 2..i + 2 + close].iter().collect();
                        let mut reference = format!("${{{name}}}");
                        let suffix = read_reference_suffix(&chars, past);
                        if !suffix.is_empty() {
                            reference.push_str(&suffix);
                        }
                        refs.push(reference);
                        i = past;
                        continue;
                    }
                } else if i + 1 < chars.len() && chars[i + 1].is_ascii_alphabetic() {
                    let mut end = i + 1;
                    while end < chars.len()
                        && (chars[end].is_ascii_alphanumeric() || chars[end] == '_')
                    {
                        end += 1;
                    }
                    let name: String = chars[i + 1..end].iter().collect();
                    let mut reference = format!("${name}");
                    let suffix = read_reference_suffix(&chars, end);
                    if !suffix.is_empty() {
                        reference.push_str(&suffix);
                    }
                    refs.push(reference);
                    i = end;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    refs
}

/// Read a `/path` suffix that directly follows a variable reference.
fn read_reference_suffix(chars: &[char], start: usize) -> String {
    let mut end = start;
    while end < chars.len() {
        if chars[end] != '/' {
            break;
        }
        end += 1;
        while end < chars.len()
            && (chars[end].is_ascii_alphanumeric()
                || matches!(chars[end], '_' | '.' | '-' | '$' | '{' | '}'))
        {
            end += 1;
        }
    }
    chars[start..end].iter().collect()
}

/// A backtick span names a command when it is multi-word and does not look
/// like a path: no leading `/`, `./`, `../`, `~` or `$`, and no file-like
/// suffix. Single-word spans are treated as file references — a failure
/// message pointing at a bare file is the common shape, and treating bare
/// words as commands would flag prose.
fn is_command_like(span: &str) -> bool {
    let span = span.trim();
    if !span.contains(' ') {
        return false;
    }
    if span.starts_with('/')
        || span.starts_with("./")
        || span.starts_with("../")
        || span.starts_with("~/")
        || span.starts_with('$')
    {
        return false;
    }
    let lowered = span.to_lowercase();
    if FILE_SUFFIXES.iter().any(|suffix| lowered.ends_with(suffix)) {
        return false;
    }
    true
}

/// `AS-EVD-001` satisfaction: some line of the script runs `command` and
/// captures its output — a redirect (`>`), a pipe (`|`), or command
/// substitution (`$(`). `||` (logical OR) is not a pipe. Comment lines never
/// count as capture.
fn command_output_captured(script: &str, command: &str) -> bool {
    script.lines().any(|line| {
        let trimmed = line.trim();
        !trimmed.is_empty()
            && !trimmed.starts_with('#')
            && line_contains_command(trimmed, command)
            && has_capture_marker(trimmed)
    })
}

/// True when `command` occurs in `line` as a whole run of words. The run may
/// start at line start, after whitespace, or after a shell metacharacter that
/// ends a word (`(`, `{`, `;`, `|`, `&`, `<`, `>` — the `$(` capture case);
/// it must end at end-of-line or before whitespace.
fn line_contains_command(line: &str, command: &str) -> bool {
    let command = command.trim();
    if command.is_empty() {
        return false;
    }
    let bytes = line.as_bytes();
    let mut offset = 0usize;
    while let Some(found) = line[offset..].find(command) {
        let start = offset + found;
        let end = start + command.len();
        let preceded = start == 0
            || bytes[start - 1].is_ascii_whitespace()
            || matches!(
                bytes[start - 1],
                b'(' | b'{' | b';' | b'|' | b'&' | b'<' | b'>'
            );
        let followed = end == line.len() || bytes[end].is_ascii_whitespace();
        if preceded && followed {
            return true;
        }
        offset = end;
    }
    false
}

/// A capture marker: `>` redirect, `|` pipe (but not `||`), or `$(` command
/// substitution.
fn has_capture_marker(line: &str) -> bool {
    if line.contains("$(") || line.contains('>') {
        return true;
    }
    let chars: Vec<char> = line.chars().collect();
    chars.iter().enumerate().any(|(i, &c)| {
        c == '|' && chars.get(i.wrapping_sub(1)) != Some(&'|') && chars.get(i + 1) != Some(&'|')
    })
}

/// Every path a `trap`-based cleanup handler removes: the arguments of `rm`
/// and `unlink` inside inline trap bodies (`trap 'rm -f "$f"' EXIT`) and
/// inside named handlers (`trap cleanup EXIT` plus `cleanup() { ... }`).
/// Quote marks are stripped; flags and the command word are not targets.
pub fn trap_removal_targets(script: &str) -> Vec<String> {
    let mut targets = BTreeSet::new();
    for line in script.lines() {
        let trimmed = line.trim();
        if first_token(trimmed) != "trap" {
            continue;
        }
        let rest = trimmed.trim_start_matches("trap").trim_start();
        let (handler_or_body, signals_text) = if rest.starts_with('\'') || rest.starts_with('"') {
            // Inline body: `trap 'rm -f "$f"' EXIT`. The body runs from the
            // first quote to its matching close (the body may itself contain
            // the other quote character); signals follow the close.
            let chars: Vec<char> = rest.chars().collect();
            let quote = chars[0];
            match chars[1..].iter().position(|c| *c == quote) {
                Some(p) => (
                    chars[1..1 + p].iter().collect(),
                    chars[2 + p..].iter().collect(),
                ),
                None => (rest[1..].to_string(), String::new()),
            }
        } else {
            // Named handler: `trap cleanup EXIT`.
            let first = first_token(rest).to_string();
            (first, rest.to_string())
        };
        if handler_or_body.is_empty() || handler_or_body == "-" {
            continue;
        }
        // Only cleanup traps destroy evidence out from under a failure:
        // EXIT (always) and the interruption signals (on the way out).
        if !signals_text
            .split_whitespace()
            .any(|s| CLEANUP_SIGNALS.contains(&s))
        {
            continue;
        }
        let body = if handler_or_body.contains(' ') {
            // An inline body with spaces is the body itself.
            Some(handler_or_body)
        } else if handler_or_body.starts_with('$') {
            // `trap "$(mktemp)" EXIT` style: not removable-file logic.
            None
        } else {
            function_body(script, &handler_or_body)
        };
        if let Some(body) = body {
            for removed in removal_targets_in(&body) {
                targets.insert(removed);
            }
        }
    }
    targets.into_iter().collect()
}

/// `rm`/`unlink` arguments in a stretch of shell text (a trap body or a
/// function body).
fn removal_targets_in(body: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        let verb = first_token(trimmed);
        if verb != "rm" && verb != "unlink" {
            continue;
        }
        for token in shell_tokens(&trimmed[verb.len()..]) {
            let unquoted = token.trim_matches(|c| c == '\'' || c == '"');
            if unquoted.is_empty() || unquoted.starts_with('-') || unquoted.contains('(') {
                continue;
            }
            targets.push(unquoted.to_string());
        }
    }
    targets
}

/// The body of `name() { ... }` / `function name { ... }` declared anywhere in
/// the script, as text. Brace matching is textual (braces inside quotes are
/// not special to this lint); the first declaration wins, and the opening
/// brace may sit on a later line since the scan counts braces from the
/// declaration onward.
fn function_body(script: &str, name: &str) -> Option<String> {
    let lines: Vec<&str> = script.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        if !declares_function(line.trim(), name) {
            continue;
        }
        let mut depth = 0i32;
        let mut started = false;
        for (tail_index, tail) in lines.iter().enumerate().skip(index) {
            for ch in tail.chars() {
                match ch {
                    '{' => {
                        depth += 1;
                        started = true;
                    }
                    '}' => {
                        depth -= 1;
                        if started && depth <= 0 {
                            return Some(lines[index..=tail_index].join("\n"));
                        }
                    }
                    _ => {}
                }
            }
        }
        // Unbalanced braces: take to end of script rather than silently drop.
        return Some(lines[index..].join("\n"));
    }
    None
}

/// True when `line` opens a function definition for `name`: `name() {`,
/// `name() {` on a following line, `function name {`, `function name() {`.
fn declares_function(line: &str, name: &str) -> bool {
    if line == format!("{name}()") || line.starts_with(&format!("{name}() {{")) {
        return true;
    }
    if line.starts_with(&format!("{name} () {{")) {
        return true;
    }
    if let Some(rest) = line.strip_prefix("function ") {
        let rest = rest.trim_start();
        rest == name
            || rest.starts_with(&format!("{name} "))
            || rest == format!("{name}()")
            || rest.starts_with(&format!("{name}() {{"))
            || rest.starts_with(&format!("{name} {{"))
    } else {
        false
    }
}

/// `true` when removing `target` also removes `reference`: exact match, or the
/// reference lives inside the removed path (a directory removal destroys its
/// contents).
fn removes(target: &str, reference: &str) -> bool {
    // `${var}` and `$var` name the same shell reference.
    let normalize = |s: &str| s.replace("${", "$").replace('}', "");
    let normalized_target = normalize(target);
    let target = normalized_target.trim_end_matches('/');
    let reference = normalize(reference);
    reference == target || reference.starts_with(&format!("{target}/"))
}

/// The first whitespace-delimited token of a line (empty string if none).
fn first_token(line: &str) -> &str {
    line.trim().split_whitespace().next().unwrap_or_default()
}

/// Whitespace-delimited tokens of a stretch of shell text.
fn shell_tokens(text: &str) -> Vec<String> {
    text.split_whitespace().map(|t| t.to_string()).collect()
}
