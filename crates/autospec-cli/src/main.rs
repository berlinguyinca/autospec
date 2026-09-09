mod commands;

use std::env;
use std::process;

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let result = match args.first().map(String::as_str) {
        Some("project") => commands::managed_project::run(&args[1..])
            .map_err(|error| commands::CommandFailure::diagnostic(error.to_string())),
        Some("portfolio") => commands::managed_project::run_portfolio(&args[1..]),
        _ => commands::run(args),
    };
    if let Err(error) = result {
        if !error.message.is_empty() {
            eprintln!("{error}");
        }
        process::exit(error.exit_code);
    }
}
