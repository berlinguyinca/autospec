//! Reader fidelity: read the value the system reads, never a path or field
//! you inferred (issue #3682).
//!
//! Three wrong conclusions in one session, the same cause each time: the
//! reader needed a fact the system already stores and reconstructed it
//! instead of reading it from where the system reads it.
//!
//! 1. *"The pipeline is blocked — zero endpoints."* The reader listed
//!    `$L/endpoints/`. The endpoints live in `$L/state/endpoints/`. There were
//!    ten, all healthy. A total outage was reported that did not exist, and a
//!    fault was hunted in two healthy scripts.
//! 2. *"Four workers are unreachable."* The reader took each worker's port by
//!    grepping its *log* for a URL. The endpoint files record the real port.
//!    Registrations went to the wrong port, all failed, and the workers were
//!    declared dead. The addresses from the endpoint files returned `200` for
//!    all ten.
//! 3. *"iw-30 is hung."* `agent.out` was 0 bytes after 2h21m, so it nearly
//!    got cancelled. The workers showed nine slots generating with a 61k-token
//!    context in flight. It was working.
//!
//! In all three the authoritative value was one file read away, and in all
//! three the reconstructed value was **plausible** — which is what makes it
//! expensive. A wrong path returns an empty list, not an error. A wrong port
//! returns a connection failure, not a warning. Absent output looks exactly
//! like a stall. Every one of them presents as a *finding about the system*
//! rather than a mistake by the reader.
//!
//! Reconstructing is locally cheaper — a guess is one command, finding who
//! writes the directory is three or four — so under pressure the guess wins,
//! and the guess is silent when wrong. A fabricated fact does not fail; it
//! propagates: false "zero endpoints" becomes false "pipeline blocked"
//! becomes a search for a fault in two healthy scripts.
//!
//! Four invariants, one primitive each. Everything here is pure: no I/O, no
//! clock, no subprocesses. The caller reads the files and reports them; this
//! code never opens a connection.
//!
//! 1. **A value's source is the producer's own path, never a guess**
//!    ([`Inference`], [`Source`]). A value has a producer — the script that
//!    writes the file, the flag that sets the port, the API that serves the
//!    list — and only that producer's own path is authoritative. A path
//!    inferred from a sibling directory, a log line, or a naming convention is
//!    a candidate, not a fact.
//! 2. **Confirm the path against its writer before trusting it**
//!    ([`Confirmation`]). The cheapest confirmation is usually
//!    `grep -rl <thing>` over the scripts to find who writes the value, before
//!    reading it — the one command the #3682 reader skipped three times. A
//!    read is confirmed only when the path it used is the path the writer
//!    itself uses; naming the path is not confirmation.
//! 3. **An empty read from an unverified source is unverified, not a
//!    finding** ([`Read`], [`verdict`], [`ReadVerdict`]). An empty result and
//!    a wrong path are indistinguishable at the call site, and an empty
//!    result is exactly the shape a dramatic conclusion takes.
//! 4. **A dramatic conclusion requires a verified read** ([`Finding`],
//!    [`finding_verdict`]). "Zero endpoints", "workers unreachable", "agent
//!    hung" are findings only when they rest on a verified read; otherwise
//!    they are candidates to check, and the report says so.

use serde::{Deserialize, Serialize};

// ── 1. A value's source is the producer's own path, never a guess ─────────

/// How a candidate source for a value was derived (invariant 1).
///
/// The set is open by construction (a new way to guess is a new variant), but
/// the four named here are the ones the #3682 session proves easy to reach
/// for: three are guesses, and only one is the producer's own declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Inference {
    /// The producer's own declaration: the script that writes the file, the
    /// flag that sets the port, the API that serves the list. The only source
    /// the system itself reads, and the only one a fact may rest on.
    ProducerDeclared { producer: String },
    /// Guessed from the layout of a sibling directory (`endpoints/` instead
    /// of `state/endpoints/`).
    SiblingDirectory { sibling: String },
    /// Taken from a line the component printed in its log (a port read out of
    /// a URL in the log).
    LogLine { log: String },
    /// Guessed from a naming convention.
    NamingConvention,
}

impl Inference {
    /// Whether this is the inference the system itself trusts: the producer's
    /// own declaration. Every other variant is a guess, and a guess is silent
    /// when it is wrong.
    pub fn is_authoritative(&self) -> bool {
        matches!(self, Inference::ProducerDeclared { .. })
    }

    /// A short label for the report line, naming what the source was derived
    /// from.
    pub fn label(&self) -> String {
        match self {
            Inference::ProducerDeclared { producer } => {
                format!("producer's own path ({producer})")
            }
            Inference::SiblingDirectory { sibling } => format!("sibling directory {sibling}"),
            Inference::LogLine { log } => format!("a log line in {log}"),
            Inference::NamingConvention => "a naming convention".to_string(),
        }
    }
}

/// A candidate place to read one value from, with how it was derived
/// (invariant 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub path: String,
    pub inference: Inference,
}

impl Source {
    pub fn new(path: impl Into<String>, inference: Inference) -> Self {
        Self {
            path: path.into(),
            inference,
        }
    }

    /// Whether this is the producer's own path — the only source a fact about
    /// the system may rest on. A sibling directory, a log line, or a naming
    /// convention is a candidate, not a fact.
    pub fn is_authoritative(&self) -> bool {
        self.inference.is_authoritative()
    }
}

// ── 2. Confirm the path against its writer ───────────────────────────────

/// The confirmation step (invariant 2): the path the *writer* uses for the
/// value, discovered by reading the writer itself — the `grep -rl <thing>`
/// over the scripts the #3682 reader skipped three times.
///
/// A read is confirmed only when the path it actually used is the path the
/// writer itself uses. Naming the path is not confirmation: a named path and a
/// wrong path are the same until the writer says otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Confirmation {
    /// The path the producer writes, as the writer itself uses it.
    pub writer_path: String,
}

impl Confirmation {
    pub fn new(writer_path: impl Into<String>) -> Self {
        Self {
            writer_path: writer_path.into(),
        }
    }

    /// Whether the confirmed writer path is the same path the read used. A
    /// mismatch is the #3682 endpoints case: the writer writes
    /// `state/endpoints/`, the read used `endpoints/`.
    pub fn matches(&self, read_path: &str) -> bool {
        self.writer_path == read_path
    }
}

// ── 3. An empty read from an unverified source is unverified ─────────────

/// A read of one value: where it came from, whether it was confirmed against
/// the writer, and how much came back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Read {
    pub source: Source,
    /// The confirmation, if the reader did it (invariant 2).
    pub confirmation: Option<Confirmation>,
    /// How much the read returned: a list length, a byte count, a slot count.
    /// `0` is an empty result — the dangerous shape (invariant 3).
    pub count: u64,
}

impl Read {
    pub fn new(source: Source, confirmation: Option<Confirmation>, count: u64) -> Self {
        Self {
            source,
            confirmation,
            count,
        }
    }

    /// Whether the read returned nothing: an empty list, a 0-byte file, no
    /// matches.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Whether this read is verified: the source is the producer's own path
    /// *and* it was confirmed against the writer with a matching path. The two
    /// halves are independent — a producer path you never confirmed is still a
    /// named guess, and a guess you confirmed against the writer is still
    /// derived by guessing and must be re-derived as producer-declared.
    pub fn is_verified(&self) -> bool {
        self.source.is_authoritative()
            && self
                .confirmation
                .as_ref()
                .is_some_and(|c| c.matches(&self.source.path))
    }
}

/// Why a read is unverified: which half of verification failed. Both flags
/// are `false` only for a verified read, so [`verdict`] never emits this with
/// both unset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnverifiedReason {
    /// The source is a guess — sibling directory, log line, or naming
    /// convention — not the producer's own path.
    pub inferred: bool,
    /// The source is the producer's path but the writer's path was never
    /// confirmed to match it (or a confirmation was recorded that does not
    /// match).
    pub unconfirmed: bool,
}

/// What a read is, given where it came from (invariant 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadVerdict {
    /// The read came from a verified source: its result stands. An empty
    /// result here is a genuine zero (a real finding); a value here is the
    /// value the system holds.
    Verified,
    /// The read did not come from a verified source: its result — empty or
    /// not — is a candidate to check against the producer's own path, never a
    /// finding. An empty result here is the dangerous shape, because at the
    /// call site it is indistinguishable from a wrong path and takes the form
    /// of a dramatic conclusion.
    Unverified { reason: UnverifiedReason },
}

/// Decide what a read is (invariant 3).
///
/// A read is [`ReadVerdict::Verified`] only when it came from the producer's
/// own path *and* that path was confirmed against the writer. Anything less is
/// [`ReadVerdict::Unverified`], naming the half that failed — and an
/// unverified empty result is *never* a finding, because at the call site an
/// empty result and a wrong path are the same.
pub fn verdict(read: &Read) -> ReadVerdict {
    if read.is_verified() {
        return ReadVerdict::Verified;
    }
    let confirmed = read
        .confirmation
        .as_ref()
        .is_some_and(|c| c.matches(&read.source.path));
    ReadVerdict::Unverified {
        reason: UnverifiedReason {
            inferred: !read.source.is_authoritative(),
            unconfirmed: !confirmed,
        },
    }
}

// ── 4. A dramatic conclusion requires a verified read ─────────────────────

/// A finding about the system — a dramatic conclusion ("zero endpoints",
/// "workers unreachable", "agent hung") — and the read it rests on
/// (invariant 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub claim: String,
    pub read: Read,
}

impl Finding {
    pub fn new(claim: impl Into<String>, read: Read) -> Self {
        Self {
            claim: claim.into(),
            read,
        }
    }
}

/// Whether a finding is supported by its read (invariant 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingVerdict {
    /// The finding rests on a verified read: it may be reported.
    Sound,
    /// The finding rests on an unverified read: the claim is a candidate, not
    /// a fact. `reason` names which half of verification the read failed.
    Unsupported { reason: UnverifiedReason },
}

/// Judge a finding against the read it rests on (invariant 4).
///
/// A finding is [`FindingVerdict::Sound`] only when its read is
/// [`ReadVerdict::Verified`]. A dramatic conclusion resting on an inferred or
/// unconfirmed read is the #3682 session: plausible, silent when wrong, and
/// it propagates.
pub fn finding_verdict(finding: &Finding) -> FindingVerdict {
    match verdict(&finding.read) {
        ReadVerdict::Verified => FindingVerdict::Sound,
        ReadVerdict::Unverified { reason } => FindingVerdict::Unsupported { reason },
    }
}

// ── reporting ────────────────────────────────────────────────────────────

/// The report line for a verified read: it states its count, and an empty
/// count is called out as a genuine zero (a real finding) — the one case an
/// unverified zero is confused with.
fn verified_line(read: &Read) -> String {
    if read.is_empty() {
        format!(
            "VERIFIED: {} holds 0 — a genuine zero, confirmed against its writer",
            read.source.path
        )
    } else {
        format!(
            "VERIFIED: {} holds {} (confirmed against its writer)",
            read.source.path, read.count
        )
    }
}

/// The report line for an unverified read: it names the failed half and the
/// remedy — and never a dramatic conclusion — because at the call site an
/// unverified empty result and a wrong path are indistinguishable.
fn unverified_line(read: &Read, reason: &UnverifiedReason) -> String {
    let mut causes: Vec<String> = Vec::new();
    if reason.inferred {
        causes.push(read.source.inference.label());
    }
    if reason.unconfirmed {
        causes.push("not confirmed against its writer".to_string());
    }
    let empty_note = if read.is_empty() {
        " — an empty result here is indistinguishable from a wrong path, not a finding"
    } else {
        ""
    };
    format!(
        "UNVERIFIED: {} ({}) — confirm with grep -rl before reporting{}",
        read.source.path,
        causes.join(" and "),
        empty_note
    )
}

impl ReadVerdict {
    /// The line a verification report prints for this read: a verified read
    /// states its count; an unverified read names the failed half and the
    /// remedy, and never a dramatic conclusion.
    pub fn line(&self, read: &Read) -> String {
        match self {
            ReadVerdict::Verified => verified_line(read),
            ReadVerdict::Unverified { reason } => unverified_line(read, reason),
        }
    }
}

/// The report line for a finding: a sound finding may be reported; an
/// unsupported one is rejected, naming the unverified read it rests on.
fn finding_line(finding_verdict: &FindingVerdict, finding: &Finding) -> String {
    match finding_verdict {
        FindingVerdict::Sound => {
            format!(
                "OK: \"{}\" rests on a verified read — may be reported",
                finding.claim
            )
        }
        FindingVerdict::Unsupported { .. } => format!(
            "REJECT: \"{}\" rests on an unverified read — {} (a dramatic conclusion from an \
             unverified read propagates)",
            finding.claim,
            verdict(&finding.read).line(&finding.read)
        ),
    }
}

impl FindingVerdict {
    /// The line a review checklist prints for a finding: a sound finding may
    /// be reported; an unsupported one is rejected, naming the read that
    /// should have been verified first.
    pub fn line(&self, finding: &Finding) -> String {
        finding_line(self, finding)
    }
}
