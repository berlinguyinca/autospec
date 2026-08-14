use super::*;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::time::{SystemTime, UNIX_EPOCH};

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
fn resume_rejects_force_before_touching_a_stopped_run() {
    let fixture = CliResumeFixture::new("force-rejected");
    fixture.record_immediate_stop();
    let stop_before = fs::read_to_string(fixture.stop_flag()).expect("read stop flag");

    let output = fixture
        .command("resume")
        .args(["--epic", "12", "--force"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--force is not valid with resume"));
    assert_eq!(
        fs::read_to_string(fixture.stop_flag()).unwrap(),
        stop_before
    );
}

#[test]
#[cfg(unix)]
fn autonomous_resume_reopens_the_same_epic_and_clears_an_immediate_stop() {
    let fixture = CliResumeFixture::new("stopped-resume");
    fixture.record_immediate_stop();
    fixture.install_closed_epic();

    let output = fixture
        .command("resume")
        .args(["--epic", "12", "--json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "stdout={stdout} stderr={stderr}");
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["subcommand"], "resume");
    assert_eq!(result["epic_number"], 12);
    assert_eq!(fs::read_to_string(fixture.issue_state()).unwrap(), "OPEN\n");
    assert!(
        !fixture.stop_flag().exists(),
        "resume must clear the stop sentinel"
    );
    assert!(
        fixture.scope().join("launch.json").exists(),
        "resume must reach spawn"
    );
    let calls = fs::read_to_string(fixture.gh_calls()).unwrap();
    assert!(calls.contains("issue reopen 12"), "calls={calls}");
    fixture.stop_spawned_run();
}

struct CliResumeFixture {
    root: std::path::PathBuf,
}

impl CliResumeFixture {
    fn new(name: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "autospec-cli-resume-{name}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("repo")).unwrap();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .current_dir(root.join("repo"))
            .status()
            .unwrap()
            .success());
        Self { root }
    }

    fn command(&self, subcommand: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_autospec"));
        command
            .args(["autonomous", subcommand])
            .args(["--repo", "acme/widgets", "--repo-dir"])
            .arg(self.root.join("repo"))
            .args(["--max-cycles", "1", "--poll-interval-sec", "1"])
            .env(
                "AUTOSPEC_AUTONOMOUS_OPERATOR_DIR",
                self.root.join("operator"),
            )
            .env("AUTOSPEC_STATE_DIR", self.root.join("state"))
            .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", self.root.join("spend"))
            .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", self.root.join("logs"))
            .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
            .env("AUTOSPEC_TEST_ISSUE_BODY", self.root.join("issue-body"))
            .env("AUTOSPEC_TEST_ISSUE_STATE", self.issue_state())
            .env("AUTOSPEC_TEST_GH_CALLS", self.gh_calls());
        let bin = self.root.join("bin");
        if bin.exists() {
            command.env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            );
        }
        command
    }

    fn record_immediate_stop(&self) {
        let output = self.command("stop").arg("--immediate").output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(self.stop_flag().exists());
    }

    #[cfg(unix)]
    fn install_closed_epic(&self) {
        let projection = "Existing accountable autonomous run";
        let manifest = accountability::RecoveryManifest::new(
            run(),
            12,
            "https://github.com/acme/widgets/issues/12",
            4,
            autospec_core::autonomous::waterfall::sha256_hex(format!("{projection}\n").as_bytes()),
            12,
            2,
        )
        .unwrap()
        .with_recovery_state(accountability::RecoveryState::Terminal, vec![], vec![])
        .unwrap();
        let marker = format!(
            "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
            run().run_id()
        );
        fs::write(
            self.root.join("issue-body"),
            accountability::github::compose_managed_body(
                &marker,
                projection,
                &manifest,
                "human notes",
            ),
        )
        .unwrap();
        fs::write(self.issue_state(), "CLOSED\n").unwrap();
        let bin = self.root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let gh = bin.join("gh");
        fs::write(&gh, r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$AUTOSPEC_TEST_GH_CALLS"
case "$1 $2 $3" in
  "issue view 12")
    body=$(awk 'BEGIN{ORS=""; printf "\""} {gsub(/\\/,"\\\\"); gsub(/\"/,"\\\""); if (NR>1) printf "\\n"; printf "%s",$0} END{printf "\""}' "$AUTOSPEC_TEST_ISSUE_BODY")
    printf '{"number":12,"url":"https://github.com/acme/widgets/issues/12","state":"%s","body":%s,"labels":[{"name":"epic"},{"name":"type:tracker"},{"name":"no-auto"},{"name":"autospec:run-accountability"}]}\n' "$(tr -d '\n' < "$AUTOSPEC_TEST_ISSUE_STATE")" "$body"
    ;;
  "issue reopen 12") printf 'OPEN\n' > "$AUTOSPEC_TEST_ISSUE_STATE" ;;
  "issue edit 12") cat > "$AUTOSPEC_TEST_ISSUE_BODY" ;;
  *) printf 'unexpected gh invocation: %s\n' "$*" >&2; exit 1 ;;
esac
"#).unwrap();
        fs::set_permissions(gh, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn stop_spawned_run(&self) {
        let _ = self.command("stop").arg("--immediate").output();
    }

    fn scope(&self) -> std::path::PathBuf {
        self.root.join("operator/acme_widgets")
    }
    fn stop_flag(&self) -> std::path::PathBuf {
        self.scope().join("stop.flag")
    }
    fn issue_state(&self) -> std::path::PathBuf {
        self.root.join("issue-state")
    }
    fn gh_calls(&self) -> std::path::PathBuf {
        self.root.join("gh-calls")
    }
}

impl Drop for CliResumeFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
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
