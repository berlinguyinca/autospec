//! Revision and worktree scoping for retrieval (spec sections 15, 46, 47).
//!
//! Pi runs several agents against the same repository in different worktrees at
//! once. Two rules follow, and both are enforced here rather than left to each
//! source adapter:
//!
//! 1. An agent working in a worktree must see the worktree's version of a file,
//!    not the base revision's (section 46).
//! 2. Uncommitted content from one worktree must not reach another (section
//!    47), which makes worktree-derived evidence non-shareable by default.

/// Which source state a retrieval reads, and which one a piece of evidence came
/// from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RetrievalScope {
    repository: String,
    revision: String,
    branch: Option<String>,
    worktree: Option<String>,
    modified_paths: Vec<String>,
}

impl RetrievalScope {
    /// Scope a retrieval to a committed revision with no worktree overlay.
    pub fn committed(repository: impl Into<String>, revision: impl Into<String>) -> Self {
        Self {
            repository: repository.into(),
            revision: revision.into(),
            branch: None,
            worktree: None,
            modified_paths: Vec::new(),
        }
    }

    /// Scope a retrieval to a Pi worktree layered over a base revision.
    ///
    /// `modified_paths` is the worktree's dirty set, which every Pi process
    /// passes in (section 46). Paths are normalized and deduplicated so
    /// `contains_modified` is order-independent.
    pub fn worktree(
        repository: impl Into<String>,
        base_revision: impl Into<String>,
        worktree: impl Into<String>,
        modified_paths: impl IntoIterator<Item = String>,
    ) -> Self {
        let mut paths = modified_paths
            .into_iter()
            .map(|path| normalize_path(&path))
            .filter(|path| !path.is_empty())
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        Self {
            repository: repository.into(),
            revision: base_revision.into(),
            branch: None,
            worktree: Some(worktree.into()),
            modified_paths: paths,
        }
    }

    /// Attach a branch name.
    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    /// Repository identifier.
    pub fn repository(&self) -> &str {
        &self.repository
    }

    /// Base revision the scope reads.
    pub fn revision(&self) -> &str {
        &self.revision
    }

    /// Branch name, when known.
    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    /// Worktree identifier, when the scope has a worktree overlay.
    pub fn worktree_id(&self) -> Option<&str> {
        self.worktree.as_deref()
    }

    /// Paths modified in the worktree, sorted and deduplicated.
    pub fn modified_paths(&self) -> &[String] {
        &self.modified_paths
    }

    /// Return `true` when `path` is dirty in this scope's worktree.
    pub fn contains_modified(&self, path: &str) -> bool {
        let path = normalize_path(path);
        self.modified_paths.contains(&path)
    }

    /// Which state a read of `path` must come from.
    pub fn resolve(&self, path: &str) -> PathState {
        match (&self.worktree, self.contains_modified(path)) {
            (Some(worktree), true) => PathState::Worktree(worktree.clone()),
            _ => PathState::Committed(self.revision.clone()),
        }
    }

    /// Cache-key fragment identifying the exact source state.
    ///
    /// A worktree fragment includes the dirty set, so an edit that adds or
    /// removes a modified file produces a different key: a cached answer from
    /// before the edit cannot be served after it (section 25).
    pub fn cache_fragment(&self) -> String {
        match &self.worktree {
            None => format!("{}@{}", self.repository, self.revision),
            Some(worktree) => format!(
                "{}@{}+wt:{}[{}]",
                self.repository,
                self.revision,
                worktree,
                self.modified_paths.join(",")
            ),
        }
    }

    /// Return `true` when evidence captured in this scope may be reused by
    /// `other`.
    ///
    /// Committed evidence is shareable across worktrees of the same revision —
    /// that is the reuse section 47 wants. Worktree evidence is shareable only
    /// with the identical worktree state, because it may embed uncommitted
    /// source that no other agent is allowed to see.
    pub fn may_share_with(&self, other: &Self) -> bool {
        if self.repository != other.repository {
            return false;
        }
        match (&self.worktree, &other.worktree) {
            (None, _) => self.revision == other.revision,
            (Some(_), _) => self.cache_fragment() == other.cache_fragment(),
        }
    }
}

/// The state a single path resolves to under a [`RetrievalScope`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathState {
    /// Read the committed content at this revision.
    Committed(String),
    /// Read the working copy in this worktree.
    Worktree(String),
}

impl PathState {
    /// Return `true` when the path resolves to uncommitted worktree content.
    pub fn is_worktree(&self) -> bool {
        matches!(self, Self::Worktree(_))
    }
}

fn normalize_path(path: &str) -> String {
    path.trim().trim_start_matches("./").replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worktree_scope() -> RetrievalScope {
        RetrievalScope::worktree(
            "autospec",
            "9a223af",
            "worktree-a",
            ["./src/router.rs".to_string(), "src/registry.rs".to_string()],
        )
    }

    #[test]
    fn modified_file_resolves_to_the_worktree_copy() {
        assert_eq!(
            worktree_scope().resolve("src/router.rs"),
            PathState::Worktree("worktree-a".to_string())
        );
    }

    #[test]
    fn untouched_file_resolves_to_the_base_revision() {
        assert_eq!(
            worktree_scope().resolve("src/scheduler.rs"),
            PathState::Committed("9a223af".to_string())
        );
    }

    #[test]
    fn committed_scope_never_resolves_to_a_worktree() {
        let scope = RetrievalScope::committed("autospec", "9a223af");
        assert!(!scope.resolve("src/router.rs").is_worktree());
    }

    #[test]
    fn worktree_evidence_is_not_shared_with_a_different_worktree() {
        let left = worktree_scope();
        let right = RetrievalScope::worktree(
            "autospec",
            "9a223af",
            "worktree-b",
            ["src/router.rs".to_string()],
        );

        assert!(!left.may_share_with(&right));
    }

    #[test]
    fn committed_evidence_is_shared_across_worktrees_of_the_same_revision() {
        let committed = RetrievalScope::committed("autospec", "9a223af");
        assert!(committed.may_share_with(&worktree_scope()));
    }

    #[test]
    fn committed_evidence_is_not_shared_across_revisions() {
        let left = RetrievalScope::committed("autospec", "9a223af");
        let right = RetrievalScope::committed("autospec", "def456");
        assert!(!left.may_share_with(&right));
    }

    #[test]
    fn dirty_set_change_changes_the_cache_fragment() {
        let before = worktree_scope();
        let after = RetrievalScope::worktree(
            "autospec",
            "9a223af",
            "worktree-a",
            ["src/router.rs".to_string()],
        );

        assert_ne!(before.cache_fragment(), after.cache_fragment());
    }

    #[test]
    fn modified_paths_are_order_independent() {
        let forward = RetrievalScope::worktree("r", "1", "w", ["b".into(), "a".into()]);
        let backward = RetrievalScope::worktree("r", "1", "w", ["a".into(), "b".into()]);
        assert_eq!(forward.cache_fragment(), backward.cache_fragment());
    }
}
