//! Declared facts of a portfolio plan and their canonicalization.

use super::{PlanViolation, PlanViolationCode, PrimaryScopeSelector};
use autospec_core::managed_project::{ItemKey, SourceSpecIdentity};
use std::collections::BTreeSet;

/// GitHub owner slug ceiling; longer values cannot be a real owner.
const OWNER_MAX_LEN: usize = 39;

/// What a read-only probe established about a repository. Tri-state on purpose: "we did
/// not look" and "we looked and it is not usable" are different failures and must not
/// collapse into one refusal, because only the second one tells the operator to act.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RepositoryCapability {
    /// Readable at the observed revision; plan items may be hosted by it.
    Available,
    /// Probed and refused (missing, archived, or unreadable for this identity).
    Unavailable,
    /// Not probed. Never treated as available.
    Unknown,
}

impl RepositoryCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Unavailable => "unavailable",
            Self::Unknown => "unknown",
        }
    }
}

/// One repository as read by the probe: identity, capability, and the exact revision the
/// capability was observed at. The revision participates in the plan digest, so a plan
/// records which commit it was reasoned about rather than "whatever the default was".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryFacts {
    repository: String,
    capability: RepositoryCapability,
    observed_revision: Option<String>,
}

impl RepositoryFacts {
    pub fn available(repository: impl Into<String>, revision: impl Into<String>) -> Self {
        Self {
            repository: repository.into(),
            capability: RepositoryCapability::Available,
            observed_revision: Some(revision.into()),
        }
    }

    /// Available, but with no revision recorded. A plan may freeze over it; the digest then
    /// carries no revision for that repository.
    pub fn reachable(repository: impl Into<String>) -> Self {
        Self {
            repository: repository.into(),
            capability: RepositoryCapability::Available,
            observed_revision: None,
        }
    }

    pub fn unavailable(repository: impl Into<String>) -> Self {
        Self {
            repository: repository.into(),
            capability: RepositoryCapability::Unavailable,
            observed_revision: None,
        }
    }

    pub fn unprobed(repository: impl Into<String>) -> Self {
        Self {
            repository: repository.into(),
            capability: RepositoryCapability::Unknown,
            observed_revision: None,
        }
    }

    pub fn repository(&self) -> &str {
        &self.repository
    }

    pub fn capability(&self) -> RepositoryCapability {
        self.capability
    }

    pub fn observed_revision(&self) -> Option<&str> {
        self.observed_revision.as_deref()
    }
}

/// Role an item plays in the portfolio. The string forms are the same tokens
/// `store::recovery` persists, so a frozen plan and a recovered snapshot agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanItemRole {
    SourceTracker,
    RepoTracker,
    Prerequisite,
    Implementation,
    Audit,
}

impl PlanItemRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SourceTracker => "source-tracker",
            Self::RepoTracker => "repo-tracker",
            Self::Prerequisite => "prerequisite",
            Self::Implementation => "implementation",
            Self::Audit => "audit",
        }
    }
}

/// How the completion of an item is decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanCompletionPolicy {
    /// Closes on its own acceptance criteria.
    SelfClosing,
    /// Closes when every child beneath it closes.
    Cascade,
    /// Closes only when the whole portfolio gate closes.
    PortfolioGate,
}

impl PlanCompletionPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SelfClosing => "self",
            Self::Cascade => "cascade",
            Self::PortfolioGate => "portfolio-gate",
        }
    }
}

/// One planned item, hosted by exactly one repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanItem {
    item_key: ItemKey,
    repository: String,
    role: PlanItemRole,
    completion_policy: PlanCompletionPolicy,
    depends_on: Vec<ItemKey>,
    local_parents: Vec<ItemKey>,
}

impl PlanItem {
    /// Build an item, rejecting a key the identity layer will not accept.
    pub fn new(
        item_key: &str,
        repository: impl Into<String>,
        role: PlanItemRole,
        completion_policy: PlanCompletionPolicy,
        depends_on: &[&str],
        local_parents: &[&str],
    ) -> Result<Self, PlanViolation> {
        Ok(Self {
            item_key: parse_key(item_key)?,
            repository: repository.into(),
            role,
            completion_policy,
            depends_on: parse_keys(depends_on)?,
            local_parents: parse_keys(local_parents)?,
        })
    }

    pub fn item_key(&self) -> &ItemKey {
        &self.item_key
    }

    pub fn repository(&self) -> &str {
        &self.repository
    }

    pub fn role(&self) -> PlanItemRole {
        self.role
    }

    pub fn completion_policy(&self) -> PlanCompletionPolicy {
        self.completion_policy
    }

    pub fn depends_on(&self) -> &[ItemKey] {
        &self.depends_on
    }

    pub fn local_parents(&self) -> &[ItemKey] {
        &self.local_parents
    }
}

/// Unfrozen input to [`super::PortfolioPlan::freeze`].
#[derive(Debug, Clone, Default)]
pub struct PlanDraft {
    source_spec: Option<SourceSpecIdentity>,
    project_owner: Option<String>,
    repositories: Vec<RepositoryFacts>,
    items: Vec<PlanItem>,
    primary_scope: Option<PrimaryScopeSelector>,
}

impl PlanDraft {
    pub fn new(
        source_spec: Option<SourceSpecIdentity>,
        project_owner: Option<&str>,
        repositories: Vec<RepositoryFacts>,
        items: Vec<PlanItem>,
    ) -> Self {
        Self {
            source_spec,
            project_owner: project_owner.map(str::to_string),
            repositories,
            items,
            primary_scope: None,
        }
    }

    /// Declare how the primary tracker is chosen instead of leaving it to derivation.
    pub fn with_primary_scope(mut self, selector: PrimaryScopeSelector) -> Self {
        self.primary_scope = Some(selector);
        self
    }

    pub fn source_spec(&self) -> Option<&SourceSpecIdentity> {
        self.source_spec.as_ref()
    }

    pub fn project_owner(&self) -> Option<&str> {
        self.project_owner.as_deref()
    }

    pub fn repositories(&self) -> &[RepositoryFacts] {
        &self.repositories
    }

    pub fn items(&self) -> &[PlanItem] {
        &self.items
    }

    pub fn primary_scope(&self) -> Option<&PrimaryScopeSelector> {
        self.primary_scope.as_ref()
    }
}

/// Parse one item key, reporting the identity layer's reason under our own code.
pub(super) fn parse_key(value: &str) -> Result<ItemKey, PlanViolation> {
    ItemKey::new(value).map_err(|error| {
        PlanViolation::new(
            PlanViolationCode::ItemKeyInvalid,
            format!("`{value}`: {error}"),
        )
    })
}

pub(super) fn parse_keys(values: &[&str]) -> Result<Vec<ItemKey>, PlanViolation> {
    values.iter().map(|value| parse_key(value)).collect()
}

/// GitHub owner slug: lowercase alphanumeric with interior hyphens, anchored, and short
/// enough to be one. Trimmed first so `" Acme "` and `"acme"` are the same owner.
pub(super) fn canonical_owner(owner: &str) -> Result<String, PlanViolation> {
    let trimmed = owner.trim();
    if trimmed.is_empty() {
        return Err(PlanViolation::new(
            PlanViolationCode::OwnerMissing,
            "project owner is empty",
        ));
    }
    let lowered = trimmed.to_ascii_lowercase();
    let invalid = lowered.len() > OWNER_MAX_LEN
        || !lowered.starts_with(|c: char| c.is_ascii_alphanumeric())
        || !lowered.ends_with(|c: char| c.is_ascii_alphanumeric())
        || !lowered
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    if invalid {
        return Err(PlanViolation::new(
            PlanViolationCode::OwnerInvalid,
            format!("`{owner}` is not a GitHub owner slug"),
        ));
    }
    Ok(lowered)
}

/// Repository identity as exactly two segments, `owner/name`, both non-empty and not a
/// dot segment. Compared case-insensitively because GitHub is, so `Acme/Tool` and
/// `acme/tool` are the same repository and may not appear as two entries.
pub(super) fn canonical_repository_id(repository: &str) -> Result<String, PlanViolation> {
    let trimmed = repository.trim();
    let invalid_shape = match trimmed.split_once('/') {
        Some((owner, name)) => !segment_ok(owner) || !segment_ok(name) || name.contains('/'),
        None => true,
    };
    if invalid_shape {
        return Err(PlanViolation::new(
            PlanViolationCode::RepositoryInvalid,
            format!("`{repository}` is not an `owner/name` repository id"),
        ));
    }
    Ok(trimmed.to_ascii_lowercase())
}

/// One repository id segment: non-empty, not a dot segment, and confined to the
/// characters GitHub itself accepts in a repository name.
fn segment_ok(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

/// Canonical repository list: unique after case folding, sorted by id.
pub(super) fn canonical_repositories(
    repositories: &[RepositoryFacts],
) -> Result<Vec<RepositoryFacts>, PlanViolation> {
    if repositories.is_empty() {
        return Err(PlanViolation::new(
            PlanViolationCode::PortfolioSetEmpty,
            "a portfolio plan must declare at least one repository",
        ));
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut canonical: Vec<RepositoryFacts> = Vec::with_capacity(repositories.len());
    for facts in repositories {
        let id = canonical_repository_id(facts.repository())?;
        if !seen.insert(id.clone()) {
            return Err(PlanViolation::new(
                PlanViolationCode::RepositoryDuplicate,
                format!("repository `{id}` is declared more than once"),
            ));
        }
        canonical.push(RepositoryFacts {
            repository: id,
            capability: facts.capability(),
            observed_revision: facts
                .observed_revision()
                .map(|revision| revision.trim().to_string()),
        });
    }
    canonical.sort_by(|left, right| left.repository.cmp(&right.repository));
    Ok(canonical)
}

/// Canonical item list: sorted by key, keys unique, every item hosted by a declared
/// repository.
pub(super) fn canonical_items(
    items: &[PlanItem],
    repositories: &[RepositoryFacts],
) -> Result<Vec<PlanItem>, PlanViolation> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut canonical: Vec<PlanItem> = Vec::with_capacity(items.len());
    for item in items {
        let key = item.item_key().to_string();
        if !seen.insert(key.clone()) {
            return Err(PlanViolation::new(
                PlanViolationCode::ItemKeyDuplicate,
                format!("item key `{key}` is declared more than once"),
            ));
        }
        let repository = canonical_repository_id(item.repository()).map_err(|_| {
            PlanViolation::new(
                PlanViolationCode::ItemRepositoryUndeclared,
                format!("item `{key}` names repository `{}`", item.repository()),
            )
        })?;
        if !repositories
            .iter()
            .any(|declared| declared.repository() == repository)
        {
            return Err(PlanViolation::new(
                PlanViolationCode::ItemRepositoryUndeclared,
                format!("item `{key}` names repository `{repository}` which is not in the plan"),
            ));
        }
        canonical.push(PlanItem {
            item_key: item.item_key().clone(),
            repository,
            role: item.role(),
            completion_policy: item.completion_policy(),
            depends_on: sorted_keys(item.depends_on()),
            local_parents: sorted_keys(item.local_parents()),
        });
    }
    canonical.sort_by(|left, right| left.item_key.as_str().cmp(right.item_key.as_str()));
    Ok(canonical)
}

/// Edge lists sorted by key text so rendering, and therefore the digest, ignores
/// declaration order. Duplicates are kept: repeating an edge is a graph error, not a
/// canonicalization detail, and silently dropping it would freeze a plan the operator
/// did not write.
fn sorted_keys(keys: &[ItemKey]) -> Vec<ItemKey> {
    let mut keys: Vec<ItemKey> = keys.to_vec();
    keys.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    keys
}
