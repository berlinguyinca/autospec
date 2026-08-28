mod commands;

use std::env;
use std::process;

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let result = if args.first().is_some_and(|command| command == "project") {
        commands::managed_project::run(&args[1..])
            .map_err(|error| commands::CommandFailure::diagnostic(error.to_string()))
    } else {
        commands::run(args)
    };
    if let Err(error) = result {
        if !error.message.is_empty() {
            eprintln!("{error}");
        }
        process::exit(error.exit_code);
    }
}
