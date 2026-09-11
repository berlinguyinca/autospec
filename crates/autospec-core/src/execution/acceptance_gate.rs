//! The acceptance gate for conversion closure (issue #4274).
//!
//! A green gate proves the code is not broken. Only a check against the
//! acceptance criteria proves the issue is complete.
//!
//! The incident: a patch that satisfied 1 of 4 acceptance items was
//! converted with a `Closes #N` trailer because the gate was green. The
//! issue closed with three of its acceptance items unmet — and nothing in
//! the pipeline had ever checked them, because the gate is not a
//! criterion and the criteria were not the gate.
//!
//! Four invariants, each a primitive here:
//!
//! 1. **A patch conversion must not fire a close keyword from a patch
//!    that has not been checked against the issue's acceptance list.**
//!    [`assess_report`] reads the issue body and the agent's report and
//!    returns the [`Verdict`]; a spec with an acceptance section and a
//!    report with no per-criterion statuses is [`Verdict::Unassessed`],
//!    and [`closure_body`] gives it the held-for-review body — the patch
//!    converts but the issue stays open. A green build never substitutes
//!    for an assessment.
//! 2. **A partially-met issue closes with its remainder filed, or does
//!    not close.** [`Verdict::PartiallyMet`] carries the remainder (the
//!    1-based indexes of the partial and deferred criteria),
//!    [`follow_up_body`] renders that remainder as a follow-up issue's
//!    own acceptance section, and [`closure_body`] writes the partial-fix
//!    marker — `Refs #N (does not close it)` — never the close keyword.
//! 3. **The acceptance criteria are individually checkable.**
//!    [`parse_acceptance_criteria`] returns one item per `- [ ]` line in
//!    the `## Acceptance criteria` section, in order. The gate compares
//!    one recorded status to one criterion; a report that cannot say
//!    which criterion it met cannot say the issue is done.
//! 4. **The per-criterion status is recorded in `status.txt` alongside
//!    the build and test exit codes.**
//!    [`parse_criterion_statuses`] reads the `criteria` key from the
//!    report in either on-disk shape (the gate shape,
//!    `criteria=met,partial,met`, or the fleet shape,
//!    `criteria: met partial met`). The parser is strict about what it
//!    reads: a token that is not `met`, `partial`, or `deferred` fails
//!    the report, the same way a non-integer `build_rc` fails it in
//!    [`status_triage`](super::status_triage). An absent key is not an
//!    error and not an assessment.
//!
//! Everything here is pure: no I/O, no git, no subprocesses. The caller
//! (the CLI conversion pass) reads the issue body and the report file,
//! calls [`assess_report`], and acts on the [`Verdict`] the way
//! [`status_triage`](super::status_triage) hands its decision to the
//! caller that performs the I/O.

use crate::evidence_fidelity::closure_authorized;
use crate::execution::closure::{
    conversion_pr_body, has_held_for_review_marker, has_partial_fix_marker,
    held_for_review_pr_body, partial_fix_pr_body,
};

/// Parse the issue body's `## Acceptance criteria` section into its
/// individual criteria, one per `- [ ]` line, in order (invariant 3).
///
/// The section grammar is the issue-quality contract's: a heading line
/// whose text is `acceptance criteria` (any level, any case), then `- [ ]`
/// items until the next heading or end of body. A body with no such
/// section — or one whose section has no `- [ ]` items — returns an empty
/// vec, which the gate reads as "nothing to gate"
/// ([`Verdict::NotRequired`]), not as a failure.
pub fn parse_acceptance_criteria(body: &str) -> Vec<String> {
    let Some(start) = find_acceptance_heading(body) else {
        return Vec::new();
    };
    body.lines()
        .skip(start + 1)
        .take_while(|line| heading_level(line).is_none())
        .filter_map(|line| {
            line.trim()
                .strip_prefix("- [ ] ")
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToOwned::to_owned)
        })
        .collect()
}

/// The 0-based index of the acceptance-criteria heading line, if the body
/// carries one.
fn find_acceptance_heading(body: &str) -> Option<usize> {
    body.lines().position(|line| {
        let level = heading_level(line).unwrap_or(0);
        if level == 0 {
            return false;
        }
        let trimmed = line.trim();
        trimmed[level..]
            .trim()
            .eq_ignore_ascii_case("acceptance criteria")
    })
}

/// The level of a Markdown heading line, or `None` if the line is not a
/// heading. A heading is 1–6 `#` characters followed by whitespace and
/// text; it ends any section.
fn heading_level(line: &str) -> Option<usize> {
    let trimmed = line.trim();
    let level = trimmed
        .chars()
        .take_while(|&character| character == '#')
        .count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &trimmed[level..];
    if rest.is_empty() || !rest.starts_with(' ') {
        return None;
    }
    Some(level)
}

/// One criterion's recorded status (invariant 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CriterionStatus {
    /// The criterion is met; the agent's report says so and the evidence
    /// stands up to it.
    Met,
    /// The criterion is partially met: part of the work is in, part is
    /// not. A partially-met criterion goes to the follow-up (invariant 2).
    Partial,
    /// The criterion is deliberately deferred, with a reason. A deferred
    /// criterion also goes to the follow-up (invariant 2).
    Deferred,
}

impl CriterionStatus {
    /// The machine token the report records, and the token the gate
    /// accepts on read-back.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Met => "met",
            Self::Partial => "partial",
            Self::Deferred => "deferred",
        }
    }

    /// Parse one recorded token. Strict: anything that is not `met`,
    /// `partial`, or `deferred` (case-insensitive) is an error — a status
    /// the gate cannot read is not read as `met`.
    pub fn parse(token: &str) -> Result<Self, String> {
        match token.to_ascii_lowercase().as_str() {
            "met" => Ok(Self::Met),
            "partial" => Ok(Self::Partial),
            "deferred" => Ok(Self::Deferred),
            other => Err(format!(
                "criterion status `{other}` is not `met`, `partial`, or `deferred`"
            )),
        }
    }
}

/// Read the per-criterion statuses out of a `status.txt` body
/// (invariant 4).
///
/// The reader accepts the same two on-disk shapes the report's other
/// fields use, and may be mixed:
///
/// - the gate shape: `criteria=met,partial,met` — the value is one token
///   with comma separators, because whitespace ends a token on a
///   `key=value` line;
/// - the fleet shape: `criteria: met partial deferred met` — one key per
///   line, the value is the rest of the line split on whitespace or
///   commas.
///
/// An absent `criteria` key returns `Ok(None)`: the report simply did not
/// record an assessment, which is an input to the verdict
/// ([`Verdict::Unassessed`]), not a parse failure. A present key with an
/// empty value or a token the gate cannot read fails the report, naming
/// the offending line — the same strictness the report reader applies to
/// a non-integer `build_rc`: a silently dropped status would turn an
/// unassessed close into an assessed one.
pub fn parse_criterion_statuses(status_txt: &str) -> Result<Option<Vec<CriterionStatus>>, String> {
    for (index, raw) in status_txt.lines().enumerate() {
        let line_number = index + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.contains('=') {
            for token in line.split_whitespace() {
                let (key, value) = token
                    .split_once('=')
                    .ok_or_else(|| format!("line {line_number}: not 'key=value': {token}"))?;
                if key.trim() != "criteria" {
                    continue;
                }
                let statuses = value
                    .split(',')
                    .map(str::trim)
                    .filter(|token| !token.is_empty())
                    .map(|token| {
                        CriterionStatus::parse(token)
                            .map_err(|error| format!("line {line_number}: {error}"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                return non_empty(statuses, line_number);
            }
        } else {
            let (key, value) = line.split_once(':').ok_or_else(|| {
                format!("line {line_number}: not 'key=value' or 'key: value': {line}")
            })?;
            if key.trim() != "criteria" {
                continue;
            }
            let statuses = value
                .split(|character: char| character.is_whitespace() || character == ',')
                .map(str::trim)
                .filter(|token| !token.is_empty())
                .map(|token| {
                    CriterionStatus::parse(token)
                        .map_err(|error| format!("line {line_number}: {error}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            return non_empty(statuses, line_number);
        }
    }
    Ok(None)
}

/// An empty recorded value is not a zero-status assessment — it is a
/// malformed report.
fn non_empty(
    statuses: Vec<CriterionStatus>,
    line_number: usize,
) -> Result<Option<Vec<CriterionStatus>>, String> {
    if statuses.is_empty() {
        Err(format!(
            "line {line_number}: criterion status value is empty"
        ))
    } else {
        Ok(Some(statuses))
    }
}

/// The verdict the conversion pass acts on (invariant 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The issue declares no acceptance criteria: nothing to gate, and
    /// the conversion proceeds as before.
    NotRequired,
    /// Every criterion is recorded and every one is met: the close
    /// keyword is authorized.
    Complete,
    /// Every criterion has a recorded status, and at least one is
    /// `partial` or `deferred`. The issue closes with its remainder filed
    /// as a follow-up, or does not close (invariant 2). `remainder` holds
    /// the 1-based indexes of the not-met criteria, in order.
    PartiallyMet {
        /// The 1-based indexes of the partial and deferred criteria.
        remainder: Vec<usize>,
    },
    /// At least one criterion has no recorded status — the report is
    /// silent, short, or longer than the list. The conversion pass must
    /// not fire a close keyword for this issue; the hold names the
    /// unassessed criteria (invariant 1). `missing` holds the 1-based
    /// indexes of the criteria with no status (empty when the report is
    /// longer than the list, a malformed report the line still names).
    Unassessed {
        /// The 1-based indexes of the criteria with no recorded status.
        missing: Vec<usize>,
    },
}

/// Compare a recorded assessment against the criteria it is supposed to
/// cover. Pure: no I/O, no clock.
///
/// The comparison is one status to one criterion, in order — the gate
/// never takes "the report says met" as "criterion 2 is met". A report
/// with fewer statuses than criteria leaves the tail unassessed; a report
/// with more statuses than criteria is malformed and unassessed too,
/// because the extra rows do not name a criterion.
pub fn assess(criteria: &[String], statuses: &[CriterionStatus]) -> Verdict {
    if criteria.is_empty() {
        return Verdict::NotRequired;
    }
    let assessed = statuses.len().min(criteria.len());
    if statuses.len() != criteria.len() {
        return Verdict::Unassessed {
            missing: (assessed + 1..=criteria.len()).collect(),
        };
    }
    let remainder: Vec<usize> = statuses
        .iter()
        .enumerate()
        .filter_map(|(position, status)| {
            (!matches!(status, CriterionStatus::Met)).then_some(position + 1)
        })
        .collect();
    if remainder.is_empty() {
        Verdict::Complete
    } else {
        Verdict::PartiallyMet { remainder }
    }
}

/// The conversion pass's entry point (invariant 1): read the issue body's
/// acceptance list and the report's recorded statuses, and return the
/// verdict.
///
/// The failure direction is fixed: a spec with an acceptance section and
/// a report that recorded no `criteria` key is [`Verdict::Unassessed`]
/// with every criterion named — the absence of an assessment is never
/// read as an all-met assessment. The parse errors of
/// [`parse_criterion_statuses`] propagate: a report the pass cannot read
/// fails the conversion, the same way an unreadable report fails the
/// triage's holds.
pub fn assess_report(issue_body: &str, status_txt: &str) -> Result<Verdict, String> {
    let criteria = parse_acceptance_criteria(issue_body);
    let statuses = parse_criterion_statuses(status_txt)?;
    Ok(match statuses {
        Some(statuses) => assess(&criteria, &statuses),
        None => {
            if criteria.is_empty() {
                Verdict::NotRequired
            } else {
                Verdict::Unassessed {
                    missing: (1..=criteria.len()).collect(),
                }
            }
        }
    })
}

/// The PR body the verdict authorizes for the conversion, or the refusal
/// (invariant 2).
///
/// - [`Verdict::NotRequired`] and [`Verdict::Complete`] get the closing
///   body: `Closes #N`.
/// - [`Verdict::PartiallyMet`] gets the partial-fix body: `Refs #N
///   (does not close it)` — the close keyword is not in the body, and the
///   remainder goes out through [`follow_up_body`] instead.
/// - [`Verdict::Unassessed`] gets the held-for-review body: `Refs #N
///   (NOT closing: held for supervisor review; ...)` — the patch
///   converts but the issue stays open until the criteria are verified.
pub fn closure_body(verdict: &Verdict, issue_number: u64, summary: &str) -> Result<String, String> {
    match verdict {
        Verdict::NotRequired | Verdict::Complete => Ok(conversion_pr_body(issue_number, summary)),
        Verdict::PartiallyMet { .. } => Ok(partial_fix_pr_body(issue_number, summary)),
        Verdict::Unassessed { .. } => Ok(held_for_review_pr_body(issue_number, summary)),
    }
}

/// Whether the body [`closure_body`] would write for this verdict is an
/// authorized form — either a close keyword the tracker will honor, or
/// an explicit non-closure marker (partial-fix or held-for-review). The
/// markers are checked, not assumed: a regression that swapped the
/// bodies turns this check red before it closes an issue it should not.
pub fn closes_authorized(verdict: &Verdict, issue_number: u64) -> bool {
    let Ok(body) = closure_body(verdict, issue_number, "gate check") else {
        return false;
    };
    matches!(
        closure_authorized(&body, issue_number),
        crate::evidence_fidelity::ClosureVerdict::Authorized { .. }
    ) || has_partial_fix_marker(&body, issue_number)
        || has_held_for_review_marker(&body, issue_number)
}

/// The follow-up issue's body carrying the remainder of a partially-met
/// issue (invariant 2).
///
/// The remainder's not-met criteria become the follow-up's own
/// `## Acceptance criteria` section — individually checkable, the way
/// the parent's were (invariant 3) — so the next patch converts against
/// the remainder and not against the parent's full list. `None` for
/// every verdict but [`Verdict::PartiallyMet`]: a complete issue has no
/// remainder to file, and an unassessed one has no filed remainder yet.
pub fn follow_up_body(parent_issue: u64, criteria: &[String], verdict: &Verdict) -> Option<String> {
    let Verdict::PartiallyMet { remainder } = verdict else {
        return None;
    };
    let items: Vec<&String> = remainder
        .iter()
        .filter_map(|index| criteria.get(index - 1))
        .collect();
    if items.is_empty() {
        return None;
    }
    let mut body = String::new();
    body.push_str(&format!(
        "## Goal\n\nComplete the remainder of #{parent_issue}'s acceptance criteria.\n"
    ));
    body.push_str("\n## Acceptance criteria\n\n");
    for item in items {
        body.push_str("- [ ] ");
        body.push_str(item);
        body.push('\n');
    }
    Some(body)
}

/// The one line the pass prints for any verdict (invariant 1): what the
/// verdict is and which criteria it is acting on, by number.
pub fn verdict_line(verdict: &Verdict, criterion_count: usize) -> String {
    match verdict {
        Verdict::NotRequired => {
            "ACCEPTANCE: not required — the issue declares no acceptance criteria".to_string()
        }
        Verdict::Complete => {
            format!(
                "ACCEPTANCE: {criterion_count}/{criterion_count} criteria met — close authorized"
            )
        }
        Verdict::PartiallyMet { remainder } => {
            let met = criterion_count - remainder.len();
            format!(
                "ACCEPTANCE: {met}/{criterion_count} criteria met; remainder {} — close only with a follow-up filing the remainder",
                indexes_line(remainder)
            )
        }
        Verdict::Unassessed { missing } => {
            let assessed = criterion_count - missing.len();
            format!(
                "ACCEPTANCE: {assessed}/{criterion_count} criteria assessed; unassessed {} — do not close",
                indexes_line(missing)
            )
        }
    }
}

/// Render a list of 1-based indexes for a line: `2, 3, 4`, or `all` for
/// the empty-list edge of a malformed report.
fn indexes_line(indexes: &[usize]) -> String {
    if indexes.is_empty() {
        "all".to_string()
    } else {
        indexes
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue_body(criteria: &[&str]) -> String {
        let mut body = String::from("## Goal\n\nDo the thing.\n\n## Acceptance criteria\n\n");
        for criterion in criteria {
            body.push_str("- [ ] ");
            body.push_str(criterion);
            body.push('\n');
        }
        body
    }

    fn statuses(tokens: &[&str]) -> Vec<CriterionStatus> {
        tokens
            .iter()
            .map(|token| CriterionStatus::parse(token).expect("test token is valid"))
            .collect()
    }

    // --- parse_acceptance_criteria -----------------------------------

    #[test]
    fn parses_each_checkbox_item_in_order() {
        let criteria = parse_acceptance_criteria(&issue_body(&[
            "the CLI flag exists",
            "the test under tests/unit/ passes",
            "the doc row is present",
        ]));
        assert_eq!(
            criteria,
            vec![
                "the CLI flag exists",
                "the test under tests/unit/ passes",
                "the doc row is present",
            ]
        );
    }

    #[test]
    fn a_body_without_the_section_has_no_criteria() {
        assert!(parse_acceptance_criteria("## Goal\n\nJust prose.\n").is_empty());
        assert!(
            parse_acceptance_criteria("## Goal\n\n## Notes\n\n- [ ] not an acceptance item\n")
                .is_empty()
        );
    }

    #[test]
    fn the_section_grammar_follows_the_issue_quality_contract() {
        // Any heading level, any case; the section ends at the next
        // heading; non-checkbox lines are not criteria.
        let body = "## acceptance CRITERIA\n\n- [ ] one\n\n- prose line\n\n- [ ] two\n\n## Files touched\n\n- [ ] not counted\n";
        assert_eq!(parse_acceptance_criteria(body), vec!["one", "two"]);
    }

    #[test]
    fn a_hashed_word_is_not_a_heading() {
        let body = "## Acceptance criteria\n\n- [ ] one\n\n#hashtag line\n\n- [ ] two\n";
        assert_eq!(parse_acceptance_criteria(body), vec!["one", "two"]);
    }

    // --- parse_criterion_statuses ------------------------------------

    #[test]
    fn parses_the_gate_shape_with_comma_separators() {
        let parsed = parse_criterion_statuses(
            "status=PASS build_rc=0 test_rc=0 criteria=met,partial,met,deferred",
        )
        .unwrap();
        assert_eq!(
            parsed,
            Some(statuses(&["met", "partial", "met", "deferred"]))
        );
    }

    #[test]
    fn parses_the_fleet_shape_one_key_per_line() {
        let content = "status: PASS\nbuild_rc: 0\ncriteria: met partial deferred met\n";
        let parsed = parse_criterion_statuses(content).unwrap();
        assert_eq!(
            parsed,
            Some(statuses(&["met", "partial", "deferred", "met"]))
        );
    }

    #[test]
    fn an_absent_criteria_key_is_not_an_assessment() {
        let parsed = parse_criterion_statuses("status=PASS build_rc=0 test_rc=0").unwrap();
        assert!(parsed.is_none());
    }

    #[test]
    fn a_token_the_gate_cannot_read_fails_the_report_naming_the_line() {
        let error = parse_criterion_statuses("status=PASS\ncriteria: met,done,met").unwrap_err();
        assert!(error.contains("line 2"), "{error}");
        assert!(error.contains("`done`"), "{error}");
    }

    #[test]
    fn an_empty_recorded_value_fails_the_report() {
        let error = parse_criterion_statuses("criteria:").unwrap_err();
        assert!(error.contains("line 1"), "{error}");
        assert!(error.contains("empty"), "{error}");
    }

    #[test]
    fn the_tokens_are_case_insensitive_on_read_back() {
        let parsed = parse_criterion_statuses("criteria: Met,PARTIAL,Deferred").unwrap();
        assert_eq!(parsed, Some(statuses(&["met", "partial", "deferred"])));
    }

    // --- assess -------------------------------------------------------

    #[test]
    fn no_criteria_is_not_required() {
        assert_eq!(assess(&[], &statuses(&["met"])), Verdict::NotRequired);
    }

    #[test]
    fn an_all_met_report_is_complete() {
        let criteria = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(
            assess(&criteria, &statuses(&["met", "met", "met"])),
            Verdict::Complete
        );
    }

    #[test]
    fn a_partially_met_report_names_the_remainder_by_index() {
        let criteria: Vec<String> = (1..=4).map(|index| format!("criterion {index}")).collect();
        assert_eq!(
            assess(&criteria, &statuses(&["met", "partial", "deferred", "met"])),
            Verdict::PartiallyMet {
                remainder: vec![2, 3]
            }
        );
    }

    #[test]
    fn a_short_report_leaves_the_tail_unassessed() {
        let criteria: Vec<String> = (1..=4).map(|index| format!("criterion {index}")).collect();
        assert_eq!(
            assess(&criteria, &statuses(&["met", "met"])),
            Verdict::Unassessed {
                missing: vec![3, 4]
            }
        );
    }

    #[test]
    fn a_long_report_is_malformed_and_unassessed() {
        let criteria = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            assess(&criteria, &statuses(&["met", "met", "met"])),
            Verdict::Unassessed { missing: vec![] }
        );
    }

    #[test]
    fn no_status_at_all_is_unassessed_with_every_criterion_named() {
        let criteria: Vec<String> = (1..=4).map(|index| format!("criterion {index}")).collect();
        assert_eq!(
            assess(&criteria, &[]),
            Verdict::Unassessed {
                missing: vec![1, 2, 3, 4]
            }
        );
    }

    // --- assess_report -------------------------------------------------

    #[test]
    fn a_spec_with_criteria_and_a_report_without_them_is_unassessed() {
        let verdict = assess_report(&issue_body(&["a", "b"]), "status=PASS build_rc=0").unwrap();
        assert_eq!(
            verdict,
            Verdict::Unassessed {
                missing: vec![1, 2]
            }
        );
    }

    #[test]
    fn a_spec_without_criteria_is_not_required_regardless_of_the_report() {
        let body = "## Goal\n\nJust prose.\n";
        assert_eq!(
            assess_report(body, "status=PASS build_rc=0 criteria=met").unwrap(),
            Verdict::NotRequired
        );
        assert_eq!(
            assess_report(body, "status=PASS build_rc=0").unwrap(),
            Verdict::NotRequired
        );
    }

    #[test]
    fn a_malformed_report_propagates_the_parse_error() {
        let error = assess_report(&issue_body(&["a"]), "criteria: met,shipped").unwrap_err();
        assert!(error.contains("`shipped`"), "{error}");
    }

    // --- closure_body ---------------------------------------------------

    #[test]
    fn complete_and_not_required_bodies_carry_the_close_keyword() {
        for verdict in [Verdict::NotRequired, Verdict::Complete] {
            let body = closure_body(&verdict, 42, "converted the patch").unwrap();
            assert!(body.contains("Closes #42"), "{body:?}");
        }
    }

    #[test]
    fn the_partially_met_body_carries_the_marker_not_the_close_keyword() {
        let verdict = Verdict::PartiallyMet {
            remainder: vec![2, 3],
        };
        let body = closure_body(&verdict, 42, "partial sweep").unwrap();
        assert!(has_partial_fix_marker(&body, 42), "{body:?}");
        assert!(!body.contains("Closes #42"), "{body:?}");
        assert!(!body.contains("Fixes #42"), "{body:?}");
        assert!(!body.contains("Resolves #42"), "{body:?}");
    }

    #[test]
    fn an_unassessed_verdict_gets_the_held_for_review_body() {
        let verdict = Verdict::Unassessed {
            missing: vec![3, 4],
        };
        let body = closure_body(&verdict, 42, "converted the patch").unwrap();
        assert!(has_held_for_review_marker(&body, 42), "{body:?}");
        assert!(!body.contains("Closes #42"), "{body:?}");
        assert!(!body.contains("Fixes #42"), "{body:?}");
        assert!(!body.contains("Resolves #42"), "{body:?}");
    }

    #[test]
    fn closes_authorized_tracks_the_verdict() {
        assert!(closes_authorized(&Verdict::Complete, 7));
        assert!(closes_authorized(&Verdict::NotRequired, 7));
        // The partial-fix marker is the authorized alternative, not a
        // close: the tracker will not close the issue on it.
        assert!(closes_authorized(
            &Verdict::PartiallyMet { remainder: vec![1] },
            7
        ));
        // The held-for-review marker is the authorized alternative for
        // unassessed verdicts: the issue stays open but the body is an
        // explicit, detectable decision.
        assert!(closes_authorized(
            &Verdict::Unassessed { missing: vec![1] },
            7
        ));
    }

    // --- follow_up_body --------------------------------------------------

    #[test]
    fn the_follow_up_carries_the_remainder_as_its_own_acceptance_section() {
        let criteria = vec![
            "the CLI flag exists".to_string(),
            "the test under tests/unit/ passes".to_string(),
            "the doc row is present".to_string(),
            "the smoke command passes".to_string(),
        ];
        let verdict = Verdict::PartiallyMet {
            remainder: vec![2, 3, 4],
        };
        let body =
            follow_up_body(310, &criteria, &verdict).expect("partial verdict has a follow-up");
        assert!(body.contains("#310"), "{body:?}");
        let re_parsed = parse_acceptance_criteria(&body);
        assert_eq!(
            re_parsed,
            vec![
                "the test under tests/unit/ passes",
                "the doc row is present",
                "the smoke command passes",
            ]
        );
    }

    #[test]
    fn non_partial_verdicts_have_no_follow_up() {
        let criteria = vec!["a".to_string()];
        assert!(follow_up_body(1, &criteria, &Verdict::Complete).is_none());
        assert!(follow_up_body(1, &criteria, &Verdict::NotRequired).is_none());
        assert!(follow_up_body(1, &criteria, &Verdict::Unassessed { missing: vec![1] }).is_none());
    }

    // --- verdict_line ------------------------------------------------------

    #[test]
    fn the_verdict_line_names_the_verdict_and_the_criteria_by_index() {
        assert_eq!(
            verdict_line(&Verdict::NotRequired, 0),
            "ACCEPTANCE: not required — the issue declares no acceptance criteria"
        );
        assert_eq!(
            verdict_line(&Verdict::Complete, 3),
            "ACCEPTANCE: 3/3 criteria met — close authorized"
        );
        assert_eq!(
            verdict_line(&Verdict::PartiallyMet { remainder: vec![2, 3, 4] }, 4),
            "ACCEPTANCE: 1/4 criteria met; remainder 2, 3, 4 — close only with a follow-up filing the remainder"
        );
        assert_eq!(
            verdict_line(
                &Verdict::Unassessed {
                    missing: vec![1, 2, 3]
                },
                4
            ),
            "ACCEPTANCE: 1/4 criteria assessed; unassessed 1, 2, 3 — do not close"
        );
    }

    #[test]
    fn the_verdict_line_names_a_malformed_long_report_as_unassessed() {
        let line = verdict_line(&Verdict::Unassessed { missing: vec![] }, 2);
        assert!(line.contains("all"), "{line}");
        assert!(line.contains("do not close"), "{line}");
    }
}
