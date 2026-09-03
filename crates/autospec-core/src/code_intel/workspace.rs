use std::collections::BTreeMap;

use serde::Serialize;

use super::error::CodeIntelError;

/// Lifecycle states a semantic workspace moves through.
///
/// `Indexing` is answerable only for operations that tolerate a partial index;
/// `Invalidated` and `Expired` must be re-provisioned before any query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceState {
    Provisioning,
    Indexing,
    Ready,
    Degraded,
    Expired,
    Invalidated,
}

impl WorkspaceState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Provisioning => "provisioning",
            Self::Indexing => "indexing",
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Expired => "expired",
            Self::Invalidated => "invalidated",
        }
    }

    /// Whether a semantic query may be dispatched at all.
    pub fn accepts_queries(self) -> bool {
        matches!(self, Self::Ready | Self::Degraded | Self::Indexing)
    }

    /// Whether results from this state carry full semantic confidence.
    pub fn is_semantically_complete(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// One worktree's semantic workspace. A workspace resolves to exactly one root,
/// so diagnostics and in-memory document state can never cross worktrees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SemanticWorkspace {
    pub id: String,
    pub repository: String,
    pub root: String,
    pub revision: String,
    pub state: WorkspaceState,
    pub languages: Vec<String>,
    /// Minutes since this workspace last served a query.
    pub idle_minutes: u64,
}

impl SemanticWorkspace {
    pub fn new(
        id: impl Into<String>,
        repository: impl Into<String>,
        root: impl Into<String>,
        revision: impl Into<String>,
    ) -> Result<Self, CodeIntelError> {
        let id = id.into();
        validate_id(&id)?;
        Ok(Self {
            id,
            repository: repository.into(),
            root: normalize_root(root.into()),
            revision: revision.into(),
            state: WorkspaceState::Provisioning,
            languages: Vec::new(),
            idle_minutes: 0,
        })
    }

    pub fn with_languages(mut self, languages: Vec<String>) -> Self {
        self.languages = languages;
        self
    }

    pub fn with_state(mut self, state: WorkspaceState) -> Self {
        self.state = state;
        self
    }

    /// Cache key for a query result. It binds workspace, revision and the
    /// content identity of the request, so a result can never be reused across
    /// worktrees or across revisions of the same worktree.
    pub fn cache_key(&self, operation: &str, request_identity: &str) -> String {
        format!(
            "{}:{}:{}:{}",
            self.id, self.revision, operation, request_identity
        )
    }

    /// Whether `path` resolves inside this workspace root. Rejects absolute
    /// paths and any traversal that would escape the worktree.
    pub fn contains_path(&self, path: &str) -> bool {
        if path.starts_with('/') || path.contains('\0') {
            return false;
        }
        let mut depth = 0i32;
        for segment in path.split('/') {
            match segment {
                "" | "." => continue,
                ".." => depth -= 1,
                _ => depth += 1,
            }
            if depth < 0 {
                return false;
            }
        }
        depth >= 0
    }

    pub fn is_expired(&self, idle_ttl_minutes: u64) -> bool {
        self.idle_minutes >= idle_ttl_minutes
    }
}

/// The registry of live semantic workspaces.
///
/// It is the single place that maps a workspace ID to a worktree root, which is
/// what makes the isolation invariant enforceable: a query that names a
/// workspace can only ever reach that workspace's root.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WorkspaceRegistry {
    workspaces: BTreeMap<String, SemanticWorkspace>,
}

impl WorkspaceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a workspace. Two workspaces may not share a root: that would
    /// let one worktree's diagnostics surface under another's ID.
    pub fn register(&mut self, workspace: SemanticWorkspace) -> Result<(), CodeIntelError> {
        if self.workspaces.contains_key(&workspace.id) {
            return Err(CodeIntelError::workspace(format!(
                "workspace already registered: {}",
                workspace.id
            )));
        }
        if let Some(existing) = self
            .workspaces
            .values()
            .find(|candidate| candidate.root == workspace.root)
        {
            return Err(CodeIntelError::workspace(format!(
                "root {} is already bound to workspace {}",
                workspace.root, existing.id
            )));
        }
        self.workspaces.insert(workspace.id.clone(), workspace);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<&SemanticWorkspace, CodeIntelError> {
        self.workspaces
            .get(id)
            .ok_or_else(|| CodeIntelError::workspace(format!("unknown workspace: {id}")))
    }

    /// Resolve a workspace for a query, rejecting states that cannot serve one.
    pub fn resolve(&self, id: &str) -> Result<&SemanticWorkspace, CodeIntelError> {
        let workspace = self.get(id)?;
        if !workspace.state.accepts_queries() {
            return Err(CodeIntelError::workspace(format!(
                "workspace {id} is {} and must be re-provisioned",
                workspace.state.as_str()
            )));
        }
        Ok(workspace)
    }

    pub fn set_state(&mut self, id: &str, state: WorkspaceState) -> Result<(), CodeIntelError> {
        let workspace = self
            .workspaces
            .get_mut(id)
            .ok_or_else(|| CodeIntelError::workspace(format!("unknown workspace: {id}")))?;
        workspace.state = state;
        Ok(())
    }

    /// Mark a workspace's semantic state stale — after a revision change, a
    /// server crash, or an external edit the servers did not observe.
    pub fn invalidate(&mut self, id: &str) -> Result<(), CodeIntelError> {
        self.set_state(id, WorkspaceState::Invalidated)
    }

    /// Drop a workspace entirely. Called when its worktree is removed, so no
    /// semantic state outlives the tree it described.
    pub fn remove(&mut self, id: &str) -> Result<SemanticWorkspace, CodeIntelError> {
        self.workspaces
            .remove(id)
            .ok_or_else(|| CodeIntelError::workspace(format!("unknown workspace: {id}")))
    }

    pub fn record_idle(&mut self, id: &str, idle_minutes: u64) -> Result<(), CodeIntelError> {
        let workspace = self
            .workspaces
            .get_mut(id)
            .ok_or_else(|| CodeIntelError::workspace(format!("unknown workspace: {id}")))?;
        workspace.idle_minutes = idle_minutes;
        Ok(())
    }

    /// Expire every workspace idle beyond the TTL, returning the expired IDs.
    pub fn expire_idle(&mut self, idle_ttl_minutes: u64) -> Vec<String> {
        let mut expired = Vec::new();
        for workspace in self.workspaces.values_mut() {
            if workspace.state != WorkspaceState::Expired && workspace.is_expired(idle_ttl_minutes)
            {
                workspace.state = WorkspaceState::Expired;
                expired.push(workspace.id.clone());
            }
        }
        expired
    }

    pub fn active(&self) -> Vec<&SemanticWorkspace> {
        self.workspaces
            .values()
            .filter(|workspace| workspace.state.accepts_queries())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.workspaces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.workspaces.is_empty()
    }
}

/// Derive a workspace ID from a worktree path.
///
/// `main` maps to `repo-main`; every `.autospec/worktrees/<slug>` maps to its
/// slug, so the ID an agent quotes is the worktree an operator can find.
pub fn workspace_id_for_worktree(repository: &str, worktree_path: &str) -> String {
    let normalized = normalize_root(worktree_path.to_string());
    match normalized.rsplit_once(".autospec/worktrees/") {
        Some((_, slug)) if !slug.is_empty() => slug.to_string(),
        _ => format!("{repository}-{}", leaf_of(&normalized)),
    }
}

fn leaf_of(path: &str) -> &str {
    path.rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or(path)
}

fn normalize_root(root: String) -> String {
    let trimmed = root.trim_end_matches('/');
    if trimmed.is_empty() {
        root
    } else {
        trimmed.to_string()
    }
}

fn validate_id(id: &str) -> Result<(), CodeIntelError> {
    if id.is_empty() {
        return Err(CodeIntelError::workspace("workspace id must not be empty"));
    }
    let valid = id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '_');
    if !valid {
        return Err(CodeIntelError::workspace(format!(
            "workspace id must be alphanumeric, '-' or '_': {id}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(id: &str, root: &str) -> SemanticWorkspace {
        SemanticWorkspace::new(id, "autospec", root, "abc123")
            .unwrap()
            .with_state(WorkspaceState::Ready)
    }

    #[test]
    fn worktree_paths_map_to_stable_workspace_ids() {
        assert_eq!(workspace_id_for_worktree("repo", "main"), "repo-main");
        assert_eq!(
            workspace_id_for_worktree("repo", ".autospec/worktrees/issue-421"),
            "issue-421"
        );
        assert_eq!(
            workspace_id_for_worktree("repo", "/srv/.autospec/worktrees/issue-422/"),
            "issue-422"
        );
    }

    #[test]
    fn workspace_ids_reject_path_characters() {
        assert!(SemanticWorkspace::new("../escape", "autospec", "main", "abc").is_err());
        assert!(SemanticWorkspace::new("", "autospec", "main", "abc").is_err());
        assert!(SemanticWorkspace::new("issue-421", "autospec", "main", "abc").is_ok());
    }

    #[test]
    fn two_workspaces_cannot_share_a_root() {
        let mut registry = WorkspaceRegistry::new();
        registry.register(workspace("issue-421", "wt/a")).unwrap();

        let error = registry
            .register(workspace("issue-422", "wt/a"))
            .unwrap_err();

        assert!(error.message().contains("already bound to workspace"));
    }

    #[test]
    fn duplicate_workspace_ids_are_rejected() {
        let mut registry = WorkspaceRegistry::new();
        registry.register(workspace("issue-421", "wt/a")).unwrap();

        assert!(registry.register(workspace("issue-421", "wt/b")).is_err());
    }

    #[test]
    fn cache_keys_never_collide_across_workspaces_or_revisions() {
        let first = workspace("issue-421", "wt/a");
        let second = workspace("issue-422", "wt/b");
        let mut moved = first.clone();
        moved.revision = "def456".to_string();

        let key = first.cache_key("code.references", "Gateway::resolve");

        assert_ne!(key, second.cache_key("code.references", "Gateway::resolve"));
        assert_ne!(key, moved.cache_key("code.references", "Gateway::resolve"));
        assert_ne!(key, first.cache_key("code.callers", "Gateway::resolve"));
    }

    #[test]
    fn paths_escaping_the_worktree_are_rejected() {
        let workspace = workspace("issue-421", "wt/a");

        assert!(workspace.contains_path("src/gateway.rs"));
        assert!(workspace.contains_path("./src/gateway.rs"));
        assert!(workspace.contains_path("src/../src/gateway.rs"));
        assert!(!workspace.contains_path("/etc/passwd"));
        assert!(!workspace.contains_path("../issue-422/src/gateway.rs"));
        assert!(!workspace.contains_path("src/../../secret"));
    }

    #[test]
    fn invalidated_workspaces_refuse_queries() {
        let mut registry = WorkspaceRegistry::new();
        registry.register(workspace("issue-421", "wt/a")).unwrap();
        registry.invalidate("issue-421").unwrap();

        let error = registry.resolve("issue-421").unwrap_err();

        assert!(error.message().contains("invalidated"));
    }

    #[test]
    fn indexing_workspaces_still_accept_queries_but_are_not_complete() {
        assert!(WorkspaceState::Indexing.accepts_queries());
        assert!(!WorkspaceState::Indexing.is_semantically_complete());
        assert!(WorkspaceState::Ready.is_semantically_complete());
    }

    #[test]
    fn idle_workspaces_expire_at_the_ttl() {
        let mut registry = WorkspaceRegistry::new();
        registry.register(workspace("issue-421", "wt/a")).unwrap();
        registry.register(workspace("issue-422", "wt/b")).unwrap();
        registry.record_idle("issue-421", 30).unwrap();
        registry.record_idle("issue-422", 29).unwrap();

        let expired = registry.expire_idle(30);

        assert_eq!(expired, vec!["issue-421".to_string()]);
        assert_eq!(registry.active().len(), 1);
    }

    #[test]
    fn expiry_is_reported_once_per_workspace() {
        let mut registry = WorkspaceRegistry::new();
        registry.register(workspace("issue-421", "wt/a")).unwrap();
        registry.record_idle("issue-421", 45).unwrap();

        assert_eq!(registry.expire_idle(30).len(), 1);
        assert!(registry.expire_idle(30).is_empty());
    }

    #[test]
    fn removing_a_worktree_drops_its_semantic_state() {
        let mut registry = WorkspaceRegistry::new();
        registry.register(workspace("issue-421", "wt/a")).unwrap();

        registry.remove("issue-421").unwrap();

        assert!(registry.is_empty());
        assert!(registry.get("issue-421").is_err());
    }

    #[test]
    fn unknown_workspaces_are_rejected_rather_than_defaulted() {
        let registry = WorkspaceRegistry::new();

        let error = registry.resolve("issue-999").unwrap_err();

        assert!(error.message().contains("unknown workspace"));
    }
}
