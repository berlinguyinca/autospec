use std::process::Command;

pub fn run(args: &[String]) -> Result<(), String> {
    if std::env::var("AUTOSPEC_VALIDATE_FROM_SHELL")
        .ok()
        .as_deref()
        == Some("1")
    {
        return run_legacy_shell(args);
    }

    if super::is_json(args) {
        super::json_status("validate", "ok");
    } else {
        println!("AutoSpec validate: use bash scripts/validate.sh --fast");
    }
    Ok(())
}

fn run_legacy_shell(args: &[String]) -> Result<(), String> {
    let status = Command::new("bash")
        .arg("scripts/validate.sh")
        .args(args)
        .env("AUTOSPEC_FORCE_LEGACY_SHELL", "1")
        .env("AUTOSPEC_VALIDATE_FROM_RUST", "1")
        .status()
        .map_err(|error| format!("failed to run legacy shell validation: {error}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "legacy shell validation failed with status {}",
            status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_string())
        ))
    }
}
