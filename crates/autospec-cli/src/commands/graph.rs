//! `autospec graph` — DAG analysis of a proposed issue set.
//!
//! Spec: `docs/specs/2026-09-08-parallel-decomposition-fleet-saturation.md`
//! — §19 (DAG analyzer), §22 (AS-DAG-004 fan-in threshold), §24 (execution
//! waves), §25 (planner output summary), §38 (planning telemetry).
//!
//! All traversal (cycle detection, waves, critical path, metric formulas)
//! lives in `autospec_core::graph`; this module only parses options and
//! input files, calls the core entry points, and renders the §19 JSON, the
//! §24 wave projection, and the §25 planner summary. Untrusted input
//! (malformed JSON, hostile graphs) surfaces as a diagnostic — never a
//! panic.

use std::fs;
use std::path::PathBuf;

use autospec_core::graph::{metrics, IssueGraph, PlannedIssue};
use serde::Serialize;

use super::CommandFailure;

/// §6.2 default fleet capacity.
const DEFAULT_CAPACITY: usize = 32;
/// §22 AS-DAG-004: "more than a configurable threshold. Default 5".
const HIGH_FAN_THRESHOLD: usize = 5;
/// §25 summary column width for the aligned label field.
const SUMMARY_LABEL_WIDTH: usize = 23;
/// §24 example wraps issue id lines after 9 ids.
const WAVE_IDS_PER_LINE: usize = 9;

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    match args {
        [] => Err(CommandFailure::diagnostic(
            "autospec graph requires a subcommand (analyze)",
        )),
        [flag] if flag == "--help" || flag == "-h" => {
            print_help();
            Ok(())
        }
        [command, rest @ ..] if command == "analyze" => run_analyze(rest),
        [command, ..] => Err(CommandFailure::diagnostic(format!(
            "unknown autospec graph command: {command}"
        ))),
    }
}

fn run_analyze(args: &[String]) -> Result<(), CommandFailure> {
    if args
        .first()
        .is_some_and(|flag| flag == "--help" || flag == "-h")
    {
        print_help();
        return Ok(());
    }
    let options = parse_options(args)?;
    let source = fs::read_to_string(&options.input).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not read input file {}: {error}",
            options.input.display()
        ))
    })?;
    let graph = parse_input(&source)?;

    if let Some(cycle) = graph.detect_cycle() {
        return Err(CommandFailure::status(
            format!(
                "dependency cycle detected: {} -> {}",
                cycle.join(" -> "),
                cycle[0]
            ),
            3,
        ));
    }

    let capacity = options.capacity;
    let m = metrics(&graph, capacity);
    let waves = graph.waves().expect("cycle was checked before this call");
    let fan_in = count_fans(&graph, true);
    let fan_out = count_fans(&graph, false);

    match options.format {
        OutputFormat::Json => {
            let report = AnalyzeReport {
                issue_count: m.issue_count,
                hard_edge_count: m.hard_edge_count,
                root_count: m.root_count,
                leaf_count: m.leaf_count,
                critical_path_length: m.critical_path_length,
                maximum_width: m.maximum_width,
                initial_width: m.initial_width,
                average_wave_width: m.average_wave_width,
                serialization_ratio: m.serialization_ratio,
                shared_write_hotspots: m.shared_write_hotspots,
                high_fan_in_nodes: fan_in,
                high_fan_out_nodes: fan_out,
                estimated_fleet_saturation: Saturation {
                    capacity,
                    initial: m.initial_saturation,
                    peak: m.peak_saturation,
                },
                telemetry: Telemetry {
                    issue_count: m.issue_count,
                    hard_edge_count: m.hard_edge_count,
                    initial_width: m.initial_width,
                    maximum_width: m.maximum_width,
                    critical_path: m.critical_path_length,
                    parallelization_score: m.parallelization_score,
                    fleet_capacity: capacity,
                    initial_saturation: m.initial_saturation,
                    shared_write_hotspots: m.shared_write_hotspots,
                    // The analyzer does not run the §23 optimizer pass;
                    // those counters are zero for every analysis.
                    edges_removed_by_optimizer: 0,
                    issues_split_by_optimizer: 0,
                    issues_merged_by_optimizer: 0,
                },
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&report).map_err(|error| {
                    CommandFailure::diagnostic(format!(
                        "could not serialize analysis report: {error}"
                    ))
                })?
            );
        }
        OutputFormat::Text => {
            render_waves(&waves);
            render_summary(&m, capacity);
            render_telemetry(&m, capacity);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Json,
    Text,
}

#[derive(Debug, Clone)]
struct AnalyzeOptions {
    input: PathBuf,
    capacity: usize,
    format: OutputFormat,
}

fn parse_options(args: &[String]) -> Result<AnalyzeOptions, CommandFailure> {
    if args.len() % 2 != 0 {
        let dangling = args
            .last()
            .expect("odd-length args have a last element")
            .as_str();
        let message = if dangling.starts_with('-') {
            format!("autospec graph analyze: option {dangling} requires an argument")
        } else {
            format!("autospec graph analyze: unknown option: {dangling}")
        };
        return Err(CommandFailure::diagnostic(message));
    }
    let mut input = None;
    let mut capacity = DEFAULT_CAPACITY;
    let mut format = OutputFormat::Json;
    for pair in args.chunks_exact(2) {
        let option = pair[0].as_str();
        let value = pair[1].as_str();
        match option {
            "--input" if input.is_none() => input = Some(PathBuf::from(value)),
            "--capacity" if value.parse::<usize>().is_ok() => {
                capacity = value.parse().expect("validated above");
            }
            "--format" if value == "json" => format = OutputFormat::Json,
            "--format" if value == "text" => format = OutputFormat::Text,
            _ => {
                return Err(CommandFailure::diagnostic(format!(
                    "autospec graph analyze: unknown or invalid option {option} {value}"
                )));
            }
        }
    }
    let input = input.ok_or_else(|| {
        CommandFailure::diagnostic(
            "autospec graph analyze requires --input <PATH> (issue graph JSON or issue draft array)",
        )
    })?;
    Ok(AnalyzeOptions {
        input,
        capacity,
        format,
    })
}

/// Parse a proposed issue graph: either a full §28 `IssueGraph` object
/// (`{"issues": [...], "hard_edges": [...]}`) or a bare array of issue
/// drafts (no hard edges).
fn parse_input(source: &str) -> Result<IssueGraph, CommandFailure> {
    let value: serde_json::Value = serde_json::from_str(source)
        .map_err(|error| CommandFailure::diagnostic(format!("input is not valid JSON: {error}")))?;
    let graph = match value {
        serde_json::Value::Object(_) => {
            serde_json::from_value::<IssueGraph>(value).map_err(|error| {
                CommandFailure::diagnostic(format!("input is not a valid issue graph: {error}"))
            })?
        }
        serde_json::Value::Array(_) => {
            let issues: Vec<PlannedIssue> = serde_json::from_value(value).map_err(|error| {
                CommandFailure::diagnostic(format!(
                    "input array is not a valid issue draft list: {error}"
                ))
            })?;
            IssueGraph {
                issues,
                hard_edges: Vec::new(),
            }
        }
        _ => {
            return Err(CommandFailure::diagnostic(
                "input must be an issue graph object or an array of issue drafts",
            ))
        }
    };
    Ok(graph)
}

/// Count hard edges per node over the edge list (known ids only, matching
/// core's topology) and report every id whose fan-in or fan-out exceeds the
/// §22 AS-DAG-004 threshold of 5. A flat count over declared edges, not a
/// traversal — the walk itself stays in core.
fn count_fans(graph: &IssueGraph, fan_in: bool) -> Vec<String> {
    let known: std::collections::BTreeSet<&str> =
        graph.issues.iter().map(|issue| issue.id.as_str()).collect();
    let mut counts: std::collections::BTreeMap<&str, usize> =
        known.iter().map(|id| (*id, 0)).collect();
    for edge in &graph.hard_edges {
        let source = if fan_in {
            (edge.successor.as_str(), edge.predecessor.as_str())
        } else {
            (edge.predecessor.as_str(), edge.successor.as_str())
        };
        if known.contains(source.0) {
            *counts.get_mut(source.0).expect("known id was inserted") += 1;
        }
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count > HIGH_FAN_THRESHOLD)
        .map(|(id, _)| id.to_string())
        .collect()
}

/// §24 wave projection: one `Wave N — K issue(s)` header per wave, issue ids
/// wrapped at 9 per line.
fn render_waves(waves: &[Vec<String>]) {
    for (index, wave) in waves.iter().enumerate() {
        let unit = if wave.len() == 1 { "issue" } else { "issues" };
        println!("Wave {index} — {} {unit}", wave.len());
        for chunk in wave.chunks(WAVE_IDS_PER_LINE) {
            println!("{}", chunk.join(" "));
        }
        if index + 1 < waves.len() {
            println!();
        }
    }
}

/// §25 planner output summary (metrics-only: no Spec line, no
/// concurrency-review section — those belong to the optimizer pass).
fn render_summary_line(label: &str, value: &str) {
    println!("{label:<width$} {value}", width = SUMMARY_LABEL_WIDTH);
}

fn render_summary(m: &autospec_core::graph::GraphMetrics, capacity: usize) {
    println!();
    println!("AutoSpec decomposition complete");
    println!();
    println!("Fleet target: {capacity} agents");
    println!();
    render_summary_line("Issues created:", &m.issue_count.to_string());
    render_summary_line("Initially ready:", &m.initial_width.to_string());
    render_summary_line("Maximum projected ready:", &m.maximum_width.to_string());
    render_summary_line("Critical path:", &m.critical_path_length.to_string());
    render_summary_line("Hard dependencies:", &m.hard_edge_count.to_string());
    render_summary_line(
        "Shared-write hotspots:",
        &m.shared_write_hotspots.to_string(),
    );
    render_summary_line(
        "Parallelization score:",
        &format!("{}/100", m.parallelization_score),
    );
    println!();
    println!(
        "Projected initial utilization: {}/{} agents",
        m.initial_width, capacity
    );
}

/// §38 planning telemetry: the 12 `autospec.define` counters.
fn render_telemetry(m: &autospec_core::graph::GraphMetrics, capacity: usize) {
    println!();
    println!("Telemetry:");
    for (name, value) in telemetry_counters(m, capacity) {
        println!("autospec.define.{name}: {value}");
    }
}

fn telemetry_counters(
    m: &autospec_core::graph::GraphMetrics,
    capacity: usize,
) -> Vec<(&'static str, String)> {
    vec![
        ("issue_count", m.issue_count.to_string()),
        ("hard_edge_count", m.hard_edge_count.to_string()),
        ("initial_width", m.initial_width.to_string()),
        ("maximum_width", m.maximum_width.to_string()),
        ("critical_path", m.critical_path_length.to_string()),
        ("parallelization_score", m.parallelization_score.to_string()),
        ("fleet_capacity", capacity.to_string()),
        ("initial_saturation", format!("{:.3}", m.initial_saturation)),
        ("shared_write_hotspots", m.shared_write_hotspots.to_string()),
        ("edges_removed_by_optimizer", "0".to_string()),
        ("issues_split_by_optimizer", "0".to_string()),
        ("issues_merged_by_optimizer", "0".to_string()),
    ]
}

/// §19 report object. Field declaration order is the stable key order.
#[derive(Serialize)]
struct AnalyzeReport {
    issue_count: usize,
    hard_edge_count: usize,
    root_count: usize,
    leaf_count: usize,
    critical_path_length: usize,
    maximum_width: usize,
    initial_width: usize,
    average_wave_width: f64,
    serialization_ratio: f64,
    shared_write_hotspots: usize,
    high_fan_in_nodes: Vec<String>,
    high_fan_out_nodes: Vec<String>,
    estimated_fleet_saturation: Saturation,
    telemetry: Telemetry,
}

#[derive(Serialize)]
struct Saturation {
    capacity: usize,
    initial: f64,
    peak: f64,
}

/// The 12 `autospec.define` counters from §38, without the prefix (the
/// `telemetry` object provides it). Field declaration order is the stable
/// key order.
#[derive(Serialize)]
struct Telemetry {
    issue_count: usize,
    hard_edge_count: usize,
    initial_width: usize,
    maximum_width: usize,
    critical_path: usize,
    parallelization_score: u8,
    fleet_capacity: usize,
    initial_saturation: f64,
    shared_write_hotspots: usize,
    edges_removed_by_optimizer: usize,
    issues_split_by_optimizer: usize,
    issues_merged_by_optimizer: usize,
}

fn print_help() {
    const HELP: &str = r#"autospec graph

USAGE:
    autospec graph analyze --input <PATH> [--capacity N] [--format json|text]

SUBCOMMANDS:
    analyze    Analyze a proposed issue DAG: §19 metrics JSON, §24 execution
               wave projection, and §25 planner summary

OPTIONS (analyze):
    --input <PATH>     Issue graph JSON ({"issues": [...], "hard_edges": [...]})
                       or an array of issue drafts (no hard edges)
    --capacity <N>     Fleet capacity (default 32, per spec §6.2)
    --format <fmt>     json (default) or text

Exit codes:
    0  analysis succeeded
    2  usage or input error (missing file, invalid JSON, bad option)
    3  dependency cycle detected (cycle path printed on stderr, 1 line)"#;
    print!("{HELP}\n");
}
