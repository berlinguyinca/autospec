//! Issue #3323: repository knowledge cache and staged test resolution.
//!
//! Acceptance criteria under test:
//! 1. A changed Rust source file resolves at least one Stage 1 command.
//! 2. An unchanged module's cache entry stays byte-identical after refresh.
//! 3. Stage 2 cannot run before Stage 1 passes.
//! 4. This binary: `cargo test -p autospec-core targeted_test_resolver`.
//!
//! The Rust, shell, and documentation fixture mappings are exercised.

use std::collections::BTreeMap;

use autospec_core::aar::knowledge::{
    digest_of, language_for, resolve_stages, scan_module, KnowledgeCache, Stage, StageProgress,
    REPOSITORY_TEST_COMMAND,
};

// --- acceptance criterion 1 ----------------------------------------------------

#[test]
fn targeted_test_resolver_changed_rust_source_resolves_at_least_one_stage_one_command() {
    let cache = KnowledgeCache::new();
    let plan = resolve_stages(&["crates/autospec-core/src/aar/knowledge.rs"], &cache);

    let stage1 = plan.commands(Stage::One);
    assert!(
        !stage1.is_empty(),
        "a changed Rust source file must resolve at least one Stage 1 command"
    );
    assert_eq!(
        stage1,
        &["cargo test -p autospec-core aar::knowledge".to_string()]
    );
    assert_eq!(
        plan.commands(Stage::Two),
        &["cargo test -p autospec-core".to_string()]
    );
    assert_eq!(
        plan.commands(Stage::Three),
        &[REPOSITORY_TEST_COMMAND.to_string()]
    );
}

#[test]
fn targeted_test_resolver_changed_rust_integration_test_file_resolves_its_binary() {
    let cache = KnowledgeCache::new();
    let plan = resolve_stages(
        &["crates/autospec-core/tests/targeted_test_resolver.rs"],
        &cache,
    );
    assert_eq!(
        plan.commands(Stage::One),
        &["cargo test -p autospec-core targeted_test_resolver".to_string()]
    );
}

#[test]
fn targeted_test_resolver_changed_rust_module_file_mod_resolves_its_directory_module() {
    let cache = KnowledgeCache::new();
    let plan = resolve_stages(&["crates/autospec-core/src/aar/mod.rs"], &cache);
    assert_eq!(
        plan.commands(Stage::One),
        &["cargo test -p autospec-core aar".to_string()]
    );
}

// --- acceptance criterion 2 ----------------------------------------------------

#[test]
fn targeted_test_resolver_unchanged_module_entry_stays_byte_identical_after_refresh() {
    let unchanged_source = "pub fn stable() {}\n";
    let changed_before = "pub fn alpha() {}\n";
    let changed_after = "pub fn alpha() {}\npub fn beta() {}\n";

    let mut sources = BTreeMap::new();
    sources.insert(
        "crates/demo/src/lib.rs".to_string(),
        unchanged_source.to_string(),
    );
    sources.insert(
        "crates/demo/src/alpha.rs".to_string(),
        changed_before.to_string(),
    );

    let mut cache = KnowledgeCache::new();
    for (path, contents) in &sources {
        cache.record(scan_module(path, contents));
    }

    let untouched_before = cache
        .get("crates/demo/src/lib.rs")
        .expect("record exists before refresh")
        .clone();
    let untouched_bytes_before = KnowledgeCache::render_record(&untouched_before);

    sources.insert(
        "crates/demo/src/alpha.rs".to_string(),
        changed_after.to_string(),
    );
    let report = cache.refresh(&sources);

    assert_eq!(
        report.unchanged,
        vec!["crates/demo/src/lib.rs".to_string()],
        "the unchanged module must be reported unchanged"
    );
    assert_eq!(
        report.refreshed,
        vec!["crates/demo/src/alpha.rs".to_string()],
        "only the changed module is re-derived"
    );

    let untouched_after = cache
        .get("crates/demo/src/lib.rs")
        .expect("record survives refresh");
    assert_eq!(untouched_after, &untouched_before);
    assert_eq!(
        KnowledgeCache::render_record(untouched_after),
        untouched_bytes_before,
        "an unchanged module cache entry must stay byte-identical"
    );

    let changed = cache
        .get("crates/demo/src/alpha.rs")
        .expect("changed record is re-derived");
    assert_eq!(changed.source_digest, digest_of(changed_after));
    assert!(changed.symbols.contains(&"beta".to_string()));
}

#[test]
fn targeted_test_resolver_refresh_drops_removed_entries() {
    let mut sources = BTreeMap::new();
    sources.insert(
        "crates/demo/src/keep.rs".to_string(),
        "pub fn keep() {}\n".to_string(),
    );
    sources.insert(
        "crates/demo/src/gone.rs".to_string(),
        "pub fn gone() {}\n".to_string(),
    );
    let mut cache = KnowledgeCache::new();
    for (path, contents) in &sources {
        cache.record(scan_module(path, contents));
    }

    sources.remove("crates/demo/src/gone.rs");
    let report = cache.refresh(&sources);

    assert_eq!(report.removed, vec!["crates/demo/src/gone.rs".to_string()]);
    assert!(cache.get("crates/demo/src/gone.rs").is_none());
    assert!(cache.get("crates/demo/src/keep.rs").is_some());
}

#[test]
fn targeted_test_resolver_cache_render_is_deterministic_and_round_trips() {
    let mut sources = BTreeMap::new();
    sources.insert(
        "crates/demo/src/b.rs".to_string(),
        "pub fn b() {}\n".to_string(),
    );
    sources.insert(
        "crates/demo/src/a.rs".to_string(),
        "pub fn a() {}\n".to_string(),
    );
    let mut cache = KnowledgeCache::new();
    for (path, contents) in &sources {
        cache.record(scan_module(path, contents));
    }

    let bytes = cache.render();
    let mut loaded = KnowledgeCache::new();
    loaded.load(&bytes).expect("cache loads");
    assert_eq!(loaded.render(), bytes, "rendering is byte-stable");
    assert_eq!(loaded, cache);
}

// --- acceptance criterion 3 ----------------------------------------------------

#[test]
fn targeted_test_resolver_stage_two_cannot_run_before_stage_one_passes() {
    let mut progress = StageProgress::new();

    assert_eq!(progress.next(), Some(Stage::One));
    assert!(
        progress.allow(Stage::Two).is_err(),
        "stage 2 must be blocked before stage 1 passes"
    );
    assert!(progress.allow(Stage::Three).is_err());

    progress.record(Stage::One, false);
    assert!(
        progress.allow(Stage::Two).is_err(),
        "a failed stage 1 still blocks stage 2"
    );
    assert_eq!(
        progress.next(),
        Some(Stage::One),
        "a failed stage 1 is retried, never skipped"
    );

    progress.record(Stage::One, true);
    assert!(progress.allow(Stage::Two).is_ok());
    assert!(
        progress.allow(Stage::Three).is_err(),
        "stage 3 waits for stage 2"
    );

    progress.record(Stage::Two, true);
    assert!(progress.allow(Stage::Three).is_ok());
    progress.record(Stage::Three, true);
    assert_eq!(progress.next(), None);
}

// --- fixture mappings (Rust, shell, documentation) ------------------------------

#[test]
fn targeted_test_resolver_rust_shell_and_documentation_fixture_mappings() {
    let cache = KnowledgeCache::new();
    let plan = resolve_stages(
        &[
            "scripts/autospec-stop-check.sh",
            "tests/fixture.bats",
            "docs/superpowers/specs/foundation.md",
            "crates/autospec-core/src/aar/mod.rs",
        ],
        &cache,
    );

    let stage1 = plan.commands(Stage::One);
    assert!(stage1.contains(&"bash -n scripts/autospec-stop-check.sh".to_string()));
    assert!(stage1.contains(&"bats tests/fixture.bats".to_string()));
    assert!(stage1.contains(&"cargo test -p autospec-core aar".to_string()));
    assert!(
        stage1.contains(&REPOSITORY_TEST_COMMAND.to_string()),
        "documentation resolves to the repository check"
    );
    assert!(plan
        .commands(Stage::Two)
        .contains(&"cargo test -p autospec-core".to_string()));
    assert_eq!(
        plan.commands(Stage::Three),
        &[REPOSITORY_TEST_COMMAND.to_string()]
    );
}

#[test]
fn targeted_test_resolver_resolve_stages_deduplicates_repeated_paths() {
    let cache = KnowledgeCache::new();
    let plan = resolve_stages(
        &[
            "crates/autospec-core/src/aar/knowledge.rs",
            "crates/autospec-core/src/aar/knowledge.rs",
        ],
        &cache,
    );
    assert_eq!(
        plan.commands(Stage::One),
        &["cargo test -p autospec-core aar::knowledge".to_string()]
    );
}

#[test]
fn targeted_test_resolver_cache_records_feed_staged_resolution() {
    let mut cache = KnowledgeCache::new();
    cache.record(scan_module(
        "crates/demo/src/thing.rs",
        "pub fn do_thing() {}\n",
    ));

    let plan = resolve_stages(&["crates/demo/src/thing.rs"], &cache);
    assert_eq!(
        plan.commands(Stage::One),
        &["cargo test -p demo thing".to_string()]
    );
    assert_eq!(
        plan.commands(Stage::Two),
        &["cargo test -p demo".to_string()]
    );
    assert_eq!(
        plan.commands(Stage::Three),
        &[REPOSITORY_TEST_COMMAND.to_string()]
    );
}

// --- record derivation fixtures ---------------------------------------------------

#[test]
fn targeted_test_resolver_scan_module_records_rust_symbols_dependencies_and_interfaces() {
    let source = "use crate::support;\nuse super::other;\nuse serde::Serialize;\n\npub fn do_thing() -> i32 { 1 }\n\nfn helper() {}\n\npub struct Thing;\n\nmod nested;\n";
    let record = scan_module("crates/demo/src/thing.rs", source);

    assert_eq!(record.module, "demo::thing");
    assert!(record.symbols.contains(&"do_thing".to_string()));
    assert!(record.symbols.contains(&"helper".to_string()));
    assert!(record.symbols.contains(&"Thing".to_string()));
    assert!(record.symbols.contains(&"nested".to_string()));
    assert_eq!(record.dependencies, vec!["crate::support", "super::other"]);
    assert_eq!(record.interfaces, vec!["do_thing".to_string()]);
    assert_eq!(record.build_command, "cargo build -p demo");
    assert_eq!(
        record.test_commands,
        vec![
            "cargo test -p demo thing".to_string(),
            "cargo test -p demo".to_string(),
            REPOSITORY_TEST_COMMAND.to_string(),
        ]
    );
    assert_eq!(record.source_digest, digest_of(source));
}

#[test]
fn targeted_test_resolver_scan_module_maps_shell_fixture() {
    let record = scan_module(
        "scripts/fixture.sh",
        "#!/usr/bin/env bash\n\nrun_step() {\n  echo step\n}\n\nsource ./scripts/lib.sh\n\nmain() { run_step }\n\ncheck --verbose\n",
    );

    assert_eq!(language_for("scripts/fixture.sh").as_str(), "shell");
    assert!(record.symbols.contains(&"run_step".to_string()));
    assert!(record.symbols.contains(&"main".to_string()));
    assert_eq!(record.dependencies, vec!["./scripts/lib.sh".to_string()]);
    assert!(record.interfaces.contains(&"--verbose".to_string()));
    assert_eq!(record.build_command, "bash -n scripts/fixture.sh");
    assert_eq!(
        record.test_commands,
        vec![
            "bash -n scripts/fixture.sh".to_string(),
            REPOSITORY_TEST_COMMAND.to_string()
        ]
    );
}

#[test]
fn targeted_test_resolver_scan_module_maps_documentation_fixture() {
    let record = scan_module(
        "docs/guide.md",
        "# Title\n\nSee [spec](docs/specs/foundation.md) and [web](https://example.com).\n\n## Detail\n",
    );

    assert_eq!(language_for("docs/guide.md").as_str(), "documentation");
    // Maps are stored sorted, which keeps cache rendering byte-stable.
    assert_eq!(
        record.symbols,
        vec!["Detail".to_string(), "Title".to_string()]
    );
    assert_eq!(
        record.interfaces,
        vec!["Detail".to_string(), "Title".to_string()]
    );
    assert_eq!(
        record.dependencies,
        vec!["docs/specs/foundation.md".to_string()]
    );
    assert_eq!(
        record.test_commands,
        vec![REPOSITORY_TEST_COMMAND.to_string()]
    );
}
