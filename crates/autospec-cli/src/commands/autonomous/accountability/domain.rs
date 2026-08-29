use super::*;

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

    pub(super) fn to_value(&self) -> Value {
        json!({
            "repository": self.repository.as_str(),
            "run_nonce": self.nonce.as_str(),
            "lease_generation": self.lease_generation.get(),
            "run_id": self.run_id,
        })
    }

    pub(super) fn from_value(value: &Value) -> Result<Self, AccountabilityError> {
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

    pub(super) fn to_value(&self) -> Value {
        json!({"identity": self.identity.to_value(), "outcome": self.outcome, "why": self.why})
    }

    pub(super) fn from_value(value: &Value) -> Result<Self, AccountabilityError> {
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
    WorkSelected {
        issue: Option<u64>,
    },
    ClaimStarted {
        issue: u64,
    },
    IssueClaimed {
        issue: u64,
    },
    HeartbeatPublicationDeferred {
        issue: u64,
        claim_id: String,
    },
    StartupClaimRecovered {
        issue: u64,
        previous_claim_id: String,
        next_claim_id: String,
    },
    ImplementationStarted {
        issue: u64,
    },
    PullRequestOpened {
        pull_request: u64,
    },
    ReviewStarted {
        pull_request: u64,
    },
    Verified,
    PullRequestVerified {
        pull_request: u64,
    },
    Merged {
        pull_request: u64,
    },
    Blocked,
    Failed,
    Quarantined {
        issue: u64,
    },
    Parked,
    Stopped,
    Completed,
    ResumedFromEpic {
        epic: u64,
    },
}

impl EventKind {
    fn to_value(&self) -> Value {
        match self {
            Self::RunStarted => json!({"type":"run_started"}),
            Self::WorkSelected { issue } => json!({"type":"work_selected","issue":issue}),
            Self::ClaimStarted { issue } => json!({"type":"claim_started","issue":issue}),
            Self::IssueClaimed { issue } => json!({"type":"issue_claimed","issue":issue}),
            Self::HeartbeatPublicationDeferred { issue, claim_id } => json!({
                "type":"heartbeat_publication_deferred",
                "issue":issue,
                "claim_id":claim_id,
            }),
            Self::StartupClaimRecovered {
                issue,
                previous_claim_id,
                next_claim_id,
            } => json!({
                "type":"startup_claim_recovered",
                "issue":issue,
                "previous_claim_id":previous_claim_id,
                "next_claim_id":next_claim_id,
            }),
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
            "heartbeat_publication_deferred" => Self::HeartbeatPublicationDeferred {
                issue: unsigned(object, "issue")?,
                claim_id: string(object, "claim_id")?.to_owned(),
            },
            "startup_claim_recovered" => Self::StartupClaimRecovered {
                issue: unsigned(object, "issue")?,
                previous_claim_id: string(object, "previous_claim_id")?.to_owned(),
                next_claim_id: string(object, "next_claim_id")?.to_owned(),
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

    pub(super) fn display(&self) -> String {
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
    pub created_at: u64,
    pub updated_at: u64,
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
        let created_at = timestamp_now()?;
        Ok(Self {
            kind,
            what: validate_summary(what.into(), "event what")?,
            why: validate_summary(why.into(), "event why")?,
            evidence,
            created_at,
            updated_at: created_at,
        })
    }

    fn to_value(&self) -> Value {
        let mut value = json!({
            "kind":self.kind.to_value(), "what":self.what, "why":self.why,
            "evidence":self.evidence.iter().map(Evidence::to_value).collect::<Vec<_>>()
        });
        if self.created_at > 0 {
            value["created_at"] = json!(self.created_at);
            value["updated_at"] = json!(self.updated_at);
        }
        value
    }

    fn from_value(value: &Value) -> Result<Self, AccountabilityError> {
        let object = object(value, "event")?;
        let evidence = required(object, "evidence")?
            .as_array()
            .ok_or_else(|| AccountabilityError::new("event evidence must be an array"))?
            .iter()
            .map(Evidence::from_value)
            .collect::<Result<Vec<_>, _>>()?;
        let mut event = Self::new(
            EventKind::from_value(required(object, "kind")?)?,
            string(object, "what")?,
            string(object, "why")?,
            evidence,
        )?;
        event.created_at = object
            .get("created_at")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        event.updated_at = object
            .get("updated_at")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if (event.created_at == 0) != (event.updated_at == 0) || event.updated_at < event.created_at
        {
            return Err(AccountabilityError::new(
                "event timestamps are missing or out of order",
            ));
        }
        Ok(event)
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
    pub(super) fn create(
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

    pub(super) fn to_value(&self) -> Value {
        json!({"seq":self.seq,"event_id":self.event_id,"segment_chain_digest":self.segment_chain_digest,"event":self.event.to_value()})
    }

    pub(super) fn from_value(
        run_id: &str,
        expected_chain: &str,
        value: &Value,
    ) -> Result<Self, AccountabilityError> {
        let record = Self::from_journal_value(run_id, value)?;
        if record.segment_chain_digest != expected_chain {
            return Err(AccountabilityError::new(
                "event journal segment chain mismatch",
            ));
        }
        Ok(record)
    }

    pub(super) fn from_journal_value(
        run_id: &str,
        value: &Value,
    ) -> Result<Self, AccountabilityError> {
        let object = object(value, "event record")?;
        let seq = unsigned(object, "seq")?;
        let event = AccountabilityEvent::from_value(required(object, "event")?)?;
        let chain = string(object, "segment_chain_digest")?;
        let record = Self::create(run_id, chain, seq, event);
        if string(object, "event_id")? != record.event_id {
            return Err(AccountabilityError::new("event ID digest mismatch"));
        }
        Ok(record)
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
