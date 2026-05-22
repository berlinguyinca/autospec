// main.rs — CLI entry point using clap
use clap::Parser;

mod lib;
use lib::greet;

/// CLI greeter
#[derive(Parser)]
#[command(name = "hello")]
struct Args {
    /// Name to greet
    name: String,
}

fn main() {
    let args = Args::parse();
    println!("{}", greet(&args.name));
}
