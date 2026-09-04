//! AAR spec sections 6, 7 and 11: context minimization, durable worktree
//! memory, and cache-friendly prompt assembly.

use autospec_core::aar::classify::{classify, ClassificationInput, Complexity, TaskClass};
use autospec_core::aar::context::{
    check_context_fit, context_policy_for, CacheFriendlyPrompt, ContextSegment, PromptBlock,
    RetrievalStrategy, CACHE_BOUNDARY,
};
use autospec_core::aar::memory::{
    required_directories, scaffold, MemoryEntry, MemoryFile, WorktreeMemory, MEMORY_DIR,
    TELEMETRY_DIR,
};

fn blocks() -> Vec<PromptBlock> {
    vec![
        PromptBlock::new(
            ContextSegment::LatestResult,
            "cargo test failed on one case",
        ),
        PromptBlock::new(ContextSegment::RetrievedCode, "fn parse() {}"),
        PromptBlock::new(
            ContextSegment::HarnessInstructions,
            "you are a coding agent",
        ),
        PromptBlock::new(ContextSegment::Task, "fix the parser panic"),
        PromptBlock::new(ContextSegment::Tools, "read, edit, run"),
        PromptBlock::new(
            ContextSegment::ModelRules,
            "make one logical change at a time",
        ),
        PromptBlock::new(ContextSegment::RepositoryInstructions, "see AGENTS.md"),
        PromptBlock::new(ContextSegment::Role, "implementer"),
        PromptBlock::new(ContextSegment::State, "step 3 of 5"),
    ]
}

#[test]
fn assembly_orders_stable_segments_before_volatile_ones() {
    let prompt = CacheFriendlyPrompt::assemble(blocks()).expect("blocks assemble");

    let rendered = prompt.render();
    let boundary = rendered.find(CACHE_BOUNDARY).expect("boundary present");
    for stable in [
        "harness_instructions",
        "tools",
        "model_rules",
        "repository_instructions",
    ] {
        assert!(
            rendered.find(stable).expect("segment present") < boundary,
            "{stable} must precede the cache boundary"
        );
    }
    for volatile in ["role", "task", "state", "retrieved_code", "latest_result"] {
        assert!(
            rendered.find(volatile).expect("segment present") > boundary,
            "{volatile} must follow the cache boundary"
        );
    }
}

#[test]
fn a_duplicate_segment_is_rejected() {
    let mut duplicated = blocks();
    duplicated.push(PromptBlock::new(ContextSegment::Task, "and also this"));

    let error = CacheFriendlyPrompt::assemble(duplicated).unwrap_err();

    assert!(error.contains("duplicate prompt segment"));
}

/// The point of the boundary: changing the task must not change the prefix
/// hash, or every step re-prefills the whole prompt.
#[test]
fn the_prefix_hash_survives_a_changed_task_but_not_a_changed_rule() {
    let original = CacheFriendlyPrompt::assemble(blocks()).expect("blocks assemble");

    let mut changed_task = blocks();
    changed_task.retain(|block| block.segment != ContextSegment::Task);
    changed_task.push(PromptBlock::new(
        ContextSegment::Task,
        "an entirely different task",
    ));
    let after_task = CacheFriendlyPrompt::assemble(changed_task).expect("blocks assemble");

    let mut changed_rule = blocks();
    changed_rule.retain(|block| block.segment != ContextSegment::ModelRules);
    changed_rule.push(PromptBlock::new(
        ContextSegment::ModelRules,
        "different rules",
    ));
    let after_rule = CacheFriendlyPrompt::assemble(changed_rule).expect("blocks assemble");

    assert_eq!(
        original.stable_prefix_hash(),
        after_task.stable_prefix_hash()
    );
    assert_ne!(
        original.stable_prefix_hash(),
        after_rule.stable_prefix_hash()
    );
}

#[test]
fn the_prefix_hash_is_a_hex_sha256() {
    let prompt = CacheFriendlyPrompt::assemble(blocks()).expect("blocks assemble");

    let hash = prompt.stable_prefix_hash();

    assert_eq!(hash.len(), 64);
    assert!(hash.chars().all(|character| character.is_ascii_hexdigit()));
}

#[test]
fn full_history_is_never_included_by_default() {
    for class in TaskClass::all() {
        let mut classification = classify(&ClassificationInput::new("x", "y"));
        classification.task_class = class;
        assert!(
            !context_policy_for(&classification).include_full_history,
            "{} must not inject full history",
            class.as_str()
        );
    }
}

#[test]
fn retrieval_starts_with_the_narrowest_strategy() {
    let classification = classify(
        &ClassificationInput::new("Fix the crash in the parser", "It panics on empty input.")
            .with_paths(["src/parser.rs"]),
    );

    let policy = context_policy_for(&classification);

    assert_eq!(policy.ladder[0].strategy, RetrievalStrategy::PathSearch);
    assert!(policy.ladder[0].max_files <= policy.max_retrieved_files);
}

#[test]
fn the_retrieval_budget_grows_with_complexity() {
    let mut trivial = classify(&ClassificationInput::new("x", "y"));
    trivial.complexity = Complexity::Trivial;
    let mut exceptional = classify(&ClassificationInput::new("x", "y"));
    exceptional.complexity = Complexity::Exceptional;

    assert!(
        context_policy_for(&trivial).max_retrieved_files
            < context_policy_for(&exceptional).max_retrieved_files
    );
}

#[test]
fn expansion_stops_once_a_round_found_evidence() {
    let classification = classify(
        &ClassificationInput::new("Implement the exporter", "See plan.").with_estimated_files(6),
    );
    let policy = context_policy_for(&classification);

    assert!(policy.next_step(0, false).is_some());
    assert!(
        policy.next_step(1, true).is_none(),
        "a round that found evidence must not widen further"
    );
}

#[test]
fn expansion_stops_at_the_round_ceiling() {
    let classification =
        classify(&ClassificationInput::new("Fix typo", "One line.").with_paths(["docs/a.md"]));
    let policy = context_policy_for(&classification);

    assert!(policy
        .next_step(policy.max_expansion_rounds, false)
        .is_none());
}

#[test]
fn a_prompt_that_does_not_fit_reports_why() {
    let prompt = CacheFriendlyPrompt::assemble(blocks()).expect("blocks assemble");

    let fit = check_context_fit(&prompt, 10, 0);

    assert!(!fit.fits);
    assert!(fit
        .reasons
        .iter()
        .any(|reason| reason.contains("only 10 are free")));
}

#[test]
fn projected_growth_counts_against_the_free_window() {
    let prompt = CacheFriendlyPrompt::assemble(blocks()).expect("blocks assemble");
    let estimated = prompt.estimated_tokens();

    assert!(check_context_fit(&prompt, estimated, 0).fits);
    assert!(!check_context_fit(&prompt, estimated, 1).fits);
}

#[test]
fn worktree_memory_scaffolds_every_spec_file() {
    let files = scaffold();

    let paths: Vec<&str> = files.iter().map(|(path, _)| path.as_str()).collect();
    for expected in [
        ".autospec/task.md",
        ".autospec/plan.md",
        ".autospec/state.md",
        ".autospec/findings.md",
        ".autospec/decisions.md",
        ".autospec/tests.md",
        ".autospec/review.md",
    ] {
        assert!(paths.contains(&expected), "missing {expected}");
    }
    assert_eq!(required_directories(), vec![MEMORY_DIR, TELEMETRY_DIR]);
}

#[test]
fn memory_entries_round_trip_through_the_rendered_file() {
    let mut memory = WorktreeMemory::new();
    memory.append(
        MemoryFile::Findings,
        MemoryEntry::with_evidence(
            "queue_parser panics on an empty document",
            "crates/autospec-core/src/execution/queue_parser.rs:41",
        ),
    );
    memory.append(
        MemoryFile::Findings,
        MemoryEntry::new("the fixture reproduces it"),
    );

    let rendered = memory.render(MemoryFile::Findings);
    let mut reloaded = WorktreeMemory::new();
    reloaded.load(MemoryFile::Findings, &rendered);

    assert_eq!(
        reloaded.entries(MemoryFile::Findings),
        memory.entries(MemoryFile::Findings)
    );
}

#[test]
fn an_oversized_memory_file_is_reported_as_over_budget() {
    let mut memory = WorktreeMemory::new();
    memory.max_lines = 10;
    for index in 0..40 {
        memory.append(MemoryFile::State, MemoryEntry::new(format!("step {index}")));
    }

    assert_eq!(memory.over_budget(), vec![MemoryFile::State]);
}

#[test]
fn every_memory_file_parses_back_from_its_name() {
    for file in MemoryFile::all() {
        assert_eq!(MemoryFile::parse(file.file_name()), Some(file));
        assert!(file.relative_path().starts_with(MEMORY_DIR));
        assert!(!file.purpose().is_empty());
    }
}
