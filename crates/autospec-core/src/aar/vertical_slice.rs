//! The first isolated budgeted coding vertical slice (issue #3320, M5).
//!
//! One Qwen3.8 Pi session, in its own worktree, changing exactly one
//! implementation file, gated by exactly one targeted test command, stopped by
//! an explicit turn budget and an explicit wall-clock budget, and described
//! afterwards by one ledger record. Nothing here talks to a node or a worktree:
//! the module is the policy, [`crate::aar`] calls it the slice contract, and
//! `scripts/pi-inferweave-vertical-slice.sh` is the I/O driver that performs
//! the fixture run and writes the ledger this module's [`SliceLedger`] parses.
//!
//! The division matters for what a reviewer can trust. The budgets are not
//! prose in a prompt; they are the numbers a caller configures and this module
//! enforces, in a fixed priority: a guard breach outranks a budget, and a
//! budget outranks the model's own claim of success. A run that "succeeded"
//! one turn past `max_turns` is a budget violation, not a success, and the
//! ledger says so.
//!
//! Every ledger field required by the issue (#3320 AC 4) is a typed field of
//! [`SliceLedger`]: `profile`, `node_id`, `turns`, `tool_calls`,
//! `wall_clock_ms`. [`LEDGER_FIELDS`] is the single list of keys the driver
//! must emit; both the Rust integration test and the bats suite check the
//! emitted JSON against it, so the two halves cannot drift silently.

use serde::{Deserialize, Serialize};

use super::capsule::RolePolicy;
use super::pi::{build_pi_argv, fold_events, parse_pi_event, PiEvent, PiSessionSpec};
use super::reasoning::SamplingProfile;
use super::topology::AgentRole;

/// Bumped whenever a persisted ledger field changes meaning or disappears.
pub const SLICE_LEDGER_SCHEMA_VERSION: u32 = 1;
/// Name the driver writes the ledger under, relative to the run directory.
pub const LEDGER_FILE_NAME: &str = "ledger.json";

/// Default turn budget for one slice. The fixture proves termination, not
/// endurance: a slice that needs eight turns to change one file is already the
/// wrong shape.
pub const DEFAULT_MAX_TURNS: u32 = 8;
/// Default wall-clock budget in seconds for one slice.
pub const DEFAULT_MAX_WALL_CLOCK_SECONDS: u64 = 300;
/// Hard ceiling on `max_turns`. A caller cannot configure an unbounded run.
pub const MAX_TURNS_CEILING: u32 = 64;
/// Hard ceiling on `max_wall_clock_seconds`. A caller cannot configure a run
/// that outlives the node lease it was seated on.
pub const MAX_WALL_CLOCK_SECONDS_CEILING: u64 = 3600;

/// The slice changes at most this many implementation files.
pub const MAX_IMPLEMENTATION_FILES: usize = 1;
/// The slice runs at most this many targeted test commands.
pub const MAX_TEST_COMMANDS: usize = 1;

/// Model profile the fixture is pinned to (Qwen3.8 on the M5 node class).
pub const FIXTURE_PROFILE: &str = "qwen3.8-m5";
/// InferWeave node id the fixture is seated on.
pub const FIXTURE_NODE_ID: &str = "inferweave-local-01";
/// Provider-neutral routing label handed to Pi for the fixture.
pub const FIXTURE_PROVIDER: &str = "inferweave";
/// The single implementation file the fixture session may edit.
pub const FIXTURE_TARGET_FILE: &str = "src/convert.sh";
/// The fixture's targeted test file, owned by the harness, never by the session.
pub const FIXTURE_TEST_FILE: &str = "tests/targeted_test.sh";
/// The single targeted test command the harness runs for the fixture.
pub const FIXTURE_TEST_COMMAND: &str = "bash tests/targeted_test.sh";
/// Context ceiling the fixture Pi session is configured with (small-LLM node).
pub const FIXTURE_MAX_CONTEXT_TOKENS: u64 = 32_768;
/// Reasoning-token budget for the fixture session.
pub const FIXTURE_REASONING_TOKENS: u32 = 512;

/// Ledger keys a driver must emit, in the order [`SliceLedger`] serializes
/// them. `tests/pi-inferweave-vertical-slice.bats` and the Rust integration
/// test both compare the driver's emitted JSON against this list.
pub const LEDGER_FIELDS: &[&str] = &[
    "schema_version",
    "run_id",
    "profile",
    "node_id",
    "session_id",
    "worktree",
    "turns",
    "tool_calls",
    "wall_clock_ms",
    "max_turns",
    "max_wall_clock_seconds",
    "files_changed",
    "test_commands",
    "tests_passed",
    "terminated_by",
    "within_budget",
    "session_closed",
];

/// The five fields issue #3320 AC 4 names explicitly. A ledger missing any of
/// them is not slice evidence.
pub const MANDATORY_LEDGER_FIELDS: &[&str] =
    &["profile", "node_id", "turns", "tool_calls", "wall_clock_ms"];

/// The two termination budgets a slice is configured with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SliceBudget {
    pub max_turns: u32,
    pub max_wall_clock_seconds: u64,
}

impl Default for SliceBudget {
    fn default() -> Self {
        Self {
            max_turns: DEFAULT_MAX_TURNS,
            max_wall_clock_seconds: DEFAULT_MAX_WALL_CLOCK_SECONDS,
        }
    }
}

impl SliceBudget {
    /// Configure a budget, rejecting zeroes and anything past the ceilings.
    ///
    /// A zero budget is rejected rather than clamped: it is a caller bug, and
    /// silently running a slice with a different budget than was asked for is
    /// worse than failing loudly.
    pub fn new(max_turns: u32, max_wall_clock_seconds: u64) -> Result<Self, String> {
        if max_turns == 0 {
            return Err("slice budget requires at least one turn".to_string());
        }
        if max_wall_clock_seconds == 0 {
            return Err("slice budget requires a non-zero wall clock".to_string());
        }
        if max_turns > MAX_TURNS_CEILING {
            return Err(format!(
                "slice max_turns {max_turns} exceeds ceiling {MAX_TURNS_CEILING}"
            ));
        }
        if max_wall_clock_seconds > MAX_WALL_CLOCK_SECONDS_CEILING {
            return Err(format!(
                "slice max_wall_clock_seconds {max_wall_clock_seconds} exceeds ceiling \
                 {MAX_WALL_CLOCK_SECONDS_CEILING}"
            ));
        }
        Ok(Self {
            max_turns,
            max_wall_clock_seconds,
        })
    }

    /// The wall-clock budget expressed in milliseconds, the unit of `now`.
    pub fn wall_clock_ms(&self) -> u64 {
        self.max_wall_clock_seconds.saturating_mul(1_000)
    }
}

/// Everything the driver needs to start one isolated slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceRequest {
    pub run_id: String,
    pub session_id: String,
    pub profile: String,
    pub node_id: String,
    pub provider: String,
    /// The isolated worktree the session is confined to.
    pub worktree: String,
    /// The single implementation file the session may edit.
    pub target_file: String,
    /// The single targeted test command the harness runs after the session.
    pub test_command: String,
    pub budget: SliceBudget,
}

impl SliceRequest {
    /// The fixture request: pinned profile and node, one file, one test.
    pub fn fixture(worktree: &str, run_id: &str) -> Self {
        Self {
            run_id: run_id.to_string(),
            session_id: format!("{run_id}-pi"),
            profile: FIXTURE_PROFILE.to_string(),
            node_id: FIXTURE_NODE_ID.to_string(),
            provider: FIXTURE_PROVIDER.to_string(),
            worktree: worktree.to_string(),
            target_file: FIXTURE_TARGET_FILE.to_string(),
            test_command: FIXTURE_TEST_COMMAND.to_string(),
            budget: SliceBudget::default(),
        }
    }

    /// Reject a request that cannot produce a single-file, single-test slice.
    pub fn validate(&self) -> Result<(), String> {
        for (label, value) in [
            ("run id", &self.run_id),
            ("session id", &self.session_id),
            ("profile", &self.profile),
            ("node id", &self.node_id),
            ("provider", &self.provider),
            ("worktree", &self.worktree),
            ("target file", &self.target_file),
            ("test command", &self.test_command),
        ] {
            if value.trim().is_empty() {
                return Err(format!("slice request requires a {label}"));
            }
        }
        if self.target_file.starts_with('/') || self.target_file.contains("..") {
            return Err(format!(
                "slice target file must be worktree-relative: {}",
                self.target_file
            ));
        }
        // One targeted test command means one command. A chained command hides
        // how many tests the slice really ran, which is AC 2.
        if self.test_command.contains("&&") || self.test_command.contains(';') {
            return Err(format!(
                "slice test command must be a single command, not a chain: {}",
                self.test_command
            ));
        }
        Ok(())
    }

    /// The Pi session spec for this slice, at the provider-neutral boundary.
    pub fn pi_spec(&self) -> PiSessionSpec {
        PiSessionSpec {
            session_id: self.session_id.clone(),
            worktree: self.worktree.clone(),
            role: AgentRole::Implementer,
            policy: RolePolicy::for_role(AgentRole::Implementer),
            provider: self.provider.clone(),
            model: self.profile.clone(),
            reasoning_tokens: FIXTURE_REASONING_TOKENS,
            sampling: SamplingProfile::qwen_thinking(),
            extra_rules: vec![
                format!("Edit only {}.", self.target_file),
                "Run no test yourself: the harness runs the one targeted test.".to_string(),
                "Stop as soon as the change is made.".to_string(),
            ],
            stable_prefix_hash: String::new(),
            max_context_tokens: FIXTURE_MAX_CONTEXT_TOKENS,
            allow_forks: false,
        }
    }

    /// The Pi argv for this slice, built by the existing adapter boundary.
    pub fn pi_argv(&self) -> Result<Vec<String>, String> {
        build_pi_argv(&self.pi_spec())
    }
}

/// How a slice ended. Exactly one of these is recorded per run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Termination {
    /// One implementation file changed, the targeted test passed, budget held.
    AcceptanceMet,
    /// The turn budget ran out before the session declared completion.
    MaxTurns,
    /// The wall-clock budget ran out before the session declared completion.
    WallClock,
    /// The session touched something other than the one target file.
    ScopeViolation,
    /// More than [`MAX_TEST_COMMANDS`] test commands were requested.
    TestCommandBudget,
    /// The session reported failure.
    SessionFailed,
    /// The session completed but its targeted test failed.
    TestFailed,
    /// Pi reported an error event.
    HarnessError,
}

impl Termination {
    pub fn as_str(&self) -> &'static str {
        match self {
            Termination::AcceptanceMet => "acceptance_met",
            Termination::MaxTurns => "max_turns",
            Termination::WallClock => "wall_clock",
            Termination::ScopeViolation => "scope_violation",
            Termination::TestCommandBudget => "test_command_budget",
            Termination::SessionFailed => "session_failed",
            Termination::TestFailed => "test_failed",
            Termination::HarnessError => "harness_error",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "acceptance_met" => Termination::AcceptanceMet,
            "max_turns" => Termination::MaxTurns,
            "wall_clock" => Termination::WallClock,
            "scope_violation" => Termination::ScopeViolation,
            "test_command_budget" => Termination::TestCommandBudget,
            "session_failed" => Termination::SessionFailed,
            "test_failed" => Termination::TestFailed,
            "harness_error" => Termination::HarnessError,
            _ => return None,
        })
    }

    /// Only a met acceptance criteria counts as a slice that worked.
    pub fn is_success(&self) -> bool {
        matches!(self, Termination::AcceptanceMet)
    }

    /// Terminations caused by a configured budget rather than by the session.
    pub fn is_budget(&self) -> bool {
        matches!(self, Termination::MaxTurns | Termination::WallClock)
    }
}

/// The persisted record of one slice run (issue #3320 AC 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceLedger {
    pub schema_version: u32,
    pub run_id: String,
    pub profile: String,
    pub node_id: String,
    pub session_id: String,
    pub worktree: String,
    pub turns: u32,
    pub tool_calls: u32,
    pub wall_clock_ms: u64,
    pub max_turns: u32,
    pub max_wall_clock_seconds: u64,
    pub files_changed: Vec<String>,
    pub test_commands: Vec<String>,
    pub tests_passed: bool,
    pub terminated_by: Termination,
    pub within_budget: bool,
    pub session_closed: bool,
}

impl SliceLedger {
    /// One-line JSON, key order fixed by field declaration order.
    pub fn to_json(&self) -> String {
        // Serialization of these fields cannot fail; an error here would mean
        // a schema bug, which is a panic-worthy invariant, not a run failure.
        serde_json::to_string(self).unwrap_or_else(|err| panic!("ledger serialization: {err}"))
    }

    /// Parse a driver-written ledger. Unknown `terminated_by` values are
    /// rejected, so a driver cannot invent a termination quietly.
    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text.trim()).map_err(|err| format!("invalid slice ledger: {err}"))
    }

    /// Schema-level validity: versioned, attributable, terminated for a known
    /// reason. Callers that also require success check [`Termination`] and
    /// [`SliceLedger::within_budget`] themselves — a violating run still has to
    /// be writable, or the violation would be unrecorded.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != SLICE_LEDGER_SCHEMA_VERSION {
            return Err(format!(
                "unsupported slice ledger schema version {}",
                self.schema_version
            ));
        }
        for (label, value) in [
            ("run id", &self.run_id),
            ("profile", &self.profile),
            ("node id", &self.node_id),
            ("session id", &self.session_id),
            ("worktree", &self.worktree),
        ] {
            if value.trim().is_empty() {
                return Err(format!("slice ledger requires a {label}"));
            }
        }
        if self.max_turns == 0 || self.max_wall_clock_seconds == 0 {
            return Err("slice ledger requires non-zero budgets".to_string());
        }
        if self.terminated_by == Termination::AcceptanceMet && !self.session_closed {
            return Err("a successful slice ledger must record a closed session".to_string());
        }
        Ok(())
    }

    /// The AC 4 fields as (key, value) pairs, for reports and assertions.
    pub fn mandatory_fields(&self) -> [(&'static str, String); 5] {
        [
            ("profile", self.profile.clone()),
            ("node_id", self.node_id.clone()),
            ("turns", self.turns.to_string()),
            ("tool_calls", self.tool_calls.to_string()),
            ("wall_clock_ms", self.wall_clock_ms.to_string()),
        ]
    }

    /// Exit code convention shared by the driver and the bats suite: 0 only for
    /// a successful, budgeted, closed run.
    pub fn exit_code(&self) -> i32 {
        if self.terminated_by.is_success() && self.tests_passed && self.within_budget {
            0
        } else {
            1
        }
    }
}

/// One slice run, observed through the events the harness forwards.
///
/// `now_ms` is supplied by the caller at every step so the state machine is a
/// pure function of (request, events, clock): the budgets are testable without
/// waiting for a wall clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceRun {
    request: SliceRequest,
    turns: u32,
    tool_calls: u32,
    files_changed: Vec<String>,
    test_commands: Vec<String>,
    tests_passed: bool,
    declared_complete: bool,
    started_ms: u64,
    wall_clock_ms: u64,
    terminated: Option<Termination>,
    session_closed: bool,
}

impl SliceRun {
    /// Begin a run at `now_ms`. The request is validated by the caller's
    /// [`SliceRequest::validate`]; a run started from an invalid request still
    /// produces a ledger describing why it failed.
    pub fn start(request: &SliceRequest, now_ms: u64) -> Self {
        Self {
            request: request.clone(),
            turns: 0,
            tool_calls: 0,
            files_changed: Vec::new(),
            test_commands: Vec::new(),
            tests_passed: false,
            declared_complete: false,
            started_ms: now_ms,
            wall_clock_ms: 0,
            terminated: None,
            session_closed: false,
        }
    }

    pub fn request(&self) -> &SliceRequest {
        &self.request
    }

    pub fn turns(&self) -> u32 {
        self.turns
    }

    pub fn tool_calls(&self) -> u32 {
        self.tool_calls
    }

    pub fn files_changed(&self) -> &[String] {
        &self.files_changed
    }

    pub fn test_commands(&self) -> &[String] {
        &self.test_commands
    }

    pub fn terminated(&self) -> Option<Termination> {
        self.terminated
    }

    pub fn within_budget(&self) -> bool {
        self.turns <= self.request.budget.max_turns
            && self.wall_clock_ms <= self.request.budget.wall_clock_ms()
    }

    /// Forward one turn's Pi event lines (the `kind key=value` wire format
    /// parsed by [`crate::aar::parse_pi_event`]).
    pub fn record_turn_lines(
        &mut self,
        now_ms: u64,
        lines: &[&str],
    ) -> Result<Option<Termination>, String> {
        let mut events = Vec::with_capacity(lines.len());
        for line in lines {
            events.push(parse_pi_event(line)?);
        }
        Ok(self.record_turn(now_ms, &events))
    }

    /// Record one model turn and the events it produced, then evaluate
    /// termination. Returns the termination if this turn ended the run.
    pub fn record_turn(&mut self, now_ms: u64, events: &[PiEvent]) -> Option<Termination> {
        if let Some(reason) = self.terminated {
            return Some(reason);
        }
        self.turns += 1;
        self.wall_clock_ms = self.elapsed(now_ms);

        let folded = fold_events(&self.request.session_id, AgentRole::Implementer, events);
        self.tool_calls = self.tool_calls.saturating_add(folded.tool_calls);
        for path in &folded.files_edited {
            if !self.files_changed.contains(path) {
                self.files_changed.push(path.clone());
            }
        }
        if !folded.errors.is_empty() {
            self.terminated = Some(Termination::HarnessError);
        }
        if let Some(PiEvent::Result { success, .. }) = events
            .iter()
            .rev()
            .find(|event| matches!(event, PiEvent::Result { .. }))
        {
            self.declared_complete = *success;
            if !*success {
                self.terminated = Some(Termination::SessionFailed);
            }
        }

        self.evaluate()
    }

    /// Record the harness running the one targeted test command. Running a
    /// second one is a budget breach; running any test after the run terminated
    /// is a driver bug and reported as an error, not silently absorbed.
    pub fn record_test_command(
        &mut self,
        now_ms: u64,
        command: &str,
        passed: bool,
    ) -> Result<Option<Termination>, String> {
        if let Some(reason) = self.terminated {
            return Err(format!(
                "run already terminated as {}; targeted test must not run",
                reason.as_str()
            ));
        }
        self.wall_clock_ms = self.elapsed(now_ms);
        if self.test_commands.len() >= MAX_TEST_COMMANDS {
            self.terminated = Some(Termination::TestCommandBudget);
            return Ok(self.terminated);
        }
        self.test_commands.push(command.to_string());
        self.tests_passed = passed;
        if !passed {
            self.terminated = Some(Termination::TestFailed);
        }
        Ok(self.evaluate())
    }

    /// Close the Pi session and produce the ledger. If nothing has terminated
    /// the run by now, the close itself decides the verdict.
    pub fn close(&mut self, now_ms: u64) -> SliceLedger {
        self.wall_clock_ms = self.elapsed(now_ms);
        self.session_closed = true;
        if self.terminated.is_none() {
            self.terminated = Some(self.evaluate_close());
        }
        self.ledger()
    }

    /// The ledger for the current state, before or after close.
    pub fn ledger(&self) -> SliceLedger {
        SliceLedger {
            schema_version: SLICE_LEDGER_SCHEMA_VERSION,
            run_id: self.request.run_id.clone(),
            profile: self.request.profile.clone(),
            node_id: self.request.node_id.clone(),
            session_id: self.request.session_id.clone(),
            worktree: self.request.worktree.clone(),
            turns: self.turns,
            tool_calls: self.tool_calls,
            wall_clock_ms: self.wall_clock_ms,
            max_turns: self.request.budget.max_turns,
            max_wall_clock_seconds: self.request.budget.max_wall_clock_seconds,
            files_changed: self.files_changed.clone(),
            test_commands: self.test_commands.clone(),
            tests_passed: self.tests_passed,
            terminated_by: self.terminated.unwrap_or(Termination::SessionFailed),
            within_budget: self.within_budget(),
            session_closed: self.session_closed,
        }
    }

    fn elapsed(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.started_ms)
    }

    /// Guard breaches outrank budgets, budgets outrank the session's own
    /// verdict. See the module docs.
    fn evaluate(&mut self) -> Option<Termination> {
        if self.terminated.is_some() {
            return self.terminated;
        }
        if self.files_changed.len() > MAX_IMPLEMENTATION_FILES
            || self
                .files_changed
                .iter()
                .any(|path| *path != self.request.target_file)
        {
            self.terminated = Some(Termination::ScopeViolation);
            return self.terminated;
        }
        if self.wall_clock_ms >= self.request.budget.wall_clock_ms() {
            self.terminated = Some(Termination::WallClock);
            return self.terminated;
        }
        if self.turns >= self.request.budget.max_turns {
            self.terminated = Some(Termination::MaxTurns);
        }
        self.terminated
    }

    /// Verdict when the session ends inside budget without a guard breach.
    fn evaluate_close(&self) -> Termination {
        if !self.declared_complete {
            return Termination::SessionFailed;
        }
        if self.files_changed.as_slice() != [self.request.target_file.as_str()] {
            return Termination::ScopeViolation;
        }
        if self.test_commands.len() != MAX_TEST_COMMANDS {
            return Termination::TestCommandBudget;
        }
        if !self.tests_passed {
            return Termination::TestFailed;
        }
        Termination::AcceptanceMet
    }
}
