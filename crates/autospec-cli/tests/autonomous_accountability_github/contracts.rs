use super::*;

#[test]
fn autonomous_cli_exposes_explicit_epic_start_and_resume_contract() {
    let help = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args(["autonomous", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("resume"));
    assert!(help.contains("--epic N"));

    let missing = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args(["autonomous", "resume", "--repo", "acme/widgets"])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("resume requires --epic N"));
}

#[test]
fn launcher_binds_verified_epic_before_any_conductor_spawn_and_supervisor_reuses_it() {
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/autonomous.rs"),
    )
    .unwrap();
    let start = source.find("fn start(options:").unwrap();
    let after_start = &source[start..];
    let binding = after_start.find("bind_accountability_epic").unwrap();
    let launch = after_start.find("start_after_lease").unwrap();
    assert!(binding < launch, "epic binding must precede launch");

    let control = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/commands/autonomous/accountability_runtime/control.rs"),
    )
    .unwrap();
    let repair = control.find("fn repair_stopped_conductor").unwrap();
    let repair_body = &control[repair..];
    let verification = repair_body.find("verify_existing_accountability").unwrap();
    let spawn = repair_body.find("spawn_unit(").unwrap();
    assert!(
        verification < spawn,
        "supervisor must verify the inherited epic before respawn"
    );
    assert!(source.contains("\\\"accountability\\\":{}"));
    let foreground = source.find("fn run_foreground(options:").unwrap();
    let foreground_body = &source[foreground..];
    let foreground_binding = foreground_body
        .find("verify_existing_accountability")
        .unwrap();
    let foreground_cycles = foreground_body.find("run_foreground_cycles").unwrap();
    assert!(
        foreground_binding < foreground_cycles,
        "inherited foreground workers must verify accountability before work"
    );
}
