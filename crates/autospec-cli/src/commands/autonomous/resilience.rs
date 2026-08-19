use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::CommandFailure;
use autospec_core::autonomous_lifecycle::{
    decide as decide_lifecycle, decide_capacity, decide_conductor_lease, CapacityDecision,
    CapacityInput, ConductorLeaseDecision, ConductorLeaseInput, ConductorLeaseReclaim,
    LifecycleDecision, LifecycleInput, LifecycleReject, RepositoryScope,
};

mod records;
mod spawn;
mod startup_transaction;
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
    accountability_predecessor_generation: Option<u64>,
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

    pub(super) fn accountability_predecessor_generation(&self) -> Option<u64> {
        self.accountability_predecessor_generation
    }

    fn from_store(
        store: &ResilienceStore,
        token: String,
        generation: u64,
        accountability_predecessor_generation: Option<u64>,
    ) -> Self {
        Self {
            token,
            generation,
            accountability_predecessor_generation,
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
// The lease transaction; see the module header there.
#[path = "resilience/lease_transaction.rs"]
mod lease_transaction;
use lease_transaction::LeaseTransaction;

// Owner-liveness predicates; see the module header there.
#[path = "resilience/liveness.rs"]
mod liveness;
use liveness::{pid_is_dead, same_known_host};


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

        let previous_generation = stored_state
            .as_ref()
            .and_then(|(state, _)| state.lease_generation);
        let accountability_predecessor_generation = stored_state.as_ref().and_then(|(state, _)| {
            (state.status == "released")
                .then_some(state.lease_generation)
                .flatten()
        });
        let generation = previous_generation
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| {
                StoreError::Diagnostic("cannot advance resilience lease generation".to_string())
            })?;
        let lease = ConductorLease::from_store(
            self,
            lease_token(),
            generation,
            accountability_predecessor_generation,
        );
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
            None,
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
    startup_transaction::retry(|| store.adopt(token)).map_err(store_error_to_lease_error)
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
    startup_transaction::retry(|| store.admit_owned(lease, issue, usage_cap, issue_cap))
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
    #[cfg(test)]
    let transaction = startup_transaction::retry(|| LeaseTransaction::try_open(&lease.lock_path));
    #[cfg(not(test))]
    let transaction = LeaseTransaction::try_open(&lease.lock_path);
    let _transaction = transaction.map_err(|error| match error {
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
mod test_support;
#[cfg(test)]
pub(super) use test_support::{acquire_test_lifecycle, replace_test_lifecycle_generation};

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

#[cfg(all(test, unix))]
mod tests {
    include!("resilience/policy_tests.rs");
    include!("resilience/spawn_tests.rs");
    include!("resilience/predecessor_tests.rs");
    use super::super::waterfall::retry_transient_lock;
    use super::*;
    use autospec_core::coordination::{QueueGateCounts, ReadyQueuePlan, WorkerCap};
    use std::process::{Child, Command};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    fn assert_token_mismatch<T>(operation: impl FnMut() -> Result<T, StoreError>) {
        assert!(matches!(
            retry_transient_lock(operation, |result| matches!(result, Err(StoreError::Held))),
            Err(StoreError::TokenMismatch)
        ));
    }

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

    fn dead_child_pid() -> u32 {
        let mut child = Command::new("true")
            .spawn()
            .expect("start dead PID fixture");
        let pid = child.id();
        assert!(child.wait().expect("reap dead PID fixture").success());
        pid
    }

    include!("resilience/heartbeat_tests.rs");

    fn wait_for(path: &Path, description: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !path.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(path.exists(), "timed out waiting for {description}");
    }
}
