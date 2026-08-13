/// Does this git failure mean the ref simply is not there?
///
/// Matched on git's own wording, lowercased so a future capitalisation change does not silently
/// reclassify a permanent condition as transient.
pub(super) fn missing_remote_ref(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("couldn't find remote ref") || error.contains("no such ref was fetched")
}

pub(super) fn classify_error(branch: &str, explore_mode: bool, error: String) -> String {
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
