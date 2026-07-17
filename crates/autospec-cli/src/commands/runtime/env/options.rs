use std::path::PathBuf;

use crate::commands::CommandFailure;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum NormalizeMode {
    Check,
    Apply,
}

#[derive(Debug, Clone)]
pub(super) struct NormalizeOptions {
    pub repo: PathBuf,
    pub mode: NormalizeMode,
    pub fingerprint: Option<String>,
}

struct ParseState {
    repo: PathBuf,
    repo_seen: bool,
    mode: Option<NormalizeMode>,
    fingerprint: Option<String>,
}

pub(super) fn parse_normalize_options(args: &[String]) -> Result<NormalizeOptions, CommandFailure> {
    let mut state = ParseState {
        repo: PathBuf::from("."),
        repo_seen: false,
        mode: None,
        fingerprint: None,
    };
    let mut index = 0;
    while index < args.len() {
        index += consume_option(args, index, &mut state)?;
    }
    finish(state)
}

fn consume_option(
    args: &[String],
    index: usize,
    state: &mut ParseState,
) -> Result<usize, CommandFailure> {
    let argument = &args[index];
    match argument.as_str() {
        "--repo" => {
            set_repo(state, option_value(args, index, "--repo")?)?;
            Ok(2)
        }
        "--check" => {
            set_mode(&mut state.mode, NormalizeMode::Check)?;
            Ok(1)
        }
        "--apply" => {
            set_mode(&mut state.mode, NormalizeMode::Apply)?;
            Ok(1)
        }
        "--fingerprint" => {
            set_fingerprint(state, option_value(args, index, "--fingerprint")?)?;
            Ok(2)
        }
        _ if argument.starts_with("--repo=") => {
            set_repo(state, equals_value(argument, "--repo")?)?;
            Ok(1)
        }
        _ if argument.starts_with("--fingerprint=") => {
            set_fingerprint(state, equals_value(argument, "--fingerprint")?)?;
            Ok(1)
        }
        _ => Err(CommandFailure::diagnostic(format!(
            "unknown autospec runtime env normalize-compose option: {argument}"
        ))),
    }
}

fn finish(state: ParseState) -> Result<NormalizeOptions, CommandFailure> {
    let mode = state.mode.ok_or_else(mode_error)?;
    if mode == NormalizeMode::Apply && state.fingerprint.is_none() {
        return Err(CommandFailure::diagnostic(
            "normalize-compose --apply requires --fingerprint SHA256",
        ));
    }
    if mode == NormalizeMode::Check && state.fingerprint.is_some() {
        return Err(CommandFailure::diagnostic(
            "normalize-compose --fingerprint is valid only with --apply",
        ));
    }
    if state.fingerprint.as_deref().is_some_and(|value| {
        value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }) {
        return Err(CommandFailure::diagnostic(
            "normalize-compose --fingerprint must be a 64-character lowercase SHA-256 hex digest",
        ));
    }
    Ok(NormalizeOptions {
        repo: state.repo,
        mode,
        fingerprint: state.fingerprint,
    })
}

fn set_repo(state: &mut ParseState, value: &str) -> Result<(), CommandFailure> {
    if state.repo_seen {
        return Err(CommandFailure::diagnostic(
            "normalize-compose --repo may be supplied only once",
        ));
    }
    state.repo = PathBuf::from(value);
    state.repo_seen = true;
    Ok(())
}

fn set_fingerprint(state: &mut ParseState, value: &str) -> Result<(), CommandFailure> {
    if state.fingerprint.is_some() {
        return Err(CommandFailure::diagnostic(
            "normalize-compose --fingerprint may be supplied only once",
        ));
    }
    state.fingerprint = Some(value.to_string());
    Ok(())
}

fn set_mode(
    target: &mut Option<NormalizeMode>,
    value: NormalizeMode,
) -> Result<(), CommandFailure> {
    if target.replace(value).is_some() {
        Err(mode_error())
    } else {
        Ok(())
    }
}

fn mode_error() -> CommandFailure {
    CommandFailure::diagnostic("normalize-compose requires exactly one of --check or --apply")
}

fn option_value<'a>(
    args: &'a [String],
    index: usize,
    option: &str,
) -> Result<&'a str, CommandFailure> {
    let value = args.get(index + 1).map(String::as_str).ok_or_else(|| {
        CommandFailure::diagnostic(format!("normalize-compose {option} requires a value"))
    })?;
    if value.is_empty() || value.starts_with("--") {
        Err(CommandFailure::diagnostic(format!(
            "normalize-compose {option} requires a value"
        )))
    } else {
        Ok(value)
    }
}

fn equals_value<'a>(argument: &'a str, option: &str) -> Result<&'a str, CommandFailure> {
    let value = argument
        .strip_prefix(&format!("{option}="))
        .unwrap_or_default();
    if value.is_empty() {
        Err(CommandFailure::diagnostic(format!(
            "normalize-compose {option} requires a value"
        )))
    } else {
        Ok(value)
    }
}
