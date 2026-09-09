//! Path layout for the repo-local evaluation store.
//!
//! ```text
//! .autospec/evaluation/
//!   policy.json                       PromotionPolicy; write-once (re-init fails)
//!   evaluators/<slot>/v<N>.json       EvaluatorDefinition; immutable
//!   anchors/<suite-id>/v<N>.json      AnchorSuite; immutable
//!   epochs/epoch-<NNNNNN>.json        EvaluatorEpoch; immutable
//!   current.json                      the single atomic epoch pointer
//!   trials/<trial-id>.json            ChallengerTrial; immutable
//!   promotions/<promotion-id>.json    PromotionEvent; pending -> committed
//!   records/<evaluation-id>.json      EvaluationRecord
//!   events.jsonl                      hash-chained, idempotency keys
//!   events.checkpoint.json            journal high-watermark checkpoint
//! ```
//!
//! (Store layout table,
//! `docs/specs/2026-09-05-evaluator-coevolution-design.md` §Architecture.)

use std::path::{Path, PathBuf};

use super::{EvaluationError, Result};

/// All filesystem locations of one evaluation store, rooted at
/// `<repo_root>/.autospec/evaluation`.
///
/// The layout is pure path arithmetic: construction never touches the
/// filesystem, and every accessor returns a fresh `PathBuf` so callers own
/// what they do with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationLayout {
    root: PathBuf,
}

impl EvaluationLayout {
    /// Build the layout for `repo_root` without touching the filesystem.
    pub fn new(repo_root: impl AsRef<Path>) -> Self {
        Self {
            root: repo_root.as_ref().join(".autospec").join("evaluation"),
        }
    }

    /// `<repo_root>/.autospec/evaluation`.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `policy.json` — the write-once promotion policy.
    pub fn policy_file(&self) -> PathBuf {
        self.root.join("policy.json")
    }

    /// `evaluators/` — one subdirectory per evaluator slot.
    pub fn evaluators_dir(&self) -> PathBuf {
        self.root.join("evaluators")
    }

    /// `evaluators/<slot>/v<N>.json` — one immutable file per version.
    pub fn evaluator_file(&self, slot: &str, version: u32) -> PathBuf {
        self.evaluators_dir()
            .join(slot)
            .join(format!("v{version}.json"))
    }

    /// `anchors/` — one subdirectory per anchor suite id.
    pub fn anchors_dir(&self) -> PathBuf {
        self.root.join("anchors")
    }

    /// `anchors/<suite-id>/v<N>.json` — one immutable file per version.
    pub fn anchor_file(&self, suite_id: &str, version: u32) -> PathBuf {
        self.anchors_dir()
            .join(suite_id)
            .join(format!("v{version}.json"))
    }

    /// `epochs/` — one immutable file per epoch.
    pub fn epochs_dir(&self) -> PathBuf {
        self.root.join("epochs")
    }

    /// `epochs/epoch-<NNNNNN>.json` for epoch number `epoch`
    /// (zero-padded to six digits, e.g. `epoch-000000.json` for genesis).
    pub fn epoch_file(&self, epoch: u64) -> PathBuf {
        self.epochs_dir().join(format!("epoch-{epoch:06}.json"))
    }

    /// `current.json` — the single atomic pointer to the active epoch.
    pub fn current_file(&self) -> PathBuf {
        self.root.join("current.json")
    }

    /// `trials/` — one immutable file per challenger trial.
    pub fn trials_dir(&self) -> PathBuf {
        self.root.join("trials")
    }

    /// `trials/<trial-id>.json` — one immutable file per challenger trial.
    pub fn trial_file(&self, trial_id: &str) -> PathBuf {
        self.trials_dir().join(format!("{trial_id}.json"))
    }

    /// `promotions/` — one file per promotion (pending, then committed by
    /// atomic rewrite).
    pub fn promotions_dir(&self) -> PathBuf {
        self.root.join("promotions")
    }

    /// `promotions/<promotion-id>.json`.
    pub fn promotion_file(&self, promotion_id: &str) -> PathBuf {
        self.promotions_dir().join(format!("{promotion_id}.json"))
    }

    /// `records/` — one file per evaluation record.
    pub fn records_dir(&self) -> PathBuf {
        self.root.join("records")
    }

    /// `records/<evaluation-id>.json`.
    pub fn record_file(&self, evaluation_id: &str) -> PathBuf {
        self.records_dir().join(format!("{evaluation_id}.json"))
    }

    /// `events.jsonl` — the hash-chained event journal.
    pub fn journal_file(&self) -> PathBuf {
        self.root.join("events.jsonl")
    }

    /// `events.checkpoint.json` — the journal high-watermark checkpoint.
    pub fn checkpoint_file(&self) -> PathBuf {
        self.root.join("events.checkpoint.json")
    }

    /// Every directory the store can hold files in.
    fn directories(&self) -> [PathBuf; 7] {
        [
            self.root.clone(),
            self.evaluators_dir(),
            self.anchors_dir(),
            self.epochs_dir(),
            self.trials_dir(),
            self.promotions_dir(),
            self.records_dir(),
        ]
    }

    /// Create the store directories (and `repo_root` parents of the store)
    /// if they do not exist yet. Idempotent.
    pub fn ensure_directories(&self) -> Result<()> {
        for directory in self.directories() {
            std::fs::create_dir_all(&directory).map_err(|error| {
                EvaluationError::io(format!(
                    "failed to create evaluation directory {}: {error}",
                    directory.display()
                ))
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn root_is_the_autospec_evaluation_directory() {
        let layout = EvaluationLayout::new("/tmp/repo");
        assert_eq!(layout.root(), Path::new("/tmp/repo/.autospec/evaluation"));
        assert!(layout.root().ends_with(".autospec/evaluation"));
    }

    #[test]
    fn evaluator_file_renders_slot_and_version() {
        let layout = EvaluationLayout::new("/tmp/repo");
        assert_eq!(
            layout.evaluator_file("architecture", 1),
            layout.root().join("evaluators/architecture/v1.json")
        );
        assert_eq!(
            layout.evaluator_file("ui_ux", 12),
            layout.root().join("evaluators/ui_ux/v12.json")
        );
    }

    #[test]
    fn every_document_renders_the_layout_table() {
        let layout = EvaluationLayout::new("/tmp/repo");
        let root = layout.root();
        assert_eq!(layout.policy_file(), root.join("policy.json"));
        assert_eq!(
            layout.anchor_file("architecture-fixture", 2),
            root.join("anchors/architecture-fixture/v2.json")
        );
        assert_eq!(layout.epoch_file(0), root.join("epochs/epoch-000000.json"));
        assert_eq!(layout.epoch_file(1), root.join("epochs/epoch-000001.json"));
        assert_eq!(layout.current_file(), root.join("current.json"));
        assert_eq!(
            layout.trial_file("ct-0123456789abcdef"),
            root.join("trials/ct-0123456789abcdef.json")
        );
        assert_eq!(
            layout.promotion_file("promo-0123456789abcdef"),
            root.join("promotions/promo-0123456789abcdef.json")
        );
        assert_eq!(layout.record_file("ev-1"), root.join("records/ev-1.json"));
        assert_eq!(layout.journal_file(), root.join("events.jsonl"));
        assert_eq!(
            layout.checkpoint_file(),
            root.join("events.checkpoint.json")
        );
    }

    #[test]
    fn ensure_directories_creates_the_store_and_is_idempotent() {
        let dir = std::env::temp_dir().join(format!(
            "autospec-eval-layout-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let layout = EvaluationLayout::new(&dir);
        layout.ensure_directories().unwrap();
        for directory in layout.directories() {
            assert!(directory.is_dir(), "{}", directory.display());
        }
        layout.ensure_directories().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
