//! `evaluator epoch current|history` subcommand handlers.

use serde_json::json;

use super::args::{check_args, has_flag, open_store, split_common};
use crate::commands::CommandFailure;

const EPOCH_USAGE: &str =
    "Usage: autospec evaluator epoch <current|history> [--root <dir>] [--json]";

/// Dispatch `epoch current` and `epoch history`.
pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    if has_flag(args, "--help") || has_flag(args, "-h") {
        println!("{EPOCH_USAGE}");
        return Ok(());
    }
    match args.first().map(String::as_str) {
        Some("current") => current(&args[1..]),
        Some("history") => history(&args[1..]),
        Some(other) => Err(CommandFailure::diagnostic(format!(
            "unknown epoch subcommand: {other} ({EPOCH_USAGE})"
        ))),
        None => Err(CommandFailure::diagnostic(
            "evaluator epoch requires current or history",
        )),
    }
}

fn current(args: &[String]) -> Result<(), CommandFailure> {
    let (common, rest) = split_common(args)?;
    check_args(&rest, &[], &["--help", "-h"])?;
    let epoch = open_store(&common.root)
        .epoch_current()
        .map_err(CommandFailure::from)?;
    if common.json {
        println!("{}", json!({ "epoch": epoch }));
    } else {
        println!("epoch: {}", epoch.epoch_id);
        println!("started_at: {}", epoch.started_at);
        println!("policy_digest: {}", epoch.policy_digest);
        for (slot, version) in &epoch.slot_versions {
            println!("{slot}@{version}");
        }
    }
    Ok(())
}

fn history(args: &[String]) -> Result<(), CommandFailure> {
    let (common, rest) = split_common(args)?;
    check_args(&rest, &[], &["--help", "-h"])?;
    let epochs = open_store(&common.root)
        .epoch_history()
        .map_err(CommandFailure::from)?;
    if common.json {
        println!("{}", json!({ "epochs": epochs }));
    } else {
        for epoch in &epochs {
            let promotion = epoch.promotion.as_deref().unwrap_or("-");
            println!("{} promotion: {promotion}", epoch.epoch_id);
        }
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

    #[test]
    fn epoch_requires_a_known_subcommand() {
        let error = run(&[]).unwrap_err();
        assert!(error.message.contains("requires current or history"));
        let error = run(&[s("bogus")]).unwrap_err();
        assert!(error.message.contains("bogus"));
    }

    #[test]
    fn epoch_current_and_history_on_a_pinned_store() {
        let base = tests_support::temp_base();
        let store = crate::commands::evaluator::store::EvaluationStore::new(
            base.join(".autospec").join("evaluation"),
        );
        store.init(None).unwrap();
        let digest = "c".repeat(64);
        let definition = serde_json::json!({
            "schema": 1,
            "slot": "test_quality",
            "version": 1,
            "kind": "deterministic",
            "rubric_ref": "rubrics/test_quality.md",
            "routing_policy_digest": digest,
            "tool_policy_digest": digest,
            "created_at": 1_757_217_600
        })
        .to_string();
        let file = base.join("def.json");
        std::fs::write(&file, definition).unwrap();
        store.register(std::path::Path::new(&file)).unwrap();
        let reference: crate::commands::evaluator::types::EvaluatorVersionRef =
            "test_quality@1".parse().unwrap();
        store.pin(&reference, "operator").unwrap();

        let args = vec![s("--root"), base.to_string_lossy().into_owned()];
        current(&args).unwrap();
        history(&args).unwrap();
        let json_args = vec![
            s("--root"),
            base.to_string_lossy().into_owned(),
            s("--json"),
        ];
        current(&json_args).unwrap();
        history(&json_args).unwrap();
    }
}
