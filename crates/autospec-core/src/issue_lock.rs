//! Issue-level repository locks (issue #3575).
//!
//! An issue can declare, in its body, that it needs the repository (or part of
//! it) to itself:
//!
//! ```text
//! lock: exclusive          # no other issue may be dispatched against this repo
//! lock: paths: [ "crates/example-app/src/commands/autonomous/**" ]
//! ```
//!
//! Guarantees this module enforces:
//!
//! * **Queuing, not rejection** — dispatch of an issue that collides with a
//!   held lock is queued and reports what it waits on
//!   (`waiting on #N since HH:MM`), never silently dropped.
//! * **Expiry** — every lock carries an expiry (the issue's own time limit
//!   plus a margin). An expired lock releases automatically and is recorded
//!   as an *expiry*, never as a completion.
//! * **Dead-holder detection** — the holder renews a liveness timestamp with
//!   the same session-liveness signal used for stall detection (issue #3563);
//!   a lock whose holder stopped renewing is released early.
//! * **Maximum duration** — past the maximum total duration, continuing
//!   requires an explicit renewal with a recorded, non-empty reason.
//! * **No cycles** — locks are acquired only at dispatch time, in one place
//!   ([`IssueLockManager::dispatch`]); a running agent that requests a second
//!   lock gets [`LockError::SecondLock`] and cannot extend its hold
//!   incrementally, so a wait-for cycle is structurally impossible.
//! * **Forced release** — a human or the supervisor can break a lock with a
//!   mandatory reason; the break is recorded, the holder is notified via
//!   [`LockRecord::holder_notice`], and a broken lock is distinguishable from
//!   a clean release in the run record ([`LockRelease`]).
//!
//! Platform independence: lock state is one file per lock
//! (`lock-<issue>.json`), created atomically (claim via `create_new`, content
//! published via temp file + `rename`), under `.autospec/locks/` in the state
//! root. No advisory locking (`flock`) is used anywhere, so the state is
//! durable across process restarts and correct on shared network storage.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Default issue time limit (seconds) used when the caller does not supply one.
pub const DEFAULT_ISSUE_TIME_LIMIT_SECS: u64 = 4 * 3600;
/// Margin added to the issue time limit to form the hard expiry.
pub const LOCK_EXPIRY_MARGIN_SECS: u64 = 30 * 60;
/// How long a holder may go without a liveness renewal before the lock is
/// released early as a dead holder.
pub const DEFAULT_LIVENESS_TTL_SECS: u64 = 15 * 60;
/// Maximum total hold before an explicit renewal with a recorded reason is
/// required ("a refactor that has held the repo for eight hours should have to
/// justify the ninth").
pub const DEFAULT_MAX_DURATION_SECS: u64 = 8 * 3600;

fn locks_directory(root: &Path) -> PathBuf {
    root.join(".autospec").join("locks")
}

/// What an issue asks to lock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LockScope {
    /// The whole repository: no other issue may be dispatched.
    Exclusive,
    /// A set of repo-relative path patterns (`**` suffix means "and
    /// everything under it").
    Paths { paths: Vec<String> },
}

/// A parsed `lock:` declaration from an issue body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockDeclaration {
    pub scope: LockScope,
    pub reason: Option<String>,
}

impl LockDeclaration {
    fn exclusive(reason: Option<String>) -> Self {
        Self {
            scope: LockScope::Exclusive,
            reason,
        }
    }
}

/// Parse the first `lock:` declaration in an issue body.
///
/// Returns `Ok(None)` when the body declares no lock. Fenced code blocks are
/// ignored so example YAML does not accidentally grant a lock.
pub fn parse_lock_declaration(body: &str) -> Result<Option<LockDeclaration>, String> {
    let mut in_fence = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("lock:") else {
            continue;
        };
        return parse_lock_value(rest).map(Some);
    }
    Ok(None)
}

fn parse_lock_value(rest: &str) -> Result<LockDeclaration, String> {
    let (value_part, reason) = split_reason(rest);
    let value = value_part.trim();
    if value.is_empty() {
        return Err(
            "lock declaration is missing a scope (`exclusive` or `paths: [...]`)".to_string(),
        );
    }
    if let Some(exclusive) = value.strip_prefix("exclusive") {
        if !exclusive.trim().is_empty() {
            return Err(format!("unrecognized lock scope: {value}"));
        }
        return Ok(LockDeclaration::exclusive(reason));
    }
    if let Some(after_paths) = value.strip_prefix("paths") {
        let inner = after_paths
            .trim_start()
            .strip_prefix(':')
            .ok_or_else(|| "lock: paths must be `paths: [ ... ]`".to_string())?
            .trim();
        let (start, end) = inner
            .find('[')
            .and_then(|start| {
                inner[start..]
                    .find(']')
                    .map(|offset| (start, start + offset))
            })
            .ok_or_else(|| "lock: paths must be a bracketed list: `paths: [ ... ]`".to_string())?;
        let items = inner[start + 1..end]
            .split(',')
            .map(|item| item.trim().trim_matches('"').trim_matches('\'').trim())
            .filter(|item| !item.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if items.is_empty() {
            return Err("lock: paths must list at least one path".to_string());
        }
        for item in &items {
            if item.starts_with('/') || item.contains("..") {
                return Err(format!("lock path must be repo-relative and safe: {item}"));
            }
        }
        if !inner[end + 1..].trim().is_empty() {
            return Err(format!("unrecognized lock scope: {value}"));
        }
        return Ok(LockDeclaration {
            scope: LockScope::Paths { paths: items },
            reason,
        });
    }
    Err(format!("unrecognized lock scope: {value}"))
}

/// Split an optional trailing `# reason: ...` comment off a declaration.
fn split_reason(rest: &str) -> (&str, Option<String>) {
    let marker = "# reason:";
    match rest.find(marker) {
        Some(index) => {
            let reason = rest[index + marker.len()..].trim().to_string();
            (&rest[..index], (!reason.is_empty()).then_some(reason))
        }
        None => (rest, None),
    }
}

/// Do two scopes collide (would two issues holding them produce work that
/// cannot merge)? `exclusive` collides with everything. Two `paths` scopes
/// collide when one pattern's root is the other's root or an ancestor of it —
/// deliberately conservative: it over-queues rather than under-queues.
pub fn scopes_collide(a: &LockScope, b: &LockScope) -> bool {
    match (a, b) {
        (LockScope::Exclusive, _) | (_, LockScope::Exclusive) => true,
        (LockScope::Paths { paths: a }, LockScope::Paths { paths: b }) => a.iter().any(|left| {
            b.iter().any(|right| {
                let (rl, rr) = (path_root(left), path_root(right));
                rl == rr || rr.starts_with(&format!("{rl}/")) || rl.starts_with(&format!("{rr}/"))
            })
        }),
    }
}

/// The root a pattern locks: `a/**` locks `a`, `a/b.rs` locks `a/b.rs`.
fn path_root(pattern: &str) -> &str {
    let trimmed = pattern.trim();
    trimmed
        .strip_suffix("/**")
        .unwrap_or(trimmed)
        .trim_end_matches('/')
}

/// One durable lock record. One file per lock; the release kind is part of
/// the record so a broken lock never looks like a clean release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockRecord {
    pub issue: u64,
    pub scope: LockScope,
    pub reason: String,
    pub acquired_at: u64,
    pub expires_at: u64,
    pub max_duration_secs: u64,
    /// Seconds of maximum duration granted by recorded renewals.
    pub max_duration_extended_secs: u64,
    pub last_liveness_at: u64,
    pub liveness_ttl_secs: u64,
    pub renewals: Vec<LockRenewal>,
    pub release: Option<LockRelease>,
    /// Set when a supervisor or human breaks the lock; the holder reads this
    /// so a broken lock cannot be reported as a clean completion.
    pub holder_notice: Option<String>,
}

impl LockRecord {
    pub fn is_active(&self) -> bool {
        self.release.is_none()
    }

    pub fn is_forced(&self) -> bool {
        matches!(self.release, Some(LockRelease::Forced { .. }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockRenewal {
    pub at: u64,
    pub reason: String,
    pub added_secs: u64,
}

/// How a lock left the "held" state. `Clean` is a completion; everything else
/// is recorded distinctly so the run record cannot conflate them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LockRelease {
    Clean {
        at: u64,
    },
    Expired {
        at: u64,
        cause: ExpiryCause,
    },
    DeadHolder {
        at: u64,
        last_liveness_at: u64,
    },
    Forced {
        at: u64,
        actor: String,
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpiryCause {
    /// The issue's own time limit plus margin ran out.
    TimeLimit,
    /// The maximum total duration ran out without a recorded renewal.
    MaxDuration,
}

/// The run record: append-only, one JSON line per event, one file per lock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LockEvent {
    Acquired {
        at: u64,
    },
    Liveness {
        at: u64,
    },
    Renewed {
        at: u64,
        reason: String,
        added_secs: u64,
    },
    Queued {
        at: u64,
        issue: u64,
    },
    Dequeued {
        at: u64,
        issue: u64,
    },
    ReleasedClean {
        at: u64,
    },
    ReleasedExpired {
        at: u64,
        cause: ExpiryCause,
    },
    ReleasedDeadHolder {
        at: u64,
    },
    ForcedBreak {
        at: u64,
        actor: String,
        reason: String,
    },
}

impl LockEvent {
    fn to_json_line(&self) -> String {
        format!(
            "{}\n",
            serde_json::to_string(self).expect("lock events are serializable")
        )
    }
}

/// An issue parked because a colliding lock was held. `since` is when the
/// issue first entered the queue, so the wait is reported, never silent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueEntry {
    pub issue: u64,
    pub waiting_on: u64,
    pub since: u64,
}

/// The only two outcomes of a dispatch-time admission check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchDecision {
    Admit,
    Queued { waiting_on: u64, since: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockError {
    /// The issue already holds an active lock. Locks are acquired only at
    /// dispatch; a second request mid-run is refused so no cycle is possible.
    SecondLock {
        issue: u64,
    },
    /// A held lock collides with the requested scope.
    Collides {
        held_by: u64,
        issue: u64,
    },
    /// The issue does not hold an active lock.
    NotHeld {
        issue: u64,
    },
    /// A renewal was requested without the mandatory recorded reason.
    MissingRenewalReason,
    Parse {
        message: String,
    },
    Io {
        operation: String,
        path: String,
        source: String,
    },
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SecondLock { issue } => write!(
                f,
                "issue #{issue} already holds an active lock; a second lock cannot be requested mid-run"
            ),
            Self::Collides { held_by, issue } => {
                write!(f, "issue #{issue} collides with the lock held by #{held_by}")
            }
            Self::NotHeld { issue } => write!(f, "issue #{issue} does not hold an active lock"),
            Self::MissingRenewalReason => {
                write!(
                    f,
                    "a lock renewal past the maximum duration requires a recorded reason"
                )
            }
            Self::Parse { message } => write!(f, "lock parse error: {message}"),
            Self::Io {
                operation,
                path,
                source,
            } => write!(f, "{operation} {path}: {source}"),
        }
    }
}

impl std::error::Error for LockError {}

fn scope_name(scope: &LockScope) -> String {
    match scope {
        LockScope::Exclusive => "exclusive".to_string(),
        LockScope::Paths { paths } => format!("paths: {}", paths.join(", ")),
    }
}

/// Format an epoch-seconds timestamp as `HH:MM` UTC (deterministic and
/// platform-independent).
pub fn format_hhmm_utc(epoch: u64) -> String {
    let secs = epoch % 86_400;
    format!("{:02}:{:02}", secs / 3_600, (secs % 3_600) / 60)
}

/// Write `content` to `dir/name` atomically: temp file in the same directory,
/// then `rename` over the target.
fn write_json_atomic(dir: &Path, name: &str, content: &str) -> Result<(), LockError> {
    let temporary = dir.join(format!(".{name}.tmp"));
    let target = dir.join(name);
    fs::write(&temporary, content)
        .and_then(|()| fs::rename(&temporary, &target))
        .map_err(|error| LockError::Io {
            operation: format!("atomically write {name}"),
            path: target.display().to_string(),
            source: error.to_string(),
        })
}

/// An admitted issue with no lock declaration collides only with an
/// `exclusive` lock (a `paths` lock cannot prove it would touch anything the
/// undeclared issue touches — issue #3564's collision prediction can upgrade
/// this check later). A declared scope collides per [`scopes_collide`].
fn matches_declared_scope(held: &LockScope, declaration: Option<&LockDeclaration>) -> bool {
    match declaration {
        Some(declaration) => scopes_collide(held, &declaration.scope),
        None => matches!(held, LockScope::Exclusive),
    }
}

/// The single point where locks are created, checked, and released.
///
/// State lives in one file per lock under `.autospec/locks/` in `root`. All
/// timestamps are caller-supplied epoch seconds so behaviour is deterministic
/// and testable.
pub struct IssueLockManager {
    locks_dir: PathBuf,
}

impl IssueLockManager {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            locks_dir: locks_directory(root.as_ref()),
        }
    }

    fn lock_path(&self, issue: u64) -> PathBuf {
        self.locks_dir.join(format!("lock-{issue}.json"))
    }

    fn event_path(&self, issue: u64) -> PathBuf {
        self.locks_dir.join(format!("lock-{issue}.events.jsonl"))
    }

    // -- loading -----------------------------------------------------------

    /// Load every lock record (active and released). An empty lock file is a
    /// crashed claim (name claimed, content never published) and is ignored;
    /// a non-empty unreadable file is an error.
    pub fn load(&self) -> Result<Vec<LockRecord>, LockError> {
        let mut records = Vec::new();
        let entries = match fs::read_dir(&self.locks_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(records),
            Err(error) => {
                return Err(LockError::Io {
                    operation: "read lock directory".to_string(),
                    path: self.locks_dir.display().to_string(),
                    source: error.to_string(),
                })
            }
        };
        for entry in entries {
            let entry = entry.map_err(|error| LockError::Io {
                operation: "read lock directory entry".to_string(),
                path: self.locks_dir.display().to_string(),
                source: error.to_string(),
            })?;
            let name = entry.file_name().to_string_lossy().to_string();
            // Lock files are `lock-<issue>.json`; events files (`*.events.jsonl`)
            // and `queue.json` must not be mistaken for records.
            let Some(rest) = name.strip_prefix("lock-") else {
                continue;
            };
            let Some(issue_part) = rest.strip_suffix(".json") else {
                continue;
            };
            if issue_part.is_empty() || issue_part.parse::<u64>().is_err() {
                continue;
            }
            let path = entry.path();
            let document = match fs::read_to_string(&path) {
                Ok(document) => document,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(LockError::Io {
                        operation: "read lock file".to_string(),
                        path: path.display().to_string(),
                        source: error.to_string(),
                    })
                }
            };
            if document.trim().is_empty() {
                // A claim that crashed before publishing content is not a lock.
                continue;
            }
            let record: LockRecord =
                serde_json::from_str(&document).map_err(|error| LockError::Parse {
                    message: format!("{}: {error}", path.display()),
                })?;
            records.push(record);
        }
        records.sort_by_key(|record| record.issue);
        Ok(records)
    }

    pub fn active_locks(&self) -> Result<Vec<LockRecord>, LockError> {
        Ok(self
            .load()?
            .into_iter()
            .filter(|record| record.is_active())
            .collect())
    }

    pub fn record(&self, issue: u64) -> Result<Option<LockRecord>, LockError> {
        Ok(self
            .load()?
            .into_iter()
            .find(|record| record.issue == issue))
    }

    fn require_active(&self, issue: u64) -> Result<LockRecord, LockError> {
        self.record(issue)?
            .filter(|record| record.is_active())
            .ok_or(LockError::NotHeld { issue })
    }

    fn publish_record(&self, record: &LockRecord) -> Result<(), LockError> {
        fs::create_dir_all(&self.locks_dir).map_err(|error| LockError::Io {
            operation: "create lock directory".to_string(),
            path: self.locks_dir.display().to_string(),
            source: error.to_string(),
        })?;
        let document = serde_json::to_string_pretty(record).expect("lock records are serializable");
        write_json_atomic(
            &self.locks_dir,
            &format!("lock-{}.json", record.issue),
            &document,
        )
    }

    fn append_event(&self, issue: u64, event: &LockEvent) -> Result<(), LockError> {
        fs::create_dir_all(&self.locks_dir).map_err(|error| LockError::Io {
            operation: "create lock directory".to_string(),
            path: self.locks_dir.display().to_string(),
            source: error.to_string(),
        })?;
        let path = self.event_path(issue);
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut file| file.write_all(event.to_json_line().as_bytes()))
            .map_err(|error| LockError::Io {
                operation: "append lock event".to_string(),
                path: path.display().to_string(),
                source: error.to_string(),
            })
    }

    // -- dispatch (the only acquisition point) ------------------------------

    /// Dispatch-time admission. The ONLY place a lock is acquired.
    ///
    /// * The current holder re-dispatching is admitted (idempotent); the lock
    ///   is not re-acquired, so a running agent can never request a second
    ///   lock incrementally.
    /// * A dispatch whose declared scope collides with a held lock is queued
    ///   (its first-wait time preserved) and reports what it waits on.
    /// * An admitted dispatch that declares a lock acquires it; an admitted
    ///   dispatch with no declaration simply proceeds.
    ///
    /// `issue_time_limit_secs` sets the expiry: limit + margin.
    pub fn dispatch(
        &self,
        issue: u64,
        declaration: Option<&LockDeclaration>,
        reason: &str,
        now: u64,
        issue_time_limit_secs: u64,
    ) -> Result<DispatchDecision, LockError> {
        let held = self.active_locks()?;
        if held.iter().any(|record| record.issue == issue) {
            // Idempotent re-dispatch of the holder; never a second lock.
            return Ok(DispatchDecision::Admit);
        }
        let collision = held
            .iter()
            .filter(|record| matches_declared_scope(&record.scope, declaration))
            .map(|record| record.issue)
            .min();
        match collision {
            Some(held_by) => {
                let since = self
                    .queue_entry(issue)?
                    .map(|entry| entry.since)
                    .unwrap_or(now);
                self.queue_upsert(QueueEntry {
                    issue,
                    waiting_on: held_by,
                    since,
                })?;
                self.append_event(held_by, &LockEvent::Queued { at: now, issue })?;
                Ok(DispatchDecision::Queued {
                    waiting_on: held_by,
                    since,
                })
            }
            None => {
                if let Some(declaration) = declaration {
                    self.acquire(issue, declaration, reason, now, issue_time_limit_secs)?;
                }
                if let Some(entry) = self.queue_entry(issue)? {
                    self.append_event(entry.waiting_on, &LockEvent::Dequeued { at: now, issue })?;
                }
                self.queue_remove(issue)?;
                Ok(DispatchDecision::Admit)
            }
        }
    }

    /// Acquire a lock. Refused with [`LockError::SecondLock`] when the issue
    /// already holds an active lock, and with [`LockError::Collides`] when a
    /// held lock would produce work that cannot merge.
    pub fn acquire(
        &self,
        issue: u64,
        declaration: &LockDeclaration,
        reason: &str,
        now: u64,
        issue_time_limit_secs: u64,
    ) -> Result<LockRecord, LockError> {
        let held = self.active_locks()?;
        if held.iter().any(|record| record.issue == issue) {
            return Err(LockError::SecondLock { issue });
        }
        if let Some(colliding) = held
            .iter()
            .find(|record| scopes_collide(&record.scope, &declaration.scope))
        {
            return Err(LockError::Collides {
                held_by: colliding.issue,
                issue,
            });
        }
        fs::create_dir_all(&self.locks_dir).map_err(|error| LockError::Io {
            operation: "create lock directory".to_string(),
            path: self.locks_dir.display().to_string(),
            source: error.to_string(),
        })?;
        let path = self.lock_path(issue);
        // Claim the name atomically. Advisory locks (flock) are unreliable on
        // shared network storage, so the claim is `create_new` + rename.
        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
        {
            Ok(mut claim) => {
                // Mark the claim as in-flight, then publish the real content
                // atomically below. An empty file at load time is a crashed
                // claim, not a lock.
                claim
                    .write_all(b"claiming\n")
                    .and_then(|()| claim.sync_all())
                    .map_err(|error| LockError::Io {
                        operation: "claim lock file".to_string(),
                        path: path.display().to_string(),
                        source: error.to_string(),
                    })?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                // A prior record exists for this issue; it must be released
                // (or an empty crashed claim) to be reused.
                if let Some(existing) = fs::read_to_string(&path)
                    .ok()
                    .filter(|document| !document.trim().is_empty())
                    .and_then(|document| serde_json::from_str::<LockRecord>(&document).ok())
                {
                    if existing.is_active() {
                        return Err(LockError::SecondLock { issue });
                    }
                }
            }
            Err(error) => {
                return Err(LockError::Io {
                    operation: "claim lock file".to_string(),
                    path: path.display().to_string(),
                    source: error.to_string(),
                })
            }
        }
        let record = LockRecord {
            issue,
            scope: declaration.scope.clone(),
            reason: declaration
                .reason
                .clone()
                .unwrap_or_else(|| reason.to_string()),
            acquired_at: now,
            expires_at: now
                .saturating_add(issue_time_limit_secs.saturating_add(LOCK_EXPIRY_MARGIN_SECS)),
            max_duration_secs: DEFAULT_MAX_DURATION_SECS,
            max_duration_extended_secs: 0,
            last_liveness_at: now,
            liveness_ttl_secs: DEFAULT_LIVENESS_TTL_SECS,
            renewals: Vec::new(),
            release: None,
            holder_notice: None,
        };
        self.publish_record(&record)?;
        self.append_event(issue, &LockEvent::Acquired { at: now })?;
        Ok(record)
    }

    // -- release paths -------------------------------------------------------

    fn apply_release(
        &self,
        issue: u64,
        _now: u64,
        release: LockRelease,
        event: LockEvent,
        notice: Option<String>,
    ) -> Result<LockRecord, LockError> {
        let mut record = self.require_active(issue)?;
        record.release = Some(release);
        if let Some(notice) = notice {
            record.holder_notice = Some(notice);
        }
        self.publish_record(&record)?;
        self.append_event(issue, &event)?;
        Ok(record)
    }

    /// The holder's completion: a clean release, recorded as such.
    pub fn release_clean(&self, issue: u64, now: u64) -> Result<LockRecord, LockError> {
        self.apply_release(
            issue,
            now,
            LockRelease::Clean { at: now },
            LockEvent::ReleasedClean { at: now },
            None,
        )
    }

    /// A human or the supervisor breaks the lock. The actor and a mandatory
    /// reason are recorded, and the holder is notified so the break cannot be
    /// reported as a clean completion.
    pub fn force_release(
        &self,
        issue: u64,
        actor: &str,
        reason: &str,
        now: u64,
    ) -> Result<LockRecord, LockError> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(LockError::Parse {
                message: "a forced release requires a reason".to_string(),
            });
        }
        let release = LockRelease::Forced {
            at: now,
            actor: actor.to_string(),
            reason: reason.to_string(),
        };
        let event = LockEvent::ForcedBreak {
            at: now,
            actor: actor.to_string(),
            reason: reason.to_string(),
        };
        let notice = Some(format!("lock broken by {actor}: {reason}"));
        self.apply_release(issue, now, release, event, notice)
    }

    /// Past the maximum total duration, continuing requires an explicit
    /// renewal with a stated, recorded reason.
    pub fn renew(
        &self,
        issue: u64,
        reason: &str,
        added_secs: u64,
        now: u64,
    ) -> Result<LockRecord, LockError> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(LockError::MissingRenewalReason);
        }
        if added_secs == 0 {
            return Err(LockError::Parse {
                message: "a lock renewal must add a positive number of seconds".to_string(),
            });
        }
        let mut record = self.require_active(issue)?;
        record.expires_at = record.expires_at.saturating_add(added_secs);
        record.max_duration_extended_secs =
            record.max_duration_extended_secs.saturating_add(added_secs);
        record.renewals.push(LockRenewal {
            at: now,
            reason: reason.to_string(),
            added_secs,
        });
        self.publish_record(&record)?;
        self.append_event(
            issue,
            &LockEvent::Renewed {
                at: now,
                reason: reason.to_string(),
                added_secs,
            },
        )?;
        Ok(record)
    }

    /// The holder's liveness renewal. The timestamp source is the session
    /// liveness signal (stall detection, issue #3563), not a timer the agent
    /// controls.
    pub fn record_liveness(&self, issue: u64, now: u64) -> Result<LockRecord, LockError> {
        let mut record = self.require_active(issue)?;
        record.last_liveness_at = now;
        self.publish_record(&record)?;
        self.append_event(issue, &LockEvent::Liveness { at: now })?;
        Ok(record)
    }

    /// Check every active lock against liveness, expiry, and the maximum
    /// duration. Returns the events that fired; releases are logged
    /// distinctly from completions.
    pub fn tick(&self, now: u64) -> Result<Vec<LockEvent>, LockError> {
        let mut fired = Vec::new();
        for record in self.active_locks()? {
            let issue = record.issue;
            let (release, event) =
                if now.saturating_sub(record.last_liveness_at) > record.liveness_ttl_secs {
                    (
                        LockRelease::DeadHolder {
                            at: now,
                            last_liveness_at: record.last_liveness_at,
                        },
                        LockEvent::ReleasedDeadHolder { at: now },
                    )
                } else if now >= record.expires_at {
                    (
                        LockRelease::Expired {
                            at: now,
                            cause: ExpiryCause::TimeLimit,
                        },
                        LockEvent::ReleasedExpired {
                            at: now,
                            cause: ExpiryCause::TimeLimit,
                        },
                    )
                } else if now
                    > record
                        .acquired_at
                        .saturating_add(record.max_duration_secs)
                        .saturating_add(record.max_duration_extended_secs)
                {
                    (
                        LockRelease::Expired {
                            at: now,
                            cause: ExpiryCause::MaxDuration,
                        },
                        LockEvent::ReleasedExpired {
                            at: now,
                            cause: ExpiryCause::MaxDuration,
                        },
                    )
                } else {
                    continue;
                };
            self.apply_release(issue, now, release.clone(), event.clone(), None)?;
            fired.push(event);
        }
        Ok(fired)
    }

    // -- queue ---------------------------------------------------------------

    pub fn queue(&self) -> Result<Vec<QueueEntry>, LockError> {
        let path = self.locks_dir.join("queue.json");
        let Ok(document) = fs::read_to_string(&path) else {
            return Ok(Vec::new());
        };
        if document.trim().is_empty() {
            return Ok(Vec::new());
        }
        serde_json::from_str(&document).map_err(|error| LockError::Parse {
            message: format!("{}: {error}", path.display()),
        })
    }

    pub fn queue_entry(&self, issue: u64) -> Result<Option<QueueEntry>, LockError> {
        Ok(self.queue()?.into_iter().find(|entry| entry.issue == issue))
    }

    /// "N issues waiting on #N" — queue entries whose holder is still active.
    pub fn waiting_on(&self, held_by: u64) -> Result<Vec<QueueEntry>, LockError> {
        let held = self
            .active_locks()?
            .iter()
            .any(|record| record.issue == held_by);
        Ok(self
            .queue()?
            .into_iter()
            .filter(|entry| entry.waiting_on == held_by && held)
            .collect())
    }

    fn queue_upsert(&self, entry: QueueEntry) -> Result<(), LockError> {
        let mut queue = self.queue()?;
        match queue
            .iter_mut()
            .find(|existing| existing.issue == entry.issue)
        {
            Some(existing) => {
                // Keep the original first-wait time; track the current holder.
                existing.waiting_on = entry.waiting_on;
            }
            None => queue.push(entry),
        }
        queue.sort_by_key(|entry| entry.issue);
        self.queue_write(&queue)
    }

    fn queue_remove(&self, issue: u64) -> Result<(), LockError> {
        let mut queue = self.queue()?;
        queue.retain(|entry| entry.issue != issue);
        self.queue_write(&queue)
    }

    fn queue_write(&self, queue: &[QueueEntry]) -> Result<(), LockError> {
        fs::create_dir_all(&self.locks_dir).map_err(|error| LockError::Io {
            operation: "create lock directory".to_string(),
            path: self.locks_dir.display().to_string(),
            source: error.to_string(),
        })?;
        write_json_atomic(
            &self.locks_dir,
            "queue.json",
            &serde_json::to_string(queue).expect("queue entries are serializable"),
        )
    }

    // -- status ---------------------------------------------------------------

    /// Human-readable status: holder, reason, acquisition time, and the queue.
    pub fn status_lines(&self) -> Result<Vec<String>, LockError> {
        let records = self.load()?;
        let queue = self.queue()?;
        let mut lines = Vec::new();
        for record in records.iter().filter(|record| record.is_active()) {
            lines.push(format!(
                "lock: #{} ({}) reason: {} — acquired {}Z expires {}Z liveness {}Z renewals {} max {}s",
                record.issue,
                scope_name(&record.scope),
                record.reason,
                format_hhmm_utc(record.acquired_at),
                format_hhmm_utc(record.expires_at),
                format_hhmm_utc(record.last_liveness_at),
                record.renewals.len(),
                record
                    .max_duration_secs
                    .saturating_add(record.max_duration_extended_secs)
            ));
            for entry in queue
                .iter()
                .filter(|entry| entry.waiting_on == record.issue)
            {
                lines.push(format!(
                    "  #{} waiting on #{} since {}Z",
                    entry.issue,
                    entry.waiting_on,
                    format_hhmm_utc(entry.since)
                ));
            }
        }
        for entry in queue.iter().filter(|entry| {
            !records
                .iter()
                .any(|record| record.issue == entry.waiting_on && record.is_active())
        }) {
            lines.push(format!(
                "#{} cleared (waited on #{} since {}Z)",
                entry.issue,
                entry.waiting_on,
                format_hhmm_utc(entry.since)
            ));
        }
        Ok(lines)
    }

    /// Machine-readable status for `autospec status --json` output.
    pub fn status_json(&self) -> Result<String, LockError> {
        let records = self.load()?;
        let queue = self.queue()?;
        let active: Vec<&LockRecord> = records.iter().filter(|r| r.is_active()).collect();
        let waiting_counts: BTreeMap<u64, usize> = active
            .iter()
            .map(|record| {
                (
                    record.issue,
                    queue
                        .iter()
                        .filter(|entry| entry.waiting_on == record.issue)
                        .count(),
                )
            })
            .collect();
        let active_lines: Vec<serde_json::Value> = active
            .iter()
            .map(|record| {
                serde_json::json!({
                    "issue": record.issue,
                    "scope": scope_name(&record.scope),
                    "reason": record.reason,
                    "acquired_at": record.acquired_at,
                    "expires_at": record.expires_at,
                    "last_liveness_at": record.last_liveness_at,
                    "renewals": record.renewals.len(),
                    "waiting": waiting_counts[&record.issue],
                })
            })
            .collect();
        let released_lines: Vec<serde_json::Value> = records
            .iter()
            .filter(|record| !record.is_active())
            .map(|record| {
                let kind = match record.release {
                    Some(ref release) => match release {
                        LockRelease::Clean { .. } => "clean",
                        LockRelease::Expired { cause, .. } => match cause {
                            ExpiryCause::TimeLimit => "expired_time_limit",
                            ExpiryCause::MaxDuration => "expired_max_duration",
                        },
                        LockRelease::DeadHolder { .. } => "dead_holder",
                        LockRelease::Forced { .. } => "forced",
                    },
                    None => "unknown",
                };
                let mut value = serde_json::json!({
                    "issue": record.issue,
                    "release": kind,
                    "reason": record.reason,
                    "acquired_at": record.acquired_at,
                });
                if let Some(LockRelease::Forced {
                    actor, reason, at, ..
                }) = record.release.clone()
                {
                    value["broken_by"] = serde_json::json!(actor);
                    value["broken_reason"] = serde_json::json!(reason);
                    value["broken_at"] = serde_json::json!(at);
                }
                value
            })
            .collect();
        let queued_lines: Vec<serde_json::Value> = queue
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "issue": entry.issue,
                    "waiting_on": entry.waiting_on,
                    "since": entry.since,
                })
            })
            .collect();
        Ok(serde_json::to_string(&serde_json::json!({
            "active": active_lines,
            "released": released_lines,
            "queued": queued_lines,
        }))
        .expect("status is serializable"))
    }

    /// The run record for one lock: every event, one JSON line each.
    pub fn events(&self, issue: u64) -> Result<Vec<LockEvent>, LockError> {
        let path = self.event_path(issue);
        let Ok(document) = fs::read_to_string(&path) else {
            return Ok(Vec::new());
        };
        document
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str::<LockEvent>(line).map_err(|error| LockError::Parse {
                    message: format!("{path:?}: {error}"),
                })
            })
            .collect::<Result<_, _>>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "issue-lock-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn exclusive(reason: Option<&str>) -> LockDeclaration {
        LockDeclaration::exclusive(reason.map(str::to_string))
    }

    fn paths(paths: &[&str]) -> LockDeclaration {
        LockDeclaration {
            scope: LockScope::Paths {
                paths: paths.iter().map(|path| (*path).to_string()).collect(),
            },
            reason: None,
        }
    }

    #[test]
    fn parses_exclusive_declaration() {
        let declaration = parse_lock_declaration("body text\nlock: exclusive\nmore text").unwrap();
        assert_eq!(declaration, Some(exclusive(None)));
    }

    #[test]
    fn parses_exclusive_declaration_with_reason() {
        let declaration =
            parse_lock_declaration("lock: exclusive # reason: mass extraction").unwrap();
        let declaration = declaration.expect("a lock is declared");
        assert_eq!(declaration.scope, LockScope::Exclusive);
        assert_eq!(declaration.reason.as_deref(), Some("mass extraction"));
    }

    #[test]
    fn parses_paths_declaration() {
        let body =
            "lock: paths: [ \"crates/example-app/src/commands/autonomous/**\", \"scripts/x.sh\" ]";
        let declaration = parse_lock_declaration(body)
            .unwrap()
            .expect("a lock is declared");
        assert_eq!(
            declaration,
            LockDeclaration {
                scope: LockScope::Paths {
                    paths: vec![
                        "crates/example-app/src/commands/autonomous/**".to_string(),
                        "scripts/x.sh".to_string(),
                    ],
                },
                reason: None,
            }
        );
    }

    #[test]
    fn no_lock_declaration_returns_none() {
        assert_eq!(
            parse_lock_declaration("plain body with no locks").unwrap(),
            None
        );
    }

    #[test]
    fn fenced_code_blocks_do_not_grant_locks() {
        let body = "examples:\n```\nlock: exclusive\n```\nno real lock here";
        assert_eq!(parse_lock_declaration(body).unwrap(), None);
    }

    #[test]
    fn malformed_lock_declarations_are_rejected() {
        assert!(parse_lock_declaration("lock:\n").is_err());
        assert!(parse_lock_declaration("lock: everything").is_err());
        assert!(parse_lock_declaration("lock: paths: ( no list )").is_err());
        assert!(parse_lock_declaration("lock: paths: [ ]").is_err());
        assert!(parse_lock_declaration("lock: paths: [ \"/abs\" ]").is_err());
        assert!(parse_lock_declaration("lock: paths: [ \"a/../b\" ]").is_err());
    }

    #[test]
    fn exclusive_collides_with_everything() {
        assert!(scopes_collide(&LockScope::Exclusive, &LockScope::Exclusive));
        assert!(scopes_collide(
            &LockScope::Exclusive,
            &paths(&["crates/**"]).scope
        ));
    }

    #[test]
    fn overlapping_path_locks_collide() {
        let outer = paths(&["crates/example-app/**"]);
        let inner = paths(&["crates/example-app/src/commands/autonomous/**"]);
        let sibling = paths(&["crates/autospec-core/**"]);
        assert!(scopes_collide(&outer.scope, &inner.scope));
        assert!(scopes_collide(&inner.scope, &outer.scope));
        assert!(scopes_collide(&outer.scope, &outer.scope));
        assert!(!scopes_collide(&outer.scope, &sibling.scope));
    }

    #[test]
    fn dispatch_of_colliding_issue_queues_and_reports_waiter() {
        let root = temp_root();
        let manager = IssueLockManager::new(&root);
        let t0 = 10_000;

        let holder = exclusive(Some("mass extraction of 22k lines"));
        let decision = manager
            .dispatch(100, Some(&holder), "holder", t0, 3600)
            .unwrap();
        assert_eq!(decision, DispatchDecision::Admit);

        let other = exclusive(None);
        let decision = manager
            .dispatch(101, Some(&other), "other", t0 + 60, 3600)
            .unwrap();
        assert_eq!(
            decision,
            DispatchDecision::Queued {
                waiting_on: 100,
                since: t0 + 60
            }
        );

        let status = manager.status_lines().unwrap();
        assert!(
            status
                .iter()
                .any(|line| line.contains("#101 waiting on #100 since")),
            "status must report the wait, got: {status:?}"
        );
        assert_eq!(manager.waiting_on(100).unwrap().len(), 1);

        // The wait time is preserved across repeated queue attempts.
        let decision = manager
            .dispatch(101, Some(&other), "other", t0 + 120, 3600)
            .unwrap();
        assert_eq!(
            decision,
            DispatchDecision::Queued {
                waiting_on: 100,
                since: t0 + 60
            }
        );

        // When the lock clears, the queued issue is admitted and unqueued.
        manager.release_clean(100, t0 + 180).unwrap();
        let decision = manager
            .dispatch(101, Some(&other), "other", t0 + 190, 3600)
            .unwrap();
        assert_eq!(decision, DispatchDecision::Admit);
        assert!(manager.queue_entry(101).unwrap().is_none());
    }

    #[test]
    fn disjoint_path_locks_both_admit() {
        let root = temp_root();
        let manager = IssueLockManager::new(&root);
        let app = paths(&["crates/example-app/**"]);
        let core = paths(&["crates/autospec-core/**"]);
        assert_eq!(
            manager.dispatch(1, Some(&app), "app", 100, 3600).unwrap(),
            DispatchDecision::Admit
        );
        assert_eq!(
            manager.dispatch(2, Some(&core), "core", 200, 3600).unwrap(),
            DispatchDecision::Admit
        );
    }

    #[test]
    fn undeclared_issue_blocked_only_by_exclusive_lock() {
        let root = temp_root();
        let manager = IssueLockManager::new(&root);
        let paths_lock = paths(&["crates/**"]);
        assert_eq!(
            manager
                .dispatch(1, Some(&paths_lock), "holder", 100, 3600)
                .unwrap(),
            DispatchDecision::Admit
        );
        // No declared scope: a paths lock cannot prove a collision.
        assert_eq!(
            manager.dispatch(2, None, "plain", 200, 3600).unwrap(),
            DispatchDecision::Admit
        );

        let root2 = temp_root();
        let manager = IssueLockManager::new(&root2);
        assert_eq!(
            manager
                .dispatch(1, Some(&exclusive(None)), "holder", 100, 3600)
                .unwrap(),
            DispatchDecision::Admit
        );
        let decision = manager.dispatch(2, None, "plain", 200, 3600).unwrap();
        assert!(matches!(
            decision,
            DispatchDecision::Queued { waiting_on: 1, .. }
        ));
    }

    #[test]
    fn holder_redispatch_is_idempotent_not_second_lock() {
        let root = temp_root();
        let manager = IssueLockManager::new(&root);
        let declaration = exclusive(None);
        manager
            .dispatch(7, Some(&declaration), "holder", 100, 3600)
            .unwrap();
        assert_eq!(
            manager
                .dispatch(7, Some(&declaration), "holder", 500, 3600)
                .unwrap(),
            DispatchDecision::Admit
        );
        // Exactly one lock record, acquired at the original time.
        let record = manager.record(7).unwrap().expect("lock record");
        assert_eq!(record.acquired_at, 100);
    }

    #[test]
    fn a_second_lock_during_a_run_is_refused() {
        let root = temp_root();
        let manager = IssueLockManager::new(&root);
        let first = exclusive(None);
        manager
            .dispatch(7, Some(&first), "holder", 100, 3600)
            .unwrap();
        // The running agent requests another lock: refused, no cycle.
        let second = paths(&["scripts/**"]);
        let error = manager
            .acquire(7, &second, "mid-run request", 200, 3600)
            .unwrap_err();
        assert!(
            matches!(error, LockError::SecondLock { issue: 7 }),
            "got: {error}"
        );
        // A colliding different issue is refused too, not admitted.
        let error = manager
            .acquire(8, &exclusive(None), "other", 250, 3600)
            .unwrap_err();
        assert!(
            matches!(
                error,
                LockError::Collides {
                    held_by: 7,
                    issue: 8
                }
            ),
            "got: {error}"
        );
        // The original lock is untouched.
        let record = manager.record(7).unwrap().expect("lock record");
        assert!(record.is_active());
        assert_eq!(record.scope, LockScope::Exclusive);
    }

    #[test]
    fn expired_lock_releases_and_is_not_a_completion() {
        let root = temp_root();
        let manager = IssueLockManager::new(&root);
        let t0 = 100_000;
        // time limit 3600s + 1800s margin => expires at t0 + 5400
        manager
            .dispatch(5, Some(&exclusive(None)), "holder", t0, 3600)
            .unwrap();
        assert!(manager.record(5).unwrap().expect("lock record").is_active());
        // The holder is alive (renewed near the end), so expiry — not
        // dead-holder detection — is what fires.
        manager.record_liveness(5, t0 + 5000).unwrap();
        assert_eq!(manager.tick(t0 + 5399).unwrap(), Vec::<LockEvent>::new());

        let fired = manager.tick(t0 + 5400).unwrap();
        assert_eq!(fired.len(), 1);
        assert!(matches!(
            fired[0],
            LockEvent::ReleasedExpired {
                cause: ExpiryCause::TimeLimit,
                ..
            }
        ));
        let record = manager.record(5).unwrap().expect("lock record");
        assert!(!record.is_active());
        match record.release {
            Some(LockRelease::Expired { cause, .. }) => {
                assert_eq!(cause, ExpiryCause::TimeLimit)
            }
            other => panic!("expected an expired release, got: {other:?}"),
        }
        // A colliding dispatch is now admitted: the queue drains.
        manager
            .dispatch(6, Some(&exclusive(None)), "other", t0 + 5401, 3600)
            .unwrap();
        assert!(manager.active_locks().unwrap().iter().any(|r| r.issue == 6));
    }

    #[test]
    fn dead_holder_lock_released_via_liveness() {
        let root = temp_root();
        let manager = IssueLockManager::new(&root);
        let t0 = 100_000;
        manager
            .dispatch(5, Some(&exclusive(None)), "holder", t0, 3600)
            .unwrap();
        // Holder renews liveness at t0 + 300.
        manager.record_liveness(5, t0 + 300).unwrap();
        // t0 + 300 + 900 = t0 + 1200; TTL is 900, strictly greater fires.
        assert_eq!(manager.tick(t0 + 1200).unwrap(), Vec::<LockEvent>::new());
        let fired = manager.tick(t0 + 1201).unwrap();
        assert_eq!(fired.len(), 1);
        assert!(matches!(fired[0], LockEvent::ReleasedDeadHolder { .. }));
        let record = manager.record(5).unwrap().expect("lock record");
        assert!(matches!(
            record.release,
            Some(LockRelease::DeadHolder {
                last_liveness_at,
                ..
            }) if last_liveness_at == t0 + 300
        ));
    }

    #[test]
    fn max_duration_released_without_renewal() {
        let root = temp_root();
        let manager = IssueLockManager::new(&root);
        let t0 = 100_000;
        // Issue time limit 8h keeps the hard expiry (8h + margin) past the
        // 8h max duration, so max-duration is what fires.
        manager
            .dispatch(5, Some(&exclusive(None)), "holder", t0, 28_800)
            .unwrap();
        // Liveness keeps renewing; only the max duration (8h) matters.
        manager
            .record_liveness(5, t0 + DEFAULT_MAX_DURATION_SECS)
            .unwrap();
        let fired = manager.tick(t0 + DEFAULT_MAX_DURATION_SECS + 1).unwrap();
        assert_eq!(fired.len(), 1);
        assert!(matches!(
            fired[0],
            LockEvent::ReleasedExpired {
                cause: ExpiryCause::MaxDuration,
                ..
            }
        ));
    }

    #[test]
    fn renewal_past_max_duration_requires_recorded_reason() {
        let root = temp_root();
        let manager = IssueLockManager::new(&root);
        let t0 = 100_000;
        manager
            .dispatch(5, Some(&exclusive(None)), "holder", t0, 28_800)
            .unwrap();
        assert!(matches!(
            manager.renew(5, "", 3600, t0 + 1).unwrap_err(),
            LockError::MissingRenewalReason
        ));
        assert!(matches!(
            manager.renew(5, "  ", 3600, t0 + 1).unwrap_err(),
            LockError::MissingRenewalReason
        ));
        assert!(matches!(
            manager.renew(5, "still on it", 0, t0 + 1).unwrap_err(),
            LockError::Parse { .. }
        ));

        let record = manager
            .renew(5, "the ninth hour: finishing extraction", 3600, t0 + 1)
            .unwrap();
        assert_eq!(record.renewals.len(), 1);
        assert_eq!(
            record.renewals[0].reason,
            "the ninth hour: finishing extraction"
        );
        assert_eq!(record.max_duration_extended_secs, 3600);
        // The max-duration release is now deferred by the recorded extension.
        manager
            .record_liveness(5, t0 + DEFAULT_MAX_DURATION_SECS + 1)
            .unwrap();
        assert_eq!(
            manager
                .tick(t0 + DEFAULT_MAX_DURATION_SECS + 1)
                .unwrap()
                .len(),
            0
        );
        // And it still expires, just later.
        manager
            .record_liveness(5, t0 + DEFAULT_MAX_DURATION_SECS + 3600 + 1)
            .unwrap();
        let fired = manager
            .tick(t0 + DEFAULT_MAX_DURATION_SECS + 3600 + 1)
            .unwrap();
        assert!(matches!(
            fired[0],
            LockEvent::ReleasedExpired {
                cause: ExpiryCause::MaxDuration,
                ..
            }
        ));
    }

    #[test]
    fn renewing_an_unheld_issue_fails_closed() {
        let root = temp_root();
        let manager = IssueLockManager::new(&root);
        let error = manager.renew(9, "reason", 60, 10).unwrap_err();
        assert!(matches!(error, LockError::NotHeld { issue: 9 }));
        let error = manager.release_clean(9, 10).unwrap_err();
        assert!(matches!(error, LockError::NotHeld { issue: 9 }));
    }

    #[test]
    fn forced_release_records_actor_reason_and_notifies_holder() {
        let root = temp_root();
        let manager = IssueLockManager::new(&root);
        let t0 = 100_000;
        manager
            .dispatch(
                5,
                Some(&exclusive(Some("long refactor"))),
                "holder",
                t0,
                3600,
            )
            .unwrap();
        // A forced release without a reason is rejected.
        assert!(manager
            .force_release(5, "supervisor", "  ", t0 + 10)
            .is_err());

        let record = manager
            .force_release(5, "supervisor", "the refactor is stuck", t0 + 10)
            .unwrap();
        assert!(!record.is_active());
        assert!(record.is_forced());
        assert_eq!(
            record.holder_notice.as_deref(),
            Some("lock broken by supervisor: the refactor is stuck")
        );
        match record.release {
            Some(LockRelease::Forced { actor, reason, at }) => {
                assert_eq!(actor, "supervisor");
                assert_eq!(reason, "the refactor is stuck");
                assert_eq!(at, t0 + 10);
            }
            other => panic!("expected a forced release, got: {other:?}"),
        }
        // The queue drains immediately.
        assert_eq!(
            manager
                .dispatch(6, Some(&exclusive(None)), "other", t0 + 20, 3600)
                .unwrap(),
            DispatchDecision::Admit
        );
    }

    #[test]
    fn broken_lock_is_not_a_clean_release() {
        let root = temp_root();
        let manager = IssueLockManager::new(&root);
        let t0 = 100_000;
        manager
            .dispatch(5, Some(&exclusive(None)), "holder", t0, 3600)
            .unwrap();
        manager
            .force_release(5, "supervisor", "stalled", t0 + 10)
            .unwrap();
        let record = manager.record(5).unwrap().expect("lock record");
        assert!(matches!(record.release, Some(LockRelease::Forced { .. })));
        assert!(
            !matches!(record.release, Some(LockRelease::Clean { .. })),
            "a broken lock must not be recorded as a clean completion"
        );
        // The run record shows the break distinctly.
        let events = manager.events(5).unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            LockEvent::ForcedBreak { actor, .. } if actor == "supervisor"
        )));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, LockEvent::ReleasedClean { .. })),
            "no clean release event may exist for a broken lock"
        );
    }

    #[test]
    fn lock_state_survives_process_restart() {
        let root = temp_root();
        let t0 = 100_000;
        {
            let manager = IssueLockManager::new(&root);
            manager
                .dispatch(5, Some(&exclusive(Some("refactor"))), "holder", t0, 3600)
                .unwrap();
            manager
                .dispatch(6, Some(&exclusive(None)), "other", t0 + 10, 3600)
                .unwrap();
        }
        // A brand-new manager (fresh process) sees the same state.
        let manager = IssueLockManager::new(&root);
        let record = manager.record(5).unwrap().expect("lock record");
        assert!(record.is_active());
        assert_eq!(record.scope, LockScope::Exclusive);
        assert_eq!(record.reason, "refactor");
        let queued = manager.queue_entry(6).unwrap().expect("queued entry");
        assert_eq!((queued.waiting_on, queued.since), (5, t0 + 10));
        // And it can still act on that state: break the lock and admit #6.
        manager
            .force_release(5, "supervisor", "restarting work", t0 + 20)
            .unwrap();
        assert_eq!(
            manager
                .dispatch(6, Some(&exclusive(None)), "other", t0 + 30, 3600)
                .unwrap(),
            DispatchDecision::Admit
        );
    }

    #[test]
    fn status_json_reports_holder_reason_queue_and_release_kinds() {
        let root = temp_root();
        let manager = IssueLockManager::new(&root);
        let t0 = 100_000;
        manager
            .dispatch(
                5,
                Some(&exclusive(Some("mass extraction"))),
                "holder",
                t0,
                3600,
            )
            .unwrap();
        manager
            .dispatch(6, Some(&exclusive(None)), "other", t0 + 10, 3600)
            .unwrap();

        let status: serde_json::Value =
            serde_json::from_str(&manager.status_json().unwrap()).unwrap();
        assert_eq!(status["active"][0]["issue"], 5);
        assert_eq!(status["active"][0]["reason"], "mass extraction");
        assert_eq!(status["active"][0]["acquired_at"], t0);
        assert_eq!(status["active"][0]["scope"], "exclusive");
        assert_eq!(status["active"][0]["waiting"], 1);
        assert_eq!(status["queued"][0]["issue"], 6);
        assert_eq!(status["queued"][0]["waiting_on"], 5);
        assert_eq!(status["queued"][0]["since"], t0 + 10);

        manager
            .force_release(5, "supervisor", "stalled", t0 + 20)
            .unwrap();
        let status: serde_json::Value =
            serde_json::from_str(&manager.status_json().unwrap()).unwrap();
        assert_eq!(status["active"].as_array().unwrap().len(), 0);
        assert_eq!(status["released"][0]["issue"], 5);
        assert_eq!(status["released"][0]["release"], "forced");
        assert_eq!(status["released"][0]["broken_by"], "supervisor");
    }

    #[test]
    fn status_lines_format_for_cli() {
        let root = temp_root();
        let manager = IssueLockManager::new(&root);
        let t0 = 7 * 3600 + 15 * 60; // 07:15 UTC
        manager
            .dispatch(
                5,
                Some(&exclusive(Some("mass extraction"))),
                "holder",
                t0,
                3600,
            )
            .unwrap();
        manager
            .dispatch(6, Some(&exclusive(None)), "other", t0 + 60, 3600)
            .unwrap();
        let lines = manager.status_lines().unwrap();
        let holder_line = lines
            .iter()
            .find(|line| line.starts_with("lock: #5"))
            .expect("a holder line");
        assert!(holder_line.contains("reason: mass extraction"));
        assert!(holder_line.contains("acquired 07:15Z"));
        let wait_line = lines
            .iter()
            .find(|line| line.starts_with("  #6 waiting on #5"))
            .expect("a wait line");
        assert!(wait_line.contains("since 07:16Z"));
    }

    #[test]
    fn run_record_is_append_only_and_complete() {
        let root = temp_root();
        let manager = IssueLockManager::new(&root);
        let t0 = 100_000;
        manager
            .dispatch(5, Some(&exclusive(None)), "holder", t0, 3600)
            .unwrap();
        manager
            .dispatch(6, Some(&exclusive(None)), "other", t0 + 10, 3600)
            .unwrap();
        manager.record_liveness(5, t0 + 20).unwrap();
        manager.release_clean(5, t0 + 30).unwrap();
        manager
            .dispatch(6, Some(&exclusive(None)), "other", t0 + 40, 3600)
            .unwrap();
        let events = manager.events(5).unwrap();
        let kinds: Vec<&str> = events
            .iter()
            .map(|event| match event {
                LockEvent::Acquired { .. } => "acquired",
                LockEvent::Liveness { .. } => "liveness",
                LockEvent::Renewed { .. } => "renewed",
                LockEvent::Queued { .. } => "queued",
                LockEvent::Dequeued { .. } => "dequeued",
                LockEvent::ReleasedClean { .. } => "released_clean",
                LockEvent::ReleasedExpired { .. } => "released_expired",
                LockEvent::ReleasedDeadHolder { .. } => "released_dead_holder",
                LockEvent::ForcedBreak { .. } => "forced_break",
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                "acquired",
                "queued",
                "liveness",
                "released_clean",
                "dequeued"
            ]
        );
    }

    #[test]
    fn crashed_claim_file_is_not_a_lock() {
        let root = temp_root();
        let manager = IssueLockManager::new(&root);
        let t0 = 100_000;
        manager
            .dispatch(5, Some(&paths(&["crates/**"])), "holder", t0, 3600)
            .unwrap();
        // Simulate a crashed claim: name created, content never published.
        fs::create_dir_all(&manager.locks_dir).unwrap();
        fs::write(manager.lock_path(9), "").unwrap();
        let records = manager.load().unwrap();
        assert_eq!(
            records
                .iter()
                .map(|record| record.issue)
                .collect::<Vec<_>>(),
            vec![5]
        );
        // The crashed claim does not block a disjoint acquisition.
        let declaration = paths(&["docs/**"]);
        assert!(matches!(
            manager
                .acquire(9, &declaration, "reclaim", t0 + 1, 3600)
                .unwrap()
                .issue,
            9
        ));
    }
}
