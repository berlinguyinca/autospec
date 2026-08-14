use autospec_core::autonomous::waterfall::sha256_hex;
use serde_json::{json, Value};
use std::fmt;

#[path = "accountability/github.rs"]
pub mod github;
#[path = "accountability/render.rs"]
mod render;
#[path = "accountability/store.rs"]
mod store;

#[allow(unused_imports)]
pub use store::{AccountabilityStatus, AccountabilityStore, RecoveryManifest, RecoveryState};

pub const ACCOUNTABILITY_SCHEMA: u64 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryIdentity(String);

impl RepositoryIdentity {
    pub fn parse(value: &str) -> Result<Self, AccountabilityError> {
        let mut parts = value.split('/');
        let owner = parts.next().unwrap_or_default();
        let repository = parts.next().unwrap_or_default();
        if owner.is_empty()
            || repository.is_empty()
            || parts.next().is_some()
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/')
            })
        {
            return Err(AccountabilityError::new("repository must be OWNER/REPO"));
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunNonce(String);

impl RunNonce {
    pub fn parse(value: &str) -> Result<Self, AccountabilityError> {
        if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(AccountabilityError::new(
                "run nonce must be exactly 128 bits of hexadecimal",
            ));
        }
        Ok(Self(value.to_ascii_lowercase()))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LeaseGeneration(u64);

impl LeaseGeneration {
    pub fn new(value: u64) -> Result<Self, AccountabilityError> {
        if value == 0 {
            return Err(AccountabilityError::new(
                "lifecycle lease generation must be positive",
            ));
        }
        Ok(Self(value))
    }

    fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunIdentity {
    repository: RepositoryIdentity,
    nonce: RunNonce,
    lease_generation: LeaseGeneration,
    run_id: String,
}

impl RunIdentity {
    pub fn derive(
        repository: RepositoryIdentity,
        nonce: RunNonce,
        lease_generation: LeaseGeneration,
    ) -> Self {
        let frame = format!(
            "{}\0{}\0{}",
            repository.as_str(),
            nonce.as_str(),
            lease_generation.get()
        );
        let run_id = sha256_hex(frame.as_bytes());
        Self {
            repository,
            nonce,
            lease_generation,
            run_id,
        }
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn repository(&self) -> &RepositoryIdentity {
        &self.repository
    }

    pub fn run_nonce(&self) -> &str {
        self.nonce.as_str()
    }

    pub fn lease_generation(&self) -> u64 {
        self.lease_generation.get()
    }

    fn to_value(&self) -> Value {
        json!({
            "repository": self.repository.as_str(),
            "run_nonce": self.nonce.as_str(),
            "lease_generation": self.lease_generation.get(),
            "run_id": self.run_id,
        })
    }

    fn from_value(value: &Value) -> Result<Self, AccountabilityError> {
        let object = object(value, "run identity")?;
        let repository = RepositoryIdentity::parse(string(object, "repository")?)?;
        let nonce = RunNonce::parse(string(object, "run_nonce")?)?;
        let lease_generation = LeaseGeneration::new(unsigned(object, "lease_generation")?)?;
        let identity = Self::derive(repository, nonce, lease_generation);
        if string(object, "run_id")? != identity.run_id {
            return Err(AccountabilityError::new("run identity digest mismatch"));
        }
        Ok(identity)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchDescriptor {
    pub identity: RunIdentity,
    pub outcome: String,
    pub why: String,
}

impl LaunchDescriptor {
    pub fn new(
        identity: RunIdentity,
        outcome: impl Into<String>,
        why: impl Into<String>,
    ) -> Result<Self, AccountabilityError> {
        Ok(Self {
            identity,
            outcome: validate_summary(outcome.into(), "launch outcome")?,
            why: validate_summary(why.into(), "launch rationale")?,
        })
    }

    fn to_value(&self) -> Value {
        json!({"identity": self.identity.to_value(), "outcome": self.outcome, "why": self.why})
    }

    fn from_value(value: &Value) -> Result<Self, AccountabilityError> {
        let object = object(value, "launch")?;
        Self::new(
            RunIdentity::from_value(required(object, "identity")?)?,
            string(object, "outcome")?,
            string(object, "why")?,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EventKind {
    RunStarted,
    WorkSelected { issue: Option<u64> },
    ClaimStarted { issue: u64 },
    IssueClaimed { issue: u64 },
    ImplementationStarted { issue: u64 },
    PullRequestOpened { pull_request: u64 },
    ReviewStarted { pull_request: u64 },
    Verified,
    PullRequestVerified { pull_request: u64 },
    Merged { pull_request: u64 },
    Blocked,
    Failed,
    Quarantined { issue: u64 },
    Parked,
    Stopped,
    Completed,
    ResumedFromEpic { epic: u64 },
}

impl EventKind {
    fn to_value(&self) -> Value {
        match self {
            Self::RunStarted => json!({"type":"run_started"}),
            Self::WorkSelected { issue } => json!({"type":"work_selected","issue":issue}),
            Self::ClaimStarted { issue } => json!({"type":"claim_started","issue":issue}),
            Self::IssueClaimed { issue } => json!({"type":"issue_claimed","issue":issue}),
            Self::ImplementationStarted { issue } => {
                json!({"type":"implementation_started","issue":issue})
            }
            Self::PullRequestOpened { pull_request } => {
                json!({"type":"pull_request_opened","pull_request":pull_request})
            }
            Self::Verified => json!({"type":"verified"}),
            Self::PullRequestVerified { pull_request } => {
                json!({"type":"pull_request_verified","pull_request":pull_request})
            }
            Self::ReviewStarted { pull_request } => {
                json!({"type":"review_started","pull_request":pull_request})
            }
            Self::Merged { pull_request } => json!({"type":"merged","pull_request":pull_request}),
            Self::Blocked => json!({"type":"blocked"}),
            Self::Failed => json!({"type":"failed"}),
            Self::Quarantined { issue } => json!({"type":"quarantined","issue":issue}),
            Self::Parked => json!({"type":"parked"}),
            Self::Stopped => json!({"type":"stopped"}),
            Self::Completed => json!({"type":"completed"}),
            Self::ResumedFromEpic { epic } => json!({"type":"resumed_from_epic","epic":epic}),
        }
    }

    fn from_value(value: &Value) -> Result<Self, AccountabilityError> {
        let object = object(value, "event kind")?;
        Ok(match string(object, "type")? {
            "run_started" => Self::RunStarted,
            "work_selected" => Self::WorkSelected {
                issue: object.get("issue").and_then(Value::as_u64),
            },
            "claim_started" => Self::ClaimStarted {
                issue: unsigned(object, "issue")?,
            },
            "issue_claimed" => Self::IssueClaimed {
                issue: unsigned(object, "issue")?,
            },
            "implementation_started" => Self::ImplementationStarted {
                issue: unsigned(object, "issue")?,
            },
            "pull_request_opened" => Self::PullRequestOpened {
                pull_request: unsigned(object, "pull_request")?,
            },
            "verified" => Self::Verified,
            "pull_request_verified" => Self::PullRequestVerified {
                pull_request: unsigned(object, "pull_request")?,
            },
            "review_started" => Self::ReviewStarted {
                pull_request: unsigned(object, "pull_request")?,
            },
            "merged" => Self::Merged {
                pull_request: unsigned(object, "pull_request")?,
            },
            "blocked" => Self::Blocked,
            "failed" => Self::Failed,
            "quarantined" => Self::Quarantined {
                issue: unsigned(object, "issue")?,
            },
            "parked" => Self::Parked,
            "stopped" => Self::Stopped,
            "completed" => Self::Completed,
            "resumed_from_epic" => Self::ResumedFromEpic {
                epic: unsigned(object, "epic")?,
            },
            _ => {
                return Err(AccountabilityError::new(
                    "unknown accountability event kind",
                ))
            }
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Evidence {
    Outcome(String),
    RepositoryPath(String),
    GithubUrl(String),
    Command {
        executable: String,
        arguments: Vec<String>,
    },
}

impl Evidence {
    pub fn outcome(value: impl Into<String>) -> Self {
        Self::Outcome(sanitize_text(&value.into(), 240))
    }

    pub fn repository_path(value: &str) -> Result<Self, AccountabilityError> {
        if value.is_empty()
            || value.starts_with('/')
            || value.starts_with('~')
            || value.starts_with("\\\\")
            || value.as_bytes().get(1) == Some(&b':')
            || value.split('/').any(|part| matches!(part, "" | "." | ".."))
            || value.chars().any(char::is_control)
        {
            return Err(AccountabilityError::new(
                "evidence path must be repository-relative",
            ));
        }
        Ok(Self::RepositoryPath(sanitize_text(value, 240)))
    }

    pub fn github_url(value: impl AsRef<str>) -> Result<Self, AccountabilityError> {
        let value = value.as_ref();
        let without_suffix = value.split(['?', '#']).next().unwrap_or(value);
        let rest = without_suffix
            .strip_prefix("https://")
            .ok_or_else(|| AccountabilityError::new("evidence URL must use HTTPS"))?;
        let safe_authority = rest.rsplit_once('@').map_or(rest, |(_, safe)| safe);
        if safe_authority != "github.com" && !safe_authority.starts_with("github.com/") {
            return Err(AccountabilityError::new("evidence URL must use github.com"));
        }
        if !safe_authority
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.'))
        {
            return Err(AccountabilityError::new(
                "evidence URL contains unsafe characters",
            ));
        }
        Ok(Self::GithubUrl(format!("https://{safe_authority}")))
    }

    pub fn command<I, S>(executable: &str, arguments: I) -> Result<Self, AccountabilityError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        const EXECUTABLES: &[&str] = &["autospec", "bash", "cargo", "git", "shellcheck"];
        const ARGUMENTS: &[&str] = &[
            "build",
            "check",
            "clippy",
            "test",
            "--all-targets",
            "--release",
            "--workspace",
        ];
        if !EXECUTABLES.contains(&executable) {
            return Err(AccountabilityError::new(
                "command executable is not allowlisted",
            ));
        }
        Ok(Self::Command {
            executable: executable.to_owned(),
            arguments: arguments
                .into_iter()
                .map(|argument| {
                    let argument = argument.as_ref();
                    if ARGUMENTS.contains(&argument) {
                        argument.to_owned()
                    } else {
                        "[redacted]".to_owned()
                    }
                })
                .collect(),
        })
    }

    fn to_value(&self) -> Value {
        match self {
            Self::Outcome(value) => json!({"type":"outcome","value":value}),
            Self::RepositoryPath(value) => json!({"type":"repository_path","value":value}),
            Self::GithubUrl(value) => json!({"type":"github_url","value":value}),
            Self::Command {
                executable,
                arguments,
            } => {
                json!({"type":"command","executable":executable,"arguments":arguments})
            }
        }
    }

    fn from_value(value: &Value) -> Result<Self, AccountabilityError> {
        let object = object(value, "evidence")?;
        match string(object, "type")? {
            "outcome" => Ok(Self::outcome(string(object, "value")?)),
            "repository_path" => Self::repository_path(string(object, "value")?),
            "github_url" => Self::github_url(string(object, "value")?),
            "command" => Self::command(
                string(object, "executable")?,
                required(object, "arguments")?
                    .as_array()
                    .ok_or_else(|| AccountabilityError::new("command arguments must be an array"))?
                    .iter()
                    .map(|item| item.as_str().unwrap_or("[redacted]")),
            ),
            _ => Err(AccountabilityError::new("unknown evidence kind")),
        }
    }

    fn display(&self) -> String {
        match self {
            Self::Outcome(value) | Self::RepositoryPath(value) => value.clone(),
            Self::GithubUrl(value) => value.clone(),
            Self::Command {
                executable,
                arguments,
            } => format!("{} {}", executable, arguments.join(" "))
                .trim()
                .to_owned(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountabilityEvent {
    pub kind: EventKind,
    pub what: String,
    pub why: String,
    pub evidence: Vec<Evidence>,
}

impl AccountabilityEvent {
    pub fn new(
        kind: EventKind,
        what: impl Into<String>,
        why: impl Into<String>,
        evidence: Vec<Evidence>,
    ) -> Result<Self, AccountabilityError> {
        if evidence.is_empty() || evidence.len() > 8 {
            return Err(AccountabilityError::new("event evidence is mandatory"));
        }
        Ok(Self {
            kind,
            what: validate_summary(what.into(), "event what")?,
            why: validate_summary(why.into(), "event why")?,
            evidence,
        })
    }

    fn to_value(&self) -> Value {
        json!({
            "kind":self.kind.to_value(), "what":self.what, "why":self.why,
            "evidence":self.evidence.iter().map(Evidence::to_value).collect::<Vec<_>>()
        })
    }

    fn from_value(value: &Value) -> Result<Self, AccountabilityError> {
        let object = object(value, "event")?;
        let evidence = required(object, "evidence")?
            .as_array()
            .ok_or_else(|| AccountabilityError::new("event evidence must be an array"))?
            .iter()
            .map(Evidence::from_value)
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(
            EventKind::from_value(required(object, "kind")?)?,
            string(object, "what")?,
            string(object, "why")?,
            evidence,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventRecord {
    pub seq: u64,
    pub event_id: String,
    pub segment_chain_digest: String,
    pub kind: EventKind,
    pub event: AccountabilityEvent,
}

impl EventRecord {
    fn create(
        run_id: &str,
        segment_chain_digest: &str,
        seq: u64,
        event: AccountabilityEvent,
    ) -> Self {
        let canonical = serde_json::to_string(&event.to_value()).expect("JSON value serializes");
        let event_id =
            sha256_hex(format!("{run_id}\0{segment_chain_digest}\0{seq}\0{canonical}").as_bytes());
        Self {
            seq,
            event_id,
            segment_chain_digest: segment_chain_digest.to_owned(),
            kind: event.kind.clone(),
            event,
        }
    }

    fn to_value(&self) -> Value {
        json!({"seq":self.seq,"event_id":self.event_id,"segment_chain_digest":self.segment_chain_digest,"event":self.event.to_value()})
    }

    fn from_value(
        run_id: &str,
        expected_chain: &str,
        value: &Value,
    ) -> Result<Self, AccountabilityError> {
        let object = object(value, "event record")?;
        let seq = unsigned(object, "seq")?;
        let event = AccountabilityEvent::from_value(required(object, "event")?)?;
        let chain = string(object, "segment_chain_digest")?;
        if chain != expected_chain {
            return Err(AccountabilityError::new(
                "event journal segment chain mismatch",
            ));
        }
        let record = Self::create(run_id, chain, seq, event);
        if string(object, "event_id")? != record.event_id {
            return Err(AccountabilityError::new("event ID digest mismatch"));
        }
        Ok(record)
    }
}

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
}

impl AccountabilityError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            projection_disposition: None,
        }
    }

    fn projection(message: impl Into<String>, disposition: ProjectionDisposition) -> Self {
        Self {
            message: message.into(),
            projection_disposition: Some(disposition),
        }
    }

    pub fn projection_disposition(&self) -> Option<ProjectionDisposition> {
        self.projection_disposition
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
