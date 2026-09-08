//! Hash-chained, checkpointed event journal with idempotency keys.
//!
//! `events.jsonl` is an append-only log where every committed line extends a
//! SHA-256 chain: `chain(i) = sha256(chain(i-1) || 0x00 || line(i))`,
//! seeded with `sha256("autospec-evaluation-journal-v1")`. The chain is
//! verified from the seed on every [`Journal::open`], so a single edited or
//! removed committed line fails closed with an
//! [`EvaluationErrorKind::Integrity`] error — the journal is the system of
//! record (ADR 0001 D3) and a tampered chain must never be trusted.
//!
//! Idempotency: every event carries a caller-chosen `key`. Re-appending an
//! existing key with identical content is a no-op returning `false`; reusing
//! a key with different content is an `Integrity` error. This makes every
//! multi-step transition (Task 11's promotion transaction) safely
//! re-runnable after a crash.
//!
//! Design: `docs/specs/2026-09-05-evaluator-coevolution-design.md` §Data
//! Model; plan Task 10,
//! `docs/superpowers/plans/2026-09-05-evaluator-coevolution-slice-1.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::io::{append_synced_line, atomic_write, read_json_if_exists};
use super::{EvaluationError, EvaluationErrorKind, Result};
use crate::autonomous::waterfall::sha256_hex;
use crate::evaluation::EVALUATION_SCHEMA_VERSION;

/// Chain seed material; never reused by any event line.
const CHAIN_SEED: &[u8] = b"autospec-evaluation-journal-v1";

/// Separator between the prior chain digest and the new line, matching the
/// NUL-separated digest convention used across the store.
const CHAIN_SEPARATOR: u8 = 0x00;

/// One committed journal entry. Serialized as exactly one compact JSON line
/// in `events.jsonl`; the chain covers the line bytes without the newline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEvent {
    /// Document schema version (see `EVALUATION_SCHEMA_VERSION`).
    pub schema: u64,
    /// 1-based position in the chain; strictly increasing by one.
    pub sequence: u64,
    /// Unix seconds, supplied by the caller (core never reads a clock).
    pub at: u64,
    /// Event kind, e.g. `evaluator.version.created`.
    pub kind: String,
    /// Idempotency key; unique across the whole journal.
    pub key: String,
    /// Structured payload. Always contains `repo` (injected on append).
    pub fields: BTreeMap<String, String>,
}

/// The durable checkpoint recording how much of the chain is verified:
/// `events.checkpoint.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Document schema version.
    pub schema: u64,
    /// Sequence number of the last committed event.
    pub high_watermark: u64,
    /// Chain digest after folding the first `high_watermark` lines.
    pub digest: String,
}

/// A verified, replayable view of `events.jsonl`.
#[derive(Debug, Clone)]
pub struct Journal {
    layout: super::layout::EvaluationLayout,
    events: Vec<JournalEvent>,
    keys: BTreeSet<String>,
    digest: String,
    high_watermark: u64,
    repo: String,
}

impl Journal {
    /// Replay `events.jsonl`, recompute the chain from the seed, and verify
    /// it against `events.checkpoint.json`. Fails closed:
    ///
    /// - a torn (unterminated) trailing line, a sequence gap, a duplicate
    ///   key, or a wrong schema on any committed line is `Integrity`;
    /// - a digest mismatch against the checkpoint, a journal behind (or
    ///   ahead of) the checkpoint's high watermark, a missing checkpoint
    ///   over a non-empty journal, or a journal missing behind a non-zero
    ///   checkpoint is `Integrity`;
    /// - a fresh store (neither file present) starts from the seed digest
    ///   and materializes the initial checkpoint.
    pub fn open(layout: &super::layout::EvaluationLayout) -> Result<Journal> {
        layout.ensure_directories()?;
        let checkpoint: Option<Checkpoint> = read_json_if_exists(&layout.checkpoint_file())?;
        if let Some(checkpoint) = &checkpoint {
            if checkpoint.schema != EVALUATION_SCHEMA_VERSION {
                return Err(integrity(format!(
                    "journal checkpoint schema {} expected {}",
                    checkpoint.schema, EVALUATION_SCHEMA_VERSION
                )));
            }
        }

        let raw = match std::fs::read_to_string(layout.journal_file()) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(EvaluationError::io(format!(
                    "failed to read {}: {error}",
                    layout.journal_file().display()
                )))
            }
        };

        let mut events: Vec<JournalEvent> = Vec::new();
        let mut keys: BTreeSet<String> = BTreeSet::new();
        let mut digest = seed_digest();
        let mut sequence: u64 = 0;

        if !raw.is_empty() {
            if !raw.ends_with('\n') {
                return Err(integrity(format!(
                    "torn trailing line in {}: file does not end with a newline",
                    layout.journal_file().display()
                )));
            }
            for line in raw.lines() {
                if line.is_empty() {
                    continue;
                }
                let event: JournalEvent = serde_json::from_str(line).map_err(|error| {
                    integrity(format!(
                        "unreadable committed line in {}: {error}",
                        layout.journal_file().display()
                    ))
                })?;
                if event.schema != EVALUATION_SCHEMA_VERSION {
                    return Err(integrity(format!(
                        "journal event schema {} expected {}",
                        event.schema, EVALUATION_SCHEMA_VERSION
                    )));
                }
                sequence += 1;
                if event.sequence != sequence {
                    return Err(integrity(format!(
                        "journal sequence gap at line {sequence}: event claims {}",
                        event.sequence
                    )));
                }
                if !keys.insert(event.key.clone()) {
                    return Err(integrity(format!(
                        "duplicate idempotency key in journal: {}",
                        event.key
                    )));
                }
                digest = extend_digest(&digest, line.as_bytes());
                events.push(event);
            }
        }

        let checkpoint = match checkpoint {
            Some(checkpoint) => checkpoint,
            None => {
                if !events.is_empty() {
                    return Err(integrity(format!(
                        "journal has {} events but no checkpoint at {}",
                        events.len(),
                        layout.checkpoint_file().display()
                    )));
                }
                let initial = Checkpoint {
                    schema: EVALUATION_SCHEMA_VERSION,
                    high_watermark: 0,
                    digest: digest.clone(),
                };
                write_checkpoint(layout, &initial)?;
                initial
            }
        };

        if (events.len() as u64) < checkpoint.high_watermark {
            return Err(integrity(format!(
                "journal is behind its checkpoint: {} events, high watermark {}",
                events.len(),
                checkpoint.high_watermark
            )));
        }
        if (events.len() as u64) > checkpoint.high_watermark {
            return Err(integrity(format!(
                "journal is ahead of its checkpoint: {} events, high watermark {}",
                events.len(),
                checkpoint.high_watermark
            )));
        }
        if digest != checkpoint.digest {
            return Err(integrity(format!(
                "journal chain digest {digest} does not match checkpoint {}",
                checkpoint.digest
            )));
        }

        Ok(Journal {
            layout: layout.clone(),
            events,
            keys,
            digest,
            high_watermark: checkpoint.high_watermark,
            repo: repo_name(layout),
        })
    }

    /// True if an event with this idempotency key is already committed.
    pub fn contains(&self, key: &str) -> bool {
        self.keys.contains(key)
    }

    /// All replayed events, in chain order.
    pub fn events(&self) -> &[JournalEvent] {
        &self.events
    }

    /// Sequence number of the last committed event (0 for an empty journal).
    pub fn high_watermark(&self) -> u64 {
        self.high_watermark
    }

    /// Append one event, deduplicated by `key`.
    ///
    /// Returns `true` when the event was appended, `false` when an identical
    /// event (same `kind` and `fields`, after the `repo` field is injected)
    /// is already committed — nothing is written in that case. Reusing a key
    /// with different content is an `Integrity` error. On success the line
    /// is durably appended and synced first, then the checkpoint is
    /// atomically rewritten with the new high watermark and digest, so a
    /// crash between the two writes is detected (journal ahead of
    /// checkpoint) on the next [`Journal::open`].
    pub fn append(
        &mut self,
        at: u64,
        kind: &str,
        key: &str,
        fields: BTreeMap<String, String>,
    ) -> Result<bool> {
        self.append_with_fault(at, kind, key, fields, None)
    }

    /// [`Journal::append`] with a crash-injection hook for tests: with
    /// `fail_after = Some(n)` the appended line is torn after `n` bytes and
    /// rolled back by [`append_synced_line`], leaving the journal file, the
    /// in-memory chain, and the checkpoint exactly as they were.
    #[doc(hidden)]
    pub fn append_with_fault(
        &mut self,
        at: u64,
        kind: &str,
        key: &str,
        mut fields: BTreeMap<String, String>,
        fail_after: Option<usize>,
    ) -> Result<bool> {
        fields
            .entry("repo".to_string())
            .or_insert_with(|| self.repo.clone());

        if let Some(existing) = self.events.iter().find(|event| event.key == key) {
            if existing.kind == kind && existing.fields == fields {
                return Ok(false);
            }
            return Err(integrity(format!(
                "idempotency key {key} is already used by event kind {} \
                 with different content",
                existing.kind
            )));
        }

        let event = JournalEvent {
            schema: EVALUATION_SCHEMA_VERSION,
            sequence: self.high_watermark + 1,
            at,
            kind: kind.to_string(),
            key: key.to_string(),
            fields,
        };
        let mut line = serde_json::to_vec(&event).map_err(|error| {
            EvaluationError::new(
                EvaluationErrorKind::Invariant,
                format!("failed to serialize journal event {key}: {error}"),
            )
        })?;

        // The chain covers the line bytes; the newline is framing only.
        self.digest = extend_digest(&self.digest, &line);
        line.push(b'\n');
        append_synced_line(&self.layout.journal_file(), &line, fail_after)?;

        self.high_watermark = event.sequence;
        self.events.push(event);
        self.keys.insert(key.to_string());

        let checkpoint = Checkpoint {
            schema: EVALUATION_SCHEMA_VERSION,
            high_watermark: self.high_watermark,
            digest: self.digest.clone(),
        };
        write_checkpoint(&self.layout, &checkpoint)?;
        Ok(true)
    }
}

/// Chain or checkpoint verification failure; fail closed on any of them.
fn integrity(message: impl Into<String>) -> EvaluationError {
    EvaluationError::new(EvaluationErrorKind::Integrity, message)
}

/// Chain seed: `sha256("autospec-evaluation-journal-v1")`.
fn seed_digest() -> String {
    sha256_hex(CHAIN_SEED)
}

/// `sha256(prior || 0x00 || line)` where `prior` is the lowercase hex chain
/// digest and `line` is the committed JSON line without its newline,
/// matching the `extend_journal_digest` pattern in `managed_project.rs`.
fn extend_digest(prior: &str, line: &[u8]) -> String {
    let mut buffer = Vec::with_capacity(prior.len() + 1 + line.len());
    buffer.extend_from_slice(prior.as_bytes());
    buffer.push(CHAIN_SEPARATOR);
    buffer.extend_from_slice(line);
    sha256_hex(&buffer)
}

fn write_checkpoint(
    layout: &super::layout::EvaluationLayout,
    checkpoint: &Checkpoint,
) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(checkpoint).map_err(|error| {
        EvaluationError::new(
            EvaluationErrorKind::Invariant,
            format!("failed to serialize journal checkpoint: {error}"),
        )
    })?;
    bytes.push(b'\n');
    atomic_write(&layout.checkpoint_file(), &bytes)
}

/// The repository's final path component, taken from the store root
/// (`<repo>/.autospec/evaluation`); injected as the `repo` field of every
/// event. `"unknown"` when the layout path has no usable component.
fn repo_name(layout: &super::layout::EvaluationLayout) -> String {
    layout
        .root()
        .parent() // `.autospec`
        .and_then(Path::parent) // repo root
        .and_then(|root| root.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_layout(tag: &str) -> (super::super::layout::EvaluationLayout, TempDir) {
        let dir = std::env::temp_dir().join(format!(
            "autospec-eval-journal-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let layout = super::super::layout::EvaluationLayout::new(&dir);
        layout.ensure_directories().unwrap();
        (layout, TempDir(dir))
    }

    struct TempDir(std::path::PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn append_is_idempotent_by_key_and_chained() {
        let (layout, _guard) = temp_layout("idempotent");
        let mut journal = Journal::open(&layout).unwrap();
        assert!(journal
            .append(
                1,
                "evaluator.version.created",
                "evaluator:architecture@1",
                fields(&[("slot", "architecture")]),
            )
            .unwrap());
        assert!(!journal
            .append(
                2,
                "evaluator.version.created",
                "evaluator:architecture@1",
                fields(&[("slot", "architecture")]),
            )
            .unwrap());
        assert_eq!(
            journal
                .append(
                    3,
                    "evaluator.version.created",
                    "evaluator:architecture@1",
                    fields(&[("slot", "documentation")]),
                )
                .unwrap_err()
                .kind,
            EvaluationErrorKind::Integrity
        );
        assert_eq!(journal.high_watermark(), 1);
        let reopened = Journal::open(&layout).unwrap();
        assert_eq!(reopened.events().len(), 1);
        assert!(reopened.contains("evaluator:architecture@1"));
    }

    #[test]
    fn tampered_journal_fails_closed_on_open() {
        let (layout, _guard) = temp_layout("tamper");
        let mut journal = Journal::open(&layout).unwrap();
        journal.append(1, "k", "a", BTreeMap::new()).unwrap();
        journal.append(2, "k", "b", BTreeMap::new()).unwrap();
        let text = std::fs::read_to_string(layout.journal_file())
            .unwrap()
            .replace("\"key\":\"a\"", "\"key\":\"z\"");
        std::fs::write(layout.journal_file(), text).unwrap();
        assert_eq!(
            Journal::open(&layout).unwrap_err().kind,
            EvaluationErrorKind::Integrity
        );
    }

    #[test]
    fn torn_append_is_rolled_back_and_checkpoint_stays_consistent() {
        let (layout, _guard) = temp_layout("torn");
        let mut journal = Journal::open(&layout).unwrap();
        journal.append(1, "k", "a", BTreeMap::new()).unwrap();
        assert!(journal
            .append_with_fault(2, "k", "b", BTreeMap::new(), Some(5))
            .is_err());
        let reopened = Journal::open(&layout).unwrap();
        assert_eq!(reopened.high_watermark(), 1);
        assert!(!reopened.contains("b"));
    }

    #[test]
    fn open_materializes_checkpoint_and_survives_reopen() {
        let (layout, _guard) = temp_layout("fresh");
        let mut journal = Journal::open(&layout).unwrap();
        assert_eq!(journal.high_watermark(), 0);
        assert!(journal.events().is_empty());
        assert!(std::fs::exists(layout.checkpoint_file()).unwrap());

        journal
            .append(7, "evaluation.completed", "ev-1", BTreeMap::new())
            .unwrap();
        let reopened = Journal::open(&layout).unwrap();
        assert_eq!(reopened.high_watermark(), 1);
        assert_eq!(reopened.events()[0].at, 7);
        assert_eq!(reopened.events()[0].sequence, 1);
    }

    #[test]
    fn missing_journal_behind_checkpoint_fails_closed() {
        let (layout, _guard) = temp_layout("deleted");
        let mut journal = Journal::open(&layout).unwrap();
        journal.append(1, "k", "a", BTreeMap::new()).unwrap();
        std::fs::remove_file(layout.journal_file()).unwrap();
        assert_eq!(
            Journal::open(&layout).unwrap_err().kind,
            EvaluationErrorKind::Integrity
        );
    }

    #[test]
    fn torn_trailing_line_fails_closed() {
        let (layout, _guard) = temp_layout("trailing");
        let mut journal = Journal::open(&layout).unwrap();
        journal.append(1, "k", "a", BTreeMap::new()).unwrap();
        let mut text = std::fs::read_to_string(layout.journal_file()).unwrap();
        text.pop(); // drop the final newline
        std::fs::write(layout.journal_file(), text).unwrap();
        assert_eq!(
            Journal::open(&layout).unwrap_err().kind,
            EvaluationErrorKind::Integrity
        );
    }

    #[test]
    fn every_event_carries_the_repo_field() {
        let (layout, _guard) = temp_layout("repo");
        let mut journal = Journal::open(&layout).unwrap();
        journal
            .append(
                1,
                "anchor.suite.verified",
                "suite:1",
                fields(&[("suite", "s")]),
            )
            .unwrap();
        let event = &journal.events()[0];
        assert!(event.fields.contains_key("repo"));
        assert_eq!(event.fields["suite"], "s");
    }
}
