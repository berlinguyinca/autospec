#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpecId(String);

impl SpecId {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if is_valid_spec_id(&value) {
            Ok(Self(value))
        } else {
            Err(value)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecVersion(String);

impl SpecVersion {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        let has_digits = value
            .chars()
            .skip(1)
            .all(|character| character.is_ascii_digit());
        if value.starts_with('V') && value.len() > 1 && has_digits {
            Ok(Self(value))
        } else {
            Err(value)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecStatus {
    Ready,
    Completed,
    Blocked,
    Deferred,
    Superseded,
}

impl SpecStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SpecStatus::Ready => "ready",
            SpecStatus::Completed => "completed",
            SpecStatus::Blocked => "blocked",
            SpecStatus::Deferred => "deferred",
            SpecStatus::Superseded => "superseded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecMetadata {
    pub id: SpecId,
    pub title: String,
    pub version: SpecVersion,
    pub status: SpecStatus,
    pub objective: String,
    pub dependencies: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub validation_command: String,
}

impl SpecMetadata {
    pub fn to_json(&self) -> String {
        let dependencies = json_string_array(&self.dependencies);
        let acceptance_criteria = json_string_array(&self.acceptance_criteria);
        format!(
            "{{\"id\":\"{}\",\"title\":\"{}\",\"version\":\"{}\",\"status\":\"{}\",\"objective\":\"{}\",\"dependencies\":{},\"acceptance_criteria\":{},\"validation_command\":\"{}\"}}",
            escape_json(self.id.as_str()),
            escape_json(&self.title),
            escape_json(self.version.as_str()),
            self.status.as_str(),
            escape_json(&self.objective),
            dependencies,
            acceptance_criteria,
            escape_json(&self.validation_command)
        )
    }
}

pub fn is_valid_spec_id(value: &str) -> bool {
    let Some(rest) = value.strip_prefix('v') else {
        return false;
    };
    let Some((version, slug)) = rest.split_once('-') else {
        return false;
    };
    !version.is_empty()
        && version.chars().all(|character| character.is_ascii_digit())
        && !slug.is_empty()
        && slug.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

fn json_string_array(values: &[String]) -> String {
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
            other => vec![other],
        })
        .collect()
}
