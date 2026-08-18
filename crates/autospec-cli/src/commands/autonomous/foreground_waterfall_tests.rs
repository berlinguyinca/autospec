use std::cell::Cell;
use std::fs;
use std::process::Command;

use autospec_core::autonomous::config::AutonomousConfig;
use autospec_core::autonomous::no_work::NoWorkTier;
use autospec_core::coordination::{QueueGateCounts, ReadyQueuePlan, WorkerCap};

use super::foreground_waterfall::{
    collect_after_preflight, run_injected, run_one_tier, ForegroundWaterfallProgress,
    InjectedProgress,
};
use super::resilience::{acquire_test_lifecycle, replace_test_lifecycle_generation};
use super::tier15_receipts::ReceiptPreflight;
use super::tier2_receipts_tests::{TempRoot, REPO};
use super::tier4_receipts_tests::seed_tier_four_cursor;
use super::waterfall::{StoreAcquisition, WaterfallStore};
use super::waterfall_coordinator::Tier1QueueEvidence;
use super::waterfall_policy::WaterfallPolicy;

const CLOSED_ORDER: [NoWorkTier; 5] = [
    NoWorkTier::Tier1,
    NoWorkTier::Tier1_5,
    NoWorkTier::Tier2,
    NoWorkTier::Tier3,
    NoWorkTier::Tier4,
];
const ISOLATED_TIER4_TEST: &str = "commands::autonomous::foreground_waterfall_tests::nonempty_tier4_config_advances_disabled_policy_without_fetching_isolated";

#[test]
fn driver_runs_only_the_current_tier_and_returns_the_reloaded_cursor() {
    let fixture = DriverFixture::at(NoWorkTier::Tier1);
    let (progress, calls) =
        fixture.run_with_outcomes([(NoWorkTier::Tier1, InjectedProgress::Advanced)]);

    assert_eq!(
        progress,
        ForegroundWaterfallProgress::Pending {
            tier: NoWorkTier::Tier1_5
        }
    );
    assert_eq!(calls, vec![NoWorkTier::Tier1]);
    assert_eq!(fixture.reloads.get(), 1);
}

#[test]
fn driver_retains_pending_produced_failed_blocked_and_not_run() {
    for outcome in DriverFixture::retained_outcomes() {
        let fixture = DriverFixture::at(NoWorkTier::Tier2);
        let expected = outcome.expected_progress(NoWorkTier::Tier2);
        let (progress, calls) = fixture.run_with_outcomes([(NoWorkTier::Tier2, outcome)]);

        assert_eq!(progress, expected);
        assert_eq!(calls, vec![NoWorkTier::Tier2]);
        assert_eq!(fixture.cursor(), NoWorkTier::Tier2);
        assert_eq!(fixture.reloads.get(), 0);
    }
}

#[test]
fn retained_progress_precedes_later_tier_producer_construction() {
    for retained in DriverFixture::retained_outcomes() {
        let constructed = Cell::new(false);

        let progress =
            collect_after_preflight(ReceiptPreflight::Replayed(retained.clone()), || {
                constructed.set(true);
                Ok(InjectedProgress::Advanced)
            })
            .expect("replay retained progress");

        assert_eq!(progress, retained);
        assert!(!constructed.get());
    }
}

#[test]
fn closed_tier_order_is_exact_and_each_advance_stops_after_one_call() {
    for (index, tier) in CLOSED_ORDER.into_iter().enumerate() {
        let fixture = DriverFixture::at(tier);
        let (progress, calls) = fixture.run_with_outcomes([(tier, InjectedProgress::Advanced)]);
        let next = CLOSED_ORDER[(index + 1) % CLOSED_ORDER.len()];

        assert_eq!(
            progress,
            ForegroundWaterfallProgress::Pending { tier: next }
        );
        assert_eq!(calls, vec![tier]);
        assert_eq!(fixture.cursor(), next);
    }
}

#[test]
fn nonempty_tier4_config_advances_disabled_policy_without_fetching() {
    let output = Command::new(std::env::current_exe().expect("current test executable"))
        .args(["--ignored", "--exact", ISOLATED_TIER4_TEST, "--nocapture"])
        .output()
        .expect("run isolated Tier 4 dispatch test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let receipt = format!("test {ISOLATED_TIER4_TEST} ... ok");
    let receipt_count = stdout.lines().filter(|line| *line == receipt).count();
    assert!(
        output.status.success() && receipt_count == 1,
        "isolated Tier 4 dispatch emitted {receipt_count} exact receipts: stdout={stdout} stderr={stderr}"
    );
}

#[test]
#[ignore = "launched in isolation by the Tier 4 dispatch test"]
fn nonempty_tier4_config_advances_disabled_policy_without_fetching_isolated() {
    let root = TempRoot::new();
    seed_tier_four_cursor(&root);
    let lease = acquire_test_lifecycle(root.path(), REPO).expect("lifecycle lease");
    let config = configured_tier4();
    let policy = WaterfallPolicy::from_config(&config).expect("derive policy");
    let plan = empty_plan();

    let progress = run_one_tier(
        root.path(),
        REPO,
        root.path(),
        &lease,
        &config,
        &policy,
        Tier1QueueEvidence::EmptyPage(&plan),
    )
    .expect("disabled Tier 4 dispatch");

    assert_eq!(
        progress,
        ForegroundWaterfallProgress::Pending {
            tier: NoWorkTier::Tier1
        }
    );
    assert_eq!(source_fetch_count(&root), 0);
}

#[test]
fn waterfall_lock_contention_is_pending_and_never_blocked() {
    let root = TempRoot::new();
    let lease = acquire_test_lifecycle(root.path(), REPO).expect("lifecycle lease");
    let held = match WaterfallStore::acquire(root.path().join("waterfall"), REPO)
        .expect("waterfall lock")
    {
        StoreAcquisition::Acquired(store) => store,
        StoreAcquisition::Held => panic!("fresh fixture lock is unexpectedly held"),
    };
    let config = AutonomousConfig::default();
    let policy = WaterfallPolicy::from_config(&config).expect("derive policy");
    let plan = empty_plan();

    let progress = run_one_tier(
        root.path(),
        REPO,
        root.path(),
        &lease,
        &config,
        &policy,
        Tier1QueueEvidence::EmptyPage(&plan),
    )
    .expect("contended dispatch");

    assert_eq!(
        progress,
        ForegroundWaterfallProgress::Pending {
            tier: NoWorkTier::Tier1
        }
    );
    drop(held);
}

#[test]
fn stale_lease_cannot_probe_or_create_waterfall_state() {
    let lease_root = TempRoot::new();
    let operator_root = TempRoot::new();
    let lease = acquire_test_lifecycle(lease_root.path(), REPO).expect("lifecycle lease");
    replace_test_lifecycle_generation(&lease).expect("replace lease generation");
    let config = AutonomousConfig::default();
    let policy = WaterfallPolicy::from_config(&config).expect("derive policy");
    let plan = empty_plan();

    let error = run_one_tier(
        operator_root.path(),
        REPO,
        operator_root.path(),
        &lease,
        &config,
        &policy,
        Tier1QueueEvidence::EmptyPage(&plan),
    )
    .expect_err("stale lease must fail before cursor probing");

    assert!(
        error.contains("token") || error.contains("generation"),
        "unexpected stale lease error: {error}"
    );
    assert!(!operator_root.path().join("waterfall").exists());
}

#[test]
fn dispatcher_has_no_no_work_ideation_or_execution_authority() {
    let source = include_str!("foreground_waterfall.rs");
    let production = source
        .split("\n#[cfg(test)]")
        .next()
        .expect("production source");

    assert!(production.contains("TIER15_OBSERVATION_BUDGET: usize = 5"));
    let dispatch = production
        .split("fn dispatch_tier")
        .nth(1)
        .expect("tier dispatcher")
        .split("fn finish_progress")
        .next()
        .expect("dispatcher body");
    assert!(!dispatch.contains("Blocked"));
    for forbidden in [
        "why_no_work",
        "ideat",
        "queue::",
        "claim::",
        "executor",
        "std::process",
        "Command",
        "bash",
        "sh -c",
        "repo_root",
    ] {
        assert!(
            !production.contains(forbidden),
            "dispatcher retains prohibited authority: {forbidden}"
        );
    }
}

struct DriverFixture {
    cursor: Cell<NoWorkTier>,
    reloads: Cell<usize>,
}

impl DriverFixture {
    fn at(tier: NoWorkTier) -> Self {
        Self {
            cursor: Cell::new(tier),
            reloads: Cell::new(0),
        }
    }

    fn retained_outcomes() -> Vec<InjectedProgress> {
        vec![
            InjectedProgress::Pending,
            InjectedProgress::Produced(2),
            InjectedProgress::Failed("producer failed".to_string()),
            InjectedProgress::Blocked("policy blocked".to_string()),
            InjectedProgress::NotRun("producer disabled".to_string()),
        ]
    }

    fn run_with_outcomes<const N: usize>(
        &self,
        outcomes: [(NoWorkTier, InjectedProgress); N],
    ) -> (ForegroundWaterfallProgress, Vec<NoWorkTier>) {
        let outcomes = Vec::from(outcomes);
        let mut calls = Vec::new();
        let current = self.cursor.get();
        let progress = run_injected(
            current,
            |tier| {
                calls.push(tier);
                outcomes
                    .iter()
                    .find(|(expected, _)| *expected == tier)
                    .map(|(_, outcome)| outcome.clone())
                    .ok_or_else(|| format!("unexpected tier {tier:?}"))
            },
            || {
                self.reloads.set(self.reloads.get() + 1);
                let next = next_tier(current);
                self.cursor.set(next);
                Ok(next)
            },
        )
        .expect("injected dispatch");
        (progress, calls)
    }

    fn cursor(&self) -> NoWorkTier {
        self.cursor.get()
    }
}

impl InjectedProgress {
    fn expected_progress(&self, tier: NoWorkTier) -> ForegroundWaterfallProgress {
        match self {
            Self::Pending => ForegroundWaterfallProgress::Pending { tier },
            Self::Advanced => unreachable!("advanced reloads the cursor"),
            Self::Produced(count) => ForegroundWaterfallProgress::Produced {
                tier,
                count: *count,
            },
            Self::Failed(reason) => ForegroundWaterfallProgress::Failed {
                tier,
                reason: reason.clone(),
            },
            Self::Blocked(reason) => ForegroundWaterfallProgress::Blocked {
                tier,
                reason: reason.clone(),
            },
            Self::NotRun(reason) => ForegroundWaterfallProgress::NotRun {
                tier,
                reason: reason.clone(),
            },
        }
    }
}

fn next_tier(tier: NoWorkTier) -> NoWorkTier {
    match tier {
        NoWorkTier::Tier1 => NoWorkTier::Tier1_5,
        NoWorkTier::Tier1_5 => NoWorkTier::Tier2,
        NoWorkTier::Tier2 => NoWorkTier::Tier3,
        NoWorkTier::Tier3 => NoWorkTier::Tier4,
        NoWorkTier::Tier4 => NoWorkTier::Tier1,
    }
}

fn configured_tier4() -> AutonomousConfig {
    AutonomousConfig::parse(
        "tier4:\n  sources:\n    - id: alpha\n      host: alpha.example.test\n      path: /facts\n      max_bytes: 1024\n      deadline_millis: 1000\n",
    )
    .expect("configured Tier 4")
}

fn empty_plan() -> ReadyQueuePlan {
    ReadyQueuePlan {
        ready: Vec::new(),
        blocked: Vec::new(),
        claimed: Vec::new(),
        conflicts: Vec::new(),
        worker_cap: WorkerCap {
            max_repo_workers: 1,
            active_count: 0,
            remaining: 1,
            reached: false,
        },
        batch: Vec::new(),
        gate_counts: QueueGateCounts::default(),
    }
}

fn source_fetch_count(root: &TempRoot) -> usize {
    let tier4 = root.path().join("waterfall/1/tier4");
    ["source-policy.json", "sources.json"]
        .into_iter()
        .filter(|name| fs::metadata(tier4.join(name)).is_ok())
        .count()
}
