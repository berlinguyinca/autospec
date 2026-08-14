use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RecoveryState {
    #[default]
    Active,
    Parked,
    Terminal,
}

impl RecoveryState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Parked => "parked",
            Self::Terminal => "terminal",
        }
    }

    pub(super) fn parse(value: &str) -> Result<Self, AccountabilityError> {
        match value {
            "active" => Ok(Self::Active),
            "parked" => Ok(Self::Parked),
            "terminal" => Ok(Self::Terminal),
            _ => Err(AccountabilityError::new("invalid recovery state")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryManifest {
    pub identity: RunIdentity,
    pub epic_number: u64,
    pub epic_url: String,
    pub projection_revision: u64,
    pub remote_digest: String,
    pub high_watermark: u64,
    pub journal_segment: u64,
    pub recovery_state: RecoveryState,
    pub linked_issues: Vec<u64>,
    pub linked_pull_requests: Vec<u64>,
}

impl RecoveryManifest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        identity: RunIdentity,
        epic_number: u64,
        epic_url: impl AsRef<str>,
        projection_revision: u64,
        remote_digest: impl Into<String>,
        high_watermark: u64,
        journal_segment: u64,
    ) -> Result<Self, AccountabilityError> {
        let remote_digest = remote_digest.into();
        if epic_number == 0 || projection_revision == 0 || journal_segment == 0 {
            return Err(AccountabilityError::new(
                "recovery manifest counters must be positive",
            ));
        }
        if remote_digest.len() != 64 || !remote_digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(AccountabilityError::new(
                "recovery manifest digest must be SHA-256",
            ));
        }
        let epic_url = validate_epic_url(&identity, epic_number, epic_url.as_ref())?;
        Ok(Self {
            identity,
            epic_number,
            epic_url,
            projection_revision,
            remote_digest: remote_digest.to_ascii_lowercase(),
            high_watermark,
            journal_segment,
            recovery_state: RecoveryState::Active,
            linked_issues: Vec::new(),
            linked_pull_requests: Vec::new(),
        })
    }

    pub fn with_recovery_state(
        mut self,
        recovery_state: RecoveryState,
        linked_issues: Vec<u64>,
        linked_pull_requests: Vec<u64>,
    ) -> Result<Self, AccountabilityError> {
        validate_links(&linked_issues)?;
        validate_links(&linked_pull_requests)?;
        self.recovery_state = recovery_state;
        self.linked_issues = linked_issues;
        self.linked_pull_requests = linked_pull_requests;
        Ok(self)
    }

    fn unsigned_value(&self) -> Value {
        json!({
            "schema":ACCOUNTABILITY_SCHEMA, "identity":self.identity.to_value(),
            "epic_number":self.epic_number, "epic_url":self.epic_url,
            "projection_revision":self.projection_revision, "remote_digest":self.remote_digest,
            "high_watermark":self.high_watermark, "journal_segment":self.journal_segment,
            "recovery_state":self.recovery_state.as_str(), "linked_issues":self.linked_issues,
            "linked_pull_requests":self.linked_pull_requests,
        })
    }

    pub fn to_json(&self) -> String {
        let value = self.unsigned_value();
        let digest = sha256_hex(
            serde_json::to_string(&value)
                .expect("JSON value serializes")
                .as_bytes(),
        );
        let mut object = value.as_object().expect("manifest is object").clone();
        object.insert("manifest_digest".to_owned(), json!(digest));
        serde_json::to_string(&object).expect("JSON value serializes")
    }

    pub fn parse(document: &str) -> Result<Self, AccountabilityError> {
        let value: Value = serde_json::from_str(document).map_err(|error| {
            AccountabilityError::new(format!("invalid recovery manifest: {error}"))
        })?;
        let object = super::object(&value, "recovery manifest")?;
        if super::unsigned(object, "schema")? != ACCOUNTABILITY_SCHEMA {
            return Err(AccountabilityError::new(
                "unsupported recovery manifest schema",
            ));
        }
        let manifest = Self::new(
            RunIdentity::from_value(super::required(object, "identity")?)?,
            super::unsigned(object, "epic_number")?,
            super::string(object, "epic_url")?,
            super::unsigned(object, "projection_revision")?,
            super::string(object, "remote_digest")?,
            super::unsigned(object, "high_watermark")?,
            super::unsigned(object, "journal_segment")?,
        )?
        .with_recovery_state(
            RecoveryState::parse(super::string(object, "recovery_state")?)?,
            parse_links(super::required(object, "linked_issues")?)?,
            parse_links(super::required(object, "linked_pull_requests")?)?,
        )?;
        let expected = sha256_hex(
            serde_json::to_string(&manifest.unsigned_value())
                .expect("JSON value serializes")
                .as_bytes(),
        );
        if super::string(object, "manifest_digest")? != expected {
            return Err(AccountabilityError::new(
                "recovery manifest integrity digest mismatch",
            ));
        }
        Ok(manifest)
    }

    pub fn parse_for_repository(
        document: &str,
        repository: &RepositoryIdentity,
    ) -> Result<Self, AccountabilityError> {
        let manifest = Self::parse(document)?;
        if manifest.identity.repository() != repository {
            return Err(AccountabilityError::new(
                "recovery manifest repository mismatch",
            ));
        }
        Ok(manifest)
    }
}
