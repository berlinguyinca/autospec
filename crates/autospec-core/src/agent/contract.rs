use std::collections::BTreeMap;

use crate::agent::session::{
    CreationIntent, NativeSessionV1, SessionCapabilities, SessionEvent, SessionHarness,
    SessionLineage,
};
use crate::state::json::{JsonParser, JsonValue};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTask {
    pub spec_id: String,
    pub instructions: String,
    pub validation_command: String,
}

impl AgentTask {
    pub fn new(
        spec_id: impl Into<String>,
        instructions: impl Into<String>,
        validation_command: impl Into<String>,
    ) -> Self {
        Self {
            spec_id: spec_id.into(),
            instructions: instructions.into(),
            validation_command: validation_command.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentResult {
    pub result: String,
    pub files_changed: Vec<String>,
    pub validation: String,
    pub blockers: Vec<String>,
    pub handoff: String,
}

impl AgentResult {
    pub fn new(
        result: impl Into<String>,
        files_changed: Vec<String>,
        validation: impl Into<String>,
        blockers: Vec<String>,
        handoff: impl Into<String>,
    ) -> Self {
        Self {
            result: result.into(),
            files_changed,
            validation: validation.into(),
            blockers,
            handoff: handoff.into(),
        }
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"result\":\"{}\",\"files_changed\":{},\"validation\":\"{}\",\"blockers\":{},\"handoff\":\"{}\"}}",
            escape_json(&self.result),
            json_array(&self.files_changed),
            escape_json(&self.validation),
            json_array(&self.blockers),
            escape_json(&self.handoff)
        )
    }

    pub fn from_json(document: &str) -> Result<Self, String> {
        Self::from_json_value(JsonParser::new(document).parse()?)
    }

    pub(crate) fn from_json_value(value: JsonValue) -> Result<Self, String> {
        let mut object = value.into_object("agent result")?;
        require_keys(
            &object,
            [
                "result",
                "files_changed",
                "validation",
                "blockers",
                "handoff",
            ]
            .as_slice(),
            "agent result",
        )?;
        Ok(Self {
            result: take(&mut object, "result", "agent result")?.into_string("result")?,
            files_changed: string_array(
                take(&mut object, "files_changed", "agent result")?,
                "files_changed",
            )?,
            validation: take(&mut object, "validation", "agent result")?
                .into_string("validation")?,
            blockers: string_array(take(&mut object, "blockers", "agent result")?, "blockers")?,
            handoff: take(&mut object, "handoff", "agent result")?.into_string("handoff")?,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SafeModePolicy {
    pub allow_destructive: bool,
}

impl SafeModePolicy {
    pub fn check(&self, task: &AgentTask) -> Result<(), String> {
        if self.allow_destructive {
            return Ok(());
        }
        let text =
            format!("{}\n{}", task.instructions, task.validation_command).to_ascii_lowercase();
        let checks = [
            (
                "destructive git",
                ["git reset --hard", "git push --force"].as_slice(),
            ),
            ("filesystem deletion", ["rm -rf", "unlink "].as_slice()),
            (
                "credential access",
                ["aws_secret", "github_token", "private key"].as_slice(),
            ),
            (
                "network publication",
                ["gh pr merge", "gh release upload"].as_slice(),
            ),
            (
                "production mutation",
                ["production", "prod database"].as_slice(),
            ),
        ];
        for (category, patterns) in checks {
            if patterns.iter().any(|pattern| text.contains(pattern)) {
                return Err(format!("safe mode blocked {category}"));
            }
        }
        Ok(())
    }
}

/// Provider-neutral coding agent runtime.
///
/// [`CodingAgentRuntime::execute_once`] preserves the existing one-shot
/// `AgentTask`/`AgentResult` handoff contract; the session operations carry
/// the "Native session delta" of
/// `docs/specs/2026-09-01-observational-memory-native-sessions-readiness-integration-delta-design.md`
/// (scoped native IDs, resume/inspect/attach, heartbeat/lease/fencing,
/// reconciliation, truthful capability reporting, typed event streams).
pub trait CodingAgentRuntime {
    /// Run a one-shot task under the existing `AgentTask`/`AgentResult`
    /// compatibility contract.
    fn execute_once(&self, task: &AgentTask) -> Result<AgentResult, String>;

    /// Create a session idempotently. Retrying a crashed create with the same
    /// `idempotency_key` must return the already-created session and must
    /// never create a second one. `capabilities` is the caller's truthful
    /// report of what the harness actually supports; a harness fallback maps
    /// to a degraded capability set, never a widened one.
    fn create_session(
        &mut self,
        harness: SessionHarness,
        intent: CreationIntent,
        idempotency_key: &str,
        lineage: SessionLineage,
        capabilities: SessionCapabilities,
    ) -> Result<NativeSessionV1, String>;

    /// Resume the session under a new lease epoch, invalidating every stale
    /// client epoch. Lineage is preserved across the resume.
    fn resume(&mut self, session_id: &str, holder: &str) -> Result<NativeSessionV1, String>;

    /// Inspect the session without mutating it.
    fn inspect(&self, session_id: &str) -> Option<NativeSessionV1>;

    /// Attach to the session under `lease_epoch`. A stale epoch must be
    /// rejected and must not mutate the session.
    fn attach(
        &self,
        session_id: &str,
        holder: &str,
        lease_epoch: u64,
    ) -> Result<NativeSessionV1, String>;

    /// Record a heartbeat under the lease fence.
    fn heartbeat(&mut self, session_id: &str, holder: &str, lease_epoch: u64)
        -> Result<(), String>;

    /// Reconcile live sessions against one work item; returns every session
    /// with that lineage, ordered by scoped ID.
    fn reconcile(&self, work_item: &str) -> Vec<NativeSessionV1>;

    /// The session's typed event stream, in order.
    fn events(&self, session_id: &str) -> Option<Vec<SessionEvent>>;
}

pub fn render_handoff_prompt(agent: &str, task: &AgentTask) -> String {
    let agent_name = match agent {
        "codex" => "Codex",
        "claude" => "Claude",
        "fable" => "Fable",
        _ => "Generic",
    };
    format!(
        "# {agent_name} Agent Handoff\n\nSpec: {}\n\nInstructions:\n{}\n\nValidation:\n{}\n",
        task.spec_id, task.instructions, task.validation_command
    )
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
    let mut escaped = String::with_capacity(value.len());
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
                escaped.push_str(&format!("\\u{:04x}", character as u32))
            }
            character => escaped.push(character),
        }
    }
    escaped
}

fn string_array(value: JsonValue, context: &str) -> Result<Vec<String>, String> {
    value
        .into_array(context)?
        .into_iter()
        .map(|value| value.into_string(context))
        .collect()
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
