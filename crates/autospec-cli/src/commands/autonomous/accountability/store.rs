use super::render;
use super::{
    AccountabilityError, AccountabilityEvent, EventKind, EventRecord, Evidence, LaunchDescriptor,
    RenderedProjection, RepositoryIdentity, RunIdentity, ACCOUNTABILITY_SCHEMA,
};
use autospec_core::autonomous::waterfall::sha256_hex;
use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

static TEMP_SERIAL: AtomicU64 = AtomicU64::new(1);

const STATE_FILE: &str = "accountability.json";
const EVENTS_FILE: &str = "accountability-events.jsonl";
const OUTBOX_FILE: &str = "accountability-outbox.jsonl";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RecoveryState {
    #[default]
    Active,
    Parked,
    Terminal,
}

impl RecoveryState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Parked => "parked",
            Self::Terminal => "terminal",
        }
    }

    fn parse(value: &str) -> Result<Self, AccountabilityError> {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountabilityStatus {
    pub run_id: Option<String>,
    pub epic_number: Option<u64>,
    pub epic_url: Option<String>,
    pub event_count: u64,
    pub pending_projection_count: u64,
    pub projection_revision: u64,
    pub desired_high_watermark: u64,
    pub acknowledged_high_watermark: u64,
    pub journal_segment: u64,
    pub prior_remote_digest: Option<String>,
    pub segment_chain_digest: String,
    pub lifecycle_phase: String,
    pub last_projected_at: Option<u64>,
}

#[derive(Default)]
struct State {
    launch: Option<LaunchDescriptor>,
    epic_number: Option<u64>,
    epic_url: Option<String>,
    event_count: u64,
    last_seq: u64,
    projection_revision: u64,
    desired_digest: Option<String>,
    desired_high_watermark: u64,
    acknowledged_high_watermark: u64,
    pending_projection_count: u64,
    journal_segment: u64,
    prior_remote_digest: Option<String>,
    segment_chain_digest: String,
    create_attempted: bool,
    resume_event_pending: bool,
    lifecycle_phase: String,
    recovery_state: RecoveryState,
    linked_issues: Vec<u64>,
    linked_pull_requests: Vec<u64>,
    last_projected_at: Option<u64>,
}

pub struct AccountabilityStore {
    root: PathBuf,
    state: State,
    events: Vec<EventRecord>,
}

impl AccountabilityStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, AccountabilityError> {
        let root = root.as_ref().to_path_buf();
        ensure_private_directory(&root)?;
        for name in [STATE_FILE, EVENTS_FILE, OUTBOX_FILE] {
            reject_unsafe_file(&root.join(name))?;
        }
        let mut state = if root.join(STATE_FILE).exists() {
            parse_state(&read_private_file(&root.join(STATE_FILE))?)?
        } else {
            State::default()
        };
        enforce_cross_file_invariants(&root, &state)?;
        reconcile_outbox(&root, &mut state)?;
        let segment_base = state.last_seq.saturating_sub(state.event_count);
        let events = recover_events(
            &root.join(EVENTS_FILE),
            state.launch.as_ref(),
            &state.segment_chain_digest,
            segment_base,
        )?;
        if let Some(last) = events.last() {
            state.last_seq = last.seq;
            state.event_count = events.len() as u64;
        } else if state.launch.is_some() {
            state.last_seq = state.acknowledged_high_watermark;
            state.event_count = 0;
        }
        let store = Self {
            root,
            state,
            events,
        };
        if store.state.launch.is_some() {
            store.persist_state()?;
            ensure_private_file(&store.path(EVENTS_FILE))?;
            ensure_private_file(&store.path(OUTBOX_FILE))?;
        }
        Ok(store)
    }

    pub fn begin_launch(&mut self, launch: LaunchDescriptor) -> Result<(), AccountabilityError> {
        if let Some(current) = &self.state.launch {
            if current.identity != launch.identity {
                return Err(AccountabilityError::new(
                    "accountability store already belongs to a different run identity",
                ));
            }
            return Ok(());
        }
        self.state.launch = Some(launch);
        self.state.lifecycle_phase = "bound_not_spawned".to_owned();
        self.state.journal_segment = 1;
        self.state.segment_chain_digest = sha256_hex(
            format!(
                "{}\0segment\01",
                self.state.launch.as_ref().unwrap().identity.run_id()
            )
            .as_bytes(),
        );
        ensure_private_file(&self.path(EVENTS_FILE))?;
        ensure_private_file(&self.path(OUTBOX_FILE))?;
        self.persist_state()
    }

    pub fn resume_from_manifest(
        &mut self,
        manifest: RecoveryManifest,
        outcome: impl Into<String>,
        why: impl Into<String>,
    ) -> Result<EventRecord, AccountabilityError> {
        if self.state.launch.is_some() || !self.events.is_empty() {
            return Err(AccountabilityError::new(
                "remote recovery requires an empty local accountability store",
            ));
        }
        let launch = LaunchDescriptor::new(manifest.identity.clone(), outcome, why)?;
        self.state.launch = Some(launch);
        self.state.epic_number = Some(manifest.epic_number);
        self.state.epic_url = Some(manifest.epic_url.clone());
        self.state.projection_revision = manifest.projection_revision;
        self.state.desired_digest = Some(manifest.remote_digest.clone());
        self.state.desired_high_watermark = manifest.high_watermark;
        self.state.acknowledged_high_watermark = manifest.high_watermark;
        self.state.last_seq = manifest.high_watermark;
        self.state.journal_segment = manifest
            .journal_segment
            .checked_add(1)
            .ok_or_else(|| AccountabilityError::new("journal segment overflow"))?;
        self.state.prior_remote_digest = Some(manifest.remote_digest.clone());
        self.state.segment_chain_digest = sha256_hex(
            format!(
                "{}\0{}\0{}\0{}",
                manifest.identity.run_id(),
                self.state.prior_remote_digest.as_deref().unwrap(),
                manifest.high_watermark,
                self.state.journal_segment
            )
            .as_bytes(),
        );
        self.state.create_attempted = true;
        self.state.resume_event_pending = true;
        self.state.lifecycle_phase = "bound_not_spawned".to_owned();
        self.state.recovery_state = RecoveryState::Active;
        self.state.linked_issues = manifest.linked_issues.clone();
        self.state.linked_pull_requests = manifest.linked_pull_requests.clone();
        ensure_private_file(&self.path(EVENTS_FILE))?;
        ensure_private_file(&self.path(OUTBOX_FILE))?;
        self.persist_state()?;
        let record = self.append_event(AccountabilityEvent::new(
            EventKind::ResumedFromEpic {
                epic: manifest.epic_number,
            },
            format!("Resumed accountability from epic {}", manifest.epic_number),
            "The managed recovery manifest reconstructed a missing local journal segment",
            vec![Evidence::github_url(manifest.epic_url)?],
        )?)?;
        self.state.resume_event_pending = false;
        self.persist_state()?;
        Ok(record)
    }

    pub fn ensure_resume_event(&mut self) -> Result<(), AccountabilityError> {
        if !self.state.resume_event_pending {
            return Ok(());
        }
        let epic = self
            .state
            .epic_number
            .ok_or_else(|| AccountabilityError::new("pending resume event has no bound epic"))?;
        if !self.events.iter().any(|record| {
            matches!(record.kind, EventKind::ResumedFromEpic { epic: found } if found == epic)
        }) {
            let url = self.state.epic_url.clone().ok_or_else(|| {
                AccountabilityError::new("pending resume event has no epic URL")
            })?;
            self.append_event(AccountabilityEvent::new(
                EventKind::ResumedFromEpic { epic },
                format!("Resumed accountability from epic {epic}"),
                "The managed recovery manifest reconstructed a missing local journal segment",
                vec![Evidence::github_url(url)?],
            )?)?;
        }
        self.state.resume_event_pending = false;
        self.persist_state()
    }

    pub fn append_event(
        &mut self,
        event: AccountabilityEvent,
    ) -> Result<EventRecord, AccountabilityError> {
        let launch =
            self.state.launch.as_ref().ok_or_else(|| {
                AccountabilityError::new("begin_launch is required before events")
            })?;
        let seq = self
            .state
            .last_seq
            .checked_add(1)
            .ok_or_else(|| AccountabilityError::new("event sequence overflow"))?;
        let terminal = matches!(&event.kind, EventKind::Completed | EventKind::Stopped);
        match &event.kind {
            EventKind::WorkSelected { issue: Some(issue) }
            | EventKind::ClaimStarted { issue }
            | EventKind::IssueClaimed { issue }
            | EventKind::ImplementationStarted { issue }
            | EventKind::Quarantined { issue } => {
                insert_link(&mut self.state.linked_issues, *issue)
            }
            EventKind::PullRequestOpened { pull_request }
            | EventKind::PullRequestVerified { pull_request }
            | EventKind::ReviewStarted { pull_request }
            | EventKind::Merged { pull_request } => {
                insert_link(&mut self.state.linked_pull_requests, *pull_request)
            }
            EventKind::Parked => self.state.recovery_state = RecoveryState::Parked,
            EventKind::Completed | EventKind::Stopped => {
                self.state.recovery_state = RecoveryState::Terminal
            }
            EventKind::ResumedFromEpic { .. } => self.state.recovery_state = RecoveryState::Active,
            _ => {}
        }
        let record = EventRecord::create(
            launch.identity.run_id(),
            &self.state.segment_chain_digest,
            seq,
            event,
        );
        append_synced_line(&self.path(EVENTS_FILE), &record.to_value())?;
        self.events.push(record.clone());
        self.state.last_seq = seq;
        self.state.event_count += 1;
        if terminal {
            self.state.lifecycle_phase = "terminal".to_owned();
        }
        self.persist_state()?;
        Ok(record)
    }

    pub fn render(&mut self) -> Result<RenderedProjection, AccountabilityError> {
        let launch =
            self.state.launch.as_ref().ok_or_else(|| {
                AccountabilityError::new("begin_launch is required before render")
            })?;
        let markdown = render::markdown(launch, &self.events);
        if markdown.len() > render::MAX_MARKDOWN_BYTES {
            return Err(AccountabilityError::new(
                "rendered projection exceeds 48 KiB",
            ));
        }
        let digest = sha256_hex(markdown.as_bytes());
        let revision = self
            .state
            .projection_revision
            .checked_add(1)
            .ok_or_else(|| AccountabilityError::new("projection revision overflow"))?;
        let desired_high_watermark = self.state.last_seq;
        let projection = RenderedProjection {
            revision,
            digest: digest.clone(),
            desired_high_watermark,
            markdown,
        };
        atomic_write(
            &self.path(OUTBOX_FILE),
            format!(
                "{}\n",
                serde_json::to_string(&json!({
                    "revision": revision, "digest": digest,
                    "desired_high_watermark": desired_high_watermark
                }))
                .expect("JSON value serializes")
            )
            .as_bytes(),
        )?;
        self.state.projection_revision = revision;
        self.state.desired_digest = Some(projection.digest.clone());
        self.state.desired_high_watermark = desired_high_watermark;
        self.state.pending_projection_count = 1;
        self.persist_state()?;
        Ok(projection)
    }

    pub fn projection_for_delivery(&mut self) -> Result<RenderedProjection, AccountabilityError> {
        if self.state.pending_projection_count == 0 {
            return self.render();
        }
        let launch = self.state.launch.as_ref().ok_or_else(|| {
            AccountabilityError::new("begin_launch is required before projection delivery")
        })?;
        let markdown = render::markdown(launch, &self.events);
        let digest = sha256_hex(markdown.as_bytes());
        if self.state.desired_digest.as_deref() != Some(&digest)
            || self.state.desired_high_watermark != self.state.last_seq
        {
            return self.render();
        }
        Ok(RenderedProjection {
            revision: self.state.projection_revision,
            digest,
            desired_high_watermark: self.state.desired_high_watermark,
            markdown,
        })
    }

    pub fn ack_projection(
        &mut self,
        revision: u64,
        digest: &str,
        high_watermark: u64,
    ) -> Result<(), AccountabilityError> {
        if revision != self.state.projection_revision
            || self.state.desired_digest.as_deref() != Some(digest)
            || high_watermark != self.state.desired_high_watermark
            || high_watermark < self.state.acknowledged_high_watermark
        {
            return Err(AccountabilityError::new(
                "projection acknowledgment does not match the desired projection",
            ));
        }
        self.state.acknowledged_high_watermark = high_watermark;
        self.state.pending_projection_count = 0;
        self.state.last_projected_at = Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| AccountabilityError::new("system clock precedes Unix epoch"))?
                .as_secs(),
        );
        self.persist_state()?;
        atomic_write(&self.path(OUTBOX_FILE), b"")
    }

    pub fn status(&self) -> AccountabilityStatus {
        AccountabilityStatus {
            run_id: self
                .state
                .launch
                .as_ref()
                .map(|launch| launch.identity.run_id().to_owned()),
            epic_number: self.state.epic_number,
            epic_url: self.state.epic_url.clone(),
            event_count: self.state.event_count,
            pending_projection_count: self.state.pending_projection_count,
            projection_revision: self.state.projection_revision,
            desired_high_watermark: self.state.desired_high_watermark,
            acknowledged_high_watermark: self.state.acknowledged_high_watermark,
            journal_segment: self.state.journal_segment,
            prior_remote_digest: self.state.prior_remote_digest.clone(),
            segment_chain_digest: self.state.segment_chain_digest.clone(),
            lifecycle_phase: self.state.lifecycle_phase.clone(),
            last_projected_at: self.state.last_projected_at,
        }
    }

    pub fn identity(&self) -> Option<&RunIdentity> {
        self.state.launch.as_ref().map(|launch| &launch.identity)
    }

    pub fn has_event(&self, kind: &EventKind) -> bool {
        self.events.iter().any(|record| &record.kind == kind)
    }

    pub fn create_attempted(&self) -> bool {
        self.state.create_attempted
    }

    pub fn desired_projection_digest(&self) -> Option<&str> {
        self.state.desired_digest.as_deref()
    }

    pub fn recovery_projection(&self) -> (RecoveryState, Vec<u64>, Vec<u64>) {
        (
            self.state.recovery_state,
            self.state.linked_issues.clone(),
            self.state.linked_pull_requests.clone(),
        )
    }

    pub fn mark_create_attempted(&mut self) -> Result<(), AccountabilityError> {
        if self.state.launch.is_none() {
            return Err(AccountabilityError::new(
                "begin_launch is required before remote creation",
            ));
        }
        self.state.create_attempted = true;
        self.persist_state()
    }

    pub fn mark_spawned(&mut self) -> Result<(), AccountabilityError> {
        if self.state.launch.is_none() || self.state.epic_number.is_none() {
            return Err(AccountabilityError::new(
                "a bound accountability epic is required before spawn",
            ));
        }
        if self.state.lifecycle_phase == "terminal" {
            return Err(AccountabilityError::new(
                "terminal accountability state cannot be spawned",
            ));
        }
        self.state.lifecycle_phase = "spawned".to_owned();
        self.persist_state()
    }

    pub fn bind_epic(
        &mut self,
        epic_number: u64,
        epic_url: impl AsRef<str>,
    ) -> Result<(), AccountabilityError> {
        if epic_number == 0 || self.state.launch.is_none() {
            return Err(AccountabilityError::new(
                "a positive epic and launch identity are required",
            ));
        }
        let epic_url = match Evidence::github_url(epic_url)? {
            Evidence::GithubUrl(url) => url,
            _ => unreachable!(),
        };
        if let Some(existing) = self.state.epic_number {
            if existing != epic_number || self.state.epic_url.as_deref() != Some(&epic_url) {
                return Err(AccountabilityError::new(
                    "accountability store is already bound to a different epic",
                ));
            }
            return Ok(());
        }
        self.state.epic_number = Some(epic_number);
        self.state.epic_url = Some(epic_url);
        self.persist_state()
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn persist_state(&self) -> Result<(), AccountabilityError> {
        let launch = self.state.launch.as_ref().map(LaunchDescriptor::to_value);
        let document = serde_json::to_vec(&json!({
            "schema":ACCOUNTABILITY_SCHEMA, "launch":launch,
            "epic_number":self.state.epic_number, "epic_url":self.state.epic_url,
            "event_count":self.state.event_count, "last_seq":self.state.last_seq,
            "projection_revision":self.state.projection_revision,
            "desired_digest":self.state.desired_digest,
            "desired_high_watermark":self.state.desired_high_watermark,
            "acknowledged_high_watermark":self.state.acknowledged_high_watermark,
            "pending_projection_count":self.state.pending_projection_count,
            "journal_segment":self.state.journal_segment,
            "prior_remote_digest":self.state.prior_remote_digest,
            "segment_chain_digest":self.state.segment_chain_digest,
            "create_attempted":self.state.create_attempted,
            "resume_event_pending":self.state.resume_event_pending,
            "lifecycle_phase":self.state.lifecycle_phase,
            "recovery_state":self.state.recovery_state.as_str(),
            "linked_issues":self.state.linked_issues,
            "linked_pull_requests":self.state.linked_pull_requests,
            "last_projected_at":self.state.last_projected_at,
        }))
        .expect("JSON value serializes");
        atomic_write(&self.path(STATE_FILE), &document)
    }
}

fn parse_state(document: &str) -> Result<State, AccountabilityError> {
    let value: Value = serde_json::from_str(document).map_err(|error| {
        AccountabilityError::new(format!("invalid accountability state: {error}"))
    })?;
    let object = value
        .as_object()
        .ok_or_else(|| AccountabilityError::new("accountability state must be an object"))?;
    let number = |name: &str| {
        object
            .get(name)
            .and_then(Value::as_u64)
            .ok_or_else(|| AccountabilityError::new(format!("invalid state field {name}")))
    };
    if number("schema")? != ACCOUNTABILITY_SCHEMA {
        return Err(AccountabilityError::new(
            "unsupported accountability state schema",
        ));
    }
    let launch = match object.get("launch") {
        Some(Value::Null) | None => None,
        Some(value) => Some(LaunchDescriptor::from_value(value)?),
    };
    let optional_string = |name: &str| object.get(name).and_then(Value::as_str).map(str::to_owned);
    let has_launch = launch.is_some();
    Ok(State {
        launch,
        epic_number: object.get("epic_number").and_then(Value::as_u64),
        epic_url: optional_string("epic_url"),
        event_count: number("event_count")?,
        last_seq: number("last_seq")?,
        projection_revision: number("projection_revision")?,
        desired_digest: optional_string("desired_digest"),
        desired_high_watermark: number("desired_high_watermark")?,
        acknowledged_high_watermark: number("acknowledged_high_watermark")?,
        pending_projection_count: number("pending_projection_count")?,
        journal_segment: number("journal_segment")?,
        prior_remote_digest: optional_string("prior_remote_digest"),
        segment_chain_digest: optional_string("segment_chain_digest").unwrap_or_default(),
        create_attempted: object
            .get("create_attempted")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        resume_event_pending: object
            .get("resume_event_pending")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        lifecycle_phase: optional_string("lifecycle_phase").unwrap_or_else(|| {
            if has_launch {
                "bound_not_spawned".to_owned()
            } else {
                String::new()
            }
        }),
        recovery_state: object
            .get("recovery_state")
            .and_then(Value::as_str)
            .map(RecoveryState::parse)
            .transpose()?
            .unwrap_or_default(),
        linked_issues: object
            .get("linked_issues")
            .map(parse_links)
            .transpose()?
            .unwrap_or_default(),
        linked_pull_requests: object
            .get("linked_pull_requests")
            .map(parse_links)
            .transpose()?
            .unwrap_or_default(),
        last_projected_at: object.get("last_projected_at").and_then(Value::as_u64),
    })
}

fn insert_link(links: &mut Vec<u64>, value: u64) {
    if let Err(index) = links.binary_search(&value) {
        links.insert(index, value);
    }
}

fn validate_links(values: &[u64]) -> Result<(), AccountabilityError> {
    if values.contains(&0) || values.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(AccountabilityError::new(
            "linked identifiers must be positive, sorted, and unique",
        ));
    }
    Ok(())
}

fn parse_links(value: &Value) -> Result<Vec<u64>, AccountabilityError> {
    let values = value
        .as_array()
        .ok_or_else(|| AccountabilityError::new("linked identifiers must be an array"))?
        .iter()
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| AccountabilityError::new("linked identifier must be unsigned"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate_links(&values)?;
    Ok(values)
}

fn validate_epic_url(
    identity: &RunIdentity,
    epic_number: u64,
    value: &str,
) -> Result<String, AccountabilityError> {
    let expected = format!(
        "https://github.com/{}/issues/{epic_number}",
        identity.repository().as_str()
    );
    if value != expected {
        return Err(AccountabilityError::new(
            "epic URL must exactly match the manifest repository and issue number",
        ));
    }
    Ok(expected)
}

fn enforce_cross_file_invariants(root: &Path, state: &State) -> Result<(), AccountabilityError> {
    if state.launch.is_some() {
        if !root.join(EVENTS_FILE).is_file() || !root.join(OUTBOX_FILE).is_file() {
            return Err(AccountabilityError::new(
                "accountability metadata requires both journal and outbox files",
            ));
        }
        if state.journal_segment == 0 || state.segment_chain_digest.len() != 64 {
            return Err(AccountabilityError::new(
                "invalid journal segment chain metadata",
            ));
        }
    }
    Ok(())
}

fn reconcile_outbox(root: &Path, state: &mut State) -> Result<(), AccountabilityError> {
    if state.launch.is_none() {
        return Ok(());
    }
    let path = root.join(OUTBOX_FILE);
    let document = read_private_file(&path)?;
    if document.is_empty() {
        if state.pending_projection_count > 0 {
            return Err(AccountabilityError::new(
                "pending projection metadata has no durable outbox record",
            ));
        }
        return Ok(());
    }
    let value: Value = serde_json::from_str(document.trim_end())
        .map_err(|error| AccountabilityError::new(format!("invalid projection outbox: {error}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| AccountabilityError::new("projection outbox must be an object"))?;
    let revision = super::unsigned(object, "revision")?;
    let digest = super::string(object, "digest")?;
    let high_watermark = super::unsigned(object, "desired_high_watermark")?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AccountabilityError::new(
            "projection outbox digest is invalid",
        ));
    }
    if state.pending_projection_count == 0
        && state.projection_revision == revision
        && state.acknowledged_high_watermark >= high_watermark
    {
        atomic_write(&path, b"")?;
    } else if revision > state.projection_revision && high_watermark >= state.desired_high_watermark
    {
        state.projection_revision = revision;
        state.desired_digest = Some(digest.to_owned());
        state.desired_high_watermark = high_watermark;
        state.pending_projection_count = 1;
    } else if state.pending_projection_count != 1
        || state.projection_revision != revision
        || state.desired_digest.as_deref() != Some(digest)
        || state.desired_high_watermark != high_watermark
    {
        return Err(AccountabilityError::new(
            "projection outbox and metadata disagree",
        ));
    }
    Ok(())
}

fn recover_events(
    path: &Path,
    launch: Option<&LaunchDescriptor>,
    segment_chain_digest: &str,
    segment_base: u64,
) -> Result<Vec<EventRecord>, AccountabilityError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut file = private_options()
        .read(true)
        .write(true)
        .open(path)
        .map_err(io_error)?;
    validate_open_file(&file)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(io_error)?;
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        let complete = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        file.set_len(complete as u64).map_err(io_error)?;
        file.seek(SeekFrom::Start(complete as u64))
            .map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        bytes.truncate(complete);
    }
    let Some(launch) = launch else {
        if bytes.is_empty() {
            return Ok(Vec::new());
        }
        return Err(AccountabilityError::new(
            "event journal exists without launch identity",
        ));
    };
    let mut records = Vec::new();
    let mut prior = segment_base;
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let value: Value = serde_json::from_slice(line).map_err(|error| {
            AccountabilityError::new(format!("invalid completed event line: {error}"))
        })?;
        let record =
            EventRecord::from_value(launch.identity.run_id(), segment_chain_digest, &value)?;
        if record.seq
            != prior
                .checked_add(1)
                .ok_or_else(|| AccountabilityError::new("event sequence overflow"))?
        {
            return Err(AccountabilityError::new(
                "event journal sequence is not monotonic",
            ));
        }
        prior = record.seq;
        records.push(record);
    }
    Ok(records)
}

fn append_synced_line(path: &Path, value: &Value) -> Result<(), AccountabilityError> {
    reject_unsafe_file(path)?;
    let mut file = private_options()
        .append(true)
        .create(true)
        .open(path)
        .map_err(io_error)?;
    validate_open_file(&file)?;
    serde_json::to_writer(&mut file, value)
        .map_err(|error| AccountabilityError::new(error.to_string()))?;
    file.write_all(b"\n").map_err(io_error)?;
    file.sync_all().map_err(io_error)
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), AccountabilityError> {
    reject_unsafe_file(path)?;
    let serial = TEMP_SERIAL.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_extension(format!("tmp-{}-{serial}", std::process::id()));
    let mut file = private_options()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(io_error)?;
    validate_open_file(&file)?;
    if let Err(error) = file.write_all(contents).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(io_error(error));
    }
    fs::rename(&temporary, path).map_err(io_error)?;
    File::open(path.parent().expect("state file has parent"))
        .and_then(|directory| directory.sync_all())
        .map_err(io_error)
}

fn ensure_private_directory(path: &Path) -> Result<(), AccountabilityError> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(AccountabilityError::new(
                "accountability root is not a safe directory",
            ));
        }
        validate_owner(&metadata)?;
        #[cfg(unix)]
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(AccountabilityError::new(
                "accountability root permissions must be private",
            ));
        }
    } else {
        #[cfg(unix)]
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .map_err(io_error)?;
        #[cfg(not(unix))]
        fs::create_dir_all(path).map_err(io_error)?;
    }
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
    Ok(())
}

fn ensure_private_file(path: &Path) -> Result<(), AccountabilityError> {
    reject_unsafe_file(path)?;
    let file = private_options()
        .create(true)
        .append(true)
        .open(path)
        .map_err(io_error)?;
    validate_open_file(&file)?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(io_error)?;
    Ok(())
}

fn reject_unsafe_file(path: &Path) -> Result<(), AccountabilityError> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(AccountabilityError::new(
                "accountability file is not a safe regular file",
            ));
        }
        validate_owner(&metadata)?;
        #[cfg(unix)]
        if metadata.permissions().mode() & 0o777 != 0o600 {
            return Err(AccountabilityError::new(
                "accountability file permissions must be 0600",
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn validate_owner(metadata: &fs::Metadata) -> Result<(), AccountabilityError> {
    if metadata.uid() != nix::unistd::geteuid().as_raw() {
        return Err(AccountabilityError::new(
            "accountability state ownership mismatch",
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_owner(_metadata: &fs::Metadata) -> Result<(), AccountabilityError> {
    Ok(())
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    options
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC);
    options
}

fn read_private_file(path: &Path) -> Result<String, AccountabilityError> {
    let mut file = private_options().read(true).open(path).map_err(io_error)?;
    validate_open_file(&file)?;
    let mut document = String::new();
    file.read_to_string(&mut document).map_err(io_error)?;
    Ok(document)
}

fn validate_open_file(file: &File) -> Result<(), AccountabilityError> {
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() {
        return Err(AccountabilityError::new(
            "accountability descriptor is not a regular file",
        ));
    }
    validate_owner(&metadata)?;
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o777 != 0o600 {
        return Err(AccountabilityError::new(
            "accountability descriptor permissions must be 0600",
        ));
    }
    Ok(())
}

fn io_error(error: std::io::Error) -> AccountabilityError {
    AccountabilityError::new(error.to_string())
}
