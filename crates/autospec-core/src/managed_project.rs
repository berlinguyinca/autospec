use crate::autonomous::waterfall::sha256_hex;
use serde::{Deserialize, Deserializer, Serialize};
use std::{fmt, str::FromStr};

pub const BINDING_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectMode {
    Managed,
    #[default]
    External,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedProjectKind {
    #[default]
    Product,
    SpecPortfolio,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PortfolioId(String);

impl PortfolioId {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err("portfolio id must be a lowercase SHA-256 digest".to_string());
        }
        Ok(Self(value))
    }

    pub fn from_source(
        canonical_source_repo: &str,
        source_spec_path: &str,
        source_spec_blob_oid: &str,
    ) -> Result<Self, String> {
        if canonical_source_repo.is_empty()
            || source_spec_path.is_empty()
            || source_spec_blob_oid.is_empty()
        {
            return Err("portfolio source identity components must not be empty".to_string());
        }
        let identity = format!("{canonical_source_repo}{source_spec_path}{source_spec_blob_oid}");
        Self::new(sha256_hex(identity.as_bytes()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PortfolioId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for PortfolioId {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<String> for PortfolioId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<PortfolioId> for String {
    fn from(value: PortfolioId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ManagedProjectNamespace {
    Product(ProductKey),
    Portfolio(PortfolioId),
}

impl ManagedProjectNamespace {
    pub fn product(product_key: ProductKey) -> Self {
        Self::Product(product_key)
    }

    pub fn portfolio(portfolio_id: PortfolioId) -> Self {
        Self::Portfolio(portfolio_id)
    }
}

impl fmt::Display for ManagedProjectNamespace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Product(product_key) => write!(formatter, "product.{product_key}"),
            Self::Portfolio(portfolio_id) => write!(formatter, "portfolio.{portfolio_id}"),
        }
    }
}

impl FromStr for ManagedProjectNamespace {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if let Some(product_key) = value.strip_prefix("product.") {
            return ProductKey::new(product_key).map(Self::Product);
        }
        if let Some(portfolio_id) = value.strip_prefix("portfolio.") {
            return PortfolioId::new(portfolio_id).map(Self::Portfolio);
        }
        Err("managed project namespace must start with 'product.' or 'portfolio.'".to_string())
    }
}

impl Serialize for ManagedProjectNamespace {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ManagedProjectNamespace {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
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
        let source = normalize_identity(&self.source);
        let target = normalize_identity(&self.target);
        let evidence_kind = self.evidence.kind.trim().to_ascii_lowercase();
        let evidence_location = self.evidence.location.trim();
        let mut key = "relationship-dedupe-v1".to_string();
        append_dedupe_component(&mut key, "product_key", self.product_key.as_str());
        append_dedupe_component(&mut key, "kind", self.kind.as_str());
        append_dedupe_component(&mut key, "source", &source);
        append_dedupe_component(&mut key, "target", &target);
        append_dedupe_component(&mut key, "evidence_kind", &evidence_kind);
        append_dedupe_component(&mut key, "evidence_location", evidence_location);
        key
    }
}

fn append_dedupe_component(key: &mut String, field: &str, value: &str) {
    key.push('|');
    key.push_str(field);
    key.push(':');
    key.push_str(&value.len().to_string());
    key.push(':');
    key.push_str(value);
}

fn normalize_identity(value: &str) -> String {
    value.trim().trim_end_matches('/').to_ascii_lowercase()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedProjectBinding {
    #[serde(deserialize_with = "deserialize_binding_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub kind: ManagedProjectKind,
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
            kind: ManagedProjectKind::Product,
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

fn deserialize_binding_schema_version<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    match u32::deserialize(deserializer)? {
        1 => Ok(BINDING_SCHEMA_VERSION),
        version => Ok(version),
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
    fn managed_project_binding_uses_schema_version_two() {
        assert_eq!(BINDING_SCHEMA_VERSION, 2);
        assert_eq!(ManagedProjectBinding::SCHEMA_VERSION, 2);
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
            "relationship-dedupe-v1|product_key:8:autospec|kind:10:depends-on|source:41:https://github.com/berlinguyinca/autospec|target:46:https://github.com/berlinguyinca/autospec-node|evidence_kind:19:manifest-dependency|evidence_location:33:Cargo.toml#workspace.dependencies"
        );
    }

    #[test]
    fn managed_project_relationship_dedupe_key_is_unambiguous_with_delimiters() {
        let edge = |source: &str, target: &str| RelationshipEdge {
            product_key: ProductKey::new("autospec").unwrap(),
            kind: RelationshipKind::DependsOn,
            source: source.to_string(),
            target: target.to_string(),
            evidence: RelationshipEvidence {
                kind: "manifest-dependency".to_string(),
                location: "Cargo.toml".to_string(),
                discovered_at: "2026-08-27T00:00:00Z".to_string(),
                confidence: 100,
            },
            state: RelationshipState::Active,
        };

        assert_ne!(
            edge("source|segment", "target").dedupe_key(),
            edge("source", "segment|target").dedupe_key()
        );
    }
}
