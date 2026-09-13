//! Pattern-based process termination that cannot kill its own session
//! (issue #4448).
//!
//! `pkill -f <pattern>` matches against full command lines, and the shell
//! running the `pkill` carries the pattern in *its own* command line: the
//! pattern matches the invoking shell, which dies with it. The signature is
//! exit 144 and no output at all — including none of the work queued after
//! the kill in the same command. This has recurred four times in one session
//! *after* a written prohibition existed, which is the finding the helper
//! exists for: rules that must be recalled at the moment of writing a
//! routine-looking command do not fire; mechanisms do
//! (`AGENTS.d/4449-silent-false-negatives-recall-rules-are-not-fixes.md`).
//!
//! Two layers, because neither alone is enough:
//!
//! - **Bracketing** — `bracket_pattern` rewrites the pattern's first
//!   character into a one-character class (`cargo.*test` ->
//!   `[c]argo.*test`). The killer's own argv then carries the *bracketed*
//!   text, which the bracketed regex does not match (it expects `c` where
//!   the text has `[`). This makes the matching tool safe to run.
//! - **Session exclusion** — the invoking shell's argv carries the *plain*
//!   pattern, which the bracketed regex *does* match. So the killer also
//!   excludes its own session — itself and every ancestor, the processes
//!   whose argv carries the plain pattern — from the target set, and kills
//!   the remaining matches **by pid**.
//!
//! The public surface is `kill_matching`, the implementation of the
//! `autospec process-kill` command.

use std::collections::BTreeSet;
use std::process::Command;

use nix::sys::signal::{kill, Signal};
use nix::unistd::{getpid, Pid};

/// Why a pattern cannot be auto-bracketed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BracketError {
    /// The pattern is empty: there is nothing to match, and killing
    /// "everything" is not an acceptable interpretation.
    Empty,
    /// The first character is a regex metacharacter whose meaning changes
    /// when bracketed (`^` anchors, `(` groups, `*` quantifies, `]` closes a
    /// class, ...). Bracketing it would silently change what the pattern
    /// matches; the caller must rewrite the pattern or bracket it by hand.
    FirstCharNotBracketable { ch: char },
}

impl std::fmt::Display for BracketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "the pattern is empty; refusing to match everything"),
            Self::FirstCharNotBracketable { ch } => write!(
                f,
                "the first character '{ch}' is a regex metacharacter that changes meaning \
                 when bracketed; rewrite the pattern or bracket it by hand"
            ),
        }
    }
}

/// Metacharacters that change meaning when wrapped in a one-character
/// class. `[` is deliberately absent: a pattern that already starts with a
/// class is treated as already bracketed (idempotent pass-through).
const NOT_BRACKETABLE_FIRST: [char; 9] = ['^', '(', ')', '*', '+', '?', '|', '{', ']'];

/// Rewrite the pattern's first character into a one-character class so the
/// killer's own argv (which carries the rewritten text) cannot match the
/// rewritten regex.
///
/// - `cargo.*test` -> `[c]argo.*test`
/// - `[c]argo` -> `[c]argo` (already bracketed; idempotent)
/// - empty -> `Err(Empty)`
/// - `^foo`, `]foo`, ... -> `Err(FirstCharNotBracketable)`
pub fn bracket_pattern(pattern: &str) -> Result<String, BracketError> {
    let first = pattern
        .chars()
        .next()
        .ok_or(BracketError::Empty)?;
    if first == '[' {
        return Ok(pattern.to_string());
    }
    if NOT_BRACKETABLE_FIRST.contains(&first) {
        return Err(BracketError::FirstCharNotBracketable { ch: first });
    }
    let mut out = String::with_capacity(pattern.len() + 2);
    out.push('[');
    out.push(first);
    out.push(']');
    out.push_str(&pattern[first.len_utf8()..]);
    Ok(out)
}

/// What `kill_matching` did, for the operator-facing report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KillReport {
    /// The pattern as given by the caller.
    pub pattern: String,
    /// The pattern after `bracket_pattern`.
    pub bracketed: String,
    /// The signal name that was sent.
    pub signal: String,
    /// The pids the signal was sent to successfully.
    pub killed: Vec<u32>,
    /// Pids `pgrep` reported that were dropped as the killer's own session
    /// (itself or an ancestor): their argv carries the plain pattern.
    pub excluded: Vec<u32>,
    /// (pid, reason) for pids the signal could not be sent to.
    pub failed: Vec<(u32, String)>,
}

impl KillReport {
    /// True when the kill reached at least one process. A kill that matches
    /// nothing is a false negative (wrong pattern, already dead, typo), not
    /// a clean state — the caller must treat it as a warning, not success.
    pub fn matched_any(&self) -> bool {
        !self.killed.is_empty() || !self.failed.is_empty()
    }

    /// The operator-facing line.
    pub fn line(&self) -> String {
        let mut out = format!(
            "process-kill: pattern '{}' -> '{}' (signal {})",
            self.pattern, self.bracketed, self.signal
        );
        if self.killed.is_empty() && self.failed.is_empty() {
            out.push_str(": matched 0 pid(s) — a kill that matches nothing is a false \
                         negative, not a clean state; check the pattern")
        } else {
            if !self.killed.is_empty() {
                out.push_str(&format!(
                    ": killed {} pid(s) ({})",
                    self.killed.len(),
                    self.killed.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")
                ));
            }
            if !self.failed.is_empty() {
                out.push_str(&format!(
                    "; {} failed ({})",
                    self.failed.len(),
                    self.failed
                        .iter()
                        .map(|(p, r)| format!("{p}: {r}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if !self.excluded.is_empty() {
                out.push_str(&format!(
                    "; excluded {} session pid(s) ({})",
                    self.excluded.len(),
                    self.excluded.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")
                ));
            }
        }
        out
    }
}

/// Why `kill_matching` failed before it could produce a report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminationError {
    /// The pattern could not be bracketed.
    Bracket(BracketError),
    /// The process-table query could not be run.
    Query(String),
}

impl std::fmt::Display for TerminationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bracket(b) => write!(f, "cannot bracket the pattern: {b}"),
            Self::Query(q) => write!(f, "process-table query failed: {q}"),
        }
    }
}

impl std::error::Error for TerminationError {}

/// The pids in the killer's own session: itself and every ancestor. These
/// are the processes whose argv carries the plain pattern (the operator
/// typed it, the shell running the command has it in its command line).
/// They must never be targets, whatever the pattern matches.
fn own_session() -> BTreeSet<u32> {
    let mut session = BTreeSet::new();
    let mut pid = getpid();
    while pid.as_raw() > 1 {
        session.insert(pid.as_raw() as u32);
        match parent_of(pid) {
            Some(parent) => pid = parent,
            None => break,
        }
    }
    session
}

/// The parent pid of `pid`, via a portable `ps` lookup (Linux, macOS, and
/// the BSDs all ship `ps -o ppid=`). `None` when the lookup fails or the
/// pid has no parent (pid 1, or already gone).
fn parent_of(pid: Pid) -> Option<Pid> {
    let out = Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let token = text.split_whitespace().next()?.trim();
    let raw: i32 = token.parse().ok()?;
    (raw > 1).then_some(Pid::from_raw(raw))
}

/// Terminate the processes whose command line matches `pattern`, with the
/// two safety layers described in the module docs: the pattern is bracketed
/// before matching (so the matching tool cannot match its own argv), and
/// the killer's own session is excluded from the targets (so the invoking
/// shell, whose argv carries the plain pattern, survives). Remaining
/// matches are killed by pid with `signal` (convention: SIGTERM).
///
/// The returned report is non-empty even when nothing matched: a kill that
/// matches zero processes is reported as such, never as a silent success.
pub fn kill_matching(pattern: &str, signal: Signal) -> Result<KillReport, TerminationError> {
    let bracketed = bracket_pattern(pattern).map_err(TerminationError::Bracket)?;
    let session = own_session();

    // `pgrep -f` with the bracketed pattern: pgrep's own argv carries the
    // bracketed text, which the bracketed regex does not match, so the
    // matcher cannot find itself. Exit 0 = matches, 1 = none, else error.
    let out = Command::new("pgrep")
        .args(["-f", &bracketed])
        .output()
        .map_err(|e| TerminationError::Query(e.to_string()))?;
    let code = out.status.code().unwrap_or(-1);
    if code != 0 && code != 1 {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(TerminationError::Query(format!(
            "pgrep exited {code}: {stderr}"
        )));
    }

    let mut candidates: Vec<u32> = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect();

    let mut excluded = Vec::new();
    candidates.retain(|pid| {
        if session.contains(pid) {
            excluded.push(*pid);
            false
        } else {
            true
        }
    });
    candidates.sort();
    excluded.sort();

    let mut killed = Vec::new();
    let mut failed = Vec::new();
    for pid in candidates {
        match kill(Pid::from_raw(pid as i32), signal) {
            Ok(()) => killed.push(pid),
            Err(e) => failed.push((pid, e.to_string())),
        }
    }

    Ok(KillReport {
        pattern: pattern.to_string(),
        bracketed,
        signal: signal_name(signal),
        killed,
        excluded,
        failed,
    })
}

/// The signal name, the way the operator typed it (TERM, KILL, ...).
fn signal_name(signal: Signal) -> String {
    use Signal::*;
    match signal {
        SIGHUP => "HUP".into(),
        SIGINT => "INT".into(),
        SIGQUIT => "QUIT".into(),
        SIGKILL => "KILL".into(),
        SIGTERM => "TERM".into(),
        SIGUSR1 => "USR1".into(),
        SIGUSR2 => "USR2".into(),
        other => format!("{other:?}"),
    }
}


