//! Deterministic issue-body lint rules shared with the shell autospec linter.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueQualityRule {
    GoalNotOneSentence,
    AcProse,
    SmokeMultiLine,
}

impl IssueQualityRule {
    pub fn id(self) -> &'static str {
        match self {
            Self::GoalNotOneSentence => "GOAL_NOT_ONE_SENTENCE",
            Self::AcProse => "AC_PROSE",
            Self::SmokeMultiLine => "SMOKE_MULTI_LINE",
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

pub fn lint_issue_body(body: &str) -> Vec<IssueLintFinding> {
    let mut findings = Vec::new();
    check_goal(body, &mut findings);
    check_acceptance_criteria(body, &mut findings);
    check_primary_smoke(body, &mut findings);
    findings
}

fn check_goal(body: &str, findings: &mut Vec<IssueLintFinding>) {
    let Some(goal) = section(body, "## Goal") else {
        findings.push(IssueLintFinding::new(
            IssueQualityRule::GoalNotOneSentence,
            "Goal section is empty or missing",
        ));
        return;
    };
    let text = collapse_nonblank_lines(goal);
    if text.is_empty() {
        findings.push(IssueLintFinding::new(
            IssueQualityRule::GoalNotOneSentence,
            "Goal section is empty or missing",
        ));
        return;
    }

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
}

fn check_acceptance_criteria(body: &str, findings: &mut Vec<IssueLintFinding>) {
    let ac = section(body, "## Acceptance criteria")
        .or_else(|| section(body, "## Acceptance Criteria"))
        .unwrap_or("");
    let lines = ac
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    let checkbox_count = lines
        .iter()
        .filter(|line| starts_with_checkbox(line))
        .count();

    if checkbox_count == 0 {
        return;
    }

    for (idx, line) in lines.iter().enumerate() {
        if !is_checkbox_with_content(line) {
            findings.push(IssueLintFinding::new(
                IssueQualityRule::AcProse,
                format!("AC line {} is not a checkbox", idx + 1),
            ));
        }
    }
}

fn starts_with_checkbox(line: &str) -> bool {
    let trimmed = line.trim_start();
    let Some(after_dash) = trimmed.strip_prefix('-') else {
        return false;
    };
    after_dash.trim_start().starts_with("[ ]")
}

fn is_checkbox_with_content(line: &str) -> bool {
    let trimmed = line.trim_start();
    let Some(after_dash) = trimmed.strip_prefix('-') else {
        return false;
    };
    let after_space = after_dash.trim_start();
    let Some(after_box) = after_space.strip_prefix("[ ]") else {
        return false;
    };
    !after_box.trim().is_empty()
}

fn check_primary_smoke(body: &str, findings: &mut Vec<IssueLintFinding>) {
    let Some(smoke_section) =
        subsection(body, "### Primary smoke test").or_else(|| section(body, "## Verification"))
    else {
        return;
    };
    let Some(block) = first_fenced_block(smoke_section) else {
        return;
    };
    let executable_lines = block
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .count();

    if executable_lines > 1 {
        findings.push(IssueLintFinding::new(
            IssueQualityRule::SmokeMultiLine,
            format!("Primary smoke test has {executable_lines} executable lines"),
        ));
    }
}

fn section<'a>(body: &'a str, heading: &str) -> Option<&'a str> {
    let mut start = None;
    for (offset, line) in line_offsets(body) {
        if line.trim_end() == heading {
            start = Some(offset + line.len());
            break;
        }
    }
    let start = start?;
    let tail = &body[start..];
    let end = line_offsets(tail)
        .find(|(_, line)| line.starts_with("## "))
        .map(|(offset, _)| offset)
        .unwrap_or(tail.len());
    Some(&tail[..end])
}

fn subsection<'a>(body: &'a str, heading: &str) -> Option<&'a str> {
    let mut start = None;
    for (offset, line) in line_offsets(body) {
        if line.trim_end().starts_with(heading) {
            start = Some(offset + line.len());
            break;
        }
    }
    let start = start?;
    let tail = &body[start..];
    let end = line_offsets(tail)
        .find(|(_, line)| line.starts_with("##") || line.starts_with("###"))
        .map(|(offset, _)| offset)
        .unwrap_or(tail.len());
    Some(&tail[..end])
}

fn first_fenced_block(section: &str) -> Option<&str> {
    let open = section.find("```")?;
    let after_open_line = section[open..].find('\n').map(|idx| open + idx + 1)?;
    let close = section[after_open_line..].find("```")? + after_open_line;
    Some(&section[after_open_line..close])
}

fn collapse_nonblank_lines(input: &str) -> String {
    input
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn count_sentence_terminals(text: &str) -> usize {
    let chars: Vec<char> = text.chars().collect();
    chars
        .iter()
        .enumerate()
        .filter(|(idx, ch)| match ch {
            '?' | '!' => true,
            '.' => chars.get(idx + 1).is_none_or(|next| next.is_whitespace()),
            _ => false,
        })
        .count()
}

fn line_offsets(input: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut offset = 0;
    input.split_inclusive('\n').map(move |line| {
        let current = offset;
        offset += line.len();
        (current, line)
    })
}
