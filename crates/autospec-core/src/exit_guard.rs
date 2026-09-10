//! A guard that gates an action on an external command's exit status must
//! distinguish "the command said no" from "the command failed" (issue #4009).
//!
//! The incident: the conversion pass skipped any issue that already had a
//! branch or a PR, by running
//!
//! ```bash
//! headRefName | grep -qE "(issue-|conv/|cp/|probe/)$n(\\b|-)"
//! ```
//!
//! and treating every non-zero result as "no match, proceed". In an
//! interactive shell that is true for `grep`: `0` is a match, `1` is no match,
//! and the guard's `else` branch is only ever reached for a genuine no-match.
//! In the runner's environment, though, `grep` was a shell *function* wrapper
//! that exited `2` on input it could not parse, and the guard read that `2` as
//! the same "no match" as a real no-match — so an issue that already had a PR
//! was dispatched to implementers while its PR sat open. The line works in one
//! shell and misbehaves in the script's own environment, and no one noticed
//! because both outcomes fell down the same exit path.
//!
//! This is the same defect family as the other "absence collapsed into the
//! success value" backlogs (#3764: a probe read a failed ssh as "no patch";
//! #3768: a timed-out gate read "no output" as "all fixed"): a result that is
//! not the tool's documented answer was never given its own representation, so
//! it was silently read as the tool's "no".
//!
//! The invariants this module makes checkable:
//!
//! 1. **A non-zero status that is not the tool's documented negative status is
//!    an *error*, and an error is never a passing precondition.** A status is
//!    a match only if it is the documented match status, a no-match only if it
//!    is the documented no-match status, and an error otherwise.
//!    ([`read_exit`], [`decide`]).
//! 2. **A tool whose semantics vary by implementation must be bound by
//!    absolute path or have its implementation asserted once, before any guard
//!    runs on its output.** A shell *function* named `grep` is not the `grep`
//!    the guard was written against, and a bare `PATH` lookup with no
//!    assertion is the same risk one `PATH` edit away.
//!    ([`ToolBinding`], [`binding_is_safe`]).
//! 3. **A guard's pattern must mean the same thing in every implementation.**
//!    `\b`, `\d`, and `\s` (and their companions) are not POSIX; they are
//!    different regular expressions — or none at all — in different `grep` and
//!    `sed` builds, so a guard that relies on them is a different guard in a
//!    different environment. ([`portable_pattern_violations`]).
//!
//! The module is pure: the caller runs the external command in whatever
//! environment the guard actually lives in, observes its exit status, and
//! reports it here. The module never spawns a process and never reads a clock;
//! it only decides what the observed status *means* for the guarded action.

use serde::{Deserialize, Serialize};

/// The documented exit-status contract of the external tool a guard gates on.
///
/// Most text tools share one convention: a positive answer has a fixed status
/// (conventionally `0`), and the tool documents exactly **one** further status
/// meaning "ran, but no". Everything else is an error. The guard needs this
/// contract to separate "the tool said no" from "the tool broke" — the two are
/// indistinguishable without it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ToolContract {
    /// The status meaning "found / matched / true". Conventionally `0`.
    pub match_status: i32,
    /// The documented status meaning "ran, but no / not found / false". This
    /// is the *only* status other than [`match_status`] that a guard may read
    /// as "no match". Every other status is an error.
    pub no_match_status: i32,
}

impl ToolContract {
    /// The `grep` / `fgrep` / `egrep` family: `0` a match, `1` no match, and
    /// any other status (notably `2`) an error — unreadable input, a bad
    /// pattern, a write failure.
    pub const GREP: Self = Self {
        match_status: 0,
        no_match_status: 1,
    };
    /// `test` / `[`: `0` true, `1` false. Any other status is an error.
    pub const TEST: Self = Self {
        match_status: 0,
        no_match_status: 1,
    };
    /// `comm`, `diff`, and the other "report a difference" tools: `0` no
    /// difference, `1` a difference, `2` an error.
    pub const DIFF: Self = Self {
        match_status: 0,
        no_match_status: 1,
    };

    /// Whether `status` is one of this tool's two documented answers.
    ///
    /// A status that is *not* documented is not a quiet "no": it is an error,
    /// and the guard must treat it as such ([`read_exit`]).
    pub fn is_documented(&self, status: i32) -> bool {
        status == self.match_status || status == self.no_match_status
    }
}

/// What a tool's observed exit status means for the guard that gates on it.
///
/// This is the representation the incident lacked. "No match" and "error" are
/// separate variants, so code that proceeds on [`NoMatch`] *cannot* accidentally
/// proceed on [`Errored`]: the two no longer share an exit path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitReading {
    /// The tool found what it was asked about. The guard's "already exists"
    /// condition holds, so the guarded action is blocked (it would duplicate
    /// work).
    Matched,
    /// The tool ran and answered its documented "no": no match. This is the
    /// only reading that lets the guarded action proceed.
    NoMatch,
    /// The tool exited with a status that is neither its match status nor its
    /// documented no-match status. The tool is broken, or is not the tool the
    /// guard was written against (a wrapper function, a differing build). The
    /// guard cannot tell "no" from "I failed", so it must not read this as a
    /// pass.
    Errored { status: i32 },
}

impl ExitReading {
    /// Only a documented no-match lets the guarded action proceed. A match
    /// blocks because the thing already exists; an error blocks because the
    /// guard is broken. Both blocked readings are distinct from `NoMatch`.
    pub fn proceeds(&self) -> bool {
        matches!(self, Self::NoMatch)
    }

    /// Whether the tool exited in error — the fail-closed case.
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Errored { .. })
    }
}

/// The guard's decision about the guarded action, rendered for a log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardVerdict {
    /// The tool answered its documented "no"; the action's precondition
    /// (nothing already exists) holds, so the action may proceed.
    Proceed,
    /// The tool found what it was asked about; the action's precondition is
    /// already met, so the action is blocked as redundant.
    AlreadyExists,
    /// The tool errored; the guard cannot tell "no" from "I failed", so it
    /// holds the action fail-closed. Distinct from [`GuardVerdict::AlreadyExists`]:
    /// the action is blocked because the *guard* is broken, not because the
    /// thing already exists — and the status is kept so the log line can say
    /// which one it was.
    FailClosed { status: i32 },
}

impl GuardVerdict {
    /// Whether the guarded action proceeds. Only [`GuardVerdict::Proceed`]
    /// does; both blocked states hold.
    pub fn proceeds(&self) -> bool {
        matches!(self, Self::Proceed)
    }

    /// The one-line report a wrapper prints in the environment it actually
    /// runs in — the "diagnose in the script's own context" line. Naming the
    /// tool and the exact status is what makes the incident visible in a log:
    /// "no match" and "exit 2" no longer read the same.
    pub fn line(&self, tool: &str) -> String {
        match self {
            Self::Proceed => format!("{tool}: no match — proceed"),
            Self::AlreadyExists => format!("{tool}: match — already exists, hold"),
            Self::FailClosed { status } => {
                format!("{tool}: exit {status} is not a documented answer — fail closed, hold")
            }
        }
    }
}

/// Classify a tool's observed exit status against its documented contract.
///
/// This is the single primitive that makes the #4009 incident impossible:
///
/// - `status == contract.match_status` → [`ExitReading::Matched`];
/// - `status == contract.no_match_status` → [`ExitReading::NoMatch`];
/// - otherwise → [`ExitReading::Errored`].
///
/// A "no match" and an "error" are different readings, so a guard that
/// proceeds on `NoMatch` cannot accidentally proceed on `Errored`. The `grep`
/// example: `0` is a match, `1` is a no-match, and `2` — the wrapper function
/// the runner actually ran — is an error, not a no-match.
pub fn read_exit(status: i32, contract: ToolContract) -> ExitReading {
    if status == contract.match_status {
        ExitReading::Matched
    } else if status == contract.no_match_status {
        ExitReading::NoMatch
    } else {
        ExitReading::Errored { status }
    }
}

/// Decide whether the guarded action proceeds, from the tool's reading.
///
/// Only [`ExitReading::NoMatch`] proceeds. A match blocks because the thing
/// already exists; an error blocks because the guard is broken and cannot
/// answer. The two blocked verdicts stay distinct so a log line — and a test —
/// can tell a real "already has a PR" from a guard that misfired on a `2`.
pub fn decide(reading: ExitReading) -> GuardVerdict {
    match reading {
        ExitReading::Matched => GuardVerdict::AlreadyExists,
        ExitReading::NoMatch => GuardVerdict::Proceed,
        ExitReading::Errored { status } => GuardVerdict::FailClosed { status },
    }
}

/// How a guard resolves the external tool it depends on.
///
/// The #4009 guard broke because its environment's `grep` was a shell
/// *function*, not the `grep` binary the guard was written against. Whether a
/// guard may trust a tool's exit codes is a property of *how the guard bound
/// the tool*, and this makes that binding checkable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolBinding {
    /// An absolute path to the binary. Safe: the guard is written against
    /// exactly this executable, regardless of `PATH` or any shell function.
    AbsolutePath { path: String },
    /// A bare command name resolved through `PATH`, with no assertion that the
    /// resolved implementation is the one the guard was written against.
    /// Unsafe: one differing `PATH` entry changes the tool's semantics
    /// silently.
    PathLookup { name: String },
    /// A bare name whose implementation the guard asserted once, before any
    /// check ran on its output — the "assert the implementation once at
    /// startup" half of the invariant. `evidence` records *what* was asserted
    /// (a resolved absolute path, a version string), so the assertion survives
    /// in the report.
    Asserted { name: String, evidence: String },
    /// A shell *function* or alias masquerading as the tool. This is the
    /// #4009 defect: a function named `grep` is not `grep`, and its exit codes
    /// are not `grep`'s. A guard must refuse to run on a function binding.
    ShellFunction { name: String },
}

/// Whether a guard may rely on this binding for its tool's exit codes.
///
/// An absolute path and an asserted lookup are safe: the guard is written
/// against a specific implementation. A bare, unasserted `PATH` lookup and a
/// shell function are not: a differing `PATH` or a wrapper function silently
/// changes the tool's semantics, which is exactly how the #4009 guard read a
/// `2` as a no-match.
pub fn binding_is_safe(binding: &ToolBinding) -> bool {
    match binding {
        ToolBinding::AbsolutePath { .. } | ToolBinding::Asserted { .. } => true,
        ToolBinding::PathLookup { .. } | ToolBinding::ShellFunction { .. } => false,
    }
}

/// The note a guard records next to its tool binding — why it is trusted, or
/// why it must be.
pub fn binding_note(binding: &ToolBinding) -> String {
    match binding {
        ToolBinding::AbsolutePath { path } => format!("bound by absolute path: {path}"),
        ToolBinding::Asserted { name, evidence } => {
            format!("implementation asserted once at startup: {name} ({evidence})")
        }
        ToolBinding::PathLookup { name } => format!(
            "bare {name} from PATH, unasserted: a differing build or PATH entry can change its exit codes — bind by absolute path or assert the implementation"
        ),
        ToolBinding::ShellFunction { name } => format!(
            "{name} is a shell function, not the {name} binary: its exit codes are not the tool's — replace the binding before gating on it"
        ),
    }
}

/// Find non-POSIX escape sequences in a guard's pattern, in order of first
/// appearance.
///
/// `\b`, `\d`, and `\s` (and their companions `\B \D \S \w \W`) are not POSIX
/// ERE: they are different regular expressions — or match nothing at all — in
/// different `grep` and `sed` builds. A guard whose pattern relies on them is
/// a different guard in a different environment, the same class of defect that
/// let the #4009 guard behave differently in a runner than in an interactive
/// shell. A portable guard uses explicit anchoring and POSIX character classes
/// (`[[:space:]]`, `[0-9]`, `(^|[^A-Za-z0-9_])` in place of `\b`) instead.
///
/// Escaped backslashes are handled: `\\b` is a literal backslash followed by a
/// literal `b`, not a word boundary, and is not flagged.
pub fn portable_pattern_violations(pattern: &str) -> Vec<String> {
    // The non-POSIX escapes a guard must not rely on. The issue calls out
    // `\b`, `\d`, and `\s`; the companion forms are the same class.
    const OFFENDERS: &[u8] = b"bBsSwWdD";
    let bytes = pattern.as_bytes();
    let mut found: Vec<String> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            i += 1;
            continue;
        }
        // Count the run of consecutive backslashes starting here. An even run
        // is all literal backslash pairs, so the following character (if any)
        // is a literal, not an escape target. An odd run leaves exactly one
        // live escape, whose target is the byte at `k` (if present).
        let run_start = i;
        let mut k = i;
        while k < bytes.len() && bytes[k] == b'\\' {
            k += 1;
        }
        let run = k - run_start;
        if run % 2 == 1 && k < bytes.len() && OFFENDERS.contains(&bytes[k]) {
            let token = format!("\\{}", bytes[k] as char);
            if !found.iter().any(|t| t == &token) {
                found.push(token);
            }
        }
        i = k;
    }
    found
}

/// Whether a guard's pattern is portable across `grep`/`sed` builds: it
/// contains no non-POSIX escape.
pub fn pattern_is_portable(pattern: &str) -> bool {
    portable_pattern_violations(pattern).is_empty()
}
