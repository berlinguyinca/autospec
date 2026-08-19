use std::sync::{Mutex, MutexGuard, PoisonError};

pub(super) type TestForkLifecycleGuard = MutexGuard<'static, ()>;

static TEST_FORK_LIFECYCLE: Mutex<()> = Mutex::new(());

pub(super) fn lock_test_fork_lifecycle() -> TestForkLifecycleGuard {
    TEST_FORK_LIFECYCLE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn test_fork_lifecycle_is_available() -> bool {
    TEST_FORK_LIFECYCLE.try_lock().is_ok()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LaunchFailpoint {
    None = 0,
    PersistAfterSpawn = 1,
    LogAfterSpawn = 2,
    BeforeSnapshotVerification = 3,
    NeverReady = 4,
    NeverCloseExecStatus = 5,
    AdoptedPoll = 6,
    AdoptedFlush = 7,
    AdoptedLog = 8,
    DirectPoll = 9,
    CleanupSignal = 10,
    CleanupLiveness = 11,
    ParentAfterPidfd = 12,
    ParentHarnessCapture = 13,
    ParentBirthRefresh = 14,
    RingBeforeSync = 15,
    DirectSetup = 16,
    PostReturnIdentity = 17,
    CleanupFreezeWindow = 18,
    ParentHarnessPidRead = 19,
    ParentHarnessBirth = 20,
    ParentHarnessPidfd = 21,
    ParentReadiness = 22,
    JournalCreate = 23,
    JournalWrite = 24,
    JournalSync = 25,
    JournalRename = 26,
    JournalDirectorySync = 27,
    DescendantCapture = 28,
    RingReadInterrupted = 29,
    ArchiveAfterManifest = 30,
    ArchiveMidMove = 31,
    ArchiveBeforeComplete = 32,
    RetireAfterProof = 33,
    RetireMidDelete = 34,
    RetireAfterLaunchDelete = 35,
    BeforeEvidenceBundle = 36,
    RecoveryAfterAnchorClear = 37,
    RecoveryBeforeSnapshot = 38,
    RetireAfterPendingRemoval = 39,
    RotationAfterArchive = 40,
    RotationAfterActive = 41,
    EvidenceAfterGenerationSelect = 42,
    OwnershipBeforeMarker = 43,
    OwnershipAfterMarker = 44,
}
