//! Hash-chained, idempotency-keyed event journal for the evaluation store.
//!
//! Design: `docs/specs/2026-09-05-evaluator-coevolution-design.md` (Data
//! model). Events are appended to `events.jsonl`, one compact JSON line per
//! event, through [`append_synced_line`] so a torn append rolls the file
//! back to its pre-append length.
//!
//! The chain digest starts at `sha256("autospec-evaluation-journal-v1")`
//! and extends with `extend(prior, line) = sha256(prior ‖ 0x00 ‖ line)`
//! over the stored line bytes without the trailing newline — the same rule
//! as `managed_project`'s `extend_journal_digest`.
//! `events.checkpoint.json` pins the high watermark and the chain digest at
//! that watermark; [`Journal::open`] replays the journal and fails closed
//! with [`EvaluationErrorKind::Integrity`] when the journal is behind the
//! checkpoint, the chain digest at the watermark differs, or the sequence or
//! key of any event is corrupted. A mismatch blocks promotion: the store
//! refuses to advance on a journal it cannot verify.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::autonomous::waterfall::sha256_hex;
use crate::evaluation::store::io::{append_synced_line, atomic_write};
use crate::evaluation::store::layout::EvaluationLayout;
use crate::evaluation::store::{EvaluationError, EvaluationErrorKind, Result};

/// Journal schema version stamped on every event and checkpoint.
pub const JOURNAL_SCHEMA_VERSION: u64 = 1;

/// Seed literal for the chain digest.
const CHAIN_SEED: &str = "autospec-evaluation-journal-v1";

/// The chain digest of an empty journal: `sha256(CHAIN_SEED)`.
fn seed_digest() -> String {
    sha256_hex(CHAIN_SEED.as_bytes())
}

/// `extend(prior, line) = sha256(prior ‖ 0x00 ‖ line)`; `line` is the stored
/// line bytes without the trailing newline.
fn extend_digest(prior: &str, line: &[u8]) -> String {
    let mut input = Vec::with_capacity(prior.len() + 1 + line.len());
    input.extend_from_slice(prior.as_bytes());
    input.push(0);
    input.extend_from_slice(line);
    sha256_hex(&input)
}

fn integrity(message: impl Into<String>) -> EvaluationError {
    EvaluationError::new(EvaluationErrorKind::Integrity, message)
}

fn is_digest_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// One appended event, serialized as a single compact JSON line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEvent {
    pub schema: u64,
    pub sequence: u64,
    pub at: u64,
    pub kind: String,
    pub key: String,
    pub fields: BTreeMap<String, String>,
}

/// Persisted high watermark and chain digest (`events.checkpoint.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub schema: u64,
    pub high_watermark: u64,
    pub digest: String,
}

/// In-memory replay of the journal: events, a key index for idempotency, and
/// the running chain digest.
#[derive(Debug)]
pub struct Journal {
    events: Vec<JournalEvent>,
    index: BTreeMap<String, usize>,
    digest: String,
    journal: PathBuf,
    checkpoint: PathBuf,
}

impl Journal {
    /// Replay `events.jsonl` and verify it against
    /// `events.checkpoint.json`. Fails closed with `Integrity` when the
    /// journal is behind the checkpoint, the chain digest at the watermark
    /// differs, or the sequence or key of any event is corrupted.
    pub fn open(layout: &EvaluationLayout) -> Result<Journal> {
        let journal = layout.journal_file();
        let checkpoint = layout.checkpoint_file();

        let expected = if checkpoint.exists() {
            let text = std::fs::read_to_string(&checkpoint).map_err(|err| {
                EvaluationError::io(format!(
                    "failed to read journal checkpoint {}: {err}",
                    checkpoint.display()
                ))
            })?;
            let parsed: Checkpoint = serde_json::from_str(&text).map_err(|err| {
                integrity(format!(
                    "journal checkpoint is not a checkpoint document: {err}"
                ))
            })?;
            if parsed.schema != JOURNAL_SCHEMA_VERSION {
                return Err(integrity(format!(
                    "journal checkpoint has unsupported schema {}",
                    parsed.schema
                )));
            }
            if !is_digest_hex(&parsed.digest) {
                return Err(integrity(
                    "journal checkpoint digest is not 64 lowercase hex characters",
                ));
            }
            Some(parsed)
        } else {
            None
        };

        let (events, index, digest) = replay_journal(&journal, &expected)?;
        Ok(Journal {
            events,
            index,
            digest,
            journal,
            checkpoint,
        })
    }

    /// Append one event. Returns `Ok(false)` and changes nothing when `key`
    /// already exists with identical `kind` + `fields`; returns `Integrity`
    /// when the key exists with different content. Otherwise assigns
    /// `sequence = high_watermark + 1`, appends a synced line, updates the
    /// in-memory digest, and atomically rewrites the checkpoint.
    pub fn append(
        &mut self,
        at: u64,
        kind: &str,
        key: &str,
        fields: BTreeMap<String, String>,
    ) -> Result<bool> {
        if self.key_exists(kind, key, &fields)? {
            return Ok(false);
        }
        self.append_with_fault(at, kind, key, fields, None)?;
        Ok(true)
    }

    /// Append with a crash hook for tests: when `fail_after` is `Some(n)`,
    /// the line write is truncated to at most `n` bytes, the file is rolled
    /// back to its pre-append length, and an error is returned — leaving the
    /// on-disk state and this in-memory journal unchanged.
    #[doc(hidden)]
    pub fn append_with_fault(
        &mut self,
        at: u64,
        kind: &str,
        key: &str,
        fields: BTreeMap<String, String>,
        fail_after: Option<usize>,
    ) -> Result<()> {
        if self.key_exists(kind, key, &fields)? {
            return Ok(());
        }
        let event = JournalEvent {
            schema: JOURNAL_SCHEMA_VERSION,
            sequence: self.high_watermark() + 1,
            at,
            kind: kind.to_string(),
            key: key.to_string(),
            fields,
        };
        let line = serde_json::to_vec(&event)
            .map_err(|err| integrity(format!("failed to serialize journal event: {err}")))?;
        let mut stored = line.clone();
        stored.push(b'\n');
        append_synced_line(&self.journal, &stored, fail_after)?;
        // A fault already rolled the file back before reaching this point
        // (the error would have short-circuited), so in-memory state now
        // advances in lockstep with the durably stored line.
        self.digest = extend_digest(&self.digest, &line);
        self.index.insert(event.key.clone(), self.events.len());
        self.events.push(event);
        self.write_checkpoint()?;
        Ok(())
    }

    /// Idempotency check shared by `append` and `append_with_fault`: returns
    /// `Ok(true)` when `key` is already present with identical content,
    /// `Integrity` when it is present with different content, `Ok(false)`
    /// when it is absent. Timestamps may differ across retries: `at` is not
    /// part of the identity.
    fn key_exists(&self, kind: &str, key: &str, fields: &BTreeMap<String, String>) -> Result<bool> {
        match self.index.get(key) {
            None => Ok(false),
            Some(i) => {
                let prior = &self.events[*i];
                if prior.kind == kind && prior.fields == *fields {
                    Ok(true)
                } else {
                    Err(integrity(format!(
                        "journal key {key} already exists with different content (at {})",
                        prior.at
                    )))
                }
            }
        }
    }

    fn write_checkpoint(&self) -> Result<()> {
        let checkpoint = Checkpoint {
            schema: JOURNAL_SCHEMA_VERSION,
            high_watermark: self.high_watermark(),
            digest: self.digest.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&checkpoint)
            .map_err(|err| integrity(format!("failed to serialize checkpoint: {err}")))?;
        let mut stored = bytes;
        stored.push(b'\n');
        atomic_write(&self.checkpoint, &stored)
    }

    /// Whether an event with this key has already been appended.
    pub fn contains(&self, key: &str) -> bool {
        self.index.contains_key(key)
    }

    /// Replayed events in sequence order.
    pub fn events(&self) -> &[JournalEvent] {
        &self.events
    }

    /// Sequence number of the last event (0 for an empty journal).
    pub fn high_watermark(&self) -> u64 {
        self.events.len() as u64
    }
}

/// Verify one replayed line against the running chain and record the event.
fn replay_line(
    line: &str,
    events: &[JournalEvent],
    index: &BTreeMap<String, usize>,
    digest: &str,
    expected: &Option<Checkpoint>,
) -> Result<(JournalEvent, String)> {
    let event: JournalEvent = serde_json::from_str(line)
        .map_err(|err| integrity(format!("journal line is not an event document: {err}")))?;
    if event.schema != JOURNAL_SCHEMA_VERSION {
        return Err(integrity(format!(
            "journal event {} has unsupported schema {}",
            event.sequence, event.schema
        )));
    }
    let sequence = events.len() as u64 + 1;
    if event.sequence != sequence {
        return Err(integrity(format!(
            "journal sequence gap: expected {sequence}, found {}",
            event.sequence
        )));
    }
    if index.contains_key(&event.key) {
        return Err(integrity(format!(
            "journal key {} appears more than once",
            event.key
        )));
    }
    let digest = extend_digest(digest, line.as_bytes());
    if let Some(checkpoint) = expected {
        if checkpoint.high_watermark == sequence && checkpoint.digest != digest {
            return Err(integrity(format!(
                "journal digest at sequence {sequence} ({digest}) does not match checkpoint digest {}",
                checkpoint.digest
            )));
        }
    }
    Ok((event, digest))
}

/// Replay and verify the journal file end to end. Returns the events, the
/// key-to-event-index map, and the chain digest after the final line.
fn replay_journal(
    journal: &Path,
    expected: &Option<Checkpoint>,
) -> Result<(Vec<JournalEvent>, BTreeMap<String, usize>, String)> {
    let mut events: Vec<JournalEvent> = Vec::new();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    let mut digest = seed_digest();
    if journal.exists() {
        let text = std::fs::read_to_string(journal).map_err(|err| {
            EvaluationError::io(format!(
                "failed to read journal {}: {err}",
                journal.display()
            ))
        })?;
        for line in text.lines() {
            if line.is_empty() {
                continue;
            }
            let (event, line_digest) = replay_line(line, &events, &index, &digest, expected)?;
            index.insert(event.key.clone(), events.len());
            digest = line_digest;
            events.push(event);
        }
    }
    if let Some(checkpoint) = expected {
        if checkpoint.high_watermark > events.len() as u64 {
            return Err(integrity("journal is behind its checkpoint high watermark"));
        }
    }
    Ok((events, index, digest))
}

#[cfg(test)]
mod tests {
    use super::{extend_digest, seed_digest, Journal};
    use crate::evaluation::store::layout::EvaluationLayout;

    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    struct TempGuard(PathBuf);

    impl Drop for TempGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_layout() -> (EvaluationLayout, TempGuard) {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("autospec-eval-journal-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        (EvaluationLayout::new(root.clone()), TempGuard(root))
    }

    #[test]
    fn append_is_idempotent_by_key_and_chained() {
        let (layout, _guard) = temp_layout();
        let mut j = Journal::open(&layout).unwrap();
        assert!(j
            .append(
                1,
                "evaluator.version.created",
                "evaluator:architecture@1",
                fields(&[("slot", "architecture")])
            )
            .unwrap());
        // Same key with identical kind + fields is a no-op, even at a
        // different timestamp.
        assert!(!j
            .append(
                2,
                "evaluator.version.created",
                "evaluator:architecture@1",
                fields(&[("slot", "architecture")])
            )
            .unwrap());
        // Same key with different fields is a conflicting reuse: Integrity.
        assert_eq!(
            j.append(
                3,
                "evaluator.version.created",
                "evaluator:architecture@1",
                fields(&[("slot", "documentation")])
            )
            .unwrap_err()
            .kind,
            crate::evaluation::store::EvaluationErrorKind::Integrity
        );
        assert_eq!(j.high_watermark(), 1);
        let reopened = Journal::open(&layout).unwrap();
        assert_eq!(reopened.events().len(), 1);
        assert!(reopened.contains("evaluator:architecture@1"));
    }

    #[test]
    fn tampered_journal_fails_closed_on_open() {
        let (layout, _guard) = temp_layout();
        let mut j = Journal::open(&layout).unwrap();
        j.append(1, "k", "a", BTreeMap::new()).unwrap();
        j.append(2, "k", "b", BTreeMap::new()).unwrap();
        // Edit a committed line: the chain digest can no longer be rebuilt.
        let text = std::fs::read_to_string(layout.journal_file())
            .unwrap()
            .replace("\"key\":\"a\"", "\"key\":\"z\"");
        std::fs::write(layout.journal_file(), text).unwrap();
        assert_eq!(
            Journal::open(&layout).unwrap_err().kind,
            crate::evaluation::store::EvaluationErrorKind::Integrity
        );
    }

    #[test]
    fn torn_append_is_rolled_back_and_checkpoint_stays_consistent() {
        let (layout, _guard) = temp_layout();
        let mut j = Journal::open(&layout).unwrap();
        j.append(1, "k", "a", BTreeMap::new()).unwrap();
        assert!(j
            .append_with_fault(2, "k", "b", BTreeMap::new(), Some(5))
            .is_err());
        // The partial line was rolled back; the journal reopens with the
        // checkpoint's watermark and no trace of key "b".
        let reopened = Journal::open(&layout).unwrap();
        assert_eq!(reopened.high_watermark(), 1);
        assert!(!reopened.contains("b"));
        // The in-memory journal that suffered the fault is still consistent.
        assert_eq!(j.high_watermark(), 1);
        assert!(!j.contains("b"));
        // And the append can now succeed cleanly.
        assert!(j.append(2, "k", "b", BTreeMap::new()).unwrap());
        assert_eq!(j.high_watermark(), 2);
    }

    #[test]
    fn chain_digest_matches_the_seed_and_extension_rule() {
        let (layout, _guard) = temp_layout();
        let mut j = Journal::open(&layout).unwrap();
        j.append(1, "k", "a", fields(&[("repo", "demo")])).unwrap();
        let checkpoint: super::Checkpoint =
            serde_json::from_str(&std::fs::read_to_string(layout.checkpoint_file()).unwrap())
                .unwrap();
        assert_eq!(checkpoint.high_watermark, 1);
        let journal_text = std::fs::read_to_string(layout.journal_file()).unwrap();
        let line = journal_text.lines().next().unwrap();
        let expected = extend_digest(&seed_digest(), line.as_bytes());
        assert_eq!(checkpoint.digest, expected);
        assert_ne!(checkpoint.digest, seed_digest());
    }

    #[test]
    fn journal_behind_checkpoint_fails_closed() {
        let (layout, _guard) = temp_layout();
        let mut j = Journal::open(&layout).unwrap();
        j.append(1, "k", "a", BTreeMap::new()).unwrap();
        j.append(2, "k", "b", BTreeMap::new()).unwrap();
        // Truncate the journal to one event but keep the two-event checkpoint.
        let text = std::fs::read_to_string(layout.journal_file()).unwrap();
        let first_line_end = text.find('\n').unwrap() + 1;
        std::fs::write(layout.journal_file(), &text[..first_line_end]).unwrap();
        assert_eq!(
            Journal::open(&layout).unwrap_err().kind,
            crate::evaluation::store::EvaluationErrorKind::Integrity
        );
    }

    #[test]
    fn sequence_gap_fails_closed() {
        let (layout, _guard) = temp_layout();
        let mut j = Journal::open(&layout).unwrap();
        j.append(1, "k", "a", BTreeMap::new()).unwrap();
        j.append(2, "k", "c", BTreeMap::new()).unwrap();
        // Skip sequence 2: the replay expects 2, finds 3.
        let text = std::fs::read_to_string(layout.journal_file())
            .unwrap()
            .replace("\"sequence\":2", "\"sequence\":3");
        std::fs::write(layout.journal_file(), text).unwrap();
        assert_eq!(
            Journal::open(&layout).unwrap_err().kind,
            crate::evaluation::store::EvaluationErrorKind::Integrity
        );
    }
}
