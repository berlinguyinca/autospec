use crate::spec::is_valid_spec_id;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecRunState {
    Planned,
    Ready,
    Running,
    Passed,
    Failed,
    Blocked,
    Deferred,
    Superseded,
}

impl SpecRunState {
    pub fn as_str(&self) -> &'static str {
        match self {
            SpecRunState::Planned => "planned",
            SpecRunState::Ready => "ready",
            SpecRunState::Running => "running",
            SpecRunState::Passed => "passed",
            SpecRunState::Failed => "failed",
            SpecRunState::Blocked => "blocked",
            SpecRunState::Deferred => "deferred",
            SpecRunState::Superseded => "superseded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecLifecycle {
    pub spec_id: String,
    pub state: SpecRunState,
    pub deferred_reason: Option<String>,
    pub superseded_by: Option<String>,
}

impl SpecLifecycle {
    pub fn new(spec_id: impl Into<String>) -> Self {
        Self {
            spec_id: spec_id.into(),
            state: SpecRunState::Planned,
            deferred_reason: None,
            superseded_by: None,
        }
    }

    pub fn transition_to(&mut self, next: SpecRunState) -> Result<(), String> {
        if is_allowed_transition(&self.state, &next) {
            self.state = next;
            Ok(())
        } else {
            Err(format!(
                "invalid transition from {} to {}",
                self.state.as_str(),
                next.as_str()
            ))
        }
    }

    pub fn deferred(mut self, reason: impl Into<String>) -> Result<Self, String> {
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err("deferred reason is required".to_string());
        }
        self.transition_to(SpecRunState::Deferred)?;
        self.deferred_reason = Some(reason);
        Ok(self)
    }

    pub fn superseded_by(mut self, replacement: impl Into<String>) -> Result<Self, String> {
        let replacement = replacement.into();
        if !is_valid_spec_id(&replacement) {
            return Err(format!("invalid replacement spec id: {replacement}"));
        }
        self.transition_to(SpecRunState::Superseded)?;
        self.superseded_by = Some(replacement);
        Ok(self)
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"spec_id\":\"{}\",\"state\":\"{}\",\"deferred_reason\":{},\"superseded_by\":{}}}",
            self.spec_id,
            self.state.as_str(),
            optional_json_string(&self.deferred_reason),
            optional_json_string(&self.superseded_by)
        )
    }
}

fn is_allowed_transition(current: &SpecRunState, next: &SpecRunState) -> bool {
    matches!(
        (current, next),
        (SpecRunState::Planned, SpecRunState::Ready)
            | (SpecRunState::Planned, SpecRunState::Deferred)
            | (SpecRunState::Planned, SpecRunState::Superseded)
            | (SpecRunState::Ready, SpecRunState::Running)
            | (SpecRunState::Ready, SpecRunState::Deferred)
            | (SpecRunState::Ready, SpecRunState::Superseded)
            | (SpecRunState::Running, SpecRunState::Passed)
            | (SpecRunState::Running, SpecRunState::Failed)
            | (SpecRunState::Running, SpecRunState::Blocked)
            | (SpecRunState::Failed, SpecRunState::Running)
            | (SpecRunState::Blocked, SpecRunState::Ready)
    )
}

fn optional_json_string(value: &Option<String>) -> String {
    value
        .as_ref()
        .map(|value| format!("\"{}\"", value.replace('"', "\\\"")))
        .unwrap_or_else(|| "null".to_string())
}
