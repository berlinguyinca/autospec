use autospec_core::safety::IssuePromotionDecision;

pub(super) fn promotion_decision_json(decision: &IssuePromotionDecision, changed: bool) -> String {
    format!(
        "{{\"issue\":{{\"number\":{},\"title\":{}}},\"safety\":{{\"decision\":{},\"reason\":{}}},\"auto-implement\":{},\"eligible\":{},\"changed\":{},\"final_labels\":{},\"blocked_by_reason\":{}}}",
        decision.number,
        json_string(&decision.title),
        json_string(decision.safety_decision.as_str()),
        json_string(&decision.safety_reason),
        json_bool(decision.auto_implement),
        json_bool(decision.eligible),
        json_bool(changed),
        json_string_array(&decision.final_labels),
        json_usize_map(&decision.blocked_by_reason),
    )
}

fn json_usize_map(values: &std::collections::BTreeMap<String, usize>) -> String {
    format!(
        "{{{}}}",
        values
            .iter()
            .map(|(key, value)| format!("{}:{}", json_string(key), value))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn json_string_array(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| json_string(value))
            .collect::<Vec<_>>()
            .join(",")
    )
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

fn json_bool(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}
