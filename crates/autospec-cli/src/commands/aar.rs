//! `autospec aar` — inspect and apply Adaptive Agent Runtime policy.
//!
//! The decision engine itself is pure and lives in `autospec_core::aar`; this
//! command is the I/O edge: it parses arguments, reads a body file, writes the
//! worktree memory scaffold, and renders the decision as prose or JSON.

use std::fs;
use std::path::{Path, PathBuf};

use autospec_core::aar::classify::ClassificationInput;
use autospec_core::aar::memory::{required_directories, scaffold};
use autospec_core::aar::pi::working_rules_block;
use autospec_core::aar::policy::{decide, PolicyConfig};
use autospec_core::aar::profile::ModelProfileRegistry;

use super::CommandFailure;

const USAGE: &str = "\
autospec aar

USAGE:
    autospec aar <SUBCOMMAND> [OPTIONS]

SUBCOMMANDS:
    classify    Classify a work item and print its classification
    plan        Produce the execution policy for a work item
    explain     Print the prose explanation for a work item's policy
    memory      Scaffold worktree-local durable task memory
    rules       Print the harness working rules

OPTIONS (classify, plan, explain):
    --title <TEXT>        Work item title (required)
    --body <TEXT>         Work item body
    --body-file <PATH>    Read the body from a file
    --label <LABEL>       Repeatable issue label
    --path <PATH>         Repeatable referenced path
    --files <N>           Override the estimated file count
    --language <NAME>     Override the detected language
    --policy-version <V>  Version recorded on the decision
    --json                Emit JSON instead of text

OPTIONS (memory):
    --worktree <DIR>      Worktree root (default: current directory)
";

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    match args.first().map(String::as_str) {
        None | Some("--help") | Some("-h") => {
            print!("{USAGE}");
            Ok(())
        }
        Some("classify") => classify_command(&args[1..]),
        Some("plan") => plan_command(&args[1..], Output::Policy),
        Some("explain") => plan_command(&args[1..], Output::Explanation),
        Some("memory") => memory_command(&args[1..]),
        Some("rules") => {
            println!("{}", working_rules_block());
            Ok(())
        }
        Some(other) => Err(CommandFailure::diagnostic(format!(
            "unknown autospec aar subcommand: {other}"
        ))),
    }
}

enum Output {
    Policy,
    Explanation,
}

#[derive(Default)]
struct Options {
    input: ClassificationInput,
    policy_version: Option<String>,
    json: bool,
    worktree: Option<PathBuf>,
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut body_file: Option<PathBuf> = None;
    let mut index = 0;

    while index < args.len() {
        let argument = args[index].as_str();
        let mut take_value = |name: &str| -> Result<String, String> {
            index += 1;
            args.get(index)
                .cloned()
                .ok_or_else(|| format!("{name} requires a value"))
        };
        match argument {
            "--json" => options.json = true,
            "--title" => options.input.title = take_value("--title")?,
            "--body" => options.input.body = take_value("--body")?,
            "--body-file" => body_file = Some(PathBuf::from(take_value("--body-file")?)),
            "--label" => {
                let label = take_value("--label")?;
                options.input.labels.push(label);
            }
            "--path" => {
                let path = take_value("--path")?;
                options.input.referenced_paths.push(path);
            }
            "--language" => options.input.language = take_value("--language")?,
            "--files" => {
                let value = take_value("--files")?;
                options.input.estimated_files = value
                    .parse::<usize>()
                    .map_err(|_| format!("--files expects a number, got {value}"))?;
            }
            "--policy-version" => options.policy_version = Some(take_value("--policy-version")?),
            "--worktree" => options.worktree = Some(PathBuf::from(take_value("--worktree")?)),
            other => return Err(format!("unknown option: {other}")),
        }
        index += 1;
    }

    if let Some(path) = body_file {
        let body = fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        options.input.body = body;
    }

    Ok(options)
}

fn require_title(options: &Options) -> Result<(), String> {
    if options.input.title.trim().is_empty() {
        return Err("--title is required".to_string());
    }
    Ok(())
}

fn config(options: &Options) -> PolicyConfig {
    let mut config = PolicyConfig {
        registry: ModelProfileRegistry::starter(),
        ..PolicyConfig::default()
    };
    if let Some(version) = &options.policy_version {
        config.policy_version = version.clone();
    }
    config
}

fn classify_command(args: &[String]) -> Result<(), CommandFailure> {
    let options = parse_options(args).map_err(CommandFailure::diagnostic)?;
    require_title(&options).map_err(CommandFailure::diagnostic)?;
    let classification = autospec_core::aar::classify::classify(&options.input);

    if options.json {
        let decision = decide(&options.input, &config(&options))
            .map_err(CommandFailure::diagnostic)?;
        println!(
            "{}",
            decision.to_json().map_err(CommandFailure::diagnostic)?
        );
        return Ok(());
    }

    println!("task_class: {}", classification.task_class.as_str());
    println!("complexity: {}", classification.complexity.as_str());
    println!("risk: {}", classification.risk.as_str());
    println!("language: {}", classification.language);
    println!("estimated_files: {}", classification.estimated_files);
    println!(
        "capabilities: {}",
        classification
            .capabilities
            .iter()
            .map(|capability| capability.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("requires_vision: {}", classification.requires_vision);
    println!("requires_web: {}", classification.requires_web);
    println!(
        "requires_long_context: {}",
        classification.requires_long_context
    );
    println!("confidence: {:.2}", classification.confidence);
    println!("evidence:");
    for line in &classification.evidence {
        println!("  - {line}");
    }
    Ok(())
}

fn plan_command(args: &[String], output: Output) -> Result<(), CommandFailure> {
    let options = parse_options(args).map_err(CommandFailure::diagnostic)?;
    require_title(&options).map_err(CommandFailure::diagnostic)?;
    let decision =
        decide(&options.input, &config(&options)).map_err(CommandFailure::diagnostic)?;

    if options.json {
        println!(
            "{}",
            decision.to_json().map_err(CommandFailure::diagnostic)?
        );
        return Ok(());
    }

    if matches!(output, Output::Explanation) {
        println!("{}", decision.explain());
        return Ok(());
    }

    println!("policy_version: {}", decision.policy_version);
    println!("task_class: {}", decision.policy.task_class.as_str());
    println!("complexity: {}", decision.policy.complexity.as_str());
    println!("risk: {}", decision.policy.risk.as_str());
    println!(
        "roles: {}",
        decision
            .policy
            .topology
            .roles
            .iter()
            .map(|role| role.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "model: {}",
        decision
            .selected_model()
            .unwrap_or_else(|| "none eligible".to_string())
    );
    println!(
        "reasoning: {} ({} tokens)",
        decision.policy.reasoning.budget.as_str(),
        decision.policy.reasoning.tokens
    );
    if let Some(sampling) = &decision.policy.sampling {
        println!("sampling: {}", sampling.identity());
    }
    println!(
        "minimum_context_free: {}",
        decision.policy.model_requirements.minimum_context_free
    );
    println!(
        "retrieval: {}",
        decision
            .policy
            .context
            .ladder
            .iter()
            .map(|step| step.strategy.as_str())
            .collect::<Vec<_>>()
            .join(" -> ")
    );
    println!("rationale:");
    for line in &decision.rationale {
        println!("  - {line}");
    }
    println!();
    println!("{}", decision.explain());
    Ok(())
}

fn memory_command(args: &[String]) -> Result<(), CommandFailure> {
    let subcommand = args.first().map(String::as_str);
    let rest = match subcommand {
        Some("init") => &args[1..],
        Some(other) => {
            return Err(CommandFailure::diagnostic(format!(
                "unknown autospec aar memory subcommand: {other}"
            )))
        }
        None => {
            return Err(CommandFailure::diagnostic(
                "autospec aar memory requires a subcommand (init)".to_string(),
            ))
        }
    };
    let options = parse_options(rest).map_err(CommandFailure::diagnostic)?;
    let root = options
        .worktree
        .unwrap_or_else(|| PathBuf::from("."));

    if !root.is_dir() {
        return Err(CommandFailure::diagnostic(format!(
            "worktree {} is not a directory",
            root.display()
        )));
    }

    for directory in required_directories() {
        let path = root.join(directory);
        fs::create_dir_all(&path).map_err(|error| {
            CommandFailure::diagnostic(format!("cannot create {}: {error}", path.display()))
        })?;
    }

    let mut created = Vec::new();
    for (relative, contents) in scaffold() {
        let path = root.join(&relative);
        // Never overwrite: these files carry an in-flight task's durable state,
        // and a second `init` must not erase it.
        if path.exists() {
            continue;
        }
        write_new(&path, &contents)?;
        created.push(relative);
    }

    if options.json {
        let payload = created
            .iter()
            .map(|path| format!("\"{path}\""))
            .collect::<Vec<_>>()
            .join(",");
        println!("{{\"created\":[{payload}]}}");
    } else if created.is_empty() {
        println!("worktree memory already present at {}", root.display());
    } else {
        for path in &created {
            println!("created {path}");
        }
    }
    Ok(())
}

fn write_new(path: &Path, contents: &str) -> Result<(), CommandFailure> {
    fs::write(path, contents).map_err(|error| {
        CommandFailure::diagnostic(format!("cannot write {}: {error}", path.display()))
    })
}
