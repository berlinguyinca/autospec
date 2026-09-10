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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrimaryProjectPolicy {
    SpecPortfolio,
    ManagedProduct,
    CreateManagedProduct,
}

impl PrimaryProjectPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SpecPortfolio => "spec_portfolio",
            Self::ManagedProduct => "managed_product",
            Self::CreateManagedProduct => "create_managed_product",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrimaryProjectBinding {
    policy: PrimaryProjectPolicy,
    identity: ManagedProjectIdentity,
    owner: String,
}

impl PrimaryProjectBinding {
    pub fn new(
        policy: PrimaryProjectPolicy,
        identity: ManagedProjectIdentity,
        owner: String,
    ) -> Result<Self, String> {
        validate_project_owner(&owner)?;
        Ok(Self {
            policy,
            identity,
            owner,
        })
    }

    pub fn policy(&self) -> PrimaryProjectPolicy {
        self.policy
    }

    pub fn identity(&self) -> &ManagedProjectIdentity {
        &self.identity
    }

    pub fn namespace(&self) -> ManagedProjectNamespace {
        self.identity.namespace()
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn product_key(&self) -> Option<&ProductKey> {
        match &self.identity {
            ManagedProjectIdentity::Product { product_key } => Some(product_key),
            ManagedProjectIdentity::SpecPortfolio(_) => None,
        }
    }

    pub fn portfolio_identity(&self) -> Option<&SpecPortfolioIdentity> {
        match &self.identity {
            ManagedProjectIdentity::Product { .. } => None,
            ManagedProjectIdentity::SpecPortfolio(identity) => Some(identity),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrimaryProjectFacts {
    canonical_source_repo: String,
    source_spec: Option<SourceSpecIdentity>,
    verified_portfolio: Option<SpecPortfolioIdentity>,
    verified_portfolio_owner: Option<String>,
    product: Option<ProductKey>,
    product_mode: ProjectMode,
    planned_issue_count: u64,
    cross_repository_edges: u64,
    requested_owner: Option<String>,
}

impl PrimaryProjectFacts {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        canonical_source_repo: &str,
        source_spec: Option<SourceSpecIdentity>,
        verified_portfolio: Option<SpecPortfolioIdentity>,
        verified_portfolio_owner: Option<&str>,
        product: Option<ProductKey>,
        product_mode: ProjectMode,
        planned_issue_count: u64,
        cross_repository_edges: u64,
        requested_owner: Option<&str>,
    ) -> Result<Self, String> {
        let verified_portfolio_owner = match verified_portfolio_owner {
            Some(owner) => {
                let owner = owner.trim().to_owned();
                validate_project_owner(&owner)?;
                Some(owner)
            }
            None => None,
        };
        let requested_owner = match requested_owner {
            Some(owner) => {
                let owner = owner.trim().to_owned();
                validate_project_owner(&owner)?;
                Some(owner)
            }
            None => None,
        };
        if let Some(verified) = verified_portfolio.as_ref() {
            let source = source_spec.as_ref().ok_or_else(|| {
                "verified spec portfolio binding requires the source spec identity".to_string()
            })?;
            if verified.source() != source {
                return Err(
                    "verified spec portfolio binding does not match the source spec identity"
                        .to_string(),
                );
            }
        }
        Ok(Self {
            canonical_source_repo: canonical_repository(canonical_source_repo)?,
            source_spec,
            verified_portfolio,
            verified_portfolio_owner,
            product,
            product_mode,
            planned_issue_count,
            cross_repository_edges,
            requested_owner,
        })
    }

    pub fn canonical_source_repo(&self) -> &str {
        &self.canonical_source_repo
    }

    pub fn source_spec(&self) -> Option<&SourceSpecIdentity> {
        self.source_spec.as_ref()
    }

    pub fn verified_portfolio(&self) -> Option<&SpecPortfolioIdentity> {
        self.verified_portfolio.as_ref()
    }

    pub fn product(&self) -> Option<&ProductKey> {
        self.product.as_ref()
    }

    pub fn product_mode(&self) -> ProjectMode {
        self.product_mode
    }

    pub fn planned_issue_count(&self) -> u64 {
        self.planned_issue_count
    }

    pub fn cross_repository_edges(&self) -> u64 {
        self.cross_repository_edges
    }

    pub fn requested_owner(&self) -> Option<&str> {
        self.requested_owner.as_deref()
    }

    pub fn resolved_owner(&self) -> Result<String, String> {
        if let Some(owner) = self.requested_owner.clone() {
            return Ok(owner);
        }
        let owner = self
            .canonical_source_repo
            .split_once('/')
            .map(|(owner, _)| owner)
            .unwrap_or_default()
            .to_owned();
        validate_project_owner(&owner)?;
        Ok(owner)
    }

    pub fn resolve_primary_project(&self) -> Result<PrimaryProjectBinding, String> {
        let owner = self.resolved_owner()?;
        if let Some(verified) = self.verified_portfolio.clone() {
            let expected = self
                .verified_portfolio_owner
                .clone()
                .ok_or_else(|| "verified spec portfolio binding has no owner".to_string())?;
            if owner != expected {
                return Err(format!(
                    "project owner {owner} conflicts with verified spec portfolio owner {expected}"
                ));
            }
            return PrimaryProjectBinding::new(
                PrimaryProjectPolicy::SpecPortfolio,
                ManagedProjectIdentity::SpecPortfolio(verified),
                owner,
            );
        }
        match self.source_spec.clone() {
            Some(source) => PrimaryProjectBinding::new(
                PrimaryProjectPolicy::SpecPortfolio,
                ManagedProjectIdentity::SpecPortfolio(SpecPortfolioIdentity::new(source)),
                owner,
            ),
            None => self.bounded_product_binding(owner),
        }
    }

    fn bounded_product_binding(&self, owner: String) -> Result<PrimaryProjectBinding, String> {
        if self.planned_issue_count > 1 || self.cross_repository_edges > 0 {
            return Err(
                "multi-issue or cross-repository scope requires a source spec identity for the primary spec portfolio"
                    .to_string(),
            );
        }
        let (policy, product) = match (self.product.clone(), self.product_mode) {
            (Some(product), ProjectMode::Managed) => {
                (PrimaryProjectPolicy::ManagedProduct, product)
            }
            (Some(product), ProjectMode::External) => {
                (PrimaryProjectPolicy::CreateManagedProduct, product)
            }
            (None, _) => (
                PrimaryProjectPolicy::CreateManagedProduct,
                repository_product_key(&self.canonical_source_repo)?,
            ),
        };
        PrimaryProjectBinding::new(
            policy,
            ManagedProjectIdentity::Product {
                product_key: product,
            },
            owner,
        )
    }
}

pub fn validate_project_owner(owner: &str) -> Result<(), String> {
    if !(1..=39).contains(&owner.len())
        || !owner
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || owner.starts_with('-')
        || owner.ends_with('-')
        || owner.contains("--")
    {
        return Err(
            "project owner must be a GitHub login of 1-39 ASCII letters, digits, or single hyphens that does not start or end with a hyphen"
                .to_string(),
        );
    }
    Ok(())
}

fn repository_product_key(canonical_source_repo: &str) -> Result<ProductKey, String> {
    let (owner, repository) = canonical_source_repo.split_once('/').ok_or_else(|| {
        "source repository must be a canonical owner/repository identity".to_string()
    })?;
    ProductKey::new(format!("repo.{owner}__{repository}"))
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
        ManagedProjectBinding, ManagedProjectIdentity, ManagedProjectNamespace, PortfolioId,
        PrimaryProjectBinding, PrimaryProjectFacts, PrimaryProjectPolicy, ProductKey, ProjectMode,
        RelationshipEdge, RelationshipEvidence, RelationshipKind, RelationshipState,
        SourceSpecIdentity, SpecPortfolioIdentity, BINDING_SCHEMA_VERSION,
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

    const OID40: &str = "0123456789abcdef0123456789abcdef01234567";

    fn demo_source() -> SourceSpecIdentity {
        SourceSpecIdentity::new("berlinguyinca/autospec", "docs/specs/demo-design.md", OID40)
            .expect("valid source spec identity")
    }

    #[test]
    fn primary_project_selection_adopts_verified_spec_lineage() {
        let source = demo_source();
        let verified = SpecPortfolioIdentity::new(source.clone());
        let facts = PrimaryProjectFacts::new(
            "berlinguyinca/autospec",
            Some(source),
            Some(verified.clone()),
            Some("berlinguyinca"),
            None,
            ProjectMode::External,
            12,
            3,
            None,
        )
        .expect("valid facts");
        let binding = facts
            .resolve_primary_project()
            .expect("verified lineage resolves");
        assert_eq!(binding.policy(), PrimaryProjectPolicy::SpecPortfolio);
        assert_eq!(
            binding.identity(),
            &ManagedProjectIdentity::SpecPortfolio(verified)
        );
        assert_eq!(binding.owner(), "berlinguyinca");
        assert!(binding.product_key().is_none());
        assert!(binding.portfolio_identity().is_some());
    }

    #[test]
    fn primary_project_selection_creates_spec_portfolio_for_new_spec() {
        let source = demo_source();
        let facts = PrimaryProjectFacts::new(
            "berlinguyinca/autospec",
            Some(source.clone()),
            None,
            None,
            None,
            ProjectMode::External,
            1,
            0,
            None,
        )
        .expect("valid facts");
        let binding = facts
            .resolve_primary_project()
            .expect("spec-sized scope resolves");
        assert_eq!(binding.policy(), PrimaryProjectPolicy::SpecPortfolio);
        assert_eq!(
            binding.identity(),
            &ManagedProjectIdentity::SpecPortfolio(SpecPortfolioIdentity::new(source))
        );
    }

    #[test]
    fn primary_project_selection_prefers_spec_portfolio_over_managed_product() {
        let facts = PrimaryProjectFacts::new(
            "berlinguyinca/autospec",
            Some(demo_source()),
            None,
            None,
            Some(ProductKey::new("autospec").expect("valid key")),
            ProjectMode::Managed,
            1,
            0,
            None,
        )
        .expect("valid facts");
        assert_eq!(
            facts.resolve_primary_project().expect("resolves").policy(),
            PrimaryProjectPolicy::SpecPortfolio
        );
    }

    #[test]
    fn primary_project_selection_bounded_work_uses_managed_product() {
        let facts = PrimaryProjectFacts::new(
            "berlinguyinca/autospec",
            None,
            None,
            None,
            Some(ProductKey::new("autospec").expect("valid key")),
            ProjectMode::Managed,
            1,
            0,
            None,
        )
        .expect("valid facts");
        let binding = facts
            .resolve_primary_project()
            .expect("bounded scope resolves");
        assert_eq!(binding.policy(), PrimaryProjectPolicy::ManagedProduct);
        assert_eq!(
            binding.product_key().map(ProductKey::as_str),
            Some("autospec")
        );
        assert_eq!(binding.namespace().to_string(), "product.autospec");
        assert!(binding.portfolio_identity().is_none());
    }

    #[test]
    fn primary_project_selection_external_mode_is_never_primary() {
        let facts = PrimaryProjectFacts::new(
            "berlinguyinca/autospec",
            None,
            None,
            None,
            Some(ProductKey::new("autospec").expect("valid key")),
            ProjectMode::External,
            1,
            0,
            None,
        )
        .expect("valid facts");
        let binding = facts
            .resolve_primary_project()
            .expect("bounded scope resolves");
        assert_eq!(binding.policy(), PrimaryProjectPolicy::CreateManagedProduct);
        assert_eq!(
            binding.product_key().map(ProductKey::as_str),
            Some("autospec")
        );
    }

    #[test]
    fn primary_project_selection_creates_repository_product_for_untracked_work() {
        let facts = PrimaryProjectFacts::new(
            "berlinguyinca/autospec",
            None,
            None,
            None,
            None,
            ProjectMode::External,
            1,
            0,
            None,
        )
        .expect("valid facts");
        let binding = facts
            .resolve_primary_project()
            .expect("bounded scope resolves");
        assert_eq!(binding.policy(), PrimaryProjectPolicy::CreateManagedProduct);
        assert_eq!(
            binding.product_key().map(ProductKey::as_str),
            Some("repo.berlinguyinca__autospec")
        );
        assert_eq!(
            binding.namespace().to_string(),
            "product.repo.berlinguyinca__autospec"
        );
    }

    #[test]
    fn primary_project_selection_rejects_multi_issue_scope_without_spec() {
        let facts = PrimaryProjectFacts::new(
            "berlinguyinca/autospec",
            None,
            None,
            None,
            None,
            ProjectMode::External,
            2,
            0,
            None,
        )
        .expect("valid facts");
        let error = facts
            .resolve_primary_project()
            .expect_err("multi-issue scope without spec must block");
        assert!(error.contains("source spec identity"), "{error}");
    }

    #[test]
    fn primary_project_selection_rejects_cross_repository_scope_without_spec() {
        let facts = PrimaryProjectFacts::new(
            "berlinguyinca/autospec",
            None,
            None,
            None,
            None,
            ProjectMode::External,
            1,
            1,
            None,
        )
        .expect("valid facts");
        let error = facts
            .resolve_primary_project()
            .expect_err("cross-repository scope without spec must block");
        assert!(error.contains("source spec identity"), "{error}");
    }

    #[test]
    fn primary_project_facts_reject_verified_binding_without_spec() {
        let verified = SpecPortfolioIdentity::new(demo_source());
        assert!(PrimaryProjectFacts::new(
            "berlinguyinca/autospec",
            None,
            Some(verified),
            Some("berlinguyinca"),
            None,
            ProjectMode::External,
            1,
            0,
            None,
        )
        .is_err());
    }

    #[test]
    fn primary_project_facts_reject_verified_binding_lineage_mismatch() {
        let source = demo_source();
        let foreign = SourceSpecIdentity::new(
            "berlinguyinca/autospec",
            "docs/specs/other-design.md",
            OID40,
        )
        .expect("valid identity");
        let verified = SpecPortfolioIdentity::new(foreign);
        assert!(PrimaryProjectFacts::new(
            "berlinguyinca/autospec",
            Some(source),
            Some(verified),
            Some("berlinguyinca"),
            None,
            ProjectMode::External,
            1,
            0,
            None,
        )
        .is_err());
    }

    #[test]
    fn primary_project_selection_blocks_owner_conflict_with_verified_portfolio() {
        let source = demo_source();
        let verified = SpecPortfolioIdentity::new(source.clone());
        let facts = PrimaryProjectFacts::new(
            "berlinguyinca/autospec",
            Some(source),
            Some(verified),
            Some("berlinguyinca"),
            None,
            ProjectMode::External,
            1,
            0,
            Some("other-org"),
        )
        .expect("valid facts");
        let error = facts
            .resolve_primary_project()
            .expect_err("owner conflict must block adoption");
        assert!(error.contains("other-org"), "{error}");
        assert!(error.contains("berlinguyinca"), "{error}");
    }

    #[test]
    fn primary_project_owner_grammar_accepts_logins_and_rejects_unsafe_values() {
        for owner in ["berlinguyinca", "My-Org", "a1-b2", "x".repeat(39).as_str()] {
            assert!(
                PrimaryProjectFacts::new(
                    "berlinguyinca/autospec",
                    None,
                    None,
                    None,
                    None,
                    ProjectMode::External,
                    1,
                    0,
                    Some(owner)
                )
                .is_ok(),
                "{owner} must be accepted"
            );
        }
        for owner in [
            "",
            "-",
            "a--b",
            "a-".repeat(20).as_str(),
            "bad owner",
            "owner_1",
            "owner/1",
        ] {
            assert!(
                PrimaryProjectFacts::new(
                    "berlinguyinca/autospec",
                    None,
                    None,
                    None,
                    None,
                    ProjectMode::External,
                    1,
                    0,
                    Some(owner)
                )
                .is_err(),
                "{owner:?} must be rejected"
            );
        }
    }

    #[test]
    fn primary_project_owner_defaults_to_source_repository_owner() {
        let facts = PrimaryProjectFacts::new(
            "berlinguyinca/autospec",
            None,
            None,
            None,
            None,
            ProjectMode::External,
            1,
            0,
            None,
        )
        .expect("valid facts");
        assert_eq!(
            facts.resolved_owner().expect("owner resolves"),
            "berlinguyinca"
        );
    }

    #[test]
    fn primary_project_binding_round_trips_through_serde() {
        let facts = PrimaryProjectFacts::new(
            "berlinguyinca/autospec",
            Some(demo_source()),
            None,
            None,
            None,
            ProjectMode::External,
            1,
            0,
            None,
        )
        .expect("valid facts");
        let binding = facts.resolve_primary_project().expect("resolves");
        let decoded: PrimaryProjectBinding =
            serde_json::from_str(&serde_json::to_string(&binding).expect("binding serializes"))
                .expect("binding deserializes");
        assert_eq!(decoded, binding);
    }

    #[test]
    fn managed_project_namespaces_round_trip_and_resist_portfolio_product_collision() {
        let id =
            PortfolioId::from_source("berlinguyinca/autospec", "docs/specs/demo-design.md", OID40)
                .expect("valid portfolio id");
        let portfolio = ManagedProjectNamespace::portfolio(id.clone());
        assert_eq!(
            portfolio
                .to_string()
                .parse::<ManagedProjectNamespace>()
                .expect("portfolio namespace round trip"),
            portfolio
        );
        let product =
            ManagedProjectNamespace::product(ProductKey::new("autospec").expect("valid key"));
        assert_eq!(
            product
                .to_string()
                .parse::<ManagedProjectNamespace>()
                .expect("product namespace round trip"),
            product
        );
        let disguised =
            ProductKey::new(format!("portfolio.{id}")).expect("grammatically valid product key");
        let parsed = ManagedProjectNamespace::product(disguised)
            .to_string()
            .parse::<ManagedProjectNamespace>()
            .expect("round trip");
        assert!(matches!(parsed, ManagedProjectNamespace::Product(_)));
    }
}
