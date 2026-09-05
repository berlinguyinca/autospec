//! The shipped harness alias table must have a row for every harness we claim
//! to support.
//!
//! Regression guard. `HarnessKind::parse` accepted "pi" and
//! `HarnessKind::Pi` existed in the enum, but `config/harness-runtime-aliases.tsv`
//! had no `pi` row — so every Pi dispatch failed at `resolve_kind` with
//! "executor harness alias missing: pi". Two resolvers disagreed: one could name
//! the harness, the other could not find it. Nothing caught that, because
//! neither the enum nor the parser is wrong on its own; only the pairing is.

use std::fs;
use std::path::PathBuf;

fn shipped_table() -> String {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "..", "config", "harness-runtime-aliases.tsv"]
        .iter()
        .collect();
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn every_supported_harness_kind_has_a_row() {
    let body = shipped_table();
    let kinds: Vec<&str> = body
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split('\t').next())
        .map(str::trim)
        .collect();

    // Keep in step with HarnessKind. Adding a variant without a row here is the
    // exact defect this test exists to catch.
    for expected in ["claude", "codex", "opencode", "pi"] {
        assert!(
            kinds.contains(&expected),
            "no `{expected}` row in config/harness-runtime-aliases.tsv; rows found: {kinds:?}"
        );
    }
}

#[test]
fn every_row_has_exactly_four_tab_separated_columns() {
    for (n, line) in shipped_table().lines().enumerate() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let cols = line.split('\t').count();
        assert_eq!(cols, 4, "row {} has {} columns, want 4: {line:?}", n + 1, cols);
    }
}
