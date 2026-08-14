use super::*;

pub(crate) fn acquire_test_lifecycle(
    root: &Path,
    repo: &str,
) -> Result<ConductorLease, String> {
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

pub(crate) fn replace_test_lifecycle_generation(lease: &ConductorLease) -> Result<(), String> {
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
    super::super::atomic_write(&lease.state_path, &state.to_json(&scope.as_str()))
}
