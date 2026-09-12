//! An empty file you created is not evidence about a process you did not
//! instrument, and `pgrep -f` matches its own caller (issue #4145).
//!
//! A conversion pass was declared dead twice on evidence that could not have
//! shown it either way. It had in fact run for 88 minutes and converted 8
//! patches.
//!
//! The pass was launched with its stdout redirected to a capture file:
//!
//! ```sh
//! setsid --fork bash -c 'exec bash convpass.sh 3194 3210 ...' > "$T/convpassW.log" 2>&1
//! ```
//!
//! `convpassW.log` stayed at 0 bytes and was read as "it produced nothing and
//! died." But the script does not log to stdout:
//!
//! ```sh
//! say(){ echo "$(date +%H:%M:%S) $*" >> "$LOG"; }
//! LOG="${LOG:-$T/convpass.log}"
//! ```
//!
//! Every line went to `$T/convpass.log`, which showed the pass finishing
//! normally: `converted=8 held=4 skipped=1`.
//!
//! The second error confirmed the first. Before declaring the pass dead, its
//! liveness was checked:
//!
//! ```sh
//! pgrep -f 'convpass.sh 3194'   # -> 3142082 3142084
//! ```
//!
//! Those PIDs were the shell running that very `pgrep`: its own command line
//! contains the string `convpass.sh 3194`, so the pattern matched itself. The
//! check reports "alive" whenever it is run and is therefore incapable of
//! reporting "dead." Combined with the empty log, this produced a confident
//! and completely wrong conclusion in both directions within the same
//! iteration — first "alive" when the evidence was self-referential, then
//! "dead" when the process had legitimately finished.
//!
//! The three invariants, each made checkable:
//!
//! 1. **A redirect is not evidence about an unrouted stream.** Before treating
//!    an empty capture file as a result, confirm the
//!    reader's source is the place the program actually writes
//!    ([`capture_routed`]). A program with its own log destination leaves a
//!    wrapper's capture empty on every run, success and failure alike —
//!    useless precisely when it looks most damning
//!    ([`unrouted_capture_finding`]). Where a wrapper needs the output, read
//!    the program's own log (`LOG=... convpass.sh`) rather than redirecting a
//!    stream it never uses.
//! 2. **A liveness check must not be able to match itself.** `pgrep -f
//!    PATTERN` scans full command lines, including the command line of the
//!    shell invoking it, so any pattern drawn from the arguments just used
//!    matches the caller ([`LivenessCheck::Pattern`]). Check by PID, recorded
//!    at launch, and verify with `kill -0` ([`LivenessCheck::Pid`]); where a
//!    pattern is unavoidable, exclude the caller and require the match to be
//!    the program, not a shell whose argv quotes it. A pattern check whose
//!    only matches are its own caller is [`Liveness::SelfReferential`]
//!    ([`self_referential_finding`]).
//! 3. **"No output" and "not running" need two pieces of evidence.** Neither
//!    implies the other: a finished run produces no new output and is not
//!    running; a wedged run produces no new output and is running; a run
//!    logging elsewhere produces no output *in your file* and is perfectly
//!    healthy. Terminal lines exist to separate these (#4094) — read the log
//!    the program actually writes and look for its terminal line.
//!
//! [`diagnose`] combines the output evidence (invariant 1) and the liveness
//! evidence (invariant 2), and names the claim(s) a "dead and produced
//! nothing" conclusion is not allowed to rest on ([`diagnosis_finding`]).
//!
//! Everything here is pure: no I/O, no clock. The caller supplies the
//! destination, the source, the matches and the terminal line; this module
//! decides what they mean.

// ── Invariant 1: a redirect is not evidence about an unrouted stream ──────

/// Where a program routes the output a wrapper might try to capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputDestination {
    /// The program appends to its own log file (the `LOG=...` case): it never
    /// writes to a stream a wrapper redirected.
    OwnLog { path: String },
    /// The program writes to stdout.
    Stdout,
    /// The program writes to stderr.
    Stderr,
}

/// A stream a wrapper redirected to a capture file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapturedStream {
    Stdout,
    Stderr,
}

/// What the reader is reading as evidence of the program's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceSource {
    /// A capture of a stream the wrapper redirected (e.g. `convpassW.log`).
    CapturedStream(CapturedStream),
    /// The program's own log file, read directly (e.g. `convpass.log`).
    OwnLog { path: String },
}

/// Invariant 1: is the reader's source the place the program actually writes?
///
/// This is the confirmation an empty capture file requires before it is read
/// as a result. A reader looking at a stream the program never writes to is
/// unrouted; a reader reading the program's own log (or the stream it does
/// write to) is routed.
pub fn capture_routed(dest: &OutputDestination, source: &EvidenceSource) -> bool {
    match (dest, source) {
        (OutputDestination::OwnLog { path: a }, EvidenceSource::OwnLog { path: b }) => a == b,
        (OutputDestination::Stdout, EvidenceSource::CapturedStream(CapturedStream::Stdout)) => true,
        (OutputDestination::Stderr, EvidenceSource::CapturedStream(CapturedStream::Stderr)) => true,
        _ => false,
    }
}

fn destination_name(dest: &OutputDestination) -> String {
    match dest {
        OutputDestination::OwnLog { path } => format!("its own log at `{path}`"),
        OutputDestination::Stdout => "stdout".to_string(),
        OutputDestination::Stderr => "stderr".to_string(),
    }
}

fn source_name(source: &EvidenceSource) -> String {
    match source {
        EvidenceSource::CapturedStream(CapturedStream::Stdout) => "the stdout capture".to_string(),
        EvidenceSource::CapturedStream(CapturedStream::Stderr) => "the stderr capture".to_string(),
        EvidenceSource::OwnLog { path } => format!("`{path}`"),
    }
}

/// Invariant 1, as a check: the finding for treating a file as evidence about
/// a program's output when the reader's source is not the place the program
/// writes.
///
/// The incident: `convpassW.log` (a stdout capture) stayed at 0 bytes while
/// `convpass.sh` wrote every line to `$T/convpass.log` via `LOG`. That
/// capture is empty on every run — success and failure alike — so it is
/// useless exactly when it looks most damning. The finding names the source
/// read and the destination the program actually uses, and the remedy: read
/// the program's own log (set `LOG=` to the capture path) rather than
/// redirecting a stream it never uses.
pub fn unrouted_capture_finding(dest: &OutputDestination, source: &EvidenceSource) -> Vec<String> {
    if capture_routed(dest, source) {
        return Vec::new();
    }
    vec![format!(
        "UNROUTED_CAPTURE: {} is not evidence about the program's output — the program \
writes {}, so this file is empty on every run, success and failure alike; read \
the program's own log (set LOG=... to the capture path) rather than redirecting \
a stream it never uses",
        source_name(source),
        destination_name(dest)
    )]
}

// ── Invariant 2: a liveness check must not be able to match itself ────────

/// How a liveness check identifies the process it is checking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LivenessCheck {
    /// By a PID recorded at launch, verified with `kill -0`. A specific
    /// identity: the checker can never be it.
    ///
    /// ```sh
    /// setsid --fork bash -c 'echo $$ > run.pid; exec ./thing' &
    /// kill -0 "$(cat run.pid)"
    /// ```
    Pid { pid: u32 },
    /// By command-line pattern (`pgrep -f PATTERN`). Scans full command lines,
    /// including the command line of the shell invoking it, so any pattern
    /// drawn from the arguments just used also matches the caller.
    Pattern { pattern: String },
}

/// A process a liveness check observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessMatch {
    pub pid: u32,
    /// The full command line (`/proc/<pid>/cmdline`) the pattern matched
    /// against.
    pub cmdline: String,
    /// True when this match is the program being checked. False when it is
    /// the check's own caller — a shell whose argv quotes the pattern — or a
    /// subprocess of it. The caller identifies its own invocation (it knows
    /// `$$` and the command line it ran) and the primitive decides the
    /// verdict from this fact.
    pub is_program: bool,
}

/// What a liveness check concludes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    /// The program is running (a non-self process matched, or the recorded
    /// PID is alive).
    Alive { pid: u32 },
    /// The program is not running (no non-self process matched, or the
    /// recorded PID is gone).
    Dead,
    /// A pattern check whose only matches are its own caller: it reports
    /// "alive" whenever it is run and is therefore incapable of reporting
    /// "dead". The check itself is the defect, not the process.
    SelfReferential,
}

/// Invariant 2: the verdict of a liveness check over the processes it
/// observed.
///
/// A [`LivenessCheck::Pid`] is inherently safe — a specific identity the
/// checker can never match — so a hit is the program and a miss is the
/// program gone. A [`LivenessCheck::Pattern`] must be read against
/// [`ProcessMatch::is_program`]: a non-program match is the check's own
/// caller, and a pattern check whose only matches are its own caller is
/// [`Liveness::SelfReferential`], not [`Liveness::Alive`].
pub fn liveness_verdict(check: &LivenessCheck, matches: &[ProcessMatch]) -> Liveness {
    match check {
        LivenessCheck::Pid { pid } => {
            if matches.iter().any(|m| m.pid == *pid) {
                Liveness::Alive { pid: *pid }
            } else {
                Liveness::Dead
            }
        }
        LivenessCheck::Pattern { .. } => {
            let programs: Vec<u32> = matches
                .iter()
                .filter(|m| m.is_program)
                .map(|m| m.pid)
                .collect();
            if !programs.is_empty() {
                Liveness::Alive { pid: programs[0] }
            } else if matches.is_empty() {
                Liveness::Dead
            } else {
                // Matches exist but none is the program: the pattern matched
                // only shells quoting it — the check matched its own caller.
                Liveness::SelfReferential
            }
        }
    }
}

/// Invariant 2, as a check: the finding for a liveness check that can match
/// itself and did — a pattern check whose only matches are its own caller.
///
/// The incident: `pgrep -f 'convpass.sh 3194'` returned the two PIDs of the
/// shell running that very `pgrep`. Such a check reports "alive" whenever it
/// is run and cannot report "dead," so it is more dangerous than its loud
/// twin (`pkill -f` killing its own invoking shell) — a false "alive" is
/// silent.
pub fn self_referential_finding(check: &LivenessCheck, matches: &[ProcessMatch]) -> Vec<String> {
    if liveness_verdict(check, matches) != Liveness::SelfReferential {
        return Vec::new();
    }
    let pattern = match check {
        LivenessCheck::Pattern { pattern } => pattern.clone(),
        LivenessCheck::Pid { .. } => return Vec::new(),
    };
    vec![format!(
        "SELF_REFERENTIAL_LIVENESS: the liveness check `pgrep -f '{pattern}'` matched \
only its own caller ({} PID(s): shell(s) whose argv quote the pattern) — it \
reports 'alive' whenever it is run and cannot report 'dead'; record the PID \
at launch and verify with `kill -0`, or exclude the caller and require the \
match to be the program",
        matches.len()
    )]
}

// ── Invariant 3: two claims need two pieces of evidence ───────────────────

/// The two independent observations a "dead and produced nothing" conclusion
/// is built from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// Where the program actually routes its output (invariant 1).
    pub dest: OutputDestination,
    /// What the reader is reading as evidence of the program's output
    /// (invariant 1).
    pub source: EvidenceSource,
    /// The program's *actual* log carries its terminal completion line — the
    /// pass finished (#4094). Terminal lines are what separate "finished"
    /// from "wedged."
    pub terminal_line_present: bool,
    /// The verdict of the liveness check (invariant 2).
    pub liveness: Liveness,
}

/// Invariant 3: what "the process is dead and produced nothing" is allowed
/// to conclude, given the two pieces of evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diagnosis {
    /// Not running, and the program's actual log shows it finished: both
    /// claims are supported, each by its own evidence.
    Finished,
    /// A correct liveness check says the program is running: the "not
    /// running" claim is unsupported. The process may be wedged (no new
    /// output) or healthy — read its actual log to tell those apart.
    Running,
    /// A correct liveness check says the program is not running, but the
    /// output evidence is about the wrong source or carries no terminal
    /// line: "not running" is supported but "produced no output" is not —
    /// the reader cannot say what the program produced.
    NotRunningUnknownOutput,
    /// The liveness evidence is self-referential: the check cannot report
    /// "dead," so neither "running" nor "not running" is supported, and the
    /// combined conclusion is undecidable. The reasons say why.
    Undecidable { reasons: Vec<String> },
}

/// Invariant 3: the diagnosis of "the process is dead and produced nothing"
/// from its two independent pieces of evidence.
///
/// A self-referential liveness check supports neither claim, so the whole
/// conclusion is [`Diagnosis::Undecidable`] before any output is weighed.
/// Otherwise the liveness claim decides first: a program a correct check
/// says is running is [`Diagnosis::Running`], and only a program a correct
/// check says is gone can be [`Diagnosis::Finished`]. For a not-running
/// program, the output claim still needs its own evidence — the reader's
/// source must be the one the program writes to, and it must carry the
/// terminal line.
pub fn diagnose(obs: &Observation) -> Diagnosis {
    if obs.liveness == Liveness::SelfReferential {
        return Diagnosis::Undecidable {
            reasons: vec![
                "the liveness check matched only its own caller, so it is incapable of \
reporting 'dead' — neither 'running' nor 'not running' is supported by it"
                    .to_string(),
            ],
        };
    }
    if matches!(obs.liveness, Liveness::Alive { .. }) {
        return Diagnosis::Running;
    }
    // Not running, by a check that can report dead. Now the output claim,
    // which is a separate piece of evidence.
    if !capture_routed(&obs.dest, &obs.source) {
        return Diagnosis::NotRunningUnknownOutput;
    }
    if obs.terminal_line_present {
        Diagnosis::Finished
    } else {
        Diagnosis::NotRunningUnknownOutput
    }
}

/// Invariant 3, as a check: the finding for concluding "the process is dead
/// and produced nothing" from evidence that does not support both claims.
///
/// The conclusion is [`Diagnosis::Finished`]; anything else is a finding
/// naming the unsupported claim(s). The incident concluded "dead and produced
/// nothing" from a self-referential liveness check and an unrouted empty
/// capture — both claims were unsupported.
pub fn diagnosis_finding(obs: &Observation) -> Vec<String> {
    match diagnose(obs) {
        Diagnosis::Finished => Vec::new(),
        Diagnosis::Undecidable { reasons } => vec![format!(
            "UNDECIDABLE_DIAGNOSIS: 'dead and produced nothing' is not supported — \
{}",
            reasons.join("; ")
        )],
        Diagnosis::Running => vec![
            "CONFLATED_CLAIMS: 'dead and produced nothing' is not supported — the process \
is running (a correct liveness check says so), so 'not running' is unsupported; \
read the program's actual log for its state"
                .to_string(),
        ],
        Diagnosis::NotRunningUnknownOutput => vec![
            "CONFLATED_CLAIMS: 'dead and produced nothing' is not supported — 'not running' \
is supported, but 'produced no output' is not: the output was read from a source the \
program never writes to, or the program's log has no terminal line"
                .to_string(),
        ],
    }
}
