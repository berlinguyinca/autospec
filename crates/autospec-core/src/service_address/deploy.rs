//! Deployment of a long-running service: revision drift and the redeploy
//! action (issue #4228).
//!
//! The incident: a fix merged to `main`. The deployed service ran as a job
//! with a 24-hour walltime, died five hours after startup, and was never
//! restarted — the merged fix waited 27 days to deploy, and nobody could
//! have told, because:
//!
//! * no endpoint reported the revision the service was running, so
//!   "deployed?" had no answer;
//! * the reconciler's actions were "job exists" and "nothing to do" — a
//!   service running stale code was indistinguishable from one running
//!   current code;
//! * restart safety was discovered during the incident, not tested before
//!   it.
//!
//! The four invariants, mechanical here:
//!
//! 1. A long-running service needs a deployment action that is not "wait
//!    for the job to die." The reconciler's decision is
//!    [`DeployAction::Redeploy`], produced by [`decide`] — not by patience.
//! 2. Merged and deployed are separate observations: the service states the
//!    revision it is running (`branch @ sha`, e.g. `main @ 1a2b…` in
//!    `/healthz` or `/v1/stats`), and [`parse_revision`] reads it. A service
//!    that states no revision is [`RevisionDrift::Unreported`], never
//!    assumed current.
//! 3. Deployment must not require guessing: [`drift`] compares the reported
//!    revision against the expected tip (e.g. `origin/main`) and reports the
//!    difference, and [`reconcile_line`] never prints "nothing to do" while
//!    the running revision is stale or unreported.
//! 4. Restart safety is a precondition of deployability: [`PreconditionLedger`]
//!    records the three checks the incident named — the build works, the
//!    preflight refuses to bind without auth, and the reconciler starts a
//!    replacement — and [`decide`] refuses a redeploy, naming what would
//!    tell it, while any of them is unverified.
//!
//! The postscript of the same incident: the redeploy changed the service's
//! address, and consumers that had captured the old one had to fail. That
//! half already holds — [`super::AddressResolver`] resolves the address from
//! the published record at use time and fails closed when the record is
//! unreadable, so a consumer with a stale captured address is the one that
//! breaks, exactly as it should. This module assumes the same of its caller.
//!
//! Everything here is pure: no I/O, no clock, no scheduler. The caller reads
//! the reported revision from the service and the expected tip from the
//! repository, and passes both in.

use std::collections::BTreeSet;
use std::fmt;

/// What the service states it is running: a branch and the commit sha the
/// build was made from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    /// The branch the build was made from, e.g. `main`.
    pub branch: String,
    /// The commit sha the build was made from (hex, full or abbreviated).
    pub sha: String,
}

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} @ {}", self.branch, self.sha)
    }
}

/// Why a reported revision could not be read.
///
/// Every variant is a *no answer*, not an answer: a report that fails to
/// parse is [`RevisionDrift::Unreported`], never assumed current.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionError {
    /// No `@` separator: not a `branch @ sha` report.
    NoSeparator,
    /// The branch half is empty (or not a single ref token).
    EmptyBranch,
    /// The sha half is not 4–64 hex digits.
    BadSha,
}

impl fmt::Display for RevisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSeparator => f.write_str("no 'branch @ sha' separator in reported revision"),
            Self::EmptyBranch => f.write_str("empty branch in reported revision"),
            Self::BadSha => f.write_str("sha in reported revision is not 4-64 hex digits"),
        }
    }
}

impl std::error::Error for RevisionError {}

/// Is `token` a plausible git sha: 4–64 hex digits (abbreviated or full,
/// sha-1 or sha-256 repositories)?
fn is_sha(token: &str) -> bool {
    (4..=64).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Parse the revision a service reports it is running: `branch @ sha`.
///
/// The documented contract is what the gateway states in `/healthz` or
/// `/v1/stats`, e.g. `main @ 1a2b3c4d`. Anything that does not parse is a
/// [`RevisionError`], and the caller must treat it as "no revision
/// reported", not as "current".
pub fn parse_revision(raw: &str) -> Result<Revision, RevisionError> {
    let (branch, sha) = raw
        .trim()
        .split_once('@')
        .ok_or(RevisionError::NoSeparator)?;
    let branch = branch.trim();
    let sha = sha.trim();
    if branch.is_empty() || branch.contains('@') || branch.contains(char::is_whitespace) {
        return Err(RevisionError::EmptyBranch);
    }
    if !is_sha(sha) {
        return Err(RevisionError::BadSha);
    }
    Ok(Revision {
        branch: branch.to_string(),
        sha: sha.to_string(),
    })
}

/// Whether the reported revision matches the expected tip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionDrift {
    /// The service reports the expected revision.
    Current,
    /// The service reports a different revision: the deployed code is not
    /// the expected one.
    Drifted {
        /// What the service said it is running.
        running: Revision,
        /// What the reconciler expected (e.g. `origin/main`).
        expected: Revision,
    },
    /// The service states no revision: drift cannot be established in
    /// either direction. This is not `Current`.
    Unreported,
}

impl fmt::Display for RevisionDrift {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Current => f.write_str("current"),
            Self::Drifted { running, expected } => {
                write!(f, "drifted: running {running}, expected {expected}")
            }
            Self::Unreported => {
                f.write_str("unreported: the service does not state the revision it is running")
            }
        }
    }
}

/// Compare the revision the service reported against the expected tip.
///
/// Equality is by **sha**: the same commit reported under another branch
/// name (`origin/main` vs `main`) is the same code. `running = None` is the
/// service not stating a revision at all, which is [`RevisionDrift::Unreported`]
/// — the fail-closed answer.
pub fn drift(running: Option<&Revision>, expected: &Revision) -> RevisionDrift {
    match running {
        None => RevisionDrift::Unreported,
        Some(r) if r.sha == expected.sha => RevisionDrift::Current,
        Some(r) => RevisionDrift::Drifted {
            running: r.clone(),
            expected: expected.clone(),
        },
    }
}

/// The restart-safety checks that must be *tested* before a redeploy is
/// allowed. These are the three the incident named, in the order it needed
/// them: killing the old job must be safe because the replacement path was
/// already proven, not discovered while the service is down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Precondition {
    /// The deployed build compiles and starts.
    Build,
    /// The preflight refuses to bind a port without auth, so a half-started
    /// replacement cannot serve unauthenticated.
    RefusesBindWithoutAuth,
    /// The reconciler starts a replacement — the job is not the only copy,
    /// so killing it does not take the service with it.
    ReplacementStarts,
}

impl fmt::Display for Precondition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Build => f.write_str("the build works"),
            Self::RefusesBindWithoutAuth => f.write_str("preflight refuses to bind without auth"),
            Self::ReplacementStarts => f.write_str("the reconciler starts a replacement"),
        }
    }
}

/// Which preconditions have been verified by a test.
///
/// The ledger starts empty and is only ever added to: a precondition is
/// verified by running its check, never by assuming it. An empty ledger is
/// the incident state — restart safety "discovered during the incident" —
/// and [`decide`] refuses against it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreconditionLedger {
    verified: BTreeSet<Precondition>,
}

impl PreconditionLedger {
    /// A ledger with nothing verified yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that one precondition's check passed.
    pub fn record(&mut self, precondition: Precondition) {
        self.verified.insert(precondition);
    }

    /// Was this precondition verified by a test?
    pub fn is_verified(&self, precondition: Precondition) -> bool {
        self.verified.contains(&precondition)
    }

    /// The preconditions still unverified, in declaration order.
    pub fn missing(&self) -> Vec<Precondition> {
        all_preconditions()
            .into_iter()
            .filter(|p| !self.verified.contains(p))
            .collect()
    }

    /// True only when every precondition has been verified.
    pub fn all_verified(&self) -> bool {
        self.verified.len() == all_preconditions().len()
    }
}

/// The complete set of preconditions, in declaration order.
fn all_preconditions() -> Vec<Precondition> {
    vec![
        Precondition::Build,
        Precondition::RefusesBindWithoutAuth,
        Precondition::ReplacementStarts,
    ]
}

/// The reconciler's deployment decision for one long-running service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeployAction {
    /// The running revision matches the expected tip: the only case where
    /// "nothing to do" is true.
    NothingToDo,
    /// The service is running a different revision and every restart-safety
    /// precondition is verified: redeploy now.
    Redeploy {
        /// The revision the redeploy must land on.
        expected: Revision,
    },
    /// The service is running a different revision, but some precondition
    /// is unverified: refuse, and name what would tell us.
    Refused {
        /// The unverified preconditions, in declaration order.
        missing: Vec<Precondition>,
    },
    /// The service states no revision: it cannot even be told whether a
    /// deploy is needed. This is never `NothingToDo`.
    Unknown,
}

impl fmt::Display for DeployAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NothingToDo => f.write_str("nothing to do"),
            Self::Redeploy { expected } => write!(f, "redeploy to {expected}"),
            Self::Refused { missing } => write!(
                f,
                "refused: unverified preconditions: {}",
                missing
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Unknown => f.write_str("revision unreported; cannot verify deployment"),
        }
    }
}

/// Decide what the reconciler does about one long-running service.
///
/// The decision table is the whole policy:
///
/// | running revision          | preconditions           | decision                    |
/// |---------------------------|-------------------------|-----------------------------|
/// | matches expected          | (any)                   | [`NothingToDo`]             |
/// | differs from expected     | all verified            | [`Redeploy`]                |
/// | differs from expected     | one or more unverified  | [`Refused`] (names them)    |
/// | not reported              | (any)                   | [`Unknown`]                 |
///
/// In particular, "nothing to do" is unreachable while the running
/// revision is stale or unreported: the reconciler must say which.
pub fn decide(
    running: Option<&Revision>,
    expected: &Revision,
    ledger: &PreconditionLedger,
) -> DeployAction {
    match drift(running, expected) {
        RevisionDrift::Current => DeployAction::NothingToDo,
        RevisionDrift::Unreported => DeployAction::Unknown,
        RevisionDrift::Drifted { .. } => {
            if ledger.all_verified() {
                DeployAction::Redeploy {
                    expected: expected.clone(),
                }
            } else {
                DeployAction::Refused {
                    missing: ledger.missing(),
                }
            }
        }
    }
}

/// The reconciler's line for one pass over one service.
///
/// It carries the running revision and the expected tip next to the
/// decision, so "nothing to do" can never again be said without naming the
/// revision that makes it true. Inconsistent inputs (an `action` that does
/// not match `running`/`expected` under [`decide`]) fall through to the
/// unreported line: the line never overstates what was actually verified.
pub fn reconcile_line(
    running: Option<&Revision>,
    expected: &Revision,
    action: &DeployAction,
) -> String {
    match (action, running) {
        (DeployAction::NothingToDo, Some(r)) => {
            format!("service: {r}; expected {expected}; in sync; nothing to do")
        }
        (DeployAction::Redeploy { .. }, Some(r)) => {
            format!("service: {r}; expected {expected}; DRIFTED — redeploy")
        }
        (DeployAction::Refused { missing }, Some(r)) => format!(
            "service: {r}; expected {expected}; DRIFTED — refused: unverified {}",
            missing
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        (DeployAction::Unknown, None) | (DeployAction::Unknown, Some(_)) => {
            format!("service: revision unreported; expected {expected}; cannot verify deployment")
        }
        _ => format!("service: revision unreported; expected {expected}; cannot verify deployment"),
    }
}
