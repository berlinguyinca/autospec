use super::render;
use super::{
    object, required, string, unsigned, AccountabilityError, AccountabilityEvent, EventKind,
    EventRecord, Evidence, LaunchDescriptor, RenderedProjection, RepositoryIdentity, RunIdentity,
    ACCOUNTABILITY_SCHEMA,
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

#[path = "store/fs.rs"]
mod fs_support;
#[path = "store/journal.rs"]
mod journal;
#[path = "store/manifest.rs"]
mod manifest;
#[path = "store/retry.rs"]
mod retry;
use fs_support::*;
use journal::*;
pub use manifest::{RecoveryManifest, RecoveryState};
use retry::unix_timestamp;
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
    pub next_projection_retry_at: Option<u64>,
    pub recovery_state: RecoveryState,
    pub accountability_state: String,
    pub created_at: u64,
    pub updated_at: u64,
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
    next_projection_retry_at: Option<u64>,
    projection_retry_attempt: u32,
    created_at: u64,
    updated_at: u64,
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
        if state.launch.is_some() && state.created_at == 0 {
            let now = unix_timestamp()?;
            state.created_at = now;
            state.updated_at = now;
        }
        if state.updated_at < state.created_at {
            return Err(AccountabilityError::new(
                "accountability state updated_at precedes created_at",
            ));
        }
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
        let mut store = Self {
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

    pub fn resume_bound_from_manifest(
        &mut self,
        manifest: RecoveryManifest,
    ) -> Result<(), AccountabilityError> {
        let identity = self.identity().cloned().ok_or_else(|| {
            AccountabilityError::new("local resume requires an existing run identity")
        })?;
        if identity.run_id() != manifest.identity.run_id()
            || self.state.epic_number != Some(manifest.epic_number)
            || self.state.epic_url.as_deref() != Some(&manifest.epic_url)
        {
            return Err(AccountabilityError::new(
                "local accountability state does not own the selected epic",
            ));
        }
        if manifest.recovery_state == RecoveryState::Active {
            return self.ensure_resume_event();
        }
        let journal_segment = manifest
            .journal_segment
            .checked_add(1)
            .ok_or_else(|| AccountabilityError::new("journal segment overflow"))?;
        atomic_write(&self.path(EVENTS_FILE), b"")?;
        atomic_write(&self.path(OUTBOX_FILE), b"")?;
        self.events.clear();
        self.state.event_count = 0;
        self.state.last_seq = manifest.high_watermark;
        self.state.projection_revision = manifest.projection_revision;
        self.state.desired_digest = Some(manifest.remote_digest.clone());
        self.state.desired_high_watermark = manifest.high_watermark;
        self.state.acknowledged_high_watermark = manifest.high_watermark;
        self.state.pending_projection_count = 0;
        self.state.journal_segment = journal_segment;
        self.state.prior_remote_digest = Some(manifest.remote_digest.clone());
        self.state.segment_chain_digest = sha256_hex(
            format!(
                "{}\0{}\0{}\0{}",
                identity.run_id(),
                manifest.remote_digest,
                manifest.high_watermark,
                journal_segment
            )
            .as_bytes(),
        );
        self.state.create_attempted = true;
        self.state.resume_event_pending = true;
        self.state.lifecycle_phase = "bound_not_spawned".to_owned();
        self.state.recovery_state = RecoveryState::Active;
        self.state.linked_issues = manifest.linked_issues;
        self.state.linked_pull_requests = manifest.linked_pull_requests;
        self.persist_state()?;
        self.ensure_resume_event()
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
        self.state.next_projection_retry_at = None;
        self.state.projection_retry_attempt = 0;
        self.persist_state()?;
        atomic_write(&self.path(OUTBOX_FILE), b"")
    }

    pub fn status(&self) -> AccountabilityStatus {
        let accountability_state = match self.state.recovery_state {
            RecoveryState::Parked => "parked".to_owned(),
            RecoveryState::Terminal => "terminal".to_owned(),
            RecoveryState::Active => self.state.lifecycle_phase.clone(),
        };
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
            next_projection_retry_at: self.state.next_projection_retry_at,
            recovery_state: self.state.recovery_state,
            accountability_state,
            created_at: self.state.created_at,
            updated_at: self.state.updated_at,
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

    fn persist_state(&mut self) -> Result<(), AccountabilityError> {
        let now = unix_timestamp()?;
        if self.state.created_at == 0 {
            self.state.created_at = now;
        }
        self.state.updated_at = now.max(self.state.created_at);
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
            "next_projection_retry_at":self.state.next_projection_retry_at,
            "projection_retry_attempt":self.state.projection_retry_attempt,
            "created_at":self.state.created_at,
            "updated_at":self.state.updated_at,
        }))
        .expect("JSON value serializes");
        atomic_write(&self.path(STATE_FILE), &document)
    }
}
