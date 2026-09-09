//! Populated-case coverage for declaration-only conflict resolution (#3935).
//!
//! The shape that stranded four dispatches: a 20+ line `mod.rs` whose only
//! conflict is one additive hunk among module declarations. The check must
//! be exercised against a populated case — auto-resolve the clean one, and
//! refuse a variant where a single line of real code enters the same hunk.

use autospec_core::execution::{hold_shape, resolve, RefusalKind, ResolveOutcome};

/// `crates/autospec-core/src/insights/mod.rs` as it stood on `main` before
/// the correlate stage landed: the doc header, three module declarations,
/// and the re-export block.
const DOC_HEADER: &str = "\
//! Continuous Improvement Engine subsystem.
//!
//! Spec: `docs/specs/2026-09-08-continuous-improvement-engine.md`.
//!
//! The engine is a loop over session telemetry: ingest raw harness sessions,
//! normalize them, detect patterns, find findings, propose improvements,
//! evaluate them, and verify them after deployment. This module owns the
//! stages that have landed so far; ingestion, storage, enrichment,
//! redaction and the CLI surfaces live in their own issues.
//!
//! The analytics here are recommendation-only (spec §18): nothing in this
//! module is applied to live dispatch or written to any routing policy file.";

const USE_BLOCK: &str = "\
pub use models::{
    model_performance, recommend, ModelPerformance, RecommendationConfidence,
    RoutingRecommendation, Window, TASK_DIMENSIONS,
};";

/// The conflicted file: `ours` (main) holds the three declarations, `theirs`
/// (the patch) adds `pub mod correlate;` in the same hunk. Everything else
/// in the file is outside the hunk.
fn populated_conflict(theirs_extra: &[&str]) -> (&'static str, String) {
    let file = "crates/autospec-core/src/insights/mod.rs";
    let theirs = [
        "pub mod config;",
        "pub mod models;",
        "pub mod proposals;",
        "pub mod correlate;",
    ]
    .iter()
    .chain(theirs_extra.iter())
    .copied()
    .collect::<Vec<_>>()
    .join("\n");
    let content = format!(
        "{DOC_HEADER}\n\n<<<<<<< HEAD\npub mod config;\npub mod models;\npub mod proposals;\n=======\n{theirs}\n>>>>>>> feat/insights-correlate\n\n{USE_BLOCK}\n",
    );
    (file, content)
}

#[test]
fn populated_declaration_only_conflict_auto_resolves() {
    let (file, content) = populated_conflict(&[]);

    // The hold record names the file, the hunk count, and the shape.
    let shape = hold_shape(file, &content);
    assert!(shape.declaration_only, "{shape}");
    assert_eq!(shape.hunks, 1);
    assert_eq!(
        shape.to_string(),
        format!("{file}: 1 hunk(s), module declarations only")
    );

    match resolve(file, &content) {
        ResolveOutcome::Resolved(resolved) => {
            assert_eq!(resolved.kept, vec!["config", "models", "proposals"]);
            assert_eq!(resolved.added, vec!["correlate"]);
            assert!(!resolved.is_verified());

            // Invariant 3: not offered before compile and tests have passed.
            assert!(resolved.clone().into_content().is_err());

            let content = resolved
                .mark_verified()
                .into_content()
                .expect("verified resolution is offerable");
            assert_eq!(
                content,
                format!("{DOC_HEADER}\n\npub mod config;\npub mod models;\npub mod proposals;\npub mod correlate;\n\n{USE_BLOCK}\n")
            );
            // The markers are gone and the out-of-hunk code survived verbatim.
            assert!(!content.contains("<<<<<<<"));
            assert!(content.contains(USE_BLOCK));
        }
        other => panic!("expected Resolved, got {other:?}"),
    }
}

#[test]
fn populated_conflict_with_one_line_of_real_code_refuses() {
    let (file, content) = populated_conflict(&["pub fn correlate_debug() -> u32 { 0 }"]);

    let shape = hold_shape(file, &content);
    assert!(!shape.declaration_only, "{shape}");
    assert_eq!(shape.hunks, 1);
    let refusal = shape.refusal.expect("stated reason");
    assert!(
        refusal.contains("code beyond module declarations"),
        "{refusal}"
    );
    assert!(refusal.contains("correlate_debug"), "{refusal}");

    match resolve(file, &content) {
        ResolveOutcome::Refused(r) => {
            assert_eq!(r.kind, RefusalKind::CodeBeyondDeclarations);
            assert_eq!(r.file, file);
            assert!(
                r.to_string()
                    .contains("pub fn correlate_debug() -> u32 { 0 }"),
                "the reason names the offending line: {r}"
            );
        }
        other => panic!("expected Refused, got {other:?}"),
    }
}
