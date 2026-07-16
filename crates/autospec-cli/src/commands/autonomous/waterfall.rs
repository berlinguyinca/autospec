use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use autospec_core::autonomous::no_work::NoWorkTier;
use autospec_core::autonomous::waterfall::{
    receipt_reference, sha256_hex, SealedEvidence, TierReceipt, TierStatus, WaterfallState,
};

static ATOMIC_WRITE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) enum StoreAcquisition {
    Acquired(WaterfallStore),
    Held,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Tier1EvidenceArtifact {
    ReadyPage,
    ReadFailure,
}

impl Tier1EvidenceArtifact {
    fn file_name(self) -> &'static str {
        match self {
            Self::ReadyPage => "ready-page.json",
            Self::ReadFailure => "read-failure.json",
        }
    }
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
    _lock: File,
}

impl WaterfallStore {
    pub(super) fn acquire(
        root: impl AsRef<Path>,
        repo: impl Into<String>,
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
                _lock: lock,
            })),
            None => Ok(StoreAcquisition::Held),
        }
    }

    pub(super) fn state_path(&self) -> &Path {
        // The state filename is intentionally stable: Task 3 resumes only from this cursor.
        &self.state_path
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
        let reference = tier1_evidence_reference(pass_id, artifact)?;
        let evidence = SealedEvidence::new(&reference, sha256_hex(contents.as_bytes()))
            .map_err(WaterfallStoreError::InvalidReceipt)?;
        let path = self.root.join(&reference);
        match fs::read_to_string(&path) {
            Ok(existing) if existing == contents => Ok(evidence),
            Ok(_) => Err(WaterfallStoreError::InvalidReceipt(
                "conflicting sealed waterfall evidence".to_string(),
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                atomic_write(&path, contents)?;
                Ok(evidence)
            }
            Err(error) => Err(WaterfallStoreError::Diagnostic(format!(
                "cannot read waterfall evidence {}: {error}",
                path.display()
            ))),
        }
    }

    pub(super) fn verify_tier1_evidence(
        &self,
        pass_id: u64,
        artifact: Tier1EvidenceArtifact,
        receipt: &TierReceipt,
    ) -> Result<(), WaterfallStoreError> {
        let reference = tier1_evidence_reference(pass_id, artifact)?;
        let path = self.root.join(&reference);
        let contents = fs::read_to_string(&path).map_err(|error| {
            let message = if error.kind() == io::ErrorKind::NotFound {
                format!("missing sealed waterfall evidence {reference}")
            } else {
                format!("cannot read waterfall evidence {}: {error}", path.display())
            };
            WaterfallStoreError::InvalidReceipt(message)
        })?;
        let digest = sha256_hex(contents.as_bytes());
        let marker = format!("\"reference\":\"{reference}\",\"digest\":\"{digest}\"");
        if receipt.to_json().contains(&marker) {
            Ok(())
        } else {
            Err(WaterfallStoreError::InvalidReceipt(
                "sealed waterfall evidence does not match receipt".to_string(),
            ))
        }
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
        for completed in state.completed_receipts() {
            let receipt = self
                .load_receipt(state.next_pass_id(), completed.tier)
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
            if completed.tier == NoWorkTier::Tier1 {
                self.verify_tier1_evidence(
                    state.next_pass_id(),
                    tier1_artifact_for_receipt(&receipt)?,
                    &receipt,
                )
                .map_err(state_receipt_error)?;
            }
        }
        Ok(())
    }
}

fn tier1_artifact_for_receipt(
    receipt: &TierReceipt,
) -> Result<Tier1EvidenceArtifact, WaterfallStoreError> {
    match receipt.status() {
        TierStatus::Exhausted { .. } => Ok(Tier1EvidenceArtifact::ReadyPage),
        TierStatus::Failed { .. } => Ok(Tier1EvidenceArtifact::ReadFailure),
        status => Err(WaterfallStoreError::InvalidState(format!(
            "Tier 1 receipt has unexpected {} status",
            status.as_str()
        ))),
    }
}

fn tier1_evidence_reference(
    pass_id: u64,
    artifact: Tier1EvidenceArtifact,
) -> Result<String, WaterfallStoreError> {
    if pass_id == 0 {
        return Err(WaterfallStoreError::InvalidReceipt(
            "waterfall Tier 1 evidence pass id must be positive".to_string(),
        ));
    }
    Ok(format!(
        "waterfall/{pass_id}/tier1/{}",
        artifact.file_name()
    ))
}

fn state_receipt_error(error: WaterfallStoreError) -> WaterfallStoreError {
    let detail = match error {
        WaterfallStoreError::Diagnostic(detail)
        | WaterfallStoreError::InvalidReceipt(detail)
        | WaterfallStoreError::InvalidState(detail) => detail,
    };
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
        use std::os::fd::AsRawFd;

        unsafe extern "C" {
            fn flock(fd: i32, operation: i32) -> i32;
        }
        const LOCK_EX: i32 = 2;
        const LOCK_NB: i32 = 4;
        // SAFETY: the returned file retains the successful flock for the store lifetime.
        if unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) } == 0 {
            return Ok(Some(file));
        }
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::WouldBlock {
            Ok(None)
        } else {
            Err(WaterfallStoreError::Diagnostic(format!(
                "cannot lock waterfall store {}: {error}",
                path.display()
            )))
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
