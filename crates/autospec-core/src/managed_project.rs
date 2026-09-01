use crate::autonomous::waterfall::sha256_hex;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, ops::Deref, str::FromStr};

pub const BINDING_SCHEMA_VERSION: u32 = 2;

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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ItemKey(String);

impl ItemKey {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.is_empty() || value.len() > 200 {
            return Err("portfolio item key must contain between 1 and 200 bytes".to_string());
        }
        if !value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
        }) {
            return Err("portfolio item key contains an unsafe character".to_string());
        }
        if value
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        {
            return Err("portfolio item key contains an unsafe path segment".to_string());
        }
        if !value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
            || !value
                .as_bytes()
                .last()
                .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        {
            return Err(
                "portfolio item key must start with a letter and end with a letter or digit"
                    .to_string(),
            );
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ItemKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ItemKey {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<String> for ItemKey {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ItemKey> for String {
    fn from(value: ItemKey) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SourceSpecIdentity {
    canonical_source_repo: String,
    source_spec_path: String,
    source_spec_blob_oid: String,
}

impl SourceSpecIdentity {
    pub fn new(
        source_repo: &str,
        source_spec_path: &str,
        source_spec_blob_oid: &str,
    ) -> Result<Self, String> {
        Ok(Self {
            canonical_source_repo: canonical_repository(source_repo)?,
            source_spec_path: canonical_spec_path(source_spec_path)?,
            source_spec_blob_oid: canonical_blob_oid(source_spec_blob_oid)?,
        })
    }

    pub fn canonical_source_repo(&self) -> &str {
        &self.canonical_source_repo
    }

    pub fn source_spec_path(&self) -> &str {
        &self.source_spec_path
    }

    pub fn source_spec_blob_oid(&self) -> &str {
        &self.source_spec_blob_oid
    }

    pub fn portfolio_id(&self) -> PortfolioId {
        PortfolioId::from_source(
            self.canonical_source_repo(),
            self.source_spec_path(),
            self.source_spec_blob_oid(),
        )
        .expect("validated source identity always produces a portfolio id")
    }
}

impl fmt::Display for SourceSpecIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}@{}",
            self.canonical_source_repo, self.source_spec_path, self.source_spec_blob_oid
        )
    }
}

impl FromStr for SourceSpecIdentity {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (repository, path_and_oid) = value
            .split_once(':')
            .ok_or_else(|| "source spec identity must contain ':'".to_string())?;
        let (path, oid) = path_and_oid
            .rsplit_once('@')
            .ok_or_else(|| "source spec identity must contain '@'".to_string())?;
        Self::new(repository, path, oid)
    }
}

impl TryFrom<String> for SourceSpecIdentity {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<SourceSpecIdentity> for String {
    fn from(value: SourceSpecIdentity) -> Self {
        value.to_string()
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
        let mut identity = b"autospec.portfolio-id.v1".to_vec();
        for component in [
            canonical_source_repo,
            source_spec_path,
            source_spec_blob_oid,
        ] {
            identity.extend_from_slice(&(component.len() as u64).to_be_bytes());
            identity.extend_from_slice(component.as_bytes());
        }
        Self::new(sha256_hex(&identity))
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct SpecPortfolioIdentity {
    portfolio_id: PortfolioId,
    source: SourceSpecIdentity,
}

impl SpecPortfolioIdentity {
    pub fn new(source: SourceSpecIdentity) -> Self {
        Self {
            portfolio_id: source.portfolio_id(),
            source,
        }
    }

    pub fn portfolio_id(&self) -> &PortfolioId {
        &self.portfolio_id
    }

    pub fn source(&self) -> &SourceSpecIdentity {
        &self.source
    }

    fn from_parts(portfolio_id: PortfolioId, source: SourceSpecIdentity) -> Result<Self, String> {
        if source.portfolio_id() != portfolio_id {
            return Err("spec portfolio id does not match its source identity".to_string());
        }
        Ok(Self {
            portfolio_id,
            source,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpecPortfolioIdentityWire {
    portfolio_id: PortfolioId,
    source: SourceSpecIdentity,
}

impl<'de> Deserialize<'de> for SpecPortfolioIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = SpecPortfolioIdentityWire::deserialize(deserializer)?;
        Self::from_parts(wire.portfolio_id, wire.source).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ManagedProjectIdentity {
    Product { product_key: ProductKey },
    SpecPortfolio(SpecPortfolioIdentity),
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ManagedProjectIdentityRef<'a> {
    Product {
        product_key: &'a ProductKey,
    },
    SpecPortfolio {
        portfolio_id: &'a PortfolioId,
        source: &'a SourceSpecIdentity,
    },
}

impl Serialize for ManagedProjectIdentity {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Product { product_key } => {
                ManagedProjectIdentityRef::Product { product_key }.serialize(serializer)
            }
            Self::SpecPortfolio(identity) => ManagedProjectIdentityRef::SpecPortfolio {
                portfolio_id: identity.portfolio_id(),
                source: identity.source(),
            }
            .serialize(serializer),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ManagedProjectIdentityWire {
    Product {
        product_key: ProductKey,
    },
    SpecPortfolio {
        portfolio_id: PortfolioId,
        source: SourceSpecIdentity,
    },
}

impl<'de> Deserialize<'de> for ManagedProjectIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match ManagedProjectIdentityWire::deserialize(deserializer)? {
            ManagedProjectIdentityWire::Product { product_key } => {
                Ok(Self::Product { product_key })
            }
            ManagedProjectIdentityWire::SpecPortfolio {
                portfolio_id,
                source,
            } => SpecPortfolioIdentity::from_parts(portfolio_id, source)
                .map(Self::SpecPortfolio)
                .map_err(serde::de::Error::custom),
        }
    }
}

impl ManagedProjectIdentity {
    pub fn namespace(&self) -> ManagedProjectNamespace {
        match self {
            Self::Product { product_key } => ManagedProjectNamespace::Product(product_key.clone()),
            Self::SpecPortfolio(identity) => {
                ManagedProjectNamespace::Portfolio(identity.portfolio_id().clone())
            }
        }
    }

    fn compatibility_product_key(&self) -> Option<ProductKey> {
        match self {
            Self::Product { product_key } => Some(product_key.clone()),
            Self::SpecPortfolio(_) => None,
        }
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

fn canonical_repository(value: &str) -> Result<String, String> {
    let value = value.trim().trim_end_matches('/').to_ascii_lowercase();
    let mut segments = value.split('/');
    let owner = segments.next().unwrap_or_default();
    let repository = segments.next().unwrap_or_default();
    if segments.next().is_some()
        || !safe_repository_segment(owner)
        || !safe_repository_segment(repository)
    {
        return Err("source repository must be a canonical owner/repository identity".to_string());
    }
    Ok(format!("{owner}/{repository}"))
}

fn safe_repository_segment(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
        && value
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && value
            .as_bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

fn canonical_spec_path(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || value.starts_with('/')
        || value.contains(['\\', ':', '@'])
        || value
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-'))
    {
        return Err("source spec path must be a safe repository-relative path".to_string());
    }
    Ok(value.to_string())
}

fn canonical_blob_oid(value: &str) -> Result<String, String> {
    let value = value.trim().to_ascii_lowercase();
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(
            "source spec blob OID must be a 40- or 64-character hexadecimal digest".to_string(),
        );
    }
    Ok(value)
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedProjectBinding {
    pub schema_version: u32,
    identity: ManagedProjectIdentity,
    compatibility: Option<ManagedProjectProductCompatibility>,
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

#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedProjectProductCompatibility {
    pub product_key: ProductKey,
}

impl Deref for ManagedProjectBinding {
    type Target = ManagedProjectProductCompatibility;

    fn deref(&self) -> &Self::Target {
        // Directive: remove this product-only bridge after managed-project store callers
        // consume ManagedProjectIdentity directly. Portfolio bindings intentionally fail closed.
        self.compatibility
            .as_ref()
            .expect("spec portfolio bindings do not expose product compatibility")
    }
}

impl ManagedProjectBinding {
    pub const SCHEMA_VERSION: u32 = BINDING_SCHEMA_VERSION;

    pub fn new(product_key: ProductKey) -> Self {
        Self::new_identity(ManagedProjectIdentity::Product { product_key })
            .expect("product identity is always internally consistent")
    }

    pub fn new_identity(identity: ManagedProjectIdentity) -> Result<Self, String> {
        let compatibility = identity
            .compatibility_product_key()
            .map(|product_key| ManagedProjectProductCompatibility { product_key });
        Ok(Self {
            schema_version: Self::SCHEMA_VERSION,
            identity,
            compatibility,
            owner: None,
            project_node_id: None,
            project_number: None,
            project_url: None,
            project_title: None,
            repositories: Vec::new(),
            last_reconciled_at: None,
            pending_projections: Vec::new(),
            relationships: Vec::new(),
        })
    }

    pub fn identity(&self) -> &ManagedProjectIdentity {
        &self.identity
    }
}

#[derive(Serialize)]
struct ManagedProjectBindingV2<'a> {
    schema_version: u32,
    identity: &'a ManagedProjectIdentity,
    owner: &'a Option<String>,
    project_node_id: &'a Option<String>,
    project_number: &'a Option<u64>,
    project_url: &'a Option<String>,
    project_title: &'a Option<String>,
    repositories: &'a [RepositoryRecord],
    last_reconciled_at: &'a Option<String>,
    pending_projections: &'a [String],
    relationships: &'a [RelationshipEdge],
}

impl Serialize for ManagedProjectBinding {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        ManagedProjectBindingV2 {
            schema_version: Self::SCHEMA_VERSION,
            identity: &self.identity,
            owner: &self.owner,
            project_node_id: &self.project_node_id,
            project_number: &self.project_number,
            project_url: &self.project_url,
            project_title: &self.project_title,
            repositories: &self.repositories,
            last_reconciled_at: &self.last_reconciled_at,
            pending_projections: &self.pending_projections,
            relationships: &self.relationships,
        }
        .serialize(serializer)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedProjectBindingWire {
    schema_version: u32,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    product_key: Option<ProductKey>,
    #[serde(default)]
    identity: Option<ManagedProjectIdentity>,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    project_node_id: Option<String>,
    #[serde(default)]
    project_number: Option<u64>,
    #[serde(default)]
    project_url: Option<String>,
    #[serde(default)]
    project_title: Option<String>,
    #[serde(default)]
    repositories: Vec<RepositoryRecord>,
    #[serde(default)]
    last_reconciled_at: Option<String>,
    #[serde(default)]
    pending_projections: Vec<String>,
    #[serde(default)]
    relationships: Vec<RelationshipEdge>,
    #[serde(default)]
    journal_high_watermark: Option<u64>,
    #[serde(default)]
    journal_digest: Option<String>,
}

impl<'de> Deserialize<'de> for ManagedProjectBinding {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ManagedProjectBindingWire::deserialize(deserializer)?;
        let _journal_envelope = (&wire.journal_high_watermark, &wire.journal_digest);
        let identity = match wire.schema_version {
            1 => {
                if wire.identity.is_some()
                    || wire.kind.as_deref().is_some_and(|kind| kind != "product")
                {
                    return Err(serde::de::Error::custom(
                        "schema 1 managed project binding must be product-only",
                    ));
                }
                ManagedProjectIdentity::Product {
                    product_key: wire.product_key.ok_or_else(|| {
                        serde::de::Error::custom(
                            "schema 1 managed project binding requires product_key",
                        )
                    })?,
                }
            }
            2 => {
                if wire.kind.is_some() || wire.product_key.is_some() {
                    return Err(serde::de::Error::custom(
                        "schema 2 managed project binding requires typed identity",
                    ));
                }
                wire.identity.ok_or_else(|| {
                    serde::de::Error::custom(
                        "schema 2 managed project binding requires typed identity",
                    )
                })?
            }
            _ => {
                return Err(serde::de::Error::custom(
                    "unsupported managed project binding schema",
                ))
            }
        };
        let mut binding = Self::new_identity(identity).map_err(serde::de::Error::custom)?;
        binding.owner = wire.owner;
        binding.project_node_id = wire.project_node_id;
        binding.project_number = wire.project_number;
        binding.project_url = wire.project_url;
        binding.project_title = wire.project_title;
        binding.repositories = wire.repositories;
        binding.last_reconciled_at = wire.last_reconciled_at;
        binding.pending_projections = wire.pending_projections;
        binding.relationships = wire.relationships;
        Ok(binding)
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
