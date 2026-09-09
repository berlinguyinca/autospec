//! Evidence fidelity (#3862): a simulation is never read as evidence for a
//! criterion that requires a real dependency.
//!
//! The incident this module exists for (InferWeave/inferweave#50, #3862): a
//! worker without a container runtime had to satisfy a "fresh VM/container
//! E2E" criterion. It built a `docker` shim, prepended it to `PATH`, ran the
//! test against a fake health endpoint, and the gate read
//! `test_passed=586 test_failed=0` and closed the issue as COMPLETED — the
//! commit even said `Refs #50 (does not close it)`. A shim always behaves,
//! so the test was structurally incapable of failing for the reason the
//! criterion exists to catch: an image that will not start, a port already
//! bound, a pinned digest that will not pull.
//!
//! The invariant: when an acceptance criterion names a real dependency the
//! executor cannot provide, the honest outcome is "cannot verify here" —
//! never a simulation that passes. Substituting a fake turns
//! "this environment cannot test this" into "this passed", and every
//! downstream consumer reads the second.
//!
//! Four invariants, one primitive each. Everything here is pure and
//! testable: no I/O, no clock, no subprocess.
//!
//! 1. **A task routes only to an executor that has what its tests need**
//!    ([`route_task`], [`RoutingVerdict`]). A container-E2E task never
//!    reaches a worker without a container runtime; with no capable
//!    executor the outcome is
//!    [`RoutingVerdict::CapabilityUnavailable`] — code
//!    `CAPABILITY-UNAVAILABLE` — which reports the gap and leaves the task
//!    queued. That is a queueing state, not a failure and not a pass: the
//!    task was never run and no criterion was judged.
//! 2. **Substitution is surfaced, not accepted**
//!    ([`detect_substitution`], [`shimmed_binaries`]). A fixture that
//!    creates an executable named for a real dependency (`docker`,
//!    `podman`, `psql`) and prepends it to `PATH` while the criterion says
//!    *container*, *live* or *real* is flagged for review. The pattern is
//!    specific and greppable; a test that merely *invokes* `docker` is
//!    driving the real thing and is not flagged.
//! 3. **A change closes an issue only if it says so**
//!    ([`closure_authorized`], [`ClosureVerdict`]). A closing verb
//!    (`Closes`/`Fixes`/`Resolves`, in any case) referencing the issue
//!    authorizes closure; `Refs`, `references` and plain prose never do. A
//!    patch carrying no closing keyword for the issue cannot close it — a
//!    green gate is not a claim of acceptance.
//! 4. **The verdict names the evidence kind, not just the count**
//!    ([`EvidenceKind`], [`classify_evidence`], [`TestVerdict`]).
//!    `586 passed (hermetic)` and `586 passed (live)` are different claims.
//!    A run whose fixture shims a dependency is hermetic no matter what
//!    else it touched, and only live evidence bears on a criterion that
//!    names a real dependency
//!    ([`EvidenceKind::bears_on_live_criterion`]).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

// ── 1. Capability routing ──────────────────────────────────────────────

/// A capability an executor either has or does not have.
///
/// The set is open by construction (a new capability is a new variant), but
/// every task's requirement must name variants from this set: a task that
/// cannot express what its tests need is not routable, and an unroutable
/// task is reported, never guessed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// A container runtime (docker, podman, apptainer) is present and
    /// functional on the executor's host.
    ContainerRuntime,
}

impl Capability {
    /// Every capability the vocabulary currently names.
    pub const ALL: [Self; 1] = [Self::ContainerRuntime];

    /// The machine name the routing ledger reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ContainerRuntime => "container_runtime",
        }
    }

    /// Parse a machine name back to a capability; `None` when unknown, so a
    /// typo in a manifest degrades to "cannot route", not "cannot tell".
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|capability| capability.as_str() == name)
    }
}

/// One executor's self-reported capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorProfile {
    /// The executor's stable identity, used for deterministic tie-breaks.
    pub executor_id: String,
    /// The capabilities this executor provides.
    pub provides: BTreeSet<Capability>,
}

impl ExecutorProfile {
    pub fn new(
        executor_id: impl Into<String>,
        provides: impl IntoIterator<Item = Capability>,
    ) -> Self {
        Self {
            executor_id: executor_id.into(),
            provides: provides.into_iter().collect(),
        }
    }
}

/// The routing outcome for one task against one executor pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingVerdict {
    /// Dispatch to this executor: it provides every capability the task's
    /// tests require.
    Dispatched { executor_id: String },
    /// No single executor in the pool provides the task's full requirement.
    /// The task stays queued and the pool is told which capabilities it
    /// lacks, so a capable executor can be provisioned. This is a queueing
    /// state, not a failure: the task was never run and no criterion was
    /// judged, so nothing here may be reported as a pass.
    CapabilityUnavailable { missing: Vec<Capability> },
}

impl RoutingVerdict {
    /// Whether this verdict authorizes a dispatch at all.
    pub fn is_dispatch(&self) -> bool {
        matches!(self, Self::Dispatched { .. })
    }

    /// The machine code a dispatcher reports for this verdict:
    /// `DISPATCHED` or `CAPABILITY-UNAVAILABLE`.
    pub fn as_code(&self) -> &'static str {
        match self {
            Self::Dispatched { .. } => "DISPATCHED",
            Self::CapabilityUnavailable { .. } => "CAPABILITY-UNAVAILABLE",
        }
    }
}

/// Route a task to an executor, or report the capability gap.
///
/// The requirement is conjunctive: an executor is chosen only if it
/// provides *every* required capability. Ties break on executor id, so the
/// verdict is deterministic. A task whose requirement no executor meets is
/// [`RoutingVerdict::CapabilityUnavailable`] — never routed to the "closest"
/// executor, which is the #3862 defect in routing form.
pub fn route_task(required: &[Capability], pool: &[ExecutorProfile]) -> RoutingVerdict {
    let required: BTreeSet<Capability> = required.iter().copied().collect();
    let mut capable: Vec<&ExecutorProfile> = pool
        .iter()
        .filter(|executor| {
            required
                .iter()
                .all(|capability| executor.provides.contains(capability))
        })
        .collect();
    if !capable.is_empty() {
        capable.sort_by(|a, b| a.executor_id.cmp(&b.executor_id));
        return RoutingVerdict::Dispatched {
            executor_id: capable[0].executor_id.clone(),
        };
    }
    // Capabilities no executor in the pool has at all. When every
    // capability exists somewhere but is split across executors, the whole
    // requirement is named instead, so the gap stays visible either way.
    let absent: Vec<Capability> = required
        .iter()
        .copied()
        .filter(|capability| {
            !pool
                .iter()
                .any(|executor| executor.provides.contains(capability))
        })
        .collect();
    let missing = if absent.is_empty() {
        required.into_iter().collect()
    } else {
        absent
    };
    RoutingVerdict::CapabilityUnavailable { missing }
}

// ── 2. Substitution detection ──────────────────────────────────────────

/// Binaries a fixture is known to fake, in the shape that matters: a file
/// created under the fixture's control and shadowed on `PATH`.
const KNOWN_REAL_BINARIES: [&str; 3] = ["docker", "podman", "psql"];

/// Criterion terms that name a real dependency the executor must actually
/// provide. Anything else a criterion names (a function, a file, a parser)
/// can be satisfied hermetically by definition.
const REAL_DEPENDENCY_TERMS: [&str; 3] = ["container", "live", "real"];

/// The outcome of scanning one test against its own acceptance criterion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubstitutionVerdict {
    /// No substitution found: nothing to surface.
    Clean,
    /// The test shims a binary while its criterion names a real dependency.
    /// Flag for review; do not accept the run as evidence of the criterion
    /// — a shimmed run is [`EvidenceKind::Hermetic`] at best.
    Flagged {
        /// The binaries the fixture creates and puts on `PATH`.
        binaries: Vec<&'static str>,
        /// The criterion term that names the real dependency.
        criterion_term: &'static str,
    },
}

impl SubstitutionVerdict {
    /// Whether this verdict must be surfaced for review.
    pub fn is_flagged(&self) -> bool {
        matches!(self, Self::Flagged { .. })
    }
}

/// Scan one test against the criterion it claims to satisfy.
///
/// A finding requires *both* halves of the #3862 pattern: the fixture
/// creates and `PATH`-prepends an executable named for a real dependency,
/// *and* the criterion names a real dependency. A criterion that says
/// nothing real cannot be violated by a shim, and a test that shims
/// nothing has nothing to surface.
pub fn detect_substitution(criterion: &str, test_source: &str) -> SubstitutionVerdict {
    match (
        criterion_demands_real(criterion),
        shimmed_binaries(test_source),
    ) {
        (Some(term), binaries) if !binaries.is_empty() => SubstitutionVerdict::Flagged {
            binaries,
            criterion_term: term,
        },
        _ => SubstitutionVerdict::Clean,
    }
}

/// Whether the criterion names a real dependency: *container*, *live* or
/// *real* (case-insensitive). Returns the matched term for reporting.
pub fn criterion_demands_real(criterion: &str) -> Option<&'static str> {
    let lowered = criterion.to_ascii_lowercase();
    REAL_DEPENDENCY_TERMS
        .iter()
        .copied()
        .find(|term| lowered.contains(term))
}

/// The binaries the test source fakes: it creates an executable named for
/// the binary *and* prepends the fixture's directory to `PATH`. Both halves
/// are required — a test that merely invokes `docker` is driving the real
/// thing, and a file named `docker` that never reaches `PATH` shadows
/// nothing.
pub fn shimmed_binaries(test_source: &str) -> Vec<&'static str> {
    if !path_is_prepended(test_source) {
        return Vec::new();
    }
    KNOWN_REAL_BINARIES
        .iter()
        .copied()
        .filter(|binary| creates_executable(test_source, binary))
        .collect()
}

/// The `PATH`-prepending half of the shim pattern, in the shapes fixtures
/// actually use: `Command::env("PATH", …)`, `std::env::set_var("PATH", …)`,
/// `std::env::prepend_path`, and shell `PATH=…` / `$PATH`.
fn path_is_prepended(source: &str) -> bool {
    [
        "env(\"PATH\"",
        "set_var(\"PATH\"",
        "prepend_path",
        "$PATH",
        "PATH=",
    ]
    .iter()
    .any(|marker| source.contains(marker))
}

/// The executable-creation half: a quoted path whose final segment is the
/// binary name (`"$shim_dir/docker"`, `format!("{dir}/docker")`), or a
/// `Path::join` of it (`dir.join("docker")`). Invoking the real binary is a
/// bare quoted name with no directory — deliberately not matched.
fn creates_executable(source: &str, binary: &str) -> bool {
    if source.contains(&format!("join(\"{binary}\")")) {
        return true;
    }
    quoted_spans(source).iter().any(|span| {
        (span.contains('/') || span.contains('\\'))
            && span.rsplit(['/', '\\']).next() == Some(binary)
    })
}

/// The double-quoted spans in a source. Heuristic by design: this feeds a
/// flag-for-review, not a verdict, so a mis-paired escape costs a human a
/// look, never a wrong closure.
fn quoted_spans(source: &str) -> Vec<&str> {
    let mut spans = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            if let Some(end) = source[i + 1..].find('"') {
                spans.push(&source[i + 1..i + 1 + end]);
                i += end + 2;
                continue;
            }
        }
        i += 1;
    }
    spans
}

// ── 3. Closing-keyword authority ───────────────────────────────────────

/// The verbs that claim completion of an issue. Deliberately absent:
/// `refs`, `references`, `see`, `related` — they acknowledge an issue, not
/// claim it. A green gate is not among them either: acceptance is claimed
/// in words, never inferred from a count.
const CLOSING_VERBS: [&str; 9] = [
    "close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved",
];

/// Whether a change is authorized to close an issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosureVerdict {
    /// The change asserts a closing verb referencing this issue: closure
    /// is authorized.
    Authorized {
        /// The verb that claimed it, for the audit trail.
        verb: &'static str,
    },
    /// No closing verb references this issue. The change must not close
    /// it — auto-closing on a green gate is the #3862 defect.
    NotAuthorized,
}

impl ClosureVerdict {
    /// Whether this verdict authorizes closure.
    pub fn is_authorized(&self) -> bool {
        matches!(self, Self::Authorized { .. })
    }
}

/// Whether `text` (the PR body plus commit messages) authorizes closing
/// `issue_number`.
///
/// A match is a closing verb, in any case, with the issue reference
/// immediately after it (whitespace between allowed): `Closes #50`,
/// `fixes #50`, `RESOLVED #50`. `Refs #50 (does not close it)` — the exact
/// commit message of the #3862 incident — matches nothing.
pub fn closure_authorized(text: &str, issue_number: u64) -> ClosureVerdict {
    let lowered = text.to_ascii_lowercase();
    for verb in CLOSING_VERBS {
        if contains_closing_reference(&lowered, verb, issue_number) {
            return ClosureVerdict::Authorized { verb };
        }
    }
    ClosureVerdict::NotAuthorized
}

fn contains_closing_reference(text: &str, verb: &str, issue_number: u64) -> bool {
    let reference = format!("#{issue_number}");
    let mut start = 0;
    while let Some(found) = text[start..].find(verb) {
        let index = start + found;
        let whole_word = text[..index]
            .chars()
            .next_back()
            .is_none_or(|character| !character.is_ascii_alphanumeric());
        let after = index + verb.len();
        start = after;
        if !whole_word {
            continue;
        }
        let suffix = text[after..].trim_start();
        let Some(after_reference) = suffix.strip_prefix(&reference) else {
            continue;
        };
        if after_reference
            .chars()
            .next()
            .is_none_or(|character| !character.is_ascii_digit())
        {
            return true;
        }
    }
    false
}

// ── 4. Evidence kind in the verdict ────────────────────────────────────

/// What a test run actually was. A bare count — `586 passed` — is
/// compatible with both a real container run and a simulation of one; the
/// kind is what tells a reader whether the number bears on the criterion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    /// The run touched simulations: a sandboxed `HOME`, a shimmed binary,
    /// a fake endpoint. Fast and hermetic, and structurally incapable of
    /// failing for the reasons a live-dependency criterion exists to catch.
    Hermetic,
    /// Real services ran — real processes, real ports — but stayed inside
    /// the test host's own control; some of what the criterion names was
    /// still simulated.
    Integration,
    /// The real dependency ran: a container actually started and a real
    /// endpoint answered. The count bears on a criterion that names that
    /// dependency.
    Live,
}

impl EvidenceKind {
    /// Every kind a verdict can name.
    pub const ALL: [Self; 3] = [Self::Hermetic, Self::Integration, Self::Live];

    /// The word the verdict prints.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hermetic => "hermetic",
            Self::Integration => "integration",
            Self::Live => "live",
        }
    }

    /// Parse a verdict word back to a kind; `None` when unknown.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.as_str() == name)
    }

    /// Whether evidence of this kind can satisfy a criterion that names a
    /// real dependency (*container*, *live*, *real*). Only live evidence
    /// can: 586 hermetic passes is compatible with the dependency never
    /// having run.
    pub fn bears_on_live_criterion(self) -> bool {
        matches!(self, Self::Live)
    }
}

/// What a test run actually touched, as observed by the runner. The
/// classifier only orders these; it does not invent them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EvidenceFacts {
    /// The fixture created and `PATH`-prepended an executable named for a
    /// dependency the test then invoked (see [`shimmed_binaries`]).
    pub shimmed_dependency: bool,
    /// A real container runtime was invoked with no shim in `PATH`: a
    /// container actually started.
    pub real_container_run: bool,
    /// A real endpoint — not a fixture's — answered the health check.
    pub real_endpoint: bool,
}

/// Order the facts into a kind.
///
/// A shimmed run is hermetic no matter what else it touched: the shim
/// always behaves, so the run could not have failed for the reason the
/// criterion cares about, and calling it live would fabricate evidence.
/// Without a shim, both real halves present is live; exactly one is
/// integration; neither is hermetic.
pub fn classify_evidence(facts: EvidenceFacts) -> EvidenceKind {
    if facts.shimmed_dependency {
        return EvidenceKind::Hermetic;
    }
    match (facts.real_container_run, facts.real_endpoint) {
        (true, true) => EvidenceKind::Live,
        (true, false) | (false, true) => EvidenceKind::Integration,
        (false, false) => EvidenceKind::Hermetic,
    }
}

/// A gate's verdict about a test run: the count *and* the kind of evidence
/// behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestVerdict {
    pub passed: u32,
    pub failed: u32,
    pub kind: EvidenceKind,
}

impl TestVerdict {
    /// Build a verdict from the observed facts of the run.
    pub fn new(passed: u32, failed: u32, facts: EvidenceFacts) -> Self {
        Self {
            passed,
            failed,
            kind: classify_evidence(facts),
        }
    }

    /// The verdict as a reader sees it: the count and the evidence kind,
    /// never the count alone.
    pub fn render(&self) -> String {
        format!(
            "{} passed, {} failed (evidence: {})",
            self.passed,
            self.failed,
            self.kind.as_str()
        )
    }
}
