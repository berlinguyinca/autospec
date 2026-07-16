use std::fs;
use std::path::{Path, PathBuf};

fn pure_tier2_sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sources = vec![root.join("src/autonomous/tier2.rs")];
    for entry in
        fs::read_dir(root.join("src/autonomous/tier2")).expect("read Tier 2 module directory")
    {
        let path = entry.expect("read Tier 2 module entry").path();
        if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
    sources.sort();
    sources
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

    let mut saw_roi_rank = false;
    let gh_cli = ["\"", "g", "h "].concat();
    for source in sources {
        let contents = fs::read_to_string(&source).expect("read pure Tier 2 source");
        saw_roi_rank |= contents.contains("roi_rank");
        for forbidden in [
            "std::fs",
            "std::io",
            "std::path",
            "std::env",
            "std::process",
            "std::net",
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
            "sh -c",
            "omx",
            "curl",
            gh_cli.as_str(),
            "github",
            "queue",
            "claim",
            "label",
            "branch",
            "worktree",
            "pull_request",
            "pull request",
            "pr create",
            "issue create",
            "issue edit",
            "issue comment",
            "auto-implement",
            "\"POST\"",
            "\"PATCH\"",
            "\"PUT\"",
            "\"DELETE\"",
            "graphql",
            "pr edit",
            "pr comment",
            "pr merge",
        ] {
            assert!(
                !contents.contains(forbidden),
                "{} retains prohibited authority: {forbidden}",
                source.display()
            );
        }
    }
    assert!(saw_roi_rank, "guard the ROI/rank document path");
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
        "Legacy deletion remains blocked on broader native producer, foreground, and parity work.",
    ] {
        assert!(plan.contains(required), "cutover plan omits: {required}");
    }
}
