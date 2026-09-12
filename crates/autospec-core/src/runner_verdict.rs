//! The runner's verdict is a gate, or it is not one (issue #4027).
//!
//! ## The defect this module exists to prevent
//!
//! The agent runner grades its own output: after the agent finishes it runs
//! the full gate set on the produced tree, diffs the failures against the
//! recorded baseline, and writes a verdict next to the patch:
//!
//! ```text
//! issue=3818
//! status=VERIFIED
//! test_passed=916
//! test_failed=4
//! new_failing_tests=0
//! fixed_baseline_failures=3
//! ```
//!
//! The differential design is right, and so is what is gated: unmodified
//! `main` is not green, so an absolute gate would reject a perfect patch,
//! and what is gated is whether *this patch* adds anything.
//!
//! Then, regardless of the verdict, the runner wrote the patch to the same
//! path the converter consumes:
//!
//! ```bash
//! git commit -qm "fix: address issue #$NUM"
//! git diff "$BASE_SHA"..HEAD > "$OUT/changes.patch"
//! ```
//!
//! Three of the patches held in one conversion pass — #3726, #3820, #3831 —
//! carried `status=NEW-TEST-FAILURES` with `new-failures.txt` naming the
//! exact test. The cluster knew each was broken, wrote it out anyway, and
//! the conversion pass then spent roughly ten minutes per patch re-running
//! the suite to reach the same conclusion. Nothing downstream read
//! `status.txt`: the verdict was computed at real cost, recorded correctly,
//! and consulted by no one.
//!
//! A pipeline stage that grades its output and passes it on unchanged has
//! done the work of a gate without being one. Worse, it *looks* like a gate
//! — the status file exists, the numbers are accurate — so the absence of
//! enforcement is invisible until bad output arrives downstream with a
//! correct failing grade attached. The cost compounds: the failing patch
//! consumes a conversion slot, it blocks its own issue from re-dispatch
//! because a patch on disk marks the issue as produced, and the downstream
//! stage duplicates the entire verification because it cannot distinguish
//! "this was never checked" from "this was checked and failed".
//!
//! Five invariants, each a primitive here:
//!
//! 1. **A failing verdict does not publish a conversion candidate**
//!    ([`is_failing`], [`route`], [`rejected_path`]). The patch is written
//!    to a sibling path that is clearly not a conversion candidate —
//!    nothing is discarded and nothing is picked up by mistake. [`is_failing`]
//!    matches exhaustively over the vocabulary, so a status added to
//!    [`crate::run_status::Status`] without a routing decision stops
//!    compiling (#4206).
//! 2. **The verdict travels with the artifact** ([`companion_files`],
//!    [`RunnerVerdict`], [`consume`]). Any consumer reading a patch also
//!    reads the status recorded for it, and a consumer that finds a failing
//!    status stops immediately with the runner's own reason rather than
//!    re-deriving it: the local gate is not invoked
//!    ([`ConsumeAction::gate_invoked`]).
//! 3. **The hold line names the runner's own reason** ([`held_line`]). The
//!    conversion pass records `held: runner already reported <status>, new
//!    failures: <names>` without running local gates, so the failure reason
//!    is the runner's own and no compute is duplicated.
//! 4. **A failed patch does not mark its issue produced**
//!    ([`eligibility`]). A patch known to be broken returns its issue to
//!    the eligible pool instead of blocking re-dispatch; a patch with no
//!    recorded verdict is fail-closed and still held for the conversion
//!    pass.
//! 5. **Verification runs against the tree the patch will land in**
//!    ([`landing_verification`]). A patch can be correct against the tree
//!    it was written on and violate a repository-wide invariant on the tree
//!    it lands in — #3818, #3819 and #3832 were genuinely `VERIFIED`
//!    against their own base and still failed on current `main`, because
//!    they added a bats suite without registering it and the registration
//!    invariant is a property of the whole tree, not of any one patch. That
//!    is not the runner's error, and it needs a different remedy: when the
//!    dispatch base has drifted from current `origin/main`, verification
//!    runs against `origin/main` — while the agent can still fix it — not
//!    only against the dispatch base.
//!
//! Everything here is pure: no I/O, no clock, no subprocess. The caller —
//! the runner, the dispatch guard, the conversion pass — performs the file
//! writes and gate invocations the decisions name, the way
//! [`crate::execution::status_triage`] and [`crate::dispatch_recheck`]
//! hand their decisions to the caller that performs the I/O.

use crate::run_status::{canonical_status, unknown_status_refusal, Status};

/// The artifact path the conversion pass consumes, relative to the issue's
/// output directory.
pub const CONVERSION_CANDIDATE: &str = "changes.patch";

/// The verdict file the runner writes next to the artifact.
pub const STATUS_FILE: &str = "status.txt";

/// The sidecar file the runner writes when tests fail, naming the exact
/// newly-failing tests.
pub const NEW_FAILURES_FILE: &str = "new-failures.txt";

/// Whether the recorded status is a failure: the gate ran and produced a
/// negative verdict *about the patch*.
///
/// The distinction is between "the patch is known to be broken" and
/// "nothing was measured":
///
/// - [`Status::NewTestFailures`], [`Status::TestTimeout`],
///   [`Status::FmtDirty`] and [`Status::BuildFail`] are failures — the gate
///   ran, the patch failed it, and the verdict is a property of the
///   submission. Re-dispatching or re-gating re-derives the same answer.
/// - [`Status::Timeout`], [`Status::TimeoutNoOutput`] and
///   [`Status::NoOutput`] are not: the run stopped before the gate, so no
///   verdict about the patch exists at all.
/// - [`Status::UnknownNoBaseline`] and [`Status::NoTestDb`] are gate-only
///   names: the gate could not measure (no baseline, no test database), so
///   they say nothing about the patch either.
/// - [`Status::Verified`] is the green verdict.
///
/// The match is exhaustive over the vocabulary: a status added to
/// [`Status`] without a routing decision here is a compile error, so
/// "any future failing status" cannot silently reach the conversion
/// candidate path (#4206).
pub const fn is_failing(status: Status) -> bool {
    matches!(
        status,
        Status::NewTestFailures | Status::TestTimeout | Status::FmtDirty | Status::BuildFail
    )
}

/// The sibling path a failing artifact is written to: in the issue's output
/// directory, clearly not the conversion candidate, and carrying the
/// recorded status so a reader sees the verdict without opening the status
/// file.
///
/// The name is exact, not a glob: the selector counts `changes.patch`
/// files, and `changes.rejected-NEW-TEST-FAILURES.patch` is not one.
pub fn rejected_path(status: Status) -> String {
    format!("changes.rejected-{}.patch", status.as_str())
}

/// Where the runner writes a produced artifact, given the verdict it
/// recorded for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishTarget {
    /// The artifact is a conversion candidate: it is written to
    /// [`CONVERSION_CANDIDATE`] and the conversion pass owns it.
    Candidate {
        /// `changes.patch`.
        path: String,
    },
    /// The recorded status is a failure: the artifact is written to the
    /// sibling [`rejected_path`]. Nothing is discarded — the patch, the
    /// status and the new-failures sidecar all survive for review — and
    /// nothing is picked up by mistake, because the path the converter
    /// consumes was never written.
    Rejected {
        /// The failing status the artifact is rejected on.
        status: String,
        /// The sibling path the artifact is written to.
        path: String,
    },
}

impl PublishTarget {
    /// The path the runner writes the artifact to.
    pub fn path(&self) -> &str {
        match self {
            Self::Candidate { path } | Self::Rejected { path, .. } => path,
        }
    }

    /// Whether the artifact lands on the path the converter consumes.
    pub fn is_conversion_candidate(&self) -> bool {
        matches!(self, Self::Candidate { .. })
    }
}

/// Decide where a produced artifact goes, given the recorded verdict.
///
/// The runner calls this *after* grading and *before* writing: a stage
/// that grades its output must route on the grade, or it is not grading at
/// all.
pub fn route(status: Status) -> PublishTarget {
    if is_failing(status) {
        PublishTarget::Rejected {
            status: status.as_str().to_string(),
            path: rejected_path(status),
        }
    } else {
        PublishTarget::Candidate {
            path: CONVERSION_CANDIDATE.to_string(),
        }
    }
}

/// The files a consumer must read alongside the artifact before acting on
/// it.
///
/// The verdict travels with the artifact: a consumer reading
/// `changes.patch` that does not also read [`STATUS_FILE`] cannot tell
/// "this was never checked" from "this was checked and failed", and will
/// re-derive a verdict the runner already paid for. When the recorded
/// status is a failure, the consumer also reads [`NEW_FAILURES_FILE`],
/// because the hold line carries the runner's own test names.
pub fn companion_files(status: Option<Status>) -> Vec<&'static str> {
    let mut files = vec![STATUS_FILE];
    if status.is_some_and(is_failing) {
        files.push(NEW_FAILURES_FILE);
    }
    files
}

/// The verdict a consumer reads next to the artifact: the recorded status,
/// resolved to its canonical spelling, and the newly-failing test names
/// the runner recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerVerdict {
    /// The recorded status, canonical after alias resolution; `None` when
    /// no status was recorded at all (the file was never written, or
    /// written with no status field).
    pub status: Option<Status>,
    /// The newly-failing tests the runner recorded
    /// ([`NEW_FAILURES_FILE`]), in the order the runner wrote them.
    pub new_failures: Vec<String>,
}

impl RunnerVerdict {
    /// Build a verdict from the raw recorded name.
    ///
    /// The name is resolved through the shared vocabulary: the legacy
    /// spelling `BUILD-FAILED` means what the vocabulary says it means, and
    /// a consumer cannot be fooled by a spelling. A name the vocabulary
    /// does not declare is refused rather than guessed: a consumer that
    /// cannot determine whether a status is failing must name what would
    /// tell it, and the file that would is
    /// [`crate::run_status::VOCABULARY_PATH`].
    pub fn from_recorded(raw: Option<&str>, new_failures: Vec<String>) -> Result<Self, String> {
        let status = match raw {
            None => None,
            Some(name) => {
                let canonical = canonical_status(name)
                    .ok_or_else(|| unknown_status_refusal("the conversion pass", name))?;
                Some(canonical)
            }
        };
        Ok(Self {
            status,
            new_failures,
        })
    }
}

/// What a consumer does with a patch and its recorded verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsumeAction {
    /// The recorded status is a failure: the consumer stops immediately
    /// with the runner's own reason. It records [`held_line`] and moves on
    /// — the local gate is not invoked, because re-running the suite
    /// re-derives what the runner already established at real cost.
    Hold {
        /// The failing status the runner recorded.
        status: Status,
        /// The newly-failing tests the runner recorded, named in the hold
        /// line.
        new_failures: Vec<String>,
    },
    /// The recorded status is not a failure — or no verdict was recorded:
    /// the consumer proceeds and its own gate runs when it needs evidence
    /// the runner did not provide (a `VERIFIED` run is confirmed against
    /// the current main; an unrecorded run is gated for the first time).
    Proceed {
        /// The recorded status, if any.
        status: Option<Status>,
    },
}

impl ConsumeAction {
    /// Whether the consumer's local gate runs under this action.
    ///
    /// A hold never runs the gate: the failure reason is the runner's own,
    /// and duplicating the verification is the compute the issue exists to
    /// save. Everything else proceeds to the gate.
    pub const fn gate_invoked(&self) -> bool {
        matches!(self, Self::Proceed { .. })
    }
}

/// Decide what the consumer does with a patch and its recorded verdict.
///
/// Pure: no I/O, no gate execution — the caller reads the companion files,
/// calls this, and executes the decision.
pub fn consume(verdict: &RunnerVerdict) -> ConsumeAction {
    match verdict.status {
        Some(status) if is_failing(status) => ConsumeAction::Hold {
            new_failures: verdict.new_failures.clone(),
            status,
        },
        status => ConsumeAction::Proceed { status },
    }
}

/// The hold line the conversion pass records for a patch held on the
/// runner's own verdict: the failure reason is the runner's, the new
/// failures are the runner's names, and no local gate ran to produce the
/// line.
///
/// `held: runner already reported NEW-TEST-FAILURES, new failures: a::b, c::d`
///
/// A failing status with no recorded test names (a `FMT-DIRTY` or
/// `BUILD-FAIL` run) records the status alone.
pub fn held_line(status: Status, new_failures: &[String]) -> String {
    if new_failures.is_empty() {
        format!("held: runner already reported {}", status.as_str())
    } else {
        format!(
            "held: runner already reported {}, new failures: {}",
            status.as_str(),
            new_failures.join(", ")
        )
    }
}

/// Whether the issue's produced patch marks it produced — i.e. blocks
/// re-dispatch — given the verdict recorded next to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Eligibility {
    /// The patch carries no failing verdict: the conversion pass owns it
    /// and the issue is not re-dispatched.
    Produced {
        /// The recorded status.
        status: Status,
    },
    /// The recorded status is a failure: the patch is not a conversion
    /// candidate and must not block re-dispatch. The issue returns to the
    /// eligible pool; a patch known to be broken is not work in flight.
    Redispatchable {
        /// The failing status the patch is rejected on.
        status: Status,
    },
    /// No verdict was recorded next to the patch: fail-closed. The issue
    /// is held for the conversion pass, never re-dispatched and never
    /// archived on a guess — the same posture
    /// [`crate::dispatch_guard::classify_artifact_outcome`] takes for an
    /// unrecorded artifact.
    Unrecorded,
}

impl Eligibility {
    /// Whether the issue may be re-dispatched.
    pub const fn redispatchable(self) -> bool {
        matches!(self, Self::Redispatchable { .. })
    }
}

/// Classify the issue's eligibility given the verdict recorded next to its
/// patch, if any.
pub fn eligibility(status: Option<Status>) -> Eligibility {
    match status {
        None => Eligibility::Unrecorded,
        Some(status) if is_failing(status) => Eligibility::Redispatchable { status },
        Some(status) => Eligibility::Produced { status },
    }
}

/// The check that refuses the incident: a runner that graded a patch as
/// failing and still published it to the path the converter consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishAudit {
    /// The publish matches the verdict: the candidate path was written
    /// only for a non-failing status, or the failing artifact went to its
    /// sibling path.
    Clean,
    /// A failing verdict was published to the conversion-candidate path:
    /// the stage graded its output and passed it on unchanged — the work of
    /// a gate without being one.
    EmittedDespiteFailure {
        /// The failing status the runner recorded.
        status: Status,
        /// The path the artifact was written to (`changes.patch`).
        path: String,
    },
}

impl PublishAudit {
    /// The finding line, for a log or a gate message.
    pub fn line(&self) -> String {
        match self {
            Self::Clean => "publish matches the recorded verdict".to_string(),
            Self::EmittedDespiteFailure { status, path } => format!(
                "EMITTED-DESPITE-FAILURE: status={} was published to {path}; the artifact belongs at {}",
                status.as_str(),
                rejected_path(*status)
            ),
        }
    }
}

/// Audit one publish: was the artifact written where the verdict says it
/// belongs?
///
/// `None` (no status recorded) is clean here: the consumer fail-closes by
/// gating the patch locally, and the runner's routing decision applies
/// only to a verdict it actually recorded.
pub fn publish_audit(recorded: Option<Status>, emitted_path: &str) -> PublishAudit {
    match recorded {
        Some(status) if is_failing(status) && emitted_path == CONVERSION_CANDIDATE => {
            PublishAudit::EmittedDespiteFailure {
                status,
                path: emitted_path.to_string(),
            }
        }
        _ => PublishAudit::Clean,
    }
}

/// Whether a verdict earned against the dispatch base still covers the
/// tree the patch will land in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LandingVerification {
    /// The dispatch base is current `origin/main`: verifying against the
    /// dispatch base is verifying against the landing tree.
    BaseIsCurrent {
        /// The base commit both read as.
        base: String,
    },
    /// The dispatch base is not current `origin/main`: the verdict was
    /// earned on a tree the patch will not land in. A patch can be correct
    /// against the tree it was written on and violate a repository-wide
    /// invariant on the tree it lands in — the invariant is a property of
    /// the whole tree, not of any one patch. Verification runs against
    /// current `origin/main`, and it runs while the agent can still fix
    /// it, not at the converter.
    ReverifyAgainstLandingTree {
        /// The commit the patch was dispatched from and graded against.
        dispatch_base: String,
        /// The commit current `origin/main` points at.
        origin_main: String,
    },
    /// The landing tree could not be read: fail-closed. Verification
    /// refuses and names what would tell it — the dispatch base and
    /// current `origin/main` — never guessing that a drift exists or does
    /// not.
    Unverifiable {
        /// What is missing, naming the unreadable side.
        detail: String,
    },
}

impl LandingVerification {
    /// Whether verification must run against the landing tree rather than
    /// trusting a verdict earned against the dispatch base.
    pub const fn reverify_needed(&self) -> bool {
        matches!(self, Self::ReverifyAgainstLandingTree { .. })
    }
}

/// Decide where verification must run, given the dispatch base the patch
/// was graded against and the commit current `origin/main` points at.
///
/// An unreadable or empty base is treated as not read: an empty commit
/// record is an error, never "the same commit".
pub fn landing_verification(
    dispatch_base: Option<&str>,
    origin_main: Option<&str>,
) -> LandingVerification {
    let base = dispatch_base.map(str::trim).filter(|s| !s.is_empty());
    let main = origin_main.map(str::trim).filter(|s| !s.is_empty());
    match (base, main) {
        (Some(base), Some(main)) if base == main => LandingVerification::BaseIsCurrent {
            base: base.to_string(),
        },
        (Some(base), Some(main)) => LandingVerification::ReverifyAgainstLandingTree {
            dispatch_base: base.to_string(),
            origin_main: main.to_string(),
        },
        (None, Some(_)) => LandingVerification::Unverifiable {
            detail: "the dispatch base was not recorded; record the commit the patch was dispatched from and re-check against origin/main".to_string(),
        },
        (Some(_), None) => LandingVerification::Unverifiable {
            detail: "current origin/main could not be read; fetch origin and re-check before trusting a verdict earned against the dispatch base".to_string(),
        },
        (None, None) => LandingVerification::Unverifiable {
            detail: "neither the dispatch base nor current origin/main could be read; record the dispatch base and fetch origin before trusting a verdict earned against the dispatch base".to_string(),
        },
    }
}
