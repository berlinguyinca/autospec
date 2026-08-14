use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use super::CommandFailure;
use autospec_core::autonomous_lifecycle::{
    decide as decide_lifecycle, decide_capacity, decide_conductor_lease, CapacityDecision,
    CapacityInput, ConductorLeaseDecision, ConductorLeaseInput, ConductorLeaseReclaim,
    LifecycleDecision, LifecycleInput, LifecycleReject, RepositoryScope,
};

mod records;
mod spawn;
pub(super) use spawn::{assert_lifecycle_before_spawn, renew_lifecycle};

use records::{parse_failures, ResilienceReject, ResilienceState, Spend};

const DEFAULT_LIFETIME_TOKENS: u64 = 10_000_000;
const DEFAULT_LIFETIME_ISSUES: u64 = 500;
static LEASE_TOKEN_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct ResilienceStore {
    scope: RepositoryScope,
    state_root: PathBuf,
    spend_root: PathBuf,
    host: String,
}
pub(super) struct ResilienceAdmission {
    pub failure_count: u8,
    pub capacity: CapacityDecision,
    pub lease: Option<ConductorLeaseDecision>,
    pub spend: ResilienceSpend,
}

pub(super) struct ResilienceStatus {
    pub status: String,
    pub heartbeat_at: Option<u64>,
    pub last_cycle: String,
}

pub(super) struct ResilienceSpend {
    pub tokens: u64,
    pub filed_issues: u64,
    pub budget_issues: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TerminatedOwnerRelease {
    Released,
    Absent,
    OwnerMismatch,
}

pub(super) enum LifecycleAdmissionError {
    Reject(&'static str),
    Diagnostic(String),
}

#[derive(Clone)]
pub(super) struct ConductorLease {
    token: String,
    generation: u64,
    repo: String,
    state_path: PathBuf,
    lock_path: PathBuf,
}

pub(super) struct LifecycleHeartbeat {
    stop: Sender<()>,
    handle: Option<JoinHandle<Result<(), LifecycleLeaseError>>>,
}

impl LifecycleHeartbeat {
    pub(super) fn finish(mut self) -> Result<(), LifecycleLeaseError> {
        let _ = self.stop.send(());
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        handle.join().unwrap_or_else(|_| {
            Err(LifecycleLeaseError::Diagnostic(
                "resilience lease heartbeat thread panicked".to_string(),
            ))
        })
    }
}

impl Drop for LifecycleHeartbeat {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl ConductorLease {
    pub(super) fn token(&self) -> &str {
        &self.token
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    fn from_store(store: &ResilienceStore, token: String, generation: u64) -> Self {
        Self {
            token,
            generation,
            repo: store.scope.as_str(),
            state_path: store.canonical_state_path(),
            lock_path: store.lock_path(),
        }
    }
}

#[allow(dead_code)] // Transaction-only outcomes are consumed by Task 3's lifecycle adapter.
enum StoreError {
    Reject(ResilienceReject),
    Diagnostic(String),
    Held,
    TokenMismatch,
    Policy(ResilienceAdmission),
}

pub(super) enum LifecycleLeaseError {
    Reject(&'static str),
    Diagnostic(String),
    Held,
    TokenMismatch,
    Policy(ResilienceAdmission),
}

#[allow(dead_code)] // Acquisition/adoption/release are intentionally not public command actions.
struct LeaseTransaction {
    _file: fs::File,
}

#[allow(dead_code)] // Task 3 calls this only through the store transaction primitives.
impl LeaseTransaction {
    #[cfg(unix)]
    fn try_open(path: &Path) -> Result<Self, StoreError> {
        let parent = path.parent().expect("lease lock path has a parent");
        fs::create_dir_all(parent).map_err(|error| {
            StoreError::Diagnostic(format!(
                "cannot create resilience lease directory {}: {error}",
                parent.display()
            ))
        })?;
        let file = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)
            .map_err(|error| {
                StoreError::Diagnostic(format!(
                    "cannot open resilience lease lock {}: {error}",
                    path.display()
                ))
            })?;

        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(fs::TryLockError::WouldBlock) => Err(StoreError::Held),
            Err(fs::TryLockError::Error(error)) => Err(StoreError::Diagnostic(format!(
                "cannot lock resilience lease {}: {error}",
                path.display()
            ))),
        }
    }

    #[cfg(not(unix))]
    fn try_open(_path: &Path) -> Result<Self, StoreError> {
        Err(StoreError::Diagnostic(
            "resilience lease transactions require Unix flock support".to_string(),
        ))
    }
}

impl ResilienceStore {
    fn from_env(repo: &str) -> Result<Self, String> {
        let scope = RepositoryScope::try_from(repo).map_err(|reason| format!("--repo {reason}"))?;
        let state_base = env_path("AUTOSPEC_STATE_DIR", &[".autospec"]);
        let spend_root = env::var("AUTOSPEC_AUTONOMOUS_SPEND_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home_path(&[".autospec", "autonomous-spend"]));
        Ok(Self {
            scope,
            state_root: state_base.join("autonomous"),
            spend_root,
            host: local_host_identity(),
        })
    }
    fn read_state(&self) -> Result<Option<(ResilienceState, bool)>, StoreError> {
        for (candidate_index, path) in self
            .candidates(&self.state_root, "state.json")
            .into_iter()
            .enumerate()
        {
            match fs::read_to_string(&path) {
                Ok(raw) => {
                    let state = ResilienceState::parse(&raw)
                        .map_err(|_| StoreError::Reject(ResilienceReject::MalformedState))?;
                    if state.repo != self.scope.as_str() {
                        return Err(StoreError::Reject(ResilienceReject::ForeignState));
                    }
                    return Ok(Some((state, candidate_index > 0)));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(StoreError::Diagnostic(format!(
                        "cannot read resilience state {}: {error}",
                        path.display()
                    )))
                }
            }
        }
        Ok(None)
    }

    fn read_status(&self) -> Result<Option<ResilienceStatus>, StoreError> {
        for path in self.candidates(&self.state_root, "state.json") {
            match fs::read_to_string(&path) {
                Ok(raw) => {
                    let state = records::StatusState::parse(&raw)
                        .map_err(|_| StoreError::Reject(ResilienceReject::MalformedState))?;
                    if state.repo != self.scope.as_str() {
                        return Err(StoreError::Reject(ResilienceReject::ForeignState));
                    }
                    let last_cycle = state
                        .cycle
                        .map(|cycle| cycle.to_string())
                        .or_else(|| trailing_cycle(&state.status))
                        .unwrap_or_default();
                    return Ok(Some(ResilienceStatus {
                        status: state.status,
                        heartbeat_at: state.heartbeat_at,
                        last_cycle,
                    }));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(StoreError::Diagnostic(format!(
                        "cannot read resilience state {}: {error}",
                        path.display()
                    )))
                }
            }
        }
        Ok(None)
    }
    fn read_failures(&self, issue: Option<u64>) -> Result<u8, StoreError> {
        let Some(issue) = issue else {
            return Ok(0);
        };
        for path in self.candidates(&self.state_root, &format!("issues/{issue}.json")) {
            match fs::read_to_string(&path) {
                Ok(raw) => return parse_failures(&raw, issue).map_err(StoreError::Reject),
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(StoreError::Diagnostic(format!(
                        "cannot read resilience failure record {}: {error}",
                        path.display()
                    )))
                }
            }
        }
        Ok(0)
    }
    fn read_spend(&self) -> Result<Spend, StoreError> {
        for path in self.candidates(&self.spend_root, "spend.json") {
            match fs::read_to_string(&path) {
                Ok(raw) => return Spend::parse(&raw).map_err(StoreError::Reject),
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(StoreError::Diagnostic(format!(
                        "cannot read resilience spend record {}: {error}",
                        path.display()
                    )))
                }
            }
        }
        Ok(Spend::default())
    }

    fn write_state(&self, value: &ResilienceState) -> Result<(), StoreError> {
        let path = self.canonical_state_path();
        fs::create_dir_all(path.parent().expect("state path has a parent")).map_err(|error| {
            StoreError::Diagnostic(format!(
                "cannot create resilience state directory {}: {error}",
                path.parent().expect("state path has a parent").display()
            ))
        })?;
        let [canonical, ..] = self.slugs();
        super::atomic_write(&path, &value.to_json(&canonical)).map_err(StoreError::Diagnostic)
    }

    fn admit(
        &self,
        issue: Option<u64>,
        usage_cap: u64,
        issue_cap: u64,
    ) -> Result<ResilienceAdmission, StoreError> {
        self.read_admission(issue, usage_cap, issue_cap)
            .map(|(admission, _)| admission)
    }

    fn acquire(
        &self,
        issue: Option<u64>,
        usage_cap: u64,
        issue_cap: u64,
    ) -> Result<(ResilienceAdmission, ConductorLease), StoreError> {
        let _transaction = LeaseTransaction::try_open(&self.lock_path())?;
        let (admission, stored_state) = self.read_admission(issue, usage_cap, issue_cap)?;
        if matches!(admission.lease, Some(ConductorLeaseDecision::Held)) {
            return Err(StoreError::Held);
        }
        if !self.admission_can_claim(&admission) {
            return Err(StoreError::Policy(admission));
        }

        let generation = stored_state
            .as_ref()
            .and_then(|(state, _)| state.lease_generation)
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| {
                StoreError::Diagnostic("cannot advance resilience lease generation".to_string())
            })?;
        let lease = ConductorLease::from_store(self, lease_token(), generation);
        self.write_state(&self.claimed_state(&lease))?;
        Ok((admission, lease))
    }

    fn adopt(&self, token: &str) -> Result<ConductorLease, StoreError> {
        let _transaction = LeaseTransaction::try_open(&self.lock_path())?;
        let Some((mut state, _)) = self.read_state()? else {
            return Err(StoreError::TokenMismatch);
        };
        let Some(generation) = state.lease_generation else {
            return Err(StoreError::TokenMismatch);
        };
        if !matches!(state.status.as_str(), "claimed" | "running")
            || state.lease_token.as_deref() != Some(token)
        {
            return Err(StoreError::TokenMismatch);
        }

        let now = now_secs();
        state.status = "running".to_string();
        state.host = Some(self.host.clone());
        state.heartbeat_at = Some(now);
        state.lock_pid = Some(std::process::id());
        state.lock_host = Some(self.host.clone());
        state.lock_session = None;
        state.lock_acquired_at = Some(now);
        self.write_state(&state)?;
        Ok(ConductorLease::from_store(
            self,
            token.to_string(),
            generation,
        ))
    }

    fn renew(&self, lease: &ConductorLease) -> Result<(), StoreError> {
        let _transaction = LeaseTransaction::try_open(&self.lock_path())?;
        let Some((mut state, _)) = self.read_state()? else {
            return Err(StoreError::TokenMismatch);
        };
        if !state_matches_lease(&state, lease) {
            return Err(StoreError::TokenMismatch);
        }

        state.status = "running".to_string();
        state.heartbeat_at = Some(now_secs());
        state.lock_pid = Some(std::process::id());
        state.lock_host = Some(self.host.clone());
        self.write_state(&state)
    }

    fn release(&self, lease: &ConductorLease) -> Result<(), StoreError> {
        let _transaction = LeaseTransaction::try_open(&self.lock_path())?;
        let Some((mut state, _)) = self.read_state()? else {
            return Err(StoreError::TokenMismatch);
        };
        if !state_matches_lease(&state, lease) {
            return Err(StoreError::TokenMismatch);
        }

        state.status = "released".to_string();
        state.heartbeat_at = Some(now_secs());
        state.lock_pid = None;
        state.lock_host = None;
        state.lock_session = None;
        state.lock_acquired_at = None;
        state.lease_token = None;
        state.lease_generation = Some(lease.generation);
        self.write_state(&state)
    }

    fn release_terminated_owner(
        &self,
        expected_pid: u32,
    ) -> Result<TerminatedOwnerRelease, StoreError> {
        let _transaction = LeaseTransaction::try_open(&self.lock_path())?;
        let Some((mut state, _)) = self.read_state()? else {
            return Ok(TerminatedOwnerRelease::Absent);
        };
        if state.status == "released" {
            return Ok(TerminatedOwnerRelease::Absent);
        }
        if !pid_is_dead(expected_pid) {
            return Err(StoreError::Held);
        }
        let exact_owner = state.lock_pid == Some(expected_pid);
        let abandoned_claim = state.status == "claimed"
            && state.lock_pid.is_some_and(pid_is_dead)
            && state.lease_token.is_some();
        if !exact_owner && !abandoned_claim {
            return Ok(TerminatedOwnerRelease::OwnerMismatch);
        }

        state.status = "released".to_string();
        state.heartbeat_at = Some(now_secs());
        state.lock_pid = None;
        state.lock_host = None;
        state.lock_session = None;
        state.lock_acquired_at = None;
        state.lease_token = None;
        self.write_state(&state)?;
        Ok(TerminatedOwnerRelease::Released)
    }

    fn read_admission(
        &self,
        issue: Option<u64>,
        usage_cap: u64,
        issue_cap: u64,
    ) -> Result<(ResilienceAdmission, Option<(ResilienceState, bool)>), StoreError> {
        let stored_state = self.read_state()?;
        let failure_count = self.read_failures(issue)?;
        let spend = self.read_spend()?;
        let capacity = decide_capacity(CapacityInput::new(
            spend.tokens,
            usage_cap,
            spend.budget_issues,
            issue_cap,
        ));
        let lease = stored_state
            .as_ref()
            .and_then(|(state, _)| conductor_lease_decision(state, &self.host));
        Ok((
            ResilienceAdmission {
                failure_count,
                capacity,
                lease,
                spend: ResilienceSpend {
                    tokens: spend.tokens,
                    filed_issues: spend.filed_issues,
                    budget_issues: spend.budget_issues,
                },
            },
            stored_state,
        ))
    }

    fn admit_owned(
        &self,
        lease: &ConductorLease,
        issue: Option<u64>,
        usage_cap: u64,
        issue_cap: u64,
    ) -> Result<ResilienceAdmission, StoreError> {
        let (mut admission, stored_state) = self.read_admission(issue, usage_cap, issue_cap)?;
        let Some((state, _)) = stored_state else {
            return Err(StoreError::TokenMismatch);
        };
        if !state_matches_lease(&state, lease) {
            return Err(StoreError::TokenMismatch);
        }
        admission.lease = None;
        Ok(admission)
    }

    fn admission_can_claim(&self, admission: &ResilienceAdmission) -> bool {
        matches!(admission.capacity, CapacityDecision::WithinCap)
            && !matches!(
                decide_lifecycle(
                    &LifecycleInput::from_scope(self.scope.clone())
                        .with_failure_count(admission.failure_count),
                ),
                LifecycleDecision::Reject(LifecycleReject::FailureCap)
            )
    }

    #[allow(dead_code)] // Called only by the transaction primitives above.
    fn claimed_state(&self, lease: &ConductorLease) -> ResilienceState {
        let now = now_secs();
        ResilienceState {
            repo: self.scope.as_str(),
            status: "claimed".to_string(),
            host: Some(self.host.clone()),
            session: None,
            heartbeat_at: Some(now),
            lock_pid: Some(std::process::id()),
            lock_host: Some(self.host.clone()),
            lock_session: None,
            lock_acquired_at: Some(now),
            lease_token: Some(lease.token.clone()),
            lease_generation: Some(lease.generation),
        }
    }

    fn canonical_state_path(&self) -> PathBuf {
        let [canonical, ..] = self.slugs();
        self.state_root.join(canonical).join("state.json")
    }

    #[allow(dead_code)] // Called only by the transaction primitives above.
    fn lock_path(&self) -> PathBuf {
        self.canonical_state_path()
            .with_file_name("conductor.lease.lock")
    }

    fn slugs(&self) -> [String; 3] {
        let repo = self.scope.as_str();
        [
            repo.replace('/', "__"),
            repo.replace('/', "_"),
            repo.replace('/', "-"),
        ]
    }

    fn candidates(&self, root: &Path, leaf: &str) -> [PathBuf; 3] {
        let [canonical, underscore, hyphen] = self.slugs();
        [
            root.join(canonical).join(leaf),
            root.join(underscore).join(leaf),
            root.join(hyphen).join(leaf),
        ]
    }
}

fn local_host_identity() -> String {
    ["AUTOSPEC_HOST", "HOSTNAME"]
        .into_iter()
        .find_map(|name| {
            env::var(name)
                .ok()
                .map(|host| host.trim().to_string())
                .filter(|host| !host.is_empty())
        })
        .or_else(|| {
            let output = Command::new("hostname").arg("-s").output().ok()?;
            output
                .status
                .success()
                .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        })
        .filter(|host| !host.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn conductor_lease_decision(
    state: &ResilienceState,
    current_host: &str,
) -> Option<ConductorLeaseDecision> {
    let lock_pid = state.lock_pid?;
    let known_same_host = state
        .lock_host
        .as_deref()
        .is_some_and(|host| same_known_host(host, current_host));
    let dead_same_host_pid = known_same_host && pid_is_dead(lock_pid);
    let input = match state.heartbeat_at {
        Some(heartbeat_at) => {
            let age = now_secs().saturating_sub(heartbeat_at);
            if state.status == "claimed" {
                ConductorLeaseInput::claimed(age, dead_same_host_pid)
            } else {
                ConductorLeaseInput::running(age, dead_same_host_pid)
            }
        }
        None => {
            ConductorLeaseInput::missing_heartbeat(state.status == "claimed", dead_same_host_pid)
        }
    };
    Some(decide_conductor_lease(input))
}

fn state_matches_lease(state: &ResilienceState, lease: &ConductorLease) -> bool {
    state.lease_token.as_deref() == Some(lease.token.as_str())
        && state.lease_generation == Some(lease.generation)
}

fn lease_token() -> String {
    let sequence = LEASE_TOKEN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}-{sequence}", std::process::id(), now_nanos())
}

pub(super) fn admit_lifecycle(
    repo: &str,
    issue: Option<u64>,
    budget_tokens: Option<u64>,
    budget_issues: Option<u64>,
) -> Result<ResilienceAdmission, LifecycleAdmissionError> {
    let store = ResilienceStore::from_env(repo).map_err(LifecycleAdmissionError::Diagnostic)?;
    let usage_cap = lifecycle_budget(
        budget_tokens,
        "AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS",
        DEFAULT_LIFETIME_TOKENS,
    )?;
    let issue_cap = lifecycle_budget(
        budget_issues,
        "AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES",
        DEFAULT_LIFETIME_ISSUES,
    )?;
    store
        .admit(issue, usage_cap, issue_cap)
        .map_err(store_error_to_lifecycle_error)
}

pub(super) fn acquire_lifecycle(
    repo: &str,
    issue: Option<u64>,
    budget_tokens: Option<u64>,
    budget_issues: Option<u64>,
) -> Result<(ResilienceAdmission, ConductorLease), LifecycleLeaseError> {
    let store = ResilienceStore::from_env(repo).map_err(LifecycleLeaseError::Diagnostic)?;
    let usage_cap = lifecycle_budget(
        budget_tokens,
        "AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS",
        DEFAULT_LIFETIME_TOKENS,
    )
    .map_err(lifecycle_admission_to_lease_error)?;
    let issue_cap = lifecycle_budget(
        budget_issues,
        "AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES",
        DEFAULT_LIFETIME_ISSUES,
    )
    .map_err(lifecycle_admission_to_lease_error)?;
    store
        .acquire(issue, usage_cap, issue_cap)
        .map_err(store_error_to_lease_error)
}

pub(super) fn adopt_lifecycle(
    repo: &str,
    token: &str,
) -> Result<ConductorLease, LifecycleLeaseError> {
    let store = ResilienceStore::from_env(repo).map_err(LifecycleLeaseError::Diagnostic)?;
    store.adopt(token).map_err(store_error_to_lease_error)
}

pub(super) fn admit_owned_lifecycle(
    repo: &str,
    lease: &ConductorLease,
    issue: Option<u64>,
    budget_tokens: Option<u64>,
    budget_issues: Option<u64>,
) -> Result<ResilienceAdmission, LifecycleLeaseError> {
    let store = ResilienceStore::from_env(repo).map_err(LifecycleLeaseError::Diagnostic)?;
    let usage_cap = lifecycle_budget(
        budget_tokens,
        "AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS",
        DEFAULT_LIFETIME_TOKENS,
    )
    .map_err(lifecycle_admission_to_lease_error)?;
    let issue_cap = lifecycle_budget(
        budget_issues,
        "AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES",
        DEFAULT_LIFETIME_ISSUES,
    )
    .map_err(lifecycle_admission_to_lease_error)?;
    store
        .admit_owned(lease, issue, usage_cap, issue_cap)
        .map_err(store_error_to_lease_error)
}

pub(super) fn start_lifecycle_heartbeat(
    repo: &str,
    lease: &ConductorLease,
) -> Result<LifecycleHeartbeat, LifecycleLeaseError> {
    const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);
    let store = ResilienceStore::from_env(repo).map_err(LifecycleLeaseError::Diagnostic)?;
    start_lifecycle_heartbeat_with_store(store, lease.clone(), HEARTBEAT_INTERVAL)
}

fn start_lifecycle_heartbeat_with_store(
    store: ResilienceStore,
    lease: ConductorLease,
    interval: Duration,
) -> Result<LifecycleHeartbeat, LifecycleLeaseError> {
    renew_lifecycle_store(&store, &lease)?;
    let lease = lease.clone();
    let (stop, stopped) = mpsc::channel();
    let handle = thread::spawn(move || loop {
        match stopped.recv_timeout(interval) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                renew_lifecycle_store(&store, &lease)?;
            }
        }
    });
    Ok(LifecycleHeartbeat {
        stop,
        handle: Some(handle),
    })
}

fn renew_lifecycle_store(
    store: &ResilienceStore,
    lease: &ConductorLease,
) -> Result<(), LifecycleLeaseError> {
    const RETRY_DELAYS: [Duration; 3] = [
        Duration::from_millis(10),
        Duration::from_millis(20),
        Duration::from_millis(40),
    ];
    for delay in RETRY_DELAYS {
        match store.renew(lease) {
            Ok(()) => return Ok(()),
            Err(StoreError::Held) => thread::sleep(delay),
            Err(error) => return Err(store_error_to_lease_error(error)),
        }
    }
    // A held transaction fences the lease while another owned mutation completes.
    // Defer this tick; the next heartbeat retries without treating contention as loss.
    Ok(())
}

pub(super) fn release_lifecycle(
    repo: &str,
    lease: &ConductorLease,
) -> Result<(), LifecycleLeaseError> {
    let store = ResilienceStore::from_env(repo).map_err(LifecycleLeaseError::Diagnostic)?;
    store.release(lease).map_err(store_error_to_lease_error)
}

pub(super) fn release_terminated_lifecycle_owner(
    repo: &str,
    expected_pid: u32,
) -> Result<TerminatedOwnerRelease, LifecycleLeaseError> {
    let store = ResilienceStore::from_env(repo).map_err(LifecycleLeaseError::Diagnostic)?;
    store
        .release_terminated_owner(expected_pid)
        .map_err(store_error_to_lease_error)
}

pub(super) fn with_current_lifecycle_lease<T>(
    lease: &ConductorLease,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    // Global mutation lock order is resilience lease first, then subsystem locks. Holding the
    // lease transaction across `operation` prevents a reclaim from racing a fenced write.
    let _transaction =
        LeaseTransaction::try_open(&lease.lock_path).map_err(|error| match error {
            StoreError::Held => "resilience lease transaction is held".to_string(),
            StoreError::Diagnostic(reason) => reason,
            StoreError::TokenMismatch => "resilience lease token mismatch".to_string(),
            StoreError::Reject(reject) => reject.reason().to_string(),
            StoreError::Policy(_) => "resilience lease policy rejected the transaction".to_string(),
        })?;
    let raw = fs::read_to_string(&lease.state_path)
        .map_err(|error| format!("cannot read resilience lease state: {error}"))?;
    let state = ResilienceState::parse(&raw)
        .map_err(|_| "resilience lease state is malformed".to_string())?;
    if state.repo != lease.repo || !state_matches_lease(&state, lease) {
        return Err("resilience lease token or generation was replaced".to_string());
    }
    operation()
}

#[cfg(test)]
pub(super) fn acquire_test_lifecycle(root: &Path, repo: &str) -> Result<ConductorLease, String> {
    let store = ResilienceStore {
        scope: RepositoryScope::try_from(repo).map_err(|error| error.to_string())?,
        state_root: root.join("state").join("autonomous"),
        spend_root: root.join("spend"),
        host: "autospec-test-host".to_string(),
    };
    store
        .acquire(None, 1, 1)
        .map(|(_, lease)| lease)
        .map_err(|_| "cannot acquire test lifecycle lease".to_string())
}

#[cfg(test)]
pub(super) fn replace_test_lifecycle_generation(lease: &ConductorLease) -> Result<(), String> {
    let raw = fs::read_to_string(&lease.state_path)
        .map_err(|error| format!("cannot read test lifecycle state: {error}"))?;
    let mut state = ResilienceState::parse(&raw)
        .map_err(|_| "test lifecycle state is malformed".to_string())?;
    state.lease_generation = Some(
        state
            .lease_generation
            .unwrap_or_default()
            .checked_add(1)
            .ok_or_else(|| "test lifecycle generation overflow".to_string())?,
    );
    state.lease_token = Some("replacement-test-lifecycle-token".to_string());
    let scope =
        RepositoryScope::try_from(lease.repo.as_str()).map_err(|error| error.to_string())?;
    super::atomic_write(&lease.state_path, &state.to_json(&scope.as_str()))
}

pub(super) fn status(repo: &str) -> Result<Option<ResilienceStatus>, LifecycleAdmissionError> {
    let store = ResilienceStore::from_env(repo).map_err(LifecycleAdmissionError::Diagnostic)?;
    store.read_status().map_err(store_error_to_lifecycle_error)
}

pub(super) fn spend_status(repo: &str) -> Result<ResilienceSpend, LifecycleAdmissionError> {
    let store = ResilienceStore::from_env(repo).map_err(LifecycleAdmissionError::Diagnostic)?;
    let spend = store.read_spend().map_err(store_error_to_lifecycle_error)?;
    Ok(ResilienceSpend {
        tokens: spend.tokens,
        filed_issues: spend.filed_issues,
        budget_issues: spend.budget_issues,
    })
}

pub(super) fn run(args: &[String]) -> Result<(), CommandFailure> {
    if matches!(args, [flag] if flag == "--help" || flag == "-h") {
        print_help();
        return Ok(());
    }
    let options = DecisionOptions::parse(args).map_err(CommandFailure::diagnostic)?;
    let store = ResilienceStore::from_env(&options.repo).map_err(CommandFailure::diagnostic)?;
    let admission = match store.admit(options.issue, options.usage_cap, options.issue_cap) {
        Ok(admission) => admission,
        Err(StoreError::Reject(reject)) => return emit(Decision::Reject(reject.reason()), None),
        Err(StoreError::Diagnostic(message)) => return Err(CommandFailure::diagnostic(message)),
        Err(StoreError::Held) => return emit(Decision::Held, None),
        Err(StoreError::TokenMismatch) => {
            return Err(CommandFailure::diagnostic(
                "resilience token mismatch is not valid for read-only admission".to_string(),
            ))
        }
        Err(StoreError::Policy(_)) => {
            return Err(CommandFailure::diagnostic(
                "resilience claim policy is not valid for read-only admission".to_string(),
            ))
        }
    };
    if let Some(ConductorLeaseDecision::Held) = admission.lease {
        return emit(Decision::Held, Some(&admission.spend));
    }
    if matches!(
        decide_lifecycle(
            &LifecycleInput::from_scope(store.scope.clone())
                .with_failure_count(admission.failure_count),
        ),
        LifecycleDecision::Reject(LifecycleReject::FailureCap)
    ) {
        return emit(Decision::Reject("failure_cap"), Some(&admission.spend));
    }
    match admission.capacity {
        CapacityDecision::UsageCap => emit(Decision::Park("usage_cap"), Some(&admission.spend)),
        CapacityDecision::IssueCap => emit(Decision::Park("issue_cap"), Some(&admission.spend)),
        CapacityDecision::WithinCap => match admission.lease {
            Some(ConductorLeaseDecision::Reclaim(reason)) => {
                emit(Decision::Reclaim(reason), Some(&admission.spend))
            }
            Some(ConductorLeaseDecision::Held) => emit(Decision::Held, Some(&admission.spend)),
            None => emit(Decision::Available, Some(&admission.spend)),
        },
    }
}

fn print_help() {
    println!(
        "autospec autonomous resilience\n\nUSAGE:\n    autospec autonomous resilience decide --repo OWNER/REPO [--issue N] [--budget-tokens N] [--budget-issues N]\n\nSTATE:\n    Reads owner__repo, then owner_repo, then owner-repo.\n    Writes resilience state only to the canonical owner__repo layout.\n\nOPTIONS:\n    --repo OWNER/REPO    Repository whose resilience state is evaluated\n    --issue N            Optional positive issue number for failure-cap evaluation\n    --budget-tokens N    Non-negative lifetime token limit\n    --budget-issues N    Non-negative lifetime issue limit\n    -h, --help           Print help"
    );
}

struct DecisionOptions {
    repo: String,
    issue: Option<u64>,
    usage_cap: u64,
    issue_cap: u64,
}

impl DecisionOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        if args.first().map(String::as_str) != Some("decide") {
            return Err("autonomous resilience requires the decide action".to_string());
        }
        let mut repo = None;
        let mut issue = None;
        let mut usage_cap = None;
        let mut issue_cap = None;
        let mut index = 1;
        while index < args.len() {
            let flag = &args[index];
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| format!("{flag} requires a value"))?;
            match flag.as_str() {
                "--repo" => set_once(&mut repo, value, "--repo")?,
                "--issue" => set_number(&mut issue, value, "--issue", true)?,
                "--budget-tokens" => set_number(&mut usage_cap, value, "--budget-tokens", false)?,
                "--budget-issues" => set_number(&mut issue_cap, value, "--budget-issues", false)?,
                _ => return Err(format!("unknown autonomous resilience option: {flag}")),
            }
            index += 1;
        }
        Ok(Self {
            repo: repo.ok_or_else(|| "autonomous resilience requires --repo".to_string())?,
            issue,
            usage_cap: match usage_cap {
                Some(value) => value,
                None => env_budget(
                    "AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS",
                    DEFAULT_LIFETIME_TOKENS,
                )?,
            },
            issue_cap: match issue_cap {
                Some(value) => value,
                None => env_budget(
                    "AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES",
                    DEFAULT_LIFETIME_ISSUES,
                )?,
            },
        })
    }
}

fn set_once(slot: &mut Option<String>, value: &str, flag: &str) -> Result<(), String> {
    if slot.replace(value.to_string()).is_some() {
        Err(format!("{flag} may be specified only once"))
    } else {
        Ok(())
    }
}

fn set_number(
    slot: &mut Option<u64>,
    value: &str,
    flag: &str,
    positive: bool,
) -> Result<(), String> {
    let number = value.parse::<u64>().map_err(|_| {
        format!(
            "{flag} must be a {} integer",
            if positive { "positive" } else { "non-negative" }
        )
    })?;
    if positive && number == 0 {
        return Err(format!("{flag} must be a positive integer"));
    }
    if slot.replace(number).is_some() {
        Err(format!("{flag} may be specified only once"))
    } else {
        Ok(())
    }
}

enum Decision {
    Available,
    Held,
    Reclaim(ConductorLeaseReclaim),
    Park(&'static str),
    Reject(&'static str),
}

fn emit(decision: Decision, spend: Option<&ResilienceSpend>) -> Result<(), CommandFailure> {
    let spend_suffix = spend.map(spend_json_suffix).unwrap_or_default();
    let (body, exit_code) = match decision {
        Decision::Available => (format!("{{\"decision\":\"available\"{spend_suffix}}}"), 0),
        Decision::Held => (format!("{{\"decision\":\"held\"{spend_suffix}}}"), 20),
        Decision::Reclaim(reason) => (
            format!(
                "{{\"decision\":\"reclaim\",\"reason\":\"{}\"{spend_suffix}}}",
                reclaim_name(reason)
            ),
            0,
        ),
        Decision::Park(reason) => (
            format!("{{\"decision\":\"park\",\"reason\":\"{reason}\"{spend_suffix}}}"),
            20,
        ),
        Decision::Reject(reason) => (
            format!("{{\"decision\":\"reject\",\"reason\":\"{reason}\"{spend_suffix}}}"),
            3,
        ),
    };
    println!("{body}");
    if exit_code == 0 {
        Ok(())
    } else {
        Err(CommandFailure::status(String::new(), exit_code))
    }
}

fn spend_json_suffix(spend: &ResilienceSpend) -> String {
    format!(
        ",\"spend\":{{\"tokens\":{},\"issues\":{},\"filed_issues\":{},\"budget_issues\":{}}}",
        spend.tokens, spend.budget_issues, spend.filed_issues, spend.budget_issues
    )
}

fn reclaim_name(reason: ConductorLeaseReclaim) -> &'static str {
    match reason {
        ConductorLeaseReclaim::DeadSameHostPid => "dead_same_host_pid",
        ConductorLeaseReclaim::MissingHeartbeat => "missing_heartbeat",
        ConductorLeaseReclaim::Abandoned => "abandoned",
        ConductorLeaseReclaim::ClaimedExpired => "claimed_expired",
    }
}

fn env_path(name: &str, suffix: &[&str]) -> PathBuf {
    env::var(name)
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_path(suffix))
}

fn home_path(suffix: &[&str]) -> PathBuf {
    let mut path = env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    for component in suffix {
        path.push(component);
    }
    path
}

fn env_budget(name: &str, default: u64) -> Result<u64, String> {
    match env::var(name) {
        Ok(value) => value
            .parse()
            .map_err(|_| format!("{name} must be a non-negative integer")),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(env::VarError::NotUnicode(_)) => Err(format!("{name} must be a non-negative integer")),
    }
}

fn lifecycle_budget(
    override_value: Option<u64>,
    environment: &str,
    default: u64,
) -> Result<u64, LifecycleAdmissionError> {
    match override_value {
        Some(value) => Ok(value),
        None => env_budget(environment, default).map_err(LifecycleAdmissionError::Diagnostic),
    }
}

fn store_error_to_lifecycle_error(error: StoreError) -> LifecycleAdmissionError {
    match error {
        StoreError::Reject(reject) => LifecycleAdmissionError::Reject(reject.reason()),
        StoreError::Diagnostic(message) => LifecycleAdmissionError::Diagnostic(message),
        StoreError::Held => {
            LifecycleAdmissionError::Diagnostic("resilience lease transaction is held".to_string())
        }
        StoreError::TokenMismatch => LifecycleAdmissionError::Diagnostic(
            "resilience token mismatch is not valid for read-only admission".to_string(),
        ),
        StoreError::Policy(_) => LifecycleAdmissionError::Diagnostic(
            "resilience claim policy is not valid for read-only admission".to_string(),
        ),
    }
}

fn lifecycle_admission_to_lease_error(error: LifecycleAdmissionError) -> LifecycleLeaseError {
    match error {
        LifecycleAdmissionError::Reject(reason) => LifecycleLeaseError::Reject(reason),
        LifecycleAdmissionError::Diagnostic(message) => LifecycleLeaseError::Diagnostic(message),
    }
}

fn store_error_to_lease_error(error: StoreError) -> LifecycleLeaseError {
    match error {
        StoreError::Reject(reject) => LifecycleLeaseError::Reject(reject.reason()),
        StoreError::Diagnostic(message) => LifecycleLeaseError::Diagnostic(message),
        StoreError::Held => LifecycleLeaseError::Held,
        StoreError::TokenMismatch => LifecycleLeaseError::TokenMismatch,
        StoreError::Policy(admission) => LifecycleLeaseError::Policy(admission),
    }
}

fn trailing_cycle(status: &str) -> Option<String> {
    let suffix = status.strip_prefix("running:")?;
    let digits = suffix.strip_prefix("cycle-")?;
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| suffix.to_string())
}

fn same_known_host(recorded: &str, current: &str) -> bool {
    let recorded = recorded.trim();
    let current = current.trim();
    !recorded.is_empty()
        && !current.is_empty()
        && !recorded.eq_ignore_ascii_case("unknown")
        && !current.eq_ignore_ascii_case("unknown")
        && recorded == current
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(unix)]
fn pid_is_dead(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return true;
    }
    if super::process_is_zombie(pid as i32) {
        return true;
    }
    pid_probe_is_dead(nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid as i32),
        None,
    ))
}

#[cfg(unix)]
fn pid_probe_is_dead(result: Result<(), nix::errno::Errno>) -> bool {
    matches!(result, Err(nix::errno::Errno::ESRCH))
}

#[cfg(not(unix))]
fn pid_is_dead(_pid: u32) -> bool {
    false
}

#[cfg(all(test, unix))]
mod tests {
    include!("resilience/spawn_tests.rs");
    use super::super::waterfall::retry_transient_lock;
    use super::*;
    use autospec_core::coordination::{QueueGateCounts, ReadyQueuePlan, WorkerCap};
    use std::process::{Child, Command};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    const LEASE_LOCK_TEST_ROOT: &str = "AUTOSPEC_LEASE_LOCK_TEST_ROOT";
    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct ChildGuard(Child);

    impl ChildGuard {
        fn stop_and_reap(&mut self) {
            self.0.kill().expect("stop liveness child");
            self.0.wait().expect("reap liveness child");
        }
    }

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn pid_liveness_requires_observed_process_absence() {
        assert!(!pid_is_dead(std::process::id()));
        assert!(pid_is_dead(0));
        assert!(pid_is_dead(i32::MAX as u32 + 1));

        let mut child = ChildGuard(
            Command::new("sleep")
                .arg("60")
                .spawn()
                .expect("start liveness child"),
        );
        let pid = child.0.id();
        assert!(!pid_is_dead(pid));
        child.stop_and_reap();
        assert!(pid_is_dead(pid));
    }

    #[test]
    fn pid_probe_fails_closed_for_errors_other_than_process_absence() {
        assert!(!pid_probe_is_dead(Ok(())));
        assert!(pid_probe_is_dead(Err(nix::errno::Errno::ESRCH)));
        assert!(!pid_probe_is_dead(Err(nix::errno::Errno::EPERM)));
        assert!(!pid_probe_is_dead(Err(nix::errno::Errno::EINVAL)));
    }

    #[test]
    fn competing_conductors_hold_one_owner_before_operator_write() {
        let root = test_root("contention");
        let lock_root = root.join("lock-holder");
        let ready = lock_root.join("ready");
        let release = lock_root.join("release");
        let mut child = Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "commands::autonomous::resilience::tests::lease_lock_holder_child",
                "--nocapture",
            ])
            .env(LEASE_LOCK_TEST_ROOT, &lock_root)
            .spawn()
            .expect("start lease lock holder");

        wait_for(&ready, "lease lock holder readiness");
        let contended = test_store(&lock_root).acquire(None, 1, 1);
        assert!(matches!(contended, Err(StoreError::Held)));
        assert!(!test_store(&lock_root).canonical_state_path().exists());
        fs::write(&release, "release\n").expect("release lock holder");
        assert!(child.wait().expect("wait for lock holder").success());

        let first = test_store(&root);
        let second = test_store(&root);
        let (admission, lease) = match first.acquire(None, 1, 1) {
            Ok(value) => value,
            Err(_) => panic!("first conductor must acquire the lease"),
        };
        assert!(admission.lease.is_none());
        assert!(matches!(second.acquire(None, 1, 1), Err(StoreError::Held)));

        let state = match first.read_state() {
            Ok(Some((state, false))) => state,
            _ => panic!("claimed state must be canonical"),
        };
        assert_eq!(state.status, "claimed");
        assert_eq!(state.lease_token.as_deref(), Some(lease.token.as_str()));
        assert_eq!(state.lease_generation, Some(lease.generation));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stale_token_cannot_adopt_or_release_reclaimed_lease() {
        let root = test_root("fencing");
        let first = test_store(&root);
        let second = test_store(&root);
        let (_, stale_lease) = match first.acquire(None, 1, 1) {
            Ok(value) => value,
            Err(_) => panic!("first conductor must acquire the lease"),
        };
        let mut stale_state = match first.read_state() {
            Ok(Some((state, false))) => state,
            _ => panic!("first acquisition must write canonical state"),
        };
        stale_state.heartbeat_at = Some(now_secs().saturating_sub(300));
        match first.write_state(&stale_state) {
            Ok(()) => {}
            Err(_) => panic!("make first lease reclaimable"),
        }

        let (_, replacement) = match second.acquire(None, 1, 1) {
            Ok(value) => value,
            Err(_) => panic!("second conductor must reclaim the stale lease"),
        };
        assert_ne!(replacement.token, stale_lease.token);
        assert_eq!(replacement.generation, stale_lease.generation + 1);
        let replacement_state =
            fs::read_to_string(second.canonical_state_path()).expect("read replacement state");

        assert!(matches!(
            first.adopt(&stale_lease.token),
            Err(StoreError::TokenMismatch)
        ));
        assert_eq!(
            fs::read_to_string(second.canonical_state_path())
                .expect("read state after stale adopt"),
            replacement_state
        );
        assert!(matches!(
            first.release(&stale_lease),
            Err(StoreError::TokenMismatch)
        ));
        assert_eq!(
            fs::read_to_string(second.canonical_state_path())
                .expect("read state after stale release"),
            replacement_state
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terminated_owner_release_requires_the_exact_dead_pid() {
        let root = test_root("terminated-owner");
        let store = test_store(&root);
        let (_, _) = store
            .acquire(None, 1, 1)
            .unwrap_or_else(|_| panic!("acquire owner lease"));
        let before =
            fs::read_to_string(store.canonical_state_path()).expect("read owner lease state");

        assert_eq!(
            store
                .release_terminated_owner(u32::MAX)
                .unwrap_or_else(|_| panic!("reject a mismatched dead pid")),
            TerminatedOwnerRelease::OwnerMismatch
        );
        assert_eq!(
            fs::read_to_string(store.canonical_state_path())
                .expect("read state after mismatched release"),
            before
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terminated_owner_release_handles_a_dead_claim_owner_before_adoption() {
        let root = test_root("terminated-claimed-owner");
        let store = test_store(&root);
        let (_, _) = store
            .acquire(None, 1, 1)
            .unwrap_or_else(|_| panic!("acquire claimed lease"));
        let mut child = ChildGuard(
            Command::new("sleep")
                .arg("60")
                .spawn()
                .expect("start claimed owner"),
        );
        let claimed_pid = child.0.id();
        let mut state = store
            .read_state()
            .unwrap_or_else(|_| panic!("read claimed state"))
            .expect("claimed state")
            .0;
        state.lock_pid = Some(claimed_pid);
        store
            .write_state(&state)
            .unwrap_or_else(|_| panic!("record claimed owner"));
        child.stop_and_reap();

        assert_eq!(
            store
                .release_terminated_owner(u32::MAX)
                .unwrap_or_else(|_| panic!("release abandoned claim")),
            TerminatedOwnerRelease::Released
        );
        let released = store
            .read_state()
            .unwrap_or_else(|_| panic!("read released state"))
            .expect("released state")
            .0;
        assert_eq!(released.status, "released");
        assert!(released.lock_pid.is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn owned_lease_renewal_prevents_abandonment_reclaim() {
        let root = test_root("renewal");
        let owner = test_store(&root);
        let contender = test_store(&root);
        let (_, claimed) = owner
            .acquire(None, 1, 1)
            .unwrap_or_else(|_| panic!("claim lease"));
        let lease = owner
            .adopt(&claimed.token)
            .unwrap_or_else(|_| panic!("adopt lease"));
        let mut stale_state = owner
            .read_state()
            .unwrap_or_else(|_| panic!("read state"))
            .expect("running state")
            .0;
        stale_state.heartbeat_at = Some(now_secs().saturating_sub(10_801));
        owner
            .write_state(&stale_state)
            .unwrap_or_else(|_| panic!("age running lease"));

        owner
            .renew(&lease)
            .unwrap_or_else(|_| panic!("renew owned lease"));

        assert!(matches!(
            contender.acquire(None, 1, 1),
            Err(StoreError::Held)
        ));
        let renewed = owner
            .read_state()
            .unwrap_or_else(|_| panic!("read renewed state"))
            .expect("renewed state")
            .0;
        assert!(renewed.heartbeat_at > stale_state.heartbeat_at);
        owner
            .release(&lease)
            .unwrap_or_else(|_| panic!("release renewed lease"));
        assert_eq!(
            owner
                .read_state()
                .unwrap_or_else(|_| panic!("read released state"))
                .expect("released state")
                .0
                .status,
            "released"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn heartbeat_survives_owned_transaction_contention_and_renews_later() {
        let root = test_root("heartbeat-contention");
        let owner = test_store(&root);
        let (_, claimed) = owner
            .acquire(None, 1, 1)
            .unwrap_or_else(|_| panic!("claim lease"));
        let lease = owner
            .adopt(&claimed.token)
            .unwrap_or_else(|_| panic!("adopt lease"));
        let mut stale_state = owner
            .read_state()
            .unwrap_or_else(|_| panic!("read state"))
            .expect("running state")
            .0;
        stale_state.heartbeat_at = Some(now_secs().saturating_sub(10_801));
        owner
            .write_state(&stale_state)
            .unwrap_or_else(|_| panic!("age running lease"));

        let transaction = LeaseTransaction::try_open(&owner.lock_path())
            .unwrap_or_else(|_| panic!("hold owned lease transaction"));
        let heartbeat = start_lifecycle_heartbeat_with_store(
            test_store(&root),
            lease.clone(),
            Duration::from_millis(10),
        )
        .unwrap_or_else(|_| panic!("start heartbeat despite contention"));
        thread::sleep(Duration::from_millis(90));
        assert!(
            !heartbeat
                .handle
                .as_ref()
                .expect("heartbeat handle")
                .is_finished(),
            "transient contention must not terminate the heartbeat"
        );

        drop(transaction);
        thread::sleep(Duration::from_millis(100));
        heartbeat
            .finish()
            .unwrap_or_else(|_| panic!("finish surviving heartbeat"));
        let renewed = owner
            .read_state()
            .unwrap_or_else(|_| panic!("read renewed state"))
            .expect("renewed state")
            .0;
        assert!(renewed.heartbeat_at > stale_state.heartbeat_at);
        owner
            .release(&lease)
            .unwrap_or_else(|_| panic!("release lease"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn replaced_lease_cannot_persist_tier_one_waterfall_artifacts() {
        let root = test_root("waterfall-fencing");
        let first = test_store(&root);
        let second = test_store(&root);
        let (_, stale_lease) = match first.acquire(None, 1, 1) {
            Ok(value) => value,
            Err(_) => panic!("first lease"),
        };
        let mut stale_state = match first.read_state() {
            Ok(Some((state, _))) => state,
            _ => panic!("first lease state"),
        };
        stale_state.heartbeat_at = Some(now_secs().saturating_sub(300));
        if first.write_state(&stale_state).is_err() {
            panic!("expire first lease");
        }
        let (_, replacement) = match second.acquire(None, 1, 1) {
            Ok(value) => value,
            Err(_) => panic!("replacement lease"),
        };
        assert_ne!(replacement.token, stale_lease.token);

        let plan = ReadyQueuePlan {
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
        };
        let waterfall_root = root.join("operator");
        let policy = super::super::waterfall_policy::WaterfallPolicy::from_config(
            &autospec_core::autonomous::config::AutonomousConfig::default(),
        )
        .expect("default waterfall policy");
        let result = super::super::waterfall_coordinator::record_tier_one(
            &waterfall_root,
            "owner/repo",
            &stale_lease,
            &policy,
            super::super::waterfall_coordinator::Tier1QueueEvidence::EmptyPage(&plan),
        );

        assert!(result.is_err(), "a replaced lease must fail closed");
        assert!(
            !waterfall_root.join("waterfall").exists(),
            "lease validation must precede waterfall lock and artifact creation"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn matching_token_adopts_and_releases_its_lease() {
        let root = test_root("adopt-release");
        let store = test_store(&root);
        let (_, claimed) = match store.acquire(None, 1, 1) {
            Ok(value) => value,
            Err(_) => panic!("acquire claimed lease"),
        };

        let adopted = match retry_transient_lock(
            || store.adopt(&claimed.token),
            |result| matches!(result, Err(StoreError::Held)),
        ) {
            Ok(lease) => lease,
            Err(_) => panic!("adopt matching lease"),
        };
        assert_eq!(adopted.token, claimed.token);
        assert_eq!(adopted.generation, claimed.generation);
        let running = match store.read_state() {
            Ok(Some((state, _))) => state,
            _ => panic!("read adopted state"),
        };
        assert_eq!(running.status, "running");
        assert_eq!(running.lease_token.as_deref(), Some(adopted.token.as_str()));
        assert_eq!(running.lease_generation, Some(adopted.generation));
        assert_eq!(running.lock_pid, Some(std::process::id()));
        assert_eq!(running.lock_host.as_deref(), Some("autospec-test-host"));
        assert!(running.lock_acquired_at.is_some());

        match retry_transient_lock(
            || store.release(&adopted),
            |result| matches!(result, Err(StoreError::Held)),
        ) {
            Ok(()) => {}
            Err(_) => panic!("release matching lease"),
        }
        let released = match store.read_state() {
            Ok(Some((state, _))) => state,
            _ => panic!("read released state"),
        };
        assert_eq!(released.status, "released");
        assert!(released.lease_token.is_none());
        assert_eq!(released.lease_generation, Some(adopted.generation));
        assert!(released.lock_pid.is_none());
        assert!(released.lock_host.is_none());
        assert!(released.lock_session.is_none());
        assert!(released.lock_acquired_at.is_none());

        let _ = fs::remove_dir_all(root);
    }

    fn acquisition_does_not_claim_when_core_policy_parks_or_rejects() {
        let capacity_root = test_root("capacity");
        let capacity = test_store(&capacity_root);
        fs::create_dir_all(capacity.spend_root.join("owner__repo")).expect("create spend root");
        fs::write(
            capacity.spend_root.join("owner__repo/spend.json"),
            "{\"schema\":1,\"tokens\":1,\"issues\":0}",
        )
        .expect("write spend record");
        assert!(matches!(
            capacity.acquire(None, 1, 1),
            Err(StoreError::Policy(admission)) if matches!(admission.capacity, CapacityDecision::UsageCap)
        ));
        assert!(!capacity.canonical_state_path().exists());

        let failure_root = test_root("failure");
        let failure = test_store(&failure_root);
        let failure_path = failure.state_root.join("owner__repo/issues/42.json");
        fs::create_dir_all(failure_path.parent().expect("failure parent"))
            .expect("create failure root");
        fs::write(
            failure_path,
            "{\"issue\":42,\"failures\":3,\"updated_at\":1}",
        )
        .expect("write failure record");
        assert!(matches!(
            failure.acquire(Some(42), 1, 1),
            Err(StoreError::Policy(admission)) if admission.failure_count == 3
        ));
        assert!(!failure.canonical_state_path().exists());

        let _ = fs::remove_dir_all(capacity_root);
        let _ = fs::remove_dir_all(failure_root);
    }

    #[test]
    fn lease_lock_holder_child() {
        let Ok(root) = std::env::var(LEASE_LOCK_TEST_ROOT) else {
            return;
        };
        let root = PathBuf::from(root);
        let store = test_store(&root);
        let _transaction = match LeaseTransaction::try_open(&store.lock_path()) {
            Ok(transaction) => transaction,
            Err(_) => panic!("child must acquire lease transaction lock"),
        };
        fs::write(root.join("ready"), "ready\n").expect("mark lock holder ready");
        wait_for(&root.join("release"), "lease lock holder release");
    }

    fn test_store(root: &Path) -> ResilienceStore {
        ResilienceStore {
            scope: RepositoryScope::try_from("owner/repo").expect("repository scope"),
            state_root: root.join("state").join("autonomous"),
            spend_root: root.join("spend"),
            host: "autospec-test-host".to_string(),
        }
    }

    fn test_root(name: &str) -> PathBuf {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "autospec-resilience-lease-{name}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create test root");
        root
    }

    fn wait_for(path: &Path, description: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !path.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(path.exists(), "timed out waiting for {description}");
    }
}
