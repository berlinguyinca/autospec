//! `autospec dispatch-outcomes` — dispatch outcome attribution report (#4025).

use autospec_core::dispatch_outcomes::{outcome_report, DispatchLedger, DEFAULT_MIN_SAMPLES};
use std::path::PathBuf;

const HELP: &str = "\
autospec dispatch-outcomes — dispatch outcome attribution report

Usage: autospec dispatch-outcomes [options]

Options:
  --file PATH                dispatch-outcomes ledger (default: $HOME/.autospec/dispatch-outcomes.jsonl)
  --min-samples N            minimum decided outcomes before a rate is shown (default: 10)
  --json                     print the report as a JSON object
  -h, --help                 show this help

Conversion rate per model and per (model x spec size band), with the sample
count on every row. Rows below the sample floor are reported as
`insufficient data (n=K)`, never as a rate. The ledger is an append-only
JSONL file, one record per dispatch, with the conversion result written back
to the record that produced the patch.";

struct Options {
    file: Option<String>,
    min_samples: u64,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        file: None,
        min_samples: DEFAULT_MIN_SAMPLES,
    };
    let mut i = 0;
    while i < args.len() {
        let value = |i: &mut usize, name: &str| -> Result<String, String> {
            *i += 1;
            args.get(*i)
                .cloned()
                .ok_or_else(|| format!("{name} takes a value"))
        };
        match args[i].as_str() {
            "--file" => options.file = Some(value(&mut i, "--file")?),
            "--min-samples" => {
                let raw = value(&mut i, "--min-samples")?;
                options.min_samples = raw.parse().map_err(|_| {
                    format!("--min-samples expects a non-negative integer, got {raw}")
                })?;
            }
            // Output mode is read from the raw args via super::is_json.
            "--json" => {}
            other => return Err(format!("unknown argument: {other} (see --help)")),
        }
        i += 1;
    }
    Ok(options)
}

fn ledger_path(file: &Option<String>) -> Result<PathBuf, String> {
    match file {
        Some(path) => Ok(PathBuf::from(path)),
        None => {
            let home =
                std::env::var("HOME").map_err(|_| "HOME is not set; pass --file".to_string())?;
            Ok(PathBuf::from(home).join(".autospec/dispatch-outcomes.jsonl"))
        }
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("{HELP}");
        return Ok(());
    }
    let options = parse(args)?;
    let path = ledger_path(&options.file)?;
    let records = DispatchLedger::open(path)
        .load()
        .map_err(|e| e.to_string())?;
    let report = outcome_report(&records, options.min_samples);
    if super::is_json(args) {
        let body = serde_json::to_string(&report).map_err(|e| e.to_string())?;
        let fields = body
            .strip_prefix('{')
            .ok_or_else(|| "dispatch-outcomes report JSON must be an object".to_string())?;
        println!("{{\"command\":\"dispatch-outcomes\",{fields}");
    } else {
        print!("{}", report.to_markdown());
    }
    Ok(())
}
