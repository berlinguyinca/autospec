use std::collections::{BTreeMap, BTreeSet};

use crate::state::json::{JsonParser, JsonValue};

use super::ValidationStatus;

const VALIDATION_REPORT_SCHEMA: u64 = 1;
const VALIDATION_AGGREGATE_SCHEMA: u64 = 1;
const VALIDATION_EXECUTION_SCHEMA: u64 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    pub id: String,
    pub required: bool,
    pub exit_code: Option<i32>,
    pub elapsed_ms: u128,
    pub spawn_count: u32,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub output_digest: String,
}

impl CheckResult {
    #[allow(clippy::too_many_arguments)]
    pub fn completed(
        id: impl Into<String>,
        required: bool,
        exit_code: i32,
        elapsed_ms: u128,
        spawn_count: u32,
        stdout_bytes: usize,
        stderr_bytes: usize,
        output_digest: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            required,
            exit_code: Some(exit_code),
            elapsed_ms,
            spawn_count,
            stdout_bytes,
            stderr_bytes,
            output_digest: output_digest.into(),
        }
    }

    pub fn is_success(&self) -> bool {
        self.exit_code == Some(0)
    }

    pub fn is_failure(&self) -> bool {
        !self.is_success()
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"schema\":{VALIDATION_EXECUTION_SCHEMA},\"id\":\"{}\",\"required\":{},\"exit_code\":{},\"elapsed_ms\":{},\"spawn_count\":{},\"stdout_bytes\":{},\"stderr_bytes\":{},\"output_digest\":\"{}\"}}",
            escape(&self.id),
            self.required,
            option_number(self.exit_code),
            self.elapsed_ms,
            self.spawn_count,
            self.stdout_bytes,
            self.stderr_bytes,
            escape(&self.output_digest),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationExecutionReport {
    pub results: Vec<CheckResult>,
}

impl ValidationExecutionReport {
    pub fn new(results: Vec<CheckResult>) -> Self {
        Self { results }
    }

    pub fn aggregate(&self) -> Result<ValidationExecutionAggregate, String> {
        self.validate()?;
        let mut aggregate = ValidationExecutionAggregate {
            status: ValidationStatus::Passed,
            total: self.results.len(),
            passed: 0,
            failed: 0,
            required_failed: 0,
            optional_failed: 0,
        };

        for result in &self.results {
            if result.is_success() {
                aggregate.passed += 1;
            } else {
                aggregate.failed += 1;
                if result.required {
                    aggregate.required_failed += 1;
                } else {
                    aggregate.optional_failed += 1;
                }
            }
        }
        if aggregate.required_failed > 0 {
            aggregate.status = ValidationStatus::Failed;
        }
        Ok(aggregate)
    }

    pub fn to_json(&self) -> Result<String, String> {
        let aggregate = self.aggregate()?;
        Ok(format!(
            "{{\"schema\":{VALIDATION_EXECUTION_SCHEMA},{},\"results\":[{}]}}",
            aggregate.json_fields(),
            self.results
                .iter()
                .map(CheckResult::to_json)
                .collect::<Vec<_>>()
                .join(",")
        ))
    }

    fn validate(&self) -> Result<(), String> {
        if self.results.is_empty() {
            return Err("validation execution report must contain at least one result".to_string());
        }
        for result in &self.results {
            if result.id.trim().is_empty() {
                return Err("validation execution result ID must not be empty".to_string());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationExecutionAggregate {
    pub status: ValidationStatus,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub required_failed: usize,
    pub optional_failed: usize,
}

impl ValidationExecutionAggregate {
    pub fn to_json(&self) -> String {
        format!(
            "{{\"schema\":{VALIDATION_EXECUTION_SCHEMA},{}}}",
            self.json_fields()
        )
    }

    fn json_fields(&self) -> String {
        format!(
            "\"status\":\"{}\",\"total\":{},\"passed\":{},\"failed\":{},\"required_failed\":{},\"optional_failed\":{}",
            self.status.as_str(),
            self.total,
            self.passed,
            self.failed,
            self.required_failed,
            self.optional_failed,
        )
    }
}

pub(crate) fn output_digest(stdout: &[u8], stderr: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in stdout
        .iter()
        .copied()
        .chain(std::iter::once(0xff))
        .chain(stderr.iter().copied())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationObservation {
    pub name: String,
    pub required: bool,
    pub exit_code: i32,
}

impl ValidationObservation {
    pub fn new(name: impl Into<String>, required: bool, exit_code: i32) -> Self {
        Self {
            name: name.into(),
            required,
            exit_code,
        }
    }

    fn to_json(&self) -> String {
        format!(
            "{{\"name\":\"{}\",\"required\":{},\"exit_code\":{}}}",
            escape(&self.name),
            self.required,
            self.exit_code
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReport {
    pub observations: Vec<ValidationObservation>,
}

impl ValidationReport {
    pub fn new(observations: Vec<ValidationObservation>) -> Self {
        Self { observations }
    }

    pub fn from_json(document: &str) -> Result<Self, String> {
        let mut object = JsonParser::new(document)
            .parse()?
            .into_object("validation report")?;
        require_keys(&object, &["schema", "results"], "validation report")?;
        let schema = take(&mut object, "schema", "validation report")?.into_number("schema")?;
        if schema != VALIDATION_REPORT_SCHEMA {
            return Err(format!("unsupported validation report schema: {schema}"));
        }
        let observations = take(&mut object, "results", "validation report")?
            .into_array("results")?
            .into_iter()
            .map(parse_observation)
            .collect::<Result<Vec<_>, _>>()?;
        let report = Self { observations };
        report.validate()?;
        Ok(report)
    }

    pub fn to_json(&self) -> Result<String, String> {
        self.validate()?;
        Ok(format!(
            "{{\"schema\":{VALIDATION_REPORT_SCHEMA},\"results\":[{}]}}",
            self.observations
                .iter()
                .map(ValidationObservation::to_json)
                .collect::<Vec<_>>()
                .join(",")
        ))
    }

    pub fn aggregate(&self) -> Result<ValidationAggregate, String> {
        self.validate()?;
        let mut aggregate = ValidationAggregate {
            status: ValidationStatus::Passed,
            total: self.observations.len(),
            passed: 0,
            failed: 0,
            required_failed: 0,
            optional_failed: 0,
        };
        for observation in &self.observations {
            if observation.exit_code == 0 {
                aggregate.passed += 1;
            } else {
                aggregate.failed += 1;
                if observation.required {
                    aggregate.required_failed += 1;
                } else {
                    aggregate.optional_failed += 1;
                }
            }
        }
        if aggregate.required_failed > 0 {
            aggregate.status = ValidationStatus::Failed;
        }
        Ok(aggregate)
    }

    fn validate(&self) -> Result<(), String> {
        if self.observations.is_empty() {
            return Err("validation report must contain at least one observation".to_string());
        }
        let mut names = BTreeSet::new();
        for observation in &self.observations {
            if observation.name.trim().is_empty() || !names.insert(&observation.name) {
                return Err(format!(
                    "validation observation name is empty or duplicated: {}",
                    observation.name
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationAggregate {
    pub status: ValidationStatus,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub required_failed: usize,
    pub optional_failed: usize,
}

impl ValidationAggregate {
    pub fn to_json(&self) -> String {
        format!(
            "{{\"schema\":{VALIDATION_AGGREGATE_SCHEMA},\"status\":\"{}\",\"total\":{},\"passed\":{},\"failed\":{},\"required_failed\":{},\"optional_failed\":{}}}",
            self.status.as_str(),
            self.total,
            self.passed,
            self.failed,
            self.required_failed,
            self.optional_failed
        )
    }
}

fn parse_observation(value: JsonValue) -> Result<ValidationObservation, String> {
    let mut object = value.into_object("validation observation")?;
    require_keys(
        &object,
        &["name", "required", "exit_code"],
        "validation observation",
    )?;
    let exit_code = i32::try_from(
        take(&mut object, "exit_code", "validation observation")?
            .into_signed_number("exit_code")?,
    )
    .map_err(|_| "validation exit code exceeds i32".to_string())?;
    Ok(ValidationObservation {
        name: take(&mut object, "name", "validation observation")?.into_string("name")?,
        required: take(&mut object, "required", "validation observation")?.into_bool("required")?,
        exit_code,
    })
}

fn take(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<JsonValue, String> {
    object
        .remove(key)
        .ok_or_else(|| format!("missing {key} in {context}"))
}

fn option_number(value: Option<i32>) -> String {
    value
        .map(|number| number.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn require_keys(
    object: &BTreeMap<String, JsonValue>,
    expected: &[&str],
    context: &str,
) -> Result<(), String> {
    for key in object.keys() {
        if !expected.contains(&key.as_str()) {
            return Err(format!("unknown key {key} in {context}"));
        }
    }
    Ok(())
}

fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32))
            }
            character => escaped.push(character),
        }
    }
    escaped
}
