mod commands;

use std::env;
use std::process;

fn main() {
    if let Err(error) = commands::run(env::args().skip(1).collect()) {
        if !error.message.is_empty() {
            eprintln!("{error}");
        }
        process::exit(error.exit_code);
    }
}
