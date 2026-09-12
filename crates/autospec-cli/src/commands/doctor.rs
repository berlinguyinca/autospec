use std::path::Path;

pub mod code_intel;
pub mod failures;
pub mod reservation;
pub mod resources;

pub fn run(args: &[String]) -> Result<(), String> {
    if args.first().is_some_and(|argument| argument == "failures") {
        return failures_command(&args[1..]);
    }
    if args.first().is_some_and(|argument| argument == "resources") {
        return resources_command(&args[1..]);
    }
    if args
        .first()
        .is_some_and(|argument| argument == "reservation")
    {
        return reservation_command(&args[1..]);
    }
    if args
        .first()
        .is_some_and(|argument| argument == "code-intel")
    {
        let root = std::env::current_dir()
            .map_err(|error| format!("could not resolve the current worktree: {error}"))?;
        let rest = &args[1..];
        if rest.first().is_some_and(|argument| argument == "gate") {
            return gate(&root, rest);
        }
        println!("{}", code_intel::run(&root, super::is_json(args))?);
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--readiness") {
        let report = readiness_report();
        if super::is_json(args) {
            println!("{report}");
        } else {
            println!("AutoSpec readiness: see --json for machine-readable details");
        }
        return Ok(());
    }
    if super::is_json(args) {
        print!(
            "{}",
            autospec_core::doctor_report_json().replace(
                "\"status\":\"ok\"",
                "\"command\":\"doctor\",\"status\":\"ok\""
            )
        );
    } else {
        println!("AutoSpec doctor: ok");
    }
    Ok(())
}

/// `autospec doctor reservation` — the agent budget against a Slurm
/// reservation's walltime, checked before the run starts (issue #3690). The
/// refusal travels in the exit code: 0 when the budget fits, 1 when the
/// reservation cannot host it.
fn reservation_command(args: &[String]) -> Result<(), String> {
    let outcome = reservation::run(args)?;
    println!("{}", outcome.rendered);
    if outcome.refused {
        std::process::exit(reservation::REFUSED_EXIT_CODE);
    }
    Ok(())
}

/// `autospec doctor failures` — repeated failure signatures over a rolling
/// window of fleet agent runs. The verdict travels in the exit code: 0 when no
/// signature is systemic, 1 when one is, so a monitor detects a repeated
/// failure without scraping the table.
fn failures_command(args: &[String]) -> Result<(), String> {
    let root = std::env::current_dir()
        .map_err(|error| format!("could not resolve the current worktree: {error}"))?;
    let outcome = failures::run(&root, args)?;
    println!("{}", outcome.rendered);
    if outcome.systemic {
        std::process::exit(failures::SYSTEMIC_EXIT_CODE);
    }
    Ok(())
}

/// The mandatory-gate subcommand. Role and configuration errors surface as
/// `Err` (the CLI maps them to exit 2); a gate verdict ends the process — 0 on
/// pass, 1 on fail — because a failed mandatory gate must never return
/// success to its caller.
/// `autospec doctor resources` — per-type resource health (spec §24.3).
/// Observation only: live git listings plus the core Docker and process
/// observers, bucketed into the §24.3 counts with a reclaimable-disk
/// estimate. Exits 0 even without a resource ledger.
fn resources_command(args: &[String]) -> Result<(), String> {
    let root = std::env::current_dir()
        .map_err(|error| format!("could not resolve the current worktree: {error}"))?;
    println!("{}", resources::run(&root, args)?);
    Ok(())
}

fn gate(root: &Path, args: &[String]) -> Result<(), String> {
    let outcome = code_intel::run_gate(root, args)?;
    println!(
        "{}",
        code_intel::render_gate(&outcome, super::is_json(args))
    );
    let code = code_intel::gate_exit_code(&outcome);
    if code == 0 {
        return Ok(());
    }
    std::process::exit(code);
}

fn readiness_report() -> String {
    let git_repo = std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    let github_remote = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .map(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains("github.com")
        })
        .unwrap_or(false);
    let autospec_config = std::path::Path::new(".autospec/autospec.yml").exists();
    let direct_validation = true;

    let define = git_repo && github_remote;
    let run = define && autospec_config && direct_validation;
    let autonomous = run && std::path::Path::new("scripts/lib/autospec-loop.sh").exists();

    format!(
        "{{\"command\":\"doctor\",\"mode\":\"readiness\",\"status\":\"{}\",\"checks\":{{\"git_repo\":{},\"github_remote\":{},\"autospec_config\":{},\"direct_validation\":{}}},\"workflow_recommendations\":{{\"define\":\"{}\",\"run\":\"{}\",\"autonomous\":\"{}\"}}}}",
        if define { "ok" } else { "blocked" },
        git_repo,
        github_remote,
        autospec_config,
        direct_validation,
        if define { "safe" } else { "blocked:no-github-repo" },
        if run { "safe" } else { "blocked:missing-config-or-validation" },
        if autonomous { "safe" } else { "blocked:missing-autonomous-loop" },
    )
}
