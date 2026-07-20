use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "support/tier2_authority_matcher.rs"]
mod matcher;
#[path = "support/tier4_authority_scanner.rs"]
#[allow(dead_code)]
mod scanner;

use matcher::{
    contains_path_symbol, contains_qualified_path, has_forbidden_std_module, has_module_escape,
};
use scanner::{code_without_comments_and_literals, has_write_capable_github_argv, production_code};

fn pure_tier2_sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sources = vec![root.join("src/autonomous/tier2.rs")];
    collect_rust_sources(&root.join("src/autonomous/tier2"), &mut sources);
    sources.sort();
    sources
}

fn temporary_module_root() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("autospec-tier2-authority-{nanos}"))
}

fn collect_rust_sources(directory: &Path, sources: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("read Tier 2 module directory") {
        let path = entry.expect("read Tier 2 module entry").path();
        if path.is_dir() {
            collect_rust_sources(&path, sources);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
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
fn authority_matcher_ignores_comments_literals_and_identifier_substrings() {
    let code = code_without_comments_and_literals(
        "// std::fs\nlet note = \"std::fs\";\nlet High = 1;\nuse std::fs;",
    );

    assert!(!contains_code_token(&code, "github"));
    assert!(!contains_code_token(&code, "gh"));
    assert!(contains_code_token(&code, "std::fs"));

    let raw = code_without_comments_and_literals("let note = r#\"std::path\"#; use std::path;");
    assert_eq!(raw.matches("std::path").count(), 1);
}

#[test]
fn authority_matcher_handles_module_escapes_grouped_imports_and_safe_nouns() {
    let imports = code_without_comments_and_literals(
        "use std::{fs, env}; use std :: process; use std::os::unix::fs; \
         use std::{os::{unix::fs}}; let queue = Vec::new(); let checkout = 1;",
    );
    assert!(has_forbidden_std_module(&imports, "fs"));
    assert!(has_forbidden_std_module(&imports, "env"));
    assert!(has_forbidden_std_module(&imports, "process"));
    let nested_imports =
        code_without_comments_and_literals("use std::os::unix::fs; use std::{os::{unix::fs}};");
    assert!(has_forbidden_std_module(&nested_imports, "fs"));
    assert!(!contains_qualified_path(&imports, "queue"));
    assert!(!contains_path_symbol(&imports, "git::checkout"));
    assert!(contains_path_symbol(
        &code_without_comments_and_literals("git::checkout(\"branch\");"),
        "git::checkout"
    ));
    assert!(contains_qualified_path(
        &code_without_comments_and_literals("gh::issue::create();"),
        "gh"
    ));
    assert!(contains_qualified_path(
        &code_without_comments_and_literals("queue::read();"),
        "queue"
    ));
    assert!(has_module_escape(&code_without_comments_and_literals(
        "#[path = \"../escape.rs\"] mod escaped;"
    )));
    assert!(has_module_escape(&code_without_comments_and_literals(
        "#[cfg_attr(unix, path = \"../escape.rs\")] mod escaped;"
    )));
    assert!(has_module_escape(&code_without_comments_and_literals(
        "include!(\"../escape.rs\");"
    )));
}

#[test]
fn pure_tier2_sources_reject_external_and_mutation_authority() {
    let sources = pure_tier2_sources();
    let names = sources
        .iter()
        .map(|source| {
            source
                .file_name()
                .and_then(|name| name.to_str())
                .expect("UTF-8 source name")
        })
        .collect::<Vec<_>>();
    assert!(names.contains(&"evidence.rs"), "guard opaque documents");
    assert!(
        names.contains(&"partial.rs"),
        "guard sealed partial documents"
    );

    let mut saw_documents = false;
    let mut saw_roi_rank_renderer = false;
    for source in sources {
        let contents = fs::read_to_string(&source).expect("read pure Tier 2 source");
        let production = production_code(&contents, &source.display().to_string());
        assert!(!has_write_capable_github_argv(&production));
        let code = code_without_comments_and_literals(&production);
        assert!(
            !has_module_escape(&code),
            "{} escapes the guarded Tier 2 module tree",
            source.display()
        );
        saw_documents |= contains_code_token(&code, "Tier2EvidenceDocuments");
        saw_roi_rank_renderer |= contains_code_token(&code, "roi_rank_json");
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
        for call in [
            "Method::POST",
            "Method::PATCH",
            "Method::PUT",
            "Method::DELETE",
        ] {
            assert!(
                !contains_path_symbol(&code, call),
                "{} retains HTTP mutation authority: {call}",
                source.display()
            );
        }
        for forbidden in [
            "Command",
            "WaterfallStore",
            "TierReceipt",
            "TierStatus",
            "WaterfallState",
            "scan_specialists",
            "load_or_derive",
            "ScanOptions",
            "cache::",
            "AUTOSPEC_SPECIALIST_LLM_STUB_OUTPUT",
            "autospec-explore",
            "bash",
            "zsh",
            "run_shell",
            "omx",
            "curl",
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
    assert!(saw_documents, "guard opaque Tier 2 evidence documents");
    assert!(
        saw_roi_rank_renderer,
        "guard the ROI/rank document renderer"
    );
}

#[test]
fn strict_collector_source_is_read_only_and_legacy_free() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = fs::read_to_string(root.join("src/explore/specialists/strict.rs"))
        .expect("read strict collector source");
    let production = production_code(&source, "strict collector");
    assert!(!has_write_capable_github_argv(&production));
    let code = code_without_comments_and_literals(&production);
    for module in ["env", "process", "net"] {
        assert!(
            !has_forbidden_std_module(&code, module),
            "strict collector retains std::{module} authority"
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
            "strict collector retains {path} module authority"
        );
    }
    for mutation in [
        "fs::write",
        "fs::copy",
        "fs::hard_link",
        "fs::create_dir",
        "fs::remove_",
        "fs::rename",
        "std::os::unix::fs::symlink",
        "std::os::windows::fs::symlink_file",
        "std::os::windows::fs::symlink_dir",
    ] {
        assert!(
            !contains_path_symbol(&code, mutation),
            "strict collector retains filesystem mutation authority: {mutation}"
        );
    }
    for forbidden in [
        "Command",
        "OpenOptions",
        "File::create",
        "write_all",
        "set_permissions",
        "symlink_file",
        "symlink_dir",
        "TierReceipt",
        "TierStatus",
        "WaterfallState",
        "WaterfallStore",
        "evaluate_tier2",
        "Tier2Input",
        "Tier2Scan",
        "scan_specialists",
        "load_or_derive",
        "AUTOSPEC_SPECIALIST_LLM_STUB_OUTPUT",
        "autospec-explore",
        "bash",
        "zsh",
        "run_shell",
        "omx",
        "curl",
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
            "strict collector retains prohibited authority: {forbidden}"
        );
    }
    assert!(
        !contains_path_symbol(&code, "git::checkout"),
        "strict collector retains git checkout authority"
    );
    assert!(
        !contains_code_token(&code, "symlink"),
        "strict collector retains symlink creation authority"
    );
}

#[test]
fn cutover_plan_states_tier2_completion_and_remaining_gates() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let plan = fs::read_to_string(
        root.join("../../docs/superpowers/plans/2026-07-16-rust-autonomous-waterfall.md"),
    )
    .expect("read autonomous waterfall plan")
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ");
    for required in [
        "Tier 2 strict collection, pure typed funnel, sealed receipt replay, and checked-in disabled policy are complete.",
        "A disabled policy produces `NotRun`, retains Tier 2, and is not a dry result.",
        "Live model activation remains a separate direct-child safety gate.",
        "Legacy deletion remains blocked on broader native producer activation and parity work.",
    ] {
        assert!(plan.contains(required), "cutover plan omits: {required}");
    }
}
