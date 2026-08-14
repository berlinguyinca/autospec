use super::*;

type CleanupFailureTestHook = (
    std::sync::Arc<std::sync::Barrier>,
    std::sync::Arc<std::sync::Barrier>,
);

static CLEANUP_FAILURE_TEST_HOOK: std::sync::OnceLock<
    std::sync::Mutex<Option<CleanupFailureTestHook>>,
> = std::sync::OnceLock::new();

#[allow(dead_code)]
pub(crate) fn install_cleanup_failure_test_hook() -> CleanupFailureTestHook {
    let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
    let release = std::sync::Arc::new(std::sync::Barrier::new(2));
    *CLEANUP_FAILURE_TEST_HOOK
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("cleanup failure test hook lock") = Some((entered.clone(), release.clone()));
    (entered, release)
}

#[allow(dead_code)]
pub(crate) fn wait_for_cleanup_failure_test_hook() {
    let hook = CLEANUP_FAILURE_TEST_HOOK
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("cleanup failure test hook lock")
        .take();
    if let Some((entered, release)) = hook {
        entered.wait();
        release.wait();
    }
}

#[allow(dead_code)]
pub(crate) fn try_transition_environment_lifecycle_for_test(
    environment_dir: &Path,
    lifecycle: EnvironmentLifecycle,
) -> Result<bool, CommandFailure> {
    let Some(_lease) = EnvironmentLease::try_acquire(environment_dir)? else {
        return Ok(false);
    };
    transition_environment_lifecycle_locked_for_test(environment_dir, lifecycle)?;
    Ok(true)
}

#[allow(dead_code)]
pub(crate) fn transition_environment_lifecycle_for_test(
    environment_dir: &Path,
    lifecycle: EnvironmentLifecycle,
) -> Result<(), CommandFailure> {
    let _lease = EnvironmentLease::acquire(environment_dir)?;
    transition_environment_lifecycle_locked_for_test(environment_dir, lifecycle)
}

fn transition_environment_lifecycle_locked_for_test(
    environment_dir: &Path,
    lifecycle: EnvironmentLifecycle,
) -> Result<(), CommandFailure> {
    let layout = StateLayout::new(
        environment_dir
            .parent()
            .expect("test environment directory has a state root"),
        environment_dir
            .file_name()
            .and_then(|value| value.to_str())
            .expect("test environment ID is UTF-8"),
    );
    let Some(mut authoritative) = read_authoritative_state(&layout)? else {
        return Err(CommandFailure::diagnostic(
            "test lifecycle transition requires authoritative state",
        ));
    };
    state::write_lifecycle(&layout, &mut authoritative.owner, lifecycle)
}
