# Rust Evidence Persistence Design

**Date:** 2026-07-12
**Status:** implemented
**Depends on:** [Rust Spec-State Persistence Design](2026-07-12-rust-spec-state-persistence-design.md), [Rust Execution-Queue Persistence Design](2026-07-12-rust-execution-queue-persistence-design.md)

## Goal

Persist local Rust evidence bundles at `.autospec/evidence/<run-id>/bundle.json` so later report and CLI layers can consume validated proof rather than success-shaped in-memory output.

## Scope

This slice makes the existing `EvidenceBundle` schema-versioned, timestamped, deterministic, and durable. It validates run IDs and artifact paths, writes through a recovery file, and loads a named bundle only when the document is structurally valid. It does not run validation commands, invoke agents, upload artifacts, or enable CLI execution.

Each command record stores its command, exit code, stdout path, stderr path, and capture timestamp. Artifact and log paths must be relative, normalized paths beneath `.autospec/evidence/<run-id>/`; the loader rejects traversal, absolute paths, duplicate artifacts, and mismatched run IDs.

On platforms where the standard library cannot synchronize directories, durable saves fail explicitly instead of claiming crash-safe persistence.

## Acceptance criteria

- A bundle round-trips through `.autospec/evidence/<run-id>/bundle.json` without changing command or artifact order.
- A complete temporary bundle recovers a missing or malformed primary document; a valid primary wins over stale recovery data.
- Invalid run IDs, mismatched run IDs, traversal paths, duplicate artifacts, malformed records, and unknown keys fail clearly.
- Existing Markdown and JSON release reports remain compatible and properly escape JSON control characters.
- Rust workspace tests, formatting, clippy, and fast repository validation pass.

## Non-goals

Command execution and remote artifact storage remain out of scope. Agent-result ingestion and core CLI productization consume this persisted local evidence in later slices.
