use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use autospec_core::agent::AgentResult;
use autospec_core::execution::{
    AgentOutcome, ExecutionQueue, FailureKind, IngestedAgentResult, QueueResultApplication,
};

const DEFAULT_RETRY_LIMIT: u32 = 3;
const RESUME_COMMAND_ENV: &str = "AUTOSPEC_RESUME_COMMAND";
const RESUME_COMMAND_FILE: &str = "resume-command.json";
const STARTUP_FAILURE_FILE: &str = "startup-failed.json";

#[derive(Debug)]
enum Mode {
    Create,
    Ingest(PathBuf),
}

#[derive(Debug)]
struct Options {
    mode: Mode,
    run_id: String,
    specs: Vec<String>,
    result_id: Option<String>,
    outcome: Option<String>,
    failure_kind: Option<String>,
    retry_limit: u32,
    json: bool,
}

pub fn run(args: &[String]) -> Result<(), String> {
    let options = parse_options(args)?;
    match options.mode {
        Mode::Create => create_queue(&options),
        Mode::Ingest(ref input) => ingest_result(&options, input),
    }
}

fn create_queue(options: &Options) -> Result<(), String> {
    if options.result_id.is_some() || options.outcome.is_some() || options.failure_kind.is_some() {
        return Err("autospec run result options require --ingest <agent-result.json>".to_string());
    }
    if options.retry_limit != DEFAULT_RETRY_LIMIT {
        return Err("autospec run --retry-limit requires --ingest <agent-result.json>".to_string());
    }

    let queue =
        ExecutionQueue::create_if_absent(".", options.run_id.clone(), options.specs.clone())?;
    let resume_command_persisted = match persist_resume_command(".", &queue.run_id) {
        Ok(persisted) => persisted,
        Err(error) => {
            let _ = record_startup_failure(".", &queue.run_id, &error);
            if options.json {
                println!(
                    "{{\"command\":\"run\",\"mode\":\"create\",\"status\":\"startup_failed\",\"run_id\":\"{}\",\"recovery\":\"retry\",\"lock\":\"released\",\"error\":\"{}\"}}",
                    escape_json(&queue.run_id),
                    escape_json(&error),
                );
            }
            return Err(error);
        }
    };

    if options.json {
        println!(
            "{{\"command\":\"run\",\"mode\":\"create\",\"status\":\"created\",\"run_id\":\"{}\",\"spec_count\":{},\"resume_command_persisted\":{}}}",
            escape_json(&queue.run_id),
            options.specs.len(),
            resume_command_persisted,
        );
    } else {
        println!(
            "AutoSpec created local run {} with {} queued spec(s); no agent or validation command was executed",
            queue.run_id,
            options.specs.len()
        );
    }
    Ok(())
}

pub(super) fn resume_command_path(root: impl AsRef<Path>, run_id: &str) -> PathBuf {
    run_directory(root.as_ref(), run_id).join(RESUME_COMMAND_FILE)
}

pub(super) fn startup_failure_path(root: impl AsRef<Path>, run_id: &str) -> PathBuf {
    run_directory(root.as_ref(), run_id).join(STARTUP_FAILURE_FILE)
}

fn run_directory(root: &Path, run_id: &str) -> PathBuf {
    root.join(".autospec").join("runs").join(run_id)
}

fn persist_resume_command(root: impl AsRef<Path>, run_id: &str) -> Result<bool, String> {
    let command = match env::var(RESUME_COMMAND_ENV) {
        Ok(command) if !command.is_empty() => command,
        Ok(_) | Err(env::VarError::NotPresent) => return Ok(false),
        Err(error) => return Err(format!("{RESUME_COMMAND_ENV} is not valid UTF-8: {error}")),
    };
    let path = resume_command_path(root, run_id);
    write_atomic(
        &path,
        &format!(
            "{{\n  \"schema\": 1,\n  \"status\": \"ready\",\n  \"resume_command\": {{\n    \"kind\": \"literal\",\n    \"source\": \"{}\",\n    \"value\": \"{}\"\n  }}\n}}\n",
            RESUME_COMMAND_ENV,
            escape_json(&command),
        ),
    )?;
    Ok(true)
}

fn record_startup_failure(
    root: impl AsRef<Path>,
    run_id: &str,
    reason: &str,
) -> Result<(), String> {
    let path = startup_failure_path(root, run_id);
    write_atomic(
        &path,
        &format!(
            "{{\n  \"schema\": 1,\n  \"status\": \"startup_failed\",\n  \"recovery\": \"retry\",\n  \"lock\": \"released\",\n  \"reason\": \"{}\"\n}}\n",
            escape_json(reason),
        ),
    )
}

fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let temporary = path.with_extension(format!("json.tmp-{}", std::process::id()));
    fs::write(&temporary, contents)
        .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!("cannot publish {}: {error}", path.display())
    })
}

fn ingest_result(options: &Options, input: &PathBuf) -> Result<(), String> {
    let result_id = options
        .result_id
        .as_deref()
        .ok_or_else(|| "autospec run --ingest requires --result-id <id>".to_string())?;
    let outcome = parse_outcome(options)?;
    if options.specs.len() != 1 {
        return Err("autospec run --ingest requires exactly one --spec <id>".to_string());
    }
    let agent_result = fs::read_to_string(input)
        .map_err(|error| format!("failed to read agent result {}: {error}", input.display()))?;
    let agent_result = AgentResult::from_json(&agent_result)?;
    let ingested = IngestedAgentResult::new(
        options.run_id.clone(),
        options.specs[0].clone(),
        result_id,
        outcome,
        agent_result,
    )?;
    let receipt = ExecutionQueue::ingest_agent_result(".", &ingested, options.retry_limit)?;

    if options.json {
        println!(
            "{{\"command\":\"run\",\"mode\":\"ingest\",\"status\":\"recorded\",\"run_id\":\"{}\",\"spec_id\":\"{}\",\"result_id\":\"{}\",\"outcome\":\"{}\",\"application\":\"{}\"}}",
            escape_json(&ingested.run_id),
            escape_json(&ingested.spec_id),
            escape_json(&ingested.result_id),
            ingested.outcome.as_str(),
            application_name(&receipt.application),
        );
    } else {
        println!(
            "AutoSpec recorded {} result {} for {} in local run {}; no agent or validation command was executed",
            ingested.outcome.as_str(),
            ingested.result_id,
            ingested.spec_id,
            ingested.run_id,
        );
    }
    Ok(())
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut mode = Mode::Create;
    let mut run_id = None;
    let mut specs = Vec::new();
    let mut result_id = None;
    let mut outcome = None;
    let mut failure_kind = None;
    let mut retry_limit = DEFAULT_RETRY_LIMIT;
    let mut json = false;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--run" => run_id = Some(required_value(args, &mut index, "--run")?),
            "--spec" => specs.push(required_value(args, &mut index, "--spec")?),
            "--ingest" => {
                if !matches!(mode, Mode::Create) {
                    return Err("autospec run accepts only one --ingest file".to_string());
                }
                mode = Mode::Ingest(PathBuf::from(required_value(args, &mut index, "--ingest")?));
            }
            "--result-id" => result_id = Some(required_value(args, &mut index, "--result-id")?),
            "--outcome" => outcome = Some(required_value(args, &mut index, "--outcome")?),
            "--failure-kind" => {
                failure_kind = Some(required_value(args, &mut index, "--failure-kind")?)
            }
            "--retry-limit" => {
                let value = required_value(args, &mut index, "--retry-limit")?;
                retry_limit = value.parse::<u32>().map_err(|_| {
                    "autospec run --retry-limit requires a non-negative integer".to_string()
                })?;
            }
            option => return Err(format!("unknown autospec run option: {option}")),
        }
        index += 1;
    }

    let run_id = run_id.ok_or_else(|| "autospec run requires --run <id>".to_string())?;
    if specs.is_empty() {
        return Err("autospec run requires at least one --spec <id>".to_string());
    }
    Ok(Options {
        mode,
        run_id,
        specs,
        result_id,
        outcome,
        failure_kind,
        retry_limit,
        json,
    })
}

fn required_value(args: &[String], index: &mut usize, option: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .filter(|value| !value.is_empty() && !value.starts_with("--"))
        .cloned()
        .ok_or_else(|| format!("autospec run {option} requires a value"))
}

fn parse_outcome(options: &Options) -> Result<AgentOutcome, String> {
    let outcome = options.outcome.as_deref().ok_or_else(|| {
        "autospec run --ingest requires --outcome <passed|failed|blocked>".to_string()
    })?;
    match outcome {
        "passed" => {
            if options.failure_kind.is_some() {
                return Err(
                    "autospec run --failure-kind is valid only for --outcome failed".to_string(),
                );
            }
            Ok(AgentOutcome::Passed)
        }
        "blocked" => {
            if options.failure_kind.is_some() {
                return Err(
                    "autospec run --failure-kind is valid only for --outcome failed".to_string(),
                );
            }
            Ok(AgentOutcome::Blocked)
        }
        "failed" => {
            let failure_kind = options.failure_kind.as_deref().ok_or_else(|| {
                "autospec run --outcome failed requires --failure-kind <kind>".to_string()
            })?;
            let failure_kind = match failure_kind {
                "validation" => FailureKind::Validation,
                "environment" => FailureKind::Environment,
                "agent" => FailureKind::Agent,
                "dependency" => FailureKind::Dependency,
                "safety" => FailureKind::Safety,
                _ => return Err(format!("unknown autospec run failure kind: {failure_kind}")),
            };
            Ok(AgentOutcome::Failed { failure_kind })
        }
        _ => Err(format!("unknown autospec run outcome: {outcome}")),
    }
}

fn application_name(application: &QueueResultApplication) -> &'static str {
    match application {
        QueueResultApplication::Applied => "applied",
        QueueResultApplication::AlreadyApplied => "already-applied",
    }
}

fn escape_json(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}
