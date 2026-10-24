use serde::{Deserialize, Serialize};

const MAX_SUMMARY: usize = 1024;
const MAX_ARTIFACTS: usize = 32;
const MAX_ARTIFACT_REF: usize = 2048;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentFailureEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub schema_version: u32,
    pub incident_id: String,
    pub run_id: String,
    pub repo: String,
    pub revision: String,
    pub worktree: String,
    pub agent_role: String,
    pub failure_class: String,
    pub exit_code: Option<i32>,
    pub summary: String,
    pub artifact_refs: Vec<String>,
    pub reproduction: String,
    pub command: String,
    pub root_cause: String,
    pub classification: String,
    pub risk: String,
    pub created_at: String,
    pub fingerprint: String,
}

pub fn parse_agent_failure_event(document: &str) -> Result<AgentFailureEvent, String> {
    let event: AgentFailureEvent = serde_json::from_str(document)
        .map_err(|error| format!("invalid agent_failure event JSON: {error}"))?;
    if event.event_type != "agent_failure" {
        return Err("agent failure event type must be agent_failure".to_string());
    }
    if event.schema_version != 1 {
        return Err("agent failure event schema_version must be 1".to_string());
    }
    for (field, value) in [
        ("incident_id", &event.incident_id),
        ("run_id", &event.run_id),
        ("repo", &event.repo),
        ("revision", &event.revision),
        ("worktree", &event.worktree),
        ("agent_role", &event.agent_role),
        ("failure_class", &event.failure_class),
        ("reproduction", &event.reproduction),
        ("command", &event.command),
        ("root_cause", &event.root_cause),
        ("classification", &event.classification),
        ("risk", &event.risk),
        ("created_at", &event.created_at),
        ("fingerprint", &event.fingerprint),
    ] {
        if value.trim().is_empty() {
            return Err(format!("{field} must be nonblank"));
        }
    }
    if event.summary.trim().is_empty() || event.summary.chars().count() > MAX_SUMMARY {
        return Err(format!(
            "summary must be nonblank and at most {MAX_SUMMARY} characters"
        ));
    }
    if event.artifact_refs.len() > MAX_ARTIFACTS {
        return Err(format!(
            "artifact_refs must contain at most {MAX_ARTIFACTS} entries"
        ));
    }
    for reference in &event.artifact_refs {
        if reference.trim().is_empty() || reference.chars().count() > MAX_ARTIFACT_REF {
            return Err(format!(
                "artifact_refs entries must be nonblank and at most {MAX_ARTIFACT_REF} characters"
            ));
        }
    }
    if event.fingerprint.len() != 16 || !event.fingerprint.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("fingerprint must be a 16-character hexadecimal value".to_string());
    }
    Ok(event)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebuggerDispatch {
    pub role: String,
    pub dry_run: bool,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncidentIssueProjection {
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub debugger_dispatch: DebuggerDispatch,
}

impl IncidentIssueProjection {
    pub fn from_event(event: &AgentFailureEvent) -> Self {
        let marker = format!("<!-- pi-incident:fingerprint={} -->", event.fingerprint);
        let exit_code = event
            .exit_code
            .map_or_else(|| "null".to_string(), |code| code.to_string());
        let evidence = format_evidence(&event.artifact_refs);
        let body = format!(
            "{marker}\n\n## Pi agent failure\n\n- **Incident:** `{}`\n- **Run:** `{}`\n- **Repository:** `{}`\n- **Revision:** `{}`\n- **Agent role:** `{}`\n- **Failure class:** `{}`\n- **Classification:** `{}`\n- **Risk:** `{}`\n- **Exit code:** `{}`\n\n### Summary\n\n{}\n\n### Reproduction\n\n```text\n{}\n```\n\n### Command\n\n```text\n{}\n```\n\n### Root cause signature\n\n{}\n\n### Evidence\n\n{}\n\n## Autonomous debugger handoff\n\nThe debugger must reproduce the failure in a fresh isolated worktree, preserve the original evidence, and propose the smallest verified fix.\n",
            event.incident_id,
            event.run_id,
            event.repo,
            event.revision,
            event.agent_role,
            event.failure_class,
            event.classification,
            event.risk,
            exit_code,
            event.summary,
            event.reproduction,
            event.command,
            event.root_cause,
            evidence,
        );
        let prompt = format!(
            "Debug Pi incident {} in repository {}. Reproduce `{}` first. Use evidence {}. Work only in an isolated worktree; do not weaken assertions or bypass safety gates.",
            event.incident_id,
            event.repo,
            event.reproduction,
            event.artifact_refs.join(", "),
        );
        Self {
            title: format!("[Pi incident] {}: {}", event.repo, event.failure_class),
            body,
            labels: vec![
                "auto-implement".to_string(),
                "origin:pi-incident".to_string(),
            ],
            debugger_dispatch: DebuggerDispatch {
                role: "debugger".to_string(),
                dry_run: true,
                prompt,
            },
        }
    }

    pub fn marker(&self) -> String {
        self.body.lines().next().unwrap_or_default().to_string()
    }
}

fn format_evidence(references: &[String]) -> String {
    if references.is_empty() {
        return "- none".to_string();
    }
    references
        .iter()
        .map(|reference| format!("- {reference}"))
        .collect::<Vec<_>>()
        .join("\n")
}
