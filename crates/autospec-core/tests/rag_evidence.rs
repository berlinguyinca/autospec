//! Evidence, provenance and privacy (spec sections 10, 11, 53).

mod rag_support;

use autospec_core::rag::authority::SourceAuthority;
use autospec_core::rag::compression::{self, CompressionLevel};
use autospec_core::rag::evidence::{
    ContentForm, EvidenceBuilder, EvidenceCapture, Privacy, QueryProvenance, SourceLocation,
    content_hash,
};
use autospec_core::rag::score::Score;
use autospec_core::rag::scope::RetrievalScope;
use autospec_core::rag::source::SourceKind;
use rag_support::{NOW, capture, evidence, evidence_in, scope};

#[test]
fn evidence_citation_names_source_revision_authority_and_form() {
    let item = evidence(
        "ev_101",
        SourceKind::Repository,
        "crates/core/src/router.rs",
        "fn select_node()",
        SourceAuthority::Implementation,
        940,
    );

    let citation = item.citation();
    assert!(
        citation.starts_with("crates/core/src/router.rs:"),
        "{citation}"
    );
    assert!(
        citation.ends_with(" [9a223af] implementation (raw)"),
        "{citation}"
    );
}

#[test]
fn identical_content_at_the_same_revision_is_a_duplicate() {
    let left = evidence(
        "ev_1",
        SourceKind::Repository,
        "src/router.rs",
        "same body",
        SourceAuthority::Implementation,
        900,
    );
    let right = evidence(
        "ev_2",
        SourceKind::Repository,
        "src/router.rs",
        "same body",
        SourceAuthority::Implementation,
        900,
    );

    assert!(left.duplicates(&right));
}

#[test]
fn identical_content_at_different_revisions_is_not_a_duplicate() {
    let left = evidence(
        "ev_1",
        SourceKind::Repository,
        "src/router.rs",
        "same body",
        SourceAuthority::Implementation,
        900,
    );
    let right = evidence_in(
        "ev_2",
        SourceKind::Repository,
        "src/router.rs",
        "same body",
        SourceAuthority::Implementation,
        900,
        RetrievalScope::committed("autospec", "def456"),
    );

    assert!(!left.duplicates(&right));
}

#[test]
fn model_summarized_evidence_must_cite_its_sources() {
    let capture = capture(scope(), "planner");
    let error = EvidenceBuilder::new(
        "ev_summary",
        SourceKind::Repository,
        SourceLocation::document("src/router.rs"),
        SourceAuthority::Implementation,
        "routing picks the least-loaded node",
        &capture,
    )
    .form(ContentForm::ModelSummarized)
    .build()
    .expect_err("an unattributed summary must be rejected");

    assert!(error.contains("must cite the evidence"), "{error}");
}

#[test]
fn content_hash_is_stable_and_content_addressed() {
    assert_eq!(content_hash("router"), content_hash("router"));
    assert_ne!(content_hash("router"), content_hash("routers"));
    assert!(content_hash("router").starts_with("sha256:"));
}

#[test]
fn summary_inherits_the_strictest_privacy_of_its_sources() {
    let public = build_with_privacy("ev_public", Privacy::Public);
    let private = build_with_privacy("ev_private", Privacy::Private);

    let summary = compression::summarize(
        "ev_summary",
        CompressionLevel::Module,
        "the router delegates to the scheduler",
        &[public, private],
        NOW,
    )
    .expect("summary builds");

    assert_eq!(summary.privacy(), Privacy::Private);
}

#[test]
fn summary_cannot_be_more_authoritative_than_its_weakest_source() {
    let spec = evidence(
        "ev_spec",
        SourceKind::Specification,
        "docs/specs/routing.md",
        "timeout is 30s",
        SourceAuthority::AcceptedSpecification,
        900,
    );
    let blog = evidence(
        "ev_blog",
        SourceKind::Web,
        "https://example.invalid/post",
        "timeout is 30s",
        SourceAuthority::ExternalCommunity,
        400,
    );

    let summary = compression::summarize(
        "ev_summary",
        CompressionLevel::File,
        "timeout is 30s",
        &[spec, blog],
        NOW,
    )
    .expect("summary builds");

    assert_eq!(summary.authority(), SourceAuthority::ExternalCommunity);
    assert!(summary.form().is_model_transformed());
}

#[test]
fn summary_records_every_source_it_was_derived_from() {
    let first = evidence(
        "ev_1",
        SourceKind::Repository,
        "src/a.rs",
        "alpha",
        SourceAuthority::Implementation,
        900,
    );
    let second = evidence(
        "ev_2",
        SourceKind::Repository,
        "src/b.rs",
        "beta",
        SourceAuthority::Implementation,
        900,
    );

    let summary =
        compression::summarize("ev_s", CompressionLevel::Module, "a and b", &[first, second], NOW)
            .expect("summary builds");

    assert_eq!(summary.derived_from(), ["ev_1".to_string(), "ev_2".to_string()]);
}

#[test]
fn summary_without_sources_is_rejected() {
    let error = compression::summarize("ev_s", CompressionLevel::Module, "nothing", &[], NOW)
        .expect_err("a sourceless summary must be rejected");

    assert!(error.contains("at least one evidence item"), "{error}");
}

#[test]
fn line_ranges_must_be_one_based_and_ordered() {
    assert!(SourceLocation::lines("src/a.rs", 0, 5).is_err());
    assert!(SourceLocation::lines("src/a.rs", 9, 4).is_err());
    assert!(SourceLocation::lines("src/a.rs", 4, 4).is_ok());
}

#[test]
fn token_estimate_counts_short_identifier_lines_by_word() {
    // Four characters per token would undercount a line of short identifiers;
    // the word floor keeps the budget honest.
    assert!(compression::estimate_tokens("a b c d e f g h") >= 8);
    assert_eq!(compression::estimate_tokens(""), 0);
}

fn build_with_privacy(id: &str, privacy: Privacy) -> autospec_core::rag::evidence::Evidence {
    let capture = EvidenceCapture::new(
        scope(),
        QueryProvenance {
            original: "q".to_string(),
            rewritten: None,
            iteration: 1,
        },
        NOW,
        "planner",
    );
    EvidenceBuilder::new(
        id,
        SourceKind::Repository,
        SourceLocation::document("src/router.rs"),
        SourceAuthority::Implementation,
        "body",
        &capture,
    )
    .privacy(privacy)
    .scores(Score::ONE, Score::from_permille(900), Score::ONE)
    .build()
    .expect("valid evidence")
}
