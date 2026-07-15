use std::collections::BTreeMap;

use crate::state::json::{JsonParser, JsonValue};

use super::model::{
    normalized_slug, DetectedDomain, FileLineEvidence, SpecialistRoster, SuggestedSpecialist,
};

pub(crate) fn parse_roster_json(input: &str) -> Result<SpecialistRoster, String> {
    let mut root = JsonParser::new(input)
        .parse()?
        .into_object("specialist roster")?;
    reject_unknown(
        &root,
        &["schema_version", "domains", "suggested_specialists"],
        "specialist roster",
    )?;
    let schema_version = take_required(&mut root, "schema_version", "specialist roster")?
        .into_number("specialist roster.schema_version")?;
    if schema_version != 1 {
        return Err("specialist roster.schema_version must equal 1".to_string());
    }
    Ok(SpecialistRoster {
        schema_version: 1,
        domains: parse_domains(take_required(&mut root, "domains", "specialist roster")?)?,
        suggested_specialists: parse_specialists(take_required(
            &mut root,
            "suggested_specialists",
            "specialist roster",
        )?)?,
    })
}

pub(crate) fn parse_proposal_specialists(input: &str) -> Option<Vec<SuggestedSpecialist>> {
    let value = JsonParser::new(input).parse().ok()?;
    let candidates = match value {
        JsonValue::Array(values) => values,
        JsonValue::Object(mut values) => values
            .remove("suggested_specialists")?
            .into_array("proposal")
            .ok()?,
        _ => return None,
    };
    let specialists = candidates
        .into_iter()
        .filter_map(|value| parse_specialist(value, "proposal specialist").ok())
        .filter_map(normalize_specialist)
        .collect::<Vec<_>>();
    (!specialists.is_empty()).then_some(specialists)
}

impl SpecialistRoster {
    pub fn to_json_pretty(&self) -> String {
        let domains = self
            .domains
            .iter()
            .map(domain_json)
            .collect::<Vec<_>>()
            .join(",\n");
        let specialists = self
            .suggested_specialists
            .iter()
            .map(specialist_json)
            .collect::<Vec<_>>()
            .join(",\n");
        format!(
            "{{\n  \"schema_version\": {},\n  \"domains\": [{}{}{}],\n  \"suggested_specialists\": [{}{}{}]\n}}\n",
            self.schema_version,
            bracket_prefix(&domains),
            indent(&domains, 4),
            bracket_suffix(&domains),
            bracket_prefix(&specialists),
            indent(&specialists, 4),
            bracket_suffix(&specialists),
        )
    }
}

fn parse_domains(value: JsonValue) -> Result<Vec<DetectedDomain>, String> {
    value
        .into_array("specialist roster.domains")?
        .into_iter()
        .enumerate()
        .map(|(index, value)| parse_domain(value, &format!("specialist roster.domains[{index}]")))
        .collect()
}

fn parse_domain(value: JsonValue, context: &str) -> Result<DetectedDomain, String> {
    let mut object = value.into_object(context)?;
    reject_unknown(&object, &["name", "score", "evidence"], context)?;
    let score =
        take_required(&mut object, "score", context)?.into_number(&format!("{context}.score"))?;
    Ok(DetectedDomain {
        name: take_required(&mut object, "name", context)?
            .into_string(&format!("{context}.name"))?,
        score: usize::try_from(score).map_err(|_| format!("{context}.score exceeds usize"))?,
        evidence: parse_evidence(take_required(&mut object, "evidence", context)?, context)?,
    })
}

fn parse_evidence(value: JsonValue, context: &str) -> Result<Vec<FileLineEvidence>, String> {
    value
        .into_array(&format!("{context}.evidence"))?
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let item_context = format!("{context}.evidence[{index}]");
            let mut object = value.into_object(&item_context)?;
            reject_unknown(&object, &["file", "line", "match"], &item_context)?;
            let line = take_required(&mut object, "line", &item_context)?
                .into_number(&format!("{item_context}.line"))?;
            Ok(FileLineEvidence {
                file: take_required(&mut object, "file", &item_context)?
                    .into_string(&format!("{item_context}.file"))?,
                line: usize::try_from(line)
                    .map_err(|_| format!("{item_context}.line exceeds usize"))?,
                r#match: take_required(&mut object, "match", &item_context)?
                    .into_string(&format!("{item_context}.match"))?,
            })
        })
        .collect()
}

fn parse_specialists(value: JsonValue) -> Result<Vec<SuggestedSpecialist>, String> {
    value
        .into_array("specialist roster.suggested_specialists")?
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            parse_specialist(
                value,
                &format!("specialist roster.suggested_specialists[{index}]"),
            )
        })
        .collect()
}

fn parse_specialist(value: JsonValue, context: &str) -> Result<SuggestedSpecialist, String> {
    let mut object = value.into_object(context)?;
    reject_unknown(
        &object,
        &["slug", "persona", "lens", "why", "evidence"],
        context,
    )?;
    Ok(SuggestedSpecialist {
        slug: take_required(&mut object, "slug", context)?
            .into_string(&format!("{context}.slug"))?,
        persona: take_required(&mut object, "persona", context)?
            .into_string(&format!("{context}.persona"))?,
        lens: take_required(&mut object, "lens", context)?
            .into_string(&format!("{context}.lens"))?,
        why: take_required(&mut object, "why", context)?.into_string(&format!("{context}.why"))?,
        evidence: take_required(&mut object, "evidence", context)?
            .into_string(&format!("{context}.evidence"))?,
    })
}

fn normalize_specialist(mut specialist: SuggestedSpecialist) -> Option<SuggestedSpecialist> {
    specialist.slug = normalized_slug(&specialist.slug)?;
    (!specialist.persona.trim().is_empty()
        && !specialist.lens.trim().is_empty()
        && !specialist.why.trim().is_empty()
        && !specialist.evidence.trim().is_empty())
    .then_some(specialist)
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

fn reject_unknown(
    object: &BTreeMap<String, JsonValue>,
    expected: &[&str],
    context: &str,
) -> Result<(), String> {
    if let Some(key) = object.keys().find(|key| !expected.contains(&key.as_str())) {
        return Err(format!("unknown {context} key: {key}"));
    }
    Ok(())
}

fn domain_json(domain: &DetectedDomain) -> String {
    let evidence = domain
        .evidence
        .iter()
        .map(evidence_json)
        .collect::<Vec<_>>()
        .join(",\n");
    format!(
        "{{\n  \"name\": {},\n  \"score\": {},\n  \"evidence\": [{}{}{}]\n}}",
        json_string(&domain.name),
        domain.score,
        bracket_prefix(&evidence),
        indent(&evidence, 4),
        bracket_suffix(&evidence),
    )
}

fn evidence_json(evidence: &FileLineEvidence) -> String {
    format!(
        "{{\n  \"file\": {},\n  \"line\": {},\n  \"match\": {}\n}}",
        json_string(&evidence.file),
        evidence.line,
        json_string(&evidence.r#match),
    )
}

fn specialist_json(specialist: &SuggestedSpecialist) -> String {
    format!(
        "{{\n  \"slug\": {},\n  \"persona\": {},\n  \"lens\": {},\n  \"why\": {},\n  \"evidence\": {}\n}}",
        json_string(&specialist.slug),
        json_string(&specialist.persona),
        json_string(&specialist.lens),
        json_string(&specialist.why),
        json_string(&specialist.evidence),
    )
}

fn bracket_prefix(values: &str) -> &'static str {
    if values.is_empty() {
        ""
    } else {
        "\n"
    }
}

fn bracket_suffix(values: &str) -> &'static str {
    if values.is_empty() {
        ""
    } else {
        "\n  "
    }
}

fn indent(value: &str, spaces: usize) -> String {
    if value.is_empty() {
        return String::new();
    }
    let prefix = " ".repeat(spaces);
    value
        .lines()
        .map(|line| format!("{prefix}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
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
    escaped.push('"');
    escaped
}
