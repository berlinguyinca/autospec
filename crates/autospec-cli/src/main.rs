mod commands;

use std::env;
use std::process;

fn main() {
    if let Err(error) = commands::run(env::args().skip(1).collect()) {
        eprintln!("{error}");
        process::exit(2);
    }
}
