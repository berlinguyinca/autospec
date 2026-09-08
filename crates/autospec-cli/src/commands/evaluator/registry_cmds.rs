//! `evaluator init|register|list|show|pin` subcommand handlers.

use std::path::Path;

use serde_json::json;

use super::args::{check_args, has_flag, open_store, option, positional_ref, split_common};
use super::store::EvaluationStore;
use crate::commands::CommandFailure;

const INIT_USAGE: &str =
    "Usage: autospec evaluator init [--policy <policy.json>] [--root <dir>] [--json]";
const REGISTER_USAGE: &str =
    "Usage: autospec evaluator register --file <definition.json> [--root <dir>] [--json]";
const LIST_USAGE: &str = "Usage: autospec evaluator list [--root <dir>] [--json]";
const SHOW_USAGE: &str = "Usage: autospec evaluator show <slot@version> [--root <dir>] [--json]";
const PIN_USAGE: &str =
    "Usage: autospec evaluator pin <slot@version> --actor <name> [--root <dir>] [--json]";

fn store_for(root: &std::path::Path) -> EvaluationStore {
    open_store(root)
}

/// `evaluator init` — write-once policy, genesis epoch, current pointer.
pub fn init(args: &[String]) -> Result<(), CommandFailure> {
    if has_flag(args, "--help") || has_flag(args, "-h") {
        println!("{INIT_USAGE}");
        return Ok(());
    }
    let (common, rest) = split_common(args)?;
    let policy = option(&rest, "--policy")?;
    check_args(&rest, &["--policy"], &["--help", "-h"])?;
    let report = store_for(&common.root)
        .init(policy.as_deref().map(Path::new))
        .map_err(CommandFailure::from)?;
    if common.json {
        println!(
            "{}",
            json!({
                "initialized": report.base.to_string_lossy(),
                "epoch": report.epoch.to_string(),
                "policy_digest": report.policy_digest,
            })
        );
    } else {
        println!("initialized evaluation store at {}", report.base.display());
        println!("epoch: {}", report.epoch);
        println!("policy_digest: {}", report.policy_digest);
    }
    Ok(())
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
    let report = store_for(&common.root)
        .register(Path::new(&file))
        .map_err(CommandFailure::from)?;
    if common.json {
        println!(
            "{}",
            json!({
                "registered": report.reference.to_string(),
                "digest": report.digest,
                "path": report.path.to_string_lossy(),
            })
        );
    } else {
        println!("registered {}", report.reference);
        println!("digest: {}", report.digest);
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
    let entries = store_for(&common.root)
        .list()
        .map_err(CommandFailure::from)?;
    if common.json {
        let items: Vec<serde_json::Value> = entries
            .iter()
            .map(|entry| {
                json!({
                    "reference": entry.reference.to_string(),
                    "kind": entry.kind.as_str(),
                    "digest": entry.digest,
                    "created_at": entry.created_at,
                })
            })
            .collect();
        println!("{}", json!({ "evaluators": items }));
    } else {
        for entry in &entries {
            println!(
                "{} {} {} {}",
                entry.reference,
                entry.kind,
                &entry.digest[..16],
                entry.created_at
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
    let (definition, digest) = store_for(&common.root)
        .show(&reference)
        .map_err(CommandFailure::from)?;
    if common.json {
        let value = serde_json::to_value(&definition)
            .map_err(|err| CommandFailure::diagnostic(format!("encode definition: {err}")))?;
        println!(
            "{}",
            json!({
                "evaluator": reference.to_string(),
                "digest": digest,
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
    let report = store_for(&common.root)
        .pin(&reference, &actor)
        .map_err(CommandFailure::from)?;
    if common.json {
        println!(
            "{}",
            json!({
                "pinned": report.reference.to_string(),
                "epoch": report.epoch.to_string(),
                "promotion": report.promotion_id,
                "actor": report.actor,
            })
        );
    } else {
        println!("pinned {} into {}", report.reference, report.epoch);
        println!("promotion: {}", report.promotion_id);
        println!("actor: {}", report.actor);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::evaluator::store::tests_support;

    fn s(text: &str) -> String {
        text.to_string()
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
        let base = tests_support::temp_base();
        let root = base.join("root");
        let _ = std::fs::create_dir_all(&root);

        run_capture(&["init"], &root).unwrap();
        run_capture(&["init"], &root).unwrap_err();

        let definition = base.join("def.json");
        let digest = "b".repeat(64);
        std::fs::write(
            &definition,
            serde_json::json!({
                "schema": 1,
                "slot": "architecture",
                "version": 1,
                "kind": "deterministic",
                "rubric_ref": "rubrics/architecture.md",
                "routing_policy_digest": digest,
                "tool_policy_digest": digest,
                "created_at": 1_757_217_600
            })
            .to_string(),
        )
        .unwrap();
        run_capture(&["register", "--file", definition.to_str().unwrap()], &root).unwrap();
        run_capture(&["register", "--file", definition.to_str().unwrap()], &root).unwrap_err();

        run_capture(&["list"], &root).unwrap();
        run_capture(&["show", "architecture@1"], &root).unwrap();
        let error = run_capture(&["show", "architecture@9"], &root).unwrap_err();
        assert!(error.message.contains("architecture@9"));

        run_capture(&["pin", "architecture@1", "--actor", "operator"], &root).unwrap();
        let error =
            run_capture(&["pin", "architecture@1", "--actor", "operator"], &root).unwrap_err();
        assert!(error.message.contains("already pinned"));
    }

    #[test]
    fn register_without_file_is_a_diagnostic() {
        let base = tests_support::temp_base();
        let error = register(&[s("--root"), base.to_string_lossy().into_owned()]).unwrap_err();
        assert!(error.message.contains("--file"));
    }

    #[test]
    fn pin_without_actor_is_a_diagnostic() {
        let base = tests_support::temp_base();
        let error = pin(&[
            s("architecture@1"),
            s("--root"),
            base.to_string_lossy().into_owned(),
        ])
        .unwrap_err();
        assert!(error.message.contains("--actor"));
    }
}
