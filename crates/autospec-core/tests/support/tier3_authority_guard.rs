use crate::contains_code_token;
use crate::matcher::{code_tokens, contains_path_symbol, contains_qualified_path};

pub(crate) fn assert_no_execution_authority(code: &str, scope: &str, allows_waterfall_store: bool) {
    for path in [
        "queue",
        "claim",
        "github",
        "gh",
        "branch",
        "worktree",
        "issue",
        "label",
        "pull_request",
    ] {
        assert!(
            !contains_qualified_path(code, path),
            "{scope} retains {path} module authority"
        );
    }
    for mutation in [
        "Method::POST",
        "Method::PATCH",
        "Method::PUT",
        "Method::DELETE",
    ] {
        assert!(
            !contains_path_symbol(code, mutation),
            "{scope} retains HTTP mutation authority: {mutation}"
        );
    }
    for forbidden in [
        "Command",
        "bash",
        "zsh",
        "run_shell",
        "curl",
        "legacy",
        "ModelClient",
        "ModelRequest",
        "run_model",
        "remote",
        "foreground",
        "executor",
        "mutation",
        "RemoteIssue",
        "PullRequest",
        "ExecutorRequest",
        "ConductorEvent",
        "run_foreground",
        "scan_foreground",
        "add_label",
        "remove_label",
        "create_issue",
        "edit_issue",
        "comment_issue",
        "create_branch",
    ] {
        assert!(
            !contains_code_token(code, forbidden),
            "{scope} retains prohibited authority: {forbidden}"
        );
    }
    assert!(
        !contains_path_symbol(code, "git::checkout"),
        "{scope} retains git checkout authority"
    );
    for namespace in ["reqwest", "ureq", "hyper", "surf", "isahc", "awc"] {
        assert!(
            !contains_qualified_path(code, namespace),
            "{scope} retains external remote namespace: {namespace}"
        );
    }
    for facade in ["sqlx::Pool", "sled::Db", "rusqlite::Connection"] {
        assert!(
            !contains_path_symbol(code, facade),
            "{scope} retains external persistence authority: {facade}"
        );
    }
    for token in code_tokens(code) {
        assert!(
            token == "WaterfallStore"
                || !(token.ends_with("Client")
                    || token.ends_with("Store")
                    || token.ends_with("Repository")),
            "{scope} retains external client or persistence facade: {token}"
        );
    }
    assert_eq!(
        contains_code_token(code, "WaterfallStore"),
        allows_waterfall_store,
        "{scope} has an invalid WaterfallStore boundary"
    );
}
