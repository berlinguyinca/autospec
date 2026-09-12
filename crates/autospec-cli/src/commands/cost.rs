//! `autospec cost` — GPU-hour accounting over the run records under `out/`.
//!
//! Two surfaces:
//!
//! - the report (`autospec cost [options]`): runs, GPU-hours and share by
//!   terminal status, rework separated from productive hours, known-defect
//!   costs, threshold flags;
//! - the pre-redispatch archive (`autospec cost archive <issue>`): the
//!   default a re-dispatch performs instead of `rm -rf` on the run
//!   directory, so cost accounting is computed from immutable per-run
//!   records (issue #3940).

use std::path::Path;

use autospec_core::cost::{archive_run, parse_iso8601, scan_out_dir, summarize};

const HELP: &str = "\
autospec cost — account GPU-hours by terminal status

Usage: autospec cost [options]
       autospec cost archive [--out-dir DIR] <issue>

Report options:
  --out-dir DIR              directory containing run subdirectories (default: out)
  --since ISO-8601           report the window of runs finished (or started) at or
                             after this instant, alongside the cumulative total
  --threshold-percent N      flag any status whose share of GPU-hours strictly
                             exceeds N percent (default: 10.0)
  --json                     print the report as a JSON object
  -h, --help                 show this help

Archive subcommand:
  autospec cost archive [--out-dir DIR] <issue>
                         archive <issue>'s current run directory into
                         <out-dir>/archive/<issue>/run-N before a re-dispatch
                         overwrites it. Prints where it moved (or that there
                         was nothing to move). Exits 2 when the archive
                         destination already exists: a refused archive must
                         stop the re-dispatch, not be worked around.

The report always exits 0 when it can read the directory; threshold flags are
report content, not failures.";

struct Options {
    out_dir: String,
    since: Option<String>,
    threshold_percent: f64,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut out_dir = String::from("out");
    let mut since: Option<String> = None;
    let mut threshold_percent = 10.0_f64;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        let value = |i: &mut usize, name: &str| -> Result<String, String> {
            *i += 1;
            args.get(*i)
                .cloned()
                .ok_or_else(|| format!("{name} takes a value"))
        };
        match arg.as_str() {
            "--out-dir" => out_dir = value(&mut i, "--out-dir")?,
            "--since" => {
                let raw = value(&mut i, "--since")?;
                parse_iso8601(&raw).map_err(|e| format!("--since: {e}"))?;
                since = Some(raw);
            }
            "--threshold-percent" => {
                let raw = value(&mut i, "--threshold-percent")?;
                let parsed: f64 = raw
                    .parse()
                    .map_err(|_| format!("--threshold-percent expects a number, got {raw}"))?;
                if !parsed.is_finite() || parsed < 0.0 || parsed > 100.0 {
                    return Err(format!(
                        "--threshold-percent expects a percentage in [0, 100], got {raw}"
                    ));
                }
                threshold_percent = parsed;
            }
            // Output mode is read from the raw args via super::is_json.
            "--json" => {}
            other => {
                return Err(format!("unknown argument: {other} (see --help)"));
            }
        }
        i += 1;
    }
    Ok(Options {
        out_dir,
        since,
        threshold_percent,
    })
}

pub fn run(args: &[String]) -> Result<(), String> {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("{HELP}");
        return Ok(());
    }
    if args.first().is_some_and(|arg| arg == "archive") {
        return run_archive(&args[1..]);
    }
    let options = parse(args)?;
    let root = std::env::current_dir().map_err(|e| format!("current directory: {e}"))?;
    let out_path = root.join(&options.out_dir);
    let scan = scan_out_dir(Path::new(&out_path))?;
    let since = options.since.map(|raw| {
        let at = parse_iso8601(&raw).expect("validated during argument parsing");
        (at, raw)
    });
    let report = summarize(&options.out_dir, &scan, since, options.threshold_percent);
    if super::is_json(args) {
        let body = report.to_json();
        let fields = body
            .strip_prefix('{')
            .ok_or_else(|| "cost report JSON must be an object".to_string())?;
        println!("{{\"command\":\"cost\",{fields}");
    } else {
        print!("{}", report.to_text());
    }
    Ok(())
}

/// The pre-redispatch archive: the default a re-dispatch performs instead of
/// `rm -rf` on `out/<issue>` (#3940). A refused archive is an `Err` (exit
/// 2), so the re-dispatch chain (`archive && rm -rf`) stops instead of
/// destroying a record twice.
fn run_archive(args: &[String]) -> Result<(), String> {
    let mut out_dir = String::from("out");
    let mut issue: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "--out-dir" => {
                i += 1;
                out_dir = args
                    .get(i)
                    .cloned()
                    .ok_or_else(|| "--out-dir takes a value".to_string())?;
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown argument: {other} (see --help)"));
            }
            other => {
                if issue.is_some() {
                    return Err(format!(
                        "unexpected argument {other:?}: cost archive takes exactly one <issue> name"
                    ));
                }
                issue = Some(other.to_string());
            }
        }
        i += 1;
    }
    let issue =
        issue.ok_or_else(|| "cost archive expects an <issue> name (see --help)".to_string())?;
    let root = std::env::current_dir().map_err(|e| format!("current directory: {e}"))?;
    let out_path = root.join(&out_dir);
    match archive_run(&out_path, &issue)? {
        Some(dest) => {
            let relative: &Path = dest.strip_prefix(&root).unwrap_or(&dest);
            println!("archived {issue} -> {}", relative.display());
        }
        None => println!("fresh: no previous run for {issue} under {out_dir}; nothing to archive"),
    }
    Ok(())
}
