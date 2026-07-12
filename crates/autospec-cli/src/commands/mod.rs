pub mod autonomous;
pub mod benchmark;
pub mod doctor;
pub mod growth_report;
pub mod init;
pub mod plan;
pub mod report;
pub mod resume;
pub mod run;
pub mod showcase;
pub mod status;
pub mod validate;

const COMMANDS: &[(&str, &str)] = &[
    ("init", "Initialize AutoSpec metadata"),
    ("doctor", "Check the Rust core workspace"),
    ("status", "Summarize local AutoSpec state"),
    ("autonomous", "Plan and supervise autonomous conductor runs"),
    ("plan", "Inspect a generated spec package"),
    ("validate", "Run configured validation gates"),
    ("run", "Execute the spec queue"),
    ("resume", "Resume an interrupted run"),
    ("report", "Render release and run reports"),
    ("showcase", "Render a local demo showcase"),
    ("benchmark", "Run local benchmark checks"),
    (
        "growth-report",
        "Render local-only launch readiness metrics",
    ),
];

pub fn run(args: Vec<String>) -> Result<(), String> {
    match args.as_slice() {
        [] => {
            print_help();
            Ok(())
        }
        [flag] if flag == "--help" || flag == "-h" => {
            print_help();
            Ok(())
        }
        [command, rest @ ..] => match command.as_str() {
            "init" => init::run(rest),
            "doctor" => doctor::run(rest),
            "status" => status::run(rest),
            "autonomous" => autonomous::run(rest),
            "plan" => plan::run(rest),
            "validate" => validate::run(rest),
            "run" => run::run(rest),
            "resume" => resume::run(rest),
            "report" => report::run(rest),
            "showcase" => showcase::run(rest),
            "benchmark" => benchmark::run(rest),
            "growth-report" => growth_report::run(rest),
            _ => Err(format!("unknown autospec command: {command}")),
        },
    }
}

fn print_help() {
    println!("autospec\n\nUSAGE:\n    autospec [COMMAND]\n\nCOMMANDS:");
    for (command, description) in COMMANDS {
        println!("    {command:<14} {description}");
    }
    println!("\nOPTIONS:\n    -h, --help       Print help");
}

fn json_status(command: &str, status: &str) {
    println!("{{\"command\":\"{command}\",\"status\":\"{status}\"}}");
}

fn not_implemented(command: &str) -> Result<(), String> {
    Err(format!("autospec {command} is not yet implemented"))
}

fn is_json(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--json")
}
