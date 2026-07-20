use autospec_core::autonomous::no_work::{DryReason, NoWorkTier};
use autospec_core::autonomous::waterfall::{
    receipt_reference, FunnelCounts, SealedEvidence, TierReceipt, TierStatus, WaterfallState,
};
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
    fixture.write_foreground_state(
        all_blocked_state()
            .record_no_progress_cycle("tier1_all_blocked")
            .expect("no-progress diagnostic"),
    );

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
    assert!(stdout.contains("\"no_progress_reason\":\"tier1_all_blocked\""));
    assert!(stdout.contains("\"no_progress_cycles\":1"));

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

#[test]
fn status_json_derives_tier_dry_from_persisted_waterfall_receipt() {
    let fixture = StatusFixture::new();
    fixture.write_foreground_state(
        ConductorState::new(REPO, ConductorScope::Repository, 2).expect("state"),
    );
    fixture.write_waterfall_receipts(&[tier_receipt(
        1,
        NoWorkTier::Tier1,
        DryReason::NoProposalsGenerated,
    )]);

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

    assert!(stdout.contains("\"normalized_state\":\"cycle 0: tier-dry;"));
    assert!(stdout.contains("tier=tier1"));
    assert!(stdout.contains("next=advance waterfall to tier1_5"));
}

#[test]
fn timeline_derives_idle_rescan_from_completed_waterfall_pass() {
    let fixture = StatusFixture::new();
    fixture.write_foreground_state(
        ConductorState::new(REPO, ConductorScope::Repository, 2).expect("state"),
    );
    fixture.write_waterfall_receipts(&[
        tier_receipt(1, NoWorkTier::Tier1, DryReason::NoProposalsGenerated),
        tier_receipt(1, NoWorkTier::Tier1_5, DryReason::NoProposalsGenerated),
        tier_receipt(1, NoWorkTier::Tier2, DryReason::VerificationRejected),
        tier_receipt(1, NoWorkTier::Tier3, DryReason::NoMetadataFindings),
        tier_receipt(1, NoWorkTier::Tier4, DryReason::RoiFiltered),
    ]);

    let timeline = fixture.command("timeline").output().expect("timeline");
    assert!(
        timeline.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&timeline.stderr)
    );
    let stdout = String::from_utf8_lossy(&timeline.stdout);

    assert!(stdout.contains("time unknown - conductor cycle 0: idle-rescan;"));
    assert!(stdout.contains("action=rescan"));
    assert!(stdout.contains("next=sleep until the next rescan interval"));
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

    fn write_waterfall_receipts(&self, receipts: &[TierReceipt]) {
        let root = self.operator.join(scope_slug()).join("waterfall");
        let mut state = WaterfallState::new(REPO, 1, NoWorkTier::Tier1).expect("waterfall state");
        for receipt in receipts {
            let path = root.join(receipt_reference(receipt.pass_id(), receipt.tier()));
            fs::create_dir_all(path.parent().expect("receipt parent")).expect("receipt dir");
            fs::write(&path, format!("{}\n", receipt.to_json())).expect("receipt");
            state = state.record_receipt(receipt).expect("record receipt");
        }
        fs::create_dir_all(&root).expect("waterfall root");
        fs::write(
            root.join("waterfall-state.json"),
            format!("{}\n", state.to_json()),
        )
        .expect("waterfall state");
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

fn tier_receipt(pass_id: u64, tier: NoWorkTier, reason: DryReason) -> TierReceipt {
    TierReceipt::new(
        REPO,
        pass_id,
        tier,
        "test-producer",
        1,
        2,
        TierStatus::Exhausted { reason },
        FunnelCounts::new(0, 0, 0, 0, 0).expect("funnel"),
        vec![SealedEvidence::new(
            format!("waterfall/{pass_id}/{}.json", tier.as_str()),
            format!("{:064x}", pass_id * 10 + tier_index(tier)),
        )
        .expect("evidence")],
    )
    .expect("receipt")
}

fn tier_index(tier: NoWorkTier) -> u64 {
    match tier {
        NoWorkTier::Tier1 => 1,
        NoWorkTier::Tier1_5 => 2,
        NoWorkTier::Tier2 => 3,
        NoWorkTier::Tier3 => 4,
        NoWorkTier::Tier4 => 5,
    }
}
