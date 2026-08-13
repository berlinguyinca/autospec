# Security-Critical Spec and Issue Generation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Make /autospec and /autospec-define produce evidence-backed security/database specs and validated dependency-ordered implementation issues without weakening existing small-issue or safety gates.

**Architecture:** Add a PyYAML-backed deterministic sidecar validator as a standalone installed script, extend the existing YAML issue renderer to consume validated security metadata, and teach the canonical skill prompts to select and enforce the profile. Keep semantic mitigation judgment in Tier-A review while deterministic scripts enforce schema, coverage, dependencies, and queue readiness.

**Tech Stack:** Bash 3.2-compatible scripts, Python 3 with the repository's existing PyYAML convention, Bats fixtures, Markdown skill trios, derive-trio.sh, and skill golden hashes.

## Global Constraints

- No new dependency; use the existing Python/PyYAML convention and fail closed when yaml cannot be imported.
- The tracked Markdown spec remains reviewer-facing; .autospec/spec-artifacts/<slug>.security-database.yml is the deterministic sidecar.
- Ordinary generation stays unchanged unless a repaired lint rule intentionally rejects an invalid body.
- Validation failures use stable RULE_ID: description output and non-zero exits.
- Blocking prerequisites prevent auto-implement; security-profile validation never falls back to free-form filing.
- Multi-harness bodies remain byte-identical through derive-trio.sh; regenerate goldens with canonical skill edits.
- Use test-first red/green cycles for every behavior change.
- Do not repair unrelated macOS Rust autonomous-executor baseline failures.

---

### Task 1: Align issue lint with its documented contract

**Files:**
- Modify: scripts/lint-issue.sh
- Modify: tests/unit/test_lint_issue.bats
- Modify: tests/lint/test_lint_issue_sections.bats
- Create: tests/fixtures/issue-quality/bad-ac-no-token.md
- Create: tests/fixtures/issue-quality/bad-smoke-missing.md
- Create: tests/fixtures/issue-quality/bad-goal-two-sentences.md
- Create: tests/fixtures/issue-quality/good-generated-metadata.md

**Interfaces:**
- Consumes: Existing lint-issue.sh [--json] body contract.
- Produces: AC_NOT_CHECKABLE, required SMOKE_NOT_FENCED, exact-one-sentence GOAL_NOT_ONE_SENTENCE, MISSING_SECTION_DEPENDENCIES, and metadata-excluded sizing.

- [ ] Write failing Bats cases for tokenless AC, absent smoke/dependencies, two-sentence Goals, and a passing under-budget authored body whose generated metadata pushes raw words above 400.
- [ ] Run: bats tests/unit/test_lint_issue.bats tests/lint/test_lint_issue_sections.bats. Expected: new cases fail for missing behavior.
- [ ] Implement: require terminal_count = 1 and at most 30 Goal words; add AC_NOT_CHECKABLE when has_token = 0; make absent/empty smoke fail; require Dependencies; strip complete autospec-classify and autospec-shared-contracts marker sections plus UI sections before body counting.
- [ ] Run the focused Bats suites and bash -n scripts/lint-issue.sh. Expected: pass.
- [ ] Commit with Lore trailers: fix: make generated issue lint match its quality contract

---

### Task 2: Validate security/database sidecar completeness

**Files:**
- Create: scripts/validate-security-artifact.py
- Create: tests/security-artifact-validator.bats
- Create: tests/fixtures/security-artifact/valid.yml
- Create one isolated invalid fixture for each rule below.

**Interfaces:**
- Consumes: autospec.security_database.v1 YAML.
- Produces: validate-security-artifact.py [--json] artifact.yml; plain findings on stderr or JSON findings on stdout.

- [ ] Write failing fixtures/tests for PROFILE_SCHEMA_INVALID, AUTHORITATIVE_CONTROL_MISSING, CONTROL_WITHOUT_TEST, EVIDENCE_UNRESOLVED, BLOCKING_PREREQUISITE_QUEUED, DEPENDENCY_UNKNOWN, DEPENDENCY_CYCLE, SPEC_SECTION_UNCOVERED, NEGATIVE_TEST_UNOWNED, and ATOMIC_CONTRACT_SPLIT. Also cover help, malformed YAML, JSON output, and python3 -S import failure.
- [ ] Run: bats tests/security-artifact-validator.bats. Expected: fail because the validator is absent.
- [ ] Implement with yaml.safe_load, explicit type/ID checks, status and authority enums, reference coverage, prerequisite label readiness, DFS cycle detection, and capped findings. Never execute evidence commands; catch imports, YAML, and I/O as PROFILE_SCHEMA_INVALID without tracebacks.
- [ ] Run Bats, python3 -m py_compile scripts/validate-security-artifact.py, and the valid fixture. Expected: pass.
- [ ] Commit with Lore trailers: feat: fail closed on incomplete security artifacts

---

### Task 3: Make deterministic issue rendering profile-aware

**Files:**
- Modify: scripts/gen-issue-skeleton.sh
- Modify: tests/gen-issue-skeleton.bats
- Modify: tests/fixtures/gen-issue-skeleton/minimal.yaml
- Modify: tests/fixtures/gen-issue-skeleton/expected-minimal.md
- Create: tests/fixtures/gen-issue-skeleton/security-database.yaml
- Create: tests/fixtures/gen-issue-skeleton/expected-security-database.md

**Interfaces:**
- Consumes: Existing YAML plus files_touched, local_llm_notes, dependencies; optional feature_profile, evidence_consumed, controls_covered, prerequisites.
- Produces: lint-clean Markdown with exact Source spec, Files touched, Local-LLM execution notes, Dependencies, and conditional security headings.

- [ ] Add failing ordinary/security goldens. Require Files touched and Dependencies in both; require Evidence consumed, Controls covered, and Prerequisites only for security_database. Assert both pass lint and missing fields fail closed.
- [ ] Run: bats tests/gen-issue-skeleton.bats. Expected: fail on missing output.
- [ ] Extend existing scalar/list parsing and rendering. Require new common fields; conditionally require/render security lists. Preserve ordinary omission of security headings.
- [ ] Run Bats, bash -n, golden diffs, and both rendered outputs through lint-issue.sh. Expected: pass.
- [ ] Commit with Lore trailers: feat: render validated security issue context

---

### Task 4: Wire the profile into design and decomposition

**Files:**
- Modify and derive trio: skills/autospec
- Modify and derive trio: skills/autospec-define
- Modify: tests/unit/test_phase3_lint_integration.bats
- Create: tests/unit/test_security_profile_skill_contract.bats
- Regenerate: autospec and autospec-define skill goldens

**Interfaces:**
- Consumes: Validator and renderer from Tasks 2-3.
- Produces: Identical profile selection, sidecar creation, validation-before-spec-PR, graph validation, deterministic rendering, semantic portfolio review, and blocked-prerequisite behavior.

- [ ] Add failing contract assertions for feature_profile: security_database, the sidecar path, validator, renderer, autospec:blocked-prerequisite, AUTHORITATIVE_CONTROL_MISSING, validation-before-gh-create ordering, and ordinary-profile opt-out.
- [ ] Run the new contract suite and phase3 integration suite. Expected: new assertions fail.
- [ ] Update canonical SKILL.md bodies: Phase 1 emits profile/evidence; Phase 2 writes required security sections and sidecar then validates before spec PR; Phase 3 builds produces/consumes/covers/prerequisite/control/test/atomic mappings, validates, performs Tier-A portfolio review, and renders deterministically. Blocked consumers omit auto-implement.
- [ ] Add retry directives for Task 1 rule IDs. Keep schema prose compact and refer to the design spec.
- [ ] Run derive-trio.sh --in-place for both skills and gen-skill-goldens.sh autospec autospec-define.
- [ ] Run focused Bats, both derive --check commands, and git diff --check. Expected: pass.
- [ ] Commit canonical skills, mirrors, tests, and goldens atomically: feat: preserve security controls through decomposition

---

### Task 5: Refuse blocked prerequisites at implementation runtime

**Files:**
- Modify and derive trio: skills/autospec-run
- Create: tests/unit/test_autospec_run_security_prerequisites.bats
- Regenerate: autospec-run skill goldens

**Interfaces:**
- Consumes: autospec:blocked-prerequisite and Prerequisites section.
- Produces: Queue and pre-dispatch rejection with code_health:security_prerequisite_blocked.

- [ ] Add failing assertions that queue selection excludes the label, dispatch rechecks it, and security children require none or only verified prerequisite entries.
- [ ] Run focused Bats. Expected: fail.
- [ ] Update canonical run instructions to preserve and label unresolved issues, emit the exact status, and never dispatch them.
- [ ] Derive mirrors, regenerate goldens, run focused Bats and derive --check. Expected: pass.
- [ ] Commit: fix: keep unresolved security work out of the queue

---

### Task 6: Public docs, installation proof, and full validation

**Files:**
- Modify: docs/API_REFERENCE.md
- Modify: docs/USER_MANUAL.md
- Modify: tests/smoke/test_install_all_skills.bats
- Modify: crates/autospec-core/src/validation/external.rs

**Interfaces:**
- Consumes: Public validator, renderer, and queue behavior.
- Produces: Installed-helper proof and repository validation registration.

- [ ] Add failing install assertions for validate-security-artifact.py and gen-issue-skeleton.sh, plus an external validation check for validator help, valid fixture, and profile contract Bats.
- [ ] Run focused install/validation checks. Expected: fail until registration is complete.
- [ ] Document sidecar schema/path, validator exits, conditional renderer fields, blocked prerequisites, and a synthetic command sequence.
- [ ] Run all focused Bats suites, three derive --check commands, autospec validate --fast, validate-launch-readiness.sh, and git diff --check.
- [ ] Run autospec validate. If it reaches the known macOS executor compile failure, record the exact baseline signature and prove all feature-relevant checks passed before it.
- [ ] Commit: docs: expose security artifact generation controls
- [ ] Request whole-branch review against the approved design; fix every Critical/Important finding and rerun covering tests plus the full feature verification set.
