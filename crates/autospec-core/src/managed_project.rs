use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

pub const BINDING_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectMode {
    Managed,
    #[default]
    External,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProductKey(String);

impl ProductKey {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.is_empty() {
            return Err("product key must not be empty".to_string());
        }
        if !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        }) {
            return Err(
                "product key must contain only lowercase ASCII letters, digits, '.', '_', or '-'"
                    .to_string(),
            );
        }
        if !value
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        {
            return Err(
                "product key must start with a lowercase ASCII letter or digit".to_string(),
            );
        }
        if !value
            .as_bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        {
            return Err("product key must end with a lowercase ASCII letter or digit".to_string());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProductKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ProductKey {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<String> for ProductKey {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ProductKey> for String {
    fn from(value: ProductKey) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedProjectPolicy {
    pub product_key: ProductKey,
    pub owner: String,
    pub repository_seeds: Vec<String>,
    pub repo_allowlist: Vec<String>,
    pub discovery_max_repos: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryRecord {
    pub repository: String,
    pub entry_kind: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RelationshipKind {
    Contains,
    DependsOn,
    Implements,
    Tracks,
    SpawnedFrom,
    Blocks,
}

impl RelationshipKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Contains => "contains",
            Self::DependsOn => "depends-on",
            Self::Implements => "implements",
            Self::Tracks => "tracks",
            Self::SpawnedFrom => "spawned-from",
            Self::Blocks => "blocks",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RelationshipState {
    Active,
    Proposed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationshipEvidence {
    pub kind: String,
    pub location: String,
    pub discovered_at: String,
    pub confidence: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationshipEdge {
    pub product_key: ProductKey,
    pub kind: RelationshipKind,
    pub source: String,
    pub target: String,
    pub evidence: RelationshipEvidence,
    pub state: RelationshipState,
}

impl RelationshipEdge {
    pub fn dedupe_key(&self) -> String {
        [
            self.product_key.as_str().to_string(),
            self.kind.as_str().to_string(),
            normalize_identity(&self.source),
            normalize_identity(&self.target),
            self.evidence.kind.trim().to_ascii_lowercase(),
            self.evidence.location.trim().to_string(),
        ]
        .join("|")
    }
}

fn normalize_identity(value: &str) -> String {
    value.trim().trim_end_matches('/').to_ascii_lowercase()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedProjectBinding {
    pub schema_version: u32,
    pub product_key: ProductKey,
    pub owner: Option<String>,
    pub project_node_id: Option<String>,
    pub project_number: Option<u64>,
    pub project_url: Option<String>,
    pub project_title: Option<String>,
    pub repositories: Vec<RepositoryRecord>,
    pub last_reconciled_at: Option<String>,
    pub pending_projections: Vec<String>,
    pub relationships: Vec<RelationshipEdge>,
}

impl ManagedProjectBinding {
    pub const SCHEMA_VERSION: u32 = BINDING_SCHEMA_VERSION;

    pub fn new(product_key: ProductKey) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            product_key,
            owner: None,
            project_node_id: None,
            project_number: None,
            project_url: None,
            project_title: None,
            repositories: Vec::new(),
            last_reconciled_at: None,
            pending_projections: Vec::new(),
            relationships: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ManagedProjectBinding, ProductKey, RelationshipEdge, RelationshipEvidence,
        RelationshipKind, RelationshipState, BINDING_SCHEMA_VERSION,
    };

    #[test]
    fn managed_project_product_key_accepts_safe_identity_and_rejects_paths() {
        let key = ProductKey::new("autospec").expect("valid product key");
        assert_eq!(key.as_str(), "autospec");
        assert!(ProductKey::new("../autospec").is_err());
    }

    #[test]
    fn managed_project_binding_uses_schema_version_one() {
        assert_eq!(BINDING_SCHEMA_VERSION, 1);
        assert_eq!(ManagedProjectBinding::SCHEMA_VERSION, 1);
    }

    #[test]
    fn managed_project_relationship_dedupe_key_contains_stable_identity() {
        let edge = RelationshipEdge {
            product_key: ProductKey::new("autospec").unwrap(),
            kind: RelationshipKind::DependsOn,
            source: " HTTPS://GitHub.com/BerlinGuyInCA/Autospec ".to_string(),
            target: "https://github.com/BerlinGuyInCA/Autospec-Node/".to_string(),
            evidence: RelationshipEvidence {
                kind: "manifest-dependency".to_string(),
                location: " Cargo.toml#workspace.dependencies ".to_string(),
                discovered_at: "2026-08-27T00:00:00Z".to_string(),
                confidence: 100,
            },
            state: RelationshipState::Active,
        };

        assert_eq!(
            edge.dedupe_key(),
            "autospec|depends-on|https://github.com/berlinguyinca/autospec|https://github.com/berlinguyinca/autospec-node|manifest-dependency|Cargo.toml#workspace.dependencies"
        );
    }
}
