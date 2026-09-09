//! `autospec dispatch authority` — verify the spec set in force before
//! dispatching against it (#3947).
//!
//! The dispatch pipeline knows which *issue* it is implementing and never
//! which *program*. This subcommand reads the spec documents a run intends to
//! implement and answers one question: is this the current authority? It
//! refuses (exit 1) when a document declares no currency at all, when the set
//! is superseded, when two sets claim the same component, or when the current
//! authority for the requested components is not unique. Throughput, when task
//! records are supplied, is reported per authority and never as one aggregate
//! number, because a large merge count aimed at a superseded architecture is
//! the most expensive kind of success.
//!
//! Exit codes: 0 current authority, 1 refused, 2 unusable input.

use std::fs;
use std::path::{Path, PathBuf};

use autospec_core::spec_authority::{
    self, DispatchVerdict, SpecDocument, SpecSet, ThroughputReport, UNDETERMINED,
};

use super::CommandFailure;

/// The spec set in force; dispatch may proceed.
const OK_EXIT: i32 = 0;
/// The spec set is not established as current; dispatch must not proceed.
const REFUSED_EXIT: i32 = 1;

/// Task records plus the lines that could not be read as records. A dropped
/// row would silently change a merge count, so problems travel with the
/// report and are printed.
struct Tasks {
    throughput: Option<ThroughputReport>,
    problems: Vec<String>,
}

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print_help();
        return Ok(());
    }

    let sources = spec_sources(args)?;
    let documents = read_documents(&sources)?;
    if documents.is_empty() {
        return Err(CommandFailure::diagnostic(format!(
            "no markdown documents found under {}",
            sources.join(", ")
        )));
    }

    let set = SpecSet::new(documents);
    let verdict = set.dispatch_verdict(&opt_list(args, "--component"));
    let tasks = load_tasks(args, &set)?;

    if is_json(args) {
        print_json(&set, &verdict, &tasks);
    } else {
        print_text(&set, &verdict, &tasks);
    }

    if verdict.allowed {
        Ok(())
    } else {
        Err(CommandFailure::status(
            format!(
                "dispatch refused: {}",
                verdict
                    .blocking
                    .first()
                    .map(|finding| finding.code.as_str())
                    .unwrap_or("AUTHORITY_AMBIGUOUS")
            ),
            REFUSED_EXIT,
        ))
    }
}

pub fn print_help() {
    println!("USAGE: autospec dispatch authority [options]");
    println!();
    println!("Verify the spec set a dispatch would implement against is current.");
    println!("A superseded set, a set with no currency marker, and a component two");
    println!("sets both claim all refuse the dispatch.");
    println!();
    println!("SUBCOMMANDS:");
    println!("    authority             This gate (no nested subcommand; flags only)");
    println!();
    println!("OPTIONS:");
    println!("    --spec-dir <DIR>      Directory of spec documents (repeatable)");
    println!("    --spec-file <PATH>    One spec document (repeatable)");
    println!("    --component <NAME>    Component the run intends to touch (repeatable)");
    println!("    --tasks <PATH>        Task records: task<TAB>authority<TAB>outcome");
    println!("    --json                Machine-readable report");
    println!();
    println!("EXIT: 0 current authority / 1 refused / 2 unusable input");
}

/// Where the spec documents live. Explicit sources win; with none given the
/// repository's own `docs/specs` is used, and a missing default is an error
/// rather than a silently empty spec set — an empty set would read as "nothing
/// is stale".
fn spec_sources(args: &[String]) -> Result<Vec<String>, CommandFailure> {
    let mut collected = Vec::new();
    for index in 0..args.len() {
        if matches!(args[index].as_str(), "--spec-dir" | "--spec-file") {
            let value = args.get(index + 1).ok_or_else(|| {
                CommandFailure::diagnostic(format!("flag needs a value: {}", args[index]))
            })?;
            collected.push(value.clone());
        }
    }
    if collected.is_empty() {
        let default = Path::new("docs/specs");
        if default.is_dir() {
            return Ok(vec![default.to_string_lossy().into_owned()]);
        }
        return Err(CommandFailure::diagnostic(format!(
            "no spec source given and the default {} does not exist; pass --spec-dir DIR or --spec-file PATH",
            default.display()
        )));
    }
    Ok(collected)
}

/// Reads every document under the given files/directories.
fn read_documents(sources: &[String]) -> Result<Vec<SpecDocument>, CommandFailure> {
    let mut documents = Vec::new();
    for source in sources {
        let path = PathBuf::from(source);
        if path.is_dir() {
            let mut found = Vec::new();
            collect_markdown(&path, &mut found)?;
            found.sort();
            for file in found {
                documents.push(parse_file(&file)?);
            }
        } else if path.is_file() {
            documents.push(parse_file(&path)?);
        } else {
            return Err(CommandFailure::diagnostic(format!(
                "spec source {source} does not exist"
            )));
        }
    }
    Ok(documents)
}

fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), CommandFailure> {
    let entries = fs::read_dir(dir)
        .map_err(|error| CommandFailure::diagnostic(format!("{dir:?}: {error}")))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| CommandFailure::diagnostic(format!("{dir:?}: {error}")))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| CommandFailure::diagnostic(format!("{path:?}: {error}")))?;
        if file_type.is_dir() {
            collect_markdown(&path, out)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            out.push(path);
        }
    }
    Ok(())
}

fn parse_file(path: &Path) -> Result<SpecDocument, CommandFailure> {
    let text = fs::read_to_string(path)
        .map_err(|error| CommandFailure::diagnostic(format!("cannot read {path:?}: {error}")))?;
    Ok(spec_authority::parse_document(
        &path.to_string_lossy(),
        &text,
    ))
}

/// Reads `--tasks` records and groups them by authority against `set`, so a
/// merge count carries the currency of the set it was aimed at.
fn load_tasks(args: &[String], set: &SpecSet) -> Result<Tasks, CommandFailure> {
    let Some(path) = opt_string(args, "--tasks")? else {
        return Ok(Tasks {
            throughput: None,
            problems: Vec::new(),
        });
    };
    let text = fs::read_to_string(&path).map_err(|error| {
        CommandFailure::diagnostic(format!("cannot read task records {path}: {error}"))
    })?;
    let (records, problems) = spec_authority::parse_task_records(&text);
    Ok(Tasks {
        throughput: Some(spec_authority::throughput(&records, set)),
        problems,
    })
}

fn print_text(set: &SpecSet, verdict: &DispatchVerdict, tasks: &Tasks) {
    println!("SPEC SETS");
    for authority in set.authorities() {
        let status = set.status_of(&authority);
        let documents = set
            .documents()
            .iter()
            .filter(|document| document.authority == authority)
            .count();
        println!(
            "  {authority:<24} {:<10} {documents} document(s)",
            status.as_str()
        );
    }

    let conflicts = set.conflicts();
    if !conflicts.is_empty() {
        println!();
        println!("CONFLICTS");
        for conflict in &conflicts {
            println!(
                "  {}: claimed by {}",
                conflict.component,
                conflict.authorities.join(", ")
            );
        }
    }

    if !verdict.blocking.is_empty() || !verdict.advisories.is_empty() {
        println!();
        println!("FINDINGS");
        for finding in verdict.blocking.iter().chain(verdict.advisories.iter()) {
            let severity = if verdict.blocking.contains(finding) {
                "BLOCK"
            } else {
                "ADVISORY"
            };
            println!(
                "  {severity} {}: {}: {}",
                finding.code.as_str(),
                finding.subject,
                finding.message
            );
        }
    }

    if let Some(throughput) = &tasks.throughput {
        println!();
        println!("THROUGHPUT BY SPEC AUTHORITY");
        println!(
            "  {:<24} {:>6} {:>6} {:>6} {:>6}",
            "authority", "merged", "open", "failed", "total"
        );
        for row in &throughput.rows {
            println!(
                "  {:<24} {:>6} {:>6} {:>6} {:>6}",
                row.authority, row.merged, row.open, row.failed, row.total
            );
        }
        println!("  {:<24} {:>6}", "merges", throughput.merged_total());
        for warning in &throughput.warnings {
            println!("  WARNING: {warning}");
        }
    }
    for problem in &tasks.problems {
        println!("  MALFORMED: {problem}");
    }

    println!();
    if verdict.allowed {
        let authority = verdict
            .authority
            .clone()
            .unwrap_or_else(|| UNDETERMINED.to_string());
        println!("ALLOWED: spec authority {authority} is current (exit {OK_EXIT})");
    } else {
        println!("REFUSED: no current spec authority for this dispatch (exit {REFUSED_EXIT})");
    }
}

fn print_json(set: &SpecSet, verdict: &DispatchVerdict, tasks: &Tasks) {
    let payload = serde_json::json!({
        "spec_sets": set.authorities().into_iter().map(|authority| serde_json::json!({
            "authority": authority,
            "status": set.status_of(&authority).as_str(),
        })).collect::<Vec<_>>(),
        "conflicts": set.conflicts(),
        "verdict": verdict,
        "throughput": tasks.throughput,
        "task_record_problems": tasks.problems,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
    );
}

fn is_json(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--json")
}

fn opt_string(args: &[String], flag: &str) -> Result<Option<String>, CommandFailure> {
    let Some(index) = args.iter().position(|arg| arg == flag) else {
        return Ok(None);
    };
    let value = args
        .get(index + 1)
        .ok_or_else(|| CommandFailure::diagnostic(format!("flag needs a value: {flag}")))?;
    Ok(Some(value.clone()))
}

/// Repeatable option: every occurrence is collected, in order.
fn opt_list(args: &[String], flag: &str) -> Vec<String> {
    let mut values = Vec::new();
    for index in 0..args.len() {
        if args[index] == flag {
            if let Some(value) = args.get(index + 1) {
                values.push(value.clone());
            }
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &[&str]) -> Vec<String> {
        text.iter().map(|arg| arg.to_string()).collect()
    }

    /// A repeated option collects every occurrence in order; a dispatch that
    /// touches three components must have all three checked, not the last.
    #[test]
    fn repeatable_options_collect_every_value() {
        let parsed = args(&["--component", "gateway", "--other", "--component", "edge"]);
        assert_eq!(opt_list(&parsed, "--component"), vec!["gateway", "edge"]);
    }

    /// An option with no value is an input error rather than an empty string:
    /// an empty component list means "check the whole set", which is a
    /// different question than a truncated argument.
    #[test]
    fn a_flag_without_its_value_is_an_input_error() {
        let parsed = args(&["--tasks"]);
        let error = opt_string(&parsed, "--tasks").expect_err("missing value must error");
        assert_eq!(error.exit_code, 2);
        let parsed = args(&["--spec-dir"]);
        let error = spec_sources(&parsed).expect_err("missing value must error");
        assert_eq!(error.exit_code, 2);
    }

    /// A source path that does not exist exits 2 rather than reporting a clean
    /// spec set: an empty set would read as "nothing is stale".
    #[test]
    fn a_missing_source_is_refused_as_input() {
        let parsed = args(&["--spec-dir", "/nonexistent/spec-set-3947"]);
        let error = run(&parsed).expect_err("missing source must error");
        assert_eq!(error.exit_code, 2, "{:?}", error.message);
        assert!(
            error.message.contains("does not exist"),
            "{}",
            error.message
        );
    }

    /// The help text documents every flag and the exit codes, which is the
    /// only explanation an operator gets when a dispatch refuses.
    #[test]
    fn help_documents_the_flags_and_exit_codes() {
        print_help();
        let parsed = args(&["--help"]);
        assert!(run(&parsed).is_ok());
    }
}
