use super::*;

pub(super) fn is_launch_preview(options: &Options) -> bool {
    options.dry_run
        && matches!(
            options.subcommand.as_str(),
            "start" | "restart" | "resume" | "run-foreground"
        )
}

pub(super) fn requires_autonomous_runtime_support(
    options: &Options,
    launch_mode: LaunchMode,
) -> bool {
    options.subcommand == "run-foreground"
        || (!options.dry_run
            && (matches!(options.subcommand.as_str(), "start" | "restart" | "resume")
                || launch_mode == LaunchMode::Foreground))
}

pub(super) fn preview_launch(
    options: &Options,
    launch_mode: LaunchMode,
) -> Result<(), CommandFailure> {
    let commands = launch_commands(options).map_err(CommandFailure::diagnostic)?;
    let foreground = foreground_command(options).map_err(CommandFailure::diagnostic)?;
    if options.json {
        let mut body = format!(
            "{{\"command\":\"autonomous\",\"subcommand\":\"{}\",\"status\":\"dry-run\",\"repo\":\"{}\",\"repo_dir\":\"{}\",\"conductor\":\"{}\",\"companions\":{{\"monitor\":\"{}\",\"supervisor\":\"{}\"}}}}",
            json_escape(&options.subcommand),
            json_escape(&options.repo),
            json_escape(&options.repo_dir),
            json_escape(&foreground.display()),
            json_escape(&commands.monitor.display()),
            json_escape(&commands.supervisor.display())
        );
        if launch_mode == LaunchMode::Follow {
            body.pop();
            body.push_str(",\"follow\":\"scoped conductor log\"}");
        }
        println!("{body}");
    } else {
        println!("autospec autonomous {}: dry-run", options.subcommand);
        println!("conductor: {}", foreground.display());
        println!("monitor: {}", commands.monitor.display());
        println!("supervisor: {}", commands.supervisor.display());
        if launch_mode == LaunchMode::Follow {
            println!("follow: scoped conductor log");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutating_foreground_entries_require_native_runtime_support() {
        let mut options = Options {
            subcommand: "run-foreground".to_string(),
            ..Options::default()
        };
        assert!(requires_autonomous_runtime_support(
            &options,
            LaunchMode::Detached
        ));

        options.subcommand = "start".to_string();
        options.foreground = true;
        assert!(requires_autonomous_runtime_support(
            &options,
            LaunchMode::Foreground
        ));

        options.dry_run = true;
        assert!(!requires_autonomous_runtime_support(
            &options,
            LaunchMode::Foreground
        ));

        options.subcommand = "run-foreground".to_string();
        assert!(is_launch_preview(&options));
        assert!(requires_autonomous_runtime_support(
            &options,
            LaunchMode::Detached
        ));
    }
}
