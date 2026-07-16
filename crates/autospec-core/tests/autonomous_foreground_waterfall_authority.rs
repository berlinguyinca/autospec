use std::fs;
use std::path::{Path, PathBuf};

#[path = "support/tier2_authority_matcher.rs"]
#[allow(dead_code)]
mod matcher;
#[path = "support/tier4_authority_scanner.rs"]
#[allow(dead_code)]
mod scanner;

use matcher::{code_tokens, contains_path_symbol, has_module_escape};
use scanner::code_without_comments_and_literals;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn collect_foreground_sources(root: &Path) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    let module = root.join("foreground_waterfall.rs");
    if module.is_file() {
        sources.push(module);
    }
    collect_rust_sources_if_present(&root.join("foreground_waterfall"), &mut sources);
    sources.sort();
    sources
}

fn collect_rust_sources_if_present(directory: &Path, sources: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries {
        let path = entry.expect("read foreground authority entry").path();
        if path.is_dir() {
            collect_rust_sources_if_present(&path, sources);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
}

fn production_tokens(path: &Path) -> Vec<String> {
    let source = fs::read_to_string(path).expect("read foreground authority source");
    let production = source.split("\n#[cfg(test)]").next().unwrap_or(&source);
    code_tokens(&code_without_comments_and_literals(production))
}

fn function_tokens(path: &Path, name: &str) -> Vec<String> {
    let source = fs::read_to_string(path).expect("read foreground wiring source");
    let tokens = code_tokens(&code_without_comments_and_literals(&source));
    let start = tokens
        .windows(2)
        .position(|window| window == ["fn", name])
        .unwrap_or_else(|| panic!("missing function {name}"));
    let open = tokens[start..]
        .iter()
        .position(|token| token == "{")
        .map(|offset| start + offset)
        .unwrap_or_else(|| panic!("missing body for {name}"));
    let mut depth = 0_u32;
    for index in open..tokens.len() {
        match tokens[index].as_str() {
            "{" => depth += 1,
            "}" => {
                depth = depth.checked_sub(1).expect("balanced function braces");
                if depth == 0 {
                    return tokens[start..=index].to_vec();
                }
            }
            _ => {}
        }
    }
    panic!("unterminated function {name}");
}

fn count_path(tokens: &[String], expected: &[&str]) -> usize {
    tokens
        .windows(expected.len())
        .filter(|window| {
            window
                .iter()
                .map(String::as_str)
                .eq(expected.iter().copied())
        })
        .count()
}

fn count_calls(tokens: &[String], function: &str) -> usize {
    tokens
        .windows(2)
        .filter(|window| window[0] == function && window[1] == "(")
        .count()
}

fn assert_no_foreground_authority(tokens: &[String], scope: &str) {
    for forbidden in [
        "Command",
        "shell",
        "bash",
        "zsh",
        "legacy",
        "drain",
        "queue",
        "admit",
        "admission",
        "claim",
        "executor",
        "why_no_work",
        "ideate",
        "ideation",
    ] {
        assert!(
            !tokens.iter().any(|token| token == forbidden),
            "{scope} retains prohibited authority token {forbidden}"
        );
    }
    for forbidden in [
        &["NoWorkState", "::", "record"][..],
        &["issue", "::", "edit"][..],
        &["issue", "::", "comment"][..],
        &["label", "::", "add"][..],
        &["label", "::", "remove"][..],
    ] {
        assert_eq!(
            count_path(tokens, forbidden),
            0,
            "{scope} retains mutation authority"
        );
    }
    for write_verb in ["POST", "PUT", "PATCH", "DELETE"] {
        assert!(
            !tokens.iter().any(|token| token == write_verb),
            "{scope} retains GitHub write verb {write_verb}"
        );
    }
}

#[test]
fn foreground_dispatcher_inventory_is_recursive_and_closed() {
    let root = workspace_root().join("crates/autospec-cli/src/commands/autonomous");
    let sources = collect_foreground_sources(&root);
    assert_eq!(sources, [root.join("foreground_waterfall.rs")]);

    let tokens = production_tokens(&sources[0]);
    assert_no_foreground_authority(&tokens, "foreground dispatcher");
    assert!(!has_module_escape(&tokens.join(" ")));

    for (operation, expected) in [
        ("record_tier_one", 1),
        ("record_tier15_with_lease", 1),
        ("record_tier2_with_lease", 1),
        ("record_tier3_with_lease", 1),
        ("record_tier4_with_lease", 1),
        ("disabled_by_checked_in_policy", 3),
        ("with_current_lifecycle_lease", 2),
        ("acquire_with_policy", 1),
    ] {
        assert_eq!(
            count_calls(&tokens, operation),
            expected,
            "foreground dispatcher changed closed operation count for {operation}"
        );
    }
}

#[test]
fn foreground_wiring_only_delegates_empty_repository_traversal() {
    let source = workspace_root().join("crates/autospec-cli/src/commands/autonomous.rs");
    let tokens = function_tokens(&source, "scan_foreground");
    assert_no_foreground_authority(&tokens, "scan_foreground");

    assert_eq!(
        count_path(&tokens, &["foreground_waterfall", "::", "run_one_tier"]),
        1
    );
    assert_eq!(
        count_path(&tokens, &["waterfall_coordinator", "::", "record_tier_one"]),
        1
    );
    assert_eq!(
        count_path(
            &tokens,
            &["waterfall_coordinator", "::", "should_start_tier_one"]
        ),
        1
    );
    assert_eq!(
        count_path(&tokens, &["ConductorEvent", "::", "ScanFoundWork"]),
        1
    );
    assert_eq!(
        count_path(&tokens, &["ConductorEvent", "::", "ScanEmpty"]),
        1
    );
}

#[test]
fn tier4_config_is_trust_context_for_the_disabled_adapter_only() {
    let source = workspace_root()
        .join("crates/autospec-cli/src/commands/autonomous/foreground_waterfall.rs");
    let tokens = production_tokens(&source);

    assert_eq!(
        count_path(&tokens, &["WaterfallPolicy", "::", "from_config"]),
        1
    );
    assert_eq!(count_path(&tokens, &["config", ".", "tier4"]), 1);
    assert_eq!(
        count_path(&tokens, &["tier4", "::", "disabled_by_checked_in_policy"]),
        1
    );
    assert_eq!(count_path(&tokens, &["tier4", "::", "scan"]), 0);
    assert!(!contains_path_symbol(&tokens.join(" "), "tier4::fetch"));
}

#[test]
fn recursive_inventory_finds_nested_helpers_and_ignores_inert_text() {
    let root = std::env::temp_dir().join(format!(
        "autospec-foreground-authority-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("foreground_waterfall/nested"))
        .expect("create nested foreground fixture");
    fs::write(root.join("foreground_waterfall.rs"), "fn root() {}\n").expect("write root fixture");
    fs::write(
        root.join("foreground_waterfall/nested/helper.rs"),
        "fn helper() { let note = \"Command legacy drain\"; }\n",
    )
    .expect("write nested fixture");

    let sources = collect_foreground_sources(&root);
    assert_eq!(sources.len(), 2);
    for source in sources {
        assert_no_foreground_authority(&production_tokens(&source), "fixture");
    }
    fs::remove_dir_all(root).expect("remove foreground fixture");
}

#[test]
fn cutover_checklist_records_traversal_without_activating_later_gates() {
    let plan = fs::read_to_string(
        workspace_root().join("docs/superpowers/plans/2026-07-16-rust-autonomous-waterfall.md"),
    )
    .expect("read autonomous waterfall cutover plan")
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ");

    for completed in [
        "## Foreground cursor traversal — complete",
        "- [x] Resume exactly one current cursor tier on a genuinely empty repository queue",
        "Current production traversal advances through empty Tier 1 and Tier 1.5 observations, then stops at Tier 2 `NotRun`",
    ] {
        assert!(plan.contains(completed), "cutover plan omits: {completed}");
    }
    for incomplete in [
        "- [ ] Activate normal Rust queue admission only after Tier 1.5 is integrated",
        "- [ ] Activate Tier 2 model-backed local discovery",
        "- [ ] Activate Tier 3 trusted metadata collection",
        "- [ ] Activate Tier 4 bounded external retrieval",
        "- [ ] Write `why-no-work.json` only after the five verified receipt outcomes",
        "- [ ] Edge-trigger `autospec autonomous ideate`",
        "- [ ] Complete native executor and premerge parity",
        "- [ ] Migrate validation and installer ownership",
        "- [ ] Run workspace formatting, clippy, tests, fast validation",
        "- [ ] Only after parity review, remove legacy waterfall ownership",
    ] {
        assert!(
            plan.contains(incomplete),
            "cutover gate is missing or no longer unchecked: {incomplete}"
        );
    }
}
