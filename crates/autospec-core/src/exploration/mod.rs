use std::collections::{BTreeMap, BTreeSet};

use crate::state::json::{JsonParser, JsonValue};

const ARCHIVED_PENALTY: i64 = 1_000_000;
const REVIVAL_REQUESTED_BONUS: i64 = 500;
const PUSH_RECENCY_WEIGHT: i64 = 100;
const README_WEIGHT: i64 = 10;
const MODULE_PATH_WEIGHT: i64 = 5;
const PACKAGE_WEIGHT: i64 = 3;
const INBOUND_DEPENDENCY_WEIGHT: i64 = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplorationInput {
    pub repositories: Vec<RepositoryEvidence>,
    pub findings: Vec<Finding>,
}

impl ExplorationInput {
    pub fn from_json(document: &str) -> Result<Self, String> {
        let mut object = JsonParser::new(document)
            .parse()?
            .into_object("repository exploration input")?;
        reject_unknown_keys(
            &object,
            &["repositories", "findings"],
            "repository exploration input",
        )?;
        let repositories =
            take_required(&mut object, "repositories", "repository exploration input")?
                .into_array("repository exploration input.repositories")?
                .into_iter()
                .enumerate()
                .map(|(index, value)| {
                    parse_repository(
                        value,
                        &format!("repository exploration input.repositories[{index}]"),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
        let findings = take_required(&mut object, "findings", "repository exploration input")?
            .into_array("repository exploration input.findings")?
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                parse_finding(
                    value,
                    &format!("repository exploration input.findings[{index}]"),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let input = Self {
            repositories,
            findings,
        };
        validate_input(&input)?;
        Ok(input)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryEvidence {
    pub name: String,
    pub family: String,
    pub archived: bool,
    pub revival_requested: bool,
    pub pushed_at: String,
    pub readme: String,
    pub module_paths: Vec<String>,
    pub packages: Vec<String>,
    pub dependency_references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub repository: String,
    pub fingerprint: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalTarget {
    pub family: String,
    pub repository: String,
    pub score: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutedFinding {
    pub repository: String,
    pub fingerprint: String,
    pub title: String,
    pub canonical_target: String,
    pub duplicate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferredFinding {
    pub repository: String,
    pub fingerprint: String,
    pub title: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplorationReport {
    pub canonical_targets: Vec<CanonicalTarget>,
    pub do_not_file_by_default: Vec<String>,
    pub routed_findings: Vec<RoutedFinding>,
    pub deferred_findings: Vec<DeferredFinding>,
}

impl ExplorationReport {
    pub fn to_json(&self) -> String {
        let canonical_targets = self
            .canonical_targets
            .iter()
            .map(|target| {
                format!(
                    "{{\"family\":\"{}\",\"repository\":\"{}\",\"score\":{}}}",
                    escape_json(&target.family),
                    escape_json(&target.repository),
                    target.score
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let routed_findings = self
            .routed_findings
            .iter()
            .map(|finding| {
                format!(
                    "{{\"repository\":\"{}\",\"fingerprint\":\"{}\",\"title\":\"{}\",\"canonical_target\":\"{}\",\"duplicate\":{}}}",
                    escape_json(&finding.repository),
                    escape_json(&finding.fingerprint),
                    escape_json(&finding.title),
                    escape_json(&finding.canonical_target),
                    finding.duplicate
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let deferred_findings = self
            .deferred_findings
            .iter()
            .map(|finding| {
                format!(
                    "{{\"repository\":\"{}\",\"fingerprint\":\"{}\",\"title\":\"{}\",\"reason\":\"{}\"}}",
                    escape_json(&finding.repository),
                    escape_json(&finding.fingerprint),
                    escape_json(&finding.title),
                    escape_json(&finding.reason)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"canonical_targets\":[{canonical_targets}],\"do_not_file_by_default\":{},\"routed_findings\":[{routed_findings}],\"deferred_findings\":[{deferred_findings}]}}",
            json_array(&self.do_not_file_by_default)
        )
    }
}

pub fn route_repositories(input: &ExplorationInput) -> Result<ExplorationReport, String> {
    validate_input(input)?;
    let pushed_at = input
        .repositories
        .iter()
        .map(|repository| {
            Ok((
                repository.name.as_str(),
                parse_timestamp(&repository.pushed_at)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let repositories_by_name = input
        .repositories
        .iter()
        .map(|repository| (repository.name.as_str(), repository))
        .collect::<BTreeMap<_, _>>();
    let mut families = BTreeMap::<String, Vec<&RepositoryEvidence>>::new();
    for repository in &input.repositories {
        families
            .entry(repository.family.clone())
            .or_default()
            .push(repository);
    }

    let do_not_file_by_default = input
        .repositories
        .iter()
        .filter(|repository| repository.archived && !repository.revival_requested)
        .map(|repository| repository.name.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut canonical_targets = Vec::new();
    let mut canonical_by_family = BTreeMap::new();
    for (family, repositories) in &families {
        let timestamps = repositories
            .iter()
            .map(|repository| pushed_at[repository.name.as_str()])
            .collect::<BTreeSet<_>>();
        let mut candidates = repositories
            .iter()
            .filter(|repository| !repository.archived || repository.revival_requested)
            .map(|repository| {
                let push_rank = timestamps
                    .range(..pushed_at[repository.name.as_str()])
                    .count();
                let inbound_references = input
                    .repositories
                    .iter()
                    .filter(|candidate| {
                        candidate
                            .dependency_references
                            .iter()
                            .any(|reference| reference == &repository.name)
                    })
                    .count();
                (
                    *repository,
                    score(repository, push_rank, inbound_references),
                )
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| left.0.name.cmp(&right.0.name))
        });
        if let Some((repository, score)) = candidates.into_iter().next() {
            canonical_targets.push(CanonicalTarget {
                family: family.clone(),
                repository: repository.name.clone(),
                score,
            });
            canonical_by_family.insert(family.as_str(), repository.name.clone());
        }
    }

    let mut seen = BTreeSet::new();
    let mut routed_findings = Vec::new();
    let mut deferred_findings = Vec::new();
    for finding in &input.findings {
        let repository = repositories_by_name[finding.repository.as_str()];
        match canonical_by_family.get(repository.family.as_str()) {
            Some(canonical_target) => {
                let duplicate =
                    !seen.insert((canonical_target.clone(), finding.fingerprint.clone()));
                routed_findings.push(RoutedFinding {
                    repository: finding.repository.clone(),
                    fingerprint: finding.fingerprint.clone(),
                    title: finding.title.clone(),
                    canonical_target: canonical_target.clone(),
                    duplicate,
                });
            }
            None => deferred_findings.push(DeferredFinding {
                repository: finding.repository.clone(),
                fingerprint: finding.fingerprint.clone(),
                title: finding.title.clone(),
                reason: "no_eligible_canonical_target".to_string(),
            }),
        }
    }

    Ok(ExplorationReport {
        canonical_targets,
        do_not_file_by_default,
        routed_findings,
        deferred_findings,
    })
}

fn parse_repository(value: JsonValue, context: &str) -> Result<RepositoryEvidence, String> {
    let mut object = value.into_object(context)?;
    reject_unknown_keys(
        &object,
        &[
            "name",
            "family",
            "archived",
            "revival_requested",
            "pushed_at",
            "readme",
            "module_paths",
            "packages",
            "dependency_references",
        ],
        context,
    )?;
    Ok(RepositoryEvidence {
        name: take_required(&mut object, "name", context)?
            .into_string(&format!("{context}.name"))?,
        family: take_required(&mut object, "family", context)?
            .into_string(&format!("{context}.family"))?,
        archived: take_required(&mut object, "archived", context)?
            .into_bool(&format!("{context}.archived"))?,
        revival_requested: take_required(&mut object, "revival_requested", context)?
            .into_bool(&format!("{context}.revival_requested"))?,
        pushed_at: take_required(&mut object, "pushed_at", context)?
            .into_string(&format!("{context}.pushed_at"))?,
        readme: take_required(&mut object, "readme", context)?
            .into_string(&format!("{context}.readme"))?,
        module_paths: parse_string_array(
            take_required(&mut object, "module_paths", context)?,
            &format!("{context}.module_paths"),
        )?,
        packages: parse_string_array(
            take_required(&mut object, "packages", context)?,
            &format!("{context}.packages"),
        )?,
        dependency_references: parse_string_array(
            take_required(&mut object, "dependency_references", context)?,
            &format!("{context}.dependency_references"),
        )?,
    })
}

fn parse_finding(value: JsonValue, context: &str) -> Result<Finding, String> {
    let mut object = value.into_object(context)?;
    reject_unknown_keys(&object, &["repository", "fingerprint", "title"], context)?;
    Ok(Finding {
        repository: take_required(&mut object, "repository", context)?
            .into_string(&format!("{context}.repository"))?,
        fingerprint: take_required(&mut object, "fingerprint", context)?
            .into_string(&format!("{context}.fingerprint"))?,
        title: take_required(&mut object, "title", context)?
            .into_string(&format!("{context}.title"))?,
    })
}

fn parse_string_array(value: JsonValue, context: &str) -> Result<Vec<String>, String> {
    value
        .into_array(context)?
        .into_iter()
        .enumerate()
        .map(|(index, value)| value.into_string(&format!("{context}[{index}]")))
        .collect()
}

fn validate_input(input: &ExplorationInput) -> Result<(), String> {
    let mut names = BTreeSet::new();
    for repository in &input.repositories {
        if repository.name.trim().is_empty() {
            return Err("repository evidence name must not be empty".to_string());
        }
        if repository.family.trim().is_empty() {
            return Err(format!(
                "repository evidence {} family must not be empty",
                repository.name
            ));
        }
        if !names.insert(repository.name.as_str()) {
            return Err(format!("duplicate repository name: {}", repository.name));
        }
        parse_timestamp(&repository.pushed_at).map_err(|error| {
            format!(
                "repository evidence {} has invalid pushed_at: {error}",
                repository.name
            )
        })?;
    }
    for finding in &input.findings {
        if finding.fingerprint.trim().is_empty() {
            return Err(format!(
                "finding for {} has an empty fingerprint",
                finding.repository
            ));
        }
        if !names.contains(finding.repository.as_str()) {
            return Err(format!(
                "unknown finding repository: {}",
                finding.repository
            ));
        }
    }
    Ok(())
}

fn score(repository: &RepositoryEvidence, push_rank: usize, inbound_references: usize) -> i64 {
    let mut score = push_rank as i64 * PUSH_RECENCY_WEIGHT;
    if !repository.readme.trim().is_empty() {
        score += README_WEIGHT;
    }
    score += repository.module_paths.len() as i64 * MODULE_PATH_WEIGHT;
    score += repository.packages.len() as i64 * PACKAGE_WEIGHT;
    score += inbound_references as i64 * INBOUND_DEPENDENCY_WEIGHT;
    if repository.archived {
        score -= ARCHIVED_PENALTY;
    }
    if repository.revival_requested {
        score += REVIVAL_REQUESTED_BONUS;
    }
    score
}

fn parse_timestamp(value: &str) -> Result<i64, String> {
    let bytes = value.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return Err("expected UTC RFC 3339 timestamp YYYY-MM-DDTHH:MM:SSZ".to_string());
    }
    let year = parse_digits(bytes, 0, 4, value)?;
    let month = parse_digits(bytes, 5, 7, value)?;
    let day = parse_digits(bytes, 8, 10, value)?;
    let hour = parse_digits(bytes, 11, 13, value)?;
    let minute = parse_digits(bytes, 14, 16, value)?;
    let second = parse_digits(bytes, 17, 19, value)?;
    if year == 0 || !(1..=12).contains(&month) {
        return Err(format!("invalid calendar date: {value}"));
    }
    let max_day = days_in_month(year, month);
    if day == 0 || day > max_day || hour > 23 || minute > 59 || second > 59 {
        return Err(format!("invalid calendar date: {value}"));
    }
    let years_before = year - 1;
    let days_before_year =
        years_before * 365 + years_before / 4 - years_before / 100 + years_before / 400;
    let days_before_month = (1..month)
        .map(|candidate| days_in_month(year, candidate))
        .sum::<i64>();
    Ok((days_before_year + days_before_month + day - 1) * 86_400
        + hour * 3_600
        + minute * 60
        + second)
}

fn parse_digits(bytes: &[u8], start: usize, end: usize, value: &str) -> Result<i64, String> {
    let digits = &bytes[start..end];
    if !digits.iter().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("invalid UTC RFC 3339 timestamp: {value}"));
    }
    digits.iter().try_fold(0_i64, |number, byte| {
        number
            .checked_mul(10)
            .and_then(|number| number.checked_add((byte - b'0') as i64))
            .ok_or_else(|| format!("timestamp out of range: {value}"))
    })
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn take_required(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<JsonValue, String> {
    object
        .remove(key)
        .ok_or_else(|| format!("{context}.{key} is required"))
}

fn reject_unknown_keys(
    object: &BTreeMap<String, JsonValue>,
    allowed: &[&str],
    context: &str,
) -> Result<(), String> {
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("{context} contains unknown key: {key}"));
    }
    Ok(())
}

fn json_array(values: &[String]) -> String {
    let values = values
        .iter()
        .map(|value| format!("\"{}\"", escape_json(value)))
        .collect::<Vec<_>>()
        .join(",");
    format!("[{values}]")
}

fn escape_json(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            character if character.is_control() => format!("\\u{:04x}", character as u32)
                .chars()
                .collect::<Vec<_>>(),
            character => vec![character],
        })
        .collect()
}
