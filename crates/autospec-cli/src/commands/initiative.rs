//! `autospec initiative` — inspect and gate cross-repository Initiatives.
//!
//! Every subcommand reads the canonical artifact registry under
//! `.autospec/initiatives/<INIT-…>/`. Nothing here talks to GitHub: the
//! projection is rendered from canonical state, which is what makes it
//! recoverable.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::initiative::dag::{SchedulingContext, TaskGraph};
use autospec_core::initiative::definition::Definition;
use autospec_core::initiative::ids::InitiativeId;
use autospec_core::initiative::plan::ArchitecturePlan;
use autospec_core::initiative::projection::GithubProjection;
use autospec_core::initiative::repository::Workspace;
use autospec_core::initiative::store::{
    ArtifactFamily, AuditEvent, InitiativeArtifact, InitiativeStore,
};
use autospec_core::initiative::traceability::{CoverageMatrix, EvidenceRecord, Waiver};
use autospec_core::initiative::{status, Initiative};

use super::CommandFailure;

const SUBCOMMANDS: &[(&str, &str)] = &[
    ("init", "Create the artifact registry for an Initiative"),
    ("validate", "Check the definition, plan, and graph together"),
    ("ready", "Show which tasks may be released now"),
    ("coverage", "Show requirement coverage and evidence"),
    ("verify", "Check the final completion gate"),
    ("project", "Render the GitHub projection"),
    ("status", "Summarize the Initiative"),
];

#[derive(Debug)]
struct Options {
    root: PathBuf,
    id: Option<InitiativeId>,
    slug: Option<String>,
    spec: Option<String>,
    now: Option<u64>,
    json: bool,
}

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
        [subcommand, rest @ ..] => {
            let options = parse_options(rest).map_err(CommandFailure::diagnostic)?;
            match subcommand.as_str() {
                "init" => initialize(&options),
                "validate" => validate(&options),
                "ready" => ready(&options),
                "coverage" => coverage(&options),
                "verify" => verify(&options),
                "project" => project(&options),
                "status" => summarize(&options),
                other => Err(CommandFailure::diagnostic(format!(
                    "unknown autospec initiative subcommand: {other}"
                ))),
            }
        }
    }
}

fn print_help() {
    println!("autospec initiative\n\nUSAGE:\n    autospec initiative [SUBCOMMAND] [OPTIONS]\n\nSUBCOMMANDS:");
    for (subcommand, description) in SUBCOMMANDS {
        println!("    {subcommand:<10} {description}");
    }
    println!(
        "\nOPTIONS:\n    --id <INIT-YYYY-NNNN>   Initiative identifier\n    --root <dir>            Repository root holding .autospec (default: cwd)\n    --slug <slug>           Initiative slug (init only)\n    --spec <path>           Specification path (init only)\n    --now <unix-seconds>    Evaluate leases against this time\n    --json                  Machine-readable output"
    );
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        root: std::env::current_dir().map_err(|error| error.to_string())?,
        id: None,
        slug: None,
        spec: None,
        now: None,
        json: false,
    };
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--json" => options.json = true,
            "--root" => options.root = PathBuf::from(value(args, &mut index, "--root")?),
            "--id" => options.id = Some(InitiativeId::parse(value(args, &mut index, "--id")?)?),
            "--slug" => options.slug = Some(value(args, &mut index, "--slug")?),
            "--spec" => options.spec = Some(value(args, &mut index, "--spec")?),
            "--now" => {
                let raw = value(args, &mut index, "--now")?;
                options.now = Some(
                    raw.parse::<u64>()
                        .map_err(|_| format!("--now expects unix seconds: {raw}"))?,
                );
            }
            other => return Err(format!("unknown autospec initiative option: {other}")),
        }
        index += 1;
    }

    Ok(options)
}

fn value(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .filter(|value| !value.starts_with("--"))
        .cloned()
        .ok_or_else(|| format!("autospec initiative {flag} requires a value"))
}

fn store(options: &Options) -> Result<InitiativeStore, CommandFailure> {
    let id = options
        .id
        .clone()
        .ok_or_else(|| CommandFailure::diagnostic("autospec initiative requires --id"))?;
    Ok(InitiativeStore::new(&options.root, id))
}

fn now(options: &Options) -> u64 {
    options.now.unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default()
    })
}

/// Everything the read-only subcommands need from the registry.
struct Loaded {
    store: InitiativeStore,
    initiative: Initiative,
    definition: Definition,
    workspace: Workspace,
    plan: ArchitecturePlan,
    graph: TaskGraph,
    evidence: Vec<EvidenceRecord>,
    waivers: Vec<Waiver>,
}

impl Loaded {
    fn coverage(&self) -> CoverageMatrix {
        CoverageMatrix::build(
            &self.definition,
            &self.plan,
            &self.graph,
            &self.evidence,
            &self.waivers,
        )
    }
}

fn load(options: &Options) -> Result<Loaded, CommandFailure> {
    let store = store(options)?;
    if !store.root().is_dir() {
        return Err(CommandFailure::diagnostic(format!(
            "no initiative registry at {}",
            store.root().display()
        )));
    }

    let initiative: Initiative = read(&store, &InitiativeArtifact::Record)?;
    let definition_version = latest(&store, ArtifactFamily::Definition, "definition")?;
    let definition: Definition = read(
        &store,
        &InitiativeArtifact::Definition {
            version: definition_version,
        },
    )?;
    let workspace: Workspace = read(&store, &InitiativeArtifact::WorkspaceRepositories)?;
    let plan_version = latest(
        &store,
        ArtifactFamily::ArchitecturePlan,
        "architecture plan",
    )?;
    let plan: ArchitecturePlan = read(
        &store,
        &InitiativeArtifact::ArchitecturePlan {
            version: plan_version,
        },
    )?;
    let graph_version = latest(&store, ArtifactFamily::TaskGraph, "task graph")?;
    let graph: TaskGraph = read(
        &store,
        &InitiativeArtifact::TaskGraph {
            version: graph_version,
        },
    )?;

    Ok(Loaded {
        evidence: read_optional(&store, &InitiativeArtifact::Evidence)?,
        waivers: read_optional(&store, &InitiativeArtifact::Waivers)?,
        store,
        initiative,
        definition,
        workspace,
        plan,
        graph,
    })
}

fn read<T: for<'de> serde::Deserialize<'de>>(
    store: &InitiativeStore,
    artifact: &InitiativeArtifact,
) -> Result<T, CommandFailure> {
    store
        .read_json(artifact)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))
}

fn read_optional<T: for<'de> serde::Deserialize<'de> + Default>(
    store: &InitiativeStore,
    artifact: &InitiativeArtifact,
) -> Result<T, CommandFailure> {
    if store.exists(artifact) {
        read(store, artifact)
    } else {
        Ok(T::default())
    }
}

fn latest(
    store: &InitiativeStore,
    family: ArtifactFamily,
    label: &str,
) -> Result<u32, CommandFailure> {
    store.latest_version(family).ok_or_else(|| {
        CommandFailure::diagnostic(format!(
            "{} has no {label}; run the planning stages first",
            store.initiative()
        ))
    })
}

fn initialize(options: &Options) -> Result<(), CommandFailure> {
    let store = store(options)?;
    let slug = options
        .slug
        .clone()
        .ok_or_else(|| CommandFailure::diagnostic("autospec initiative init requires --slug"))?;
    let spec = options
        .spec
        .clone()
        .unwrap_or_else(|| "spec/spec.md".to_string());

    if store.exists(&InitiativeArtifact::Record) {
        return Err(CommandFailure::diagnostic(format!(
            "{} already exists at {}",
            store.initiative(),
            store.root().display()
        )));
    }

    store
        .initialize()
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    let initiative = Initiative::new(store.initiative().clone(), slug, spec);
    initiative
        .validate()
        .map_err(|problems| CommandFailure::diagnostic(problems.join("; ")))?;
    store
        .write_json(&InitiativeArtifact::Record, &initiative)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    store
        .append_event(
            &AuditEvent::new(
                now(options),
                "initiative.created",
                store.initiative().clone(),
                "autospec-cli",
            )
            .with_detail("slug", &initiative.slug)
            .with_detail("spec", &initiative.spec),
        )
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;

    if options.json {
        render_json(&initiative)?;
    } else {
        println!(
            "Initiative {} created at {}",
            initiative.id,
            store.root().display()
        );
    }
    Ok(())
}

fn validate(options: &Options) -> Result<(), CommandFailure> {
    let loaded = load(options)?;
    let mut problems = Vec::new();

    if let Err(found) = loaded.initiative.validate() {
        problems.extend(found);
    }
    if let Err(found) = loaded.definition.validate() {
        problems.extend(found);
    }
    if let Err(found) = loaded.workspace.validate() {
        problems.extend(found);
    }
    if let Err(found) = loaded.plan.validate(&loaded.definition, &loaded.workspace) {
        problems.extend(found);
    }
    if let Err(violations) = loaded.graph.validate(&loaded.definition, &loaded.workspace) {
        problems.extend(violations.iter().map(|violation| violation.message()));
    }
    for gap in loaded.definition.gaps() {
        problems.push(format!(
            "{} has no objectively verifiable acceptance criterion yet",
            gap.requirement()
        ));
    }

    if options.json {
        println!(
            "{}",
            serde_json::json!({
                "command": "initiative validate",
                "initiative": loaded.initiative.id.as_str(),
                "problems": problems,
                "valid": problems.is_empty(),
            })
        );
    } else if problems.is_empty() {
        println!(
            "{}: definition, plan, and graph agree",
            loaded.initiative.id
        );
    } else {
        println!("{}: {} problem(s)", loaded.initiative.id, problems.len());
        for problem in &problems {
            println!("- {problem}");
        }
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(CommandFailure::status(
            format!("{} is not executable", loaded.initiative.id),
            1,
        ))
    }
}

fn ready(options: &Options) -> Result<(), CommandFailure> {
    let loaded = load(options)?;
    let schedule = loaded.graph.schedule(SchedulingContext {
        workspace: &loaded.workspace,
        now: now(options),
    });

    if options.json {
        render_json(&schedule)?;
    } else {
        println!(
            "{}: {} ready, {} blocked",
            loaded.initiative.id,
            schedule.ready.len(),
            schedule.blocked.len()
        );
        for task in &schedule.ready {
            let repository = loaded
                .graph
                .get(task)
                .map(|task| task.repository.as_str().to_string())
                .unwrap_or_default();
            println!("- READY   {task} ({repository})");
        }
        for (task, reason) in &schedule.blocked {
            println!("- BLOCKED {task}: {}", reason.message());
        }
    }
    Ok(())
}

fn coverage(options: &Options) -> Result<(), CommandFailure> {
    let loaded = load(options)?;
    let matrix = loaded.coverage();

    if options.json {
        render_json(&matrix)?;
    } else {
        println!("{}: requirement coverage", loaded.initiative.id);
        for (requirement, status) in &matrix.statuses {
            println!(
                "- {requirement} {} tasks={} evidence={}",
                status.state.as_str(),
                status.tasks.len(),
                status.evidence.len()
            );
        }
        for rejected in &matrix.rejected_evidence {
            println!("! rejected evidence: {rejected}");
        }
    }
    Ok(())
}

fn verify(options: &Options) -> Result<(), CommandFailure> {
    let loaded = load(options)?;
    let matrix = loaded.coverage();
    let gate = matrix.completion_gate();

    if options.json {
        render_json(&gate)?;
    } else if gate.complete {
        println!(
            "{}: every requirement is verified or waived ({} waived)",
            loaded.initiative.id,
            gate.waived.len()
        );
    } else {
        println!(
            "{}: {} requirement(s) unverified",
            loaded.initiative.id,
            gate.unverified.len()
        );
        for requirement in &gate.unverified {
            println!("- {requirement}");
        }
        for rejected in &gate.rejected_evidence {
            println!("! {rejected}");
        }
    }

    if gate.complete {
        Ok(())
    } else {
        Err(CommandFailure::status(
            format!(
                "{} cannot complete with {} unverified requirement(s)",
                loaded.initiative.id,
                gate.unverified.len()
            ),
            1,
        ))
    }
}

fn project(options: &Options) -> Result<(), CommandFailure> {
    let loaded = load(options)?;
    let schedule = loaded.graph.schedule(SchedulingContext {
        workspace: &loaded.workspace,
        now: now(options),
    });
    let projection = GithubProjection::build(
        &loaded.initiative.id,
        &loaded.graph,
        &loaded.workspace,
        &loaded.coverage(),
        &schedule,
        loaded.initiative.github_projects.first().cloned(),
    );
    loaded
        .store
        .write_json(&InitiativeArtifact::GithubProjection, &projection)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;

    if options.json {
        render_json(&projection)?;
    } else {
        println!(
            "{}: {} issue projection(s), {} project row(s), {} unprojectable",
            loaded.initiative.id,
            projection.issues.len(),
            projection.items.len(),
            projection.unprojectable.len()
        );
        for (task, reason) in &projection.unprojectable {
            println!("- {task}: {reason}");
        }
    }
    Ok(())
}

fn summarize(options: &Options) -> Result<(), CommandFailure> {
    let loaded = load(options)?;
    let snapshot = status(
        &loaded.initiative,
        &loaded.definition,
        &loaded.plan,
        &loaded.graph,
        &loaded.workspace,
        &loaded.coverage(),
        now(options),
    );

    if options.json {
        render_json(&snapshot)?;
    } else {
        println!(
            "{} stage={} repositories={} owners={} ready={} blocked={}",
            snapshot.initiative,
            snapshot.stage.as_str(),
            snapshot.repository_count,
            snapshot.owner_scope_count,
            snapshot.ready_tasks,
            snapshot.blocked_tasks.len()
        );
        println!("tasks: {}", render_counts(&snapshot.task_states));
        println!(
            "requirements: {}",
            render_counts(&snapshot.requirement_states)
        );
        for (requirement, gap) in &snapshot.definition_gaps {
            println!("definition gap: {requirement} {gap}");
        }
        println!(
            "completion: {}",
            if snapshot.completion.complete {
                "verified".to_string()
            } else {
                format!("{} unverified", snapshot.completion.unverified.len())
            }
        );
    }
    Ok(())
}

fn render_counts(counts: &BTreeMap<String, usize>) -> String {
    if counts.is_empty() {
        return "none".to_string();
    }
    counts
        .iter()
        .map(|(name, count)| format!("{name}={count}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn render_json<T: serde::Serialize>(value: &T) -> Result<(), CommandFailure> {
    let rendered = serde_json::to_string(value)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    println!("{rendered}");
    Ok(())
}
