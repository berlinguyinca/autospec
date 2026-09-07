//! Configurable quality-balance policy. Parsed from the repository-owned
//! `.autospec/autonomous.yml` `quality_balance:` block with the same strict,
//! intentional non-generic discipline as `main_health` and `tier4`.

use std::collections::BTreeSet;

use super::{FindingConfidence, FindingSeverity};

pub const MIN_QUALITY_SHARE_BPS: u64 = 1;
pub const MAX_QUALITY_SHARE_BPS: u64 = 10_000;
pub const MIN_LOOP_BUDGET: u64 = 1;
pub const MAX_LOOP_BUDGET: u64 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityBalancePolicy {
    /// Minimum quality share (basis points of recorded work) below which the
    /// idle loop forces quality remediation before feature discovery.
    pub min_quality_share_bps: u64,
    /// Maximum audit passes per idle cycle (first audit included).
    pub max_reaudits: u64,
    /// Maximum remediation rounds per idle cycle.
    pub max_remediation_rounds: u64,
    /// Maximum remediation attempts per finding before the loop gives up.
    pub max_attempts_per_finding: u64,
    /// Minimum observed-evidence confidence for a finding to block the gate.
    pub blocking_min_confidence: FindingConfidence,
    /// Severities that block the quality gate while unremediated.
    pub blocking_severities: BTreeSet<FindingSeverity>,
}

impl Default for QualityBalancePolicy {
    fn default() -> Self {
        Self {
            min_quality_share_bps: 2_500,
            max_reaudits: 3,
            max_remediation_rounds: 5,
            max_attempts_per_finding: 3,
            blocking_min_confidence: FindingConfidence::High,
            blocking_severities: BTreeSet::from([FindingSeverity::Critical]),
        }
    }
}

impl QualityBalancePolicy {
    pub fn validate(&self) -> Result<(), String> {
        if !(MIN_QUALITY_SHARE_BPS..=MAX_QUALITY_SHARE_BPS).contains(&self.min_quality_share_bps) {
            return Err(format!(
                "quality_balance.min_quality_share_bps must be between {MIN_QUALITY_SHARE_BPS} and {MAX_QUALITY_SHARE_BPS}"
            ));
        }
        if !budget_range(self.max_reaudits) {
            return Err(format!(
                "quality_balance.max_reaudits must be between {MIN_LOOP_BUDGET} and {MAX_LOOP_BUDGET}"
            ));
        }
        if !budget_range(self.max_remediation_rounds) {
            return Err(format!(
                "quality_balance.max_remediation_rounds must be between {MIN_LOOP_BUDGET} and {MAX_LOOP_BUDGET}"
            ));
        }
        if !budget_range(self.max_attempts_per_finding) {
            return Err(format!(
                "quality_balance.max_attempts_per_finding must be between {MIN_LOOP_BUDGET} and {MAX_LOOP_BUDGET}"
            ));
        }
        if self.blocking_severities.is_empty() {
            return Err("quality_balance.blocking_severities must not be empty".to_string());
        }
        Ok(())
    }
}

fn budget_range(value: u64) -> bool {
    (MIN_LOOP_BUDGET..=MAX_LOOP_BUDGET).contains(&value)
}

/// Parses the `quality_balance:` block when present; an absent block yields
/// the default policy so the idle loop is on by default.
pub fn parse_policy(source: &str) -> Result<QualityBalancePolicy, String> {
    let mut policy = QualityBalancePolicy::default();
    let mut in_quality_balance = false;
    let mut saw_quality_balance = false;
    let mut list_open = false;
    let mut severities = BTreeSet::new();
    let mut severities_open = false;
    let mut saw_min_quality_share_bps = false;
    let mut saw_max_reaudits = false;
    let mut saw_max_remediation_rounds = false;
    let mut saw_max_attempts_per_finding = false;
    let mut saw_blocking_min_confidence = false;
    let mut saw_blocking_severities = false;

    for (index, raw_line) in source.lines().enumerate() {
        let line_number = index + 1;
        let line = strip_comment(raw_line).trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if raw_line
            .chars()
            .take_while(|character| character.is_whitespace())
            .any(|character| character == '\t')
            && in_quality_balance
        {
            return Err(error(
                line_number,
                "tabs are not valid indentation in quality_balance",
            ));
        }
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();

        if indent == 0 {
            in_quality_balance = false;
            list_open = false;
            severities_open = false;
            if trimmed == "quality_balance:" {
                if saw_quality_balance {
                    return Err(error(line_number, "duplicate quality_balance block"));
                }
                saw_quality_balance = true;
                in_quality_balance = true;
                continue;
            }
            if trimmed.starts_with("quality_balance") {
                return Err(error(line_number, "quality_balance must be a mapping"));
            }
            continue;
        }
        if !in_quality_balance {
            continue;
        }

        if indent == 2 {
            list_open = false;
            severities_open = false;
            let Some((key, value)) = trimmed.split_once(':') else {
                return Err(error(
                    line_number,
                    "quality_balance entry must use key: value syntax",
                ));
            };
            let key = key.trim();
            let value = value.trim();
            match key {
                "min_quality_share_bps" => {
                    if saw_min_quality_share_bps {
                        return Err(error(
                            line_number,
                            "duplicate quality_balance.min_quality_share_bps",
                        ));
                    }
                    saw_min_quality_share_bps = true;
                    policy.min_quality_share_bps = parse_bounded_u64(
                        value,
                        MIN_QUALITY_SHARE_BPS,
                        MAX_QUALITY_SHARE_BPS,
                        line_number,
                        "quality_balance.min_quality_share_bps",
                    )?;
                }
                "max_reaudits" => {
                    if saw_max_reaudits {
                        return Err(error(line_number, "duplicate quality_balance.max_reaudits"));
                    }
                    saw_max_reaudits = true;
                    policy.max_reaudits = parse_bounded_u64(
                        value,
                        MIN_LOOP_BUDGET,
                        MAX_LOOP_BUDGET,
                        line_number,
                        "quality_balance.max_reaudits",
                    )?;
                }
                "max_remediation_rounds" => {
                    if saw_max_remediation_rounds {
                        return Err(error(
                            line_number,
                            "duplicate quality_balance.max_remediation_rounds",
                        ));
                    }
                    saw_max_remediation_rounds = true;
                    policy.max_remediation_rounds = parse_bounded_u64(
                        value,
                        MIN_LOOP_BUDGET,
                        MAX_LOOP_BUDGET,
                        line_number,
                        "quality_balance.max_remediation_rounds",
                    )?;
                }
                "max_attempts_per_finding" => {
                    if saw_max_attempts_per_finding {
                        return Err(error(
                            line_number,
                            "duplicate quality_balance.max_attempts_per_finding",
                        ));
                    }
                    saw_max_attempts_per_finding = true;
                    policy.max_attempts_per_finding = parse_bounded_u64(
                        value,
                        MIN_LOOP_BUDGET,
                        MAX_LOOP_BUDGET,
                        line_number,
                        "quality_balance.max_attempts_per_finding",
                    )?;
                }
                "blocking_min_confidence" => {
                    if saw_blocking_min_confidence {
                        return Err(error(
                            line_number,
                            "duplicate quality_balance.blocking_min_confidence",
                        ));
                    }
                    saw_blocking_min_confidence = true;
                    policy.blocking_min_confidence = FindingConfidence::parse(value)
                        .map_err(|message| error(line_number, &message))?;
                }
                "blocking_severities" => {
                    if saw_blocking_severities {
                        return Err(error(
                            line_number,
                            "duplicate quality_balance.blocking_severities",
                        ));
                    }
                    if !value.is_empty() {
                        return Err(error(
                            line_number,
                            "quality_balance.blocking_severities must be a block list",
                        ));
                    }
                    saw_blocking_severities = true;
                    severities_open = true;
                    list_open = true;
                }
                field => {
                    return Err(error(
                        line_number,
                        &format!("unknown quality_balance field `{field}`"),
                    ));
                }
            }
            continue;
        }

        if indent == 4 && list_open && severities_open {
            let Some(value) = trimmed.strip_prefix('-') else {
                return Err(error(
                    line_number,
                    "quality_balance.blocking_severities entries must start with -",
                ));
            };
            if value.is_empty() || !value.starts_with(char::is_whitespace) {
                return Err(error(
                    line_number,
                    "quality_balance.blocking_severities entries must be scalar values",
                ));
            }
            let value = value.trim();
            let severity =
                FindingSeverity::parse(value).map_err(|message| error(line_number, &message))?;
            if !severities.insert(severity) {
                return Err(error(
                    line_number,
                    &format!("duplicate quality_balance.blocking_severities value `{value}`"),
                ));
            }
            continue;
        }

        return Err(error(
            line_number,
            "malformed indentation or nested value in quality_balance",
        ));
    }

    if severities_open && severities.is_empty() {
        return Err(error(
            source.lines().count(),
            "quality_balance.blocking_severities must list at least one severity",
        ));
    }
    if saw_blocking_severities {
        policy.blocking_severities = severities;
    }
    policy.validate()?;
    Ok(policy)
}

fn parse_bounded_u64(
    value: &str,
    min: u64,
    max: u64,
    line_number: usize,
    field: &str,
) -> Result<u64, String> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(error(
            line_number,
            &format!("{field} must be an unsigned decimal integer"),
        ));
    }
    let parsed = value.parse::<u64>().map_err(|_| {
        error(
            line_number,
            &format!("{field} is outside the supported range"),
        )
    })?;
    if !(min..=max).contains(&parsed) {
        return Err(error(
            line_number,
            &format!("{field} must be between {min} and {max}"),
        ));
    }
    Ok(parsed)
}

fn strip_comment(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote == Some('\"') {
            escaped = true;
            continue;
        }
        if matches!(character, '\'' | '\"') {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
        } else if character == '#' && quote.is_none() {
            return &line[..index];
        }
    }
    line
}

fn error(line_number: usize, message: &str) -> String {
    format!("invalid .autospec/autonomous.yml at line {line_number}: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_block_yields_defaults() {
        let policy = parse_policy("main_health:\n  branch: main\n").unwrap();
        assert_eq!(policy, QualityBalancePolicy::default());
    }

    #[test]
    fn parses_a_full_block() {
        let source = "quality_balance:\n  min_quality_share_bps: 4000\n  max_reaudits: 4\n  \
                  max_remediation_rounds: 6\n  max_attempts_per_finding: 2\n  blocking_min_confidence: \
                  medium\n  blocking_severities:\n    - critical\n    - high\n";
        let policy = parse_policy(source).unwrap();
        assert_eq!(policy.min_quality_share_bps, 4_000);
        assert_eq!(policy.max_reaudits, 4);
        assert_eq!(policy.max_remediation_rounds, 6);
        assert_eq!(policy.max_attempts_per_finding, 2);
        assert_eq!(policy.blocking_min_confidence, FindingConfidence::Medium);
        assert_eq!(
            policy.blocking_severities,
            BTreeSet::from([FindingSeverity::Critical, FindingSeverity::High])
        );
    }

    #[test]
    fn partial_block_keeps_defaults_for_absent_fields() {
        let policy = parse_policy("quality_balance:\n  max_reaudits: 2\n").unwrap();
        let expected = QualityBalancePolicy {
            max_reaudits: 2,
            ..QualityBalancePolicy::default()
        };
        assert_eq!(policy, expected);
    }

    #[test]
    fn rejects_duplicate_keys_and_unknown_fields() {
        assert!(parse_policy("quality_balance:\n  max_reaudits: 2\n  max_reaudits: 3\n").is_err());
        assert!(parse_policy(
            "quality_balance:\n  min_quality_share_bps: 1\n  min_quality_share_bps: 2\n"
        )
        .is_err());
        assert!(parse_policy("quality_balance:\n  unknown_field: 1\n").is_err());
        assert!(parse_policy(
            "quality_balance:\n  max_reaudits: 1\nquality_balance:\n  max_reaudits: 2\n"
        )
        .is_err());
        assert!(parse_policy("quality_balance: scalar\n").is_err());
    }

    #[test]
    fn rejects_out_of_range_and_non_numeric_values() {
        assert!(parse_policy("quality_balance:\n  min_quality_share_bps: 0\n").is_err());
        assert!(parse_policy("quality_balance:\n  min_quality_share_bps: 10001\n").is_err());
        assert!(parse_policy("quality_balance:\n  max_reaudits: 0\n").is_err());
        assert!(parse_policy("quality_balance:\n  max_reaudits: 11\n").is_err());
        assert!(parse_policy("quality_balance:\n  max_remediation_rounds: -1\n").is_err());
        assert!(parse_policy("quality_balance:\n  max_attempts_per_finding: many\n").is_err());
        assert!(parse_policy("quality_balance:\n  blocking_min_confidence: certain\n").is_err());
    }

    #[test]
    fn rejects_empty_or_duplicate_severity_lists() {
        assert!(parse_policy("quality_balance:\n  blocking_severities:\n").is_err());
        assert!(parse_policy(
            "quality_balance:\n  blocking_severities:\n    - critical\n    - critical\n"
        )
        .is_err());
        assert!(
            parse_policy("quality_balance:\n  blocking_severities:\n    - legendary\n").is_err()
        );
        assert!(parse_policy("quality_balance:\n  blocking_severities: [critical]\n").is_err());
    }

    #[test]
    fn comments_and_unrelated_blocks_are_ignored() {
        let source = "# top comment\nmain_health:\n  branch: main\nquality_balance: # balance\n  \
                  min_quality_share_bps: 1200 # floor\n  max_reaudits: 1\n";
        let policy = parse_policy(source).unwrap();
        assert_eq!(policy.min_quality_share_bps, 1_200);
        assert_eq!(policy.max_reaudits, 1);
    }
}
