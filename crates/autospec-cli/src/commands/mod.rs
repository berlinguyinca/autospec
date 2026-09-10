pub mod aar;
pub mod autonomous;
pub mod benchmark;
pub mod claim;
pub mod cost;
pub mod dispatch;
pub mod dispatch_spec;
pub mod doctor;
pub mod explore;
pub mod growth_report;
pub mod handoff;
pub mod init;
pub mod initiative;
pub mod insights;
pub mod issue;
pub mod lint;
pub mod managed_project;
pub mod parent;
pub mod plan;
pub mod queue;
pub mod rag;
pub mod repair_loop;
pub mod report;
pub mod resources;
pub mod resume;
pub mod run;
pub mod runtime;
pub mod showcase;
pub mod status;
pub mod validate;

#[cfg(test)]
pub(crate) static PROCESS_ENVIRONMENT: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandFailureKind {
    Diagnostic,
    Transient,
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandFailure {
    pub message: String,
    pub exit_code: i32,
    pub kind: CommandFailureKind,
}

impl CommandFailure {
    pub fn diagnostic(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 2,
            kind: CommandFailureKind::Diagnostic,
        }
    }

    pub fn transient(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 2,
            kind: CommandFailureKind::Transient,
        }
    }

    pub fn status(message: impl Into<String>, exit_code: i32) -> Self {
        Self {
            message: message.into(),
            exit_code,
            kind: CommandFailureKind::Status,
        }
    }

    pub fn into_transient(mut self) -> Self {
        self.kind = CommandFailureKind::Transient;
        self
    }
}

impl std::fmt::Display for CommandFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

const COMMANDS: &[(&str, &str)] = &[
    ("init", "Initialize AutoSpec metadata"),
    ("aar", "Inspect adaptive agent runtime policy"),
    ("initiative", "Inspect cross-repository initiatives"),
    ("lint", "Lint issue and implementation policy inputs"),
    ("claim", "Manage GitHub-backed issue claim state"),
    (
        "cost",
        "Account GPU-hours by terminal status: runs, share, rework, defect cost, threshold flags",
    ),
    ("parent", "Reconcile decomposed parent issue state"),
    ("queue", "Compute the safe GitHub issue queue"),
    (
        "repair-loop",
        "Observe repair loops: rate, consecutive-sweep escalation, defect tickets",
    ),
    (
        "dispatch",
        "Gate dispatch on queue freshness and per-hop liveness",
    ),
    (
        "resources",
        "List and show resource ledger rows (read-only)",
    ),
    (
        "doctor",
        "Check the Rust core workspace (`doctor code-intel` for LSP health)",
    ),
    ("status", "Summarize local AutoSpec state"),
    ("autonomous", "Plan and supervise autonomous conductor runs"),
    ("plan", "Inspect a generated spec package"),
    ("rag", "Inspect Agentic RAG policy and routing"),
    ("validate", "Run configured validation gates"),
    ("run", "Execute the spec queue"),
    ("runtime", "Inspect runtime ownership policy"),
    ("resume", "Resume an interrupted run"),
    ("report", "Render release and run reports"),
    ("showcase", "Render a local demo showcase"),
    ("benchmark", "Run local benchmark checks"),
    (
        "growth-report",
        "Render local-only launch readiness metrics",
    ),
    (
        "handoff",
        "Produce the autospec.implementation-handoff.v1 handoff (side-effect-free)",
    ),
];

pub fn run(args: Vec<String>) -> Result<(), CommandFailure> {
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
            "init" => init::run(rest).map_err(CommandFailure::diagnostic),
            "aar" => aar::run(rest),
            "initiative" => initiative::run(rest),
            "issue" => issue::run(rest),
            "insights" => insights::run(rest)
                .map(|code| {
                    if code == 0 {
                        Ok(())
                    } else {
                        Err(CommandFailure::status(String::new(), code))
                    }
                })
                .unwrap_or_else(|message| Err(CommandFailure::diagnostic(message))),
            "lint" => lint::run(rest),
            "claim" => claim::run(rest),
            "cost" => cost::run(rest).map_err(CommandFailure::diagnostic),
            "parent" => parent::run(rest),
            "queue" => queue::run(rest),
            "dispatch" => dispatch::run(rest),
            "resources" => resources::run(rest),
            "doctor" => doctor::run(rest).map_err(CommandFailure::diagnostic),
            "explore" => explore::run(rest),
            "status" => status::run(rest).map_err(CommandFailure::diagnostic),
            "autonomous" => autonomous::run(rest),
            "plan" => plan::run(rest).map_err(CommandFailure::diagnostic),
            "rag" => rag::run(rest),
            "repair-loop" => repair_loop::run(rest),
            "validate" => validate::run(rest).map_err(CommandFailure::diagnostic),
            "run" => run::run(rest).map_err(CommandFailure::diagnostic),
            "runtime" => runtime::run(rest),
            "resume" => resume::run(rest).map_err(CommandFailure::diagnostic),
            "report" => report::run(rest).map_err(CommandFailure::diagnostic),
            "showcase" => showcase::run(rest).map_err(CommandFailure::diagnostic),
            "benchmark" => benchmark::run(rest).map_err(CommandFailure::diagnostic),
            "growth-report" => growth_report::run(rest).map_err(CommandFailure::diagnostic),
            "handoff" => handoff::run(rest),
            _ => Err(CommandFailure::diagnostic(format!(
                "unknown autospec command: {command}"
            ))),
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

fn is_json(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--json")
}
