//! `autospec insights` — the §44 command surface of the continuous
//! improvement engine (`docs/specs/2026-09-08-continuous-improvement-engine.md`).
//!
//! All 12 subcommands are registered with help and parsing. `sessions` is
//! backed by the stored session rows of the insights pool (issue #3827);
//! the analyzer subcommands ship in their own issues and report a named
//! unavailable status with exit code 3 until then. No subcommand echoes a
//! DSN or a payload body, and an unavailable analyzer is a message, not a
//! panic.

use serde_json::{json, Map, Value};

/// Envelope version for every `--json` output:
/// `{ schema_version, command, data }`.
pub const SCHEMA_VERSION: u32 = 1;

/// Exit code for a subcommand whose analyzer (or the session store) is not
/// yet available, so an operator can script against it.
pub const UNAVAILABLE_EXIT_CODE: i32 = 3;

const HELP: &str = "autospec insights - inspect the continuous improvement engine (spec §44)

USAGE:
    autospec insights <SUBCOMMAND>

SUBCOMMANDS:
    ingest            Ingest historical agent sessions
    sessions          List stored insight sessions
    analyze           Run analysis over ingested sessions
    findings          List detected findings
    finding <id>      Show one finding
    models            Show model usage insights
    tools             Show tool usage insights
    context           Show context-window usage
    propose <finding> Create a proposal from a finding
    evaluate <proposal> Evaluate a proposal
    create-pr <proposal> Create a PR from a validated proposal
    report            Render the engine report

OPTIONS:
    Print help and exit 0:  -h, --help
    Wrap output in the versioned JSON envelope { schema_version, command, data }:  --json

EXIT CODES:
    0   success
    2   usage error (unknown subcommand or missing argument)
    3   requested analyzer or the session store is unavailable
";

/// The twelve §44 subcommands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsightsCommand {
    Ingest,
    Sessions,
    Analyze,
    Findings,
    Finding { id: String },
    Models,
    Tools,
    Context,
    Propose { finding: String },
    Evaluate { proposal: String },
    CreatePr { proposal: String },
    Report,
}

impl InsightsCommand {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ingest => "ingest",
            Self::Sessions => "sessions",
            Self::Analyze => "analyze",
            Self::Findings => "findings",
            Self::Finding { .. } => "finding",
            Self::Models => "models",
            Self::Tools => "tools",
            Self::Context => "context",
            Self::Propose { .. } => "propose",
            Self::Evaluate { .. } => "evaluate",
            Self::CreatePr { .. } => "create-pr",
            Self::Report => "report",
        }
    }
}

/// One rendered subcommand result, either as a text line or as a
/// versioned JSON envelope.
pub struct InsightsOutput {
    pub command: &'static str,
    pub data: Map<String, Value>,
    pub text: String,
    pub exit_code: i32,
}

/// Parse `autospec insights` arguments into a subcommand and the `--json`
/// switch. Unknown subcommands and missing positional arguments are usage
/// errors (the dispatch maps them to exit 2).
pub fn parse(args: &[String]) -> Result<(InsightsCommand, bool), String> {
    let mut json = false;
    let mut positional: Vec<&str> = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            other => positional.push(other),
        }
    }
    let (name, tail) = match positional.first().copied() {
        Some(name) => (name, &positional[1..]),
        None => {
            return Err(
                "insights: a subcommand is required (see `autospec insights --help`)".to_string(),
            )
        }
    };
    let command = match name {
        "ingest" => InsightsCommand::Ingest,
        "sessions" => InsightsCommand::Sessions,
        "analyze" => InsightsCommand::Analyze,
        "findings" => InsightsCommand::Findings,
        "finding" => InsightsCommand::Finding {
            id: positional_arg(name, tail)?,
        },
        "models" => InsightsCommand::Models,
        "tools" => InsightsCommand::Tools,
        "context" => InsightsCommand::Context,
        "propose" => InsightsCommand::Propose {
            finding: positional_arg(name, tail)?,
        },
        "evaluate" => InsightsCommand::Evaluate {
            proposal: positional_arg(name, tail)?,
        },
        "create-pr" => InsightsCommand::CreatePr {
            proposal: positional_arg(name, tail)?,
        },
        "report" => InsightsCommand::Report,
        unknown => return Err(format!("insights: unknown subcommand: {unknown}")),
    };
    Ok((command, json))
}

fn positional_arg(subcommand: &str, tail: &[&str]) -> Result<String, String> {
    match tail {
        [value] => Ok(value.to_string()),
        [] => Err(format!(
            "insights {subcommand}: missing required <id> argument"
        )),
        [first, extra @ ..] => Err(format!(
            "insights {subcommand}: expected exactly one argument, got {first} and {} more",
            extra.len()
        )),
    }
}

/// Render a subcommand result as a versioned JSON envelope or the named
/// text line.
pub fn render(output: &InsightsOutput, json: bool) -> String {
    if json {
        let envelope = json!({
            "schema_version": SCHEMA_VERSION,
            "command": output.command,
            "data": Value::Object(output.data.clone()),
        });
        format!("{envelope}\n")
    } else {
        format!("{}\n", output.text)
    }
}

/// Execute a parsed subcommand. `sessions` queries the stored session rows;
/// every analyzer arm reports `Unavailable` with exit code 3 until its
/// dedicated issue ships.
fn execute(command: &InsightsCommand) -> InsightsOutput {
    match command {
        InsightsCommand::Sessions => sessions(),
        other => unavailable(other.name()),
    }
}

/// The analyzer arms ship in their own issues; until then each reports a
/// named `Unavailable` status with the scriptable exit code.
fn unavailable(name: &'static str) -> InsightsOutput {
    let reason = format!("{name} analyzer is not yet implemented");
    let data = Map::from_iter([
        ("status".to_string(), json!("unavailable")),
        ("reason".to_string(), json!(reason)),
    ]);
    InsightsOutput {
        command: name,
        data,
        text: format!("insights {name}: unavailable: {reason}"),
        exit_code: UNAVAILABLE_EXIT_CODE,
    }
}

/// `sessions` lists the stored session rows of the insights pool
/// (issue #3827). Until the pool ships, the store query fails closed and
/// this arm reports the named reason instead of panicking.
fn sessions() -> InsightsOutput {
    let command = "sessions";
    match query_sessions() {
        Ok(rows) => {
            let count = rows.len();
            InsightsOutput {
                command,
                data: Map::from_iter([
                    ("status".to_string(), json!("ok")),
                    ("rows".to_string(), Value::Array(rows)),
                ]),
                text: format!("insights sessions: {count} stored session(s)"),
                exit_code: 0,
            }
        }
        Err(reason) => {
            let data = Map::from_iter([
                ("status".to_string(), json!("unavailable")),
                ("reason".to_string(), json!(reason)),
            ]);
            InsightsOutput {
                command,
                data,
                text: format!("insights {command}: unavailable: {reason}"),
                exit_code: UNAVAILABLE_EXIT_CODE,
            }
        }
    }
}

/// Query the stored session rows of the insights pool (issue #3827).
///
/// The pool has not shipped in this workspace yet, so the query fails
/// closed with a named reason. The `sessions` arm reports that reason
/// instead of panicking; no DSN or payload body is ever echoed.
fn query_sessions() -> Result<Vec<Value>, String> {
    Err("the stored session pool (issue #3827) is not wired into this build".to_string())
}

/// Run the `autospec insights` command surface. `Ok(code)` is the exit
/// code (0 success, 3 unavailable); `Err(message)` is a usage error the
/// dispatch maps to exit 2.
pub fn run(args: &[String]) -> Result<i32, String> {
    if args
        .iter()
        .any(|arg| arg.as_str() == "--help" || arg.as_str() == "-h")
    {
        println!("{HELP}");
        return Ok(0);
    }
    let (command, json) = parse(args)?;
    let output = execute(&command);
    print!("{}", render(&output, json));
    Ok(output.exit_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arg(value: &str) -> String {
        value.to_string()
    }

    const ALL_SUBCOMMANDS: [&str; 12] = [
        "ingest",
        "sessions",
        "analyze",
        "findings",
        "finding",
        "models",
        "tools",
        "context",
        "propose",
        "evaluate",
        "create-pr",
        "report",
    ];

    #[test]
    fn help_lists_all_twelve_subcommands() {
        for name in ALL_SUBCOMMANDS {
            assert!(HELP.contains(name), "help must list the {name} subcommand");
        }
    }

    #[test]
    fn parses_all_twelve_subcommands() {
        let no_arg: Vec<(&str, InsightsCommand)> = vec![
            ("ingest", InsightsCommand::Ingest),
            ("sessions", InsightsCommand::Sessions),
            ("analyze", InsightsCommand::Analyze),
            ("findings", InsightsCommand::Findings),
            ("models", InsightsCommand::Models),
            ("tools", InsightsCommand::Tools),
            ("context", InsightsCommand::Context),
            ("report", InsightsCommand::Report),
        ];
        for (name, expected) in no_arg {
            let (command, _) = parse(&[arg(name)]).expect("parses");
            assert_eq!(command, expected, "subcommand {name}");
        }
        assert_eq!(
            parse(&[arg("finding"), arg("f-1")]).unwrap().0,
            InsightsCommand::Finding { id: "f-1".into() }
        );
        assert_eq!(
            parse(&[arg("propose"), arg("f-2")]).unwrap().0,
            InsightsCommand::Propose {
                finding: "f-2".into()
            }
        );
        assert_eq!(
            parse(&[arg("evaluate"), arg("p-1")]).unwrap().0,
            InsightsCommand::Evaluate {
                proposal: "p-1".into()
            }
        );
        assert_eq!(
            parse(&[arg("create-pr"), arg("p-2")]).unwrap().0,
            InsightsCommand::CreatePr {
                proposal: "p-2".into()
            }
        );
    }

    #[test]
    fn json_flag_is_detected() {
        let (command, json) = parse(&[arg("sessions"), arg("--json")]).unwrap();
        assert_eq!(command, InsightsCommand::Sessions);
        assert!(json, "the json switch must be reported");
    }

    #[test]
    fn unknown_subcommand_is_a_usage_error() {
        let error = parse(&[arg("bogus")]).unwrap_err();
        assert!(
            error.contains("unknown subcommand"),
            "error must name the failure: {error}"
        );
    }

    #[test]
    fn missing_positional_is_a_usage_error() {
        for subcommand in ["finding", "propose", "evaluate", "create-pr"] {
            let error = parse(&[arg(subcommand)]).unwrap_err();
            assert!(
                error.contains(subcommand),
                "error must name the subcommand: {error}"
            );
        }
    }

    #[test]
    fn run_help_exits_zero() {
        assert_eq!(run(&[arg("--help")]).unwrap(), 0);
    }

    #[test]
    fn run_unknown_subcommand_is_a_usage_error() {
        assert!(run(&[arg("bogus")]).is_err());
    }

    #[test]
    fn unimplemented_analyzers_exit_three() {
        for subcommand in [
            "ingest", "analyze", "findings", "models", "tools", "context", "report",
        ] {
            let code = run(&[arg(subcommand)]).expect("unavailable is not an error");
            assert_eq!(code, UNAVAILABLE_EXIT_CODE, "subcommand {subcommand}");
        }
    }

    #[test]
    fn sessions_json_envelope_is_versioned() {
        let output = execute(&InsightsCommand::Sessions);
        let rendered = render(&output, true);
        let value: Value = serde_json::from_str(&rendered).expect("envelope parses as JSON");
        assert_eq!(
            value["schema_version"],
            json!(SCHEMA_VERSION),
            "envelope must carry schema_version"
        );
        assert_eq!(value["command"], "sessions");
        assert!(value.get("data").is_some(), "envelope must carry data");
    }

    #[test]
    fn sessions_reports_unavailable_store_without_panicking() {
        let output = execute(&InsightsCommand::Sessions);
        assert_eq!(output.exit_code, UNAVAILABLE_EXIT_CODE);
        assert!(output.text.contains("sessions"), "{}", output.text);
    }

    #[test]
    fn unavailable_envelope_is_versioned() {
        let output = execute(&InsightsCommand::Models);
        let rendered = render(&output, true);
        let value: Value = serde_json::from_str(&rendered).expect("envelope parses as JSON");
        assert_eq!(
            value["schema_version"],
            json!(SCHEMA_VERSION),
            "envelope must carry schema_version"
        );
        assert_eq!(value["command"], "models");
        assert_eq!(value["data"]["status"], "unavailable");
        assert!(
            value["data"]["reason"].as_str().is_some(),
            "the unavailable status must carry a named reason"
        );
    }

    #[test]
    fn text_render_is_the_named_message() {
        let output = execute(&InsightsCommand::Ingest);
        let rendered = render(&output, false);
        assert!(rendered.contains("ingest"), "{rendered}");
    }
}
