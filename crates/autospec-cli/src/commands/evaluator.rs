//! `autospec evaluator` — manage the evaluation store (registry, epochs, pin).

pub mod args;
mod epoch_cmds;
mod fsutil;
mod registry_cmds;
pub mod store;
pub mod types;

use crate::commands::CommandFailure;

const HELP: &str = "\
autospec evaluator — manage the evaluation store

Usage:
  autospec evaluator init [--policy <policy.json>] [--root <dir>] [--json]
      Initialize the store: write-once policy, genesis epoch, current pointer.

  autospec evaluator register --file <definition.json> [--root <dir>] [--json]
      Register an immutable evaluator definition (evaluators/<slot>/v<N>.json).

  autospec evaluator list [--root <dir>] [--json]
      List every registered evaluator, sorted by slot then version.

  autospec evaluator show <slot@version> [--root <dir>] [--json]
      Show one registered definition plus its recomputed digest.

  autospec evaluator pin <slot@version> --actor <name> [--root <dir>] [--json]
      Seed an empty slot at a registered version into the next epoch,
      committing a human-approved promotion event.

  autospec evaluator epoch <current|history> [--root <dir>] [--json]
      Show the active epoch or the full epoch history (with promotion ids).

Options:
  --root <dir>   Base directory; the store lives at .autospec/evaluation
                 underneath it (default: current directory).
  --json         Emit machine-readable JSON instead of text.
  -h, --help     Show usage.";

fn print_help() {
    println!("{HELP}");
}

/// Dispatch `autospec evaluator <subcommand> [args]`.
pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    match args.first().map(String::as_str) {
        None | Some("-h") | Some("--help") => {
            print_help();
            Ok(())
        }
        Some("init") => registry_cmds::init(&args[1..]),
        Some("register") => registry_cmds::register(&args[1..]),
        Some("list") => registry_cmds::list(&args[1..]),
        Some("show") => registry_cmds::show(&args[1..]),
        Some("pin") => registry_cmds::pin(&args[1..]),
        Some("epoch") => epoch_cmds::run(&args[1..]),
        Some(other) => Err(CommandFailure::diagnostic(format!(
            "unknown evaluator subcommand: {other}\n{HELP}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_without_subcommand_prints_help() {
        let result = run(&[]);
        assert!(result.is_ok(), "expected Ok, got {result:?}");
    }

    #[test]
    fn run_with_unknown_subcommand_is_diagnostic() {
        let error = run(&[String::from("bogus")]).unwrap_err();
        assert!(error
            .message
            .contains("unknown evaluator subcommand: bogus"));
        assert!(error.message.contains("Usage:"));
    }

    #[test]
    fn help_lists_all_six_subcommands() {
        let help = HELP.to_string();
        for subcommand in ["init", "register", "list", "show", "pin", "epoch"] {
            assert!(
                help.contains(subcommand),
                "help text missing subcommand {subcommand}"
            );
        }
        assert!(help.contains("--root"));
        assert!(help.contains("--json"));
        assert!(help.contains("--policy"));
        assert!(help.contains("--file"));
        assert!(help.contains("--actor"));
    }
}
