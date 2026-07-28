use std::path::Path;

use crate::matcher::{code_tokens, contains_path_symbol};
use crate::scanner::collect_tier4_verifier_sources;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalIo {
    None,
    ReplayRead,
    EvidenceDelegation,
    TierEvidence,
}

pub(crate) fn authority_sources(root: &Path) -> Vec<(String, bool, LocalIo)> {
    let evidence_root = root.join("crates/autospec-cli/src/commands/autonomous/waterfall/evidence");
    let mut helpers = Vec::new();
    collect_tier4_verifier_sources(&evidence_root, &mut helpers);
    let mut sources = vec![
        (
            "crates/autospec-cli/src/commands/autonomous/tier4_receipts.rs".to_string(),
            true,
            LocalIo::None,
        ),
        (
            "crates/autospec-cli/src/commands/autonomous/waterfall/evidence.rs".to_string(),
            false,
            LocalIo::EvidenceDelegation,
        ),
        (
            "crates/autospec-cli/src/commands/autonomous/waterfall/tier_evidence.rs".to_string(),
            true,
            LocalIo::TierEvidence,
        ),
    ];
    for path in helpers {
        let relative = path.strip_prefix(root).expect("Tier 4 helper in workspace");
        let file = path.file_name().and_then(|name| name.to_str());
        let io = if matches!(file, Some("tier4.rs" | "tier4_consistency.rs")) {
            LocalIo::ReplayRead
        } else {
            LocalIo::None
        };
        sources.push((relative.to_string_lossy().into_owned(), false, io));
    }
    sources.sort_by(|left, right| left.0.cmp(&right.0));
    sources
}

pub(crate) fn assert_local_io(code: &str, scope: &str, io: LocalIo) {
    assert_eq!(
        contains_path_symbol(code, "fs::read_to_string"),
        matches!(io, LocalIo::ReplayRead | LocalIo::EvidenceDelegation),
        "{scope} has an invalid replay-read boundary"
    );
    match io {
        LocalIo::EvidenceDelegation => assert_evidence_delegation(code, scope),
        LocalIo::TierEvidence => assert_tier_evidence_delegation(code, scope),
        LocalIo::None | LocalIo::ReplayRead => assert_no_direct_file_mutation(code, scope, false),
    }
}

pub(crate) fn assert_no_direct_file_mutation(code: &str, scope: &str, allows_remove_file: bool) {
    for leaf in [
        "write",
        "copy",
        "hard_link",
        "create_dir",
        "remove_dir",
        "remove_file",
        "rename",
    ] {
        if leaf == "remove_file" && allows_remove_file {
            continue;
        }
        assert!(
            !contains_code_token(code, leaf),
            "{scope} retains direct or aliased file mutation authority: {leaf}"
        );
    }
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
        if mutation == "fs::remove_file" && allows_remove_file {
            continue;
        }
        assert!(
            !contains_path_symbol(code, mutation) && !contains_code_token(code, mutation),
            "{scope} retains file mutation authority: {mutation}"
        );
    }
}

fn assert_evidence_delegation(code: &str, scope: &str) {
    assert!(contains_code_token(code, "atomic_write"));
    assert_no_direct_file_mutation(code, scope, true);
    let tokens = code_tokens(code);
    let qualified_calls = tokens
        .windows(4)
        .filter(|window| *window == ["fs", "::", "remove_file", "("])
        .count();
    let leaf_count = tokens
        .iter()
        .filter(|token| *token == "remove_file")
        .count();
    assert_eq!(
        qualified_calls, 4,
        "{scope} must retain exactly four approved fs::remove_file calls"
    );
    assert_eq!(
        leaf_count, qualified_calls,
        "{scope} imports, aliases, or invokes bare remove_file authority"
    );
}

fn assert_tier_evidence_delegation(code: &str, scope: &str) {
    assert_no_direct_file_mutation(code, scope, false);
    let tokens = code_tokens(code);
    for token in tokens
        .iter()
        .filter(|token| token.to_ascii_lowercase().contains("tier4"))
    {
        assert!(
            matches!(
                token.as_str(),
                "Tier4"
                    | "Tier4EvidenceArtifact"
                    | "persist_tier4_evidence"
                    | "clear_unreferenced_tier4_evidence"
                    | "clear_unreferenced_tier4"
                    | "verify_tier4_evidence"
                    | "verify_tier4"
                    | "expected_tier4_source_policy"
            ),
            "{scope} introduces an unapproved Tier 4 operation: {token}"
        );
    }
    for window in tokens.windows(3) {
        if window[0] == "evidence" && window[1] == "::" {
            assert!(
                matches!(
                    window[2].as_str(),
                    "persist"
                        | "WaterfallEvidenceArtifact"
                        | "verify_tier2"
                        | "clear_obsolete_tier2_policy"
                        | "remove_obsolete_tier2_receipt"
                        | "verify_tier3"
                        | "clear_unreferenced_tier4"
                        | "verify_tier4"
                ),
                "{scope} delegates an unapproved local evidence operation: {}",
                window[2]
            );
        }
    }
    for required in [
        "persist_tier4_evidence",
        "clear_unreferenced_tier4_evidence",
        "verify_tier4_evidence",
    ] {
        assert_eq!(
            tokens
                .iter()
                .filter(|token| token.as_str() == required)
                .count(),
            1,
            "{scope} must expose exactly one {required} seam"
        );
    }
    assert_eq!(
        count_path(&tokens, &["evidence", "::", "clear_unreferenced_tier4"]),
        1,
        "{scope} must delegate exactly one Tier 4 clear"
    );
    assert_eq!(
        count_path(&tokens, &["evidence", "::", "verify_tier4"]),
        1,
        "{scope} must delegate exactly one Tier 4 verify"
    );
    assert_eq!(
        count_path(&tokens, &["evidence", "::", "persist"]),
        3,
        "{scope} must retain exactly the shared Tier 2/3/4 persistence calls"
    );
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

fn contains_code_token(code: &str, expected: &str) -> bool {
    code_tokens(code).iter().any(|token| token == expected)
}
