//! Evidence fidelity (#3862): a simulation is never read as evidence for a
//! criterion that requires a real dependency.
//!
//! The four invariants, one test group each:
//! 1. a task routes only to an executor that provides what its tests need
//!    (`route_task`); with none, the code is `CAPABILITY-UNAVAILABLE` and
//!    the task stays queued — never dispatched, never passed;
//! 2. a fixture that creates an executable named `docker`/`podman`/`psql`
//!    and prepends it to `PATH` while the criterion says *container* /
//!    *live* / *real* is flagged (`detect_substitution`);
//! 3. a change closes an issue only if it carries a closing keyword for it
//!    (`closure_authorized`) — `Refs #50 (does not close it)` closes nothing;
//! 4. the verdict names the evidence kind (`TestVerdict`), and a shimmed run
//!    is hermetic no matter what else it touched.

use autospec_core::evidence_fidelity::{
    classify_evidence, closure_authorized, criterion_demands_real, detect_substitution, route_task,
    shimmed_binaries, Capability, ClosureVerdict, EvidenceFacts, EvidenceKind, ExecutorProfile,
    RoutingVerdict, SubstitutionVerdict, TestVerdict,
};

// ── fixtures ───────────────────────────────────────────────────────────

/// The #3862 incident in miniature: a bash fixture that creates an
/// executable named `docker`, marks it runnable, and shadows the real thing
/// on `PATH` before the test "runs a container".
const BASH_SHIM_FIXTURE: &str = r#"
shim_dir="$(mktemp -d)"
cat > "$shim_dir/docker" <<'SHIM'
#!/bin/sh
exit 0
SHIM
chmod +x "$shim_dir/docker"
export PATH="$shim_dir:$PATH"
docker run --rm alpine:3.20 true
"#;

/// The same pattern in Rust: a `Path::join` of the binary name and a
/// `Command::env("PATH", …)` of the fixture's directory.
const RUST_SHIM_FIXTURE: &str = r##"
let bin = tempdir.path().join("shims");
std::fs::create_dir_all(&bin)?;
std::fs::write(bin.join("docker"), "#!/bin/sh\nexit 0\n")?;
let path = format!("{}{}:{}", bin.display(), std::env::consts::SEP, std::env::var("PATH").unwrap_or_default());
Command::new("docker").env("PATH", &path).arg("run").arg("--rm").arg("alpine:3.20").status()?;
"##;

/// A live test: it invokes the real `docker`. No executable is created and
/// no fixture directory shadows anything, so nothing may be flagged.
const LIVE_DOCKER_SOURCE: &str = r#"
Command::new("docker").arg("run").arg("--rm").arg("alpine:3.20").status()?;
"#;

// ── 1. capability routing ─────────────────────────────────────────────

#[test]
fn container_task_routes_to_the_capable_executor() {
    let required = [Capability::ContainerRuntime];
    let bare_worker = ExecutorProfile::new("worker-a", []);
    let capable_worker = ExecutorProfile::new("worker-b", [Capability::ContainerRuntime]);

    let verdict = route_task(&required, &[bare_worker, capable_worker]);
    assert!(verdict.is_dispatch());
    assert_eq!(
        verdict,
        RoutingVerdict::Dispatched {
            executor_id: "worker-b".to_string()
        }
    );
    assert_eq!(verdict.as_code(), "DISPATCHED");
}

#[test]
fn container_task_without_a_capable_executor_is_capability_unavailable() {
    let required = [Capability::ContainerRuntime];
    let bare_worker = ExecutorProfile::new("worker-a", []);

    let verdict = route_task(&required, &[bare_worker]);
    assert!(!verdict.is_dispatch());
    assert_eq!(
        verdict,
        RoutingVerdict::CapabilityUnavailable {
            missing: vec![Capability::ContainerRuntime]
        }
    );
    assert_eq!(verdict.as_code(), "CAPABILITY-UNAVAILABLE");
}

#[test]
fn empty_pool_is_capability_unavailable() {
    let required = [Capability::ContainerRuntime];
    let verdict = route_task(&required, &[]);
    assert!(!verdict.is_dispatch());
    assert_eq!(verdict.as_code(), "CAPABILITY-UNAVAILABLE");
    assert_eq!(
        verdict,
        RoutingVerdict::CapabilityUnavailable {
            missing: vec![Capability::ContainerRuntime]
        }
    );
}

#[test]
fn routing_ties_break_on_executor_id() {
    let required = [Capability::ContainerRuntime];
    let second = ExecutorProfile::new("worker-b", [Capability::ContainerRuntime]);
    let first = ExecutorProfile::new("worker-a", [Capability::ContainerRuntime]);

    let verdict = route_task(&required, &[second, first]);
    assert_eq!(
        verdict,
        RoutingVerdict::Dispatched {
            executor_id: "worker-a".to_string()
        }
    );
}

#[test]
fn a_task_without_requirements_routes_to_any_executor() {
    let bare_worker = ExecutorProfile::new("worker-a", []);
    let verdict = route_task(&[], &[bare_worker]);
    assert_eq!(
        verdict,
        RoutingVerdict::Dispatched {
            executor_id: "worker-a".to_string()
        }
    );
}

#[test]
fn capability_names_round_trip_and_reject_unknowns() {
    assert_eq!(
        Capability::parse("container_runtime"),
        Some(Capability::ContainerRuntime)
    );
    assert_eq!(Capability::parse("docker"), None);
    assert_eq!(Capability::ContainerRuntime.as_str(), "container_runtime");
}

// ── 2. substitution detection ──────────────────────────────────────────

#[test]
fn the_incident_pattern_is_flagged() {
    let criterion = "Fresh VM/container E2E: the image starts from a pinned \
digest, ports 8080 and 9090 are bound, and /healthz returns 200 on a fresh \
host.";
    let verdict = detect_substitution(criterion, BASH_SHIM_FIXTURE);
    assert_eq!(
        verdict,
        SubstitutionVerdict::Flagged {
            binaries: vec!["docker"],
            criterion_term: "container",
        }
    );
    assert!(verdict.is_flagged());
}

#[test]
fn the_rust_shape_of_the_same_pattern_is_flagged() {
    let verdict = detect_substitution("container smoke test", RUST_SHIM_FIXTURE);
    assert_eq!(
        verdict,
        SubstitutionVerdict::Flagged {
            binaries: vec!["docker"],
            criterion_term: "container",
        }
    );
}

#[test]
fn a_psql_shim_under_a_live_criterion_is_flagged() {
    let fixture = r#"
mkdir -p "$fakelib"
printf '#!/bin/sh\necho 0\n' > "$fakelib/psql"
chmod +x "$fakelib/psql"
export PATH="$fakelib:$PATH"
psql -c 'select 1'
"#;
    let verdict = detect_substitution("the live database accepts the migration", fixture);
    assert_eq!(
        verdict,
        SubstitutionVerdict::Flagged {
            binaries: vec!["psql"],
            criterion_term: "live",
        }
    );
}

#[test]
fn a_podman_shim_is_flagged_too() {
    let fixture = r#"
cat > "$dir/podman" <<'SHIM'
#!/bin/sh
exit 0
SHIM
export PATH="$dir:$PATH"
"#;
    let verdict = detect_substitution("a real build from a pinned digest", fixture);
    assert_eq!(
        verdict,
        SubstitutionVerdict::Flagged {
            binaries: vec!["podman"],
            criterion_term: "real",
        }
    );
}

#[test]
fn invoking_the_real_docker_is_not_flagged() {
    assert!(shimmed_binaries(LIVE_DOCKER_SOURCE).is_empty());
    let verdict = detect_substitution("container smoke test", LIVE_DOCKER_SOURCE);
    assert_eq!(verdict, SubstitutionVerdict::Clean);
    assert!(!verdict.is_flagged());
}

#[test]
fn a_shim_under_a_hermetic_criterion_is_not_flagged() {
    // The criterion names no real dependency, so a shim cannot violate it.
    let verdict = detect_substitution("the parser round-trips the manifest", BASH_SHIM_FIXTURE);
    assert_eq!(verdict, SubstitutionVerdict::Clean);
}

#[test]
fn a_file_named_docker_that_never_reaches_path_is_not_flagged() {
    let source = r#"
std::fs::write(dir.join("docker"), "notes about docker")?;
"#;
    assert!(shimmed_binaries(source).is_empty());
}

#[test]
fn criterion_terms_match_case_insensitively() {
    assert_eq!(
        criterion_demands_real("FRESH Container E2E"),
        Some("container")
    );
    assert_eq!(criterion_demands_real("a LIVE endpoint"), Some("live"));
    assert_eq!(criterion_demands_real("the REAL database"), Some("real"));
    assert_eq!(
        criterion_demands_real("the parser handles the manifest"),
        None
    );
}

// ── 3. closing-keyword authority ───────────────────────────────────────

#[test]
fn closing_verbs_authorize_closure() {
    assert_eq!(
        closure_authorized("Closes #50", 50),
        ClosureVerdict::Authorized { verb: "closes" }
    );
    assert_eq!(
        closure_authorized("fixes #50", 50),
        ClosureVerdict::Authorized { verb: "fixes" }
    );
    assert_eq!(
        closure_authorized("RESOLVED #50", 50),
        ClosureVerdict::Authorized { verb: "resolved" }
    );
    assert_eq!(
        closure_authorized("the gate is green; closed #50", 50),
        ClosureVerdict::Authorized { verb: "closed" }
    );
    // Punctuation after the reference does not break the match.
    assert!(closure_authorized("closes #50.", 50).is_authorized());
}

#[test]
fn refs_and_prose_never_authorize_closure() {
    // The exact commit message of the #3862 incident.
    assert!(!closure_authorized("Refs #50 (does not close it)", 50).is_authorized());
    assert_eq!(
        closure_authorized("Refs #50", 50),
        ClosureVerdict::NotAuthorized
    );
    assert_eq!(
        closure_authorized("related to #50", 50),
        ClosureVerdict::NotAuthorized
    );
    // A bare reference, or a verb that does not sit next to the reference.
    assert_eq!(closure_authorized("#50", 50), ClosureVerdict::NotAuthorized);
    assert!(!closure_authorized("This fixes the bug in #50", 50).is_authorized());
    // A green gate is not a verb.
    assert!(!closure_authorized("test_passed=586 test_failed=0 — see #50", 50).is_authorized());
}

#[test]
fn a_closing_keyword_for_another_issue_does_not_authorize() {
    assert_eq!(
        closure_authorized("Closes #500", 50),
        ClosureVerdict::NotAuthorized
    );
    assert_eq!(
        closure_authorized("Closes #5", 50),
        ClosureVerdict::NotAuthorized
    );
}

// ── 4. evidence kind in the verdict ───────────────────────────────────

#[test]
fn a_shimmed_run_is_hermetic_no_matter_what_else_it_touched() {
    let facts = EvidenceFacts {
        shimmed_dependency: true,
        real_container_run: true, // the shim "ran"
        real_endpoint: true,      // the fake endpoint "answered"
    };
    assert_eq!(classify_evidence(facts), EvidenceKind::Hermetic);
}

#[test]
fn the_evidence_ladder_orders_on_what_actually_ran() {
    assert_eq!(
        classify_evidence(EvidenceFacts {
            shimmed_dependency: false,
            real_container_run: true,
            real_endpoint: true,
        }),
        EvidenceKind::Live
    );
    assert_eq!(
        classify_evidence(EvidenceFacts {
            shimmed_dependency: false,
            real_container_run: true,
            real_endpoint: false,
        }),
        EvidenceKind::Integration
    );
    assert_eq!(
        classify_evidence(EvidenceFacts {
            shimmed_dependency: false,
            real_container_run: false,
            real_endpoint: true,
        }),
        EvidenceKind::Integration
    );
    assert_eq!(
        classify_evidence(EvidenceFacts {
            shimmed_dependency: false,
            real_container_run: false,
            real_endpoint: false,
        }),
        EvidenceKind::Hermetic
    );
}

#[test]
fn the_verdict_names_the_kind_not_just_the_count() {
    let hermetic = TestVerdict::new(
        586,
        0,
        EvidenceFacts {
            shimmed_dependency: true,
            ..Default::default()
        },
    );
    assert_eq!(hermetic.kind, EvidenceKind::Hermetic);
    assert_eq!(
        hermetic.render(),
        "586 passed, 0 failed (evidence: hermetic)"
    );

    let live = TestVerdict::new(
        586,
        0,
        EvidenceFacts {
            shimmed_dependency: false,
            real_container_run: true,
            real_endpoint: true,
        },
    );
    assert_eq!(live.kind, EvidenceKind::Live);
    assert_eq!(live.render(), "586 passed, 0 failed (evidence: live)");
}

#[test]
fn only_live_evidence_bears_on_a_live_criterion() {
    assert!(EvidenceKind::Live.bears_on_live_criterion());
    assert!(!EvidenceKind::Integration.bears_on_live_criterion());
    assert!(!EvidenceKind::Hermetic.bears_on_live_criterion());
}

#[test]
fn evidence_kind_words_round_trip_and_reject_unknowns() {
    assert_eq!(
        EvidenceKind::parse("hermetic"),
        Some(EvidenceKind::Hermetic)
    );
    assert_eq!(
        EvidenceKind::parse("integration"),
        Some(EvidenceKind::Integration)
    );
    assert_eq!(EvidenceKind::parse("live"), Some(EvidenceKind::Live));
    assert_eq!(EvidenceKind::parse("prod"), None);
}

// ── the incident, end to end ───────────────────────────────────────────

#[test]
fn incident_3862_end_to_end() {
    let criterion = "Fresh VM/container E2E: the image starts from a pinned \
digest, ports 8080 and 9090 are bound, and /healthz returns 200 on a fresh \
host.";

    // 2. The run is a substitution: a `docker` shim on `PATH` under a
    //    container criterion.
    let substitution = detect_substitution(criterion, BASH_SHIM_FIXTURE);
    assert!(substitution.is_flagged());

    // 4. Whatever the shim printed, the evidence is hermetic at best — and
    //    hermetic does not bear on the criterion.
    let facts = EvidenceFacts {
        shimmed_dependency: !shimmed_binaries(BASH_SHIM_FIXTURE).is_empty(),
        real_container_run: true,
        real_endpoint: true,
    };
    let verdict = TestVerdict::new(586, 0, facts);
    assert_eq!(verdict.kind, EvidenceKind::Hermetic);
    assert!(!verdict.kind.bears_on_live_criterion());

    // 3. And the change that produced it cannot close the issue.
    assert!(!closure_authorized("Refs #50 (does not close it)", 50).is_authorized());

    // 1. Upstream, the task should not have been routed to a worker that
    //    lacks the runtime in the first place.
    let bare_worker = ExecutorProfile::new("worker-a", []);
    assert_eq!(
        route_task(&[Capability::ContainerRuntime], &[bare_worker]).as_code(),
        "CAPABILITY-UNAVAILABLE"
    );
}
