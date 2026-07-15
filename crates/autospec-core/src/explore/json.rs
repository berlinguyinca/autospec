use crate::state::json::{JsonParser, JsonValue};

use super::{DetectedDomain, DomainEvidence, SpecialistRoster, SuggestedSpecialist};

impl SpecialistRoster {
    pub fn to_json_pretty(&self) -> String {
        let mut output = String::new();
        output.push_str("{\n  \"schema_version\": 1,\n  \"domains\": [");
        if !self.domains.is_empty() {
            output.push('\n');
        }
        for (index, domain) in self.domains.iter().enumerate() {
            if index > 0 {
                output.push_str(",\n");
            }
            output.push_str(&domain_json(domain));
        }
        if !self.domains.is_empty() {
            output.push('\n');
            output.push_str("  ");
        }
        output.push_str("],\n  \"suggested_specialists\": [");
        if !self.suggested_specialists.is_empty() {
            output.push('\n');
        }
        for (index, specialist) in self.suggested_specialists.iter().enumerate() {
            if index > 0 {
                output.push_str(",\n");
            }
            output.push_str(&specialist_json(specialist));
        }
        if !self.suggested_specialists.is_empty() {
            output.push('\n');
            output.push_str("  ");
        }
        output.push_str("]\n}\n");
        output
    }
}

pub(super) fn is_valid_roster_cache(content: &str) -> bool {
    let Ok(JsonValue::Object(mut object)) = JsonParser::new(content).parse() else {
        return false;
    };
    matches!(object.remove("schema_version"), Some(JsonValue::Number(value)) if value == "1")
        && matches!(object.remove("domains"), Some(JsonValue::Array(_)))
        && matches!(
            object.remove("suggested_specialists"),
            Some(JsonValue::Array(_))
        )
}

fn domain_json(domain: &DetectedDomain) -> String {
    format!(
        "    {{\"name\": {}, \"score\": {}, \"evidence\": [{}]}}",
        json_string(&domain.name),
        domain.score,
        domain
            .evidence
            .iter()
            .map(evidence_json)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn evidence_json(evidence: &DomainEvidence) -> String {
    format!(
        "{{\"file\": {}, \"line\": {}, \"match\": {}}}",
        json_string(&evidence.file),
        evidence.line,
        json_string(&evidence.match_text)
    )
}

fn specialist_json(specialist: &SuggestedSpecialist) -> String {
    format!(
        "    {{\"slug\": {}, \"persona\": {}, \"lens\": {}, \"why\": {}, \"evidence\": {}}}",
        json_string(&specialist.slug),
        json_string(&specialist.persona),
        json_string(&specialist.lens),
        json_string(&specialist.why),
        json_string(&specialist.evidence)
    )
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0C}' => escaped.push_str("\\f"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}
