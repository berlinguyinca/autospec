//! Reader fidelity: read the value the system reads, never a path or field
//! you inferred (issue #3682).
//!
//! The three wrong conclusions pin the fixtures:
//! 1. `$L/endpoints/` (a sibling-directory guess) read empty and was reported
//!    as "zero endpoints"; the writer writes `$L/state/endpoints/`, which held
//!    ten, all healthy.
//! 2. A worker port read from a log line (inferred) sent registrations to the
//!    wrong port; the endpoint file records the real port.
//! 3. `agent.out` read 0 bytes and was reported as "hung"; the writer is
//!    actually generating, so the empty output is unconfirmed against the
//!    writer's real state.
//!
//! The four invariants, one test group each:
//! 1. a source is authoritative only when it is the producer's own path
//!    (`Inference`, `Source`);
//! 2. confirmation is the writer's path, and it matches only when the read
//!    used that exact path — naming a path is not confirmation
//!    (`Confirmation`);
//! 3. an empty read from an unverified source is `Unverified`, never a
//!    finding, because at the call site it is indistinguishable from a wrong
//!    path (`Read`, `verdict`);
//! 4. a dramatic conclusion is `Sound` only when it rests on a verified read
//!    (`Finding`, `finding_verdict`).

use autospec_core::reader_fidelity::{
    finding_verdict, verdict, Confirmation, Finding, FindingVerdict, Inference, Read, ReadVerdict,
    Source, UnverifiedReason,
};

// ── incident fixtures ────────────────────────────────────────────────────

/// The #3682 endpoints read, wrong: a sibling-directory guess that returned
/// empty and was reported as "zero endpoints".
fn endpoints_wrong() -> Read {
    Read::new(
        Source::new(
            "endpoints/",
            Inference::SiblingDirectory {
                sibling: "state/".into(),
            },
        ),
        None,
        0,
    )
}

/// The #3682 endpoints read, corrected: the path the writer itself writes,
/// confirmed against it, holding ten.
fn endpoints_correct() -> Read {
    Read::new(
        Source::new(
            "state/endpoints/",
            Inference::ProducerDeclared {
                producer: "registration".into(),
            },
        ),
        Some(Confirmation::new("state/endpoints/")),
        10,
    )
}

/// The #3682 worker-port read, wrong: a port taken from a line in the
/// worker's log.
fn worker_port_wrong() -> Read {
    Read::new(
        Source::new(
            "worker.log",
            Inference::LogLine {
                log: "worker.log".into(),
            },
        ),
        None,
        1,
    )
}

/// The #3682 worker-port read, corrected: the port from the endpoint file the
/// registration writes.
fn worker_port_correct() -> Read {
    Read::new(
        Source::new(
            "state/endpoints/w3",
            Inference::ProducerDeclared {
                producer: "registration".into(),
            },
        ),
        Some(Confirmation::new("state/endpoints/w3")),
        1,
    )
}

/// The #3682 "agent hung" read, wrong: a 0-byte output file, never confirmed
/// against the writer's real state (which is generating).
fn agent_hung_wrong() -> Read {
    Read::new(
        Source::new(
            "agent.out",
            Inference::ProducerDeclared {
                producer: "agent".into(),
            },
        ),
        None,
        0,
    )
}

/// The #3682 "agent hung" read, corrected: the writer's real state — nine
/// slots generating — confirmed.
fn agent_hung_correct() -> Read {
    Read::new(
        Source::new(
            "worker slot state",
            Inference::ProducerDeclared {
                producer: "worker scheduler".into(),
            },
        ),
        Some(Confirmation::new("worker slot state")),
        9,
    )
}

// ── 1. a source is authoritative only when it is the producer's path ─────

#[test]
fn the_producers_own_path_is_the_only_authoritative_inference() {
    assert!(Inference::ProducerDeclared {
        producer: "registration".into()
    }
    .is_authoritative());
    assert!(!Inference::SiblingDirectory {
        sibling: "state/".into()
    }
    .is_authoritative());
    assert!(!Inference::LogLine {
        log: "worker.log".into()
    }
    .is_authoritative());
    assert!(!Inference::NamingConvention.is_authoritative());
}

#[test]
fn a_source_is_authoritative_only_when_its_inference_is() {
    assert!(endpoints_correct().source.is_authoritative());
    assert!(worker_port_correct().source.is_authoritative());
    assert!(!endpoints_wrong().source.is_authoritative());
    assert!(!worker_port_wrong().source.is_authoritative());
}

// ── 2. confirmation is the writer's path ─────────────────────────────────

#[test]
fn a_confirmation_matches_only_the_writers_exact_path() {
    let c = Confirmation::new("state/endpoints/");
    assert!(c.matches("state/endpoints/"));
    // The #3682 mismatch: the writer writes state/endpoints/, the read used
    // endpoints/.
    assert!(!c.matches("endpoints/"));
}

#[test]
fn naming_a_path_is_not_confirmation() {
    // A producer path with no confirmation is not verified: the reader named
    // the path but never checked the writer.
    let read = Read::new(
        Source::new(
            "state/endpoints/",
            Inference::ProducerDeclared {
                producer: "registration".into(),
            },
        ),
        None,
        10,
    );
    assert!(!read.is_verified());
}

// ── 3. an empty read from an unverified source is unverified ─────────────

#[test]
fn the_3682_zero_endpoints_is_unverified_not_a_finding() {
    // Empty, from a sibling-directory guess, never confirmed. At the call
    // site this is indistinguishable from a wrong path, so it is unverified —
    // not the "zero endpoints" finding the reader reported.
    let read = endpoints_wrong();
    assert_eq!(
        verdict(&read),
        ReadVerdict::Unverified {
            reason: UnverifiedReason {
                inferred: true,
                unconfirmed: true,
            },
        }
    );
}

#[test]
fn the_3682_workers_unreachable_is_unverified_not_a_finding() {
    let read = worker_port_wrong();
    assert_eq!(
        verdict(&read),
        ReadVerdict::Unverified {
            reason: UnverifiedReason {
                inferred: true,
                unconfirmed: true,
            },
        }
    );
}

#[test]
fn the_3682_agent_hung_is_unconfirmed_not_a_finding() {
    // The right file (agent.out, which the agent writes) but an empty result
    // never confirmed against the writer's real state: inferred is false,
    // unconfirmed is true.
    let read = agent_hung_wrong();
    assert_eq!(
        verdict(&read),
        ReadVerdict::Unverified {
            reason: UnverifiedReason {
                inferred: false,
                unconfirmed: true,
            },
        }
    );
}

#[test]
fn a_verified_empty_read_is_a_genuine_zero() {
    // The control: an empty result from a verified source IS a finding — a
    // genuine zero. This is the case an unverified zero is confused with.
    let read = Read::new(
        Source::new(
            "state/endpoints/",
            Inference::ProducerDeclared {
                producer: "registration".into(),
            },
        ),
        Some(Confirmation::new("state/endpoints/")),
        0,
    );
    assert!(read.is_empty());
    assert_eq!(verdict(&read), ReadVerdict::Verified);
}

#[test]
fn the_corrected_reads_are_verified() {
    assert_eq!(verdict(&endpoints_correct()), ReadVerdict::Verified);
    assert_eq!(verdict(&worker_port_correct()), ReadVerdict::Verified);
    assert_eq!(verdict(&agent_hung_correct()), ReadVerdict::Verified);
}

// ── 4. a dramatic conclusion requires a verified read ─────────────────────

#[test]
fn the_3682_zero_endpoints_claim_is_unsupported() {
    let finding = Finding::new(
        "the pipeline is blocked — zero endpoints",
        endpoints_wrong(),
    );
    assert_eq!(
        finding_verdict(&finding),
        FindingVerdict::Unsupported {
            reason: UnverifiedReason {
                inferred: true,
                unconfirmed: true,
            },
        }
    );
}

#[test]
fn a_corrected_finding_is_sound() {
    let finding = Finding::new("ten endpoints, all healthy", endpoints_correct());
    assert_eq!(finding_verdict(&finding), FindingVerdict::Sound);
}

// ── reporting ────────────────────────────────────────────────────────────

#[test]
fn an_unverified_empty_read_never_prints_a_dramatic_conclusion() {
    let read = endpoints_wrong();
    let line = verdict(&read).line(&read);
    assert!(line.starts_with("UNVERIFIED:"));
    // The line names what the source was derived from, not just that it is.
    assert!(line.contains("sibling directory"));
    assert!(line.contains("not confirmed against its writer"));
    assert!(line.contains("grep -rl"));
    // The empty shape is flagged: it is indistinguishable from a wrong path.
    assert!(line.contains("indistinguishable from a wrong path"));
}

#[test]
fn a_verified_read_prints_its_count() {
    let read = endpoints_correct();
    let line = verdict(&read).line(&read);
    assert!(line.starts_with("VERIFIED:"));
    assert!(line.contains("10"));
}

#[test]
fn a_verified_empty_read_is_reported_as_a_genuine_zero() {
    let read = Read::new(
        Source::new(
            "state/endpoints/",
            Inference::ProducerDeclared {
                producer: "registration".into(),
            },
        ),
        Some(Confirmation::new("state/endpoints/")),
        0,
    );
    let line = verdict(&read).line(&read);
    assert!(line.contains("a genuine zero"));
}

#[test]
fn an_unsupported_finding_is_rejected_and_names_the_unverified_read() {
    let finding = Finding::new("four workers are unreachable", worker_port_wrong());
    let line = finding_verdict(&finding).line(&finding);
    assert!(line.starts_with("REJECT:"));
    assert!(line.contains("unverified read"));
    assert!(line.contains("grep -rl"));
}

// ── edge cases ───────────────────────────────────────────────────────────

#[test]
fn a_producer_path_with_a_mismatched_confirmation_is_unconfirmed() {
    // The reader declared the producer's path but confirmed against a
    // different path — the writer writes elsewhere.
    let read = Read::new(
        Source::new(
            "endpoints/",
            Inference::ProducerDeclared {
                producer: "registration".into(),
            },
        ),
        Some(Confirmation::new("state/endpoints/")),
        0,
    );
    assert_eq!(
        verdict(&read),
        ReadVerdict::Unverified {
            reason: UnverifiedReason {
                inferred: false,
                unconfirmed: true,
            },
        }
    );
}

#[test]
fn a_confirmed_guess_is_still_flagged_as_inferred() {
    // A guess that happens to be confirmed against the writer is still
    // derived by guessing: it must be re-derived as producer-declared. It is
    // not verified, and the reason says why.
    let read = Read::new(
        Source::new(
            "state/endpoints/",
            Inference::SiblingDirectory {
                sibling: "endpoints/".into(),
            },
        ),
        Some(Confirmation::new("state/endpoints/")),
        10,
    );
    assert!(!read.is_verified());
    assert_eq!(
        verdict(&read),
        ReadVerdict::Unverified {
            reason: UnverifiedReason {
                inferred: true,
                unconfirmed: false,
            },
        }
    );
}

#[test]
fn a_naming_convention_guess_is_not_authoritative() {
    let read = Read::new(
        Source::new("config.yaml", Inference::NamingConvention),
        None,
        1,
    );
    assert_eq!(
        verdict(&read),
        ReadVerdict::Unverified {
            reason: UnverifiedReason {
                inferred: true,
                unconfirmed: true,
            },
        }
    );
}
