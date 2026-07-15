# Rust Specialist Scanner Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Rust specialist scanner safely reuse only validated rosters, enforce the requested specialist cap on every result, preserve the offline LLM proposal seam, and meet the repository complexity gate.

**Architecture:** Replace the monolithic scanner with four focused Rust modules: typed roster data, strict roster JSON parsing/serialization using the existing internal parser, cache ownership, and local signal scanning. The CLI receives a typed, capped roster and serializes it only after validation; invalid caches are regenerated rather than echoed. The optional `AUTOSPEC_SPECIALIST_LLM_STUB_OUTPUT` remains an offline proposal-input seam parsed by Rust, never a shell/Python path.

**Tech Stack:** Rust standard library, `crate::state::json::{JsonParser, JsonValue}`, existing Cargo integration tests.

## Global Constraints

- Do not reintroduce `scripts/explore-specialist-scan.sh`, embedded Python, Bats coverage, new dependencies, or external API calls.
- Reuse a cache only after strict schema-version, key, type, and field validation; malformed or schema-invalid input must be regenerated.
- `--num-specialists` is a maximum for generated and cached output, clamped to six.
- Preserve the `AUTOSPEC_SPECIALIST_LLM_STUB_OUTPUT` offline proposal-input contract in Rust and cap its accepted proposals.
- Keep each new Rust source file at or below 500 lines and avoid nesting deeper than four levels.
- Update issue #2039 with literal changed paths before the implementation linter runs.

---

### Task 1: Lock cache validation and cached-cap behavior with regressions

**Files:**
- Modify: `crates/autospec-core/tests/explore_specialists.rs`
- Modify: `crates/autospec-cli/tests/explore_commands.rs`

**Interfaces:**
- Consumes: `scan_specialists(ScanOptions)` and `autospec explore specialists`.
- Produces: failing tests for malformed-cache regeneration, schema-invalid-cache regeneration, and six-to-one cached output capping.

- [ ] **Step 1: Write a failing core malformed-cache test**

Add a test that writes `{"schema_version":1,"domains":"wrong","suggested_specialists":[]}` to `.autospec/explore-specialists.json`, writes `ccxt` to `requirements.txt`, calls `scan_specialists`, and asserts the resulting roster contains the trading domain and the persisted cache begins with a valid schema-one object.

- [ ] **Step 2: Run the core test to verify RED**

Run: `cargo test -p autospec-core --test explore_specialists malformed_cache_is_regenerated`

Expected: FAIL because the current cache marker check returns the invalid roster instead of re-deriving it.

- [ ] **Step 3: Write a failing CLI cached-cap test**

Seed a valid six-specialist cache, invoke `autospec explore specialists --repo-dir <fixture> --num-specialists 1`, parse stdout with `serde_json`, and assert `suggested_specialists.len() == 1`.

- [ ] **Step 4: Run the CLI test to verify RED**

Run: `cargo test -p autospec-cli --test explore_commands explore_specialists_caps_cached_roster`

Expected: FAIL because the current command emits the cached six-specialist array unchanged.

### Task 2: Split typed roster and strict JSON codec from scan orchestration

**Files:**
- Modify: `crates/autospec-core/src/explore/specialists.rs`
- Create: `crates/autospec-core/src/explore/specialists/model.rs`
- Create: `crates/autospec-core/src/explore/specialists/roster_json.rs`

**Interfaces:**
- Produces: `SpecialistRoster::capped(limit) -> SpecialistRoster`, `SpecialistRoster::to_json_pretty() -> String`, and `parse_roster_json(input: &str) -> Result<SpecialistRoster, String>`.
- Consumes: `JsonParser` and `JsonValue` from `crate::state::json`.

- [ ] **Step 1: Move models and cap behavior into `model.rs`**

Define `SpecialistRoster`, `DetectedDomain`, `FileLineEvidence`, and `SuggestedSpecialist` in `model.rs`. Implement `capped` by truncating only `suggested_specialists` to `limit.min(6)` while retaining the evidence-bearing domain list.

- [ ] **Step 2: Implement strict cache parsing in `roster_json.rs`**

Parse exactly the root keys `schema_version`, `domains`, and `suggested_specialists`; require schema version `1`; reject unknown keys; require every domain's `name`, `score`, and `evidence` fields; require every evidence and specialist field with its schema type. Return `Err` on malformed JSON, wrong type, missing key, unknown key, or invalid version.

- [ ] **Step 3: Serialize only typed rosters**

Move the existing escaping and pretty JSON rendering into `roster_json.rs`, preserving `schema_version`, domains, evidence, and specialist field names. Do not return cached raw bytes from the public scanner API.

- [ ] **Step 4: Run Task 1 tests to verify GREEN**

Run: `cargo test -p autospec-core --test explore_specialists malformed_cache_is_regenerated && cargo test -p autospec-cli --test explore_commands explore_specialists_caps_cached_roster`

Expected: both tests PASS with regenerated typed output and one cached specialist.

### Task 3: Move cache ownership and local scanning into bounded modules

**Files:**
- Create: `crates/autospec-core/src/explore/specialists/cache.rs`
- Create: `crates/autospec-core/src/explore/specialists/scan.rs`
- Modify: `crates/autospec-core/src/explore/specialists.rs`
- Modify: `crates/autospec-cli/src/commands/explore.rs`

**Interfaces:**
- `cache::load(repo_dir: &Path) -> io::Result<Option<SpecialistRoster>>` returns `None` for invalid data.
- `cache::store(repo_dir: &Path, roster: &SpecialistRoster) -> io::Result<()>` persists only `to_json_pretty()` output.
- `scan::derive(repo_dir: &Path, proposal_input: Option<&str>) -> SpecialistRoster` returns domains and uncapped specialist proposals.
- `scan_specialists_json` calls the typed scan path, applies `capped`, stores a full valid roster only when regenerating, and serializes the capped result.

- [ ] **Step 1: Move cache paths and file I/O into `cache.rs`**

Load through `parse_roster_json`, return `None` for every validation error, and make `store` create `.autospec` before writing a typed roster. Keep cache reads and writes out of the scanner loop.

- [ ] **Step 2: Move lexicon traversal and evidence recording into `scan.rs`**

Keep manifest/doc/root-path discovery, deterministic sort order, eight-evidence cap, and no-network behavior. Extract file scanning, repository-name scanning, and path scanning into separate helpers so no function exceeds the nesting limit.

- [ ] **Step 3: Re-export the public surface from `specialists.rs`**

Retain `ScanOptions`, `scan_specialists`, and `scan_specialists_json` as the public API. It must select a valid cached roster unless `force`, derive and store on cache miss, apply `capped(options.num_specialists)` to the returned roster, and serialize only that typed result.

- [ ] **Step 4: Run focused core tests**

Run: `cargo test -p autospec-core --test explore_specialists`

Expected: all domain evidence, generic, cache-force, malformed-cache, and cap tests PASS.

- [ ] **Step 5: Flatten CLI option parsing**

Extract `parse_specialist_limit` and make `replace_once` generic so `--num-specialists` validation has no nested replacement branch. Preserve the duplicate-option diagnostic and re-run `cargo test -p autospec-cli --test explore_commands`.

### Task 4: Restore the Rust-owned proposal-input seam and prove compatibility

**Files:**
- Modify: `crates/autospec-core/src/explore/specialists/scan.rs`
- Modify: `crates/autospec-core/tests/explore_specialists.rs`
- Modify: `crates/autospec-cli/tests/explore_commands.rs`
- Modify: `schemas/autospec-explore-specialists.schema.json`
- Modify: `skills/autospec-explore/SKILL.md`
- Modify: `skills/autospec-explore/codex/prompt.md`
- Modify: `skills/autospec-explore/opencode/agent.md`
- Modify: `docs/specs/2026-06-15-autospec-explore-discovery-enhance.md`

**Interfaces:**
- Consumes: optional `AUTOSPEC_SPECIALIST_LLM_STUB_OUTPUT` containing a JSON array or object with `suggested_specialists`.
- Produces: validated, normalized, capped proposal specialists; falls back to deterministic personas when the environment value is absent or invalid.

- [ ] **Step 1: Write a failing proposal-seam test**

Set `AUTOSPEC_SPECIALIST_LLM_STUB_OUTPUT` to a JSON object containing one complete `market-risk` specialist, scan a `ccxt` fixture with `force`, and assert the roster contains `market-risk` rather than the deterministic trading slug. Clear the environment in an RAII guard at test exit.

- [ ] **Step 2: Run the proposal test to verify RED**

Run: `cargo test -p autospec-core --test explore_specialists proposal_input_overrides_fallback`

Expected: FAIL because the current Rust scanner ignores the proposal-input environment value.

- [ ] **Step 3: Parse and normalize the proposal input in Rust**

Accept either an array or an object containing `suggested_specialists`; require every specialist field, canonicalize the slug to lowercase ASCII alphanumeric segments separated by one hyphen, discard invalid candidates, and truncate to the requested cap. Keep deterministic fallback only when no valid proposal candidate exists.

- [ ] **Step 4: Restore lock-step documentation**

State that the Rust scanner owns deterministic signals and the offline proposal-input seam, while no shell/Python scanner or external service is invoked. Apply the same body to the skill trio and refresh their golden hashes with the repository tool.

- [ ] **Step 5: Run the compatibility tests**

Run: `cargo test -p autospec-core --test explore_specialists proposal_input_overrides_fallback && cargo test -p autospec-cli --test explore_commands`

Expected: proposal input and command JSON behavior PASS.

### Task 5: Repair the issue contract and run the complete merge gate

**Files:**
- Modify: GitHub issue `#2039` implementation outline with every changed path.
- Modify: `docs/superpowers/plans/2026-07-15-rust-specialist-scanner-hardening.md`

**Interfaces:**
- Consumes: final PR diff and issue #2039.
- Produces: a zero-finding implementation lint and a fully validated Rust-only scanner PR.

- [ ] **Step 1: Update the issue outline with literal paths**

List the CLI command/test, specialist module root and focused submodules, core tests, validation source/tests, shell caller, deleted scanner/Bats test, schema, spec, skill trio, goldens, and smoke command. Do not add a Guardian exception for newly introduced complexity.

- [ ] **Step 2: Run the implementation gate**

Run: `cargo run -q -p autospec-cli -- lint implementation 2043 --issue 2039`

Expected: exit `0` with no OUT_OF_SCOPE, COMPLEXITY, or missing-test findings.

- [ ] **Step 3: Run complete validation**

Run: `cargo fmt --all --check && cargo test --workspace && cargo run -q -p autospec-cli -- validate --fast && bash tests/smoke/explore_metabolomics_scan.sh && git diff --check`

Expected: every command exits `0`; the scanner, direct Bats suite, and legacy runtime validation references remain deleted.

- [ ] **Step 4: Commit and re-review**

Commit a conventional Lore message with real parseable trailers, push the correction to PR #2043's head branch, and request a read-only review before merge.

## Plan self-review

- Spec coverage: Tasks 1-3 cover cache validity, cap behavior, bounded Rust ownership, and the no-legacy boundary; Task 4 restores the documented proposal seam; Task 5 proves issue-contract and full-suite compliance.
- Placeholder scan: this plan names concrete files, functions, tests, commands, and expected results; it contains no deferred implementation marker.
- Type consistency: all public callers retain `ScanOptions`, `scan_specialists`, and `scan_specialists_json`; the new modules exchange `SpecialistRoster` only.
