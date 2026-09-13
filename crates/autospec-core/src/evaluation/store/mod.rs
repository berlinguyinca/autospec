//! Repo-local evaluation store under `.autospec/evaluation/`.
//!
//! This module owns the on-disk layout ([`layout`]) and the crash-safe write
//! helpers ([`io`]) that every later store component (journal, epoch
//! transition) builds on. Torn writes are the threat being designed out:
//!
//! - replaceable documents are written tmp + `sync_all` + `rename` +
//!   parent-directory `sync_all`;
//! - immutable documents are opened with `create_new`, so a second write to
//!   the same version is an [`EvaluationErrorKind::Immutable`] error naming
//!   the path;
//! - appended lines record their pre-append length and roll back with
//!   `set_len` when the write is incomplete (or an injected fault fires).
//!
//! Layout table and error kinds:
//! `docs/specs/2026-09-05-evaluator-coevolution-design.md` (Architecture /
//! Interfaces sections).

pub mod io;
pub mod journal;
pub mod layout;
mod pin;

use std::fmt;

/// Failure class for every evaluation-store operation.
///
/// Renders as the lowercase kebab-case token used on the CLI diagnostic
/// line (`<kind>: <message>`): `invariant`, `immutable`, `integrity`, `io`,
/// `parse`, `fail-closed`, `access-denied`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EvaluationErrorKind {
    /// A structural precondition of the store was violated (bad schema,
    /// version ordering, missing required field).
    Invariant,
    /// A write raced or repeated an immutable document.
    Immutable,
    /// A digest, chain, or checkpoint verification failed.
    Integrity,
    /// An underlying filesystem operation failed.
    Io,
    /// A stored document failed to parse as the expected JSON shape.
    Parse,
    /// Evidence is incomplete and the transition must not proceed.
    FailClosed,
    /// The caller's access role may not see this data.
    AccessDenied,
}

impl EvaluationErrorKind {
    /// The CLI diagnostic token for this kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Invariant => "invariant",
            Self::Immutable => "immutable",
            Self::Integrity => "integrity",
            Self::Io => "io",
            Self::Parse => "parse",
            Self::FailClosed => "fail-closed",
            Self::AccessDenied => "access-denied",
        }
    }
}

impl fmt::Display for EvaluationErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A typed evaluation-store failure. `kind` drives exit codes and the
/// diagnostic prefix; `message` names the path or field involved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationError {
    pub kind: EvaluationErrorKind,
    pub message: String,
}

impl EvaluationError {
    pub fn new(kind: EvaluationErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::new(EvaluationErrorKind::Io, message)
    }

    pub fn fail_closed(message: impl Into<String>) -> Self {
        Self::new(EvaluationErrorKind::FailClosed, message)
    }
}

impl fmt::Display for EvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for EvaluationError {}

impl From<std::io::Error> for EvaluationError {
    fn from(error: std::io::Error) -> Self {
        Self::io(error.to_string())
    }
}

/// Convenience alias used throughout the store.
pub type Result<T> = std::result::Result<T, EvaluationError>;

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::anchor::{AccessRole, AnchorSuite};
use super::digest::Digest;
use super::epoch::EvaluatorEpoch;
use super::evaluator::EvaluatorDefinition;
use super::ids::{
    AnchorSuiteId, ChallengerTrialId, EpochId, EvaluationId, EvaluatorSlot, EvaluatorVersionRef,
    PromotionId,
};
use super::policy::PromotionPolicy;
use super::promotion::{ChallengerTrial, PromotionEvent};
use super::record::EvaluationRecord;
use super::{error, EVALUATION_SCHEMA_VERSION};
use crate::evaluation::store::journal::Journal;
use crate::evaluation::store::layout::EvaluationLayout;

impl From<error::EvaluationErrorKind> for EvaluationErrorKind {
    fn from(kind: error::EvaluationErrorKind) -> Self {
        use error::EvaluationErrorKind as Module;
        match kind {
            Module::Invariant => Self::Invariant,
            Module::Immutable => Self::Immutable,
            Module::Integrity => Self::Integrity,
            Module::Io => Self::Io,
            Module::Parse => Self::Parse,
            Module::FailClosed => Self::FailClosed,
            Module::AccessDenied => Self::AccessDenied,
        }
    }
}

impl From<error::EvaluationError> for EvaluationError {
    fn from(err: error::EvaluationError) -> Self {
        Self {
            kind: err.kind.into(),
            message: err.message,
        }
    }
}

/// The single atomic pointer to the active epoch (`current.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentPointer {
    pub schema: u64,
    pub epoch_id: EpochId,
}

/// The repo-local immutable evaluation registry.
///
/// Assembles the crash-safe layout ([`EvaluationLayout`]), the write-once
/// I/O helpers ([`io`]), and the hash-chained journal ([`Journal`]) into a
/// single system of record. Every versioned document is written exactly
/// once: a second write to the same slot/version is an
/// [`EvaluationErrorKind::Immutable`] error, never an overwrite.
#[derive(Debug)]
pub struct EvaluationStore {
    layout: EvaluationLayout,
    policy: PromotionPolicy,
    journal: Journal,
}

impl EvaluationStore {
    /// Create a fresh store under `repo_root/.autospec/evaluation`.
    ///
    /// Fails [`EvaluationErrorKind::Immutable`] if `policy.json` already
    /// exists (the store is already initialized). Otherwise writes the
    /// write-once policy, the genesis `epoch-000000`, the `current.json`
    /// pointer, and materializes the empty journal checkpoint.
    pub fn init(repo_root: impl AsRef<Path>, policy: PromotionPolicy, at: u64) -> Result<Self> {
        let layout = EvaluationLayout::new(repo_root);
        if layout.policy_file().exists() {
            return Err(EvaluationError::new(
                EvaluationErrorKind::Immutable,
                format!(
                    "evaluation store already initialized: {}",
                    layout.policy_file().display()
                ),
            ));
        }
        policy.validate()?;
        layout.ensure_directories()?;

        io::write_immutable_json(&layout.policy_file(), &policy)?;

        let genesis = EvaluatorEpoch::genesis(policy.policy_digest(), at);
        io::write_immutable_json(&layout.epoch_file(genesis.epoch_id.0), &genesis)?;

        let pointer = CurrentPointer {
            schema: EVALUATION_SCHEMA_VERSION,
            epoch_id: genesis.epoch_id,
        };
        atomic_write_json(&layout.current_file(), &pointer)?;

        let journal = Journal::open(&layout)?;
        Ok(Self {
            layout,
            policy,
            journal,
        })
    }

    /// Open an existing store, re-verifying the policy and the journal chain.
    ///
    /// Requires both `policy.json` and `current.json`; either missing is an
    /// [`EvaluationErrorKind::Invariant`] error (the store was not fully
    /// initialized). The journal is replayed and its chain verified, so a
    /// tampered or torn `events.jsonl` fails closed here.
    pub fn open(repo_root: impl AsRef<Path>) -> Result<Self> {
        let layout = EvaluationLayout::new(repo_root);
        let policy: PromotionPolicy = match io::read_json_if_exists(&layout.policy_file())? {
            Some(policy) => policy,
            None => {
                return Err(EvaluationError::new(
                    EvaluationErrorKind::Invariant,
                    format!(
                        "no evaluation policy at {} (store not initialized)",
                        layout.policy_file().display()
                    ),
                ))
            }
        };
        policy.validate()?;

        let pointer: CurrentPointer = match io::read_json_if_exists(&layout.current_file())? {
            Some(pointer) => pointer,
            None => {
                return Err(EvaluationError::new(
                    EvaluationErrorKind::Invariant,
                    format!(
                        "no current pointer at {} (store not initialized)",
                        layout.current_file().display()
                    ),
                ))
            }
        };
        if pointer.schema != EVALUATION_SCHEMA_VERSION {
            return Err(EvaluationError::new(
                EvaluationErrorKind::Invariant,
                format!(
                    "current pointer schema {} expected {}",
                    pointer.schema, EVALUATION_SCHEMA_VERSION
                ),
            ));
        }

        let journal = Journal::open(&layout)?;
        Ok(Self {
            layout,
            policy,
            journal,
        })
    }

    /// The validated promotion policy this store runs under.
    pub fn policy(&self) -> &PromotionPolicy {
        &self.policy
    }

    /// The filesystem layout of this store.
    pub fn layout(&self) -> &EvaluationLayout {
        &self.layout
    }

    /// Register an evaluator version write-once.
    ///
    /// Validates the definition, writes `evaluators/<slot>/v<N>.json`
    /// immutably (a repeat is [`EvaluationErrorKind::Immutable`]), and
    /// journals `evaluator.version.created`. Returns the behaviour digest.
    pub fn register_evaluator(
        &mut self,
        definition: &EvaluatorDefinition,
        at: u64,
    ) -> Result<Digest> {
        definition.validate()?;
        let file = self.layout.evaluator_file(definition.slot.as_str(), definition.version);
        io::write_immutable_json(&file, definition)?;
        let digest = definition.definition_digest();
        let mut fields = BTreeMap::new();
        fields.insert("slot".into(), definition.slot.as_str().into());
        fields.insert("version".into(), definition.version.to_string());
        fields.insert("digest".into(), digest.as_str().into());
        self.journal.append(
            at,
            "evaluator.version.created",
            &format!("evaluator:{}", definition.version_ref()),
            fields,
        )?;
        Ok(digest)
    }

    /// Read one registered evaluator version.
    pub fn evaluator(&self, version_ref: EvaluatorVersionRef) -> Result<EvaluatorDefinition> {
        io::read_json(&self.layout.evaluator_file(version_ref.slot.as_str(), version_ref.version))
    }

    /// Every registered evaluator definition, sorted by (slot, version).
    pub fn list_evaluators(&self) -> Result<Vec<EvaluatorDefinition>> {
        let mut out = Vec::new();
        for slot in EvaluatorSlot::ALL {
            let slot_dir = self.layout.evaluators_dir().join(slot.as_str());
            let entries = match std::fs::read_dir(&slot_dir) {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            for entry in entries {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let Some(version) = parse_version_file_name(&name) else {
                    continue;
                };
                let definition: EvaluatorDefinition = io::read_json(&entry.path())?;
                if definition.slot != slot || definition.version != version {
                    return Err(EvaluationError::new(
                        EvaluationErrorKind::Integrity,
                        format!(
                            "evaluator file {} does not match its path ({}@{})",
                            entry.path().display(),
                            slot,
                            version
                        ),
                    ));
                }
                out.push(definition);
            }
        }
        out.sort_by(|a, b| (a.slot, a.version).cmp(&(b.slot, b.version)));
        Ok(out)
    }

    /// Register an anchor suite version write-once.
    ///
    /// Validates the suite, verifies every artifact against its pinned
    /// `content_digest` (a mismatch is [`EvaluationErrorKind::Integrity`]),
    /// writes `anchors/<suite-id>/v<N>.json` immutably, and journals
    /// `anchor.suite.verified`. Returns the suite digest.
    pub fn register_anchor_suite(
        &mut self,
        suite: &AnchorSuite,
        repo_root: impl AsRef<Path>,
        at: u64,
    ) -> Result<Digest> {
        suite.validate()?;
        suite.verify_artifacts(repo_root.as_ref())?;
        let file = self.layout.anchor_file(suite.suite_id.as_str(), suite.version);
        io::write_immutable_json(&file, suite)?;
        let digest = suite.suite_digest();
        let mut fields = BTreeMap::new();
        fields.insert("suite".into(), suite.suite_id.as_str().into());
        fields.insert("version".into(), suite.version.to_string());
        fields.insert("digest".into(), digest.as_str().into());
        self.journal.append(
            at,
            "anchor.suite.verified",
            &format!("suite:{}@{}", suite.suite_id, suite.version),
            fields,
        )?;
        Ok(digest)
    }

    /// Read one anchor suite version through the redaction view for `role`.
    pub fn anchor_suite(
        &self,
        suite_id: &AnchorSuiteId,
        version: u32,
        role: AccessRole,
    ) -> Result<AnchorSuite> {
        let suite: AnchorSuite =
            io::read_json(&self.layout.anchor_file(suite_id.as_str(), version))?;
        Ok(suite.view(role))
    }

    /// Every registered anchor suite version as `(suite_id, version, digest)`,
    /// sorted by (suite id, version).
    pub fn list_anchor_suites(&self) -> Result<Vec<(AnchorSuiteId, u32, Digest)>> {
        let dir = self.layout.anchors_dir();
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let suite_id_str = entry.file_name().to_string_lossy().into_owned();
            let suite_id = AnchorSuiteId::parse(&suite_id_str)?;
            let suite_dir = entry.path();
            let v_entries = std::fs::read_dir(&suite_dir)?;
            for v_entry in v_entries {
                let v_entry = v_entry?;
                let name = v_entry.file_name().to_string_lossy().into_owned();
                let Some(version) = parse_version_file_name(&name) else {
                    continue;
                };
                let suite: AnchorSuite = io::read_json(&v_entry.path())?;
                if suite.suite_id != suite_id || suite.version != version {
                    return Err(EvaluationError::new(
                        EvaluationErrorKind::Integrity,
                        format!(
                            "anchor file {} does not match its path ({}@{})",
                            v_entry.path().display(), suite_id, version
                        ),
                    ));
                }
                out.push((suite_id.clone(), version, suite.suite_digest()));
            }
        }
        out.sort_by(|a, b| (a.0.as_str(), a.1).cmp(&(b.0.as_str(), b.1)));
        Ok(out)
    }

    /// The active epoch (the one `current.json` points at).
    pub fn current_epoch(&self) -> Result<EvaluatorEpoch> {
        let pointer: CurrentPointer = io::read_json(&self.layout.current_file())?;
        self.epoch(pointer.epoch_id)
    }

    /// Read one epoch by id.
    pub fn epoch(&self, epoch_id: EpochId) -> Result<EvaluatorEpoch> {
        io::read_json(&self.layout.epoch_file(epoch_id.0))
    }

    /// Every epoch, oldest first.
    pub fn epoch_history(&self) -> Result<Vec<EvaluatorEpoch>> {
        let dir = self.layout.epochs_dir();
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(number) = name
                .strip_prefix("epoch-")
                .and_then(|s| s.strip_suffix(".json"))
                .and_then(|s| s.parse::<u64>().ok())
            else {
                continue;
            };
            let epoch: EvaluatorEpoch = io::read_json(&entry.path())?;
            if epoch.epoch_id.0 != number {
                return Err(EvaluationError::new(
                    EvaluationErrorKind::Integrity,
                    format!(
                        "epoch file {} does not match its path (epoch-{:06})",
                        entry.path().display(),
                        number
                    ),
                ));
            }
            out.push(epoch);
        }
        out.sort_by_key(|epoch| epoch.epoch_id.0);
        Ok(out)
    }

    /// Record a challenger trial write-once, journaling
    /// `evaluator.challenger.completed`.
    pub fn write_trial(&mut self, trial: &ChallengerTrial, at: u64) -> Result<()> {
        let file = self.layout.trial_file(trial.id.as_str());
        io::write_immutable_json(&file, trial)?;
        let mut fields = BTreeMap::new();
        fields.insert("incumbent".into(), trial.incumbent.to_string());
        fields.insert("challenger".into(), trial.challenger.to_string());
        fields.insert("verdict".into(), trial.verdict.as_str().into());
        self.journal.append(at, "evaluator.challenger.completed", &format!("trial:{}", trial.id), fields)?;
        Ok(())
    }

    /// Read one challenger trial.
    pub fn trial(&self, trial_id: &ChallengerTrialId) -> Result<ChallengerTrial> {
        io::read_json(&self.layout.trial_file(trial_id.as_str()))
    }

    /// Record an evaluation judgment write-once, journaling
    /// `evaluation.completed`.
    pub fn write_record(&mut self, record: &EvaluationRecord, at: u64) -> Result<()> {
        let file = self.layout.record_file(record.evaluation_id.as_str());
        io::write_immutable_json(&file, record)?;
        let mut fields = BTreeMap::new();
        fields.insert("evaluator".into(), record.evaluator.to_string());
        fields.insert("epoch".into(), record.epoch_id.to_string());
        self.journal
            .append(at, "evaluation.completed", &format!("record:{}", record.evaluation_id), fields)?;
        Ok(())
    }

    /// Read one evaluation record.
    pub fn record(&self, evaluation_id: &EvaluationId) -> Result<EvaluationRecord> {
        io::read_json(&self.layout.record_file(evaluation_id.as_str()))
    }

    /// Every evaluation record, sorted by id.
    pub fn records(&self) -> Result<Vec<EvaluationRecord>> {
        let dir = self.layout.records_dir();
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".json") {
                continue;
            }
            let record: EvaluationRecord = io::read_json(&entry.path())?;
            out.push(record);
        }
        out.sort_by_key(|record| record.evaluation_id.as_str().to_string());
        Ok(out)
    }

    /// Read one promotion event.
    pub fn promotion(&self, promotion_id: &PromotionId) -> Result<PromotionEvent> {
        io::read_json(&self.layout.promotion_file(promotion_id.as_str()))
    }

    /// Every promotion event, sorted by id.
    pub fn promotions(&self) -> Result<Vec<PromotionEvent>> {
        let dir = self.layout.promotions_dir();
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".json") {
                continue;
            }
            let event: PromotionEvent = io::read_json(&entry.path())?;
            out.push(event);
        }
        out.sort_by_key(|event| event.id.as_str().to_string());
        Ok(out)
    }
}

/// Parse a `v<N>.json` file name into its version number.
fn parse_version_file_name(name: &str) -> Option<u32> {
    name.strip_prefix("v")?
        .strip_suffix(".json")?
        .parse::<u32>()
        .ok()
}

/// Serialize `value` as pretty JSON (+ trailing newline) and replace `path`
/// atomically. Used for the replaceable `current.json` pointer.
fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| {
        EvaluationError::new(
            EvaluationErrorKind::Invariant,
            format!("failed to serialize document for {}: {error}", path.display()),
        )
    })?;
    bytes.push(b'\n');
    io::atomic_write(path, &bytes)
}

