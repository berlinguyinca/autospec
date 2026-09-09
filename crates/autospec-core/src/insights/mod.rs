//! Continuous-improvement-engine insights subsystem
//! (`docs/specs/2026-09-08-continuous-improvement-engine.md`).
//!
//! Deterministic extraction lands first (§4.1: "Facts that can be extracted
//! mechanically MUST be extracted mechanically"); semantic enrichment
//! (#3832) is a separate, later pass that never re-derives the numbers here.

pub mod summarize;
