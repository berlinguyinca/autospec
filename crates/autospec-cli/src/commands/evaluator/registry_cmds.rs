//! `evaluator init|register|list|show|pin` subcommand handlers.
//!
//! These are thin: they parse arguments, read the operator-supplied files into
//! `autospec_core` evaluation types, and delegate to the core
//! [`EvaluationStore`] (the single system of record). The on-disk layout and
//! digests are owned by the core, not this CLI slice.

use std::path::Path;

use serde_json::json;

use autospec_core::evaluation::evaluator::{EvaluatorDefinition, EvaluatorKind};
use autospec_core::evaluation::policy::PromotionPolicy;

use super::args::{check_args, has_flag, now, option, open_store, positional_ref, split_common};
use crate::commands::CommandFailure;

const INIT_USAGE: &str =
    "Usage: autospec evaluator init [--policy <policy.json>] [--root <dir>] [--json]";
const REGISTER_USAGE: &str =
    "Usage: autospec evaluator register --file <definition.json> [--root <dir>] [--json]";
const LIST_USAGE: &str = "Usage: autospec evaluator list [--root <dir>] [--json]";
const SHOW_USAGE: &str = "Usage: autospec evaluator show <slot@version> [--root <dir>] [--json]";
const PIN_USAGE: &str =
    "Usage: autospec evaluator pin <slot@version> --actor <name> [--root <dir>] [--json]";

fn store_base(root: &Path) -> std::path::PathBuf {
    root.join(".autospec").join("evaluation")
}

/// Core's `EvaluatorKind` is a plain serde enum; render it for output.
fn kind_str(kind: EvaluatorKind) -> &'static str {
    match kind {
        EvaluatorKind::Learned => "learned",
        EvaluatorKind::Deterministic => "deterministic",
    }
}

/// `evaluator init` — write-once policy, genesis epoch, current pointer.
pub fn init(args: &[String]) -> Result<(), CommandFailure> {
    if has_flag(args, "--help") || has_flag(args, "-h") {
        println!("{INIT_USAGE}");
        return Ok(());
    }
    let (common, rest) = split_common(args)?;
    let policy_file = option(&rest, "--policy")?;
    check_args(&rest, &["--policy"], &["--help", "-h"])?;
    let policy = match policy_file {
        Some(file) => {
            let raw = std::fs::read(&file).map_err(|err| {
                CommandFailure::diagnostic(format!("read policy file {file}: {err}"))
            })?;
            serde_json::from_slice::<PromotionPolicy>(&raw).map_err(|err| {
                CommandFailure::diagnostic(format!("policy file {file}: {err}"))
            })?
        }
        None => PromotionPolicy::default(),
    };
    let store = open_init_store(&common.root, policy)?;
    let epoch = store.current_epoch().map_err(CommandFailure::from)?;
    let policy_digest = store.policy().policy_digest();
    if common.json {
        println!(
            "{}",
            json!({
                "initialized": store_base(&common.root).to_string_lossy(),
                "epoch": epoch.epoch_id.to_string(),
                "policy_digest": policy_digest.to_string(),
            })
        );
    } else {
        println!("initialized evaluation store at {}", store_base(&common.root).display());
        println!("epoch: {}", epoch.epoch_id);
        println!("policy_digest: {policy_digest}");
    }
    Ok(())
}

/// `init` uses the core `init` (not `open`, which requires an existing store).
fn open_init_store(
    root: &Path,
    policy: PromotionPolicy,
) -> Result<autospec_core::evaluation::store::EvaluationStore, CommandFailure> {
    autospec_core::evaluation::store::EvaluationStore::init(root, policy, now())
        .map_err(CommandFailure::from)
}

/// `evaluator register --file <definition.json>` — immutable register.
pub fn register(args: &[String]) -> Result<(), CommandFailure> {
    if has_flag(args, "--help") || has_flag(args, "-h") {
        println!("{REGISTER_USAGE}");
        return Ok(());
    }
    let (common, rest) = split_common(args)?;
    let file = option(&rest, "--file")?.ok_or_else(|| {
        CommandFailure::diagnostic("autospec evaluator register requires --file <definition.json>")
    })?;
    check_args(&rest, &["--file"], &["--help", "-h"])?;
    let raw = std::fs::read(&file)
        .map_err(|err| CommandFailure::diagnostic(format!("read definition file {file}: {err}")))?;
    let definition: EvaluatorDefinition = serde_json::from_slice(&raw)
        .map_err(|err| CommandFailure::diagnostic(format!("definition file {file}: {err}")))?;
    let mut store = open_store(&common.root)?;
    let digest = store
        .register_evaluator(&definition, now())
        .map_err(CommandFailure::from)?;
    let reference = definition.version_ref();
    let path = store.layout().evaluator_file(reference.slot.as_str(), reference.version);
    if common.json {
        println!(
            "{}",
            json!({
                "registered": reference.to_string(),
                "digest": digest.to_string(),
                "path": path.to_string_lossy(),
            })
        );
    } else {
        println!("registered {reference}");
        println!("digest: {digest}");
    }
    Ok(())
}

/// `evaluator list` — every registered evaluator, sorted.
pub fn list(args: &[String]) -> Result<(), CommandFailure> {
    if has_flag(args, "--help") || has_flag(args, "-h") {
        println!("{LIST_USAGE}");
        return Ok(());
    }
    let (common, rest) = split_common(args)?;
    check_args(&rest, &[], &["--help", "-h"])?;
    let store = open_store(&common.root)?;
    let evaluators = store.list_evaluators().map_err(CommandFailure::from)?;
    if common.json {
        let items: Vec<serde_json::Value> = evaluators
            .iter()
            .map(|definition| {
                json!({
                    "reference": definition.version_ref().to_string(),
                    "kind": kind_str(definition.kind),
                    "digest": definition.definition_digest().to_string(),
                    "created_at": definition.created_at,
                })
            })
            .collect();
        println!("{}", json!({ "evaluators": items }));
    } else {
        for definition in &evaluators {
            println!(
                "{} {} {} {}",
                definition.version_ref(),
                kind_str(definition.kind),
                definition.definition_digest().short(),
                definition.created_at
            );
        }
    }
    Ok(())
}

/// `evaluator show <slot@version>` — one definition plus its digest.
pub fn show(args: &[String]) -> Result<(), CommandFailure> {
    if has_flag(args, "--help") || has_flag(args, "-h") {
        println!("{SHOW_USAGE}");
        return Ok(());
    }
    let (common, rest) = split_common(args)?;
    let (reference, flags) = positional_ref(&rest, "autospec evaluator show", &[])?;
    check_args(&flags, &[], &["--help", "-h"])?;
    let store = open_store(&common.root)?;
    let definition = store.evaluator(reference).map_err(|err| {
        CommandFailure::diagnostic(format!("evaluator {reference} is not registered: {err}"))
    })?;
    let digest = definition.definition_digest();
    if common.json {
        let value = serde_json::to_value(&definition)
            .map_err(|err| CommandFailure::diagnostic(format!("encode definition: {err}")))?;
        println!(
            "{}",
            json!({
                "evaluator": reference.to_string(),
                "digest": digest.to_string(),
                "definition": value,
            })
        );
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&definition)
                .map_err(|err| CommandFailure::diagnostic(format!("encode definition: {err}")))?
        );
        println!("digest: {digest}");
    }
    Ok(())
}

/// `evaluator pin <slot@version> --actor <name>` — seed an empty slot.
pub fn pin(args: &[String]) -> Result<(), CommandFailure> {
    if has_flag(args, "--help") || has_flag(args, "-h") {
        println!("{PIN_USAGE}");
        return Ok(());
    }
    let (common, rest) = split_common(args)?;
    let (reference, flags) = positional_ref(&rest, "autospec evaluator pin", &["--actor"])?;
    let actor = option(&flags, "--actor")?.ok_or_else(|| {
        CommandFailure::diagnostic("autospec evaluator pin requires --actor <name>")
    })?;
    check_args(&flags, &["--actor"], &["--help", "-h"])?;
    let mut store = open_store(&common.root)?;
    let event = store
        .pin(reference, &actor, now())
        .map_err(CommandFailure::from)?;
    if common.json {
        println!(
            "{}",
            json!({
                "pinned": event.to.to_string(),
                "epoch": event.epoch.epoch_id.to_string(),
                "promotion": event.id.to_string(),
                "actor": actor,
            })
        );
    } else {
        println!("pinned {} into {}", event.to, event.epoch.epoch_id);
        println!("promotion: {}", event.id);
        println!("actor: {actor}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn s(text: &str) -> String {
        text.to_string()
    }

    fn temp_root() -> std::path::PathBuf {
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "autospec-eval-cli-{counter}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn definition_json(slot: &str, version: u32, digest: &str) -> String {
        json!({
            "schema": 1,
            "slot": slot,
            "version": version,
            "kind": "deterministic",
            "rubric_ref": format!("rubrics/{slot}.md"),
            "routing_policy_digest": digest,
            "tool_policy_digest": digest,
            "created_at": 1_757_217_600,
            "provenance": {"created_by": "operator", "source": "manual"}
        })
        .to_string()
    }

    fn run_capture(command: &[&str], root: &std::path::Path) -> Result<(), CommandFailure> {
        let mut args: Vec<String> = command.iter().skip(1).map(|arg| s(arg)).collect();
        args.push(s("--root"));
        args.push(root.to_string_lossy().into_owned());
        match command.first().copied() {
            Some("init") => init(&args),
            Some("register") => register(&args),
            Some("list") => list(&args),
            Some("show") => show(&args),
            Some("pin") => pin(&args),
            _ => Err(CommandFailure::diagnostic("bad test command")),
        }
    }

    #[test]
    fn init_then_register_list_show_pin_flow() {
        let root = temp_root();
        let digest = "b".repeat(64);

        run_capture(&["init"], &root).unwrap();
        run_capture(&["init"], &root).unwrap_err();

        let definition = root.join("def.json");
        std::fs::write(&definition, definition_json("architecture", 1, &digest)).unwrap();
        run_capture(&["register", "--file", definition.to_str().unwrap()], &root).unwrap();
        run_capture(&["register", "--file", definition.to_str().unwrap()], &root).unwrap_err();

        run_capture(&["list"], &root).unwrap();
        run_capture(&["show", "architecture@1"], &root).unwrap();
        let error = run_capture(&["show", "architecture@9"], &root).unwrap_err();
        assert!(error.message.contains("architecture@9"), "{error}");

        run_capture(&["pin", "architecture@1", "--actor", "operator"], &root).unwrap();
        let error =
            run_capture(&["pin", "architecture@1", "--actor", "operator"], &root).unwrap_err();
        assert!(error.message.contains("pins"), "{error}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn register_without_file_is_a_diagnostic() {
        let root = temp_root();
        let error = register(&[s("--root"), root.to_string_lossy().into_owned()]).unwrap_err();
        assert!(error.message.contains("--file"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn pin_without_actor_is_a_diagnostic() {
        let root = temp_root();
        let error = pin(&[
            s("architecture@1"),
            s("--root"),
            root.to_string_lossy().into_owned(),
        ])
        .unwrap_err();
        assert!(error.message.contains("--actor"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
