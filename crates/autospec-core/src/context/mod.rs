//! Pure context-window monitor state machine.
//!
//! This module mirrors `packages/autospec_context_monitor/.../engine.py` so Rust
//! callers can classify token usage without performing any external I/O. The
//! driver remains responsible for executing returned actions.

/// State of the context monitor threshold machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextState {
    /// Below the compaction threshold or reset after successful compaction/rollover.
    Normal,
    /// Compaction was requested and the monitor is waiting to see whether it helped.
    Compacted,
    /// Rollover handoff/clear/resume was requested.
    Rolled,
}

/// Side-effect-free action descriptor returned by [`ContextMonitorEngine`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextAction {
    kind: &'static str,
    payload: String,
}

impl ContextAction {
    /// Create an action without a payload.
    pub fn new(kind: &'static str) -> Self {
        Self {
            kind,
            payload: String::new(),
        }
    }

    /// Create an action with a diagnostic payload.
    pub fn with_payload(kind: &'static str, payload: impl Into<String>) -> Self {
        Self {
            kind,
            payload: payload.into(),
        }
    }

    /// Return the action kind (`compact`, `handoff`, `clear`, `resume`, or `noop`).
    pub fn kind(&self) -> &'static str {
        self.kind
    }

    /// Return the optional action payload.
    pub fn payload(&self) -> &str {
        &self.payload
    }
}

/// Pure context monitor engine; callers execute returned actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMonitorEngine {
    state: ContextState,
}

impl Default for ContextMonitorEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextMonitorEngine {
    /// Create a monitor in [`ContextState::Normal`].
    pub fn new() -> Self {
        Self {
            state: ContextState::Normal,
        }
    }

    /// Return the current monitor state.
    pub fn state(&self) -> ContextState {
        self.state
    }

    /// Reset the monitor to [`ContextState::Normal`].
    pub fn reset(&mut self) {
        self.state = ContextState::Normal;
    }

    /// Classify token usage from integer token counts.
    pub fn classify(&mut self, used_tokens: u64, max_tokens: u64) -> Vec<ContextAction> {
        let percent = usage_percent(used_tokens, max_tokens);
        self.classify_percent(percent)
    }

    /// Classify usage as an integer percentage (`0` to `100`).
    ///
    /// The only rollover path is Normal → Compacted → Rolled; even a direct climb
    /// to 80% first emits `compact`, matching the Python engine invariant.
    pub fn classify_percent(&mut self, percent: u8) -> Vec<ContextAction> {
        match self.state {
            ContextState::Normal => self.classify_normal(percent),
            ContextState::Compacted => self.classify_compacted(percent),
            ContextState::Rolled => self.classify_rolled(percent),
        }
    }

    fn classify_normal(&mut self, percent: u8) -> Vec<ContextAction> {
        if percent < 50 {
            return Vec::new();
        }
        self.state = ContextState::Compacted;
        compact_actions()
    }

    fn classify_compacted(&mut self, percent: u8) -> Vec<ContextAction> {
        if percent >= 80 {
            self.state = ContextState::Rolled;
            return rollover_actions();
        }
        if percent < 30 {
            self.state = ContextState::Normal;
            return reset_actions("reset:compacted->normal");
        }
        Vec::new()
    }

    fn classify_rolled(&mut self, percent: u8) -> Vec<ContextAction> {
        if percent >= 30 {
            return Vec::new();
        }
        self.state = ContextState::Normal;
        reset_actions("reset:rolled->normal")
    }
}

fn usage_percent(used_tokens: u64, max_tokens: u64) -> u8 {
    if max_tokens == 0 {
        return 0;
    }
    let percent = (u128::from(used_tokens) * 100) / u128::from(max_tokens);
    percent.min(100) as u8
}

fn compact_actions() -> Vec<ContextAction> {
    vec![ContextAction::new("compact")]
}

fn rollover_actions() -> Vec<ContextAction> {
    ["handoff", "clear", "resume"]
        .into_iter()
        .map(ContextAction::new)
        .collect()
}

fn reset_actions(payload: &'static str) -> Vec<ContextAction> {
    vec![ContextAction::with_payload("noop", payload)]
}
