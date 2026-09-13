//! `autospec issue-skeleton` — render a structured YAML input into a
//! team-lensed issue body.
//!
//! CLI front-end for [`autospec_core::issue_skeleton`]. This is the Rust
//! replacement for `scripts/gen-issue-skeleton.sh` (issue #4440). The shell
//! script is kept as a thin wrapper that delegates to this command for one
//! release.
//!
//! Exit-code contract (matches the shell):
//! - `0` — rendered body printed to stdout; no blocking lint findings.
//! - `1` — a required field is missing (`MISSING_FIELD:<key>` on stderr) or
//!   the YAML does not parse.
//! - `N` — `N` blocking lint findings (printed to stderr); nothing on stdout.

use std::io::{IsTerminal, Read};

use autospec_core::issue_skeleton;

fn input_from_args(args: &[String]) -> Result<String, String> {
    let mut input_path: Option<&str> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--input" => {
                let value = args
                    .get(index + 1)
                    .ok_or("--input requires a value")?;
                input_path = Some(value);
                index += 2;
            }
            flag if flag.starts_with("--input=") => {
                input_path = Some(flag.trim_start_matches("--input="));
                index += 1;
            }
            flag if flag == "--help" || flag == "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    match input_path {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read {path}: {error}")),
        None => {
            // No `--input`: read the skeleton from stdin. If stdin is an
            // interactive terminal (nothing piped in), reading would block, so
            // surface usage instead — mirroring how `git log` and friends
            // behave with no input.
            if std::io::stdin().is_terminal() {
                print_help();
                return Err("no --input given and stdin is a terminal; pipe YAML or use --input <file>".to_string());
            }
            let mut buffer = String::new();
            std::io::stdin()
                .read_to_string(&mut buffer)
                .map_err(|error| format!("cannot read stdin: {error}"))?;
            Ok(buffer)
        }
    }
}

fn print_help() {
    println!(
        "autospec issue-skeleton\n\n\
         USAGE:\n    autospec issue-skeleton --input <file>\n    \
         autospec issue-skeleton < input.yaml\n\n\
         Render a structured YAML issue-skeleton into a team-lensed issue body.\n\n\
         OPTIONS:\n    --input <file>   Read the skeleton from <file> (default: stdin)\n\n\
         OUTPUT:\n    The issue body on stdout (only when lint passes).\n\n\
         EXIT CODES:\n    0   rendered body printed; no blocking lint findings\n    \
         1   a required field is missing, or the YAML does not parse\n    N   N \
         blocking lint findings\n"
    );
}

/// Status-code shape: `Ok(0)` succeeds, `Ok(N)` is a non-zero exit, `Err` is a
/// diagnostic.
pub fn run(args: &[String]) -> Result<i32, String> {
    let yaml = input_from_args(args)?;
    let skeleton = match issue_skeleton::parse(&yaml) {
        Ok(skeleton) => skeleton,
        Err(message) => {
            eprintln!("{message}");
            return Ok(1);
        }
    };

    let findings = skeleton.lint_findings();
    let blocking: Vec<_> = findings
        .iter()
        .filter(|finding| finding.is_blocking())
        .collect();

    if !blocking.is_empty() {
        for finding in &blocking {
            eprintln!("{}: {}", finding.rule.id(), finding.message);
        }
        return Ok(blocking.len() as i32);
    }

    // Non-blocking warnings are reported on stderr but never affect the exit.
    for finding in &findings {
        if !finding.is_blocking() {
            eprintln!("{}: {}", finding.rule.id(), finding.message);
        }
    }

    let body = skeleton.render();
    println!("{body}");
    Ok(0)
}
