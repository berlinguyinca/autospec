use autospec_core::autonomous::waterfall::sha256_hex;
use serde_json::{json, Value};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "accountability/domain.rs"]
mod domain;
#[path = "accountability/github.rs"]
pub mod github;
#[path = "accountability/render.rs"]
mod render;
#[path = "accountability/store.rs"]
mod store;

#[allow(unused_imports)]
pub use store::{AccountabilityStatus, AccountabilityStore, RecoveryManifest, RecoveryState};

pub const ACCOUNTABILITY_SCHEMA: u64 = 1;

#[allow(unused_imports)]
pub use domain::*;
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderedProjection {
    pub revision: u64,
    pub digest: String,
    pub desired_high_watermark: u64,
    pub markdown: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionDisposition {
    DegradableTransport,
    IntegrityBlock,
}

#[derive(Debug)]
pub struct AccountabilityError {
    message: String,
    projection_disposition: Option<ProjectionDisposition>,
    retry_after_seconds: Option<u64>,
}

impl AccountabilityError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            projection_disposition: None,
            retry_after_seconds: None,
        }
    }

    fn projection(message: impl Into<String>, disposition: ProjectionDisposition) -> Self {
        Self {
            message: message.into(),
            projection_disposition: Some(disposition),
            retry_after_seconds: None,
        }
    }

    pub fn projection_disposition(&self) -> Option<ProjectionDisposition> {
        self.projection_disposition
    }

    pub fn retry_after_seconds(&self) -> Option<u64> {
        self.retry_after_seconds
    }

    fn with_retry_after(mut self, retry_after_seconds: Option<u64>) -> Self {
        self.retry_after_seconds = retry_after_seconds;
        self
    }

    fn into_projection(mut self, disposition: ProjectionDisposition) -> Self {
        if self.projection_disposition.is_none() {
            self.projection_disposition = Some(disposition);
        }
        self
    }
}

impl fmt::Display for AccountabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AccountabilityError {}

fn validate_summary(value: String, field: &str) -> Result<String, AccountabilityError> {
    let sanitized = sanitize_text(&value, 1024);
    if sanitized.trim().is_empty() {
        return Err(AccountabilityError::new(format!("{field} is mandatory")));
    }
    Ok(sanitized)
}

fn sanitize_text(value: &str, limit: usize) -> String {
    let mut output = String::with_capacity(value.len().min(limit));
    let mut redact_next = false;
    let mut pem_block = false;
    for token in value.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if lower.contains("-----begin") {
            pem_block = true;
        }
        let credential_assignment = [
            "github_token=",
            "token=",
            "password=",
            "api_key=",
            "apikey=",
            "secret=",
        ]
        .iter()
        .any(|key| lower.starts_with(key));
        let embedded_absolute_path = token.split_once('=').is_some_and(|(_, value)| {
            value.starts_with('/')
                || value.starts_with('~')
                || value.starts_with("\\\\")
                || value.as_bytes().get(1) == Some(&b':')
        });
        let secret = pem_block
            || redact_next
            || lower == "bearer"
            || token.starts_with("ghp_")
            || token.starts_with("github_pat_")
            || lower.starts_with("xoxb-")
            || lower.starts_with("xoxp-")
            || lower.starts_with("glpat-")
            || lower.starts_with("sk-")
            || credential_assignment
            || lower == "private"
            || lower == "key"
            || lower.contains("private") && lower.contains("key")
            || lower.contains("-----begin")
            || lower.contains("-----end")
            || (token.starts_with("AKIA") && token.len() >= 20)
            || token.starts_with('/')
            || token.starts_with('~')
            || token.starts_with("\\\\")
            || token.as_bytes().get(1) == Some(&b':')
            || token.contains("=/")
            || token.contains("=~/")
            || token.contains("=\\\\")
            || embedded_absolute_path
            || token.contains("%%{")
            || token.contains("<!--")
            || token.contains("-->")
            || token.to_ascii_lowercase().contains("<script");
        let token = if secret { "[redacted]" } else { token };
        redact_next = lower == "bearer";
        for character in token.chars() {
            if output.len() + character.len_utf8() > limit {
                break;
            }
            if !character.is_control()
                && !matches!(
                    character,
                    '<' | '>' | '`' | '[' | ']' | '{' | '}' | '%' | '|'
                )
            {
                output.push(character);
            }
        }
        if output.len() < limit {
            output.push(' ');
        }
    }
    output.trim().to_owned()
}

fn object<'a>(
    value: &'a Value,
    name: &str,
) -> Result<&'a serde_json::Map<String, Value>, AccountabilityError> {
    value
        .as_object()
        .ok_or_else(|| AccountabilityError::new(format!("{name} must be an object")))
}

fn required<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a Value, AccountabilityError> {
    object
        .get(field)
        .ok_or_else(|| AccountabilityError::new(format!("missing {field}")))
}

fn string<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a str, AccountabilityError> {
    required(object, field)?
        .as_str()
        .ok_or_else(|| AccountabilityError::new(format!("{field} must be a string")))
}

fn unsigned(
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<u64, AccountabilityError> {
    required(object, field)?
        .as_u64()
        .ok_or_else(|| AccountabilityError::new(format!("{field} must be an unsigned integer")))
}

fn timestamp_now() -> Result<u64, AccountabilityError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| AccountabilityError::new("system clock precedes Unix epoch"))
}
