//! Revision-aware caching and worktree isolation
//! (spec sections 25, 46, 47, 55.5, 55.6).

mod rag_support;

use autospec_core::rag::authority::SourceAuthority;
use autospec_core::rag::cache::{CacheClass, CacheEntry, CacheKey, RetrievalCache};
use autospec_core::rag::coordinator::{RetrievalCoordinator, RetrievalRequest};
use autospec_core::rag::policy::AgentRole;
use autospec_core::rag::scope::{PathState, RetrievalScope};
use autospec_core::rag::source::{
    KnowledgeSource, SearchMode, SearchRequest, SourceKind, SourceRegistry,
};
use rag_support::{CountingSource, NOW, REVISION, evidence, evidence_in, scope};

fn key_for(scope: &RetrievalScope) -> CacheKey {
    let request = SearchRequest::new("scheduler", SearchMode::Semantic, scope.clone(), 10);
    CacheKey::for_request(CacheClass::Query, SourceKind::Repository, &request)
}

fn entry_at(scope: &RetrievalScope, id: &str) -> CacheEntry {
    CacheEntry {
        evidence: vec![evidence_in(
            id,
            SourceKind::Repository,
            "src/scheduler.rs",
            "fn select_node()",
            SourceAuthority::Implementation,
            900,
            scope.clone(),
        )],
        scope: scope.clone(),
        stored_at: NOW,
    }
}

#[test]
fn a_cached_answer_from_one_commit_is_not_served_for_another() {
    let stored_scope = RetrievalScope::committed("autospec", "abc123");
    let mut cache = RetrievalCache::new();
    cache.store(&key_for(&stored_scope), entry_at(&stored_scope, "ev_1"));

    let later = RetrievalScope::committed("autospec", "def456");

    assert!(
        cache.get(&key_for(&stored_scope), &later).is_none(),
        "commit abc123 evidence must not answer for def456"
    );
    assert_eq!(cache.stats().misses, 1);
}

#[test]
fn a_cached_answer_is_served_at_the_same_revision() {
    let stored_scope = scope();
    let mut cache = RetrievalCache::new();
    cache.store(&key_for(&stored_scope), entry_at(&stored_scope, "ev_1"));

    let hit = cache.get(&key_for(&stored_scope), &stored_scope);

    assert!(hit.is_some());
    assert_eq!(cache.stats().hits, 1);
    assert_eq!(cache.stats().items_saved, 1);
}

#[test]
fn a_commit_invalidates_every_entry_at_the_old_revision() {
    let stored_scope = scope();
    let mut cache = RetrievalCache::new();
    cache.store(&key_for(&stored_scope), entry_at(&stored_scope, "ev_1"));

    let dropped = cache.invalidate_revision("autospec", REVISION);

    assert_eq!(dropped, 1);
    assert!(cache.is_empty());
    assert_eq!(cache.stats().invalidations, 1);
}

#[test]
fn worktree_evidence_is_never_written_to_the_shared_cache() {
    let worktree = RetrievalScope::worktree(
        "autospec",
        REVISION,
        "worktree-a",
        ["src/scheduler.rs".to_string()],
    );
    let mut cache = RetrievalCache::new();

    let stored = cache.store(&key_for(&worktree), entry_at(&worktree, "ev_dirty"));

    assert!(!stored, "uncommitted content must not enter a shared cache");
    assert!(cache.is_empty());
}

#[test]
fn one_worktrees_modifications_do_not_reach_another() {
    let alpha = RetrievalScope::worktree(
        "autospec",
        REVISION,
        "worktree-a",
        ["src/scheduler.rs".to_string()],
    );
    let beta = RetrievalScope::worktree(
        "autospec",
        REVISION,
        "worktree-b",
        ["src/scheduler.rs".to_string()],
    );

    assert!(!alpha.may_share_with(&beta));
    assert!(!beta.may_share_with(&alpha));
}

#[test]
fn a_worktree_reads_its_own_copy_of_a_modified_file() {
    let worktree = RetrievalScope::worktree(
        "autospec",
        REVISION,
        "worktree-a",
        ["src/scheduler.rs".to_string()],
    );

    assert_eq!(
        worktree.resolve("src/scheduler.rs"),
        PathState::Worktree("worktree-a".to_string())
    );
    assert_eq!(
        worktree.resolve("src/router.rs"),
        PathState::Committed(REVISION.to_string()),
        "an untouched file still comes from the base revision"
    );
}

#[test]
fn committed_evidence_is_reused_across_parallel_agents() {
    // Section 47: agent A's discovery about committed source is safe for agent
    // B, provided the revision matches.
    let committed = scope();
    let worktree = RetrievalScope::worktree(
        "autospec",
        REVISION,
        "worktree-b",
        ["src/other.rs".to_string()],
    );
    let mut cache = RetrievalCache::new();
    cache.store(&key_for(&committed), entry_at(&committed, "ev_shared"));

    let hit = cache.get(&key_for(&committed), &worktree);

    assert!(hit.is_some(), "committed evidence is shareable");
}

#[test]
fn an_edit_to_the_dirty_set_changes_the_cache_key() {
    let before = RetrievalScope::worktree("autospec", REVISION, "wt", ["a.rs".to_string()]);
    let after = RetrievalScope::worktree(
        "autospec",
        REVISION,
        "wt",
        ["a.rs".to_string(), "b.rs".to_string()],
    );

    assert_ne!(
        key_for(&before).as_string(),
        key_for(&after).as_string(),
        "adding a dirty file must invalidate the worktree's cached answers"
    );
}

#[test]
fn the_coordinator_serves_a_second_identical_query_from_the_cache() {
    let counting = CountingSource::new(
        SourceKind::Specification,
        vec![evidence(
            "ev_1",
            SourceKind::Specification,
            "docs/specs/routing.md",
            "the scheduler picks a node",
            SourceAuthority::AcceptedSpecification,
            900,
        )],
    );
    let mut registry = SourceRegistry::new();
    let counting: Box<dyn KnowledgeSource> = Box::new(counting);
    registry.register(counting).expect("registers");
    let mut cache = RetrievalCache::new();
    let request = RetrievalRequest::new(
        "AS-1",
        AgentRole::Planner,
        "how does the scheduler work?",
        scope(),
    )
    .requiring(["scheduler".to_string()]);

    {
        let mut coordinator =
            RetrievalCoordinator::new(&registry, AgentRole::Planner, NOW, REVISION)
                .with_cache(&mut cache);
        coordinator.retrieve("rag_1", &request).expect("first run");
    }
    let after_first = cache.stats().hits;
    {
        let mut coordinator =
            RetrievalCoordinator::new(&registry, AgentRole::Planner, NOW, REVISION)
                .with_cache(&mut cache);
        coordinator.retrieve("rag_2", &request).expect("second run");
    }

    assert_eq!(after_first, 0, "the first run cannot hit an empty cache");
    assert!(
        cache.stats().hits > 0,
        "the second identical retrieval is served from the cache"
    );
}

#[test]
fn cache_hit_ratio_is_reported_without_floating_point() {
    let stored_scope = scope();
    let mut cache = RetrievalCache::new();
    cache.store(&key_for(&stored_scope), entry_at(&stored_scope, "ev_1"));
    cache.get(&key_for(&stored_scope), &stored_scope);
    cache.get(
        &key_for(&stored_scope),
        &RetrievalScope::committed("autospec", "other"),
    );

    assert_eq!(cache.stats().hit_ratio_permille(), 500);
}

#[test]
fn evidence_from_a_superseded_revision_is_dropped_and_the_trace_says_why() {
    // The corpus was captured at REVISION; the coordinator is told the tree has
    // moved on. Repository evidence is stale on revision change (section 31).
    let mut registry = SourceRegistry::new();
    let source: Box<dyn KnowledgeSource> = Box::new(rag_support::StaticSource::semantic(
        SourceKind::Repository,
        vec![evidence(
            "ev_old",
            SourceKind::Repository,
            "src/scheduler.rs",
            "the scheduler ranks nodes",
            SourceAuthority::Implementation,
            950,
        )],
    ));
    registry.register(source).expect("registers");
    let mut coordinator =
        RetrievalCoordinator::new(&registry, AgentRole::Planner, NOW, "def456");
    let request = RetrievalRequest::new(
        "AS-1",
        AgentRole::Planner,
        "how does the scheduler work?",
        scope(),
    )
    .requiring(["scheduler".to_string()]);

    let outcome = coordinator.retrieve("rag_stale", &request).expect("loop runs");

    // Once per query that surfaced it: the loop reformulates and the source
    // returns the same superseded item, which is dropped again each time.
    assert!(outcome.trace.count_events("evidence_stale") >= 1);
    assert!(
        !outcome.stop_reason.is_satisfied(),
        "stale evidence cannot satisfy the request"
    );
    assert!(outcome.trace.render().contains("current revision is def456"));
}
