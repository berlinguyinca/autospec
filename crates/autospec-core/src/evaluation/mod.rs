//! Evaluator coevolution (slice 1).
//!
//! Versioned, immutable learned evaluators pinned into per-slot epochs, a
//! protected labeled anchor suite, a deterministic challenger-versus-incumbent
//! qualification statistic, and a crash-safe promotion transaction. The
//! system of record is the repo-local file store under
//! `.autospec/evaluation/` (ADR 0001 D3: the ledger is the record; any
//! database is a projection).
//!
//! See `docs/specs/2026-09-05-evaluator-coevolution-design.md`.

pub mod store;
