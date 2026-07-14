# Rust Spec-State Persistence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` for implementation and a separate review pass before completion.

**Goal:** Persist validated spec lifecycle state under `.autospec/state/specs.json` without adding dependencies or executing any work.

**Architecture:** `autospec-core::state` owns an in-memory `SpecStateStore`, fixed-schema JSON codec, validation, and recovery-aware file persistence. The store works from an explicit project root and is not wired into the CLI yet. This makes the contract reusable by queue, evidence, and read-only command cutovers without changing current wrapper behavior.

**Tech Stack:** Rust 2021 standard library, existing `SpecLifecycle`, Cargo integration tests, repository shell validation.

## Global Constraints

- No new dependencies.
- Do not change `SpecLifecycle::transition_to`, queue behavior, or any shell wrapper in this slice.
- The only persistent path is `<root>/.autospec/state/specs.json`, with a sibling `specs.json.tmp` recovery file during promotion.
- State data must be deterministic, schema-versioned, and validated on both write and read.
- All new production behavior starts with focused failing tests.

### Task 1: Add failing durable-state tests

**Files:**
- Modify: `crates/autospec-core/tests/spec_state.rs`

**Interfaces:**
- Consumes: a project root and `SpecStateStore` API.
- Produces: round-trip, recovery, and invalid-document test coverage.

- [x] Add a unique temporary project-root helper that cleans up after each test without external crates.
- [x] Add a round-trip test that writes planned/passed/deferred/superseded records, loads them back, and asserts deterministic JSON ordering.
- [x] Add recovery tests for a valid `.tmp` state document with a missing primary file and with a malformed primary file.
- [x] Add validation tests for duplicate/invalid IDs, missing deferred reason, and a supersession reference that is missing or self-referential.
- [x] Run `cargo test -p autospec-core --test spec_state` and confirm it fails before implementation.

### Task 2: Implement the validated store and codec

**Files:**
- Modify: `crates/autospec-core/src/state/mod.rs`

**Interfaces:**
- Produces: `SpecStateStore::{new, insert, get, iter, load_or_default, save, to_json}`.
- Reads/writes: `<root>/.autospec/state/specs.json` and `<root>/.autospec/state/specs.json.tmp`.

- [x] Define the schema version and deterministic store container over `BTreeMap<String, SpecLifecycle>`.
- [x] Validate lifecycle metadata and cross-entry supersession references before rendering or persisting.
- [x] Encode JSON strings correctly for quotes, slashes, control characters, and Unicode-safe UTF-8.
- [x] Parse only the documented JSON object, reject unknown keys and trailing content, and return actionable errors.
- [x] Write the temporary file completely, synchronize it, then promote it; on load, recover a valid temporary file only when the primary is unavailable or invalid.
- [x] Run the focused `spec_state` test target until it passes.

### Task 3: Align the state schema and workflow documentation

**Files:**
- Modify: `schemas/autospec-spec-state.schema.json`
- Modify: `docs/workflows.md`

**Interfaces:**
- Consumes: schema version 1 state-store document.
- Produces: documentation that distinguishes durable lifecycle state from queue execution.

- [x] Change the schema from a single lifecycle record to a versioned document with a `specs` array and lifecycle definition.
- [x] Document the state path, recovery behavior, and non-executing compatibility boundary.

### Task 4: Verify and commit the bounded slice

**Files:**
- Modify: files from Tasks 1–3 plus this plan/spec as needed.

- [x] Run `cargo test --workspace`.
- [x] Run `cargo fmt --all --check`.
- [x] Run `cargo clippy --workspace --all-targets -- -D warnings`.
- [x] Run `autospec validate --fast`.
- [x] Run the full validation script; the unchanged `/usr/bin/bash` fixture assumption fails on this macOS host before bridge execution.
- [x] Obtain an independent code review, fix verified findings, and repeat the affected checks.
- [x] Commit with a conventional Lore-format message explaining why queue/resume needs durable validated lifecycle state first.
