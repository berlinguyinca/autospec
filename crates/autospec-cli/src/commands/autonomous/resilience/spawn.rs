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
    let store = ResilienceStore::from_env(repo).map_err(LifecycleLeaseError::Diagnostic)?;
    startup_transaction::retry(|| store.renew(lease)).map_err(store_error_to_lease_error)
}
