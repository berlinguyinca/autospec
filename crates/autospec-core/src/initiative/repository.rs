//! Repository identity, capabilities, and the workspace manifest.
//!
//! Repository ownership is data, never a global assumption (architectural
//! invariant 7): an Initiative may span arbitrary hosts, organizations, and
//! personal owners, each with its own credential and default branch.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

/// A globally qualified repository identity, `host/owner/repository`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RepositoryId {
    text: String,
    host_end: usize,
    owner_end: usize,
}

impl RepositoryId {
    /// Parse `host/owner/repository`, rejecting any shorter form.
    ///
    /// A bare `owner/repository` is rejected on purpose: it would silently
    /// assume a host, and Initiatives are allowed to span hosts.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, String> {
        let raw = value.as_ref().trim_end_matches('/');
        let segments = raw.split('/').collect::<Vec<_>>();
        if segments.len() != 3 {
            return Err(format!(
                "a repository id must be host/owner/repository: {raw}"
            ));
        }
        for segment in &segments {
            if segment.is_empty() {
                return Err(format!("a repository id has an empty segment: {raw}"));
            }
            if segment.chars().any(char::is_whitespace) {
                return Err(format!("a repository id may not contain whitespace: {raw}"));
            }
        }
        Ok(Self {
            host_end: segments[0].len(),
            owner_end: segments[0].len() + 1 + segments[1].len(),
            text: raw.to_string(),
        })
    }

    /// The canonical `host/owner/repository` text form.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The hosting service, e.g. `github.com`.
    pub fn host(&self) -> &str {
        &self.text[..self.host_end]
    }

    /// The organization or personal owner.
    pub fn owner(&self) -> &str {
        &self.text[self.host_end + 1..self.owner_end]
    }

    /// The repository name.
    pub fn name(&self) -> &str {
        &self.text[self.owner_end + 1..]
    }

    /// The `host/owner` pair that a credential is resolved against.
    pub fn credential_scope(&self) -> String {
        format!("{}/{}", self.host(), self.owner())
    }
}

impl fmt::Display for RepositoryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

impl TryFrom<String> for RepositoryId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<RepositoryId> for String {
    fn from(value: RepositoryId) -> Self {
        value.text
    }
}

/// A capability an Initiative may need on one repository.
///
/// Capabilities are recorded per repository because an Initiative routinely
/// holds different permissions in different organizations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Clone and read repository contents.
    Read,
    /// Create and update issues.
    Issues,
    /// Create branches and worktrees.
    Branches,
    /// Push commits.
    Push,
    /// Open and update pull requests.
    PullRequests,
    /// Read or dispatch workflow runs.
    Workflows,
    /// Mutate a GitHub Project that includes this repository.
    ProjectMutation,
    /// Administer repository settings.
    Administration,
}

impl Capability {
    /// The stable wire name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Capability::Read => "read",
            Capability::Issues => "issues",
            Capability::Branches => "branches",
            Capability::Push => "push",
            Capability::PullRequests => "pull_requests",
            Capability::Workflows => "workflows",
            Capability::ProjectMutation => "project_mutation",
            Capability::Administration => "administration",
        }
    }

    /// Parse the stable wire name.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "read" => Ok(Capability::Read),
            "issues" => Ok(Capability::Issues),
            "branches" => Ok(Capability::Branches),
            "push" => Ok(Capability::Push),
            "pull_requests" => Ok(Capability::PullRequests),
            "workflows" => Ok(Capability::Workflows),
            "project_mutation" => Ok(Capability::ProjectMutation),
            "administration" => Ok(Capability::Administration),
            other => Err(format!("unknown repository capability: {other}")),
        }
    }

    /// The capabilities a repository needs before code changes can land.
    pub fn write_set() -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::Read,
            Capability::Branches,
            Capability::Push,
            Capability::PullRequests,
        ])
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Text that looks like a live credential rather than a credential reference.
const SECRET_MARKERS: &[&str] = &[
    "ghp_",
    "gho_",
    "ghs_",
    "ghu_",
    "github_pat_",
    "-----BEGIN",
    "AKIA",
    "sk-",
    "xoxb-",
];

/// Reject text that carries secret material.
///
/// Artifacts and prompts are durable and frequently rendered into issues, so a
/// credential reference is stored instead of the credential itself.
pub fn reject_secret_material(field: &str, value: &str) -> Result<(), String> {
    for marker in SECRET_MARKERS {
        if value.contains(marker) {
            return Err(format!(
                "{field} looks like secret material; store a credential reference instead"
            ));
        }
    }
    Ok(())
}

/// Everything workspace discovery learned about one repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryRecord {
    /// Globally qualified identity.
    pub id: RepositoryId,
    /// The revision the plan was generated against.
    pub revision: Option<String>,
    /// The repository's own default branch; never assumed across repositories.
    pub default_branch: Option<String>,
    /// A reference to the credential or App installation, never the secret.
    pub credential_reference: Option<String>,
    /// Capabilities actually available to this Initiative here.
    pub capabilities: BTreeSet<Capability>,
    /// Detected languages, for role capability matching.
    #[serde(default)]
    pub languages: Vec<String>,
    /// Detected build systems; repositories are not assumed to share one.
    #[serde(default)]
    pub build_systems: Vec<String>,
    /// Commands that validate a change in this repository.
    #[serde(default)]
    pub validation_commands: Vec<String>,
}

impl RepositoryRecord {
    /// A read-only participant: context and dependency source only.
    pub fn read_only(id: RepositoryId) -> Self {
        Self {
            id,
            revision: None,
            default_branch: None,
            credential_reference: None,
            capabilities: BTreeSet::from([Capability::Read]),
            languages: Vec::new(),
            build_systems: Vec::new(),
            validation_commands: Vec::new(),
        }
    }

    /// Whether this repository grants `capability` to the Initiative.
    pub fn grants(&self, capability: Capability) -> bool {
        self.capabilities.contains(&capability)
    }

    /// The subset of `required` this repository does not grant.
    pub fn missing(&self, required: &BTreeSet<Capability>) -> BTreeSet<Capability> {
        required
            .iter()
            .copied()
            .filter(|capability| !self.grants(*capability))
            .collect()
    }

    /// Whether the repository can only supply context.
    pub fn is_read_only(&self) -> bool {
        !self.grants(Capability::Push)
    }

    /// Reject records that embed secrets or contradict themselves.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(reference) = &self.credential_reference {
            reject_secret_material(
                &format!("{} credential_reference", self.id.as_str()),
                reference,
            )?;
        }
        if !self.grants(Capability::Read) {
            return Err(format!(
                "{} participates without read capability",
                self.id.as_str()
            ));
        }
        if self.grants(Capability::Push) && !self.grants(Capability::Branches) {
            return Err(format!(
                "{} grants push without branch creation",
                self.id.as_str()
            ));
        }
        Ok(())
    }
}

/// The repository manifest an Initiative plans against.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    /// Discovered repositories, keyed by globally qualified identity.
    #[serde(default)]
    pub repositories: BTreeMap<RepositoryId, RepositoryRecord>,
}

impl Workspace {
    /// An empty manifest.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add or replace a repository record.
    pub fn insert(&mut self, record: RepositoryRecord) -> &mut Self {
        self.repositories.insert(record.id.clone(), record);
        self
    }

    /// Look up a repository record.
    pub fn get(&self, id: &RepositoryId) -> Option<&RepositoryRecord> {
        self.repositories.get(id)
    }

    /// Whether the manifest knows this repository at all.
    pub fn contains(&self, id: &RepositoryId) -> bool {
        self.repositories.contains_key(id)
    }

    /// The distinct `host/owner` scopes the Initiative spans.
    pub fn owner_scopes(&self) -> BTreeSet<String> {
        self.repositories
            .keys()
            .map(RepositoryId::credential_scope)
            .collect()
    }

    /// Whether the Initiative spans more than one organization or owner.
    pub fn is_multi_organization(&self) -> bool {
        self.owner_scopes().len() > 1
    }

    /// The revision snapshot a plan must record.
    pub fn revision_snapshot(&self) -> BTreeMap<RepositoryId, String> {
        self.repositories
            .iter()
            .filter_map(|(id, record)| {
                record
                    .revision
                    .as_ref()
                    .map(|revision| (id.clone(), revision.clone()))
            })
            .collect()
    }

    /// Repositories whose recorded revision differs from `observed`.
    ///
    /// A drifted repository is the canonical trigger for replanning: the
    /// requirement did not change, the repository did.
    pub fn drifted_since(
        &self,
        observed: &BTreeMap<RepositoryId, String>,
    ) -> Vec<(RepositoryId, String, String)> {
        self.repositories
            .iter()
            .filter_map(|(id, record)| {
                let planned = record.revision.as_ref()?;
                let current = observed.get(id)?;
                (planned != current).then(|| (id.clone(), planned.clone(), current.clone()))
            })
            .collect()
    }

    /// Reject a manifest that cannot be planned against.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        for (id, record) in &self.repositories {
            if &record.id != id {
                problems.push(format!(
                    "repository record {} is filed under {}",
                    record.id.as_str(),
                    id.as_str()
                ));
            }
            if let Err(problem) = record.validate() {
                problems.push(problem);
            }
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository(text: &str) -> RepositoryId {
        RepositoryId::parse(text).expect("valid repository id")
    }

    fn writable(text: &str, revision: &str) -> RepositoryRecord {
        RepositoryRecord {
            id: repository(text),
            revision: Some(revision.to_string()),
            default_branch: Some("main".to_string()),
            credential_reference: Some("app-installation/inferweave".to_string()),
            capabilities: Capability::write_set(),
            languages: vec!["rust".to_string()],
            build_systems: vec!["cargo".to_string()],
            validation_commands: vec!["cargo test --workspace".to_string()],
        }
    }

    #[test]
    fn repository_ids_expose_host_owner_and_name() {
        let id = repository("github.com/InferWeave/autospec");

        assert_eq!(id.host(), "github.com");
        assert_eq!(id.owner(), "InferWeave");
        assert_eq!(id.name(), "autospec");
        assert_eq!(id.credential_scope(), "github.com/InferWeave");
    }

    #[test]
    fn repository_ids_reject_an_assumed_host() {
        let error = RepositoryId::parse("InferWeave/autospec").expect_err("owner/repo is rejected");

        assert!(error.contains("host/owner/repository"), "{error}");
    }

    #[test]
    fn a_workspace_spanning_two_owners_is_multi_organization() {
        let mut workspace = Workspace::new();
        workspace.insert(writable("github.com/InferWeave/autospec", "aaa1111"));
        workspace.insert(RepositoryRecord::read_only(repository(
            "github.com/OtherOrg/frontend",
        )));

        assert!(workspace.is_multi_organization());
        assert_eq!(workspace.owner_scopes().len(), 2);
    }

    #[test]
    fn a_single_owner_workspace_is_not_multi_organization() {
        let mut workspace = Workspace::new();
        workspace.insert(writable("github.com/InferWeave/autospec", "aaa1111"));
        workspace.insert(writable("github.com/InferWeave/gateway", "bbb2222"));

        assert!(!workspace.is_multi_organization());
    }

    #[test]
    fn read_only_repositories_participate_as_context_sources() {
        let record = RepositoryRecord::read_only(repository("github.com/OtherOrg/frontend"));

        assert!(record.is_read_only());
        assert!(record.grants(Capability::Read));
        assert_eq!(
            record.missing(&Capability::write_set()),
            BTreeSet::from([
                Capability::Branches,
                Capability::Push,
                Capability::PullRequests
            ])
        );
        record.validate().expect("read-only records are valid");
    }

    #[test]
    fn credential_references_may_not_carry_secret_material() {
        let mut record = writable("github.com/InferWeave/autospec", "aaa1111");
        record.credential_reference = Some("ghp_livetokenmaterial".to_string());

        let error = record.validate().expect_err("a live token is rejected");

        assert!(error.contains("credential reference"), "{error}");
    }

    #[test]
    fn revision_drift_is_reported_per_repository() {
        let mut workspace = Workspace::new();
        workspace.insert(writable("github.com/InferWeave/autospec", "aaa1111"));
        workspace.insert(writable("github.com/InferWeave/gateway", "bbb2222"));

        let drift = workspace.drifted_since(&BTreeMap::from([
            (repository("github.com/InferWeave/autospec"), "aaa1111".into()),
            (repository("github.com/InferWeave/gateway"), "ccc3333".into()),
        ]));

        assert_eq!(drift.len(), 1);
        assert_eq!(drift[0].0.name(), "gateway");
        assert_eq!(drift[0].2, "ccc3333");
    }

    #[test]
    fn a_manifest_rejects_push_without_branch_creation() {
        let mut record = writable("github.com/InferWeave/autospec", "aaa1111");
        record.capabilities.remove(&Capability::Branches);
        let mut workspace = Workspace::new();
        workspace.insert(record);

        let problems = workspace.validate().expect_err("inconsistent capabilities");

        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("push without branch"), "{problems:?}");
    }
}
