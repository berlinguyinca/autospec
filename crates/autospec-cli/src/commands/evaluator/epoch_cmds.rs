//! `evaluator epoch current|history` subcommand handlers.
//!
//! Thin over the core [`EvaluationStore`]: read the active epoch or the full
//! epoch history and render them.

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
    let store = open_store(&common.root)?;
    let epoch = store.current_epoch().map_err(CommandFailure::from)?;
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
    let store = open_store(&common.root)?;
    let epochs = store.epoch_history().map_err(CommandFailure::from)?;
    if common.json {
        println!("{}", json!({ "epochs": epochs }));
    } else {
        for epoch in &epochs {
            let promotion = epoch
                .promotion
                .as_ref()
                .map(|id| id.to_string())
                .unwrap_or_else(|| "-".into());
            println!("{} promotion: {promotion}", epoch.epoch_id);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use autospec_core::evaluation::evaluator::EvaluatorDefinition;
    use autospec_core::evaluation::policy::PromotionPolicy;
    use autospec_core::evaluation::store::EvaluationStore;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

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
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "autospec-eval-epoch-{counter}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let digest = "c".repeat(64);
        let definition: EvaluatorDefinition = serde_json::from_value(json!({
            "schema": 1,
            "slot": "test_quality",
            "version": 1,
            "kind": "deterministic",
            "rubric_ref": "rubrics/test_quality.md",
            "routing_policy_digest": digest,
            "tool_policy_digest": digest,
            "created_at": 1_757_217_600,
            "provenance": {"created_by": "operator", "source": "manual"}
        }))
        .unwrap();

        let reference = definition.version_ref();
        let mut store = EvaluationStore::init(&root, PromotionPolicy::default(), 100).unwrap();
        store.register_evaluator(&definition, 100).unwrap();
        store.pin(reference, "operator", 100).unwrap();
        drop(store);

        let args = vec![s("--root"), root.to_string_lossy().into_owned()];
        current(&args).unwrap();
        history(&args).unwrap();
        let json_args = vec![
            s("--root"),
            root.to_string_lossy().into_owned(),
            s("--json"),
        ];
        current(&json_args).unwrap();
        history(&json_args).unwrap();

        let _ = std::fs::remove_dir_all(&root);
    }
}
