use std::fs;
use std::path::Path;

use autospec_core::autonomous::config::AutonomousConfig;
use autospec_core::autonomous::no_work::NoWorkTier;
use autospec_core::autonomous::waterfall::WaterfallState;

use super::resilience::{with_current_lifecycle_lease, ConductorLease};
use super::tier15;
use super::tier15_receipts::{record_tier15_with_lease, Tier15Progress};
use super::tier2;
use super::tier2_receipts::{record_tier2_with_lease, Tier2Progress};
use super::tier3;
use super::tier3_receipts::{record_tier3_with_lease, Tier3Progress};
use super::tier4;
use super::tier4_receipts::{record_tier4_with_lease, Tier4Progress};
use super::waterfall::{StoreAcquisition, WaterfallStore, WaterfallStoreError};
use super::waterfall_coordinator::{record_tier_one, Tier1Progress, Tier1QueueEvidence};
use super::waterfall_policy::WaterfallPolicy;

const TIER15_OBSERVATION_BUDGET: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ForegroundWaterfallProgress {
    Pending { tier: NoWorkTier },
    Produced { tier: NoWorkTier, count: u64 },
    Failed { tier: NoWorkTier, reason: String },
    Blocked { tier: NoWorkTier, reason: String },
    NotRun { tier: NoWorkTier, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DispatchProgress {
    Pending,
    Advanced,
    Produced(u64),
    Failed(String),
    Blocked(String),
    NotRun(String),
}

struct CursorRead {
    tier: NoWorkTier,
    trusted: bool,
}

pub(super) fn run_one_tier(
    state_root: &Path,
    repo: &str,
    lease: &ConductorLease,
    config: &AutonomousConfig,
    tier1_evidence: Tier1QueueEvidence<'_>,
) -> Result<ForegroundWaterfallProgress, String> {
    let policy = WaterfallPolicy::from_config(config)?;
    let cursor =
        with_current_lifecycle_lease(lease, || read_cursor_fenced(state_root, repo, &policy))?;
    if !cursor.trusted {
        return Ok(ForegroundWaterfallProgress::Pending { tier: cursor.tier });
    }
    finish_progress(
        cursor.tier,
        |tier| {
            dispatch_tier(
                tier,
                state_root,
                repo,
                lease,
                config,
                &policy,
                tier1_evidence,
            )
        },
        || {
            with_current_lifecycle_lease(lease, || read_cursor_fenced(state_root, repo, &policy))
                .map(|cursor| cursor.tier)
        },
    )
}

fn dispatch_tier(
    tier: NoWorkTier,
    state_root: &Path,
    repo: &str,
    lease: &ConductorLease,
    config: &AutonomousConfig,
    policy: &WaterfallPolicy,
    tier1_evidence: Tier1QueueEvidence<'_>,
) -> Result<DispatchProgress, String> {
    match tier {
        NoWorkTier::Tier1 => {
            record_tier_one(state_root, repo, lease, policy, tier1_evidence).map(map_tier1)
        }
        NoWorkTier::Tier1_5 => {
            let scan = tier15::scan(repo, TIER15_OBSERVATION_BUDGET);
            record_tier15_with_lease(state_root, repo, lease, scan).map(map_tier15)
        }
        NoWorkTier::Tier2 => record_tier2_with_lease(
            state_root,
            repo,
            lease,
            tier2::disabled_by_checked_in_policy(),
        )
        .map(map_tier2),
        NoWorkTier::Tier3 => record_tier3_with_lease(
            state_root,
            repo,
            lease,
            tier3::disabled_by_checked_in_policy(),
        )
        .map(map_tier3),
        NoWorkTier::Tier4 => record_tier4_with_lease(
            state_root,
            repo,
            lease,
            policy,
            tier4::disabled_by_checked_in_policy(&config.tier4),
        )
        .map(map_tier4),
    }
}

fn finish_progress(
    tier: NoWorkTier,
    run: impl FnOnce(NoWorkTier) -> Result<DispatchProgress, String>,
    reload_cursor: impl FnOnce() -> Result<NoWorkTier, String>,
) -> Result<ForegroundWaterfallProgress, String> {
    Ok(match run(tier)? {
        DispatchProgress::Pending => ForegroundWaterfallProgress::Pending { tier },
        DispatchProgress::Advanced => ForegroundWaterfallProgress::Pending {
            tier: reload_cursor()?,
        },
        DispatchProgress::Produced(count) => ForegroundWaterfallProgress::Produced { tier, count },
        DispatchProgress::Failed(reason) => ForegroundWaterfallProgress::Failed { tier, reason },
        DispatchProgress::Blocked(reason) => ForegroundWaterfallProgress::Blocked { tier, reason },
        DispatchProgress::NotRun(reason) => ForegroundWaterfallProgress::NotRun { tier, reason },
    })
}

fn read_cursor_fenced(
    state_root: &Path,
    repo: &str,
    policy: &WaterfallPolicy,
) -> Result<CursorRead, String> {
    match WaterfallStore::acquire_with_policy(state_root.join("waterfall"), repo, policy)
        .map_err(store_error)?
    {
        StoreAcquisition::Acquired(store) => Ok(CursorRead {
            tier: store
                .load_state()
                .map_err(store_error)?
                .map_or(NoWorkTier::Tier1, |state| state.current_tier()),
            trusted: true,
        }),
        StoreAcquisition::Held => Ok(CursorRead {
            tier: peek_cursor(state_root, repo)?,
            trusted: false,
        }),
    }
}

fn peek_cursor(state_root: &Path, repo: &str) -> Result<NoWorkTier, String> {
    let path = state_root.join("waterfall/waterfall-state.json");
    match fs::read_to_string(&path) {
        Ok(document) => {
            WaterfallState::parse_json(&document, repo).map(|state| state.current_tier())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(NoWorkTier::Tier1),
        Err(error) => Err(format!(
            "cannot read contended waterfall cursor {}: {error}",
            path.display()
        )),
    }
}

fn map_tier1(progress: Tier1Progress) -> DispatchProgress {
    match progress {
        Tier1Progress::Pending => DispatchProgress::Pending,
        Tier1Progress::Advanced => DispatchProgress::Advanced,
        Tier1Progress::Failed(reason) => DispatchProgress::Failed(reason),
    }
}

fn map_tier15(progress: Tier15Progress) -> DispatchProgress {
    match progress {
        Tier15Progress::Pending => DispatchProgress::Pending,
        Tier15Progress::Advanced => DispatchProgress::Advanced,
        Tier15Progress::Produced(count) => DispatchProgress::Produced(count),
        Tier15Progress::Failed(reason) => DispatchProgress::Failed(reason),
    }
}

fn map_tier2(progress: Tier2Progress) -> DispatchProgress {
    match progress {
        Tier2Progress::Pending => DispatchProgress::Pending,
        Tier2Progress::Advanced => DispatchProgress::Advanced,
        Tier2Progress::Produced(count) => DispatchProgress::Produced(count),
        Tier2Progress::Failed(reason) => DispatchProgress::Failed(reason),
        Tier2Progress::NotRun(reason) => DispatchProgress::NotRun(reason),
    }
}

fn map_tier3(progress: Tier3Progress) -> DispatchProgress {
    match progress {
        Tier3Progress::Pending => DispatchProgress::Pending,
        Tier3Progress::Advanced => DispatchProgress::Advanced,
        Tier3Progress::Produced(count) => DispatchProgress::Produced(count),
        Tier3Progress::Failed(reason) => DispatchProgress::Failed(reason),
        Tier3Progress::NotRun(reason) => DispatchProgress::NotRun(reason),
    }
}

fn map_tier4(progress: Tier4Progress) -> DispatchProgress {
    match progress {
        Tier4Progress::Pending => DispatchProgress::Pending,
        Tier4Progress::Advanced => DispatchProgress::Advanced,
        Tier4Progress::Produced(count) => DispatchProgress::Produced(count),
        Tier4Progress::Failed(reason) => DispatchProgress::Failed(reason),
        Tier4Progress::NotRun(reason) => DispatchProgress::NotRun(reason),
    }
}

fn store_error(error: WaterfallStoreError) -> String {
    match error {
        WaterfallStoreError::Diagnostic(reason)
        | WaterfallStoreError::InvalidReceipt(reason)
        | WaterfallStoreError::InvalidState(reason) => reason,
    }
}

#[cfg(test)]
pub(super) type InjectedProgress = DispatchProgress;

#[cfg(test)]
pub(super) fn run_injected(
    tier: NoWorkTier,
    run: impl FnOnce(NoWorkTier) -> Result<InjectedProgress, String>,
    reload_cursor: impl FnOnce() -> Result<NoWorkTier, String>,
) -> Result<ForegroundWaterfallProgress, String> {
    finish_progress(tier, run, reload_cursor)
}
