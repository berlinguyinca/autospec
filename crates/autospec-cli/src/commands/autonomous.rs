use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use autospec_core::autonomous::config::AutonomousConfig;
use autospec_core::autonomous::mainline_health::{
    apply_ignored_checks, check_run_evidence, evaluate_health, legacy_status_evidence,
    resolve_health_branch, CheckEvidence, HealthBranchInput, MainlineHealth,
    MainlineHealthDiagnostic, MainlineHealthOutcome,
};
use autospec_core::autonomous_lifecycle::{
    decide as decide_lifecycle, Budget as LifecycleBudget, CapacityDecision, ClaimBranch,
    ClaimContext, ConductorLeaseDecision, Health as LifecycleHealth, IssueNumber, LeaseFreshness,
    LifecycleDecision, LifecycleInput, LifecycleRecord, LifecycleReject, LifecycleTier,
    LifecycleTransition, ParkReason as LifecycleParkReason, RepositoryScope,
    StopMode as LifecycleStopMode, WorkerId,
};
use autospec_core::coordination::{
    ConductorEvent, ConductorOutcome, ConductorPhase, ConductorScope, ConductorState,
};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::{claim, queue, CommandFailure};

pub(crate) mod drain;
mod resilience;
// Task 1 owns only the read-only adapter; Task 2 wires its sealed receipt path.
#[allow(dead_code)]
mod tier15;
// Task 2 persists sealed observations but has no foreground caller by design.
#[allow(dead_code)]
mod tier15_receipts;
// Task 3 adds a private Tier 2 persistence seam without foreground wiring.
#[allow(dead_code)]
mod tier2;
#[allow(dead_code)]
mod tier2_receipts;
// Tier 3 remains disabled in production until a typed metadata package exists.
#[cfg(test)]
mod tier2_receipts_failure_prefix_tests;
#[cfg(test)]
mod tier2_receipts_recovery_tests;
#[cfg(test)]
mod tier2_receipts_tests;
#[allow(dead_code)]
mod tier3;
#[allow(dead_code)]
mod tier3_receipts;
// Tier 4 remains a sealed receipt boundary without foreground wiring.
#[cfg(test)]
mod tier3_receipts_failure_prefix_tests;
#[cfg(test)]
mod tier3_receipts_recovery_tests;
#[cfg(test)]
mod tier3_receipts_tests;
#[allow(dead_code)]
mod tier4;
#[allow(dead_code)]
mod tier4_receipts;
#[cfg(test)]
mod tier4_receipts_failure_prefix_tests;
#[cfg(test)]
mod tier4_receipts_recovery_tests;
#[cfg(test)]
mod tier4_receipts_state_tests;
#[cfg(test)]
mod tier4_receipts_tests;
mod waterfall;
mod waterfall_coordinator;
#[cfg(test)]
mod waterfall_tests;

const FOREGROUND_WORKER_PREFIX: &str = "rust-foreground-conductor";
const DEFERRED_EXECUTOR_REASON: &str = "awaiting_typed_implementation_executor";
static ATOMIC_WRITE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
struct Options {
    raw_args: Vec<String>,
    subcommand: String,
    repo: String,
    repo_dir: String,
    pid: String,
    interval_sec: u64,
    drain_stall_secs: u64,
    drain_poll_secs: u64,
    lines: usize,
    iterations: u64,
    all: bool,
    dry_run: bool,
    once: bool,
    json: bool,
    foreground: bool,
    force: bool,
    log_path: String,
    max_cycles: String,
    budget_tokens: Option<u64>,
    budget_issues: Option<u64>,
    no_digest: bool,
    health_branch: Option<String>,
    issue: Option<u64>,
    stop_mode: StopMode,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            raw_args: Vec::new(),
            subcommand: "start".to_string(),
            repo: "unknown".to_string(),
            repo_dir: ".".to_string(),
            pid: String::new(),
            interval_sec: 300,
            drain_stall_secs: 1_800,
            drain_poll_secs: 15,
            lines: 50,
            iterations: 0,
            all: false,
            dry_run: false,
            once: false,
            json: false,
            foreground: false,
            force: false,
            log_path: String::new(),
            max_cycles: String::new(),
            budget_tokens: None,
            budget_issues: None,
            no_digest: false,
            health_branch: None,
            issue: None,
            stop_mode: StopMode::Graceful,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopMode {
    Graceful,
    Immediate,
}

impl StopMode {
    fn as_str(self) -> &'static str {
        match self {
            StopMode::Graceful => "graceful",
            StopMode::Immediate => "immediate",
        }
    }
}

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    if args.first().is_some_and(|arg| arg == "resilience") {
        return resilience::run(&args[1..]);
    }
    if args.first().is_some_and(|arg| arg == "executor-result") {
        return executor_result(args);
    }
    if args.first().is_some_and(|arg| arg == "lifecycle") {
        return lifecycle(args);
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return Ok(());
    }
    let options = parse(args).map_err(CommandFailure::diagnostic)?;
    if options.subcommand == "run-foreground"
        || (options.subcommand == "start" && options.foreground)
    {
        return run_foreground(options);
    }
    match options.subcommand.as_str() {
        "start" => start(options),
        "restart" => restart(options),
        "status" => status(options),
        "monitor" => monitor(options).map_err(CommandFailure::diagnostic),
        "supervise" => supervise(options).map_err(CommandFailure::diagnostic),
        "stop" => stop(options).map_err(CommandFailure::diagnostic),
        "list" => list(options).map_err(CommandFailure::diagnostic),
        "logs" => logs(options).map_err(CommandFailure::diagnostic),
        "watch" => watch(options).map_err(CommandFailure::diagnostic),
        "timeline" => timeline(options).map_err(CommandFailure::diagnostic),
        "cleanup" => cleanup(options).map_err(CommandFailure::diagnostic),
        "main-health" => main_health(options).map_err(CommandFailure::diagnostic),
        "drain" => drain::run(options),
        other => Err(CommandFailure::diagnostic(format!(
            "unknown autospec autonomous subcommand: {other}"
        ))),
    }
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        raw_args: args.to_vec(),
        ..Default::default()
    };
    let mut index = 0;
    if let Some(first) = args.first() {
        if !first.starts_with('-') {
            options.subcommand = first.clone();
            index = 1;
        }
    }

    while index < args.len() {
        match args[index].as_str() {
            "--repo" => {
                index += 1;
                options.repo = args
                    .get(index)
                    .cloned()
                    .ok_or_else(|| "--repo requires a value".to_string())?;
            }
            "--repo-dir" => {
                index += 1;
                options.repo_dir = args
                    .get(index)
                    .cloned()
                    .ok_or_else(|| "--repo-dir requires a value".to_string())?;
            }
            "--pid" => {
                index += 1;
                options.pid = args
                    .get(index)
                    .cloned()
                    .ok_or_else(|| "--pid requires a value".to_string())?;
            }
            "--interval-sec" => {
                index += 1;
                let raw = args
                    .get(index)
                    .ok_or_else(|| "--interval-sec requires a value".to_string())?;
                options.interval_sec = raw
                    .parse::<u64>()
                    .map_err(|_| format!("invalid --interval-sec value: {raw}"))?;
            }
            "--poll-interval-sec" => {
                index += 1;
                let raw = args
                    .get(index)
                    .ok_or_else(|| "--poll-interval-sec requires a value".to_string())?;
                options.interval_sec = raw
                    .parse::<u64>()
                    .map_err(|_| format!("invalid --poll-interval-sec value: {raw}"))?;
            }
            "--stall-secs" => {
                index += 1;
                let raw = args
                    .get(index)
                    .ok_or_else(|| "--stall-secs requires a value".to_string())?;
                options.drain_stall_secs = parse_positive_duration(raw, "--stall-secs")?;
            }
            "--poll-secs" => {
                index += 1;
                let raw = args
                    .get(index)
                    .ok_or_else(|| "--poll-secs requires a value".to_string())?;
                options.drain_poll_secs = parse_positive_duration(raw, "--poll-secs")?;
            }
            "--max-cycles" => {
                index += 1;
                options.max_cycles = args
                    .get(index)
                    .cloned()
                    .ok_or_else(|| "--max-cycles requires a value".to_string())?;
            }
            "--budget-tokens" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--budget-tokens requires a value".to_string())?;
                options.budget_tokens = Some(parse_lifetime_budget(value, "--budget-tokens")?);
            }
            "--budget-hours" => {
                index += 1;
                let _ = args
                    .get(index)
                    .ok_or_else(|| "--budget-hours requires a value".to_string())?;
                return Err(
                    "--budget-hours is not supported until the Rust conductor owns wall-time enforcement"
                        .to_string(),
                );
            }
            "--budget-issues" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--budget-issues requires a value".to_string())?;
                options.budget_issues = Some(parse_lifetime_budget(value, "--budget-issues")?);
            }
            "--lines" => {
                index += 1;
                let raw = args
                    .get(index)
                    .ok_or_else(|| "--lines requires a value".to_string())?;
                options.lines = raw
                    .parse::<usize>()
                    .map_err(|_| format!("invalid --lines value: {raw}"))?;
            }
            "--iterations" => {
                index += 1;
                let raw = args
                    .get(index)
                    .ok_or_else(|| "--iterations requires a value".to_string())?;
                options.iterations = raw
                    .parse::<u64>()
                    .map_err(|_| format!("invalid --iterations value: {raw}"))?;
            }
            "--log" => {
                index += 1;
                options.log_path = args
                    .get(index)
                    .cloned()
                    .ok_or_else(|| "--log requires a value".to_string())?;
            }
            "--all" => options.all = true,
            "--dry-run" => options.dry_run = true,
            "--no-digest" => options.no_digest = true,
            "--branch" => options.health_branch = Some(option_value(args, &mut index, "--branch")?),
            "--issue" => {
                let issue = option_value(args, &mut index, "--issue")?
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| "--issue must be a positive integer".to_string())?;
                if options.issue.replace(issue).is_some() {
                    return Err("--issue accepts exactly one issue number".to_string());
                }
            }
            "--once" => options.once = true,
            "--json" => options.json = true,
            "--foreground" => options.foreground = true,
            "--force" => options.force = true,
            "--graceful" => options.stop_mode = StopMode::Graceful,
            "--immediate" => options.stop_mode = StopMode::Immediate,
            unknown => return Err(format!("unknown autospec autonomous option: {unknown}")),
        }
        index += 1;
    }
    Ok(options)
}

fn parse_lifetime_budget(value: &str, flag: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("{flag} must be a non-negative integer"))
}

fn parse_positive_duration(value: &str, flag: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("{flag} must be a positive integer"))
}

fn lifecycle(args: &[String]) -> Result<(), CommandFailure> {
    let input = parse_lifecycle_input(args).map_err(CommandFailure::diagnostic)?;
    emit_lifecycle_decision(decide_lifecycle(&input))
}

fn parse_lifecycle_input(args: &[String]) -> Result<LifecycleInput, String> {
    if args.get(1).map(String::as_str) != Some("decide") {
        return Err("autonomous lifecycle requires the decide action".to_string());
    }

    let mut repo = None;
    let mut claim_repo = None;
    let mut claim_issue = None;
    let mut claim_worker = None;
    let mut claim_branch = None;
    let mut claim_state = None;
    let mut lease_age_secs = None;
    let mut stop = None;
    let mut health = None;
    let mut budget = None;
    let mut ready_tier = None;
    let mut index = 2;

    while index < args.len() {
        let flag = &args[index];
        let value = executor_result_option_value(args, &mut index, flag)?;
        match flag.as_str() {
            "--repo" => set_executor_result_value(&mut repo, value, "--repo")?,
            "--claim-repo" => set_executor_result_value(&mut claim_repo, value, "--claim-repo")?,
            "--claim-issue" => {
                set_executor_result_number(&mut claim_issue, value, "--claim-issue")?
            }
            "--claim-worker" => {
                set_executor_result_value(&mut claim_worker, value, "--claim-worker")?
            }
            "--claim-branch" => {
                set_executor_result_value(&mut claim_branch, value, "--claim-branch")?
            }
            "--claim-state" => set_executor_result_value(&mut claim_state, value, "--claim-state")?,
            "--lease-age-sec" => {
                let value = value
                    .parse::<u64>()
                    .map_err(|_| "--lease-age-sec must be a non-negative integer".to_string())?;
                if lease_age_secs.replace(value).is_some() {
                    return Err("--lease-age-sec may be specified only once".to_string());
                }
            }
            "--stop" => set_executor_result_value(&mut stop, value, "--stop")?,
            "--health" => set_executor_result_value(&mut health, value, "--health")?,
            "--budget" => set_executor_result_value(&mut budget, value, "--budget")?,
            "--ready-tier" => set_executor_result_value(&mut ready_tier, value, "--ready-tier")?,
            unknown => return Err(format!("unknown autonomous lifecycle option: {unknown}")),
        }
        index += 1;
    }

    let repo = repo.ok_or_else(|| "autonomous lifecycle requires --repo".to_string())?;
    let scope = parse_lifecycle_scope(&repo, "--repo")?;
    let lease = lease_age_secs
        .map(LeaseFreshness::from_age_secs)
        .unwrap_or(LeaseFreshness::Fresh);
    let mut input = LifecycleInput::from_scope(scope);
    let has_claim_input = claim_repo.is_some()
        || claim_issue.is_some()
        || claim_worker.is_some()
        || claim_branch.is_some()
        || claim_state.is_some();
    if has_claim_input {
        let claim_scope = parse_lifecycle_scope(
            &claim_repo.ok_or_else(|| "claim input requires --claim-repo".to_string())?,
            "--claim-repo",
        )?;
        let issue = IssueNumber::new(
            claim_issue.ok_or_else(|| "claim input requires --claim-issue".to_string())?,
        )
        .ok_or_else(|| "--claim-issue must be a positive integer".to_string())?;
        let worker_value =
            claim_worker.ok_or_else(|| "claim input requires --claim-worker".to_string())?;
        let worker = WorkerId::try_from(worker_value.as_str())
            .map_err(|reason| format!("--claim-worker {reason}"))?;
        let branch_value =
            claim_branch.ok_or_else(|| "claim input requires --claim-branch".to_string())?;
        let branch = ClaimBranch::try_from(branch_value.as_str())
            .map_err(|reason| format!("--claim-branch {reason}"))?;
        let state = claim_state.unwrap_or_else(|| "active".to_string());
        let claim = match state.as_str() {
            "active" => ClaimContext::active(claim_scope, issue, worker, branch, lease),
            "terminal" => ClaimContext::terminal(claim_scope, issue, worker, branch),
            _ => return Err("--claim-state must be active or terminal".to_string()),
        };
        input = input.with_claim(claim);
    } else if let Some(lease_age_secs) = lease_age_secs {
        input = input.with_lease_age_secs(lease_age_secs);
    }
    if let Some(stop) = stop {
        input = input.with_stop(parse_lifecycle_stop(&stop)?);
    }
    if let Some(health) = health {
        input = input.with_health(parse_lifecycle_health(&health)?);
    }
    if let Some(budget) = budget {
        input = input.with_budget(parse_lifecycle_budget(&budget)?);
    }
    if let Some(tier) = ready_tier {
        input = input.with_tier(parse_lifecycle_tier(&tier)?);
    }
    Ok(input)
}

fn parse_lifecycle_scope(value: &str, flag: &str) -> Result<RepositoryScope, String> {
    RepositoryScope::try_from(value).map_err(|reason| format!("{flag} {reason}"))
}

fn parse_lifecycle_stop(value: &str) -> Result<LifecycleStopMode, String> {
    match value {
        "graceful" => Ok(LifecycleStopMode::Graceful),
        "immediate" => Ok(LifecycleStopMode::Immediate),
        _ => Err("--stop must be graceful or immediate".to_string()),
    }
}

fn parse_lifecycle_health(value: &str) -> Result<LifecycleHealth, String> {
    match value {
        "continue" => Ok(LifecycleHealth::Continue),
        "wait" => Ok(LifecycleHealth::Wait),
        "halt" => Ok(LifecycleHealth::Halt),
        _ => Err("--health must be continue, wait, or halt".to_string()),
    }
}

fn parse_lifecycle_budget(value: &str) -> Result<LifecycleBudget, String> {
    match value {
        "within" => Ok(LifecycleBudget::WithinCap),
        "soft" => Ok(LifecycleBudget::SoftCap),
        "hard" => Ok(LifecycleBudget::HardCap),
        _ => Err("--budget must be within, soft, or hard".to_string()),
    }
}

fn parse_lifecycle_tier(value: &str) -> Result<LifecycleTier, String> {
    match value {
        "1" => Ok(LifecycleTier::Tier1),
        "1.5" => Ok(LifecycleTier::Tier15),
        "2" => Ok(LifecycleTier::Tier2),
        "3" => Ok(LifecycleTier::Tier3),
        "4" => Ok(LifecycleTier::Tier4),
        "5" => Ok(LifecycleTier::Tier5),
        "6" => Ok(LifecycleTier::Tier6),
        "7" => Ok(LifecycleTier::Tier7),
        "idle" => Ok(LifecycleTier::Idle),
        _ => Err("--ready-tier must be 1, 1.5, 2, 3, 4, 5, 6, 7, or idle".to_string()),
    }
}

fn emit_lifecycle_decision(decision: LifecycleDecision) -> Result<(), CommandFailure> {
    if lifecycle_decision_exit_code(&decision) == 0 {
        println!("{}", lifecycle_decision_json(&decision));
        Ok(())
    } else {
        Err(lifecycle_non_run(&decision))
    }
}

fn lifecycle_non_run(decision: &LifecycleDecision) -> CommandFailure {
    println!("{}", lifecycle_decision_json(decision));
    CommandFailure::status(String::new(), lifecycle_decision_exit_code(decision))
}

fn lifecycle_decision_json(decision: &LifecycleDecision) -> String {
    match decision {
        LifecycleDecision::Run { tier } => format!(
            "{{\"decision\":\"run\",\"tier\":\"{}\"}}",
            lifecycle_tier_name(*tier)
        ),
        LifecycleDecision::Stop { mode } => format!(
            "{{\"decision\":\"stop\",\"mode\":\"{}\"}}",
            lifecycle_stop_name(*mode)
        ),
        LifecycleDecision::Park { reason } => format!(
            "{{\"decision\":\"park\",\"reason\":\"{}\"}}",
            lifecycle_park_name(*reason)
        ),
        LifecycleDecision::Reject(reason) => format!(
            "{{\"decision\":\"reject\",\"reason\":\"{}\"}}",
            lifecycle_reject_name(*reason)
        ),
    }
}

fn lifecycle_decision_exit_code(decision: &LifecycleDecision) -> i32 {
    match decision {
        LifecycleDecision::Run { .. } => 0,
        LifecycleDecision::Stop { .. } | LifecycleDecision::Park { .. } => 20,
        LifecycleDecision::Reject(_) => 3,
    }
}

fn lifecycle_tier_name(tier: LifecycleTier) -> &'static str {
    match tier {
        LifecycleTier::Tier1 => "1",
        LifecycleTier::Tier15 => "1.5",
        LifecycleTier::Tier2 => "2",
        LifecycleTier::Tier3 => "3",
        LifecycleTier::Tier4 => "4",
        LifecycleTier::Tier5 => "5",
        LifecycleTier::Tier6 => "6",
        LifecycleTier::Tier7 => "7",
        LifecycleTier::Idle => "idle",
    }
}

fn lifecycle_stop_name(mode: LifecycleStopMode) -> &'static str {
    match mode {
        LifecycleStopMode::Graceful => "graceful",
        LifecycleStopMode::Immediate => "immediate",
    }
}

fn lifecycle_park_name(reason: LifecycleParkReason) -> &'static str {
    match reason {
        LifecycleParkReason::HumanGate => "human_gate",
        LifecycleParkReason::HealthWait => "health_wait",
        LifecycleParkReason::HealthHalt => "health_halt",
        LifecycleParkReason::BudgetSoftCap => "budget_soft_cap",
        LifecycleParkReason::BudgetHardCap => "budget_hard_cap",
        LifecycleParkReason::IdleRescan => "idle_rescan",
    }
}

fn lifecycle_reject_name(reason: LifecycleReject) -> &'static str {
    match reason {
        LifecycleReject::InvalidScope => "invalid_scope",
        LifecycleReject::MalformedClaim => "malformed_claim",
        LifecycleReject::CrossScopeClaim => "cross_scope_claim",
        LifecycleReject::StaleLease => "stale_lease",
        LifecycleReject::AbandonedLease => "abandoned_lease",
        LifecycleReject::TerminalClaim => "terminal_claim",
        LifecycleReject::OwnershipMismatch => "ownership_mismatch",
        LifecycleReject::FailureCap => "failure_cap",
    }
}

fn option_value(args: &[String], index: &mut usize, option: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{option} requires a value"))
}

fn start(options: Options) -> Result<(), CommandFailure> {
    if options.dry_run {
        let commands = launch_commands(&options).map_err(CommandFailure::diagnostic)?;
        let foreground = foreground_command(&options).map_err(CommandFailure::diagnostic)?;
        if options.json {
            println!(
                "{{\"command\":\"autonomous\",\"subcommand\":\"start\",\"status\":\"dry-run\",\"repo\":\"{}\",\"repo_dir\":\"{}\",\"conductor\":\"{}\",\"companions\":{{\"monitor\":\"{}\",\"supervisor\":\"{}\"}}}}",
                json_escape(&options.repo),
                json_escape(&options.repo_dir),
                json_escape(&foreground.display()),
                json_escape(&commands.monitor.display()),
                json_escape(&commands.supervisor.display())
            );
        } else {
            println!("autospec autonomous start: dry-run");
            println!("conductor: {}", foreground.display());
            println!("monitor: {}", commands.monitor.display());
            println!("supervisor: {}", commands.supervisor.display());
        }
        return Ok(());
    }

    validate_repo_dir(&options).map_err(CommandFailure::diagnostic)?;
    let _config = load_autonomous_config(&options.repo_dir).map_err(CommandFailure::diagnostic)?;
    let layout = RunLayout::new(&options).map_err(CommandFailure::diagnostic)?;
    let options = options_with_resolved_repo(options, &layout);
    let (lifecycle, lease) =
        acquire_lifecycle_start(&layout, &options, LifecycleTransition::Start)?;
    let launched = start_after_lease(&layout, &options, &lifecycle, &lease);
    let (conductor, monitor, supervisor) = match launched {
        Ok(units) => units,
        Err(error) => {
            release_launch_lease(&layout.repo, &lease)?;
            return Err(error);
        }
    };

    if options.json {
        println!(
            "{{\"command\":\"autonomous\",\"subcommand\":\"start\",\"status\":\"started\",\"repo\":\"{}\",\"repo_dir\":\"{}\",\"conductor\":{},\"monitor\":{},\"supervisor\":{}}}",
            json_escape(&options.repo),
            json_escape(&options.repo_dir),
            unit_json(&conductor),
            unit_json(&monitor),
            unit_json(&supervisor)
        );
    } else {
        println!("autospec autonomous started");
        println!("conductor pid: {}", conductor.pid);
        println!("monitor pid: {}", monitor.pid);
        println!("supervisor pid: {}", supervisor.pid);
    }
    Ok(())
}

fn start_after_lease(
    layout: &RunLayout,
    options: &Options,
    lifecycle: &LifecycleDecision,
    lease: &resilience::ConductorLease,
) -> Result<(UnitRecord, UnitRecord, UnitRecord), CommandFailure> {
    let commands = launch_commands(options).map_err(CommandFailure::diagnostic)?;
    let foreground = foreground_command(options).map_err(CommandFailure::diagnostic)?;
    create_launch_directories(layout)?;
    prepare_start_scope(layout, options).map_err(CommandFailure::diagnostic)?;
    persist_lifecycle_decision(layout, lifecycle).map_err(CommandFailure::diagnostic)?;
    write_launch_json(layout, options, &foreground, &commands)
        .map_err(CommandFailure::diagnostic)?;
    launch_units(layout, options, &foreground, &commands, lease)
}

fn restart_after_lease(
    layout: &RunLayout,
    options: &Options,
    lifecycle: &LifecycleDecision,
    lease: &resilience::ConductorLease,
) -> Result<(usize, UnitRecord, UnitRecord, UnitRecord), CommandFailure> {
    let commands = launch_commands(options).map_err(CommandFailure::diagnostic)?;
    let foreground = foreground_command(options).map_err(CommandFailure::diagnostic)?;
    create_launch_directories(layout)?;
    persist_lifecycle_decision(layout, lifecycle).map_err(CommandFailure::diagnostic)?;
    let mut stopped = 0;
    for name in ["supervisor", "monitor", "conductor"] {
        let unit = read_unit(name, &layout.state_dir);
        if unit.running && terminate_pid(&unit.pid) {
            stopped += 1;
        }
    }
    wait_for_scope_stopped(&layout.state_dir);
    clear_stop_flag(layout).map_err(CommandFailure::diagnostic)?;
    write_launch_json(layout, options, &foreground, &commands)
        .map_err(CommandFailure::diagnostic)?;
    let (conductor, monitor, supervisor) =
        launch_units(layout, options, &foreground, &commands, lease)?;
    Ok((stopped, conductor, monitor, supervisor))
}

fn create_launch_directories(layout: &RunLayout) -> Result<(), CommandFailure> {
    fs::create_dir_all(&layout.state_dir).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "cannot create {}: {error}",
            layout.state_dir.display()
        ))
    })?;
    fs::create_dir_all(&layout.log_dir).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "cannot create {}: {error}",
            layout.log_dir.display()
        ))
    })?;
    Ok(())
}

fn monitor(options: Options) -> Result<(), String> {
    let mut iteration = 0;
    loop {
        iteration += 1;
        if options.json {
            println!(
                "{{\"command\":\"autonomous\",\"subcommand\":\"monitor\",\"repo\":\"{}\",\"status\":\"ok\",\"action\":\"none\"}}",
                json_escape(&options.repo)
            );
        } else {
            println!("autospec-monitor: ok repo={} action=none", options.repo);
        }
        if options.once || (options.iterations > 0 && iteration >= options.iterations) {
            break;
        }
        thread::sleep(Duration::from_secs(options.interval_sec));
    }
    Ok(())
}

fn supervise(options: Options) -> Result<(), String> {
    let mut iteration = 0;
    loop {
        iteration += 1;
        let layout = RunLayout::new(&options)?;
        let recorded = read_unit("conductor", &layout.state_dir);
        let watched_pid = if options.pid.is_empty() {
            recorded.pid.as_str()
        } else {
            options.pid.as_str()
        };
        let conductor_running = process_alive(watched_pid);
        let conductor = if conductor_running {
            "running"
        } else {
            "stopped"
        };
        let action = if options.pid.is_empty() && recorded.stale_pid {
            "stale-metadata"
        } else if !watched_pid.is_empty() && !conductor_running {
            "conductor-not-running"
        } else {
            "none"
        };
        if options.json {
            println!(
                "{{\"command\":\"autonomous\",\"subcommand\":\"supervise\",\"repo\":\"{}\",\"conductor\":\"{}\",\"pid\":\"{}\",\"action\":\"{}\"}}",
                json_escape(&options.repo),
                conductor,
                json_escape(watched_pid),
                action
            );
        } else {
            println!(
                "autospec-supervise: ok repo={} conductor={} pid={} action={}",
                options.repo, conductor, watched_pid, action
            );
        }
        if options.once || (options.iterations > 0 && iteration >= options.iterations) {
            break;
        }
        thread::sleep(Duration::from_secs(options.interval_sec));
    }
    Ok(())
}

fn status(options: Options) -> Result<(), CommandFailure> {
    if options.all {
        return list(options).map_err(CommandFailure::diagnostic);
    }
    let layout = RunLayout::new(&options).map_err(CommandFailure::diagnostic)?;
    let conductor = read_unit("conductor", &layout.state_dir);
    let monitor = read_unit("monitor", &layout.state_dir);
    let supervisor = read_unit("supervisor", &layout.state_dir);
    let state = resilience::status(&layout.repo)
        .map_err(resilience_admission_error)?
        .map(|state| StateMetadata {
            status: state.status,
            heartbeat_at: state
                .heartbeat_at
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
            last_cycle: state.last_cycle,
        })
        .unwrap_or_else(|| read_state_metadata(&layout));
    let spend = resilience::spend_status(&layout.repo).map_err(resilience_admission_error)?;
    if options.json {
        println!(
            "{{\"command\":\"autonomous\",\"subcommand\":\"status\",\"repo\":\"{}\",\"status\":\"ok\",\"state_status\":\"{}\",\"heartbeat_at\":{},\"last_cycle\":\"{}\",\"spend\":{{\"tokens\":{},\"issues\":{}}},\"conductor\":{},\"monitor\":{},\"supervisor\":{}}}",
            json_escape(&layout.repo),
            json_escape(&state.status),
            state.heartbeat_at,
            json_escape(&state.last_cycle),
            spend.tokens,
            spend.issues,
            unit_status_json(&conductor),
            unit_status_json(&monitor),
            unit_status_json(&supervisor)
        );
    } else {
        println!("autospec autonomous status: ok");
    }
    Ok(())
}

fn stop(options: Options) -> Result<(), String> {
    let layout = RunLayout::new(&options)?;
    let stop_flag = write_stop_flag(&layout, options.stop_mode)?;
    persist_lifecycle_decision(
        &layout,
        &LifecycleDecision::Stop {
            mode: lifecycle_stop_mode(options.stop_mode),
        },
    )?;
    let mut stopped = 0;
    let units: &[&str] = match options.stop_mode {
        StopMode::Graceful => &["supervisor", "monitor"],
        StopMode::Immediate => &["supervisor", "monitor", "conductor"],
    };
    for name in units {
        let unit = read_unit(name, &layout.state_dir);
        if unit.running && terminate_pid(&unit.pid) {
            stopped += 1;
        }
    }
    if options.json {
        println!(
            "{{\"command\":\"autonomous\",\"subcommand\":\"stop\",\"repo\":\"{}\",\"mode\":\"{}\",\"stop_flag\":\"{}\",\"stopped\":{}}}",
            json_escape(&options.repo),
            options.stop_mode.as_str(),
            json_escape(&stop_flag.display().to_string()),
            stopped
        );
    } else {
        println!(
            "autospec autonomous stop: mode={} stop_flag={} stopped {stopped}",
            options.stop_mode.as_str(),
            stop_flag.display()
        );
    }
    Ok(())
}

fn restart(options: Options) -> Result<(), CommandFailure> {
    validate_repo_dir(&options).map_err(CommandFailure::diagnostic)?;
    let _config = load_autonomous_config(&options.repo_dir).map_err(CommandFailure::diagnostic)?;
    let stop_options = options.clone();
    let layout = RunLayout::new(&options).map_err(CommandFailure::diagnostic)?;
    let options = options_with_resolved_repo(options, &layout);
    let (lifecycle, lease) =
        acquire_lifecycle_start(&layout, &options, LifecycleTransition::Restart)?;
    let launched = restart_after_lease(&layout, &options, &lifecycle, &lease);
    let (stopped, conductor, monitor, supervisor) = match launched {
        Ok(units) => units,
        Err(error) => {
            release_launch_lease(&layout.repo, &lease)?;
            return Err(error);
        }
    };
    if stop_options.json {
        println!(
            "{{\"command\":\"autonomous\",\"subcommand\":\"restart\",\"repo\":\"{}\",\"stopped\":{},\"status\":\"started\",\"conductor\":{},\"monitor\":{},\"supervisor\":{}}}",
            json_escape(&options.repo),
            stopped,
            unit_json(&conductor),
            unit_json(&monitor),
            unit_json(&supervisor)
        );
    } else {
        println!("autospec autonomous restarted");
    }
    Ok(())
}

fn options_with_resolved_repo(mut options: Options, layout: &RunLayout) -> Options {
    options.repo = layout.repo.clone();
    options
}

fn list(options: Options) -> Result<(), String> {
    let state_root = env_path(
        "AUTOSPEC_AUTONOMOUS_OPERATOR_DIR",
        &[".autospec", "autonomous-operator"],
    );
    let mut rows = Vec::new();
    if let Ok(entries) = fs::read_dir(&state_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let scope = entry.file_name().to_string_lossy().to_string();
            let conductor = read_unit("conductor", &path);
            let monitor = read_unit("monitor", &path);
            let supervisor = read_unit("supervisor", &path);
            let launch = read_launch_json(&path);
            let layout = RunLayout {
                state_dir: path.clone(),
                log_dir: PathBuf::new(),
                scope: scope.clone(),
                repo: extract_json_string(&launch, "repo").unwrap_or_else(|| scope.clone()),
            };
            let state = read_state_metadata(&layout);
            rows.push(format!(
                "{{\"scope\":\"{}\",\"slug\":\"{}\",\"repo\":\"{}\",\"alive\":{},\"last_cycle\":\"{}\",\"park_state\":\"{}\",\"launch\":{},\"conductor\":{},\"monitor\":{},\"supervisor\":{}}}",
                json_escape(&scope),
                json_escape(&scope),
                json_escape(&layout.repo),
                conductor.running,
                json_escape(&state.last_cycle),
                json_escape(&state.status),
                launch,
                unit_status_json(&conductor),
                unit_status_json(&monitor),
                unit_status_json(&supervisor)
            ));
        }
    }
    rows.sort();
    if options.json {
        println!(
            "{{\"command\":\"autonomous\",\"subcommand\":\"list\",\"runs\":[{}],\"conductors\":[{}]}}",
            rows.join(","),
            rows.join(",")
        );
    } else {
        println!("autospec autonomous runs");
        for row in rows {
            println!("{row}");
        }
    }
    Ok(())
}

fn logs(options: Options) -> Result<(), String> {
    let layout = RunLayout::new(&options)?;
    let unit = read_unit("conductor", &layout.state_dir);
    let logpath = if unit.logpath.is_empty() {
        newest_legacy_logpath().unwrap_or_default()
    } else {
        unit.logpath.clone()
    };
    let lines = tail_log_lines(&logpath, options.lines)?;
    if options.json {
        println!(
            "{{\"command\":\"autonomous\",\"subcommand\":\"logs\",\"repo\":\"{}\",\"logpath\":\"{}\",\"text\":\"{}\"}}",
            json_escape(&layout.repo),
            json_escape(&logpath),
            json_escape(&lines.join("\n"))
        );
    } else {
        for line in lines {
            println!("{line}");
        }
    }
    Ok(())
}

fn watch(options: Options) -> Result<(), String> {
    if options.once {
        let mut options = options;
        if options.lines == 50 {
            options.lines = usize::MAX;
        }
        return logs(options);
    }
    let layout = RunLayout::new(&options)?;
    let unit = read_unit("conductor", &layout.state_dir);
    let logpath = if unit.logpath.is_empty() {
        newest_legacy_logpath().unwrap_or_default()
    } else {
        unit.logpath.clone()
    };
    follow_log(&logpath, options.interval_sec)
}

fn timeline(options: Options) -> Result<(), String> {
    let layout = RunLayout::new(&options)?;
    let unit = read_unit("conductor", &layout.state_dir);
    let logpath = if unit.logpath.is_empty() {
        newest_legacy_logpath().unwrap_or_default()
    } else {
        unit.logpath.clone()
    };
    let all_lines = timeline_all_lines(&logpath, &layout.repo)?;
    let selected_lines = tail_lines(&all_lines, options.lines);
    let events = timeline_events(&selected_lines);
    let forecast = timeline_forecast(&all_lines);
    let timings = timeline_timings(&all_lines);
    if options.json {
        println!(
            "{{\"command\":\"autonomous\",\"subcommand\":\"timeline\",\"repo\":\"{}\",\"events\":\"{}\"}}",
            json_escape(&layout.repo),
            json_escape(&selected_lines.join("\n"))
        );
    } else {
        println!("autospec autonomous timeline");
        for line in events {
            println!("{line}");
        }
        if let Some(rows) = forecast {
            println!();
            for row in rows {
                println!("{row}");
            }
        }
        if !timings.is_empty() {
            println!();
            for row in timings {
                println!("{row}");
            }
        }
    }
    Ok(())
}

fn cleanup(options: Options) -> Result<(), String> {
    let layout = RunLayout::new(&options)?;
    let mut removed = 0;
    for name in ["conductor", "monitor", "supervisor"] {
        let unit = read_unit(name, &layout.state_dir);
        if unit.running {
            continue;
        }
        for path in [&unit.pid_file, &unit.logpath_file] {
            if path.exists() {
                fs::remove_file(path)
                    .map_err(|error| format!("cannot remove {}: {error}", path.display()))?;
                removed += 1;
            }
        }
    }
    if options.json {
        println!(
            "{{\"command\":\"autonomous\",\"subcommand\":\"cleanup\",\"repo\":\"{}\",\"removed\":{}}}",
            json_escape(&layout.repo),
            removed
        );
    } else {
        println!("autospec autonomous cleanup: removed {removed}");
    }
    Ok(())
}

fn main_health(options: Options) -> Result<(), String> {
    let config = load_autonomous_config(&options.repo_dir)?;
    let layout = RunLayout::new(&options)?;
    let health = load_main_health(&layout, &options, &config)?;
    persist_main_health(&layout, &health)?;
    print_main_health(&layout.repo, &health, options.json);
    if health.outcome == MainlineHealthOutcome::Halt {
        return Err(format!(
            "main-health blocked admission: {}",
            health.diagnostic.as_str()
        ));
    }
    Ok(())
}

fn load_autonomous_config(repo_dir: &str) -> Result<AutonomousConfig, String> {
    let path = repository_config_path(repo_dir);
    match fs::read_to_string(&path) {
        Ok(source) => AutonomousConfig::parse(&source),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(AutonomousConfig::default()),
        Err(error) => Err(format!(
            "cannot read autonomous repository config {}: {error}",
            path.display()
        )),
    }
}

fn repository_config_path(repo_dir: &str) -> PathBuf {
    git_top_level(repo_dir)
        .unwrap_or_else(|| PathBuf::from(repo_dir))
        .join(".autospec/autonomous.yml")
}

fn load_main_health(
    layout: &RunLayout,
    options: &Options,
    config: &AutonomousConfig,
) -> Result<MainlineHealth, String> {
    let explicit_branch = options.health_branch.clone();
    let configured_branch = config.main_health.branch.clone();
    let default_branch =
        if has_non_empty_branch(&explicit_branch) || has_non_empty_branch(&configured_branch) {
            None
        } else {
            gh_default_branch(&layout.repo)?
        };
    let branch = match resolve_health_branch(&HealthBranchInput {
        explicit_branch,
        configured_branch,
        default_branch,
    }) {
        Ok(branch) => branch,
        Err(diagnostic) => {
            return Ok(MainlineHealth::diagnostic(
                "",
                MainlineHealthOutcome::Halt,
                diagnostic,
            ))
        }
    };

    if !gh_branch_exists(&layout.repo, &branch.branch) {
        return Ok(evaluate_health(&branch.branch, false, Vec::new()));
    }

    let status_raw = match gh_api(&format!(
        "repos/{}/commits/{}/status",
        layout.repo, branch.branch
    )) {
        Ok(raw) => raw,
        Err(_) => {
            return Ok(MainlineHealth::diagnostic(
                branch.branch,
                MainlineHealthOutcome::Wait,
                MainlineHealthDiagnostic::GhApiFailed,
            ))
        }
    };
    let (state, has_total_count, legacy) = legacy_status_evidence(&status_raw)?;
    let evidence = if should_read_check_runs(&state, has_total_count, &legacy) {
        match check_run_evidence_for(&layout.repo, &branch.branch) {
            Ok(evidence) => evidence,
            Err(_) => {
                return Ok(MainlineHealth::diagnostic(
                    branch.branch,
                    MainlineHealthOutcome::Wait,
                    MainlineHealthDiagnostic::CheckRunsApiFailed,
                ));
            }
        }
    } else {
        legacy
    };

    Ok(evaluate_health(
        &branch.branch,
        true,
        apply_ignored_checks(evidence, &config.main_health.ignore_checks),
    ))
}

fn has_non_empty_branch(branch: &Option<String>) -> bool {
    branch
        .as_deref()
        .is_some_and(|branch| !branch.trim().is_empty())
}

fn should_read_check_runs(state: &str, has_total_count: bool, legacy: &[CheckEvidence]) -> bool {
    has_total_count && legacy.is_empty() && (state.is_empty() || state == "pending")
}

fn check_run_evidence_for(repo: &str, branch: &str) -> Result<Vec<CheckEvidence>, String> {
    match gh_api(&format!(
        "repos/{repo}/commits/{branch}/check-runs?per_page=100"
    )) {
        Ok(raw) => check_run_evidence(&raw),
        Err(_) => Err(MainlineHealthDiagnostic::CheckRunsApiFailed
            .as_str()
            .to_string()),
    }
}

fn gh_default_branch(repo: &str) -> Result<Option<String>, String> {
    let output = Command::new("gh")
        .args([
            "repo",
            "view",
            repo,
            "--json",
            "defaultBranchRef",
            "--jq",
            ".defaultBranchRef.name",
        ])
        .output()
        .map_err(|error| format!("cannot resolve default branch with gh: {error}"))?;
    if !output.status.success() {
        return Ok(None);
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() {
        Ok(None)
    } else {
        Ok(Some(branch))
    }
}

fn gh_branch_exists(repo: &str, branch: &str) -> bool {
    Command::new("gh")
        .args(["api", &format!("repos/{repo}/branches/{branch}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn gh_api(endpoint: &str) -> Result<String, String> {
    let output = Command::new("gh")
        .args(["api", endpoint])
        .output()
        .map_err(|error| format!("cannot run gh api {endpoint}: {error}"))?;
    if !output.status.success() {
        return Err(format!("gh api {endpoint} exited with {}", output.status));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn persist_main_health(layout: &RunLayout, health: &MainlineHealth) -> Result<(), String> {
    let dir = env_path(
        "AUTOSPEC_AUTONOMOUS_STATE_DIR",
        &[".autospec", "autonomous"],
    )
    .join(&layout.scope);
    fs::create_dir_all(&dir)
        .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    let path = dir.join("main-health-observations.jsonl");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    writeln!(file, "{}", health.to_json(&layout.repo))
        .map_err(|error| format!("cannot write {}: {error}", path.display()))
}

fn print_main_health(repo: &str, health: &MainlineHealth, json: bool) {
    if json {
        println!("{}", health.to_json(repo));
        return;
    }
    println!("DECISION:{}", health.outcome.as_str());
    println!("REASON:{}", health.diagnostic.as_str());
    println!("BRANCH:{}", health.branch);
    for check in &health.evidence {
        println!(
            "CHECK:{}:{}:{}:{}",
            check.name,
            check.status,
            check.conclusion.as_deref().unwrap_or(""),
            if check.required {
                "required"
            } else {
                "advisory"
            }
        );
    }
}

fn run_foreground(options: Options) -> Result<(), CommandFailure> {
    let layout = RunLayout::new(&options).map_err(CommandFailure::diagnostic)?;
    let scope = RepositoryScope::try_from(layout.repo.as_str()).map_err(|reason| {
        CommandFailure::diagnostic(format!("autonomous lifecycle invalid repo: {reason}"))
    })?;
    let inherited_lease = inherited_foreground_lease(&layout.repo)?;
    let config = match load_autonomous_config(&options.repo_dir) {
        Ok(config) => config,
        Err(error) => {
            return match inherited_lease.as_ref() {
                Some(lease) => finish_with_launch_lease(&layout.repo, lease, || {
                    Err(CommandFailure::diagnostic(error))
                }),
                None => Err(CommandFailure::diagnostic(error)),
            };
        }
    };
    let stored_stop = match persisted_stop_mode(&layout) {
        Ok(mode) => mode,
        Err(error) => {
            return match inherited_lease.as_ref() {
                Some(lease) => finish_with_launch_lease(&layout.repo, lease, || {
                    Err(CommandFailure::diagnostic(error))
                }),
                None => Err(CommandFailure::diagnostic(error)),
            };
        }
    };
    if let Some(mode) = stored_stop {
        let lifecycle = decide_lifecycle(
            &LifecycleInput::from_scope(scope)
                .with_transition(LifecycleTransition::Foreground)
                .with_stop(mode),
        );
        let persisted =
            persist_lifecycle_decision(&layout, &lifecycle).map_err(CommandFailure::diagnostic);
        return match inherited_lease.as_ref() {
            Some(lease) => finish_with_launch_lease(&layout.repo, lease, || {
                persisted?;
                emit_lifecycle_decision(lifecycle)
            }),
            None => {
                persisted?;
                emit_lifecycle_decision(lifecycle)
            }
        };
    }

    let (lease, admission) =
        acquire_foreground_lease(&layout, &options, scope.clone(), inherited_lease)?;
    let result = run_foreground_with_lease(&layout, &options, &config, scope, &lease, admission);
    finish_foreground_with_lease(&layout.repo, &lease, result)
}

fn acquire_foreground_lease(
    layout: &RunLayout,
    options: &Options,
    scope: RepositoryScope,
    inherited_lease: Option<resilience::ConductorLease>,
) -> Result<(resilience::ConductorLease, resilience::ResilienceAdmission), CommandFailure> {
    let (lease, admission) = match inherited_lease {
        Some(lease) => {
            let admission = match resilience::admit_owned_lifecycle(
                &layout.repo,
                &lease,
                options.issue,
                options.budget_tokens,
                options.budget_issues,
            ) {
                Ok(admission) => admission,
                Err(error) => {
                    return finish_with_launch_lease(&layout.repo, &lease, || {
                        Err(resilience_lease_error(error))
                    })
                }
            };
            (lease, admission)
        }
        None => match resilience::acquire_lifecycle(
            &layout.repo,
            options.issue,
            options.budget_tokens,
            options.budget_issues,
        ) {
            Ok((admission, lease)) => (lease, admission),
            Err(resilience::LifecycleLeaseError::Policy(admission)) => {
                let decision = lifecycle_decision_with_admission(
                    scope,
                    LifecycleTransition::Foreground,
                    None,
                    &admission,
                )?;
                return Err(lifecycle_non_run(&decision));
            }
            Err(error) => return Err(resilience_lease_error(error)),
        },
    };
    let lifecycle = match lifecycle_decision_with_admission(
        scope,
        LifecycleTransition::Foreground,
        None,
        &admission,
    ) {
        Ok(lifecycle) => lifecycle,
        Err(error) => return finish_with_launch_lease(&layout.repo, &lease, || Err(error)),
    };
    if !matches!(lifecycle, LifecycleDecision::Run { .. }) {
        let persisted =
            persist_lifecycle_decision(layout, &lifecycle).map_err(CommandFailure::diagnostic);
        return finish_with_launch_lease(&layout.repo, &lease, || {
            persisted?;
            Err(lifecycle_non_run(&lifecycle))
        });
    }
    Ok((lease, admission))
}

fn run_foreground_with_lease(
    layout: &RunLayout,
    options: &Options,
    config: &AutonomousConfig,
    scope: RepositoryScope,
    lease: &resilience::ConductorLease,
    admission: resilience::ResilienceAdmission,
) -> Result<ForegroundCompletion, ForegroundFailure> {
    let mut input = foreground_lifecycle_input_with_resilience(
        LifecycleInput::from_scope(scope.clone()).with_transition(LifecycleTransition::Foreground),
        &admission,
    )?;
    let preflight = decide_lifecycle(&input);
    if !matches!(preflight, LifecycleDecision::Run { .. }) {
        persist_lifecycle_decision(layout, &preflight).map_err(CommandFailure::diagnostic)?;
        return Ok(ForegroundCompletion::Lifecycle(preflight));
    }
    let health = load_main_health(layout, options, config).map_err(CommandFailure::diagnostic)?;
    input = input.with_health(lifecycle_health(health.outcome.clone()));
    let mut lifecycle = decide_lifecycle(&input);
    if !matches!(lifecycle, LifecycleDecision::Run { .. }) {
        fs::create_dir_all(&layout.state_dir).map_err(|error| {
            CommandFailure::diagnostic(format!(
                "cannot create {}: {error}",
                layout.state_dir.display()
            ))
        })?;
        persist_main_health(layout, &health).map_err(CommandFailure::diagnostic)?;
        persist_lifecycle_decision(layout, &lifecycle).map_err(CommandFailure::diagnostic)?;
        return Ok(ForegroundCompletion::Lifecycle(lifecycle));
    }
    let initial_plan = queue::ready_plan_for(&layout.repo, 1).map_err(command_error);
    if options.issue.is_none() {
        if let Some(issue) = initial_plan
            .as_ref()
            .ok()
            .and_then(|plan| plan.batch.first().map(|issue| issue.issue.number))
        {
            let admission =
                resilience_admission_for_issue(layout, options, Some(issue), Some(lease))?;
            input = foreground_lifecycle_input_with_resilience(
                LifecycleInput::from_scope(scope)
                    .with_transition(LifecycleTransition::Foreground)
                    .with_health(lifecycle_health(health.outcome.clone())),
                &admission,
            )?;
            lifecycle = decide_lifecycle(&input);
            if !matches!(lifecycle, LifecycleDecision::Run { .. }) {
                persist_lifecycle_decision(layout, &lifecycle)
                    .map_err(CommandFailure::diagnostic)?;
                return Ok(ForegroundCompletion::Lifecycle(lifecycle));
            }
        }
    }
    let scope = foreground_scope();
    let state_path = foreground_state_path(layout, scope);
    let state =
        load_foreground_state(&state_path, layout, scope).map_err(CommandFailure::diagnostic)?;
    if foreground_state_is_retained(&state) {
        return Ok(ForegroundCompletion::State(state));
    }
    if state.phase() != ConductorPhase::Scan {
        return Err(CommandFailure::diagnostic(format!(
            "foreground conductor requires recovery from {:?}",
            state.phase()
        ))
        .into());
    }

    let (state, found_work) =
        scan_foreground(layout, lease, state, initial_plan).map_err(CommandFailure::diagnostic)?;
    if !found_work {
        persist_foreground_admission(layout, &health, &lifecycle)?;
        persist_foreground_state(&state_path, &state).map_err(CommandFailure::diagnostic)?;
        return Ok(ForegroundCompletion::State(state));
    }
    let state = review_foreground(layout, state).map_err(CommandFailure::diagnostic)?;
    let selection = select_foreground(layout, options, lease, state)?;
    let (state, selected) = match selection {
        ForegroundSelectionResult::Lifecycle(lifecycle) => {
            return Ok(ForegroundCompletion::Lifecycle(lifecycle));
        }
        ForegroundSelectionResult::State(state, selected) => (state, selected),
    };
    let Some(selected) = selected else {
        persist_foreground_admission(layout, &health, &lifecycle)?;
        persist_foreground_state(&state_path, &state).map_err(CommandFailure::diagnostic)?;
        return Ok(ForegroundCompletion::State(state));
    };
    persist_foreground_admission(layout, &health, &lifecycle)?;
    persist_foreground_state(&state_path, &state).map_err(CommandFailure::diagnostic)?;
    match dispatch_foreground(layout, options, lease, &state_path, state, selected)? {
        ForegroundDispatchResult::State(state) => Ok(ForegroundCompletion::State(state)),
        ForegroundDispatchResult::Lifecycle(lifecycle) => {
            Ok(ForegroundCompletion::Lifecycle(lifecycle))
        }
    }
}

fn lifecycle_health(outcome: MainlineHealthOutcome) -> LifecycleHealth {
    match outcome {
        MainlineHealthOutcome::Continue => LifecycleHealth::Continue,
        MainlineHealthOutcome::Wait => LifecycleHealth::Wait,
        MainlineHealthOutcome::Halt => LifecycleHealth::Halt,
    }
}

fn foreground_state_is_retained(state: &ConductorState) -> bool {
    matches!(
        state.phase(),
        ConductorPhase::Paused | ConductorPhase::SliceComplete | ConductorPhase::AllDone
    )
}

#[derive(Debug, Clone)]
struct ForegroundSelection {
    issue: u64,
    serialization_reasons: Vec<String>,
}

enum ForegroundCompletion {
    State(ConductorState),
    Lifecycle(LifecycleDecision),
}

enum ForegroundFailure {
    Diagnostic(CommandFailure),
    Deferred { json: String, exit_code: i32 },
}

impl From<CommandFailure> for ForegroundFailure {
    fn from(error: CommandFailure) -> Self {
        Self::Diagnostic(error)
    }
}

impl From<claim::ConductorClaimError> for ForegroundFailure {
    fn from(error: claim::ConductorClaimError) -> Self {
        match error {
            claim::ConductorClaimError::Diagnostic(error) => Self::Diagnostic(error),
            claim::ConductorClaimError::Deferred { json, exit_code } => {
                Self::Deferred { json, exit_code }
            }
        }
    }
}

enum ForegroundSelectionResult {
    State(ConductorState, Option<ForegroundSelection>),
    Lifecycle(LifecycleDecision),
}

enum ForegroundDispatchResult {
    State(ConductorState),
    Lifecycle(LifecycleDecision),
}

fn scan_foreground(
    layout: &RunLayout,
    lease: &resilience::ConductorLease,
    state: ConductorState,
    initial_plan: Result<autospec_core::coordination::ReadyQueuePlan, String>,
) -> Result<(ConductorState, bool), String> {
    let initial_plan = match initial_plan {
        Ok(plan) => plan,
        Err(reason) => {
            return match state.scope() {
                ConductorScope::Repository => match waterfall_coordinator::record_tier_one(
                    &layout.state_dir,
                    &layout.repo,
                    lease,
                    waterfall_coordinator::Tier1QueueEvidence::Failed(&reason),
                )? {
                    waterfall_coordinator::Tier1Progress::Failed(reason) => Err(reason),
                    waterfall_coordinator::Tier1Progress::Pending
                    | waterfall_coordinator::Tier1Progress::Advanced => Ok((state, false)),
                },
                ConductorScope::Slice => Err(reason),
            };
        }
    };
    if initial_plan.gate_counts.candidate == 0 {
        if state.scope() == ConductorScope::Repository {
            match waterfall_coordinator::record_tier_one(
                &layout.state_dir,
                &layout.repo,
                lease,
                waterfall_coordinator::Tier1QueueEvidence::EmptyPage(&initial_plan),
            )? {
                waterfall_coordinator::Tier1Progress::Advanced
                | waterfall_coordinator::Tier1Progress::Pending => return Ok((state, false)),
                waterfall_coordinator::Tier1Progress::Failed(reason) => return Err(reason),
            }
        }
        let state = state
            .transition(ConductorEvent::ScanEmpty)
            .map_err(|error| format!("cannot record empty foreground scan: {error}"))?;
        return Ok((state, false));
    }
    let state = state
        .transition(ConductorEvent::ScanFoundWork)
        .map_err(|error| format!("cannot record foreground scan: {error}"))?;
    Ok((state, true))
}

fn review_foreground(layout: &RunLayout, state: ConductorState) -> Result<ConductorState, String> {
    queue::review_safety_for_repo(&layout.repo, 1, None).map_err(command_error)?;
    let state = state
        .transition(ConductorEvent::SafetyReviewed)
        .map_err(|error| format!("cannot record foreground safety review: {error}"))?;
    Ok(state)
}

fn select_foreground(
    layout: &RunLayout,
    options: &Options,
    lease: &resilience::ConductorLease,
    state: ConductorState,
) -> Result<ForegroundSelectionResult, ForegroundFailure> {
    let ready = queue::ready_plan_for(&layout.repo, 1)
        .map_err(command_error)
        .map_err(CommandFailure::diagnostic)?;
    let Some(selected) = ready.batch.first() else {
        let state = state
            .transition(ConductorEvent::Pause {
                reason: "no_ready_issue_after_review".to_string(),
            })
            .map_err(|error| {
                CommandFailure::diagnostic(format!("cannot pause foreground conductor: {error}"))
            })?;
        return Ok(ForegroundSelectionResult::State(state, None));
    };
    let selection = ForegroundSelection {
        issue: selected.issue.number,
        serialization_reasons: selected.serialization_reasons.clone(),
    };
    let admission =
        resilience_admission_for_issue(layout, options, Some(selection.issue), Some(lease))?;
    let scope = RepositoryScope::try_from(layout.repo.as_str()).map_err(|reason| {
        CommandFailure::diagnostic(format!("autonomous lifecycle invalid repo: {reason}"))
    })?;
    let input = foreground_lifecycle_input_with_resilience(
        LifecycleInput::from_scope(scope).with_transition(LifecycleTransition::Foreground),
        &admission,
    )?;
    let lifecycle = decide_lifecycle(&input);
    if !matches!(lifecycle, LifecycleDecision::Run { .. }) {
        persist_lifecycle_decision(layout, &lifecycle).map_err(CommandFailure::diagnostic)?;
        return Ok(ForegroundSelectionResult::Lifecycle(lifecycle));
    }
    let state = state
        .transition(ConductorEvent::Selected {
            issue: selection.issue,
            serialization_reasons: selection.serialization_reasons.clone(),
        })
        .map_err(|error| {
            CommandFailure::diagnostic(format!("cannot select foreground issue: {error}"))
        })?;
    Ok(ForegroundSelectionResult::State(state, Some(selection)))
}

fn dispatch_foreground(
    layout: &RunLayout,
    options: &Options,
    lease: &resilience::ConductorLease,
    state_path: &Path,
    state: ConductorState,
    selection: ForegroundSelection,
) -> Result<ForegroundDispatchResult, ForegroundFailure> {
    let admission =
        resilience_admission_for_issue(layout, options, Some(selection.issue), Some(lease))?;
    let branch = format!("autonomous/issue-{}", selection.issue);
    let worker_id = foreground_worker_id().map_err(CommandFailure::diagnostic)?;
    let claim_evidence =
        claim::lifecycle_claim_evidence(&layout.repo, selection.issue, &worker_id, &branch)?;
    let scope = RepositoryScope::try_from(layout.repo.as_str()).map_err(|reason| {
        CommandFailure::diagnostic(format!("autonomous lifecycle invalid repo: {reason}"))
    })?;
    let worker = WorkerId::try_from(worker_id.as_str()).map_err(|reason| {
        CommandFailure::diagnostic(format!("autonomous lifecycle invalid worker: {reason}"))
    })?;
    let claim_branch = ClaimBranch::try_from(branch.as_str()).map_err(|reason| {
        CommandFailure::diagnostic(format!("autonomous lifecycle invalid branch: {reason}"))
    })?;
    let input = foreground_lifecycle_input_with_resilience(
        LifecycleInput::from_scope(scope)
            .with_transition(LifecycleTransition::Foreground)
            .with_claim_evidence(claim_evidence)
            .with_expected_claim(worker, claim_branch),
        &admission,
    )?;
    let lifecycle = decide_lifecycle(&input);
    if !matches!(lifecycle, LifecycleDecision::Run { .. }) {
        persist_lifecycle_decision(layout, &lifecycle).map_err(CommandFailure::diagnostic)?;
        return Ok(ForegroundDispatchResult::Lifecycle(lifecycle));
    }
    persist_lifecycle_decision(layout, &lifecycle).map_err(CommandFailure::diagnostic)?;
    let lease = claim::acquire_for_conductor(&layout.repo, selection.issue, &worker_id, &branch)?;
    let state = state.transition(ConductorEvent::Claimed).map_err(|error| {
        CommandFailure::diagnostic(format!("cannot record foreground claim: {error}"))
    })?;
    persist_foreground_state(state_path, &state).map_err(CommandFailure::diagnostic)?;

    let receipt = ExecutorRequest::for_selected(layout, &options.repo_dir, selection.issue)
        .map_or_else(|_| ExecutorReceipt::failed(), |request| request.run());
    let state = state
        .transition(ConductorEvent::DispatchRecorded {
            outcome: receipt.outcome.clone(),
        })
        .map_err(|error| {
            CommandFailure::diagnostic(format!("cannot record executor receipt: {error}"))
        })?;
    persist_foreground_state(state_path, &state).map_err(CommandFailure::diagnostic)?;
    claim::record_executor_outcome(
        &lease.repo,
        lease.issue,
        &lease.worker_id,
        &lease.branch,
        &receipt.claim_step,
    )?;
    claim::reconcile_active_issue(&lease.repo, lease.issue)?;
    Ok(ForegroundDispatchResult::State(state))
}

fn foreground_worker_id() -> Result<String, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("cannot generate foreground worker id: {error}"))?
        .as_nanos();
    Ok(format!(
        "{FOREGROUND_WORKER_PREFIX}-{}-{nanos}",
        std::process::id()
    ))
}

fn executor_result(args: &[String]) -> Result<(), CommandFailure> {
    let input = match parse_executor_result_input(args) {
        Ok(ExecutorResultInvocation::Deferred { repo, issue }) => {
            println!("{}", executor_receipt_json(&repo, issue));
            return Ok(());
        }
        Ok(ExecutorResultInvocation::Explicit(input)) => input,
        Err(reason) => return emit_executor_protocol("malformed", None, None, Some(&reason), 2),
    };

    let recorded = match claim::record_executor_result(
        &input.repo,
        input.issue,
        &input.worker_id,
        &input.branch,
        &input.outcome,
        input.pr,
    ) {
        Ok(recorded) => recorded,
        Err(_) => {
            return emit_executor_protocol(
                "blocked",
                Some(&input),
                None,
                Some("result_recording_failed"),
                20,
            );
        }
    };

    match recorded {
        claim::ExecutorResultRecord::Recorded => match &input.outcome {
            ConductorOutcome::Succeeded => {
                emit_executor_protocol("accepted", Some(&input), input.pr, None, 0)
            }
            ConductorOutcome::Blocked(_) => {
                emit_executor_protocol("blocked", Some(&input), None, input.reason.as_deref(), 20)
            }
            ConductorOutcome::Retryable(_) => {
                emit_executor_protocol("retryable", Some(&input), None, input.reason.as_deref(), 10)
            }
        },
        claim::ExecutorResultRecord::EvidenceUnavailable => emit_executor_protocol(
            "blocked",
            Some(&input),
            None,
            Some("success_evidence_unavailable"),
            20,
        ),
        claim::ExecutorResultRecord::OwnershipLost => emit_executor_protocol(
            "ownership_lost",
            Some(&input),
            None,
            Some("claim_ownership_lost"),
            3,
        ),
    }
}

#[derive(Debug, Clone)]
struct ExecutorResultInput {
    repo: String,
    issue: u64,
    worker_id: String,
    branch: String,
    outcome: ConductorOutcome,
    pr: Option<u64>,
    reason: Option<String>,
}

impl ExecutorResultInput {
    fn outcome_name(&self) -> &'static str {
        match self.outcome {
            ConductorOutcome::Succeeded => "succeeded",
            ConductorOutcome::Blocked(_) => "blocked",
            ConductorOutcome::Retryable(_) => "retryable",
        }
    }
}

enum ExecutorResultInvocation {
    Deferred { repo: String, issue: u64 },
    Explicit(ExecutorResultInput),
}

fn parse_executor_result_input(args: &[String]) -> Result<ExecutorResultInvocation, String> {
    let mut repo = None;
    let mut issue = None;
    let mut worker_id = None;
    let mut branch = None;
    let mut outcome = None;
    let mut pr = None;
    let mut reason = None;
    let mut index = 1;

    while index < args.len() {
        let flag = &args[index];
        let value = executor_result_option_value(args, &mut index, flag)?;
        match flag.as_str() {
            "--repo" => set_executor_result_value(&mut repo, value, "--repo")?,
            "--issue" => set_executor_result_number(&mut issue, value, "--issue")?,
            "--worker-id" => set_executor_result_value(&mut worker_id, value, "--worker-id")?,
            "--branch" => set_executor_result_value(&mut branch, value, "--branch")?,
            "--outcome" => set_executor_result_value(&mut outcome, value, "--outcome")?,
            "--pr" => set_executor_result_number(&mut pr, value, "--pr")?,
            "--reason" => set_executor_result_value(&mut reason, value, "--reason")?,
            unknown => return Err(format!("unknown executor-result option: {unknown}")),
        }
        index += 1;
    }

    let has_explicit_field = worker_id.is_some()
        || branch.is_some()
        || outcome.is_some()
        || pr.is_some()
        || reason.is_some();
    if !has_explicit_field {
        return match (repo, issue) {
            (Some(repo), Some(issue)) if repo != "unknown" => {
                Ok(ExecutorResultInvocation::Deferred { repo, issue })
            }
            _ => Err("executor-result deferred receipt requires --repo and --issue".to_string()),
        };
    }

    let repo = repo.ok_or_else(|| "executor-result requires --repo".to_string())?;
    if repo == "unknown" {
        return Err("executor-result requires --repo".to_string());
    }
    let issue = issue.ok_or_else(|| "executor-result requires --issue".to_string())?;
    let worker_id = worker_id.ok_or_else(|| "executor-result requires --worker-id".to_string())?;
    let branch = branch.ok_or_else(|| "executor-result requires --branch".to_string())?;
    let outcome = outcome.ok_or_else(|| "executor-result requires --outcome".to_string())?;
    let (outcome, pr, reason) = match outcome.as_str() {
        "succeeded" if pr.is_some() && reason.is_none() => (ConductorOutcome::Succeeded, pr, None),
        "blocked" if pr.is_none() => {
            let reason =
                reason.ok_or_else(|| "blocked executor-result requires --reason".to_string())?;
            (
                ConductorOutcome::Blocked(reason.clone()),
                None,
                Some(reason),
            )
        }
        "retryable" if pr.is_none() => {
            let reason =
                reason.ok_or_else(|| "retryable executor-result requires --reason".to_string())?;
            (
                ConductorOutcome::Retryable(reason.clone()),
                None,
                Some(reason),
            )
        }
        "succeeded" => {
            return Err("succeeded executor-result requires --pr and rejects --reason".to_string())
        }
        "blocked" | "retryable" => {
            return Err(
                "blocked and retryable executor-results require --reason and reject --pr"
                    .to_string(),
            )
        }
        _ => {
            return Err(
                "executor-result --outcome must be succeeded, blocked, or retryable".to_string(),
            )
        }
    };

    Ok(ExecutorResultInvocation::Explicit(ExecutorResultInput {
        repo,
        issue,
        worker_id,
        branch,
        outcome,
        pr,
        reason,
    }))
}

fn executor_result_option_value(
    args: &[String],
    index: &mut usize,
    flag: &str,
) -> Result<String, String> {
    *index += 1;
    let value = args
        .get(*index)
        .ok_or_else(|| format!("{flag} requires a value"))?;
    if value.trim().is_empty() || value.starts_with("--") {
        return Err(format!("{flag} requires a non-empty value"));
    }
    Ok(value.clone())
}

fn set_executor_result_value(
    field: &mut Option<String>,
    value: String,
    flag: &str,
) -> Result<(), String> {
    if field.replace(value).is_some() {
        return Err(format!("{flag} may be specified only once"));
    }
    Ok(())
}

fn set_executor_result_number(
    field: &mut Option<u64>,
    value: String,
    flag: &str,
) -> Result<(), String> {
    let value = value
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("{flag} must be a positive integer"))?;
    if field.replace(value).is_some() {
        return Err(format!("{flag} may be specified only once"));
    }
    Ok(())
}

fn emit_executor_protocol(
    status: &str,
    input: Option<&ExecutorResultInput>,
    pr: Option<u64>,
    reason: Option<&str>,
    exit_code: i32,
) -> Result<(), CommandFailure> {
    let mut fields = vec![format!("\"status\":\"{}\"", json_escape(status))];
    if let Some(input) = input {
        fields.push(format!("\"repo\":\"{}\"", json_escape(&input.repo)));
        fields.push(format!("\"issue\":{}", input.issue));
        fields.push(format!("\"outcome\":\"{}\"", input.outcome_name()));
    }
    if let Some(pr) = pr {
        fields.push(format!("\"pr\":{pr}"));
    }
    if let Some(reason) = reason {
        fields.push(format!("\"reason\":\"{}\"", json_escape(reason)));
    }
    println!("{{{}}}", fields.join(","));
    if exit_code == 0 {
        Ok(())
    } else {
        Err(CommandFailure::status(String::new(), exit_code))
    }
}

#[derive(Debug, Clone)]
struct ExecutorRequest {
    program: PathBuf,
    args: Vec<String>,
    current_dir: PathBuf,
    repo: String,
    issue: u64,
}

#[derive(Debug, Clone)]
struct ExecutorReceipt {
    outcome: ConductorOutcome,
    claim_step: String,
}

impl ExecutorRequest {
    fn for_selected(layout: &RunLayout, repo_dir: &str, issue: u64) -> Result<Self, String> {
        Ok(Self {
            program: std::env::current_exe()
                .map_err(|error| format!("cannot resolve Rust executor program: {error}"))?,
            args: vec![
                "autonomous".to_string(),
                "executor-result".to_string(),
                "--repo".to_string(),
                layout.repo.clone(),
                "--issue".to_string(),
                issue.to_string(),
            ],
            current_dir: PathBuf::from(repo_dir),
            repo: layout.repo.clone(),
            issue,
        })
    }

    fn run(&self) -> ExecutorReceipt {
        let output = Command::new(&self.program)
            .args(&self.args)
            .current_dir(&self.current_dir)
            .output();
        let expected = executor_receipt_json(&self.repo, self.issue);
        if output.is_ok_and(|output| {
            output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == expected
        }) {
            ExecutorReceipt {
                outcome: ConductorOutcome::Blocked(DEFERRED_EXECUTOR_REASON.to_string()),
                claim_step: "executor_deferred".to_string(),
            }
        } else {
            ExecutorReceipt::failed()
        }
    }
}

impl ExecutorReceipt {
    fn failed() -> Self {
        Self {
            outcome: ConductorOutcome::Blocked("executor_receipt_failed".to_string()),
            claim_step: "executor_receipt_failed".to_string(),
        }
    }
}

fn executor_receipt_json(repo: &str, issue: u64) -> String {
    format!(
        "{{\"repo\":\"{}\",\"issue\":{},\"outcome\":\"blocked\",\"reason\":\"{}\"}}",
        json_escape(repo),
        issue,
        DEFERRED_EXECUTOR_REASON,
    )
}

fn foreground_state_path(layout: &RunLayout, scope: ConductorScope) -> PathBuf {
    layout.state_dir.join(format!(
        "foreground-conductor-{}.json",
        foreground_scope_key(scope)
    ))
}

fn foreground_scope_key(scope: ConductorScope) -> String {
    match scope {
        ConductorScope::Repository => "repository".to_string(),
        ConductorScope::Slice => {
            let selected = std::env::var("AUTOSPEC_RUN_ONLY_ISSUES").unwrap_or_default();
            format!(
                "slice-{}",
                selected
                    .bytes()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            )
        }
    }
}

fn load_foreground_state(
    path: &Path,
    layout: &RunLayout,
    scope: ConductorScope,
) -> Result<ConductorState, String> {
    match fs::read_to_string(path) {
        Ok(source) => {
            let state = ConductorState::parse_json(&source)
                .map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
            if state.repo() != layout.repo || state.scope() != scope {
                return Err(format!(
                    "foreground state {} does not match repository and scope",
                    path.display()
                ));
            }
            Ok(state)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            ConductorState::new(layout.repo.clone(), scope, 3)
        }
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

fn persist_foreground_state(path: &Path, state: &ConductorState) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("cannot resolve parent for {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    atomic_write(path, &format!("{}\n", state.to_json()))
}

fn acquire_lifecycle_start(
    layout: &RunLayout,
    options: &Options,
    transition: LifecycleTransition,
) -> Result<(LifecycleDecision, resilience::ConductorLease), CommandFailure> {
    let scope = RepositoryScope::try_from(layout.repo.as_str()).map_err(|reason| {
        CommandFailure::diagnostic(format!("autonomous lifecycle invalid repo: {reason}"))
    })?;
    let stored_stop = persisted_stop_mode(layout).map_err(CommandFailure::diagnostic)?;
    if matches!(transition, LifecycleTransition::Start) {
        if let Some(mode) = stored_stop {
            let decision = decide_lifecycle(
                &LifecycleInput::from_scope(scope)
                    .with_transition(transition)
                    .with_stop(mode),
            );
            return Err(lifecycle_non_run(&decision));
        }
    }
    let (admission, lease) = match resilience::acquire_lifecycle(
        &layout.repo,
        options.issue,
        options.budget_tokens,
        options.budget_issues,
    ) {
        Ok(acquired) => acquired,
        Err(resilience::LifecycleLeaseError::Policy(admission)) => {
            let decision =
                lifecycle_decision_with_admission(scope, transition, stored_stop, &admission)?;
            return Err(lifecycle_non_run(&decision));
        }
        Err(error) => return Err(resilience_lease_error(error)),
    };
    let decision = lifecycle_decision_with_admission(scope, transition, stored_stop, &admission)?;
    if !matches!(decision, LifecycleDecision::Run { .. }) {
        release_launch_lease(&layout.repo, &lease)?;
        return Err(lifecycle_non_run(&decision));
    }
    Ok((decision, lease))
}

fn lifecycle_decision_with_admission(
    scope: RepositoryScope,
    transition: LifecycleTransition,
    stored_stop: Option<LifecycleStopMode>,
    admission: &resilience::ResilienceAdmission,
) -> Result<LifecycleDecision, CommandFailure> {
    let mut input = lifecycle_input_with_resilience(
        LifecycleInput::from_scope(scope).with_transition(transition),
        admission,
    )?;
    if !matches!(transition, LifecycleTransition::Start) {
        if let Some(mode) = stored_stop {
            input = input.with_stop(mode);
        }
    }
    let decision = decide_lifecycle(&input);
    Ok(decision)
}

fn persist_foreground_admission(
    layout: &RunLayout,
    health: &MainlineHealth,
    lifecycle: &LifecycleDecision,
) -> Result<(), CommandFailure> {
    fs::create_dir_all(&layout.state_dir).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "cannot create {}: {error}",
            layout.state_dir.display()
        ))
    })?;
    persist_main_health(layout, health).map_err(CommandFailure::diagnostic)?;
    persist_lifecycle_decision(layout, lifecycle).map_err(CommandFailure::diagnostic)
}

fn resilience_admission_for_issue(
    layout: &RunLayout,
    options: &Options,
    issue: Option<u64>,
    lease: Option<&resilience::ConductorLease>,
) -> Result<resilience::ResilienceAdmission, ForegroundFailure> {
    match lease {
        Some(lease) => resilience::admit_owned_lifecycle(
            &layout.repo,
            lease,
            issue,
            options.budget_tokens,
            options.budget_issues,
        )
        .map_err(foreground_resilience_lease_error),
        None => resilience::admit_lifecycle(
            &layout.repo,
            issue,
            options.budget_tokens,
            options.budget_issues,
        )
        .map_err(foreground_resilience_admission_error),
    }
}

fn foreground_lifecycle_input_with_resilience(
    input: LifecycleInput,
    admission: &resilience::ResilienceAdmission,
) -> Result<LifecycleInput, ForegroundFailure> {
    if matches!(admission.lease, Some(ConductorLeaseDecision::Held)) {
        return Err(ForegroundFailure::Deferred {
            json: resilience_lease_held_json(),
            exit_code: 20,
        });
    }
    Ok(lifecycle_input_from_admission(input, admission))
}

fn lifecycle_input_with_resilience(
    input: LifecycleInput,
    admission: &resilience::ResilienceAdmission,
) -> Result<LifecycleInput, CommandFailure> {
    if matches!(admission.lease, Some(ConductorLeaseDecision::Held)) {
        return Err(resilience_lease_held());
    }
    Ok(lifecycle_input_from_admission(input, admission))
}

fn lifecycle_input_from_admission(
    input: LifecycleInput,
    admission: &resilience::ResilienceAdmission,
) -> LifecycleInput {
    let budget = match admission.capacity {
        CapacityDecision::WithinCap => LifecycleBudget::WithinCap,
        CapacityDecision::UsageCap | CapacityDecision::IssueCap => LifecycleBudget::HardCap,
    };
    input
        .with_failure_count(admission.failure_count)
        .with_budget(budget)
}

fn resilience_admission_error(error: resilience::LifecycleAdmissionError) -> CommandFailure {
    match error {
        resilience::LifecycleAdmissionError::Reject(reason) => resilience_reject(reason),
        resilience::LifecycleAdmissionError::Diagnostic(message) => {
            CommandFailure::diagnostic(message)
        }
    }
}

fn foreground_resilience_admission_error(
    error: resilience::LifecycleAdmissionError,
) -> ForegroundFailure {
    match error {
        resilience::LifecycleAdmissionError::Reject(reason) => ForegroundFailure::Deferred {
            json: resilience_reject_json(reason),
            exit_code: 3,
        },
        resilience::LifecycleAdmissionError::Diagnostic(message) => {
            ForegroundFailure::Diagnostic(CommandFailure::diagnostic(message))
        }
    }
}

fn resilience_lease_error(error: resilience::LifecycleLeaseError) -> CommandFailure {
    match error {
        resilience::LifecycleLeaseError::Reject(reason) => resilience_reject(reason),
        resilience::LifecycleLeaseError::Diagnostic(message) => CommandFailure::diagnostic(message),
        resilience::LifecycleLeaseError::Held => resilience_lease_held(),
        resilience::LifecycleLeaseError::TokenMismatch => resilience_token_mismatch(),
        resilience::LifecycleLeaseError::Policy(_) => CommandFailure::diagnostic(
            "resilience lease policy must be resolved before lifecycle ownership".to_string(),
        ),
    }
}

fn foreground_resilience_lease_error(error: resilience::LifecycleLeaseError) -> ForegroundFailure {
    match error {
        resilience::LifecycleLeaseError::Reject(reason) => ForegroundFailure::Deferred {
            json: resilience_reject_json(reason),
            exit_code: 3,
        },
        resilience::LifecycleLeaseError::Diagnostic(message) => {
            ForegroundFailure::Diagnostic(CommandFailure::diagnostic(message))
        }
        resilience::LifecycleLeaseError::Held => ForegroundFailure::Deferred {
            json: resilience_lease_held_json(),
            exit_code: 20,
        },
        resilience::LifecycleLeaseError::TokenMismatch => ForegroundFailure::Deferred {
            json: resilience_token_mismatch_json(),
            exit_code: 3,
        },
        resilience::LifecycleLeaseError::Policy(_) => {
            ForegroundFailure::Diagnostic(CommandFailure::diagnostic(
                "resilience lease policy must be resolved before lifecycle ownership".to_string(),
            ))
        }
    }
}

fn resilience_reject(reason: &str) -> CommandFailure {
    println!("{}", resilience_reject_json(reason));
    CommandFailure::status(String::new(), 3)
}

fn resilience_lease_held() -> CommandFailure {
    println!("{}", resilience_lease_held_json());
    CommandFailure::status(String::new(), 20)
}

fn resilience_token_mismatch() -> CommandFailure {
    println!("{}", resilience_token_mismatch_json());
    CommandFailure::status(String::new(), 3)
}

fn resilience_reject_json(reason: &str) -> String {
    format!("{{\"decision\":\"reject\",\"reason\":\"{reason}\"}}")
}

fn resilience_lease_held_json() -> String {
    "{\"decision\":\"park\",\"reason\":\"conductor_lease_held\"}".to_string()
}

fn resilience_token_mismatch_json() -> String {
    "{\"decision\":\"reject\",\"reason\":\"conductor_lease_token_mismatch\"}".to_string()
}

fn release_launch_lease(
    repo: &str,
    lease: &resilience::ConductorLease,
) -> Result<(), CommandFailure> {
    resilience::release_lifecycle(repo, lease).map_err(release_lease_error)
}

fn release_lease_error(error: resilience::LifecycleLeaseError) -> CommandFailure {
    let reason = match error {
        resilience::LifecycleLeaseError::Reject(reason) => {
            format!("release rejected: {reason}")
        }
        resilience::LifecycleLeaseError::Diagnostic(message) => message,
        resilience::LifecycleLeaseError::Held => "lease transaction is held".to_string(),
        resilience::LifecycleLeaseError::TokenMismatch => "lease token mismatch".to_string(),
        resilience::LifecycleLeaseError::Policy(_) => {
            "release cannot resolve lifecycle policy".to_string()
        }
    };
    CommandFailure::diagnostic(format!("cannot release conductor lease: {reason}"))
}

fn inherited_foreground_lease(
    repo: &str,
) -> Result<Option<resilience::ConductorLease>, CommandFailure> {
    match std::env::var("AUTOSPEC_CONDUCTOR_LEASE_TOKEN") {
        Ok(token) => resilience::adopt_lifecycle(repo, &token)
            .map(Some)
            .map_err(resilience_lease_error),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(resilience_token_mismatch()),
    }
}

fn finish_with_launch_lease<T>(
    repo: &str,
    lease: &resilience::ConductorLease,
    result: impl FnOnce() -> Result<T, CommandFailure>,
) -> Result<T, CommandFailure> {
    match release_launch_lease(repo, lease) {
        Ok(()) => result(),
        Err(error) => Err(error),
    }
}

fn finish_foreground_with_lease(
    repo: &str,
    lease: &resilience::ConductorLease,
    result: Result<ForegroundCompletion, ForegroundFailure>,
) -> Result<(), CommandFailure> {
    release_launch_lease(repo, lease)?;
    match result {
        Ok(ForegroundCompletion::State(state)) => {
            println!("{}", state.to_json());
            Ok(())
        }
        Ok(ForegroundCompletion::Lifecycle(lifecycle)) => emit_lifecycle_decision(lifecycle),
        Err(ForegroundFailure::Diagnostic(error)) => Err(error),
        Err(ForegroundFailure::Deferred { json, exit_code }) => {
            println!("{json}");
            Err(CommandFailure::status(String::new(), exit_code))
        }
    }
}

fn persisted_stop_mode(layout: &RunLayout) -> Result<Option<LifecycleStopMode>, String> {
    if let Some(mode) = read_stop_mode(layout)? {
        return Ok(Some(mode));
    }
    let Some(record) = read_lifecycle_record(layout)? else {
        return Ok(None);
    };
    if record.scope.as_str() != layout.repo {
        return Err("persisted lifecycle record belongs to another repository".to_string());
    }
    Ok(match record.decision {
        LifecycleDecision::Stop { mode } => Some(mode),
        _ => None,
    })
}

fn read_lifecycle_record(layout: &RunLayout) -> Result<Option<LifecycleRecord>, String> {
    let path = layout.state_dir.join("lifecycle.json");
    match fs::read_to_string(&path) {
        Ok(raw) => LifecycleRecord::parse_json(&raw)
            .map(Some)
            .map_err(|error| format!("cannot parse {}: {error}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

fn persist_lifecycle_decision(
    layout: &RunLayout,
    decision: &LifecycleDecision,
) -> Result<(), String> {
    fs::create_dir_all(&layout.state_dir)
        .map_err(|error| format!("cannot create {}: {error}", layout.state_dir.display()))?;
    let path = layout.state_dir.join("lifecycle.json");
    let body = format!(
        "{{\"version\":1,\"repo\":\"{}\",\"result\":{}}}\n",
        json_escape(&layout.repo),
        lifecycle_decision_json(decision)
    );
    atomic_write(&path, &body)
}

fn atomic_temporary_path(path: &Path) -> PathBuf {
    let sequence = ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("autospec-state");
    path.with_file_name(format!(
        ".{filename}.{}.{}.tmp",
        std::process::id(),
        sequence
    ))
}

fn atomic_write(path: &Path, contents: &str) -> Result<(), String> {
    let temporary = atomic_temporary_path(path);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| format!("cannot create {}: {error}", temporary.display()))?;
    let write_result = (|| {
        file.write_all(contents.as_bytes())
            .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
        file.sync_all()
            .map_err(|error| format!("cannot sync {}: {error}", temporary.display()))?;
        fs::rename(&temporary, path)
            .map_err(|error| format!("cannot finalize {}: {error}", path.display()))
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

fn lifecycle_stop_mode(mode: StopMode) -> LifecycleStopMode {
    match mode {
        StopMode::Graceful => LifecycleStopMode::Graceful,
        StopMode::Immediate => LifecycleStopMode::Immediate,
    }
}

fn foreground_scope() -> ConductorScope {
    if std::env::var("AUTOSPEC_RUN_ONLY_ISSUES").is_ok_and(|value| !value.trim().is_empty()) {
        ConductorScope::Slice
    } else {
        ConductorScope::Repository
    }
}

fn command_error(error: super::CommandFailure) -> String {
    error.message
}

#[derive(Debug, Clone)]
struct LaunchCommands {
    monitor: ForegroundCommand,
    supervisor: ForegroundCommand,
}

#[derive(Debug, Clone)]
struct ForegroundCommand {
    program: PathBuf,
    args: Vec<String>,
}

impl ForegroundCommand {
    fn display(&self) -> String {
        std::iter::once(self.program.display().to_string())
            .chain(self.args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn argv_json(&self) -> String {
        let mut argv = vec![self.program.display().to_string()];
        argv.extend(self.args.iter().cloned());
        json_string_array(&argv)
    }
}

#[derive(Debug, Clone)]
struct RunLayout {
    state_dir: PathBuf,
    log_dir: PathBuf,
    scope: String,
    repo: String,
}

#[derive(Debug, Clone)]
struct UnitRecord {
    pid: String,
    pid_file: PathBuf,
    logpath: PathBuf,
    logpath_file: PathBuf,
}

#[derive(Debug, Clone)]
struct UnitStatus {
    pid: String,
    running: bool,
    stale_pid: bool,
    metadata_only: bool,
    pid_file: PathBuf,
    logpath: String,
    logpath_file: PathBuf,
}

#[derive(Debug, Clone)]
struct Issue {
    number: Option<i64>,
    title: String,
}

#[derive(Debug, Clone)]
struct IssueTiming {
    first: i64,
    last: i64,
    step: String,
    done: bool,
}

#[derive(Debug, Clone, Default)]
struct StateMetadata {
    status: String,
    heartbeat_at: String,
    last_cycle: String,
}

impl RunLayout {
    fn new(options: &Options) -> Result<Self, String> {
        let repo = resolve_repo(options);
        let scope = scope_slug(&repo, &options.repo_dir);
        let state_root = env_path(
            "AUTOSPEC_AUTONOMOUS_OPERATOR_DIR",
            &[".autospec", "autonomous-operator"],
        );
        let log_root = env_path("AUTOSPEC_AUTONOMOUS_LOG_DIR", &[".autospec", "logs"]);
        Ok(Self {
            state_dir: state_root.join(&scope),
            log_dir: log_root.join(&scope),
            scope,
            repo,
        })
    }
}

fn launch_commands(options: &Options) -> Result<LaunchCommands, String> {
    Ok(LaunchCommands {
        monitor: companion_command(options, "monitor")?,
        supervisor: companion_command(options, "supervise")?,
    })
}

fn companion_command(options: &Options, subcommand: &str) -> Result<ForegroundCommand, String> {
    Ok(ForegroundCommand {
        program: std::env::current_exe()
            .map_err(|error| format!("cannot resolve autonomous executable: {error}"))?,
        args: vec![
            "autonomous".to_string(),
            subcommand.to_string(),
            "--repo".to_string(),
            options.repo.clone(),
            "--repo-dir".to_string(),
            options.repo_dir.clone(),
            "--interval-sec".to_string(),
            options.interval_sec.to_string(),
        ],
    })
}

fn foreground_command(options: &Options) -> Result<ForegroundCommand, String> {
    let mut args = vec![
        "autonomous".to_string(),
        "run-foreground".to_string(),
        "--repo".to_string(),
        options.repo.clone(),
        "--repo-dir".to_string(),
        options.repo_dir.clone(),
    ];
    args.extend(conductor_passthrough_args(options));
    Ok(ForegroundCommand {
        program: std::env::current_exe()
            .map_err(|error| format!("cannot resolve foreground conductor program: {error}"))?,
        args,
    })
}

fn spawn_unit(
    name: &str,
    command: &ForegroundCommand,
    repo_dir: &str,
    state_dir: &Path,
    log_dir: &Path,
    log_override: Option<&str>,
    lease_token: Option<&str>,
) -> Result<UnitRecord, String> {
    let logpath = log_override
        .map(PathBuf::from)
        .unwrap_or_else(|| log_dir.join(format!("autospec-autonomous-{name}.log")));
    if let Some(parent) = logpath.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let log = File::create(&logpath)
        .map_err(|error| format!("cannot create {}: {error}", logpath.display()))?;
    let err_log = log
        .try_clone()
        .map_err(|error| format!("cannot clone {}: {error}", logpath.display()))?;
    let mut process = Command::new(&command.program);
    process
        .args(&command.args)
        .current_dir(repo_dir)
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err_log));
    if let Some(token) = lease_token {
        process.env("AUTOSPEC_CONDUCTOR_LEASE_TOKEN", token);
    }
    let mut child = process
        .spawn()
        .map_err(|error| format!("cannot spawn {name} command: {error}"))?;
    let pid = child.id().to_string();
    let pid_file = state_dir.join(format!("{name}.pid"));
    let logpath_file = state_dir.join(format!("{name}.logpath"));
    if let Err(error) = fs::write(&pid_file, format!("{pid}\n")) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("cannot write {}: {error}", pid_file.display()));
    }
    if let Err(error) = fs::write(&logpath_file, format!("{}\n", logpath.display())) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("cannot write {}: {error}", logpath_file.display()));
    }
    Ok(UnitRecord {
        pid,
        pid_file,
        logpath,
        logpath_file,
    })
}

fn launch_units(
    layout: &RunLayout,
    options: &Options,
    foreground: &ForegroundCommand,
    commands: &LaunchCommands,
    lease: &resilience::ConductorLease,
) -> Result<(UnitRecord, UnitRecord, UnitRecord), CommandFailure> {
    let conductor = spawn_unit(
        "conductor",
        foreground,
        &options.repo_dir,
        &layout.state_dir,
        &layout.log_dir,
        log_override_for("conductor", options),
        Some(lease.token()),
    )
    .map_err(CommandFailure::diagnostic)?;
    if !companions_enabled() {
        return Ok((
            conductor,
            empty_unit("monitor", &layout.state_dir, &layout.log_dir),
            empty_unit("supervisor", &layout.state_dir, &layout.log_dir),
        ));
    }

    let monitor = match spawn_unit(
        "monitor",
        &commands.monitor,
        &options.repo_dir,
        &layout.state_dir,
        &layout.log_dir,
        log_override_for("monitor", options),
        None,
    ) {
        Ok(unit) => unit,
        Err(error) => {
            let _ = terminate_pid(&conductor.pid);
            return Err(CommandFailure::diagnostic(error));
        }
    };
    let supervisor = match spawn_unit(
        "supervisor",
        &commands.supervisor,
        &options.repo_dir,
        &layout.state_dir,
        &layout.log_dir,
        log_override_for("supervisor", options),
        None,
    ) {
        Ok(unit) => unit,
        Err(error) => {
            let _ = terminate_pid(&monitor.pid);
            let _ = terminate_pid(&conductor.pid);
            return Err(CommandFailure::diagnostic(error));
        }
    };
    Ok((conductor, monitor, supervisor))
}

fn prepare_start_scope(layout: &RunLayout, options: &Options) -> Result<(), String> {
    let live_units = ["conductor", "monitor", "supervisor"]
        .into_iter()
        .map(|name| (name, read_unit(name, &layout.state_dir)))
        .filter(|(_, unit)| unit.running)
        .collect::<Vec<_>>();
    if live_units.is_empty() {
        return Ok(());
    }
    if !options.force {
        let live = live_units
            .iter()
            .map(|(name, unit)| format!("{name}:{}", unit.pid))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "autonomous conductor already running for {} ({live}); use restart or --force",
            layout.repo
        ));
    }
    for (_, unit) in live_units {
        let _ = terminate_pid(&unit.pid);
    }
    wait_for_scope_stopped(&layout.state_dir);
    Ok(())
}

fn log_override_for<'a>(name: &str, options: &'a Options) -> Option<&'a str> {
    if name == "conductor" && !options.log_path.is_empty() {
        Some(options.log_path.as_str())
    } else {
        None
    }
}

fn empty_unit(name: &str, state_dir: &Path, log_dir: &Path) -> UnitRecord {
    UnitRecord {
        pid: String::new(),
        pid_file: state_dir.join(format!("{name}.pid")),
        logpath: log_dir.join(format!("autospec-autonomous-{name}.log")),
        logpath_file: state_dir.join(format!("{name}.logpath")),
    }
}

fn companions_enabled() -> bool {
    std::env::var("AUTOSPEC_AUTONOMOUS_COMPANIONS")
        .map(|value| value != "0")
        .unwrap_or(true)
}

fn conductor_passthrough_args(options: &Options) -> Vec<String> {
    let mut args = Vec::new();
    if !options.max_cycles.is_empty() {
        args.push("--max-cycles".to_string());
        args.push(options.max_cycles.clone());
    }
    if options.no_digest {
        args.push("--no-digest".to_string());
    }
    if let Some(branch) = options.health_branch.as_ref() {
        args.push("--branch".to_string());
        args.push(branch.clone());
    }
    if options.interval_sec != 300 {
        args.push("--poll-interval-sec".to_string());
        args.push(options.interval_sec.to_string());
    }
    if let Some(budget_tokens) = options.budget_tokens {
        args.push("--budget-tokens".to_string());
        args.push(budget_tokens.to_string());
    }
    if let Some(budget_issues) = options.budget_issues {
        args.push("--budget-issues".to_string());
        args.push(budget_issues.to_string());
    }
    if options.dry_run {
        args.push("--dry-run".to_string());
    }
    args
}

fn write_launch_json(
    layout: &RunLayout,
    options: &Options,
    foreground: &ForegroundCommand,
    commands: &LaunchCommands,
) -> Result<(), String> {
    let path = layout.state_dir.join("launch.json");
    let budget_tokens = options
        .budget_tokens
        .map(|value| value.to_string())
        .unwrap_or_default();
    let budget_issues = options
        .budget_issues
        .map(|value| value.to_string())
        .unwrap_or_default();
    let body = format!(
        "{{\"argv\":{},\"repo\":\"{}\",\"repo_dir\":\"{}\",\"scope\":\"{}\",\"conductor_argv\":{},\"monitor_argv\":{},\"supervisor_argv\":{},\"max_cycles\":\"{}\",\"budget_tokens\":\"{}\",\"budget_issues\":\"{}\",\"no_digest\":{},\"poll_interval_sec\":\"{}\"}}\n",
        json_string_array(&options.raw_args),
        json_escape(&layout.repo),
        json_escape(&options.repo_dir),
        json_escape(&layout.scope),
        foreground.argv_json(),
        commands.monitor.argv_json(),
        commands.supervisor.argv_json(),
        json_escape(&options.max_cycles),
        json_escape(&budget_tokens),
        json_escape(&budget_issues),
        options.no_digest,
        options.interval_sec
    );
    fs::write(&path, body).map_err(|error| format!("cannot write {}: {error}", path.display()))
}

const JSON_OBJECT_START: char = '{';
const EMPTY_JSON_OBJECT: &str = "{}";

fn read_launch_json(state_dir: &Path) -> String {
    let raw = fs::read_to_string(state_dir.join("launch.json")).unwrap_or_default();
    let launch = raw.trim();
    if launch.starts_with(JSON_OBJECT_START) {
        launch.to_string()
    } else {
        EMPTY_JSON_OBJECT.to_string()
    }
}

fn read_unit(name: &str, state_dir: &Path) -> UnitStatus {
    let pid_file = state_dir.join(format!("{name}.pid"));
    let logpath_file = state_dir.join(format!("{name}.logpath"));
    let pid = fs::read_to_string(&pid_file)
        .unwrap_or_default()
        .trim()
        .to_string();
    let logpath = fs::read_to_string(&logpath_file)
        .unwrap_or_default()
        .trim()
        .to_string();
    let running = process_alive(&pid);
    let stale_pid = !pid.is_empty() && !running;
    let metadata_only = !running && (!pid.is_empty() || !logpath.is_empty());
    UnitStatus {
        running,
        stale_pid,
        metadata_only,
        pid,
        pid_file,
        logpath,
        logpath_file,
    }
}

fn unit_json(unit: &UnitRecord) -> String {
    format!(
        "{{\"pid\":\"{}\",\"pid_file\":\"{}\",\"logpath\":\"{}\",\"logpath_file\":\"{}\"}}",
        json_escape(&unit.pid),
        json_escape(&unit.pid_file.display().to_string()),
        json_escape(&unit.logpath.display().to_string()),
        json_escape(&unit.logpath_file.display().to_string())
    )
}

fn unit_status_json(unit: &UnitStatus) -> String {
    format!(
        "{{\"running\":{},\"stale_pid\":{},\"metadata_only\":{},\"pid\":\"{}\",\"pid_file\":\"{}\",\"logpath\":\"{}\",\"logpath_file\":\"{}\"}}",
        unit.running,
        unit.stale_pid,
        unit.metadata_only,
        json_escape(&unit.pid),
        json_escape(&unit.pid_file.display().to_string()),
        json_escape(&unit.logpath),
        json_escape(&unit.logpath_file.display().to_string())
    )
}

fn env_path(var: &str, default_under_home: &[&str]) -> PathBuf {
    if let Ok(value) = std::env::var(var) {
        return PathBuf::from(value);
    }
    let mut path = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()));
    for part in default_under_home {
        path.push(part);
    }
    path
}

fn scope_slug(repo: &str, repo_dir: &str) -> String {
    let source = if repo != "unknown" && !repo.is_empty() {
        repo
    } else {
        repo_dir
    };
    source
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn process_alive(pid: &str) -> bool {
    if pid.is_empty() {
        return false;
    }
    Command::new("kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn terminate_pid(pid: &str) -> bool {
    if pid.is_empty() {
        return false;
    }
    let _ = Command::new("kill").arg(pid).status();
    for _ in 0..20 {
        if !process_alive(pid) {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    false
}

fn wait_for_scope_stopped(state_dir: &Path) {
    for _ in 0..20 {
        let any_running = ["conductor", "monitor", "supervisor"]
            .iter()
            .any(|name| read_unit(name, state_dir).running);
        if !any_running {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn validate_repo_dir(options: &Options) -> Result<(), String> {
    let top = git_top_level(&options.repo_dir).ok_or_else(|| {
        format!(
            "--repo-dir {} is not a git checkout",
            Path::new(&options.repo_dir).display()
        )
    })?;
    if options.repo != "unknown" && !options.repo.is_empty() {
        if let Some(remote_slug) = git_remote_slug(&top.display().to_string()) {
            if remote_slug != options.repo {
                eprintln!(
                    "warning: --repo {} does not match checkout origin {}",
                    options.repo, remote_slug
                );
            }
        }
    }
    Ok(())
}

fn git_top_level(repo_dir: &str) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(repo_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

fn read_state_metadata(layout: &RunLayout) -> StateMetadata {
    let path = env_path(
        "AUTOSPEC_AUTONOMOUS_STATE_DIR",
        &[".autospec", "autonomous"],
    )
    .join(&layout.scope)
    .join("state.json");
    let raw = fs::read_to_string(path).unwrap_or_default();
    StateMetadata {
        status: extract_json_string(&raw, "status").unwrap_or_default(),
        heartbeat_at: extract_json_number(&raw, "heartbeat_at").unwrap_or_else(|| "null".into()),
        last_cycle: extract_json_number(&raw, "cycle").unwrap_or_default(),
    }
}

fn newest_legacy_logpath() -> Option<String> {
    let log_root = env_path("AUTOSPEC_AUTONOMOUS_LOG_DIR", &[".autospec", "logs"]);
    let mut candidates = Vec::new();
    for entry in fs::read_dir(log_root).ok()?.flatten() {
        let path = entry.path();
        let name = path.file_name()?.to_string_lossy();
        if name.starts_with("autospec-autonomous-") && name.ends_with(".log") {
            candidates.push(path);
        }
    }
    candidates.sort();
    candidates.pop().map(|path| path.display().to_string())
}

fn tail_log_lines(logpath: &str, lines: usize) -> Result<Vec<String>, String> {
    if logpath.is_empty() {
        return Ok(Vec::new());
    }
    let raw =
        fs::read_to_string(logpath).map_err(|error| format!("cannot read {logpath}: {error}"))?;
    let mut all = raw.lines().map(|line| line.to_string()).collect::<Vec<_>>();
    if lines != usize::MAX && all.len() > lines {
        all = all.split_off(all.len() - lines);
    }
    Ok(all)
}

fn tail_lines(lines: &[String], count: usize) -> Vec<String> {
    if count == usize::MAX || lines.len() <= count {
        return lines.to_vec();
    }
    lines[lines.len() - count..].to_vec()
}

fn timeline_all_lines(logpath: &str, repo: &str) -> Result<Vec<String>, String> {
    let mut lines = if logpath.is_empty() {
        Vec::new()
    } else {
        fs::read_to_string(logpath)
            .map_err(|error| format!("cannot read {logpath}: {error}"))?
            .lines()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
    };
    for dir in heartbeat_dirs(repo) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|value| value.to_str()) != Some("json") {
                    continue;
                }
                if let Ok(raw) = fs::read_to_string(path) {
                    lines.extend(raw.lines().map(|line| line.to_string()));
                }
            }
        }
    }
    Ok(lines)
}

fn heartbeat_dirs(repo: &str) -> Vec<PathBuf> {
    if repo.is_empty() || repo == "unknown" {
        return Vec::new();
    }
    let base = env_path(
        "AUTOSPEC_PROCESS_HEARTBEAT_DIR",
        &[".autospec", "process-heartbeats"],
    );
    vec![
        base.join(repo.replace(['/', ':'], "_")),
        base.join(repo.replace(['/', ':'], "__")),
        base.join(repo.replace(['/', ':'], "-")),
    ]
}

fn timeline_events(lines: &[String]) -> Vec<String> {
    let mut rows = Vec::new();
    let mut index = 0;
    let mut last_workdir = String::new();
    while index < lines.len() {
        let line = lines[index].trim();
        if let Some(rest) = line.strip_prefix("workdir:") {
            last_workdir = rest.trim().to_string();
        } else if let Some(cycle) = conductor_cycle(line) {
            rows.push(format!("time unknown - started autonomous cycle {cycle}."));
        } else if line.contains("main-health pending") && line.contains("skipping drain") {
            rows.push(
                "time unknown - skipped the backlog drain because main health was still pending."
                    .to_string(),
            );
        } else if line == "Hook audit addressed." {
            rows.push("time unknown - addressed a hook audit finding.".to_string());
        } else if line == "Changed:" {
            let (items, next) = following_dash_items(lines, index + 1);
            if !items.is_empty() {
                rows.push(format!(
                    "time unknown - updated {}.",
                    summarize_paths(&items)
                ));
            }
            index = next;
            continue;
        } else if line == "Verified:" {
            let (items, next) = following_dash_items(lines, index + 1);
            if !items.is_empty() {
                rows.push(format!(
                    "time unknown - verified {} checks: {}.",
                    items.len(),
                    items.join(", ")
                ));
            }
            index = next;
            continue;
        } else if line == "user"
            && lines
                .get(index + 1)
                .map(|next| next.trim() == "$autospec-run")
                .unwrap_or(false)
        {
            if last_workdir.is_empty() {
                rows.push("time unknown - started autospec-run.".to_string());
            } else {
                rows.push(format!(
                    "time unknown - started autospec-run in {last_workdir}."
                ));
            }
            index += 2;
            continue;
        } else if !line.is_empty() {
            let summary = summarize_timeline_line(line);
            if summary != line || line.starts_with("202") {
                rows.push(format!("{summary}."));
            }
        }
        index += 1;
    }
    dedupe_rows(rows)
}

fn conductor_cycle(line: &str) -> Option<String> {
    let rest = line.strip_prefix("[conductor] cycle ")?;
    let cycle = rest.split_whitespace().next()?;
    if cycle.chars().all(|ch| ch.is_ascii_digit()) {
        Some(cycle.to_string())
    } else {
        None
    }
}

fn following_dash_items(lines: &[String], mut index: usize) -> (Vec<String>, usize) {
    let mut items = Vec::new();
    while let Some(line) = lines.get(index) {
        let trimmed = line.trim();
        if let Some(item) = trimmed.strip_prefix("- ") {
            items.push(item.trim().trim_matches('`').to_string());
            index += 1;
        } else {
            break;
        }
    }
    (items, index)
}

fn summarize_paths(paths: &[String]) -> String {
    match paths.len() {
        0 => String::new(),
        1 => paths[0].clone(),
        2 => format!("{} and {}", paths[0], paths[1]),
        _ => {
            let mut text = paths[..paths.len() - 1].join(", ");
            text.push_str(", and ");
            text.push_str(paths.last().unwrap());
            text
        }
    }
}

fn dedupe_rows(rows: Vec<String>) -> Vec<String> {
    let mut seen = Vec::<String>::new();
    let mut output = Vec::new();
    for row in rows {
        if !seen.contains(&row) {
            seen.push(row.clone());
            output.push(row);
        }
    }
    output
}

fn timeline_forecast(lines: &[String]) -> Option<Vec<String>> {
    let text = lines.join("\n");
    let mut latest = None;
    for object in json_object_strings(&text) {
        if ["ready", "claimed", "blocked", "batch"]
            .iter()
            .any(|key| extract_json_array(&object, key).is_some())
        {
            latest = Some(object);
        }
    }
    let latest = latest?;
    let mut ready = parse_issue_array(&latest, "ready");
    let mut claimed = parse_issue_array(&latest, "claimed");
    let blocked = parse_issue_array(&latest, "blocked");
    let mut batch = parse_issue_array(&latest, "batch");
    let active = active_issue_numbers(lines);
    let candidates = unique_issues(&[
        ready.clone(),
        claimed.clone(),
        blocked.clone(),
        batch.clone(),
    ]);
    for number in active {
        if claimed.iter().any(|issue| issue.number == Some(number)) {
            continue;
        }
        if let Some(issue) = candidates.iter().find(|issue| issue.number == Some(number)) {
            claimed.push(issue.clone());
            ready.retain(|candidate| candidate.number != Some(number));
            batch.retain(|candidate| candidate.number != Some(number));
        }
    }
    let all = unique_issues(&[
        ready.clone(),
        claimed.clone(),
        blocked.clone(),
        batch.clone(),
    ]);
    let total = all.len();
    if total == 0 {
        return None;
    }
    let mut rows = vec![
        "autospec-autonomous forecast".to_string(),
        format!(
            "things left: {total} total ({} ready, {} in progress, {} blocked)",
            ready.len(),
            claimed.len(),
            blocked.len()
        ),
        format!(
            "rough ETA: about {} at 45-90 minutes per item",
            format_duration_range((total * 45) as i64, (total * 90) as i64)
        ),
    ];
    let plan_candidates = [
        (
            claimed.first(),
            "finish",
            "after current item finishes, roughly 15-45 minutes of handoff overhead",
        ),
        (
            batch.first(),
            "start",
            "likely within the next conductor cycle",
        ),
        (
            ready.first(),
            "start",
            "likely within the next conductor cycle",
        ),
    ];
    let planned = plan_candidates
        .into_iter()
        .find_map(|(issue, verb, estimate)| {
            issue.map(|issue| (issue_label(issue), verb, estimate))
        });
    let planned_label = planned
        .as_ref()
        .map(|(label, _, _)| label.clone())
        .unwrap_or_default();
    if let Some((label, verb, estimate)) = planned {
        rows.push(format!("planned next: {verb} {label}"));
        rows.push(format!("next item start estimate: {estimate}"));
    }

    let then_start = if !batch.is_empty() {
        batch.first().map(issue_label).filter(|label| {
            !label.is_empty()
                && label != &planned_label
                && !claimed.iter().any(|item| issue_label(item) == *label)
        })
    } else if !claimed.is_empty() {
        ready.first().map(issue_label)
    } else {
        ready.get(1).map(issue_label)
    };
    if let Some(label) = then_start {
        rows.push(format!("then start {label}"));
    }
    if let Some(label) = blocked.first().map(issue_label) {
        rows.push(format!("blocked later: {label}"));
    }
    Some(rows)
}

fn timeline_timings(lines: &[String]) -> Vec<String> {
    let mut history = Vec::<(i64, IssueTiming)>::new();
    for object in json_object_strings(&lines.join("\n")) {
        let Some(issue) =
            extract_json_number(&object, "issue").and_then(|value| value.parse::<i64>().ok())
        else {
            continue;
        };
        let Some(ts) =
            extract_json_number(&object, "ts").and_then(|value| value.parse::<i64>().ok())
        else {
            continue;
        };
        let step = extract_json_string(&object, "step")
            .unwrap_or_else(|| "working".to_string())
            .replace('_', " ");
        let done = matches!(
            step.as_str(),
            "merged" | "complete" | "completed" | "done" | "pr merged"
        );
        if let Some((_, timing)) = history.iter_mut().find(|(number, _)| *number == issue) {
            timing.first = timing.first.min(ts);
            if ts >= timing.last {
                timing.last = ts;
                timing.step = step;
            }
            timing.done |= done;
        } else {
            history.push((
                issue,
                IssueTiming {
                    first: ts,
                    last: ts,
                    step,
                    done,
                },
            ));
        }
    }
    if history.is_empty() {
        return Vec::new();
    }
    let mut active = Vec::new();
    let mut completed = Vec::new();
    for (issue, timing) in history {
        let elapsed = format_duration(((timing.last - timing.first) / 60).max(0));
        if timing.done {
            completed.push((timing.last, format!("#{issue} completed in {elapsed}")));
        } else {
            active.push((
                timing.last,
                format!("#{issue} current step {} after {elapsed}", timing.step),
            ));
        }
    }
    active.sort_by_key(|row| std::cmp::Reverse(row.0));
    completed.sort_by_key(|row| std::cmp::Reverse(row.0));
    let mut rows = vec!["item timing".to_string()];
    rows.extend(active.into_iter().take(3).map(|(_, row)| row));
    rows.extend(completed.into_iter().take(3).map(|(_, row)| row));
    rows
}

fn active_issue_numbers(lines: &[String]) -> Vec<i64> {
    timeline_timings_raw(lines)
        .into_iter()
        .filter_map(|(issue, timing)| if timing.done { None } else { Some(issue) })
        .collect()
}

fn timeline_timings_raw(lines: &[String]) -> Vec<(i64, IssueTiming)> {
    let mut history = Vec::<(i64, IssueTiming)>::new();
    for object in json_object_strings(&lines.join("\n")) {
        let Some(issue) =
            extract_json_number(&object, "issue").and_then(|value| value.parse::<i64>().ok())
        else {
            continue;
        };
        let Some(ts) =
            extract_json_number(&object, "ts").and_then(|value| value.parse::<i64>().ok())
        else {
            continue;
        };
        let step = extract_json_string(&object, "step")
            .unwrap_or_else(|| "working".to_string())
            .replace('_', " ");
        let done = matches!(
            step.as_str(),
            "merged" | "complete" | "completed" | "done" | "pr merged"
        );
        if let Some((_, timing)) = history.iter_mut().find(|(number, _)| *number == issue) {
            timing.first = timing.first.min(ts);
            if ts >= timing.last {
                timing.last = ts;
                timing.step = step;
            }
            timing.done |= done;
        } else {
            history.push((
                issue,
                IssueTiming {
                    first: ts,
                    last: ts,
                    step,
                    done,
                },
            ));
        }
    }
    history
}

fn unique_issues(groups: &[Vec<Issue>]) -> Vec<Issue> {
    let mut output = Vec::new();
    for group in groups {
        for issue in group {
            if let Some(number) = issue.number {
                if let Some(existing) = output
                    .iter_mut()
                    .find(|item: &&mut Issue| item.number == Some(number))
                {
                    *existing = issue.clone();
                } else {
                    output.push(issue.clone());
                }
            } else if !issue.title.is_empty()
                && !output.iter().any(|item| item.title == issue.title)
            {
                output.push(issue.clone());
            }
        }
    }
    output
}

fn parse_issue_array(object: &str, key: &str) -> Vec<Issue> {
    let Some(array) = extract_json_array(object, key) else {
        return Vec::new();
    };
    json_object_strings(&array)
        .into_iter()
        .filter_map(|item| {
            let number = extract_json_number(&item, "number").and_then(|value| value.parse().ok());
            let title = extract_json_string(&item, "title").unwrap_or_default();
            if number.is_none() && title.is_empty() {
                None
            } else {
                Some(Issue { number, title })
            }
        })
        .collect()
}

fn issue_label(issue: &Issue) -> String {
    match (issue.number, issue.title.is_empty()) {
        (Some(number), false) => format!("#{number} {}", issue.title),
        (Some(number), true) => format!("#{number}"),
        (None, _) => issue.title.clone(),
    }
}

fn format_duration(minutes: i64) -> String {
    if minutes < 60 {
        format!("{minutes} minutes")
    } else if minutes % 60 == 0 {
        format!("{} hours", minutes / 60)
    } else {
        let tenths = (minutes * 10 + 30) / 60;
        format!("{}.{} hours", tenths / 10, tenths % 10)
    }
}

fn format_duration_range(low_minutes: i64, high_minutes: i64) -> String {
    if high_minutes < 60 {
        return format!("{low_minutes}-{high_minutes} minutes");
    }
    if low_minutes >= 60 && low_minutes % 60 == 0 && high_minutes % 60 == 0 {
        return format!("{}-{} hours", low_minutes / 60, high_minutes / 60);
    }
    format!(
        "{}-{}",
        format_duration(low_minutes),
        format_duration(high_minutes)
    )
}

fn follow_log(logpath: &str, interval_sec: u64) -> Result<(), String> {
    if logpath.is_empty() {
        return Ok(());
    }
    let mut offset = 0;
    loop {
        let raw = fs::read_to_string(logpath)
            .map_err(|error| format!("cannot read {logpath}: {error}"))?;
        if raw.len() > offset {
            print!("{}", &raw[offset..]);
            io::stdout()
                .flush()
                .map_err(|error| format!("cannot flush stdout: {error}"))?;
            offset = raw.len();
        }
        thread::sleep(Duration::from_secs(interval_sec.max(1)));
    }
}

fn summarize_timeline_line(line: &str) -> String {
    if line.len() > 21 && line.as_bytes().get(10) == Some(&b'T') {
        line[21..].trim_start().to_string()
    } else {
        line.to_string()
    }
}

fn resolve_repo(options: &Options) -> String {
    if options.repo != "unknown" && !options.repo.is_empty() {
        return options.repo.clone();
    }
    git_remote_slug(&options.repo_dir).unwrap_or_else(|| options.repo.clone())
}

fn git_remote_slug(repo_dir: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(repo_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_github_slug(String::from_utf8_lossy(&output.stdout).trim())
}

fn parse_github_slug(remote: &str) -> Option<String> {
    let trimmed = remote.trim().trim_end_matches(".git");
    if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
        return Some(rest.to_string());
    }
    if let Some(index) = trimmed.find("github.com/") {
        return Some(trimmed[index + "github.com/".len()..].to_string());
    }
    None
}

fn extract_json_string(raw: &str, key: &str) -> Option<String> {
    let rest = value_after_json_key(raw, key)?;
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn extract_json_number(raw: &str, key: &str) -> Option<String> {
    let rest = value_after_json_key(raw, key)?;
    let rest = rest.strip_prefix('"').unwrap_or(rest);
    let end = rest
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(rest.len());
    if end == 0 {
        None
    } else {
        Some(rest[..end].to_string())
    }
}

fn value_after_json_key<'a>(raw: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let key_start = raw.find(&needle)? + needle.len();
    let after_key = raw[key_start..].trim_start();
    let after_colon = after_key.strip_prefix(':')?.trim_start();
    Some(after_colon)
}

fn extract_json_array(raw: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let key_start = raw.find(&needle)?;
    let array_start = raw[key_start..].find('[')? + key_start;
    let mut depth = 0;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in raw[array_start..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '[' if !in_string => depth += 1,
            ']' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    let end = array_start + offset + ch.len_utf8();
                    return Some(raw[array_start..end].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn json_object_strings(raw: &str) -> Vec<String> {
    let mut objects = Vec::new();
    let mut start = None;
    let mut depth = 0;
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in raw.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '{' if !in_string => {
                if depth == 0 {
                    start = Some(index);
                }
                depth += 1;
            }
            '}' if !in_string && depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if let Some(start_index) = start.take() {
                        objects.push(raw[start_index..index + ch.len_utf8()].to_string());
                    }
                }
            }
            _ => {}
        }
    }
    objects
}

fn stop_flag_path(layout: &RunLayout) -> PathBuf {
    std::env::var("AUTOSPEC_STOP_FLAG_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| layout.state_dir.join("stop.flag"))
}

fn read_stop_mode(layout: &RunLayout) -> Result<Option<LifecycleStopMode>, String> {
    let path = stop_flag_path(layout);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let mode = raw
        .lines()
        .next()
        .ok_or_else(|| format!("cannot parse {}: missing stop mode", path.display()))?;
    match mode {
        "graceful" => Ok(Some(LifecycleStopMode::Graceful)),
        "immediate" => Ok(Some(LifecycleStopMode::Immediate)),
        _ => Err(format!(
            "cannot parse {}: invalid stop mode",
            path.display()
        )),
    }
}

fn clear_stop_flag(layout: &RunLayout) -> Result<(), String> {
    let flag = stop_flag_path(layout);
    if flag.exists() {
        fs::remove_file(&flag)
            .map_err(|error| format!("cannot remove {}: {error}", flag.display()))?;
    }
    Ok(())
}

fn write_stop_flag(layout: &RunLayout, mode: StopMode) -> Result<PathBuf, String> {
    let flag = stop_flag_path(layout);
    let dir = flag
        .parent()
        .ok_or_else(|| format!("cannot resolve parent for {}", flag.display()))?;
    fs::create_dir_all(dir).map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    let stamp = Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string());
    let user = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
    let host = Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                None
            }
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "localhost".to_string());
    fs::write(
        &flag,
        format!("{}\n{} {}@{}\n", mode.as_str(), stamp, user, host),
    )
    .map_err(|error| format!("cannot write {}: {error}", flag.display()))?;
    Ok(flag)
}

fn json_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn json_string_array(values: &[String]) -> String {
    let items = values
        .iter()
        .map(|value| format!("\"{}\"", json_escape(value)))
        .collect::<Vec<_>>();
    format!("[{}]", items.join(","))
}

#[cfg(test)]
mod foreground_tests {
    use super::*;

    #[test]
    fn lifecycle_atomic_temp_paths_are_unique_per_write() {
        let path = Path::new("/tmp/lifecycle.json");

        assert_ne!(atomic_temporary_path(path), atomic_temporary_path(path));
    }

    #[test]
    fn executor_launch_failure_becomes_a_blocked_receipt() {
        let request = ExecutorRequest {
            program: PathBuf::from("/definitely-not-an-autospec-executor"),
            args: Vec::new(),
            current_dir: std::env::temp_dir(),
            repo: "test/repo".to_string(),
            issue: 42,
        };

        let receipt = request.run();

        assert_eq!(
            receipt.outcome,
            ConductorOutcome::Blocked("executor_receipt_failed".to_string())
        );
        assert_eq!(receipt.claim_step, "executor_receipt_failed");
    }

    #[test]
    fn completed_slice_state_is_retained_on_a_fresh_foreground_invocation() {
        let state = ConductorState::new("test/repo", ConductorScope::Slice, 3)
            .expect("state")
            .transition(ConductorEvent::ScanEmpty)
            .expect("slice complete");

        assert!(foreground_state_is_retained(&state));
    }
}

fn print_help() {
    println!(
        "autospec autonomous\n\nUSAGE:\n    autospec autonomous [start|status|list|logs|watch|timeline|monitor|supervise|cleanup|stop|restart|run-foreground|main-health] [OPTIONS]\n    autospec autonomous lifecycle decide --repo OWNER/REPO [LIFECYCLE OPTIONS]\n\nCommon options:\n    --repo OWNER/REPO\n    --repo-dir DIR\n    --json\n    --dry-run\n    --max-cycles N\n    --budget-tokens N\n    --budget-issues N\n    --poll-interval-sec N\n    --graceful | --immediate"
    );
}
