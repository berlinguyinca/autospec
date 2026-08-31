use autospec_core::managed_project::PortfolioId;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSpecIdentity {
    pub canonical_source_repo: String,
    pub source_spec_path: String,
    pub source_spec_blob_oid: String,
}

impl SourceSpecIdentity {
    pub fn new(
        source_repo: &str,
        source_spec_path: &str,
        source_spec_blob_oid: &str,
    ) -> Result<Self, String> {
        let canonical_source_repo = canonical_repository(source_repo)?;
        let source_spec_path = canonical_spec_path(source_spec_path)?;
        let source_spec_blob_oid = canonical_blob_oid(source_spec_blob_oid)?;
        Ok(Self {
            canonical_source_repo,
            source_spec_path,
            source_spec_blob_oid,
        })
    }

    pub fn portfolio_id(&self) -> PortfolioId {
        PortfolioId::from_source(
            &self.canonical_source_repo,
            &self.source_spec_path,
            &self.source_spec_blob_oid,
        )
        .expect("validated source identity always produces a portfolio id")
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
        || value.contains('\\')
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
