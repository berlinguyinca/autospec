fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let result = match args.first().map(String::as_str) {
        Some("queue") => commands::run_queue(&args[1..]),
        _ => commands::run(args),
    };
    let _ = result;
}
