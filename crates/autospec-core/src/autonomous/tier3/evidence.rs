use super::model::{Tier3AdapterEvidence, Tier3Failure, Tier3Finding, Tier3Observation};
use super::{TIER3_RANK_LIMIT, TIER3_SCHEMA};

pub struct Tier3EvidenceDocuments<'a> {
    source: DocumentSource<'a>,
}

enum DocumentSource<'a> {
    Observation(&'a Tier3Observation),
    Failure(&'a Tier3Failure),
}

impl<'a> Tier3EvidenceDocuments<'a> {
    pub(super) fn observation(observation: &'a Tier3Observation) -> Self {
        Self {
            source: DocumentSource::Observation(observation),
        }
    }

    pub(super) fn failure(failure: &'a Tier3Failure) -> Self {
        Self {
            source: DocumentSource::Failure(failure),
        }
    }

    pub fn architecture_json(&self) -> Option<String> {
        self.architecture().map(adapter_json)
    }

    pub fn coverage_json(&self, predecessor_digest: &str) -> Result<Option<String>, String> {
        self.coverage()
            .map(|coverage| {
                adapter_with_predecessor_json("tier3_coverage", coverage, predecessor_digest)
            })
            .transpose()
    }

    pub fn debt_json(&self, predecessor_digest: &str) -> Result<Option<String>, String> {
        self.debt()
            .map(|debt| adapter_with_predecessor_json("tier3_debt", debt, predecessor_digest))
            .transpose()
    }

    pub fn findings_json(&self, predecessor_digest: &str) -> Result<Option<String>, String> {
        match self.source {
            DocumentSource::Observation(observation) => {
                let predecessor = digest(predecessor_digest)?;
                Ok(Some(format!(
                    "{{\"schema\":{TIER3_SCHEMA},\"kind\":\"tier3_findings\",\"predecessor_digest\":{predecessor},\"rank_limit\":{TIER3_RANK_LIMIT},\"funnel\":{},\"deduplicated\":[{}],\"ranked\":[{}]}}\n",
                    funnel_json(observation.funnel()),
                    findings_json(&observation.deduplicated),
                    findings_json(observation.ranked()),
                )))
            }
            DocumentSource::Failure(_) => Ok(None),
        }
    }

    pub fn failure_json(&self, predecessor_digest: Option<&str>) -> Result<Option<String>, String> {
        let DocumentSource::Failure(failure) = self.source else {
            return Ok(None);
        };
        let expected = failure.partial_evidence().has_architecture();
        let predecessor = match (expected, predecessor_digest) {
            (false, None) => "null".to_string(),
            (true, Some(value)) => digest(value)?,
            _ => {
                return Err("failure predecessor digest does not match completed stages".to_string())
            }
        };
        Ok(Some(format!(
            "{{\"schema\":{TIER3_SCHEMA},\"kind\":\"tier3_failure\",\"predecessor_digest\":{predecessor},\"stage\":{},\"code\":{},\"status_reason\":{},\"detail\":{},\"funnel\":{}}}\n",
            text(failure.stage().as_str()),
            text(failure.code().as_str()),
            text(&failure.status_reason()),
            text(failure.detail()),
            funnel_json(failure.partial_evidence().funnel()),
        )))
    }

    fn architecture(&self) -> Option<&Tier3AdapterEvidence> {
        match self.source {
            DocumentSource::Observation(observation) => Some(&observation.architecture),
            DocumentSource::Failure(failure) => failure.partial_evidence().architecture(),
        }
    }

    fn coverage(&self) -> Option<&Tier3AdapterEvidence> {
        match self.source {
            DocumentSource::Observation(observation) => Some(&observation.coverage),
            DocumentSource::Failure(failure) => failure.partial_evidence().coverage(),
        }
    }

    fn debt(&self) -> Option<&Tier3AdapterEvidence> {
        match self.source {
            DocumentSource::Observation(observation) => Some(&observation.debt),
            DocumentSource::Failure(failure) => failure.partial_evidence().debt(),
        }
    }
}

fn adapter_json(adapter: &Tier3AdapterEvidence) -> String {
    format!(
        "{{\"schema\":{TIER3_SCHEMA},\"kind\":\"tier3_{}\",\"adapter_version\":{},\"rule_version\":{},\"findings\":[{}]}}\n",
        adapter
            .findings
            .first()
            .map_or("architecture", |finding| finding.kind.as_str()),
        text(&adapter.adapter_version),
        text(&adapter.rule_version),
        findings_json(&adapter.findings),
    )
}

fn adapter_with_predecessor_json(
    kind: &str,
    adapter: &Tier3AdapterEvidence,
    predecessor_digest: &str,
) -> Result<String, String> {
    let predecessor = digest(predecessor_digest)?;
    Ok(format!(
        "{{\"schema\":{TIER3_SCHEMA},\"kind\":{},\"predecessor_digest\":{predecessor},\"adapter_version\":{},\"rule_version\":{},\"findings\":[{}]}}\n",
        text(kind),
        text(&adapter.adapter_version),
        text(&adapter.rule_version),
        findings_json(&adapter.findings),
    ))
}

fn findings_json(findings: &[Tier3Finding]) -> String {
    findings
        .iter()
        .map(finding_json)
        .collect::<Vec<_>>()
        .join(",")
}

fn finding_json(finding: &Tier3Finding) -> String {
    format!(
        "{{\"kind\":{},\"severity\":{},\"rule_id\":{},\"path\":{},\"line\":{},\"message\":{}}}",
        text(finding.kind.as_str()),
        text(finding.severity.as_str()),
        text(&finding.rule_id),
        text(&finding.path),
        finding.line,
        text(&finding.message),
    )
}

fn funnel_json(funnel: &crate::autonomous::waterfall::FunnelCounts) -> String {
    format!(
        "{{\"observed\":{},\"deduplicated\":{},\"verified\":{},\"roi_approved\":{},\"ranked\":{}}}",
        funnel.observed, funnel.deduplicated, funnel.verified, funnel.roi_approved, funnel.ranked
    )
}

fn digest(value: &str) -> Result<String, String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(text(value))
    } else {
        Err("predecessor digest must be a sealed lowercase SHA-256 value".to_string())
    }
}

fn text(value: &str) -> String {
    let mut rendered = String::from("\"");
    for character in value.chars() {
        match character {
            '\"' => rendered.push_str("\\\""),
            '\\' => rendered.push_str("\\\\"),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            character if character.is_control() => {
                rendered.push_str(&format!("\\u{:04x}", character as u32))
            }
            character => rendered.push(character),
        }
    }
    rendered.push('\"');
    rendered
}
