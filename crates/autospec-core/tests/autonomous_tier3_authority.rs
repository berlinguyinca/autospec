use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "support/tier2_authority_matcher.rs"]
mod matcher;

use matcher::{
    contains_path_symbol, contains_qualified_path, has_forbidden_std_module, has_module_escape,
};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn pure_tier3_sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sources = vec![root.join("src/autonomous/tier3.rs")];
    collect_rust_sources(&root.join("src/autonomous/tier3"), &mut sources);
    sources.sort();
    sources
}

fn collect_rust_sources(directory: &Path, sources: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("read Tier 3 module directory") {
        let path = entry.expect("read Tier 3 module entry").path();
        if path.is_dir() {
            collect_rust_sources(&path, sources);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
}

fn temporary_module_root() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("autospec-tier3-authority-{nanos}"))
}

fn code_without_comments_and_literals(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut code = String::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index..] {
            [b'/', b'/', ..] => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
                code.push(' ');
            }
            [b'/', b'*', ..] => {
                index = block_comment_end(bytes, index + 2);
                code.push(' ');
            }
            [b'\"', ..] => {
                index = quoted_end(bytes, index + 1);
                code.push(' ');
            }
            _ => {
                code.push(bytes[index] as char);
                index += 1;
            }
        }
    }
    code
}

fn block_comment_end(bytes: &[u8], mut index: usize) -> usize {
    let mut depth = 1;
    while index < bytes.len() && depth > 0 {
        match bytes[index..] {
            [b'/', b'*', ..] => {
                depth += 1;
                index += 2;
            }
            [b'*', b'/', ..] => {
                depth -= 1;
                index += 2;
            }
            _ => index += 1,
        }
    }
    index
}

fn quoted_end(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
        } else if bytes[index] == b'\"' {
            return index + 1;
        } else {
            index += 1;
        }
    }
    index
}

fn contains_code_token(code: &str, token: &str) -> bool {
    code.match_indices(token).any(|(offset, _)| {
        let before = code[..offset].chars().next_back();
        let after = code[offset + token.len()..].chars().next();
        !before.is_some_and(is_identifier_character) && !after.is_some_and(is_identifier_character)
    })
}

fn is_identifier_character(character: char) -> bool {
    character == '_' || character.is_ascii_alphanumeric()
}

fn production_source(relative: &str) -> String {
    fs::read_to_string(workspace_root().join(relative))
        .expect("read Tier 3 production source")
        .split("\n#[cfg(test)]")
        .next()
        .expect("production source before tests")
        .to_string()
}

#[test]
fn recursive_source_discovery_finds_nested_rust_modules() {
    let root = temporary_module_root();
    fs::create_dir_all(root.join("nested")).expect("create nested module tree");
    fs::write(root.join("top.rs"), "pub struct Top;\n").expect("write top module");
    fs::write(root.join("nested/deep.rs"), "pub struct Deep;\n").expect("write deep module");

    let mut sources = Vec::new();
    collect_rust_sources(&root, &mut sources);

    assert!(sources.contains(&root.join("top.rs")));
    assert!(sources.contains(&root.join("nested/deep.rs")));
    fs::remove_dir_all(root).expect("remove nested module tree");
}

#[test]
fn authority_matcher_rejects_evasions_but_allows_inert_names() {
    let inert = code_without_comments_and_literals(
        "// use std::fs\nlet note = \"std::path\"; let queue = Vec::new(); let checkout = 1;",
    );
    assert!(!has_forbidden_std_module(&inert, "fs"));
    assert!(!has_forbidden_std_module(&inert, "path"));
    assert!(!contains_qualified_path(&inert, "queue"));
    assert!(!contains_path_symbol(&inert, "git::checkout"));

    let imports = code_without_comments_and_literals(
        "use std::fs; use std :: env; use std::{process, net, io, path}; use std::{os::{unix::fs}};",
    );
    for module in ["fs", "env", "process", "net", "io", "path"] {
        assert!(has_forbidden_std_module(&imports, module));
    }
    assert!(contains_qualified_path(
        &code_without_comments_and_literals("gh::issue::create();"),
        "gh"
    ));
    assert!(contains_qualified_path(
        &code_without_comments_and_literals("queue::read();"),
        "queue"
    ));
    assert!(contains_path_symbol(
        &code_without_comments_and_literals("git::checkout(\"branch\");"),
        "git::checkout"
    ));
    for escape in [
        "#[path = \"../escape.rs\"] mod escaped;",
        "#[cfg_attr(unix, path = \"../escape.rs\")] mod escaped;",
        "include!(\"../escape.rs\");",
    ] {
        assert!(has_module_escape(&code_without_comments_and_literals(
            escape
        )));
    }
}

#[test]
fn pure_tier3_sources_reject_external_and_mutation_authority() {
    let sources = pure_tier3_sources();
    let documents = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/autonomous/tier3/evidence.rs"),
    )
    .expect("read opaque Tier 3 documents");
    assert!(documents.contains("pub struct Tier3EvidenceDocuments"));
    assert!(!documents.contains("pub source:"));
    let mut saw_documents = false;
    for source in sources {
        let code = code_without_comments_and_literals(
            &fs::read_to_string(&source).expect("read pure Tier 3 source"),
        );
        assert!(
            !has_module_escape(&code),
            "{} escapes the guarded Tier 3 module tree",
            source.display()
        );
        saw_documents |= contains_code_token(&code, "Tier3EvidenceDocuments");
        for module in ["fs", "io", "path", "env", "process", "net"] {
            assert!(
                !has_forbidden_std_module(&code, module),
                "{} retains std::{module} authority",
                source.display()
            );
        }
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
                !contains_qualified_path(&code, path),
                "{} retains {path} module authority",
                source.display()
            );
        }
        for mutation in [
            "Method::POST",
            "Method::PATCH",
            "Method::PUT",
            "Method::DELETE",
        ] {
            assert!(
                !contains_path_symbol(&code, mutation),
                "{} retains HTTP mutation authority: {mutation}",
                source.display()
            );
        }
        for forbidden in [
            "Command",
            "WaterfallStore",
            "TierReceipt",
            "TierStatus",
            "WaterfallState",
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
                !contains_code_token(&code, forbidden),
                "{} retains prohibited authority: {forbidden}",
                source.display()
            );
        }
        assert!(
            !contains_path_symbol(&code, "git::checkout"),
            "{} retains git checkout authority",
            source.display()
        );
    }
    assert!(saw_documents, "guard opaque Tier 3 evidence documents");
}

#[test]
fn tier3_adapter_is_checked_in_policy_only() {
    let code = code_without_comments_and_literals(&production_source(
        "crates/autospec-cli/src/commands/autonomous/tier3.rs",
    ));
    assert_eq!(
        code.matches("Tier3Input::DisabledByCheckedInPolicy")
            .count(),
        1
    );
    assert!(!has_module_escape(&code));
    for module in ["fs", "io", "path", "env", "process", "net"] {
        assert!(!has_forbidden_std_module(&code, module));
    }
    assert_no_execution_authority(&code, "Tier 3 adapter");
    assert!(!contains_code_token(&code, "WaterfallStore"));
}

#[test]
fn tier3_receipt_coordinator_has_only_local_store_persistence() {
    let code = code_without_comments_and_literals(&production_source(
        "crates/autospec-cli/src/commands/autonomous/tier3_receipts.rs",
    ));
    assert!(contains_code_token(&code, "WaterfallStore"));
    assert!(!has_module_escape(&code));
    for module in ["fs", "io", "env", "process", "net"] {
        assert!(!has_forbidden_std_module(&code, module));
    }
    assert_no_execution_authority(&code, "Tier 3 receipt coordinator");
    for forbidden in [
        "OpenOptions",
        "File",
        "write_all",
        "write_fmt",
        "set_permissions",
    ] {
        assert!(
            !contains_code_token(&code, forbidden),
            "Tier 3 receipt coordinator retains direct I/O authority: {forbidden}"
        );
    }
}

#[test]
fn tier3_receipt_verifier_is_read_only_and_keeps_shared_helpers() {
    let code = code_without_comments_and_literals(&production_source(
        "crates/autospec-cli/src/commands/autonomous/waterfall/evidence/tier3.rs",
    ));
    assert!(contains_path_symbol(&code, "fs::read_to_string"));
    assert!(contains_qualified_path(&code, "canonical"));
    assert!(!has_module_escape(&code));
    for module in ["env", "process", "net"] {
        assert!(!has_forbidden_std_module(&code, module));
    }
    assert_no_execution_authority(&code, "Tier 3 receipt verifier");
    for mutation in [
        "fs::write",
        "fs::copy",
        "fs::hard_link",
        "fs::create_dir",
        "fs::remove_",
        "fs::rename",
        "File::create",
        "OpenOptions",
        "write_all",
        "write_fmt",
        "set_permissions",
        "symlink_file",
        "symlink_dir",
    ] {
        assert!(
            !contains_path_symbol(&code, mutation) && !contains_code_token(&code, mutation),
            "Tier 3 receipt verifier retains file mutation authority: {mutation}"
        );
    }
}

fn assert_no_execution_authority(code: &str, scope: &str) {
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
}

#[test]
fn cutover_plan_states_tier3_foundation_and_remaining_gates() {
    let plan = fs::read_to_string(
        workspace_root().join("docs/superpowers/plans/2026-07-16-rust-autonomous-waterfall.md"),
    )
    .expect("read autonomous waterfall plan")
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ");
    for required in [
        "Tier 3 typed metadata foundation and checked-in disabled receipt policy are complete.",
        "Metadata-source activation requires a trusted typed metadata source and #1602 typed configuration.",
        "Foreground wiring, Tier 4, ideation, and legacy deletion remain separately gated.",
        "This foundation does not permit legacy deletion.",
    ] {
        assert!(plan.contains(required), "cutover plan omits: {required}");
    }
}
