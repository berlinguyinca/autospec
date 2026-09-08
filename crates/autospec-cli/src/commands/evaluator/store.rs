//! Minimal on-disk evaluation store used by the `autospec evaluator`
//! commands. Layout under `<root>/.autospec/evaluation/`:
//!
//! - `policy.json` — write-once promotion policy
//! - `evaluators/<slot>/v<N>.json` — immutable registered definitions
//! - `epochs/epoch-<NNNNNN>.json` — immutable epoch records
//! - `current.json` — single atomic pointer to the active epoch
//! - `promotions/promo-<16hex>.json` — committed promotion events (pin)

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use autospec_core::autonomous::waterfall::sha256_hex;

use super::args::now;
use super::fsutil::{
    atomic_write, create_new_write, io_err, load_json, parse_version_file_name, pretty_json,
};
use super::types::{
    Approval, ApprovalKind, CurrentPointer, EpochId, EvaluationError, EvaluationErrorKind,
    EvaluatorDefinition, EvaluatorEpoch, EvaluatorSlot, EvaluatorVersionRef, PromotionEvent,
    PromotionPolicy, EVALUATOR_STORE_SCHEMA,
};
use crate::commands::{CommandFailure, CommandFailureKind};

impl From<EvaluationError> for CommandFailure {
    fn from(error: EvaluationError) -> Self {
        CommandFailure {
            message: error.to_string(),
            exit_code: 2,
            kind: CommandFailureKind::Diagnostic,
        }
    }
}

/// One entry from `list`, sorted by slot then version.
#[derive(Debug)]
pub struct RegistryEntry {
    pub reference: EvaluatorVersionRef,
    pub kind: super::types::EvaluatorKind,
    pub digest: String,
    pub created_at: u64,
}

/// Result of `init`.
#[derive(Debug)]
pub struct InitReport {
    pub base: PathBuf,
    pub epoch: EpochId,
    pub policy_digest: String,
}

/// Result of `register`.
#[derive(Debug)]
pub struct RegisterReport {
    pub reference: EvaluatorVersionRef,
    pub digest: String,
    pub path: PathBuf,
}

/// Result of `pin`.
#[derive(Debug)]
pub struct PinReport {
    pub reference: EvaluatorVersionRef,
    pub epoch: EpochId,
    pub promotion_id: String,
    pub actor: String,
}

/// The evaluation store rooted at a base directory.
#[derive(Clone, Debug)]
pub struct EvaluationStore {
    pub base: PathBuf,
}

impl EvaluationStore {
    pub fn new(base: PathBuf) -> Self {
        Self { base }
    }

    pub fn policy_path(&self) -> PathBuf {
        self.base.join("policy.json")
    }

    pub fn current_path(&self) -> PathBuf {
        self.base.join("current.json")
    }

    pub fn epochs_dir(&self) -> PathBuf {
        self.base.join("epochs")
    }

    pub fn evaluators_dir(&self) -> PathBuf {
        self.base.join("evaluators")
    }

    pub fn promotions_dir(&self) -> PathBuf {
        self.base.join("promotions")
    }

    pub fn initialized(&self) -> bool {
        self.policy_path().is_file() && self.current_path().is_file()
    }

    pub fn require_initialized(&self) -> Result<(), EvaluationError> {
        if !self.initialized() {
            return Err(EvaluationError::new(
                EvaluationErrorKind::FailClosed,
                format!(
                    "evaluation store not initialized at {} (run 'autospec evaluator init')",
                    self.base.display()
                ),
            ));
        }
        Ok(())
    }

    fn evaluator_path(&self, reference: &EvaluatorVersionRef) -> PathBuf {
        self.evaluators_dir()
            .join(reference.slot.as_str())
            .join(format!("v{}.json", reference.version))
    }

    fn epoch_path(&self, epoch_id: EpochId) -> PathBuf {
        self.epochs_dir().join(format!("{epoch_id}.json"))
    }

    /// Initialize the store: policy (write-once), genesis epoch, current
    /// pointer. Refuses to re-initialize an existing store.
    pub fn init(&self, policy_file: Option<&Path>) -> Result<InitReport, EvaluationError> {
        let policy_path = self.policy_path();
        if policy_path.exists() {
            return Err(EvaluationError::new(
                EvaluationErrorKind::Immutable,
                format!(
                    "evaluation policy already initialized at {}",
                    policy_path.display()
                ),
            ));
        }
        let policy = match policy_file {
            Some(file) => {
                let raw = fs::read(file).map_err(|err| {
                    EvaluationError::new(
                        EvaluationErrorKind::Io,
                        format!("read policy file {}: {err}", file.display()),
                    )
                })?;
                let policy: PromotionPolicy = serde_json::from_slice(&raw).map_err(|err| {
                    EvaluationError::new(
                        EvaluationErrorKind::Parse,
                        format!("policy file {file:?}: {err}"),
                    )
                })?;
                policy.validate()?;
                policy
            }
            None => PromotionPolicy::default(),
        };
        let policy_digest = policy.policy_digest();

        fs::create_dir_all(&self.base).map_err(io_err("create store base"))?;
        for dir in [
            self.evaluators_dir(),
            self.epochs_dir(),
            self.promotions_dir(),
        ] {
            fs::create_dir_all(&dir).map_err(io_err("create store directory"))?;
        }
        atomic_write(&policy_path, &pretty_json(&policy)?)?;

        let epoch = EvaluatorEpoch {
            schema: EVALUATOR_STORE_SCHEMA,
            epoch_id: EpochId::genesis(),
            slot_versions: BTreeMap::new(),
            started_at: now(),
            predecessor: None,
            promotion: None,
            policy_digest: policy_digest.clone(),
            anchor_suite_digests: BTreeMap::new(),
        };
        create_new_write(&self.epoch_path(epoch.epoch_id), &pretty_json(&epoch)?)?;
        atomic_write(
            &self.current_path(),
            &pretty_json(&CurrentPointer {
                schema: EVALUATOR_STORE_SCHEMA,
                epoch_id: epoch.epoch_id,
            })?,
        )?;

        Ok(InitReport {
            base: self.base.clone(),
            epoch: epoch.epoch_id,
            policy_digest,
        })
    }

    /// Register an immutable evaluator definition from a JSON file.
    pub fn register(&self, file: &Path) -> Result<RegisterReport, EvaluationError> {
        self.require_initialized()?;
        let raw = fs::read(file).map_err(|err| {
            EvaluationError::new(
                EvaluationErrorKind::Io,
                format!("read definition file {}: {err}", file.display()),
            )
        })?;
        let definition: EvaluatorDefinition = serde_json::from_slice(&raw).map_err(|err| {
            EvaluationError::new(
                EvaluationErrorKind::Parse,
                format!("definition file {file:?}: {err}"),
            )
        })?;
        definition.validate()?;
        let reference = EvaluatorVersionRef::new(definition.slot, definition.version);
        let path = self.evaluator_path(&reference);
        if path.exists() {
            return Err(EvaluationError::new(
                EvaluationErrorKind::Immutable,
                format!("evaluator already registered at {}", path.display()),
            ));
        }
        create_new_write(&path, &pretty_json(&definition)?)?;
        Ok(RegisterReport {
            reference,
            digest: definition.definition_digest(),
            path,
        })
    }

    /// All registered evaluators, sorted by slot then version.
    pub fn list(&self) -> Result<Vec<RegistryEntry>, EvaluationError> {
        self.require_initialized()?;
        let evaluators = self.evaluators_dir();
        if !evaluators.is_dir() {
            return Err(EvaluationError::new(
                EvaluationErrorKind::FailClosed,
                format!("evaluators directory missing at {}", evaluators.display()),
            ));
        }
        let mut entries = Vec::new();
        for slot in EvaluatorSlot::ALL {
            let slot_dir = evaluators.join(slot.as_str());
            if !slot_dir.is_dir() {
                continue;
            }
            let dir = fs::read_dir(&slot_dir).map_err(io_err("list evaluators"))?;
            for entry in dir {
                let entry = entry.map_err(io_err("list evaluators"))?;
                let file_name = entry.file_name().to_string_lossy().into_owned();
                let Some(version) = parse_version_file_name(&file_name) else {
                    continue;
                };
                let reference = EvaluatorVersionRef::new(slot, version);
                let (definition, digest) = self.show(&reference)?;
                entries.push(RegistryEntry {
                    reference,
                    kind: definition.kind,
                    digest,
                    created_at: definition.created_at,
                });
            }
        }
        entries.sort_by(|a, b| {
            a.reference
                .slot
                .as_str()
                .cmp(b.reference.slot.as_str())
                .then_with(|| a.reference.version.cmp(&b.reference.version))
        });
        Ok(entries)
    }

    /// One registered definition plus its recomputed digest.
    pub fn show(
        &self,
        reference: &EvaluatorVersionRef,
    ) -> Result<(EvaluatorDefinition, String), EvaluationError> {
        let path = self.evaluator_path(reference);
        if !path.is_file() {
            return Err(EvaluationError::new(
                EvaluationErrorKind::FailClosed,
                format!(
                    "evaluator {reference} is not registered at {}",
                    path.display()
                ),
            ));
        }
        let definition: EvaluatorDefinition = load_json(&path)?;
        let digest = definition.definition_digest();
        Ok((definition, digest))
    }

    /// Seed an empty slot at a registered version into the next epoch,
    /// committing a human-approved promotion event.
    pub fn pin(
        &self,
        reference: &EvaluatorVersionRef,
        actor: &str,
    ) -> Result<PinReport, EvaluationError> {
        self.require_initialized()?;
        if actor.trim().is_empty() {
            return Err(EvaluationError::new(
                EvaluationErrorKind::Invariant,
                "pin requires a non-empty --actor",
            ));
        }
        let (definition, _digest) = self.show(reference)?;
        if definition.version != reference.version {
            return Err(EvaluationError::new(
                EvaluationErrorKind::Integrity,
                format!(
                    "registered definition for {reference} has version {}",
                    definition.version
                ),
            ));
        }
        let current = self.epoch_current()?;
        if let Some(existing) = current.slot_versions.get(&reference.slot) {
            let slot = reference.slot;
            return Err(EvaluationError::new(
                EvaluationErrorKind::FailClosed,
                format!(
                "slot {slot} is already pinned at {slot}@{existing}; pin seeds only empty slots"
            ),
            ));
        }
        let timestamp = now();
        let prior = current.epoch_id;
        let next = prior.next();
        let seed = format!(
            "pin\0{}\0{}\0{}\0{}\0{}",
            reference.slot, reference.version, prior, actor, timestamp
        );
        let promotion_id = format!("promo-{}", &sha256_hex(seed.as_bytes())[..16]);

        let event = PromotionEvent {
            schema: EVALUATOR_STORE_SCHEMA,
            promotion_id: promotion_id.clone(),
            state: "committed".to_string(),
            slot: reference.slot,
            from: None,
            to: reference.version,
            challenger_trial: None,
            prior_epoch: prior,
            new_epoch: next,
            invalidated_evaluations: Vec::new(),
            approval: Approval {
                kind: ApprovalKind::Human,
                actor: actor.to_string(),
                at: timestamp,
            },
            created_at: timestamp,
            committed_at: Some(timestamp),
        };
        let event_path = self.promotions_dir().join(format!("{promotion_id}.json"));
        create_new_write(&event_path, &pretty_json(&event)?)?;

        let epoch = EvaluatorEpoch {
            schema: EVALUATOR_STORE_SCHEMA,
            epoch_id: next,
            slot_versions: {
                let mut map = BTreeMap::new();
                map.insert(reference.slot, reference.version);
                map
            },
            started_at: timestamp,
            predecessor: Some(prior),
            promotion: Some(promotion_id.clone()),
            policy_digest: current.policy_digest,
            anchor_suite_digests: BTreeMap::new(),
        };
        create_new_write(&self.epoch_path(next), &pretty_json(&epoch)?)?;
        atomic_write(
            &self.current_path(),
            &pretty_json(&CurrentPointer {
                schema: EVALUATOR_STORE_SCHEMA,
                epoch_id: next,
            })?,
        )?;

        Ok(PinReport {
            reference: *reference,
            epoch: next,
            promotion_id,
            actor: actor.to_string(),
        })
    }

    /// The active epoch, resolved through the atomic `current.json` pointer.
    pub fn epoch_current(&self) -> Result<EvaluatorEpoch, EvaluationError> {
        self.require_initialized()?;
        let pointer: CurrentPointer = load_json(&self.current_path())?;
        if pointer.schema != EVALUATOR_STORE_SCHEMA {
            return Err(EvaluationError::new(
                EvaluationErrorKind::Invariant,
                format!("unsupported current.json schema: {}", pointer.schema),
            ));
        }
        self.read_epoch(pointer.epoch_id)
    }

    /// Every epoch record, sorted by epoch id.
    pub fn epoch_history(&self) -> Result<Vec<EvaluatorEpoch>, EvaluationError> {
        self.require_initialized()?;
        let epochs = self.epochs_dir();
        if !epochs.is_dir() {
            return Err(EvaluationError::new(
                EvaluationErrorKind::FailClosed,
                format!("epochs directory missing at {}", epochs.display()),
            ));
        }
        let dir = fs::read_dir(&epochs).map_err(io_err("list epochs"))?;
        let mut history = Vec::new();
        for entry in dir {
            let entry = entry.map_err(io_err("list epochs"))?;
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let Some(stem) = file_name.strip_suffix(".json") else {
                continue;
            };
            let Ok(epoch_id) = EpochId::parse(stem) else {
                continue;
            };
            history.push(self.read_epoch(epoch_id)?);
        }
        history.sort_by_key(|epoch| epoch.epoch_id);
        Ok(history)
    }

    fn read_epoch(&self, epoch_id: EpochId) -> Result<EvaluatorEpoch, EvaluationError> {
        let path = self.epoch_path(epoch_id);
        if !path.is_file() {
            return Err(EvaluationError::new(
                EvaluationErrorKind::FailClosed,
                format!("epoch {epoch_id} is missing at {}", path.display()),
            ));
        }
        let epoch: EvaluatorEpoch = load_json(&path)?;
        epoch.check()?;
        if epoch.epoch_id != epoch_id {
            return Err(EvaluationError::new(
                EvaluationErrorKind::Integrity,
                format!("epoch file {epoch_id} declares epoch {}", epoch.epoch_id),
            ));
        }
        Ok(epoch)
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    pub fn temp_base() -> PathBuf {
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!(
            "autospec-evaluator-test-{}-{counter}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        base
    }
}
#[cfg(test)]
mod store_tests;
