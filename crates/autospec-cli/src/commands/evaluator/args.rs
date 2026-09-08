//! Shared argument handling for the `autospec evaluator` subcommands.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::store::EvaluationStore;
use super::types::EvaluatorVersionRef;
use crate::commands::CommandFailure;

/// Store-relative flags shared by every subcommand.
#[derive(Debug)]
pub struct CommonArgs {
    pub root: PathBuf,
    pub json: bool,
}

/// Default store root: the current directory, so the store lives at
/// `.autospec/evaluation` under it.
pub fn default_root() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Current Unix time in seconds.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Split `--root <dir>` and `--json` out of a subcommand's args.
pub fn split_common(args: &[String]) -> Result<(CommonArgs, Vec<String>), CommandFailure> {
    let mut common = CommonArgs {
        root: default_root(),
        json: false,
    };
    let mut rest = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--root" => {
                let value = value_at(args, index, "--root")?;
                common.root = PathBuf::from(value);
                index += 2;
            }
            "--json" => {
                common.json = true;
                index += 1;
            }
            other => {
                rest.push(other.to_string());
                index += 1;
            }
        }
    }
    Ok((common, rest))
}

fn value_at(args: &[String], index: usize, flag: &str) -> Result<String, CommandFailure> {
    args.get(index + 1)
        .filter(|value| !value.starts_with("--"))
        .cloned()
        .ok_or_else(|| {
            CommandFailure::diagnostic(format!("{flag} requires a value (got none or a flag)"))
        })
}

/// Read `--flag <value>` out of the remaining args.
pub fn option(args: &[String], flag: &str) -> Result<Option<String>, CommandFailure> {
    let mut index = 0;
    while index < args.len() {
        if args[index] == flag {
            return value_at(args, index, flag).map(Some);
        }
        index += 1;
    }
    Ok(None)
}

/// True when `flag` appears in `args`.
pub fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

/// Reject any argument that is not one of the declared value/bool flags.
/// Positionals must already be extracted by the caller.
pub fn check_args(
    args: &[String],
    value_flags: &[&str],
    bool_flags: &[&str],
) -> Result<(), CommandFailure> {
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if value_flags.contains(&arg.as_str()) {
            value_at(args, index, arg)?;
            index += 2;
        } else if bool_flags.contains(&arg.as_str()) {
            index += 1;
        } else {
            return Err(CommandFailure::diagnostic(format!(
                "unexpected argument: {arg}"
            )));
        }
    }
    Ok(())
}

/// Extract the `<slot@version>` positional plus the remaining flags.
/// `value_flags` are the `--flag <value>` pairs whose values must not be
/// mistaken for a second positional.
pub fn positional_ref(
    args: &[String],
    command: &str,
    value_flags: &[&str],
) -> Result<(EvaluatorVersionRef, Vec<String>), CommandFailure> {
    let mut positional: Option<String> = None;
    let mut flags = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg.starts_with("--") {
            flags.push(arg.clone());
            if value_flags.contains(&arg.as_str()) {
                if let Some(value) = args.get(index + 1) {
                    flags.push(value.clone());
                }
                index += 2;
            } else {
                index += 1;
            }
        } else if positional.is_none() {
            positional = Some(arg.clone());
            index += 1;
        } else {
            return Err(CommandFailure::diagnostic(format!(
                "unexpected argument: {arg}"
            )));
        }
    }
    let text = positional
        .ok_or_else(|| CommandFailure::diagnostic(format!("{command} requires <slot@version>")))?;
    let reference: EvaluatorVersionRef = text.parse().map_err(|err| {
        CommandFailure::diagnostic(format!("invalid <slot@version> {text:?}: {err}"))
    })?;
    Ok((reference, flags))
}

/// Open the store under `<root>/.autospec/evaluation`.
pub fn open_store(root: &std::path::Path) -> EvaluationStore {
    EvaluationStore::new(root.join(".autospec").join("evaluation"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(text: &str) -> String {
        text.to_string()
    }

    #[test]
    fn split_common_extracts_root_and_json() {
        let (common, rest) = split_common(&[
            s("--root"),
            s("/tmp/wt"),
            s("register"),
            s("--file"),
            s("d.json"),
        ])
        .unwrap();
        assert_eq!(common.root, PathBuf::from("/tmp/wt"));
        assert!(common.json == false);
        assert_eq!(rest, vec![s("register"), s("--file"), s("d.json")]);
    }

    #[test]
    fn split_common_marks_json() {
        let (common, rest) = split_common(&[s("--json"), s("list")]).unwrap();
        assert!(common.json);
        assert_eq!(rest, vec![s("list")]);
    }

    #[test]
    fn value_flags_require_a_value() {
        let error = split_common(&[s("--root")]).unwrap_err();
        assert!(error.message.contains("--root"));

        let error = option(&[s("--file")], "--file").unwrap_err();
        assert!(error.message.contains("--file"));
    }

    #[test]
    fn check_args_rejects_unknown_flags_and_positionals() {
        assert!(check_args(&[s("--policy"), s("p.json")], &["--policy"], &[]).is_ok());
        let error = check_args(&[s("--bogus")], &["--policy"], &[]).unwrap_err();
        assert!(error.message.contains("--bogus"));
        let error = check_args(&[s("extra")], &["--policy"], &[]).unwrap_err();
        assert!(error.message.contains("extra"));
    }

    #[test]
    fn positional_ref_parses_slot_at_version() {
        let (reference, flags) = positional_ref(
            &[s("architecture@2"), s("--actor"), s("op")],
            "pin",
            &["--actor"],
        )
        .unwrap();
        assert_eq!(reference.to_string(), "architecture@2");
        assert_eq!(flags, vec![s("--actor"), s("op")]);

        let error = positional_ref(&[s("architecture")], "pin", &["--actor"]).unwrap_err();
        assert!(error.message.contains("slot@version"));

        let error = positional_ref(&[], "pin", &["--actor"]).unwrap_err();
        assert!(error.message.contains("requires"));
    }
}
