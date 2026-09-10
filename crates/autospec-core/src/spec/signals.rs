//! Consumed-signal semantics for spec documents and rejection decisions.
//!
//! A signal is any measured value a decision consumes — a count, an exit
//! code, a file's presence, a suite's verdict. A signal is uninterpretable
//! until the spec states what an empty or absent reading means, what the
//! value is when the population is healthy, and under what conditions it
//! was produced. The canonical failure this guards is the
//! `cmd 2>/dev/null | wc -l` shape: the error and the zero result are the
//! same value, so a decision made on that count cannot be reviewed.
//!
//! Two contracts are enforced here:
//!
//! 1. A spec that consumes signals declares them under `## Consumed Signals`;
//!    each `### <signal>` entry states `Empty/absent means:`,
//!    `Healthy value:`, and `Produced under:`. [`lint_consumed_signals`]
//!    returns one blocking finding per missing or empty declaration, naming
//!    the field.
//! 2. A rejection decision (patch retirement, issue close, run cancellation)
//!    cites the signal it used and that signal's healthy-population value.
//!    [`validate_rejection`] refuses a rejection whose cited signal was never
//!    checked against the base rate, naming the missing check.

use crate::spec::parser::section_lines;

/// Section heading that declares the signals a spec consumes.
pub const CONSUMED_SIGNALS_SECTION: &str = "Consumed Signals";

/// Rule ID emitted for every finding produced by [`lint_consumed_signals`].
pub const CONSUMED_SIGNAL_INCOMPLETE_RULE_ID: &str = "CONSUMED_SIGNAL_INCOMPLETE";

/// The three declarations every consumed signal must carry, in order.
pub const SIGNAL_FIELDS: &[&str] = &["Empty/absent means:", "Healthy value:", "Produced under:"];

/// One missing or empty declaration in a consumed-signal entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalSemanticsFinding {
    /// 1-based line number of the signal's `###` heading.
    pub line: usize,
    /// Name of the consumed signal the finding applies to.
    pub signal: String,
    /// The declaration that is missing or empty, including the colon
    /// (one of [`SIGNAL_FIELDS`]).
    pub missing_field: &'static str,
}

impl SignalSemanticsFinding {
    /// Stable rule identifier for the finding.
    pub const fn rule_id(&self) -> &'static str {
        CONSUMED_SIGNAL_INCOMPLETE_RULE_ID
    }

    /// Human-readable diagnostic for the finding.
    pub fn message(&self) -> String {
        format!(
            "line {}: consumed signal \"{}\" omits {} — declare it before the spec can be reviewed",
            self.line, self.signal, self.missing_field
        )
    }
}

/// Lint the `## Consumed Signals` section of a spec and return one finding
/// per missing or empty declaration, in line order then field order.
///
/// A spec without a `## Consumed Signals` section consumes no declared
/// signals and returns an empty list. Each `### <name>` entry inside the
/// section is one signal; the three required declarations may appear as
/// plain lines or `- ` bullets, backticks around the signal name are
/// stripped, and a declaration with nothing after the colon counts as
/// missing.
pub fn lint_consumed_signals(source: &str) -> Vec<SignalSemanticsFinding> {
    let Some(section) = section_lines(source, CONSUMED_SIGNALS_SECTION) else {
        return Vec::new();
    };

    let mut findings = Vec::new();
    let mut current: Option<(usize, String, Vec<String>)> = None;

    for (line_no, raw) in &section {
        let trimmed = raw.trim();
        if let Some(name) = trimmed.strip_prefix("### ") {
            if let Some((line, name, body)) = current.take() {
                findings.extend(check_signal_entry(line, &name, &body));
            }
            current = Some((*line_no, signal_name(name), Vec::new()));
        } else if let Some(entry) = current.as_mut() {
            entry.2.push(trimmed.to_string());
        }
    }
    if let Some((line, name, body)) = current.take() {
        findings.extend(check_signal_entry(line, &name, &body));
    }

    findings
}

fn signal_name(raw: &str) -> String {
    raw.trim().trim_matches('`').trim().to_string()
}

fn check_signal_entry(
    heading_line: usize,
    name: &str,
    body: &[String],
) -> Vec<SignalSemanticsFinding> {
    let mut findings = Vec::new();
    for field in SIGNAL_FIELDS {
        if !declares_field(body, field) {
            findings.push(SignalSemanticsFinding {
                line: heading_line,
                signal: name.to_string(),
                missing_field: field,
            });
        }
    }
    findings
}

fn declares_field(body: &[String], field: &str) -> bool {
    body.iter().any(|line| {
        let stripped = line.strip_prefix("- ").or_else(|| line.strip_prefix("* "));
        let content = stripped.unwrap_or(line.as_str());
        content
            .strip_prefix(field)
            .is_some_and(|rest| !rest.trim().is_empty())
    })
}

/// One signal cited by a rejection decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CitedSignal {
    /// Name of the signal the rejection was based on.
    pub name: String,
    /// The signal's healthy-population (base-rate) value, if checked.
    pub healthy_population_value: Option<String>,
}

/// A decision that discards or rejects work — patch retirement, issue close,
/// run cancellation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    /// What was rejected (e.g. "patch 3", "issue 4102", "run 17").
    pub target: String,
    /// The signals the decision was based on, with their base-rate values
    /// when they were checked.
    pub cited_signals: Vec<CitedSignal>,
}

/// The missing check that blocks a rejection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectionRefusal {
    /// Name of the cited signal the refusal applies to (empty when the
    /// rejection cited no signal at all).
    pub signal: String,
    /// The check that must be performed before the rejection stands.
    pub missing_check: &'static str,
}

impl RejectionRefusal {
    /// Human-readable refusal: names the signal and the missing check.
    pub fn message(&self) -> String {
        if self.signal.is_empty() {
            "rejection refused: no signal cited — cite the signal used and its healthy-population value before the decision stands"
                .to_string()
        } else {
            format!(
                "rejection refused: signal \"{}\" cited without its healthy-population value — check the base rate before rejecting",
                self.signal
            )
        }
    }
}

/// Validate that a rejection cites a signal checked against the
/// healthy population.
///
/// A rejection with no cited signal is refused outright: a rejection is a
/// claim that something is broken, and a claim needs its evidence named. A
/// rejection whose cited signal has no (non-empty) healthy-population value
/// is refused with the signal name and the missing check named. The first
/// failing signal is reported.
pub fn validate_rejection(rejection: &Rejection) -> Result<(), RejectionRefusal> {
    if rejection.cited_signals.is_empty() {
        return Err(RejectionRefusal {
            signal: String::new(),
            missing_check: "cited signal",
        });
    }
    for cited in &rejection.cited_signals {
        let checked = cited
            .healthy_population_value
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
        if !checked {
            return Err(RejectionRefusal {
                signal: cited.name.clone(),
                missing_check: "healthy-population value",
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_with_signals(signals: &str) -> String {
        format!("# Test spec\n\n## Consumed Signals\n\n{signals}\n")
    }

    fn full_signal() -> &'static str {
        "### `ci_failures`\n- Empty/absent means: the suite did not run\n- Healthy value: 0 on green main\n- Produced under: suite green at HEAD\n"
    }

    #[test]
    fn spec_without_signal_section_has_no_findings() {
        assert!(lint_consumed_signals("# Spec\n\n## Objective\n\nDo the thing.\n").is_empty());
    }

    #[test]
    fn complete_signal_declaration_has_no_findings() {
        assert!(lint_consumed_signals(&spec_with_signals(full_signal())).is_empty());
    }

    #[test]
    fn each_missing_field_is_a_finding_named_by_field() {
        let spec = spec_with_signals("### `ci_failures`\n- Healthy value: 0 on green main\n");
        let findings = lint_consumed_signals(&spec);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].rule_id(), CONSUMED_SIGNAL_INCOMPLETE_RULE_ID);
        assert_eq!(findings[0].signal, "ci_failures");
        assert_eq!(findings[0].missing_field, "Empty/absent means:");
        assert_eq!(findings[1].missing_field, "Produced under:");
        assert!(findings[0].message().contains("Empty/absent means:"));
    }

    #[test]
    fn declaration_with_empty_value_counts_as_missing() {
        let spec = spec_with_signals(
            "### `ci_failures`\n- Empty/absent means: the suite did not run\n- Healthy value:\n- Produced under: suite green at HEAD\n",
        );
        let findings = lint_consumed_signals(&spec);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].missing_field, "Healthy value:");
    }

    #[test]
    fn findings_carry_the_signal_heading_line() {
        let spec = spec_with_signals("### exit_code\n- Healthy value: 0\n");
        let findings = lint_consumed_signals(&spec);
        // `# Test spec`, blank, `## Consumed Signals`, blank, `### exit_code`.
        assert!(findings.iter().all(|f| f.line == 5));
    }

    #[test]
    fn only_the_incomplete_entries_are_flagged() {
        let spec = spec_with_signals(&format!(
            "{}\n### exit_code\n- Empty/absent means: no run\n",
            full_signal()
        ));
        let findings = lint_consumed_signals(&spec);
        assert_eq!(findings.len(), 2);
        assert!(findings.iter().all(|f| f.signal == "exit_code"));
    }

    #[test]
    fn plain_lines_count_as_declarations() {
        let spec = spec_with_signals(
            "### exit_code\nEmpty/absent means: no run\nHealthy value: 0\nProduced under: release run\n",
        );
        assert!(lint_consumed_signals(&spec).is_empty());
    }

    #[test]
    fn rejection_without_any_cited_signal_is_refused() {
        let rejection = Rejection {
            target: "patch 3".into(),
            cited_signals: Vec::new(),
        };
        let refusal = validate_rejection(&rejection).unwrap_err();
        assert_eq!(refusal.signal, "");
        assert_eq!(refusal.missing_check, "cited signal");
        assert!(refusal.message().contains("no signal cited"));
    }

    #[test]
    fn rejection_citing_unchecked_signal_is_refused_naming_the_check() {
        let rejection = Rejection {
            target: "issue 4102".into(),
            cited_signals: vec![CitedSignal {
                name: "ci_failures".into(),
                healthy_population_value: None,
            }],
        };
        let refusal = validate_rejection(&rejection).unwrap_err();
        assert_eq!(refusal.signal, "ci_failures");
        assert_eq!(refusal.missing_check, "healthy-population value");
        let message = refusal.message();
        assert!(message.contains("ci_failures"));
        assert!(message.contains("healthy-population value"));
    }

    #[test]
    fn empty_string_base_rate_counts_as_unchecked() {
        let rejection = Rejection {
            target: "run 17".into(),
            cited_signals: vec![CitedSignal {
                name: "suite_verdict".into(),
                healthy_population_value: Some("   ".into()),
            }],
        };
        assert!(validate_rejection(&rejection).is_err());
    }

    #[test]
    fn rejection_with_checked_signal_passes() {
        let rejection = Rejection {
            target: "issue 4102".into(),
            cited_signals: vec![
                CitedSignal {
                    name: "ci_failures".into(),
                    healthy_population_value: Some("0 on green main".into()),
                },
                CitedSignal {
                    name: "open_prs".into(),
                    healthy_population_value: Some("3".into()),
                },
            ],
        };
        assert!(validate_rejection(&rejection).is_ok());
    }
}
