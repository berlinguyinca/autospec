use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
use std::thread;
#[cfg(test)]
use std::time::{Duration, Instant};

use autospec_core::autonomous::no_work::{DryReason, NoWorkTier};
use autospec_core::autonomous::tier4::Tier4SourcePolicy;
use autospec_core::autonomous::waterfall::{
    receipt_reference, SealedEvidence, TierReceipt, TierStatus, WaterfallState,
};

mod evidence;
mod tier_evidence;

use super::waterfall_policy::WaterfallPolicy;
use evidence::WaterfallEvidenceArtifact;
pub(super) use evidence::{
    Tier15EvidenceArtifact, Tier1EvidenceArtifact, Tier2EvidenceArtifact, Tier3EvidenceArtifact,
    Tier4EvidenceArtifact,
};

static ATOMIC_WRITE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) enum StoreAcquisition {
    Acquired(WaterfallStore),
    Held,
}

#[derive(Debug)]
pub(super) enum WaterfallStoreError {
    Diagnostic(String),
    InvalidReceipt(String),
    InvalidState(String),
}

pub(super) struct WaterfallStore {
    repo: String,
    root: PathBuf,
    state_path: PathBuf,
    expected_tier4_source_policy: Option<Tier4SourcePolicy>,
    _lock: File,
}

impl WaterfallStore {
    pub(super) fn acquire(
        root: impl AsRef<Path>,
        repo: impl Into<String>,
    ) -> Result<StoreAcquisition, WaterfallStoreError> {
        Self::acquire_for_receipts(root, repo, None)
    }

    pub(super) fn acquire_with_policy(
        root: impl AsRef<Path>,
        repo: impl Into<String>,
        policy: &WaterfallPolicy,
    ) -> Result<StoreAcquisition, WaterfallStoreError> {
        Self::acquire_for_receipts(root, repo, policy.tier4_source().cloned())
    }

    #[cfg(test)]
    pub(in crate::commands::autonomous) fn acquire_with_tier4_source_policy(
        root: impl AsRef<Path>,
        repo: impl Into<String>,
        expected_tier4_source_policy: Tier4SourcePolicy,
    ) -> Result<StoreAcquisition, WaterfallStoreError> {
        Self::acquire_with_optional_tier4_source_policy(
            root,
            repo,
            Some(expected_tier4_source_policy),
        )
    }

    pub(super) fn acquire_for_receipts(
        root: impl AsRef<Path>,
        repo: impl Into<String>,
        expected_tier4_source_policy: Option<Tier4SourcePolicy>,
    ) -> Result<StoreAcquisition, WaterfallStoreError> {
        let root = root.as_ref().to_path_buf();
        let repo = repo.into();
        #[cfg(test)]
        {
            retry_transient_lock(
                || {
                    Self::acquire_with_optional_tier4_source_policy(
                        &root,
                        repo.clone(),
                        expected_tier4_source_policy.clone(),
                    )
                },
                |result| matches!(result, Ok(StoreAcquisition::Held)),
            )
        }
        #[cfg(not(test))]
        Self::acquire_with_optional_tier4_source_policy(root, repo, expected_tier4_source_policy)
    }

    fn acquire_with_optional_tier4_source_policy(
        root: impl AsRef<Path>,
        repo: impl Into<String>,
        expected_tier4_source_policy: Option<Tier4SourcePolicy>,
    ) -> Result<StoreAcquisition, WaterfallStoreError> {
        let repo = repo.into();
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).map_err(|error| {
            WaterfallStoreError::Diagnostic(format!(
                "cannot create waterfall store {}: {error}",
                root.display()
            ))
        })?;
        let lock_path = root.join(".waterfall.lock");
        match try_lock(&lock_path)? {
            Some(lock) => Ok(StoreAcquisition::Acquired(Self {
                repo,
                state_path: root.join("waterfall-state.json"),
                root,
                expected_tier4_source_policy,
                _lock: lock,
            })),
            None => Ok(StoreAcquisition::Held),
        }
    }

    pub(super) fn state_path(&self) -> &Path {
        // The state filename is intentionally stable: Task 3 resumes only from this cursor.
        &self.state_path
    }

    #[cfg(test)]
    pub(in crate::commands::autonomous) fn clone_lock_for_test(&self) -> File {
        self._lock.try_clone().expect("clone waterfall lock")
    }

    pub(super) fn receipt_path(
        &self,
        receipt: &TierReceipt,
    ) -> Result<PathBuf, WaterfallStoreError> {
        self.validate_receipt(receipt)?;
        Ok(self.root.join(receipt.reference()))
    }

    pub(super) fn persist_receipt(&self, receipt: &TierReceipt) -> Result<(), WaterfallStoreError> {
        self.validate_receipt(receipt)?;
        if let Some(existing) = self.load_receipt(receipt.pass_id(), receipt.tier())? {
            if existing == *receipt {
                return Ok(());
            }
            return Err(WaterfallStoreError::InvalidReceipt(
                "conflicting sealed waterfall receipt".to_string(),
            ));
        }
        let path = self.receipt_path(receipt)?;
        atomic_write(&path, &format!("{}\n", receipt.to_json()))
    }

    pub(super) fn persist_tier1_evidence(
        &self,
        pass_id: u64,
        artifact: Tier1EvidenceArtifact,
        contents: &str,
    ) -> Result<SealedEvidence, WaterfallStoreError> {
        evidence::persist(
            &self.root,
            pass_id,
            WaterfallEvidenceArtifact::Tier1(artifact),
            contents,
        )
    }

    pub(super) fn verify_tier1_evidence(
        &self,
        pass_id: u64,
        receipt: &TierReceipt,
    ) -> Result<(), WaterfallStoreError> {
        evidence::verify_tier1(&self.root, pass_id, receipt)
    }

    pub(super) fn persist_tier15_evidence(
        &self,
        pass_id: u64,
        artifact: Tier15EvidenceArtifact,
        contents: &str,
    ) -> Result<SealedEvidence, WaterfallStoreError> {
        evidence::persist(
            &self.root,
            pass_id,
            WaterfallEvidenceArtifact::Tier15(artifact),
            contents,
        )
    }

    pub(super) fn verify_tier15_evidence(
        &self,
        pass_id: u64,
        receipt: &TierReceipt,
    ) -> Result<(), WaterfallStoreError> {
        evidence::verify_tier15(&self.root, pass_id, receipt)
    }

    pub(super) fn load_receipt(
        &self,
        pass_id: u64,
        tier: NoWorkTier,
    ) -> Result<Option<TierReceipt>, WaterfallStoreError> {
        let path = self.root.join(receipt_reference(pass_id, tier));
        match fs::read_to_string(&path) {
            Ok(document) => TierReceipt::parse_json(&document, &self.repo, pass_id, tier)
                .map(Some)
                .map_err(WaterfallStoreError::InvalidReceipt),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(WaterfallStoreError::Diagnostic(format!(
                "cannot read waterfall receipt {}: {error}",
                path.display()
            ))),
        }
    }

    pub(super) fn load_state(&self) -> Result<Option<WaterfallState>, WaterfallStoreError> {
        let path = self.state_path();
        match fs::read_to_string(path) {
            Ok(document) => {
                let state = WaterfallState::parse_json(&document, &self.repo)
                    .map_err(WaterfallStoreError::InvalidState)?;
                self.verify_state_receipts(&state)?;
                Ok(Some(state))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(WaterfallStoreError::Diagnostic(format!(
                "cannot read waterfall state {}: {error}",
                path.display()
            ))),
        }
    }

    pub(super) fn persist_state(&self, state: &WaterfallState) -> Result<(), WaterfallStoreError> {
        let document = state.to_json();
        let checked = WaterfallState::parse_json(&document, &self.repo)
            .map_err(WaterfallStoreError::InvalidState)?;
        self.verify_state_receipts(&checked)?;
        atomic_write(self.state_path(), &format!("{document}\n"))
    }

    fn validate_receipt(&self, receipt: &TierReceipt) -> Result<(), WaterfallStoreError> {
        TierReceipt::parse_json(
            &receipt.to_json(),
            &self.repo,
            receipt.pass_id(),
            receipt.tier(),
        )
        .map(|_| ())
        .map_err(WaterfallStoreError::InvalidReceipt)
    }

    fn verify_state_receipts(&self, state: &WaterfallState) -> Result<(), WaterfallStoreError> {
        let receipt_pass_id =
            if state.current_tier() == NoWorkTier::Tier1 && state.next_pass_id() > 1 {
                state.next_pass_id() - 1
            } else {
                state.next_pass_id()
            };
        for completed in state.completed_receipts() {
            let receipt = self
                .load_receipt(receipt_pass_id, completed.tier)
                .map_err(state_receipt_error)?
                .ok_or_else(|| {
                    WaterfallStoreError::InvalidState(format!(
                        "waterfall state references missing receipt {}",
                        completed.reference
                    ))
                })?;
            if receipt.digest() != completed.digest || receipt.reference() != completed.reference {
                return Err(WaterfallStoreError::InvalidState(format!(
                    "waterfall state receipt does not match sealed digest: {}",
                    completed.reference
                )));
            }
            if !is_advancing_completed_receipt(&receipt) {
                return Err(WaterfallStoreError::InvalidState(format!(
                    "completed {} receipt has a non-advancing status",
                    completed.tier.as_str()
                )));
            }
            match completed.tier {
                NoWorkTier::Tier1 | NoWorkTier::Tier1_5 => {
                    match completed.tier {
                        NoWorkTier::Tier1 => {
                            evidence::verify_tier1(&self.root, receipt_pass_id, &receipt)
                        }
                        NoWorkTier::Tier1_5 => {
                            evidence::verify_tier15(&self.root, receipt_pass_id, &receipt)
                        }
                        _ => unreachable!("closed early-tier match"),
                    }
                    .map_err(state_receipt_error)?;
                }
                NoWorkTier::Tier2 => self
                    .verify_tier2_evidence(receipt_pass_id, &receipt)
                    .map_err(state_receipt_error)?,
                NoWorkTier::Tier3 => {
                    self.verify_tier3_evidence(receipt_pass_id, &receipt)
                        .map_err(state_receipt_error)?;
                }
                NoWorkTier::Tier4 => {
                    self.verify_tier4_evidence(receipt_pass_id, &receipt)
                        .map_err(state_receipt_error)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub(super) fn retry_transient_lock<T>(
    mut operation: impl FnMut() -> T,
    is_transient: impl Fn(&T) -> bool,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let result = operation();
        if !is_transient(&result) || Instant::now() >= deadline {
            return result;
        }
        thread::sleep(Duration::from_millis(1));
    }
}

fn is_advancing_completed_receipt(receipt: &TierReceipt) -> bool {
    matches!(
        (receipt.tier(), receipt.status()),
        (
            NoWorkTier::Tier1 | NoWorkTier::Tier1_5,
            TierStatus::Exhausted {
                reason: DryReason::NoProposalsGenerated,
            },
        ) | (
            NoWorkTier::Tier2 | NoWorkTier::Tier4,
            TierStatus::Exhausted {
                reason: DryReason::NoProposalsGenerated
                    | DryReason::VerificationRejected
                    | DryReason::RoiFiltered,
            },
        ) | (
            NoWorkTier::Tier3,
            TierStatus::Exhausted {
                reason: DryReason::NoMetadataFindings,
            },
        )
    ) || matches!(
        (receipt.tier(), receipt.producer_version(), receipt.status()),
        (
            NoWorkTier::Tier2,
            "rust-tier2-local-receipts-v1",
            TierStatus::Produced { count },
        ) if *count > 0
    ) || matches!(
        (receipt.tier(), receipt.producer_version(), receipt.status()),
        (
            NoWorkTier::Tier3,
            "rust-tier3-disabled-policy-v1",
            TierStatus::NotRun { reason },
        ) if reason == autospec_core::autonomous::tier3::DISABLED_REASON
    ) || matches!(
        (receipt.tier(), receipt.producer_version(), receipt.status()),
        (
            NoWorkTier::Tier4,
            "rust-tier4-disabled-policy-v1",
            TierStatus::NotRun { reason },
        ) if reason == autospec_core::autonomous::tier4::DISABLED_REASON
    )
}

fn state_receipt_error(error: WaterfallStoreError) -> WaterfallStoreError {
    let mut detail = match error {
        WaterfallStoreError::Diagnostic(detail)
        | WaterfallStoreError::InvalidReceipt(detail)
        | WaterfallStoreError::InvalidState(detail) => detail,
    };
    if detail == "Tier 4 source policy does not match the trusted checked-in policy" {
        detail = "Tier 4 evidence does not match the trusted source policy".to_string();
    }
    WaterfallStoreError::InvalidState(format!("cannot validate state receipt: {detail}"))
}

fn try_lock(path: &Path) -> Result<Option<File>, WaterfallStoreError> {
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map_err(|error| {
            WaterfallStoreError::Diagnostic(format!(
                "cannot open waterfall lock {}: {error}",
                path.display()
            ))
        })?;

    #[cfg(unix)]
    {
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(fs::TryLockError::WouldBlock) => Ok(None),
            Err(fs::TryLockError::Error(error)) => Err(WaterfallStoreError::Diagnostic(format!(
                "cannot lock waterfall store {}: {error}",
                path.display()
            ))),
        }
    }

    #[cfg(not(unix))]
    {
        let _ = file;
        Err(WaterfallStoreError::Diagnostic(
            "waterfall persistence requires Unix flock support".to_string(),
        ))
    }
}

fn atomic_write(path: &Path, contents: &str) -> Result<(), WaterfallStoreError> {
    let parent = path.parent().ok_or_else(|| {
        WaterfallStoreError::Diagnostic(format!("waterfall path has no parent: {}", path.display()))
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        WaterfallStoreError::Diagnostic(format!(
            "cannot create waterfall directory {}: {error}",
            parent.display()
        ))
    })?;
    let temporary = temporary_path(path);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| {
            WaterfallStoreError::Diagnostic(format!(
                "cannot create waterfall temporary {}: {error}",
                temporary.display()
            ))
        })?;
    let result = (|| {
        file.write_all(contents.as_bytes()).map_err(|error| {
            WaterfallStoreError::Diagnostic(format!(
                "cannot write waterfall temporary {}: {error}",
                temporary.display()
            ))
        })?;
        file.sync_all().map_err(|error| {
            WaterfallStoreError::Diagnostic(format!(
                "cannot sync waterfall temporary {}: {error}",
                temporary.display()
            ))
        })?;
        drop(file);
        fs::rename(&temporary, path).map_err(|error| {
            WaterfallStoreError::Diagnostic(format!(
                "cannot atomically replace waterfall file {}: {error}",
                path.display()
            ))
        })?;
        sync_directory(parent)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn temporary_path(path: &Path) -> PathBuf {
    let sequence = ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("waterfall");
    path.with_file_name(format!(
        ".{filename}.{}.{}.tmp",
        std::process::id(),
        sequence
    ))
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), WaterfallStoreError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            WaterfallStoreError::Diagnostic(format!(
                "cannot sync waterfall directory {}: {error}",
                path.display()
            ))
        })
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), WaterfallStoreError> {
    Ok(())
}
