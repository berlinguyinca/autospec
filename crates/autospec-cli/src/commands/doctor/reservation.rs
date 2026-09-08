//! `autospec doctor reservation` — agent budget versus reservation walltime.
//!
//! The runner's fixed 45-minute LIMIT used to kill every agent run long before
//! the 8-hour Slurm reservation it sat inside ended, and the building patch
//! went with it (issue #3690). This subcommand is the pre-start check: derive
//! the agent budget from the reservation walltime, and refuse to start when a
//! requested budget does not fit. The refusal travels in the exit code so a
//! monitor or launcher can gate on it without scraping the text.

use autospec_core::reservation_budget::{
    derive_agent_budget, format_duration, parse_walltime, validate_budget, MIN_AGENT_BUDGET_SECS,
    RESERVATION_OVERHEAD_SECS,
};

/// What the caller renders and which exit code to leave by.
pub struct Outcome {
    pub rendered: String,
    /// True when the reservation cannot host the requested agent budget. The
    /// caller exits 1 on this so a launcher refuses to start the run.
    pub refused: bool,
}

/// Exit code when the requested budget does not fit the reservation.
pub const REFUSED_EXIT_CODE: i32 = 1;

const USAGE: &str = "autospec doctor reservation --walltime SECS|MM:SS|HH:MM:SS [--budget SECS] [--json]\n\n\
Check an agent budget against a Slurm reservation's walltime before the run\n\
starts (issue #3690). The agent budget is derived as walltime minus startup/\nteardown overhead; a reservation that would leave less than the minimum\n\
useful budget is refused, and an explicit --budget that does not fit the\n\
reservation is refused too.\n\n\
OPTIONS:\n\
    --walltime WALLTIME  reservation walltime in seconds, MM:SS or HH:MM:SS (required)\n\
    --budget SECS        requested agent budget in seconds (validated against the walltime)\n\
    --json               emit the report as JSON\n\n\
EXIT CODES:\n\
    0  the budget fits (or only the derived budget was requested and it is usable)\n\
    1  refused: reservation too small, or budget does not fit\n\
    2  bad arguments";

/// Machine-readable reservation/budget report.
struct ReservationReport {
    walltime_secs: u64,
    walltime: String,
    overhead_secs: u64,
    minimum_agent_budget_secs: u64,
    /// `None` when the reservation is too small to derive a budget.
    derived_budget_secs: Option<u64>,
    requested_budget_secs: Option<u64>,
    /// Maximum agent budget the walltime can host.
    max_budget_secs: Option<u64>,
    status: &'static str,
    reason: Option<String>,
}

impl ReservationReport {
    fn to_json(&self) -> String {
        serde_json::json!({
            "command": "doctor",
            "subcommand": "reservation",
            "walltime_secs": self.walltime_secs,
            "walltime": self.walltime,
            "overhead_secs": self.overhead_secs,
            "minimum_agent_budget_secs": self.minimum_agent_budget_secs,
            "derived_budget_secs": self.derived_budget_secs,
            "requested_budget_secs": self.requested_budget_secs,
            "max_budget_secs": self.max_budget_secs,
            "status": self.status,
            "reason": self.reason,
        })
        .to_string()
    }
}

pub fn run(args: &[String]) -> Result<Outcome, String> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Ok(Outcome {
            rendered: USAGE.to_string(),
            refused: false,
        });
    }

    let options = parse(args)?;
    let walltime_secs = parse_walltime(&options.walltime)
        .map_err(|error| format!("invalid --walltime {}: {error}", options.walltime))?;

    let derived = derive_agent_budget(walltime_secs);
    let (status, reason, max_budget_secs) =
        evaluate(walltime_secs, derived.as_ref().ok(), options.budget_secs);
    let report = ReservationReport {
        walltime_secs,
        walltime: format_duration(walltime_secs),
        overhead_secs: RESERVATION_OVERHEAD_SECS,
        minimum_agent_budget_secs: MIN_AGENT_BUDGET_SECS,
        derived_budget_secs: derived.as_ref().ok().map(|budget| budget.budget_secs),
        requested_budget_secs: options.budget_secs,
        max_budget_secs,
        status,
        reason,
    };
    let rendered = if options.json {
        report.to_json()
    } else {
        render_text(&report)
    };
    Ok(Outcome {
        rendered,
        refused: status == "refused",
    })
}

/// The verdict: `ok` when the reservation hosts the budget (or only the
/// derived budget was requested and it is usable), `refused` otherwise.
fn evaluate(
    walltime_secs: u64,
    derived: Option<&autospec_core::reservation_budget::AgentBudget>,
    requested: Option<u64>,
) -> (&'static str, Option<String>, Option<u64>) {
    let Some(budget) = derived else {
        let error = derive_agent_budget(walltime_secs).unwrap_err();
        return ("refused", Some(error.to_string()), None);
    };
    let Some(requested) = requested else {
        return ("ok", None, Some(budget.budget_secs));
    };
    match validate_budget(walltime_secs, requested) {
        Ok(()) => ("ok", None, Some(budget.budget_secs)),
        Err(error) => ("refused", Some(error.to_string()), Some(budget.budget_secs)),
    }
}

fn render_text(report: &ReservationReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "reservation walltime: {}s ({})\n",
        report.walltime_secs, report.walltime
    ));
    out.push_str(&format!(
        "overhead (startup + teardown): {}s\n",
        report.overhead_secs
    ));
    match report.derived_budget_secs {
        Some(budget_secs) => out.push_str(&format!(
            "derived agent budget: {}s ({})\n",
            budget_secs,
            format_duration(budget_secs)
        )),
        None => out.push_str("derived agent budget: none\n"),
    }
    match (report.status, report.requested_budget_secs) {
        ("ok", Some(requested)) => out.push_str(&format!(
            "requested budget {}s fits (max {}s)\n",
            requested,
            report.max_budget_secs.unwrap_or(requested)
        )),
        ("refused", _) => out.push_str(&format!(
            "REFUSED: {}\n",
            report.reason.as_deref().unwrap_or("budget does not fit")
        )),
        _ => {}
    }
    out
}

struct Options {
    walltime: String,
    budget_secs: Option<u64>,
    json: bool,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        walltime: String::new(),
        budget_secs: None,
        json: false,
    };
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        match flag {
            "--walltime" => options.walltime = take_value(args, &mut index, flag)?,
            "--budget" => {
                let raw = take_value(args, &mut index, flag)?;
                options.budget_secs = Some(parse_budget(raw)?);
            }
            "--json" => options.json = true,
            other => return Err(format!("unknown doctor reservation option: {other}")),
        }
        index += 1;
    }
    if options.walltime.is_empty() {
        return Err("--walltime is required".to_string());
    }
    Ok(options)
}

fn parse_budget(raw: String) -> Result<u64, String> {
    let value: u64 = raw
        .trim()
        .parse()
        .map_err(|_| format!("--budget expects non-negative seconds, got {raw}"))?;
    Ok(value)
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}
