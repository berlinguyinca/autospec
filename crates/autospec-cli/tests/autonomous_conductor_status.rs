use autospec_core::coordination::{
    ConductorEvent, ConductorOutcome, ConductorScope, ConductorState,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const REPO: &str = "test/repo";

#[test]
fn status_json_and_timeline_render_normalized_conductor_state() {
    let fixture = StatusFixture::new();
    fixture.write_foreground_state(all_blocked_state());

    let status = fixture
        .command("status")
        .arg("--json")
        .output()
        .expect("status");
    assert!(
        status.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&status.stderr)
    );
    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(stdout.contains("\"normalized_state\":\"cycle 0: all-blocked;"));
    assert!(stdout.contains("affected issues=#42,#43"));

    let timeline = fixture.command("timeline").output().expect("timeline");
    assert!(
        timeline.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&timeline.stderr)
    );
    let stdout = String::from_utf8_lossy(&timeline.stdout);
    assert!(stdout.contains("time unknown - conductor cycle 0: all-blocked;"));
    assert!(stdout.contains("next=promote or unblock affected issues"));
}

struct StatusFixture {
    root: PathBuf,
    operator: PathBuf,
    state: PathBuf,
    spend: PathBuf,
    logs: PathBuf,
    repo_dir: PathBuf,
}

impl StatusFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "autospec-conductor-status-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let operator = root.join("operator");
        let state = root.join("state");
        let spend = root.join("spend");
        let logs = root.join("logs");
        let repo_dir = root.join("repo");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        fs::create_dir_all(operator.join(scope_slug())).expect("operator scope");
        fs::create_dir_all(&state).expect("state dir");
        fs::create_dir_all(&spend).expect("spend dir");
        fs::create_dir_all(&logs).expect("log dir");
        Self {
            root,
            operator,
            state,
            spend,
            logs,
            repo_dir,
        }
    }

    fn write_foreground_state(&self, state: ConductorState) {
        fs::write(
            self.operator
                .join(scope_slug())
                .join("foreground-conductor-repository.json"),
            format!("{}\n", state.to_json()),
        )
        .expect("write foreground state");
    }

    fn command(&self, subcommand: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_autospec"));
        command
            .args([
                "autonomous",
                subcommand,
                "--repo",
                REPO,
                "--repo-dir",
                self.repo_dir.to_str().expect("repo dir"),
            ])
            .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &self.operator)
            .env("AUTOSPEC_STATE_DIR", &self.state)
            .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", &self.spend)
            .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &self.logs);
        command
    }
}

impl Drop for StatusFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn all_blocked_state() -> ConductorState {
    selected_state()
        .transition(ConductorEvent::Claimed)
        .expect("claim")
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::AllBlocked {
                reason: "tier1_all_blocked".to_string(),
                issues: vec![42, 43].into_boxed_slice(),
            },
        })
        .expect("all blocked")
}

fn selected_state() -> ConductorState {
    ConductorState::new(REPO, ConductorScope::Repository, 2)
        .expect("state")
        .transition(ConductorEvent::ScanFoundWork)
        .expect("scan")
        .transition(ConductorEvent::SafetyReviewed)
        .expect("review")
        .transition(ConductorEvent::Selected {
            issue: 42,
            serialization_reasons: Vec::new(),
        })
        .expect("select")
}

fn scope_slug() -> &'static Path {
    Path::new("test_repo")
}
