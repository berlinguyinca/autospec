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
        &[
            "schema_version",
            "generated_at",
            "domains",
            "suggested_specialists",
        ],
        "specialist roster",
    )?;
    let schema_version = take_required(&mut root, "schema_version", "specialist roster")?
        .into_number("specialist roster.schema_version")?;
    if schema_version != 1 {
        return Err("specialist roster.schema_version must equal 1".to_string());
    }
    if let Some(generated_at) = root.remove("generated_at") {
        generated_at.into_string("specialist roster.generated_at")?;
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
    let proposal_was_empty = candidates.is_empty();
    let specialists = candidates
        .into_iter()
        .filter_map(|value| parse_specialist(value, "proposal specialist", false).ok())
        .filter_map(normalize_specialist)
        .collect::<Vec<_>>();
    (proposal_was_empty || !specialists.is_empty()).then_some(specialists)
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
    if score == 0 {
        return Err(format!("{context}.score must be at least 1"));
    }
    let name = non_empty(
        take_required(&mut object, "name", context)?.into_string(&format!("{context}.name"))?,
        &format!("{context}.name"),
    )?;
    Ok(DetectedDomain {
        name,
        score: usize::try_from(score).map_err(|_| format!("{context}.score exceeds usize"))?,
        evidence: parse_evidence(take_required(&mut object, "evidence", context)?, context)?,
    })
}

fn parse_evidence(value: JsonValue, context: &str) -> Result<Vec<FileLineEvidence>, String> {
    let values = value.into_array(&format!("{context}.evidence"))?;
    let mut evidence = Vec::with_capacity(values.len());
    for (index, value) in values.into_iter().enumerate() {
        let item_context = format!("{context}.evidence[{index}]");
        evidence.push(parse_evidence_item(value, &item_context)?);
    }
    if evidence.is_empty() {
        return Err(format!("{context}.evidence must not be empty"));
    }
    Ok(evidence)
}

fn parse_evidence_item(value: JsonValue, context: &str) -> Result<FileLineEvidence, String> {
    let mut object = value.into_object(context)?;
    reject_unknown(&object, &["file", "line", "match"], context)?;
    let file =
        take_required(&mut object, "file", context)?.into_string(&format!("{context}.file"))?;
    let line =
        take_required(&mut object, "line", context)?.into_number(&format!("{context}.line"))?;
    let matched =
        take_required(&mut object, "match", context)?.into_string(&format!("{context}.match"))?;
    if line == 0 {
        return Err(format!("{context}.line must be at least 1"));
    }
    Ok(FileLineEvidence {
        file: non_empty(file, &format!("{context}.file"))?,
        line: usize::try_from(line).map_err(|_| format!("{context}.line exceeds usize"))?,
        r#match: non_empty(matched, &format!("{context}.match"))?,
    })
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
                true,
            )
        })
        .collect()
}

fn parse_specialist(
    value: JsonValue,
    context: &str,
    require_canonical_slug: bool,
) -> Result<SuggestedSpecialist, String> {
    let mut object = value.into_object(context)?;
    reject_unknown(
        &object,
        &["slug", "persona", "lens", "why", "evidence"],
        context,
    )?;
    let slug =
        take_required(&mut object, "slug", context)?.into_string(&format!("{context}.slug"))?;
    if require_canonical_slug && normalized_slug(&slug).as_deref() != Some(slug.as_str()) {
        return Err(format!(
            "{context}.slug must be a lowercase kebab-case slug"
        ));
    }
    Ok(SuggestedSpecialist {
        slug,
        persona: non_empty(
            take_required(&mut object, "persona", context)?
                .into_string(&format!("{context}.persona"))?,
            &format!("{context}.persona"),
        )?,
        lens: non_empty(
            take_required(&mut object, "lens", context)?.into_string(&format!("{context}.lens"))?,
            &format!("{context}.lens"),
        )?,
        why: non_empty(
            take_required(&mut object, "why", context)?.into_string(&format!("{context}.why"))?,
            &format!("{context}.why"),
        )?,
        evidence: non_empty(
            take_required(&mut object, "evidence", context)?
                .into_string(&format!("{context}.evidence"))?,
            &format!("{context}.evidence"),
        )?,
    })
}

fn non_empty(value: String, context: &str) -> Result<String, String> {
    if value.is_empty() {
        return Err(format!("{context} must not be empty"));
    }
    Ok(value)
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

#[cfg(test)]
mod tests {
    use super::parse_roster_json;

    const VALID_ROSTER: &str = r#"{
        "schema_version": 1,
        "domains": [{
            "name": "trading",
            "score": 1,
            "evidence": [{"file": "requirements.txt", "line": 1, "match": "ccxt"}]
        }],
        "suggested_specialists": [{
            "slug": "trading-specialist",
            "persona": "Trading specialist",
            "lens": "risk",
            "why": "dependency",
            "evidence": "requirements.txt:1"
        }]
    }"#;

    #[test]
    fn rejects_values_that_violate_roster_schema_constraints() {
        let invalid_rosters = [
            VALID_ROSTER.replace("\"name\": \"trading\"", "\"name\": \"\""),
            VALID_ROSTER.replace("\"score\": 1", "\"score\": 0"),
            VALID_ROSTER.replace(
                "[{\"file\": \"requirements.txt\", \"line\": 1, \"match\": \"ccxt\"}]",
                "[]",
            ),
            VALID_ROSTER.replace("\"file\": \"requirements.txt\"", "\"file\": \"\""),
            VALID_ROSTER.replace("\"line\": 1", "\"line\": 0"),
            VALID_ROSTER.replace("\"match\": \"ccxt\"", "\"match\": \"\""),
            VALID_ROSTER.replace("\"slug\": \"trading-specialist\"", "\"slug\": \"Bad slug\""),
            VALID_ROSTER.replace("\"persona\": \"Trading specialist\"", "\"persona\": \"\""),
            VALID_ROSTER.replace("\"lens\": \"risk\"", "\"lens\": \"\""),
            VALID_ROSTER.replace("\"why\": \"dependency\"", "\"why\": \"\""),
            VALID_ROSTER.replace("\"evidence\": \"requirements.txt:1\"", "\"evidence\": \"\""),
        ];

        for roster in invalid_rosters {
            assert!(parse_roster_json(&roster).is_err(), "roster={roster}");
        }
    }

    #[test]
    fn accepts_optional_generated_at() {
        let roster = VALID_ROSTER.replacen(
            "\"domains\"",
            "\"generated_at\": \"2026-07-15T00:00:00Z\",\n        \"domains\"",
            1,
        );

        parse_roster_json(&roster).unwrap();
    }
}
