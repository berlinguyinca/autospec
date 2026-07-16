use std::fs;
use std::path::{Path, PathBuf};

#[path = "support/tier3_authority_guard.rs"]
mod guard;
#[path = "support/tier2_authority_matcher.rs"]
mod matcher;
#[path = "support/tier4_authority_scanner.rs"]
mod scanner;
#[path = "support/tier4_cli_authority.rs"]
mod tier4_cli;

use guard::assert_no_execution_authority;
use matcher::{code_tokens, contains_qualified_path, has_forbidden_std_module, has_module_escape};
use scanner::{
    code_without_comments_and_literals, collect_rust_sources, collect_tier4_verifier_sources,
    production_code,
};
use tier4_cli::{assert_local_io, assert_no_direct_file_mutation, authority_sources, LocalIo};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn contains_code_token(code: &str, expected: &str) -> bool {
    code_tokens(code).iter().any(|token| token == expected)
}

fn contains_unqualified_call(code: &str, expected: &str) -> bool {
    let tokens = code_tokens(code);
    tokens.iter().enumerate().any(|(index, token)| {
        token == expected
            && tokens.get(index + 1).is_some_and(|token| token == "(")
            && !matches!(
                tokens.get(index.wrapping_sub(1)).map(String::as_str),
                Some("." | "::")
            )
    })
}

fn assert_tier4_purity(code: &str, scope: &str, allows_waterfall_store: bool) {
    assert_no_execution_authority(code, scope, allows_waterfall_store);
    let tokens = code_tokens(code);
    for window in tokens.windows(3) {
        assert!(
            window != ["crate", "::", "transport"],
            "{scope} retains an unapproved crate facade"
        );
    }
    for forbidden in [
        "fetch",
        "request",
        "send",
        "post",
        "put",
        "delete",
        "patch",
        "dispatch",
        "invoke",
        "infer",
        "Bytes",
        "ByteBuf",
        "ByteString",
    ] {
        assert!(
            !tokens.iter().any(|token| token == forbidden),
            "{scope} retains prohibited transport or model token: {forbidden}"
        );
    }
    for namespace in ["reqwest", "ureq", "hyper", "surf", "isahc", "awc"] {
        assert!(
            !contains_code_token(code, namespace),
            "{scope} retains direct or aliased HTTP authority: {namespace}"
        );
    }
    for token in ["http", "https", "Http", "HTTP"] {
        assert!(
            !contains_code_token(code, token),
            "{scope} retains qualified HTTP namespace or facade: {token}"
        );
    }
    // These unqualified names are reserved in Tier 4 because a free call has
    // no typed local receiver that can distinguish it from HTTP authority.
    for function in ["get", "head", "connect"] {
        assert!(
            !contains_unqualified_call(code, function),
            "{scope} retains free-function HTTP authority: {function}"
        );
    }
    assert!(
        !contains_qualified_path(code, "transport"),
        "{scope} retains transport path authority"
    );
    assert!(
        !tokens.iter().any(|token| token.ends_with("Gateway")
            || token.ends_with("Dispatcher")
            || token.ends_with("Facade")),
        "{scope} retains a transport or model facade"
    );
    assert!(
        !has_raw_byte_type(&tokens),
        "{scope} retains a raw byte payload type"
    );
}

fn has_raw_byte_type(tokens: &[String]) -> bool {
    for pattern in [
        &["Vec", "<", "u8", ">"][..],
        &["&", "[", "u8", "]"][..],
        &["[", "u8", ";"][..],
        &["Box", "<", "[", "u8", "]", ">"][..],
        &["Arc", "<", "[", "u8", "]", ">"][..],
    ] {
        if tokens.windows(pattern.len()).any(|window| {
            window
                .iter()
                .map(String::as_str)
                .eq(pattern.iter().copied())
        }) {
            return true;
        }
    }
    false
}

fn pure_tier4_sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sources = vec![root.join("src/autonomous/tier4.rs")];
    collect_rust_sources(&root.join("src/autonomous/tier4"), &mut sources);
    sources.sort();
    sources
}

fn production_source(relative: &str) -> String {
    production_code(
        &fs::read_to_string(workspace_root().join(relative))
            .expect("read Tier 4 production source"),
        relative,
    )
}

#[test]
fn tier4_core_recursively_rejects_all_external_and_mutation_authority() {
    let sources = pure_tier4_sources();
    assert_eq!(sources.len(), 6, "guard every declared Tier 4 source");
    for source in sources {
        let code = code_without_comments_and_literals(
            &fs::read_to_string(&source).expect("read pure Tier 4 source"),
        );
        assert!(
            !has_module_escape(&code),
            "{} escapes the Tier 4 module tree",
            source.display()
        );
        for module in ["fs", "io", "path", "env", "process", "net"] {
            assert!(
                !has_forbidden_std_module(&code, module),
                "{} retains std::{module} authority",
                source.display()
            );
        }
        assert_tier4_purity(&code, &source.display().to_string(), false);
        for forbidden in [
            "include",
            "http",
            "https",
            "Http",
            "HTTP",
            "Client",
            "WaterfallStore",
        ] {
            let tokens = code_tokens(&code);
            assert!(
                !tokens.iter().any(|token| token == forbidden),
                "{} retains prohibited token {forbidden}",
                source.display()
            );
        }
    }
}

#[test]
fn comment_and_literal_stripping_does_not_create_false_authority_findings() {
    let code = code_without_comments_and_literals(
        r##"// use std::fs; curl https://example.test
        let note = r#"WaterfallStore include! Vec<u8>"#;
        let valid = 1;"##,
    );
    assert!(!has_forbidden_std_module(&code, "fs"));
    assert!(!has_module_escape(&code));
    assert!(!code_tokens(&code).iter().any(|token| token == "curl"));
}

#[test]
fn authority_guard_rejects_transport_model_facades_and_raw_byte_wrappers() {
    for fixture in [
        "crate::transport::fetch();",
        "InferenceGateway::dispatch();",
        "reqwest::Client::new();",
        "use reqwest as facade; facade::get(source);",
        "crate::http::get(source);",
        "http::head(source);",
        "Http::connect(source);",
        "HttpFacade::open(source);",
        "get(source);",
        "type Cache = ReleaseRepository;",
        "use ReleaseStore as Cache;",
        "Command::new(shell).arg(script);",
        "let payload: [u8; 64];",
        "let payload: Box<[u8]>;",
        "let payload: Arc<[u8]>;",
        "pub struct Leak { payload: Bytes }",
    ] {
        let code = code_without_comments_and_literals(fixture);
        assert!(
            std::panic::catch_unwind(|| assert_tier4_purity(&code, "fixture", false)).is_err(),
            "Tier 4 guard missed {fixture}"
        );
    }
    for fixture in [
        "#[path = \"../escape.rs\"] mod escaped;",
        "#[cfg_attr(unix, path = \"../escape.rs\")] mod escaped;",
        "include!(\"../escape.rs\");",
    ] {
        assert!(
            has_module_escape(&code_without_comments_and_literals(fixture)),
            "Tier 4 guard missed module escape {fixture}"
        );
    }
    let alias = code_without_comments_and_literals("use std::fs::{write as save}; save(path);");
    assert!(
        std::panic::catch_unwind(|| assert_no_direct_file_mutation(&alias, "fixture", false))
            .is_err(),
        "Tier 4 guard missed nested file-mutation alias"
    );
    let removal_alias = code_without_comments_and_literals(
        "use std::fs; use std::fs::remove_file as erase; fs::read_to_string(path); atomic_write(path); fs::remove_file(first); fs::remove_file(second); erase(third);",
    );
    assert!(
        std::panic::catch_unwind(|| {
            assert_local_io(&removal_alias, "fixture", LocalIo::EvidenceDelegation)
        })
        .is_err(),
        "Tier 4 guard allowed an alias for approved local removal"
    );
}

#[test]
fn authority_source_after_test_module_cannot_evade_scan() {
    let source =
        "fn guarded() {}\n#[cfg(test)] mod tests {}\nfn hidden() { crate::http::get(source); }";
    assert!(
        std::panic::catch_unwind(|| production_code(source, "fixture")).is_err(),
        "production authority after a test module evaded the scan"
    );
}

#[test]
fn cli_authority_discovery_finds_future_nested_tier4_helpers() {
    let root = std::env::temp_dir().join(format!(
        "autospec-tier4-cli-authority-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("nested/tier4")).expect("create nested verifier tree");
    fs::write(root.join("nested/tier4_future.rs"), "fn guarded() {}\n")
        .expect("write future Tier 4 helper");
    fs::write(root.join("nested/tier4/parser.rs"), "fn guarded() {}\n")
        .expect("write future nested Tier 4 helper");
    fs::write(root.join("nested/unrelated.rs"), "fn unrelated() {}\n")
        .expect("write unrelated helper");
    let mut sources = Vec::new();
    collect_tier4_verifier_sources(&root, &mut sources);
    sources.sort();
    assert_eq!(
        sources,
        [
            root.join("nested/tier4/parser.rs"),
            root.join("nested/tier4_future.rs"),
        ]
    );
    fs::remove_dir_all(root).expect("remove nested verifier tree");
}

#[test]
fn tier4_cli_boundary_allows_only_disabled_policy_and_local_receipt_replay() {
    let adapter = code_without_comments_and_literals(&production_source(
        "crates/autospec-cli/src/commands/autonomous/tier4.rs",
    ));
    assert_eq!(
        adapter
            .matches("Tier4Input::DisabledByCheckedInPolicy")
            .count(),
        1
    );
    assert!(!has_module_escape(&adapter));
    for module in ["fs", "io", "path", "env", "process", "net"] {
        assert!(!has_forbidden_std_module(&adapter, module));
    }
    assert_tier4_purity(&adapter, "Tier 4 adapter", false);

    let sources = authority_sources(&workspace_root());
    for required in [
        "tier4_receipts.rs",
        "waterfall/evidence.rs",
        "waterfall/tier_evidence.rs",
        "waterfall/evidence/tier4.rs",
        "tier4_consistency.rs",
        "tier4_shape.rs",
    ] {
        assert!(
            sources
                .iter()
                .any(|(source, _, _)| source.ends_with(required)),
            "Tier 4 CLI authority discovery omitted {required}"
        );
    }
    for (relative, allows_store, io) in sources {
        let code = code_without_comments_and_literals(&production_source(&relative));
        assert!(!has_module_escape(&code), "{relative} escapes module scope");
        for module in ["env", "process", "net"] {
            assert!(
                !has_forbidden_std_module(&code, module),
                "{relative} retains std::{module} authority"
            );
        }
        assert_tier4_purity(&code, &relative, allows_store);
        assert_local_io(&code, &relative, io);
    }
}

#[test]
fn cutover_plan_states_tier4_foundation_without_claiming_activation() {
    let plan = fs::read_to_string(
        workspace_root().join("docs/superpowers/plans/2026-07-16-rust-autonomous-waterfall.md"),
    )
    .expect("read autonomous waterfall plan")
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ");
    for required in [
        "Tier 4 typed source policy, pure typed funnel, sealed receipt replay, and checked-in disabled policy are complete.",
        "A nonempty parsed source configuration is data, not activation.",
        "Activation requires direct bounded retrieval and a trusted typed source policy.",
        "This foundation is not a full Rust cutover.",
        "Live retrieval, source activation, Task 8 ideation, executor/premerge parity, validation/installer migration, and legacy deletion remain unchecked.",
        "Tier 1.5 pure observation, read-only paginated collection, and sealed receipt replay are complete.",
        "Tier 1.5 foreground admission and mutation remain unchecked.",
    ] {
        assert!(plan.contains(required), "cutover plan omits: {required}");
    }
}
