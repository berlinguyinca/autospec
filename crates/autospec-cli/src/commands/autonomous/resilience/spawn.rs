use super::*;

pub(crate) fn renew_lifecycle(
    repo: &str,
    lease: &ConductorLease,
) -> Result<(), LifecycleLeaseError> {
    let store = ResilienceStore::from_env(repo).map_err(LifecycleLeaseError::Diagnostic)?;
    renew_lifecycle_store(&store, lease)
}

pub(crate) fn assert_lifecycle_before_spawn(
    repo: &str,
    lease: &ConductorLease,
) -> Result<(), LifecycleLeaseError> {
    const RETRY_DELAYS: [Duration; 3] = [
        Duration::from_millis(10),
        Duration::from_millis(20),
        Duration::from_millis(40),
    ];
    let store = ResilienceStore::from_env(repo).map_err(LifecycleLeaseError::Diagnostic)?;
    for delay in RETRY_DELAYS {
        match store.renew(lease) {
            Ok(()) => return Ok(()),
            Err(StoreError::Held) => thread::sleep(delay),
            Err(error) => return Err(store_error_to_lease_error(error)),
        }
    }
    Err(LifecycleLeaseError::Held)
}
