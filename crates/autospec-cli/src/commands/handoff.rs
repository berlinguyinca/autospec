//! `autospec.implementation-handoff.v1` producer (issue #3440).
//!
//! The top-level implementation gateway for automatic spec projects
//! (`docs/specs/2026-08-31-automatic-spec-projects-design.md`). Given a
//! normalized intent plus a single artifact, the producer:
//!
//! 1. probes the installed Autospec workflow surface **side-effect-free**
//!    (one read-only `autospec handoff capabilities` child process);
//! 2. reads local run-state for an interrupted typed run (read-only);
//! 3. decides one of the five typed route states — `run`, `start`,
//!    `split_then_run`, `recover`, `none` — or blocks with a typed
//!    unavailable reason.
//!
//! The producer **never** creates branches, pushes, opens PRs, or merges.
//! When Autospec is definitively unavailable the response carries
//! `route: null` and zero mutations are permitted. Transient or ambiguous
//! probe results fail closed into autospec recovery (`recover`), never
//! into direct dispatch. There is no mutating fallback.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use autospec_core::autonomous::waterfall::sha256_hex;
use serde_json::{json, Value};

use super::CommandFailure;

pub mod conformance;

/// Response schema emitted by `autospec handoff probe`.
pub const HANDOFF_SCHEMA: &str = "autospec.implementation-handoff.v1";
/// Capabilities schema advertised by `autospec handoff capabilities`.
pub const CAPABILITIES_SCHEMA: &str = "autospec.handoff-capabilities.v1";
/// Local run-state schema read for interrupted typed runs.
pub const RUN_STATE_SCHEMA: &str = "autospec.handoff-run-state.v1";

/// The five typed route states (spec §"Route state").
pub const ROUTE_RUN: &str = "run";
pub const ROUTE_START: &str = "start";
pub const ROUTE_SPLIT_THEN_RUN: &str = "split_then_run";
pub const ROUTE_RECOVER: &str = "recover";
pub const ROUTE_NONE: &str = "none";
pub const ALL_ROUTES: [&str; 5] = [
    ROUTE_RUN,
    ROUTE_START,
    ROUTE_SPLIT_THEN_RUN,
    ROUTE_RECOVER,
    ROUTE_NONE,
];

/// Typed unavailable / fail-closed reasons (spec §"availability").
pub const REASON_CLI_MISSING: &str = "cli_missing";
pub const REASON_SURFACE_MISSING: &str = "workflow_surface_missing";
pub const REASON_SURFACE_INCOMPATIBLE: &str = "workflow_surface_incompatible";
pub const REASON_PROBE_TRANSIENT: &str = "probe_transient";
pub const REASON_RUN_STATE_AMBIGUOUS: &str = "run_state_ambiguous";

/// Intent kinds. Only `implement` may dispatch; `explain`/`plan` are
/// read-only and route to `none`.
pub const INTENT_IMPLEMENT: &str = "implement";
pub const INTENT_EXPLAIN: &str = "explain";
pub const INTENT_PLAN: &str = "plan";

const ENTRY_AUTOSPEC_RUN: &str = "autospec-run";
const ENTRY_AUTOSPEC: &str = "autospec";
const ENTRY_AUTOSPEC_SPLIT: &str = "autospec-split";
const ENTRY_AUTOSPEC_RESUME: &str = "autospec-resume";

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    match args.first().map(String::as_str) {
        None | Some("--help") | Some("-h") => {
            print_usage();
            Ok(())
        }
        Some("capabilities") => {
            if args[1..].iter().any(|arg| arg != "--json") {
                return Err(CommandFailure::diagnostic(
                    "autospec handoff capabilities accepts only --json",
                ));
            }
            println!("{}", capabilities_json().to_string());
            Ok(())
        }
        Some("probe") => {
            if args[1..].iter().any(|arg| arg == "--help" || arg == "-h") {
                print_usage();
                return Ok(());
            }
            run_probe(&args[1..])
        }
        Some("conformance") => conformance::run(&args[1..]),
        Some(other) => Err(CommandFailure::diagnostic(format!(
            "unknown autospec handoff subcommand: {other}\n{USAGE}"
        ))),
    }
}

const USAGE: &str = "\
USAGE:
    autospec handoff probe --repo OWNER/NAME --intent TEXT \
[--artifact issue:N|spec:PATH] \
[--correlation ID] \
[--intent-kind implement|explain|plan] \
[--repo-dir PATH]
    autospec handoff capabilities
    autospec handoff conformance --receipt PATH --trace PATH";

fn print_usage() {
    println!("autospec handoff — produce the autospec.implementation-handoff.v1 handoff (side-effect-free)\n\n{USAGE}");
}

/// Capability surface advertised to the handoff producer. This is the
/// marker an installed Autospec must answer with for the workflow surface
/// to count as available.
fn capabilities_json() -> Value {
    json!({
        "schema": CAPABILITIES_SCHEMA,
        "routes": ALL_ROUTES.to_vec(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeState {
    Available,
    Unavailable,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProbeOutcome {
    state: ProbeState,
    /// Typed reason when `state` is `Unavailable` (definitive) or `Unknown`
    /// (transient/ambiguous — fails closed into recovery).
    reason: Option<&'static str>,
    /// Routes the installed workflow surface supports (Available only).
    routes: Vec<&'static str>,
}

fn probe_unavailable(reason: &'static str) -> ProbeOutcome {
    ProbeOutcome {
        state: ProbeState::Unavailable,
        reason: Some(reason),
        routes: Vec::new(),
    }
}

fn probe_unknown(reason: &'static str) -> ProbeOutcome {
    ProbeOutcome {
        state: ProbeState::Unknown,
        reason: Some(reason),
        routes: Vec::new(),
    }
}

/// Side-effect-free capability probe: one read-only child process against
/// the installed `autospec`. No branches, pushes, PRs, or merges happen
/// here — the child only ever answers `handoff capabilities`.
fn probe_capabilities() -> ProbeOutcome {
    let output = match Command::new("autospec")
        .args(["handoff", "capabilities"])
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            return if error.kind() == ErrorKind::NotFound {
                // No autospec on PATH at all: definitive unavailability.
                probe_unavailable(REASON_CLI_MISSING)
            } else {
                // Spawn refused for another reason: ambiguous, fail closed.
                probe_unknown(REASON_PROBE_TRANSIENT)
            };
        }
    };

    if output.status.success() {
        let parsed: Value = match serde_json::from_slice(&output.stdout) {
            Ok(parsed) => parsed,
            Err(_) => return probe_unavailable(REASON_SURFACE_INCOMPATIBLE),
        };
        return if parsed.get("schema").and_then(Value::as_str) == Some(CAPABILITIES_SCHEMA) {
            let routes = parsed
                .get("routes")
                .and_then(Value::as_array)
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|route| ALL_ROUTES.contains(route))
                        .map(|route| {
                            let static_route = ALL_ROUTES
                                .into_iter()
                                .find(|known| *known == route)
                                .unwrap_or(ROUTE_NONE);
                            static_route
                        })
                        .collect()
                })
                .unwrap_or_default();
            ProbeOutcome {
                state: ProbeState::Available,
                reason: None,
                routes,
            }
        } else {
            // Answers JSON but not the v1 capabilities schema: the installed
            // surface is definitively incompatible.
            probe_unavailable(REASON_SURFACE_INCOMPATIBLE)
        };
    }

    if output.status.code() == Some(2) {
        // Exit 2 is the CLI's "unknown command" status: the installed
        // binary predates the handoff capabilities surface — a partial or
        // outdated install, definitively unavailable.
        return probe_unavailable(REASON_SURFACE_MISSING);
    }
    // Any other non-zero status is transient (crash, OOM, tooling error):
    // ambiguous, fail closed into autospec recovery.
    probe_unknown(REASON_PROBE_TRANSIENT)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InterruptedRun {
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RunStateProbe {
    /// No run-state file (or no interrupted entries): no interrupted run.
    None,
    Interrupted(InterruptedRun),
    /// State file present but unreadable/unparseable/unexpected shape:
    /// ambiguous — the caller must fail closed into recovery.
    Ambiguous,
}

fn run_state_path(repo_dir: &Path) -> PathBuf {
    repo_dir
        .join(".autospec")
        .join("state")
        .join("handoff")
        .join("runs.json")
}

/// Read-only interrupted-run detection. A missing file is a clean "no run";
/// anything present but not exactly the v1 run-state schema is ambiguous.
fn probe_run_state(repo_dir: &Path, correlation_id: Option<&str>) -> RunStateProbe {
    let bytes = match fs::read(run_state_path(repo_dir)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return RunStateProbe::None,
        Err(_) => return RunStateProbe::Ambiguous,
    };
    let parsed: Value = match serde_json::from_slice(&bytes) {
        Ok(parsed) => parsed,
        Err(_) => return RunStateProbe::Ambiguous,
    };
    if parsed.get("schema").and_then(Value::as_str) != Some(RUN_STATE_SCHEMA) {
        return RunStateProbe::Ambiguous;
    }
    let runs = match parsed.get("runs").and_then(Value::as_array) {
        Some(runs) => runs,
        None => return RunStateProbe::Ambiguous,
    };
    let mut candidates: Vec<&Value> = runs
        .iter()
        .filter(|entry| entry.get("status").and_then(Value::as_str) == Some("interrupted"))
        .filter(|entry| {
            entry
                .get("run_id")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.is_empty())
        })
        .filter(|entry| {
            correlation_id.map_or(true, |correlation| {
                entry.get("correlation_id").and_then(Value::as_str) == Some(correlation)
            })
        })
        .collect();
    if candidates.is_empty() {
        return RunStateProbe::None;
    }
    // Deterministic pick: lexicographically smallest run_id.
    candidates.sort_by(|a, b| {
        a.get("run_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(b.get("run_id").and_then(Value::as_str).unwrap_or(""))
    });
    let run_id = candidates[0]
        .get("run_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    RunStateProbe::Interrupted(InterruptedRun { run_id })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactKind {
    None,
    Issue,
    Spec,
}

/// The route decision table (spec §"Route state"). Pure: given the probe
/// outcomes and the normalized request, exactly one of the five typed route
/// states is selected, or the request is blocked with a typed reason.
///
/// Precedence:
/// 1. definitively unavailable Autospec            → blocked (route null)
/// 2. read-only intent (explain/plan)              → none
/// 3. ambiguous local run-state                    → recover
/// 4. transient/ambiguous capability probe         → recover
/// 5. interrupted typed run                        → recover
/// 6. required workflow route not on the surface   → blocked (surface missing)
/// 7. issue artifact                               → run
/// 8. spec artifact                                → split_then_run
/// 9. no artifact                                  → start
fn decide_route(
    probe: &ProbeOutcome,
    intent_kind: &str,
    artifact: ArtifactKind,
    run_state: &RunStateProbe,
) -> (
    Option<&'static str>, // route (None = blocked)
    &'static str,         // availability
    Option<&'static str>, // typed reason (unavailable / fail-closed)
) {
    if probe.state == ProbeState::Unavailable {
        return (None, "unavailable", probe.reason);
    }

    let (desired, fail_closed_reason) = if intent_kind != INTENT_IMPLEMENT {
        // Read-only requests never dispatch; a transient probe is recorded
        // but does not change the (non-dispatching) route.
        (
            ROUTE_NONE,
            if probe.state == ProbeState::Unknown {
                probe.reason
            } else {
                None
            },
        )
    } else if matches!(run_state, RunStateProbe::Ambiguous) {
        // Corrupt or unrecognized local run-state: fail closed into
        // autospec recovery, never into direct dispatch.
        (ROUTE_RECOVER, Some(REASON_RUN_STATE_AMBIGUOUS))
    } else if probe.state == ProbeState::Unknown {
        // Transient/ambiguous probe: fail closed into autospec recovery,
        // never into direct dispatch.
        (ROUTE_RECOVER, probe.reason)
    } else if matches!(run_state, RunStateProbe::Interrupted(_)) {
        // An interrupted typed run must resume, not restart.
        (ROUTE_RECOVER, None)
    } else {
        match artifact {
            ArtifactKind::Issue => (ROUTE_RUN, None),
            ArtifactKind::Spec => (ROUTE_SPLIT_THEN_RUN, None),
            ArtifactKind::None => (ROUTE_START, None),
        }
    };

    // Any dispatching route must be on the offered workflow surface.
    // `none` dispatches nothing, so it is exempt from the surface check.
    if desired != ROUTE_NONE
        && probe.state == ProbeState::Available
        && !probe.routes.contains(&desired)
    {
        // The installed surface does not offer the workflow this request
        // needs: definitively unavailable, zero mutations permitted.
        return (None, "unavailable", Some(REASON_SURFACE_MISSING));
    }
    // A fail-closed outcome (transient probe, ambiguous run-state) is an
    // ambiguous observation: report it as `unknown`, never `available`.
    let availability = if fail_closed_reason.is_some() || probe.state != ProbeState::Available {
        "unknown"
    } else {
        "available"
    };
    (Some(desired), availability, fail_closed_reason)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunStatus {
    Proposed,
    Recovered,
}

fn derived_id(domain: &str, parts: &[&str]) -> String {
    let mut input = String::from(HANDOFF_SCHEMA);
    input.push('|');
    input.push_str(domain);
    for part in parts {
        input.push('|');
        input.push_str(part);
    }
    let digest = sha256_hex(input.as_bytes());
    format!("{domain}-{}", &digest[..16])
}

/// Provisional Project binding key (spec §"shared contracts"): the autospec
/// Product/Portfolio key the request would bind to, typed `planned`. The
/// producer does not verify the binding against a store; autospec owns
/// authoritative binding.
fn project_key(repo: &str) -> String {
    let sanitize = |segment: &str| {
        segment
            .to_lowercase()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect::<String>()
            .trim_matches('-')
            .to_string()
    };
    let mut segments = repo.split('/');
    let owner = segments.next().unwrap_or_default();
    let name = segments.next().unwrap_or_default();
    let owner = sanitize(owner);
    let name = sanitize(name);
    format!("product.{owner}__{name}")
}

fn entry_point_for(route: &str) -> (&'static str, Option<&'static str>) {
    match route {
        ROUTE_RUN => (ENTRY_AUTOSPEC_RUN, None),
        ROUTE_START => (ENTRY_AUTOSPEC, None),
        ROUTE_SPLIT_THEN_RUN => (ENTRY_AUTOSPEC_SPLIT, Some(ENTRY_AUTOSPEC_RUN)),
        ROUTE_RECOVER => (ENTRY_AUTOSPEC_RESUME, None),
        _ => ("", None),
    }
}

fn guidance_for(
    route: Option<&str>,
    reason: Option<&str>,
    artifact: ArtifactKind,
) -> Option<String> {
    match route {
        None => Some(format!(
            "Autospec is unavailable ({}); no implementation actions are permitted. \
             Install or repair the autospec CLI, then re-probe. No mutating fallback exists.",
            reason.unwrap_or("")
        )),
        Some(ROUTE_NONE) => Some("Read-only intent: no implementation dispatch.".to_string()),
        Some(ROUTE_RECOVER) => Some(
            "Recovering an interrupted or ambiguous autospec run; route through autospec resume \
             instead of direct dispatch."
                .to_string(),
        ),
        Some(ROUTE_RUN) => match artifact {
            ArtifactKind::Issue => Some(format!(
                "Dispatch {ENTRY_AUTOSPEC_RUN} for the issue artifact."
            )),
            _ => Some("Dispatch autospec-run.".to_string()),
        },
        Some(ROUTE_SPLIT_THEN_RUN) => {
            Some("Split the spec artifact into issues, then dispatch autospec-run.".to_string())
        }
        Some(ROUTE_START) => {
            Some("Start the autospec workflow from the normalized intent.".to_string())
        }
        _ => None,
    }
}

struct ProbeRequest {
    repo: String,
    intent: String,
    artifact_kind: ArtifactKind,
    artifact_ref: Option<String>,
    correlation_id: Option<String>,
    intent_kind: &'static str,
    repo_dir: PathBuf,
}

fn parse_probe_options(args: &[String]) -> Result<ProbeRequest, CommandFailure> {
    let mut request = ProbeRequest {
        repo: String::new(),
        intent: String::new(),
        artifact_kind: ArtifactKind::None,
        artifact_ref: None,
        correlation_id: None,
        intent_kind: INTENT_IMPLEMENT,
        repo_dir: std::env::current_dir().map_err(|error| {
            CommandFailure::diagnostic(format!("cannot resolve current directory: {error}"))
        })?,
    };
    let mut positional = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        let mut value = || -> Result<String, CommandFailure> {
            i += 1;
            args.get(i)
                .cloned()
                .ok_or_else(|| CommandFailure::diagnostic(format!("missing value for {arg}")))
        };
        match arg.as_str() {
            "--repo" => request.repo = value()?,
            "--intent" => request.intent = value()?,
            "--correlation" => {
                let id = value()?;
                if !is_valid_correlation(&id) {
                    return Err(CommandFailure::diagnostic(format!(
                        "--correlation must be 1-200 chars of [A-Za-z0-9._:-], got {id:?}"
                    )));
                }
                request.correlation_id = Some(id);
            }
            "--intent-kind" => {
                let kind = value()?;
                request.intent_kind = match kind.as_str() {
                    INTENT_IMPLEMENT => INTENT_IMPLEMENT,
                    INTENT_EXPLAIN => INTENT_EXPLAIN,
                    INTENT_PLAN => INTENT_PLAN,
                    other => {
                        return Err(CommandFailure::diagnostic(format!(
                            "--intent-kind must be implement|explain|plan, got {other:?}"
                        )))
                    }
                };
            }
            "--artifact" => {
                let artifact = value()?;
                let (kind, target) = parse_artifact(&artifact)?;
                request.artifact_kind = kind;
                request.artifact_ref = Some(target);
            }
            "--repo-dir" => request.repo_dir = PathBuf::from(value()?),
            other if other.starts_with("--") => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec handoff probe option: {other}\n{USAGE}"
                )));
            }
            other => positional.push(other.to_string()),
        }
        i += 1;
    }
    if !positional.is_empty() {
        return Err(CommandFailure::diagnostic(format!(
            "unexpected positional argument: {}\n{USAGE}",
            positional.join(" ")
        )));
    }
    if request.repo.is_empty() {
        return Err(CommandFailure::diagnostic(format!(
            "missing required --repo OWNER/NAME\n{USAGE}"
        )));
    }
    if !is_valid_repo(&request.repo) {
        return Err(CommandFailure::diagnostic(format!(
            "--repo must be OWNER/NAME with safe path segments, got {:?}",
            request.repo
        )));
    }
    if request.intent.is_empty() {
        return Err(CommandFailure::diagnostic(format!(
            "missing required --intent TEXT\n{USAGE}"
        )));
    }
    Ok(request)
}

fn is_valid_correlation(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 200
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
}

fn is_valid_repo(repo: &str) -> bool {
    let mut segments = repo.split('/');
    match (segments.next(), segments.next()) {
        (Some(owner), Some(name)) if segments.next().is_none() => {
            is_valid_repo_segment(owner) && is_valid_repo_segment(name)
        }
        _ => false,
    }
}

fn is_valid_repo_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 100
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn parse_artifact(artifact: &str) -> Result<(ArtifactKind, String), CommandFailure> {
    let (kind, target) = artifact.split_once(':').ok_or_else(|| {
        CommandFailure::diagnostic(format!(
            "--artifact must be issue:N or spec:PATH, got {artifact:?}"
        ))
    })?;
    match kind {
        "issue" => {
            let number = target.parse::<u64>().map_err(|_| {
                CommandFailure::diagnostic(format!(
                    "--artifact issue:N requires a positive integer, got {target:?}"
                ))
            })?;
            if number == 0 {
                return Err(CommandFailure::diagnostic(
                    "--artifact issue:N requires a positive integer, got 0",
                ));
            }
            Ok((ArtifactKind::Issue, artifact.to_string()))
        }
        "spec" => {
            if target.is_empty()
                || target.starts_with('/')
                || target
                    .split('/')
                    .any(|segment| segment == ".." || segment.is_empty())
            {
                return Err(CommandFailure::diagnostic(format!(
                    "--artifact spec:PATH must be a safe repo-relative path, got {target:?}"
                )));
            }
            Ok((ArtifactKind::Spec, artifact.to_string()))
        }
        other => Err(CommandFailure::diagnostic(format!(
            "--artifact must be issue:N or spec:PATH, got kind {other:?}"
        ))),
    }
}

fn run_probe(args: &[String]) -> Result<(), CommandFailure> {
    let request = parse_probe_options(args)?;
    let probe = probe_capabilities();
    let run_state = probe_run_state(&request.repo_dir, request.correlation_id.as_deref());
    let (route, availability, reason) = decide_route(
        &probe,
        request.intent_kind,
        request.artifact_kind,
        &run_state,
    );

    // Correlation identity: caller-supplied or deterministically derived.
    let artifact_str = request.artifact_ref.as_deref().unwrap_or("none");
    let correlation_id = request.correlation_id.clone().unwrap_or_else(|| {
        derived_id(
            "corr",
            &[
                request.repo.as_str(),
                request.intent_kind,
                request.intent.as_str(),
                artifact_str,
            ],
        )
    });

    let response = build_response(
        route,
        availability,
        reason,
        &request,
        &correlation_id,
        &run_state,
    );
    println!("{}", response.to_string());
    Ok(())
}

fn build_response(
    route: Option<&str>,
    availability: &str,
    reason: Option<&str>,
    request: &ProbeRequest,
    correlation_id: &str,
    run_state: &RunStateProbe,
) -> Value {
    let (run, stream, cancellation) = match route {
        Some(ROUTE_NONE) | None => {
            // Read-only (`none`) and blocked (route null) outcomes dispatch
            // nothing: no run, stream, or cancellation identity exists.
            (Value::Null, Value::Null, Value::Null)
        }
        Some(route) => {
            let (entry_point, follow_up) = entry_point_for(route);
            let recovered =
                matches!(run_state, RunStateProbe::Interrupted(_)) && route == ROUTE_RECOVER;
            // Recovery keeps the interrupted run's identity; a fresh route
            // proposes a deterministically derived one.
            let run_id = if recovered {
                match run_state {
                    RunStateProbe::Interrupted(run) => run.run_id.clone(),
                    _ => derived_id("run", &[request.repo.as_str(), correlation_id, route]),
                }
            } else {
                derived_id("run", &[request.repo.as_str(), correlation_id, route])
            };
            let run_status = if recovered {
                RunStatus::Recovered
            } else {
                RunStatus::Proposed
            };
            let run = json!({
                "run_id": run_id,
                "entry_point": entry_point,
                "follow_up": follow_up,
                "status": match run_status {
                    RunStatus::Proposed => "proposed",
                    RunStatus::Recovered => "recovered",
                },
            });
            let stream = json!({
                "stream_id": derived_id("stream", &[request.repo.as_str(), correlation_id, route]),
            });
            let cancellation = json!({
                "token": derived_id("cancel", &[request.repo.as_str(), correlation_id, route]),
            });
            (run, stream, cancellation)
        }
    };

    let project = match route {
        Some(_) => json!({
            "key": project_key(&request.repo),
            "state": match route {
                Some(ROUTE_RECOVER) => "recovered",
                _ => "planned",
            },
        }),
        None => Value::Null,
    };

    json!({
        "schema": HANDOFF_SCHEMA,
        "availability": availability,
        "unavailable_reason": reason,
        "guidance": guidance_for(route, reason, request.artifact_kind),
        "route": route,
        "artifact": request.artifact_ref.as_ref().map(|target| {
            json!({ "kind": match request.artifact_kind {
                ArtifactKind::Issue => "issue",
                ArtifactKind::Spec => "spec",
                ArtifactKind::None => "none",
            }, "ref": target })
        }).unwrap_or(Value::Null),
        "run": run,
        "project": project,
        "stream": stream,
        "cancellation": cancellation,
        "correlation_id": correlation_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn available_probe(routes: &[&'static str]) -> ProbeOutcome {
        ProbeOutcome {
            state: ProbeState::Available,
            reason: None,
            routes: routes.to_vec(),
        }
    }

    fn all_routes_probe() -> ProbeOutcome {
        available_probe(&ALL_ROUTES)
    }

    #[test]
    fn route_table_selects_all_five_states() {
        // issue artifact -> run
        assert_eq!(
            decide_route(
                &all_routes_probe(),
                INTENT_IMPLEMENT,
                ArtifactKind::Issue,
                &RunStateProbe::None
            ),
            (Some(ROUTE_RUN), "available", None)
        );
        // no artifact -> start
        assert_eq!(
            decide_route(
                &all_routes_probe(),
                INTENT_IMPLEMENT,
                ArtifactKind::None,
                &RunStateProbe::None
            ),
            (Some(ROUTE_START), "available", None)
        );
        // spec artifact -> split_then_run
        assert_eq!(
            decide_route(
                &all_routes_probe(),
                INTENT_IMPLEMENT,
                ArtifactKind::Spec,
                &RunStateProbe::None
            ),
            (Some(ROUTE_SPLIT_THEN_RUN), "available", None)
        );
        // interrupted run -> recover
        let interrupted = RunStateProbe::Interrupted(InterruptedRun {
            run_id: "run-abc".to_string(),
        });
        assert_eq!(
            decide_route(
                &all_routes_probe(),
                INTENT_IMPLEMENT,
                ArtifactKind::Issue,
                &interrupted
            ),
            (Some(ROUTE_RECOVER), "available", None)
        );
        // read-only -> none
        assert_eq!(
            decide_route(
                &all_routes_probe(),
                INTENT_EXPLAIN,
                ArtifactKind::Issue,
                &RunStateProbe::None
            ),
            (Some(ROUTE_NONE), "available", None)
        );
        assert_eq!(
            decide_route(
                &all_routes_probe(),
                INTENT_PLAN,
                ArtifactKind::None,
                &RunStateProbe::None
            ),
            (Some(ROUTE_NONE), "available", None)
        );
    }

    #[test]
    fn route_table_blocks_when_definitively_unavailable() {
        let missing = probe_unavailable(REASON_CLI_MISSING);
        for artifact in [ArtifactKind::None, ArtifactKind::Issue, ArtifactKind::Spec] {
            assert_eq!(
                decide_route(&missing, INTENT_IMPLEMENT, artifact, &RunStateProbe::None),
                (None, "unavailable", Some(REASON_CLI_MISSING))
            );
        }
        // Unavailable blocks everything, including read-only requests:
        // the response is route null with the typed reason, zero mutations.
        assert_eq!(
            decide_route(
                &missing,
                INTENT_EXPLAIN,
                ArtifactKind::None,
                &RunStateProbe::None
            ),
            (None, "unavailable", Some(REASON_CLI_MISSING))
        );
    }

    #[test]
    fn route_table_fails_closed_on_transient_probe() {
        let transient = probe_unknown(REASON_PROBE_TRANSIENT);
        assert_eq!(
            decide_route(
                &transient,
                INTENT_IMPLEMENT,
                ArtifactKind::Issue,
                &RunStateProbe::None
            ),
            (Some(ROUTE_RECOVER), "unknown", Some(REASON_PROBE_TRANSIENT))
        );
        // Read-only is unaffected by a transient probe.
        assert_eq!(
            decide_route(
                &transient,
                INTENT_PLAN,
                ArtifactKind::None,
                &RunStateProbe::None
            ),
            (Some(ROUTE_NONE), "unknown", Some(REASON_PROBE_TRANSIENT))
        );
    }

    #[test]
    fn route_table_fails_closed_on_ambiguous_run_state() {
        assert_eq!(
            decide_route(
                &all_routes_probe(),
                INTENT_IMPLEMENT,
                ArtifactKind::None,
                &RunStateProbe::Ambiguous
            ),
            (
                Some(ROUTE_RECOVER),
                "unknown",
                Some(REASON_RUN_STATE_AMBIGUOUS)
            )
        );
    }

    #[test]
    fn route_precedence_is_read_only_then_recovery_then_artifact() {
        let interrupted = RunStateProbe::Interrupted(InterruptedRun {
            run_id: "run-1".to_string(),
        });
        // Read-only wins over an interrupted run.
        assert_eq!(
            decide_route(
                &all_routes_probe(),
                INTENT_EXPLAIN,
                ArtifactKind::None,
                &interrupted
            ),
            (Some(ROUTE_NONE), "available", None)
        );
        // An interrupted run wins over an issue artifact.
        assert_eq!(
            decide_route(
                &all_routes_probe(),
                INTENT_IMPLEMENT,
                ArtifactKind::Issue,
                &interrupted
            ),
            (Some(ROUTE_RECOVER), "available", None)
        );
        // Ambiguous run-state wins over a transient probe's reason.
        let transient = probe_unknown(REASON_PROBE_TRANSIENT);
        assert_eq!(
            decide_route(
                &transient,
                INTENT_IMPLEMENT,
                ArtifactKind::Spec,
                &RunStateProbe::Ambiguous
            ),
            (
                Some(ROUTE_RECOVER),
                "unknown",
                Some(REASON_RUN_STATE_AMBIGUOUS)
            )
        );
    }

    #[test]
    fn route_table_blocks_when_surface_lacks_required_route() {
        // Partial surface: only `run` is offered; a start request is blocked.
        let partial = available_probe(&[ROUTE_RUN]);
        assert_eq!(
            decide_route(
                &partial,
                INTENT_IMPLEMENT,
                ArtifactKind::None,
                &RunStateProbe::None
            ),
            (None, "unavailable", Some(REASON_SURFACE_MISSING))
        );
        // The offered route still dispatches.
        assert_eq!(
            decide_route(
                &partial,
                INTENT_IMPLEMENT,
                ArtifactKind::Issue,
                &RunStateProbe::None
            ),
            (Some(ROUTE_RUN), "available", None)
        );
        // An empty surface blocks even an interrupted-run recovery: the
        // recover workflow is not on the offered surface.
        let empty = available_probe(&[]);
        assert_eq!(
            decide_route(
                &empty,
                INTENT_IMPLEMENT,
                ArtifactKind::Issue,
                &RunStateProbe::Interrupted(InterruptedRun {
                    run_id: "run-1".to_string(),
                })
            ),
            (None, "unavailable", Some(REASON_SURFACE_MISSING))
        );
    }

    #[test]
    fn project_key_is_stable_and_sanitized() {
        assert_eq!(project_key("Acme/My-Repo"), "product.acme__my-repo");
        assert_eq!(project_key("acme/my_repo.v2"), "product.acme__my_repo.v2");
        assert_eq!(project_key("a/b"), "product.a__b");
        assert_eq!(project_key("Acme/My-Repo"), project_key("acme/my-repo"));
    }

    #[test]
    fn derived_ids_are_deterministic_and_domain_separated() {
        let a = derived_id("run", &["o/r", "corr-1", "run"]);
        let b = derived_id("run", &["o/r", "corr-1", "run"]);
        let c = derived_id("stream", &["o/r", "corr-1", "run"]);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.starts_with("run-"));
        assert!(c.starts_with("stream-"));
        assert_eq!(a.len(), 4 + 16);
    }

    #[test]
    fn artifact_parsing_rejects_unsafe_shapes() {
        assert_eq!(parse_artifact("issue:42").unwrap().0, ArtifactKind::Issue);
        assert!(parse_artifact("issue:0").is_err());
        assert!(parse_artifact("issue:abc").is_err());
        assert!(parse_artifact("issue:").is_err());
        assert_eq!(
            parse_artifact("spec:docs/specs/a.md").unwrap().0,
            ArtifactKind::Spec
        );
        assert!(parse_artifact("spec:/etc/passwd").is_err());
        assert!(parse_artifact("spec:../x.md").is_err());
        assert!(parse_artifact("spec:a//b").is_err());
        assert!(parse_artifact("bogus:x").is_err());
        assert!(parse_artifact("issue42").is_err());
    }

    #[test]
    fn repo_and_correlation_parsing_rejects_unsafe_shapes() {
        assert!(is_valid_repo("owner/name"));
        assert!(!is_valid_repo("owner"));
        assert!(!is_valid_repo("owner/name/extra"));
        assert!(!is_valid_repo("own er/name"));
        assert!(!is_valid_repo("/name"));
        assert!(!is_valid_repo("owner/"));
        assert!(is_valid_correlation("corr-1.x_2:y"));
        assert!(!is_valid_correlation(""));
        assert!(!is_valid_correlation("has space"));
        assert!(!is_valid_correlation(&"a".repeat(201)));
    }
}
