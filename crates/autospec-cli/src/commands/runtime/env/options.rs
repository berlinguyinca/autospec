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

pub(super) fn parse_normalize_options(args: &[String]) -> Result<NormalizeOptions, CommandFailure> {
    let mut repo = PathBuf::from(".");
    let mut mode = None;
    let mut fingerprint = None;
    let mut index = 0;
    while let Some(argument) = args.get(index) {
        match argument.as_str() {
            "--repo" => {
                repo = PathBuf::from(option_value(args, index, "--repo")?);
                index += 2;
            }
            "--check" => {
                set_mode(&mut mode, NormalizeMode::Check)?;
                index += 1;
            }
            "--apply" => {
                set_mode(&mut mode, NormalizeMode::Apply)?;
                index += 1;
            }
            "--fingerprint" => {
                fingerprint = Some(option_value(args, index, "--fingerprint")?.to_string());
                index += 2;
            }
            _ if argument.starts_with("--repo=") => {
                repo = PathBuf::from(equals_value(argument, "--repo")?);
                index += 1;
            }
            _ if argument.starts_with("--fingerprint=") => {
                fingerprint = Some(equals_value(argument, "--fingerprint")?.to_string());
                index += 1;
            }
            _ => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec runtime env normalize-compose option: {argument}"
                )))
            }
        }
    }
    let mode = mode.ok_or_else(|| {
        CommandFailure::diagnostic("normalize-compose requires exactly one of --check or --apply")
    })?;
    if mode == NormalizeMode::Apply && fingerprint.is_none() {
        return Err(CommandFailure::diagnostic(
            "normalize-compose --apply requires --fingerprint SHA256",
        ));
    }
    if mode == NormalizeMode::Check && fingerprint.is_some() {
        return Err(CommandFailure::diagnostic(
            "normalize-compose --fingerprint is valid only with --apply",
        ));
    }
    if fingerprint.as_deref().is_some_and(|value| {
        value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) {
        return Err(CommandFailure::diagnostic(
            "normalize-compose --fingerprint must be a 64-character SHA-256 hex digest",
        ));
    }
    Ok(NormalizeOptions {
        repo,
        mode,
        fingerprint,
    })
}

fn set_mode(
    target: &mut Option<NormalizeMode>,
    value: NormalizeMode,
) -> Result<(), CommandFailure> {
    if target.replace(value).is_some() {
        Err(CommandFailure::diagnostic(
            "normalize-compose requires exactly one of --check or --apply",
        ))
    } else {
        Ok(())
    }
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
