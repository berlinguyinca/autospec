pub mod code_intel;

pub fn run(args: &[String]) -> Result<(), String> {
    if args
        .first()
        .is_some_and(|argument| argument == "code-intel")
    {
        let root = std::env::current_dir()
            .map_err(|error| format!("could not resolve the current worktree: {error}"))?;
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
