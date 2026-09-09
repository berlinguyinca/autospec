use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use autospec_core::agent::AgentResult;
use autospec_core::execution::{
    AgentOutcome, ExecutionQueue, FailureKind, IngestedAgentResult, QueueResultApplication,
    QueueStatus,
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
    dry_run: bool,
    json: bool,
}

pub fn run(args: &[String]) -> Result<(), String> {
    let options = parse_options(args)?;
    match options.mode {
        Mode::Create if options.dry_run => create_dry_run(&options),
        Mode::Create => create_queue(&options),
        Mode::Ingest(ref input) if options.dry_run => ingest_dry_run(&options, input),
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
    let mut dry_run = false;
    let mut json = false;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--dry-run" => dry_run = true,
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
        dry_run,
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

fn create_dry_run(options: &Options) -> Result<(), String> {
    if options.result_id.is_some() || options.outcome.is_some() || options.failure_kind.is_some() {
        return Err("autospec run result options require --ingest <agent-result.json>".to_string());
    }
    if options.retry_limit != DEFAULT_RETRY_LIMIT {
        return Err("autospec run --retry-limit requires --ingest <agent-result.json>".to_string());
    }
    ExecutionQueue::validate_plan(&options.run_id, &options.specs)?;
    let decision = if run_directory(Path::new("."), &options.run_id).exists() {
        dry_run_refuse(format!("queue already exists for run: {}", options.run_id))
    } else {
        DryRunDecision {
            decision: "create",
            application: None,
            status: None,
            reason: format!(
                "run {} has no local state yet; {} spec(s) would be queued",
                options.run_id,
                options.specs.len()
            ),
        }
    };
    print_dry_run(options, "create", None, None, None, &decision);
    Ok(())
}

fn ingest_dry_run(options: &Options, input: &PathBuf) -> Result<(), String> {
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
    let decision = ingest_dry_run_decision(&ingested, options.retry_limit);
    print_dry_run(
        options,
        "ingest",
        Some(&ingested.spec_id),
        Some(&ingested.result_id),
        Some(ingested.outcome.as_str()),
        &decision,
    );
    Ok(())
}

fn ingest_dry_run_decision(ingested: &IngestedAgentResult, retry_limit: u32) -> DryRunDecision {
    let queue = match inspect_queue(Path::new("."), &ingested.run_id) {
        InspectedQueue::Loaded(queue) => queue,
        InspectedQueue::Absent => {
            return dry_run_refuse(format!("queue does not exist for run: {}", ingested.run_id));
        }
        InspectedQueue::Ambiguous(error) => return dry_run_refuse(error),
    };
    let entry = match queue.entry(&ingested.spec_id) {
        Some(entry) => entry,
        None => {
            return dry_run_refuse(format!("unknown queue spec: {}", ingested.spec_id));
        }
    };
    let already = entry
        .agent_result_ids
        .iter()
        .any(|id| id == &ingested.result_id);
    if !already
        && matches!(
            entry.status,
            QueueStatus::Passed
                | QueueStatus::Blocked
                | QueueStatus::Deferred
                | QueueStatus::Superseded
        )
    {
        return dry_run_refuse(format!(
            "cannot apply a new result to terminal queue entry: {}",
            ingested.spec_id
        ));
    }
    if let Err(error) = result_file_collision(ingested) {
        return dry_run_refuse(error);
    }
    if already {
        return DryRunDecision {
            decision: "record",
            application: Some("already-applied"),
            status: Some(entry.status.as_str()),
            reason: format!(
                "result {} is already recorded for {}",
                ingested.result_id, ingested.spec_id
            ),
        };
    }
    let status = predicted_status(&ingested.outcome, entry.attempts, retry_limit);
    DryRunDecision {
        decision: "record",
        application: Some("applied"),
        status: Some(status),
        reason: format!(
            "result {} would be applied to {} as {}",
            ingested.result_id, ingested.spec_id, status
        ),
    }
}

fn predicted_status(outcome: &AgentOutcome, attempts: u32, retry_limit: u32) -> &'static str {
    match outcome {
        AgentOutcome::Passed => "passed",
        AgentOutcome::Failed { .. } => {
            if attempts + 1 > retry_limit {
                "blocked"
            } else {
                "failed"
            }
        }
        AgentOutcome::Blocked => "blocked",
    }
}

struct DryRunDecision {
    decision: &'static str,
    application: Option<&'static str>,
    status: Option<&'static str>,
    reason: String,
}

fn dry_run_refuse(reason: String) -> DryRunDecision {
    DryRunDecision {
        decision: "refuse",
        application: None,
        status: None,
        reason,
    }
}

fn print_dry_run(
    options: &Options,
    mode: &str,
    spec_id: Option<&str>,
    result_id: Option<&str>,
    outcome: Option<&str>,
    decision: &DryRunDecision,
) {
    if options.json {
        println!(
            "{{\"command\":\"run\",\"mode\":\"{mode}\",\"dry_run\":true,\"run_id\":\"{}\",\"spec_id\":{},\"result_id\":{},\"outcome\":{},\"decision\":\"{}\",\"application\":{},\"status\":{},\"reason\":\"{}\"}}",
            escape_json(&options.run_id),
            optional_json_field(spec_id),
            optional_json_field(result_id),
            optional_json_field(outcome),
            decision.decision,
            optional_json_field(decision.application),
            optional_json_field(decision.status),
            escape_json(&decision.reason),
        );
        return;
    }
    let subject = if mode == "ingest" {
        format!(
            "{} result {} for {} in local run {}",
            decision.decision,
            result_id.expect("ingest dry-run decisions carry a result id"),
            spec_id.expect("ingest dry-run decisions carry a spec id"),
            options.run_id,
        )
    } else if decision.decision == "create" {
        format!("create local run {}", options.run_id)
    } else {
        format!("refuse to create local run {}", options.run_id)
    };
    println!(
        "AutoSpec dry-run: would {subject} ({}); no state was written and no agent or validation command was executed",
        decision.reason
    );
}

fn optional_json_field(value: Option<&str>) -> String {
    value
        .map(|value| format!("\"{}\"", escape_json(value)))
        .unwrap_or_else(|| "null".to_string())
}

enum InspectedQueue {
    Loaded(ExecutionQueue),
    Absent,
    Ambiguous(String),
}

fn inspect_queue(root: &Path, run_id: &str) -> InspectedQueue {
    let directory = run_directory(root, run_id);
    let primary = directory.join("queue.json");
    let temporary = directory.join("queue.json.tmp");
    let primary_doc = match read_queue_document(&primary, run_id) {
        QueueDocument::Valid(queue) => return InspectedQueue::Loaded(queue),
        file => file,
    };
    match (primary_doc, read_queue_document(&temporary, run_id)) {
        (QueueDocument::Missing, QueueDocument::Missing) => InspectedQueue::Absent,
        (QueueDocument::Operational(error), _) => InspectedQueue::Ambiguous(error),
        (QueueDocument::Missing, QueueDocument::Valid(queue)) => InspectedQueue::Loaded(queue),
        (QueueDocument::Invalid(_), QueueDocument::Valid(queue)) => InspectedQueue::Loaded(queue),
        (QueueDocument::Missing, QueueDocument::Invalid(error))
        | (QueueDocument::Invalid(_), QueueDocument::Invalid(error)) => {
            InspectedQueue::Ambiguous(format!(
                "invalid queue recovery file {}: {error}",
                temporary.display()
            ))
        }
        (QueueDocument::Invalid(error), QueueDocument::Missing) => {
            InspectedQueue::Ambiguous(format!("invalid queue file {}: {error}", primary.display()))
        }
        (QueueDocument::Missing, QueueDocument::Operational(error))
        | (QueueDocument::Invalid(_), QueueDocument::Operational(error)) => {
            InspectedQueue::Ambiguous(error)
        }
        (QueueDocument::Valid(_), _) => unreachable!("valid queues return before recovery"),
    }
}

enum QueueDocument {
    Missing,
    Valid(ExecutionQueue),
    Invalid(String),
    Operational(String),
}

fn read_queue_document(path: &Path, run_id: &str) -> QueueDocument {
    match fs::read_to_string(path) {
        Ok(value) => match ExecutionQueue::from_json(&value) {
            Ok(queue) if queue.run_id == run_id => QueueDocument::Valid(queue),
            Ok(_) => QueueDocument::Invalid(format!(
                "queue document run id does not match path: {run_id}"
            )),
            Err(error) => QueueDocument::Invalid(error),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => QueueDocument::Missing,
        Err(error) => QueueDocument::Operational(format!(
            "failed to read queue file {}: {error}",
            path.display()
        )),
    }
}

enum ResultDocument {
    Missing,
    Stored(IngestedAgentResult),
    Invalid(String),
    Operational(String),
}

fn read_result_document(path: &Path, expected: &IngestedAgentResult) -> ResultDocument {
    match fs::read_to_string(path) {
        Ok(value) => match IngestedAgentResult::from_json(&value) {
            Ok(result)
                if result.run_id == expected.run_id
                    && result.spec_id == expected.spec_id
                    && result.result_id == expected.result_id =>
            {
                ResultDocument::Stored(result)
            }
            Ok(_) => ResultDocument::Invalid(format!(
                "agent result binding does not match path: {}/{}/{}",
                expected.run_id, expected.spec_id, expected.result_id
            )),
            Err(error) => ResultDocument::Invalid(error),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ResultDocument::Missing,
        Err(error) => ResultDocument::Operational(format!(
            "failed to read agent result {}: {error}",
            path.display()
        )),
    }
}

fn result_file_collision(ingested: &IngestedAgentResult) -> Result<(), String> {
    let directory = run_directory(Path::new("."), &ingested.run_id)
        .join("agent-results")
        .join(&ingested.spec_id);
    let primary = directory.join(format!("{}.json", ingested.result_id));
    let temporary = directory.join(format!("{}.json.tmp", ingested.result_id));
    let primary_doc = read_result_document(&primary, ingested);
    if let ResultDocument::Stored(stored) = &primary_doc {
        return same_result_identity(stored, ingested);
    }
    let temporary_doc = read_result_document(&temporary, ingested);
    if let ResultDocument::Stored(stored) = &temporary_doc {
        return same_result_identity(stored, ingested);
    }
    match (&primary_doc, &temporary_doc) {
        (&ResultDocument::Stored(_), _) | (_, &ResultDocument::Stored(_)) => {
            unreachable!("stored documents return before pairing")
        }
        (ResultDocument::Missing, ResultDocument::Missing) => Ok(()),
        (ResultDocument::Operational(error), _)
        | (ResultDocument::Missing, ResultDocument::Operational(error))
        | (ResultDocument::Invalid(_), ResultDocument::Operational(error)) => Err(error.clone()),
        (ResultDocument::Invalid(error), ResultDocument::Missing) => Err(format!(
            "invalid agent result {}: {error}",
            primary.display()
        )),
        (ResultDocument::Missing, ResultDocument::Invalid(_)) => Err(format!(
            "agent result recovery file {} is invalid; a dry run will not settle the recovery",
            temporary.display()
        )),
        (ResultDocument::Invalid(_), ResultDocument::Invalid(error)) => Err(format!(
            "invalid agent result recovery file {}: {error}",
            temporary.display()
        )),
    }
}

fn same_result_identity(
    stored: &IngestedAgentResult,
    ingested: &IngestedAgentResult,
) -> Result<(), String> {
    if stored.run_id == ingested.run_id
        && stored.spec_id == ingested.spec_id
        && stored.result_id == ingested.result_id
        && stored.outcome == ingested.outcome
        && stored.agent_result == ingested.agent_result
    {
        Ok(())
    } else {
        Err(format!(
            "agent result id {} already exists with different content",
            ingested.result_id
        ))
    }
}
