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

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

static TEMP_SERIAL: AtomicU64 = AtomicU64::new(1);

const STATE_FILE: &str = "accountability.json";
const EVENTS_FILE: &str = "accountability-events.jsonl";
const OUTBOX_FILE: &str = "accountability-outbox.jsonl";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryManifest {
    pub identity: RunIdentity,
    pub epic_number: u64,
    pub epic_url: String,
    pub projection_revision: u64,
    pub remote_digest: String,
    pub high_watermark: u64,
    pub journal_segment: u64,
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
        let epic_url = match Evidence::github_url(epic_url)? {
            Evidence::GithubUrl(url) => url,
            _ => unreachable!(),
        };
        Ok(Self {
            identity,
            epic_number,
            epic_url,
            projection_revision,
            remote_digest: remote_digest.to_ascii_lowercase(),
            high_watermark,
            journal_segment,
        })
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(&json!({
            "schema":ACCOUNTABILITY_SCHEMA, "identity":self.identity.to_value(),
            "epic_number":self.epic_number, "epic_url":self.epic_url,
            "projection_revision":self.projection_revision, "remote_digest":self.remote_digest,
            "high_watermark":self.high_watermark, "journal_segment":self.journal_segment,
        }))
        .expect("JSON value serializes")
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
        Self::new(
            RunIdentity::from_value(super::required(object, "identity")?)?,
            super::unsigned(object, "epic_number")?,
            super::string(object, "epic_url")?,
            super::unsigned(object, "projection_revision")?,
            super::string(object, "remote_digest")?,
            super::unsigned(object, "high_watermark")?,
            super::unsigned(object, "journal_segment")?,
        )
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
            parse_state(&fs::read_to_string(root.join(STATE_FILE)).map_err(io_error)?)?
        } else {
            State::default()
        };
        let events = recover_events(&root.join(EVENTS_FILE), state.launch.as_ref())?;
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
        self.state.journal_segment = 1;
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
        self.state.journal_segment = manifest.journal_segment + 1;
        self.state.prior_remote_digest = Some(manifest.remote_digest);
        ensure_private_file(&self.path(EVENTS_FILE))?;
        ensure_private_file(&self.path(OUTBOX_FILE))?;
        self.persist_state()?;
        self.append_event(AccountabilityEvent::new(
            EventKind::ResumedFromEpic {
                epic: manifest.epic_number,
            },
            format!("Resumed accountability from epic {}", manifest.epic_number),
            "The managed recovery manifest reconstructed a missing local journal segment",
            vec![Evidence::github_url(manifest.epic_url)?],
        )?)
    }

    pub fn append_event(
        &mut self,
        event: AccountabilityEvent,
    ) -> Result<EventRecord, AccountabilityError> {
        let launch =
            self.state.launch.as_ref().ok_or_else(|| {
                AccountabilityError::new("begin_launch is required before events")
            })?;
        let seq = self.state.last_seq + 1;
        let record = EventRecord::create(launch.identity.run_id(), seq, event);
        append_synced_line(&self.path(EVENTS_FILE), &record.to_value())?;
        self.events.push(record.clone());
        self.state.last_seq = seq;
        self.state.event_count += 1;
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
        let revision = self.state.projection_revision + 1;
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
        atomic_write(&self.path(OUTBOX_FILE), b"")?;
        self.state.acknowledged_high_watermark = high_watermark;
        self.state.pending_projection_count = 0;
        self.persist_state()
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
        }
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
    })
}

fn recover_events(
    path: &Path,
    launch: Option<&LaunchDescriptor>,
) -> Result<Vec<EventRecord>, AccountabilityError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(io_error)?;
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
    let mut prior = 0;
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let value: Value = serde_json::from_slice(line).map_err(|error| {
            AccountabilityError::new(format!("invalid completed event line: {error}"))
        })?;
        let record = EventRecord::from_value(launch.identity.run_id(), &value)?;
        if record.seq != prior + 1 && prior != 0 {
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
        fs::create_dir_all(path).map_err(io_error)?;
    }
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
    Ok(())
}

fn ensure_private_file(path: &Path) -> Result<(), AccountabilityError> {
    reject_unsafe_file(path)?;
    private_options()
        .create(true)
        .append(true)
        .open(path)
        .map_err(io_error)?;
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
    options.mode(0o600);
    options
}

fn io_error(error: std::io::Error) -> AccountabilityError {
    AccountabilityError::new(error.to_string())
}
