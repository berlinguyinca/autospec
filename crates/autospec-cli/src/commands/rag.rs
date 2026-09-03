//! `autospec rag` — inspect the Agentic RAG subsystem's effective policy.
//!
//! The subsystem's behavior is driven by configuration that is easy to get
//! subtly wrong: a role budget that silently truncates, a source an
//! administrator disabled, a routing decision that looks like a capacity bug.
//! These subcommands print what the subsystem would actually do, so an operator
//! can check it without running a retrieval.

use autospec_core::rag::config::RagConfig;
use autospec_core::rag::policy::{ALL_ROLES, AgentRole};
use autospec_core::rag::routing::{NodeCandidate, RagModelTask, ReasoningClass, select_node};
use autospec_core::rag::source::ALL_SOURCE_KINDS;

use super::CommandFailure;

const SUBCOMMANDS: &[(&str, &str)] = &[
    ("config", "Print the effective agentic_rag configuration"),
    ("policy", "Print retrieval policies, per role"),
    ("sources", "Print source availability for a role"),
    ("route", "Explain a model-routing decision for a subtask"),
];

/// Entry point for `autospec rag`.
pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    match args {
        [] => {
            print_help();
            Ok(())
        }
        [flag] if flag == "--help" || flag == "-h" => {
            print_help();
            Ok(())
        }
        [subcommand, rest @ ..] => match subcommand.as_str() {
            "config" => config(rest),
            "policy" => policy(rest),
            "sources" => sources(rest),
            "route" => route(rest),
            other => Err(CommandFailure::diagnostic(format!(
                "unknown autospec rag subcommand: {other}"
            ))),
        },
    }
}

fn print_help() {
    println!("autospec rag\n\nUSAGE:\n    autospec rag [SUBCOMMAND]\n\nSUBCOMMANDS:");
    for (subcommand, description) in SUBCOMMANDS {
        println!("    {subcommand:<10} {description}");
    }
    println!(
        "\nOPTIONS:\n    --role <ROLE>     Role to report on ({})\n    --set K=V         Apply a configuration override before reporting\n    --json            Emit JSON\n    -h, --help        Print help",
        ALL_ROLES
            .iter()
            .map(|role| role.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
}

fn config(args: &[String]) -> Result<(), CommandFailure> {
    let config = build_config(args)?;
    config
        .validate()
        .map_err(|error| CommandFailure::diagnostic(format!("invalid agentic_rag config: {error}")))?;
    if super::is_json(args) {
        println!("{}", config_json(&config));
    } else {
        print!("{}", config.render_yaml());
    }
    Ok(())
}

fn policy(args: &[String]) -> Result<(), CommandFailure> {
    let config = build_config(args)?;
    let roles = match role_argument(args)? {
        Some(role) => vec![role],
        None => ALL_ROLES.to_vec(),
    };
    if super::is_json(args) {
        let entries = roles
            .iter()
            .map(|role| policy_json(&config, *role))
            .collect::<Vec<_>>()
            .join(",");
        println!("{{\"policies\":[{entries}]}}");
        return Ok(());
    }
    for role in roles {
        let policy = config.policy_for(role);
        let budget = config.budget_for(role);
        println!("{}", role.as_str());
        println!("  policy:              {}", policy.name());
        println!("  max_context_tokens:  {}", policy.max_context_tokens());
        println!("  max_iterations:      {}", budget.max_iterations);
        println!("  max_queries:         {}", budget.max_queries);
        println!(
            "  sufficiency:         {}",
            policy.sufficiency_threshold()
        );
        println!(
            "  independent_review:  {}",
            policy.requires_independent_verification()
        );
        println!(
            "  priority_sources:    {}",
            policy
                .priority_sources()
                .iter()
                .map(|kind| kind.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(())
}

fn sources(args: &[String]) -> Result<(), CommandFailure> {
    let config = build_config(args)?;
    let role = role_argument(args)?.unwrap_or(AgentRole::Planner);
    // Reported both ways because the two answers differ, and an operator
    // debugging "why did it not search the web" needs to see which of the two
    // gates is closed.
    let task_permits_gated = args.iter().any(|arg| arg == "--external");
    if super::is_json(args) {
        let entries = ALL_SOURCE_KINDS
            .iter()
            .map(|kind| {
                format!(
                    "{{\"source\":\"{}\",\"availability\":\"{}\",\"allowed\":{}}}",
                    kind.as_str(),
                    config.availability(*kind).as_str(),
                    config.source_allowed(*kind, role, task_permits_gated)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        println!(
            "{{\"role\":\"{}\",\"task_permits_gated\":{task_permits_gated},\"sources\":[{entries}]}}",
            role.as_str()
        );
        return Ok(());
    }
    println!(
        "role {} (task_permits_gated: {task_permits_gated})",
        role.as_str()
    );
    for kind in ALL_SOURCE_KINDS {
        println!(
            "  {:<14} availability={:<7} allowed={}",
            kind.as_str(),
            config.availability(kind).as_str(),
            config.source_allowed(kind, role, task_permits_gated)
        );
    }
    Ok(())
}

fn route(args: &[String]) -> Result<(), CommandFailure> {
    let task = task_argument(args)?;
    let context = value_of(args, "--context")
        .map(|value| {
            value.parse::<u32>().map_err(|_| {
                CommandFailure::diagnostic(format!("--context expects an integer, found: {value}"))
            })
        })
        .transpose()?
        .unwrap_or(8_000);
    let capabilities = task.capabilities(context);
    let candidates = node_arguments(args)?;
    let decision = select_node(&capabilities, &candidates);

    if super::is_json(args) {
        let rejected = decision
            .rejected
            .iter()
            .map(|rejection| {
                format!(
                    "{{\"node\":\"{}\",\"reason\":\"{}\"}}",
                    rejection.node_id,
                    rejection.reason.replace('"', "'")
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        println!(
            "{{\"task\":\"{}\",\"required_context_tokens\":{},\"selected\":{},\"rejected\":[{rejected}]}}",
            task.as_str(),
            decision.required_context_tokens,
            decision
                .selected
                .as_ref()
                .map(|node| format!("\"{}\"", node.id))
                .unwrap_or_else(|| "null".to_string())
        );
        return Ok(());
    }
    println!("task:                    {}", task.as_str());
    println!("reasoning_class:         {}", capabilities.reasoning_class.as_str());
    println!("coding:                  {}", capabilities.coding);
    println!(
        "latency_priority:        {}",
        capabilities.latency_priority.as_str()
    );
    println!(
        "required_context_tokens: {}",
        decision.required_context_tokens
    );
    match &decision.selected {
        Some(node) => println!("selected:                {}", node.id),
        None => println!("selected:                none eligible"),
    }
    for rejection in &decision.rejected {
        println!("  rejected {}: {}", rejection.node_id, rejection.reason);
    }
    Ok(())
}

fn build_config(args: &[String]) -> Result<RagConfig, CommandFailure> {
    let mut config = RagConfig::default();
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--set" {
            let assignment = args.get(index + 1).ok_or_else(|| {
                CommandFailure::diagnostic("--set expects KEY=VALUE".to_string())
            })?;
            let (key, value) = assignment.split_once('=').ok_or_else(|| {
                CommandFailure::diagnostic(format!("--set expects KEY=VALUE, found: {assignment}"))
            })?;
            config
                .apply_override(key, value)
                .map_err(CommandFailure::diagnostic)?;
            index += 2;
            continue;
        }
        index += 1;
    }
    Ok(config)
}

fn role_argument(args: &[String]) -> Result<Option<AgentRole>, CommandFailure> {
    value_of(args, "--role")
        .map(|value| AgentRole::parse(&value).map_err(CommandFailure::diagnostic))
        .transpose()
}

fn task_argument(args: &[String]) -> Result<RagModelTask, CommandFailure> {
    let requested = value_of(args, "--task").unwrap_or_else(|| "query_rewriting".to_string());
    [
        RagModelTask::TaskClassification,
        RagModelTask::QueryRewriting,
        RagModelTask::RelevanceScoring,
        RagModelTask::CodeRelationshipAnalysis,
        RagModelTask::ArchitectureSynthesis,
        RagModelTask::ImplementationPlan,
    ]
    .into_iter()
    .find(|task| task.as_str() == requested)
    .ok_or_else(|| CommandFailure::diagnostic(format!("unknown rag model task: {requested}")))
}

/// Parse repeated `--node id:reasoning:context:speed:seats` arguments.
fn node_arguments(args: &[String]) -> Result<Vec<NodeCandidate>, CommandFailure> {
    let mut candidates = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if args[index] != "--node" {
            index += 1;
            continue;
        }
        let spec = args
            .get(index + 1)
            .ok_or_else(|| CommandFailure::diagnostic(NODE_USAGE.to_string()))?;
        let fields = spec.split(':').collect::<Vec<_>>();
        if fields.len() != 5 {
            return Err(CommandFailure::diagnostic(format!(
                "{NODE_USAGE}, found: {spec}"
            )));
        }
        let reasoning = match fields[1] {
            "small" => ReasoningClass::Small,
            "medium" => ReasoningClass::Medium,
            "strong" => ReasoningClass::Strong,
            other => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown reasoning class: {other}"
                )));
            }
        };
        let numeric = |field: &str, name: &str| {
            field.parse::<u32>().map_err(|_| {
                CommandFailure::diagnostic(format!("{name} expects an integer, found: {field}"))
            })
        };
        candidates.push(NodeCandidate {
            id: fields[0].to_string(),
            reasoning_class: reasoning,
            coding: true,
            structured_output: true,
            free_context_tokens: numeric(fields[2], "free context")?,
            speed_rank: numeric(fields[3], "speed rank")?,
            available_seats: numeric(fields[4], "available seats")?,
        });
        index += 2;
    }
    Ok(candidates)
}

const NODE_USAGE: &str = "--node expects id:reasoning:free_context:speed:seats";

fn value_of(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn config_json(config: &RagConfig) -> String {
    let sources = ALL_SOURCE_KINDS
        .iter()
        .map(|kind| {
            format!(
                "\"{}\":\"{}\"",
                kind.as_str(),
                config.availability(*kind).as_str()
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let roles = ALL_ROLES
        .iter()
        .map(|role| policy_json(config, *role))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"enabled\":{},\"default\":{{\"max_iterations\":{},\"max_queries\":{},\"max_evidence\":{},\"max_context_tokens\":{}}},\"sources\":{{{sources}}},\"graph\":{{\"enabled\":{},\"max_depth\":{}}},\"cache\":{{\"enabled\":{},\"revision_aware\":{}}},\"roles\":[{roles}]}}",
        config.enabled,
        config.default_budget.max_iterations,
        config.default_budget.max_queries,
        config.default_budget.max_evidence_items,
        config.default_budget.max_context_tokens,
        config.graph.enabled,
        config.graph.max_depth,
        config.cache.enabled,
        config.cache.revision_aware,
    )
}

fn policy_json(config: &RagConfig, role: AgentRole) -> String {
    let policy = config.policy_for(role);
    let budget = config.budget_for(role);
    let priority = policy
        .priority_sources()
        .iter()
        .map(|kind| format!("\"{}\"", kind.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"role\":\"{}\",\"policy\":\"{}\",\"max_context_tokens\":{},\"max_iterations\":{},\"max_queries\":{},\"sufficiency\":\"{}\",\"independent_verification\":{},\"priority_sources\":[{priority}]}}",
        role.as_str(),
        policy.name(),
        policy.max_context_tokens(),
        budget.max_iterations,
        budget.max_queries,
        policy.sufficiency_threshold(),
        policy.requires_independent_verification(),
    )
}
