use std::fs;
use std::path::{Path, PathBuf};

#[path = "support/tier3_authority_guard.rs"]
mod guard;
#[path = "support/tier2_authority_matcher.rs"]
mod matcher;

use guard::assert_no_execution_authority;
use matcher::{
    code_tokens, contains_path_symbol, contains_qualified_path, has_forbidden_std_module,
    has_module_escape,
};

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

fn collect_rust_sources(directory: &Path, sources: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("read Tier 4 module directory") {
        let path = entry.expect("read Tier 4 module entry").path();
        if path.is_dir() {
            collect_rust_sources(&path, sources);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
}

fn pure_tier4_sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sources = vec![root.join("src/autonomous/tier4.rs")];
    collect_rust_sources(&root.join("src/autonomous/tier4"), &mut sources);
    sources.sort();
    sources
}

fn production_source(relative: &str) -> String {
    fs::read_to_string(workspace_root().join(relative))
        .expect("read Tier 4 production source")
        .split("\n#[cfg(test)]")
        .next()
        .expect("production source before tests")
        .to_string()
}

fn code_without_comments_and_literals(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut code = String::new();
    let mut index = 0;
    while index < bytes.len() {
        if let Some(end) = raw_string_end(bytes, index) {
            code.push(' ');
            index = end;
            continue;
        }
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
            [b'"', ..] => {
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
        } else if bytes[index] == b'"' {
            return index + 1;
        } else {
            index += 1;
        }
    }
    index
}

fn raw_string_end(bytes: &[u8], index: usize) -> Option<usize> {
    let mut cursor = match bytes.get(index) {
        Some(b'r') => index + 1,
        Some(b'b') if bytes.get(index + 1) == Some(&b'r') => index + 2,
        _ => return None,
    };
    let hash_start = cursor;
    while bytes.get(cursor) == Some(&b'#') {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'"') {
        return None;
    }
    let hashes = cursor - hash_start;
    cursor += 1;
    while cursor < bytes.len() {
        let end = cursor.saturating_add(hashes + 1);
        if bytes[cursor] == b'"'
            && end <= bytes.len()
            && bytes[cursor + 1..end].iter().all(|byte| *byte == b'#')
        {
            return Some(end);
        }
        cursor += 1;
    }
    Some(cursor)
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

    for (relative, allows_store, allows_replay_read) in [
        (
            "crates/autospec-cli/src/commands/autonomous/tier4_receipts.rs",
            true,
            false,
        ),
        (
            "crates/autospec-cli/src/commands/autonomous/waterfall/evidence/tier4.rs",
            false,
            true,
        ),
        (
            "crates/autospec-cli/src/commands/autonomous/waterfall/evidence/tier4_consistency.rs",
            false,
            true,
        ),
        (
            "crates/autospec-cli/src/commands/autonomous/waterfall/evidence/tier4_shape.rs",
            false,
            false,
        ),
    ] {
        let code = code_without_comments_and_literals(&production_source(relative));
        assert!(!has_module_escape(&code), "{relative} escapes module scope");
        for module in ["env", "process", "net"] {
            assert!(
                !has_forbidden_std_module(&code, module),
                "{relative} retains std::{module} authority"
            );
        }
        assert_eq!(
            contains_path_symbol(&code, "fs::read_to_string"),
            allows_replay_read,
            "{relative} has an invalid replay-read boundary"
        );
        assert_tier4_purity(&code, relative, allows_store);
        for mutation in [
            "fs::write",
            "fs::copy",
            "fs::hard_link",
            "fs::create_dir",
            "fs::remove_dir",
            "fs::remove_file",
            "fs::rename",
            "File::create",
            "OpenOptions",
            "write_all",
            "set_permissions",
        ] {
            assert!(
                !contains_path_symbol(&code, mutation) && !contains_code_token(&code, mutation),
                "{relative} retains file mutation authority: {mutation}"
            );
        }
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
        "Activation requires a trusted typed source policy and policy-aware Tier 1.",
        "This foundation is not a full Rust cutover.",
        "Live retrieval, source activation, foreground wiring, Task 8 ideation, executor/premerge parity, validation/installer migration, and legacy deletion remain unchecked.",
    ] {
        assert!(plan.contains(required), "cutover plan omits: {required}");
    }
}
