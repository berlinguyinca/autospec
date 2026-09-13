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

// The command table below is the single declaration site for every CLI
// command. One `commands!` invocation generates all three forms that
// previously lived as hand-kept parallel lists (#4017): the `pub mod`
// declarations, the `const COMMANDS` help table, and the dispatch match arms.
// Adding a command is a one-line change; the three forms cannot disagree
// because only one exists.
//
// Entry shapes:
//   module => "help text", shape;                 // dispatch name = module name
//   module as "display-name" => "help text", shape;
//
// Handler shapes (the return type of each command's `pub fn run`):
//   direct      — `Result<(), CommandFailure>`
//   diagnostic  — `Result<(), String>`, errors wrapped in CommandFailure::diagnostic
//   status_code — `Result<i32, String>`, Ok(0) succeeds, other codes become
//                 CommandFailure::status, Err becomes CommandFailure::diagnostic

macro_rules! commands_name {
    ($module:ident) => {
        stringify!($module)
    };
    ($module:ident as $name:literal) => {
        $name
    };
}

macro_rules! commands_entry {
    ($module:ident => $help:literal) => {
        (stringify!($module), $help)
    };
    ($module:ident as $name:literal => $help:literal) => {
        ($name, $help)
    };
}

macro_rules! commands_dispatch {
    ($module:ident, direct, $rest:ident) => {
        $module::run($rest)
    };
    ($module:ident, diagnostic, $rest:ident) => {
        $module::run($rest).map_err(CommandFailure::diagnostic)
    };
    ($module:ident, status_code, $rest:ident) => {
        $module::run($rest)
            .map(|code| {
                if code == 0 {
                    Ok(())
                } else {
                    Err(CommandFailure::status(String::new(), code))
                }
            })
            .unwrap_or_else(|message| Err(CommandFailure::diagnostic(message)))
    };
}

macro_rules! commands {
    ($($module:ident $(as $name:literal)? => $help:literal, $shape:ident ;)*) => {
        $(
            pub mod $module;
        )*

        const COMMANDS: &[(&str, &str)] = &[$(
            commands_entry!($module $(as $name)? => $help),
        )*];

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
                    $(
                        commands_name!($module $(as $name)?) => {
                            commands_dispatch!($module, $shape, rest)
                        }
                    )*
                    _ => Err(CommandFailure::diagnostic(format!(
                        "unknown autospec command: {command}"
                    ))),
                },
            }
        }
    };
}

// Helper modules (not commands) stay as plain declarations outside the table.
pub mod dispatch_spec;
pub mod managed_project;

commands! {
    init => "Initialize AutoSpec metadata", diagnostic;
    aar => "Inspect adaptive agent runtime policy", direct;
    anchor => "Register and verify protected evaluator anchor suites", direct;
    evaluator => "Manage versioned evaluators, epochs, and promotions", direct;
    initiative => "Inspect cross-repository initiatives", direct;
    issue => "Stamp and promote the canonical GitHub issue", direct;
    issue_skeleton as "issue-skeleton" => "Render a structured YAML skeleton into a team-lensed issue body", status_code;
    insights => "Inspect the continuous improvement engine (spec §44)", status_code;
    lint => "Lint issue and implementation policy inputs", direct;
    claim => "Manage GitHub-backed issue claim state", direct;
    cost => "Account GPU-hours by terminal status: runs, share, rework, defect cost, threshold flags", diagnostic;
    convert => "The patch-to-PR conversion pass: plan (default) or --apply to convert fresh agent patches", direct;
    parent => "Reconcile decomposed parent issue state", direct;
    queue => "Compute the safe GitHub issue queue", direct;
    repair_loop as "repair-loop" => "Observe repair loops: rate, consecutive-sweep escalation, defect tickets", direct;
    dispatch => "Gate dispatch on queue freshness and per-hop liveness", direct;
    dispatch_outcomes as "dispatch-outcomes" => "Attribute dispatch outcomes to model and spec size band (append-only ledger report)", diagnostic;
    resources => "List and show resource ledger rows (read-only)", direct;
    cleanup => "Render the resource-cleanup dry-run report (Phase 1: observation only)", direct;
    doctor => "Check the Rust core workspace (`doctor code-intel` for LSP health)", diagnostic;
    explore => "Plan repository routing and specialist discovery", direct;
    status => "Summarize local AutoSpec state", diagnostic;
    autonomous => "Plan and supervise autonomous conductor runs", direct;
    plan => "Inspect a generated spec package", diagnostic;
    rag => "Inspect Agentic RAG policy and routing", direct;
    validate => "Run configured validation gates", diagnostic;
    run => "Execute the spec queue", diagnostic;
    runtime => "Inspect runtime ownership policy", direct;
    process_termination as "process-kill" => "Terminate processes by command-line pattern (self-safe: bracketed pattern + session exclusion)", status_code;
    resume => "Resume an interrupted run", diagnostic;
    report => "Render release and run reports", diagnostic;
    showcase => "Render a local demo showcase", diagnostic;
    benchmark => "Run local benchmark checks", diagnostic;
    growth_report as "growth-report" => "Render local-only launch readiness metrics", diagnostic;
    graph => "Analyze a proposed issue DAG: metrics, execution waves, planner summary", direct;
    handoff => "Produce the autospec.implementation-handoff.v1 handoff (side-effect-free)", direct;
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
