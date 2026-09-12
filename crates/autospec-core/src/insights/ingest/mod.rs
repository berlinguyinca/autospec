//! Session Ingestion Service (spec §6, issue #3836).
//!
//! Discover raw session sources, read them incrementally, normalize them
//! into the §7 event model, and write them idempotently into the §34
//! `sessions` / `session_events` tables. Corrupted records are quarantined
//! instead of stopping the run (spec §50: "corrupted session -> quarantine
//! session and continue").
//!
//! Adapters: `pi` (Pi JSONL sessions, this issue). Codex / Claude Code /
//! OpenCode adapters, enrichment and summarisation are out of scope here.

pub mod pi;

pub use pi::{ingest, PiAdapter};

/// Outcome of one [`ingest`](pi::ingest) run over every discovered
/// session source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IngestReport {
    /// Session sources the adapter discovered.
    pub sessions_discovered: u64,
    /// Raw records read from the sources (after applying stored cursors).
    pub events_read: u64,
    /// Rows written to `session_events`.
    pub events_inserted: u64,
    /// Rows skipped by `ON CONFLICT (event_id) DO NOTHING`.
    pub events_deduplicated: u64,
    /// Rows written to `insights_quarantine`.
    pub quarantined: u64,
    /// Sources whose stored resume cursor advanced.
    pub cursors_updated: u64,
}

/// Where the resume cursor of each source URI is stored.
///
/// The cursor is the stored half of the [`SessionRef`] contract
/// (`events::SessionRef::resume_cursor`): one row per source URI in
/// `table`, carrying the adapter-defined cursor value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngestCursor {
    /// Table name (rows: `uri` primary key, `resume_cursor`,
    /// `updated_at`).
    pub table: &'static str,
}

impl IngestCursor {
    /// Default cursor table.
    pub const DEFAULT_TABLE: &'static str = "insights_ingest_cursors";
}

impl Default for IngestCursor {
    fn default() -> Self {
        Self {
            table: Self::DEFAULT_TABLE,
        }
    }
}
