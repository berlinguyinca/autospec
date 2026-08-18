/// Does this git failure mean the ref simply is not there?
///
/// Matched on git's own wording, lowercased so a future capitalisation change does not silently
/// reclassify a permanent condition as transient.
pub(super) fn missing_remote_ref(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("couldn't find remote ref") || error.contains("no such ref was fetched")
}

/// Does this git failure mean the checkout itself is structurally unusable?
///
/// Retrying can only help a momentary condition. A path git refuses to recognise as a repository
/// is a wiring fault, so every retry re-runs the identical command against the identical path,
/// burns the retry budget, and then pauses under `retry_limit_exhausted` -- a reason that names
/// the budget the conductor ran out of rather than the cause the operator has to fix.
///
/// Matched on git's own wording, lowercased so a future capitalisation change does not silently
/// reclassify a permanent condition as transient.
pub(super) fn structural_repository_failure(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("not a git repository")
        || error.contains("does not appear to be a git repository")
}

pub(super) fn classify_error(branch: &str, explore_mode: bool, error: String) -> String {
    if structural_repository_failure(&error) {
        return format!("fetch executor base: {error}");
    }
    if !missing_remote_ref(&error) {
        return format!("TRANSIENT: fetch executor base: {error}");
    }
    if explore_mode {
        format!(
            "executor explore branch {branch} is not on the remote. \
             .autospec/explore-mode.json pins a sandbox branch that no longer exists; \
             remove or repoint that file to unwedge this repository. Underlying: {error}"
        )
    } else {
        format!("executor base branch {branch} is not on the remote: {error}")
    }
}
