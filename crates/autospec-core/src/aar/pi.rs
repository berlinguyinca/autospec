//! Pi harness adapter boundary (AAR spec section 9).
//!
//! Pi stays a thin execution harness: it talks to the model, runs tools, edits
//! files and reports events. Everything policy-shaped is decided here and
//! handed to Pi as configuration, so the same boundary works for another
//! harness without moving policy into prompt prose.

use super::reasoning::SamplingProfile;
use super::topology::AgentRole;

/// The working rules AAR injects into every generated Pi session.
///
/// Reproduced verbatim from spec section 9; a test pins the text because
/// paraphrasing them has historically changed agent behaviour.
pub const WORKING_RULES: &[&str] = &[
    "Understand relevant code before editing.",
    "Prefer small controlled edits over rewrites.",
    "Make one logical change at a time.",
    "Re-read affected code after meaningful edits.",
    "Do not invent requirements or perform unrelated refactors.",
    "For bugs: form the simplest plausible hypothesis, gather targeted evidence,",
    "implement the smallest supported fix, test it, and stop.",
    "Do not repeatedly re-prove an established conclusion unless new evidence contradicts it.",
    "When acceptance criteria are satisfied, STOP.",
];

/// Render the working rules as one block.
pub fn working_rules_block() -> String {
    WORKING_RULES.join("\n")
}

/// Everything Pi needs to start one role's session.
#[derive(Debug, Clone, PartialEq)]
pub struct PiSessionSpec {
    pub session_id: String,
    pub worktree: String,
    pub role: AgentRole,
    pub provider: String,
    pub model: String,
    pub reasoning_tokens: u32,
    pub sampling: SamplingProfile,
    /// Extra rules appended after the standard working rules.
    pub extra_rules: Vec<String>,
    /// Hash of the cache-friendly stable prefix, for prefix-cache routing.
    pub stable_prefix_hash: String,
    pub max_context_tokens: u64,
    pub allow_forks: bool,
}

impl PiSessionSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.session_id.trim().is_empty() {
            return Err("pi session spec requires a session id".to_string());
        }
        if self.worktree.trim().is_empty() {
            return Err("pi session spec requires a worktree".to_string());
        }
        if self.model.trim().is_empty() {
            return Err("pi session spec requires a model".to_string());
        }
        if self.max_context_tokens == 0 {
            return Err("pi session spec requires a non-zero context ceiling".to_string());
        }
        self.sampling.validate()
    }

    /// The full rule block for this session.
    pub fn rules(&self) -> Vec<String> {
        let mut rules: Vec<String> = WORKING_RULES.iter().map(|rule| rule.to_string()).collect();
        rules.extend(self.extra_rules.iter().cloned());
        rules
    }
}

/// Build the Pi argv for a session.
///
/// Options are kept as distinct argv entries rather than one shell string: the
/// review dispatcher already learned that joining them re-introduces
/// shell-specific splitting bugs.
pub fn build_pi_argv(spec: &PiSessionSpec) -> Result<Vec<String>, String> {
    spec.validate()?;
    let mut argv = vec![
        "pi".to_string(),
        "session".to_string(),
        "--worktree".to_string(),
        spec.worktree.clone(),
        "--session-id".to_string(),
        spec.session_id.clone(),
        "--role".to_string(),
        spec.role.as_str().to_string(),
        "--provider".to_string(),
        spec.provider.clone(),
        "--model".to_string(),
        spec.model.clone(),
        "--reasoning-tokens".to_string(),
        spec.reasoning_tokens.to_string(),
        "--sampling-profile".to_string(),
        spec.sampling.identity(),
        "--temperature".to_string(),
        format!("{:.2}", spec.sampling.temperature),
        "--top-p".to_string(),
        format!("{:.2}", spec.sampling.top_p),
        "--top-k".to_string(),
        spec.sampling.top_k.to_string(),
        "--max-output-tokens".to_string(),
        spec.sampling.max_output_tokens.to_string(),
        "--max-context-tokens".to_string(),
        spec.max_context_tokens.to_string(),
    ];
    if !spec.stable_prefix_hash.is_empty() {
        argv.push("--prefix-cache-key".to_string());
        argv.push(spec.stable_prefix_hash.clone());
    }
    if !spec.allow_forks {
        argv.push("--no-forks".to_string());
    }
    Ok(argv)
}

/// Events Pi reports back during a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PiEvent {
    ToolCall {
        name: String,
    },
    FileRead {
        path: String,
    },
    FileEdit {
        path: String,
        lines: usize,
    },
    ContextMeasurement {
        prompt_tokens: u64,
        cached_tokens: u64,
        free_tokens: u64,
    },
    Fork {
        child_session_id: String,
    },
    Result {
        success: bool,
        summary: String,
    },
    Error {
        message: String,
    },
}

impl PiEvent {
    pub fn kind(&self) -> &'static str {
        match self {
            PiEvent::ToolCall { .. } => "tool_call",
            PiEvent::FileRead { .. } => "file_read",
            PiEvent::FileEdit { .. } => "file_edit",
            PiEvent::ContextMeasurement { .. } => "context_measurement",
            PiEvent::Fork { .. } => "fork",
            PiEvent::Result { .. } => "result",
            PiEvent::Error { .. } => "error",
        }
    }
}

/// Parse one Pi event line.
///
/// The wire format is `kind key=value ...`; unknown keys are rejected rather
/// than ignored so a harness change surfaces as an error instead of silently
/// dropping telemetry.
pub fn parse_pi_event(line: &str) -> Result<PiEvent, String> {
    let line = line.trim();
    if line.is_empty() {
        return Err("empty pi event".to_string());
    }
    let mut parts = line.splitn(2, ' ');
    let kind = parts.next().unwrap_or_default();
    let rest = parts.next().unwrap_or_default();
    let fields = parse_fields(rest)?;

    let field = |name: &str| -> Result<String, String> {
        fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
            .ok_or_else(|| format!("pi event {kind} missing field {name}"))
    };
    let number = |name: &str| -> Result<u64, String> {
        field(name)?
            .parse::<u64>()
            .map_err(|_| format!("pi event {kind} field {name} is not a number"))
    };

    let allowed: &[&str] = match kind {
        "tool_call" => &["name"],
        "file_read" => &["path"],
        "file_edit" => &["path", "lines"],
        "context_measurement" => &["prompt_tokens", "cached_tokens", "free_tokens"],
        "fork" => &["child_session_id"],
        "result" => &["success", "summary"],
        "error" => &["message"],
        other => return Err(format!("unknown pi event kind: {other}")),
    };
    for (key, _) in &fields {
        if !allowed.contains(&key.as_str()) {
            return Err(format!("unknown field {key} in pi event {kind}"));
        }
    }

    Ok(match kind {
        "tool_call" => PiEvent::ToolCall {
            name: field("name")?,
        },
        "file_read" => PiEvent::FileRead {
            path: field("path")?,
        },
        "file_edit" => PiEvent::FileEdit {
            path: field("path")?,
            lines: number("lines")? as usize,
        },
        "context_measurement" => PiEvent::ContextMeasurement {
            prompt_tokens: number("prompt_tokens")?,
            cached_tokens: number("cached_tokens")?,
            free_tokens: number("free_tokens")?,
        },
        "fork" => PiEvent::Fork {
            child_session_id: field("child_session_id")?,
        },
        "result" => PiEvent::Result {
            success: matches!(field("success")?.as_str(), "true" | "1"),
            summary: field("summary")?,
        },
        "error" => PiEvent::Error {
            message: field("message")?,
        },
        _ => unreachable!("kind validated above"),
    })
}

fn parse_fields(rest: &str) -> Result<Vec<(String, String)>, String> {
    let mut fields = Vec::new();
    let mut remainder = rest.trim();
    while !remainder.is_empty() {
        let equals = remainder
            .find('=')
            .ok_or_else(|| format!("malformed pi event field: {remainder}"))?;
        let key = remainder[..equals].trim().to_string();
        if key.is_empty() {
            return Err(format!("malformed pi event field: {remainder}"));
        }
        let after = &remainder[equals + 1..];
        let (value, next) = if let Some(quoted) = after.strip_prefix('"') {
            let close = quoted
                .find('"')
                .ok_or_else(|| format!("unterminated quoted value for {key}"))?;
            (
                quoted[..close].to_string(),
                quoted[close + 1..].trim_start(),
            )
        } else {
            match after.find(' ') {
                Some(space) => (after[..space].to_string(), after[space + 1..].trim_start()),
                None => (after.to_string(), ""),
            }
        };
        fields.push((key, value));
        remainder = next;
    }
    Ok(fields)
}

/// Structured result of one Pi session, as AAR records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiSessionResult {
    pub session_id: String,
    pub role: AgentRole,
    pub success: bool,
    pub summary: String,
    pub tool_calls: u32,
    pub files_read: Vec<String>,
    pub files_edited: Vec<String>,
    pub lines_changed: u64,
    pub prompt_tokens: u64,
    pub cached_prompt_tokens: u64,
    pub errors: Vec<String>,
    pub forks: Vec<String>,
}

/// Fold a Pi event stream into a structured session result.
pub fn fold_events(
    session_id: impl Into<String>,
    role: AgentRole,
    events: &[PiEvent],
) -> PiSessionResult {
    let mut result = PiSessionResult {
        session_id: session_id.into(),
        role,
        success: false,
        summary: String::new(),
        tool_calls: 0,
        files_read: Vec::new(),
        files_edited: Vec::new(),
        lines_changed: 0,
        prompt_tokens: 0,
        cached_prompt_tokens: 0,
        errors: Vec::new(),
        forks: Vec::new(),
    };
    for event in events {
        match event {
            PiEvent::ToolCall { .. } => result.tool_calls += 1,
            PiEvent::FileRead { path } => {
                if !result.files_read.contains(path) {
                    result.files_read.push(path.clone());
                }
            }
            PiEvent::FileEdit { path, lines } => {
                if !result.files_edited.contains(path) {
                    result.files_edited.push(path.clone());
                }
                result.lines_changed += *lines as u64;
            }
            PiEvent::ContextMeasurement {
                prompt_tokens,
                cached_tokens,
                ..
            } => {
                result.prompt_tokens = *prompt_tokens;
                result.cached_prompt_tokens = *cached_tokens;
            }
            PiEvent::Fork { child_session_id } => result.forks.push(child_session_id.clone()),
            PiEvent::Result { success, summary } => {
                result.success = *success;
                result.summary = summary.clone();
            }
            PiEvent::Error { message } => result.errors.push(message.clone()),
        }
    }
    result
}
