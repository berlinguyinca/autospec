//! Distinguishing a hang from deliberation.
//!
//! A stall guard that watches only for source changes kills agents that are
//! working: reading a codebase before the first edit produces no diff for
//! minutes at a time, which is correct behaviour on a design-heavy issue. The
//! test that holds up is liveness, not output. An implementer whose session
//! transcript is still growing is thinking; only silence on *both* the
//! transcript and the child output is a stall.
//!
//! Verified against a real hang: the transcript of the run behind gateway#7
//! stopped growing entirely and its output stayed at 60 bytes, so neither
//! signal rescued it.

/// One observation of an implementer's own activity signals.
///
/// Both byte counts must be read by the supervisor from outside the child (the
/// session transcript file and the child's stdout), never reported by the agent
/// itself: an agent that is wedged is not in a position to say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LivenessSample {
    /// When this observation was taken, in whole seconds since the epoch.
    pub observed_at: u64,
    /// Size of the agent session transcript at `observed_at`.
    pub transcript_bytes: u64,
    /// Bytes the agent has written to its own output at `observed_at`.
    pub output_bytes: u64,
}

impl LivenessSample {
    pub fn new(observed_at: u64, transcript_bytes: u64, output_bytes: u64) -> Self {
        Self {
            observed_at,
            transcript_bytes,
            output_bytes,
        }
    }
}

/// What the two liveness signals say about an implementer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    /// The session transcript grew: the agent is reasoning, not stalled.
    Deliberating,
    /// The transcript is flat but the agent is still writing output.
    Producing,
    /// Neither signal moved, inside the stall window.
    Quiet,
    /// Neither signal moved for at least the stall window.
    Hung,
}

impl Liveness {
    /// Whether the attempt may keep running.
    pub fn is_live(self) -> bool {
        !self.is_stalled()
    }

    /// Whether the attempt has stalled on both liveness signals.
    pub fn is_stalled(self) -> bool {
        self == Liveness::Hung
    }
}

/// Tracks liveness across successive samples of one attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivenessMonitor {
    stall_secs: u64,
    last_sample: LivenessSample,
    last_progress_at: u64,
}

impl LivenessMonitor {
    /// Start watching an attempt from its first observation. The first sample
    /// counts as progress, so the stall window is measured from there.
    pub fn new(stall_secs: u64, first: LivenessSample) -> Self {
        Self {
            stall_secs,
            last_sample: first,
            last_progress_at: first.observed_at,
        }
    }

    /// Record the next observation and return what it means.
    pub fn observe(&mut self, sample: LivenessSample) -> Liveness {
        let transcript_grew = sample.transcript_bytes > self.last_sample.transcript_bytes;
        let output_grew = sample.output_bytes > self.last_sample.output_bytes;
        if transcript_grew || output_grew {
            self.last_progress_at = sample.observed_at;
        }
        self.last_sample = sample;

        if transcript_grew {
            return Liveness::Deliberating;
        }
        if output_grew {
            return Liveness::Producing;
        }
        if self.silence_secs(sample.observed_at) >= self.stall_secs {
            return Liveness::Hung;
        }
        Liveness::Quiet
    }

    /// Seconds since either signal last moved.
    pub fn silence_secs(&self, now: u64) -> u64 {
        now.saturating_sub(self.last_progress_at)
    }

    /// Bytes observed on the last sample, for the release note.
    pub fn last_sample(&self) -> LivenessSample {
        self.last_sample
    }

    /// The stall window this monitor applies.
    pub fn stall_secs(&self) -> u64 {
        self.stall_secs
    }
}
