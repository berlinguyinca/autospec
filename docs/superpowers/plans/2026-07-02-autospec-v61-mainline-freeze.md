# V61 Mainline Freeze Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a V61 acceptance layer that freezes the V60 mainline into a truthful operator-usable baseline without adding autonomy escalation.

**Architecture:** Reuse `scripts/autospec-baseline-v25.py` as the deterministic artifact writer and command dispatcher, then expose thin shell scripts for each V61 acceptance command. V61 writes JSON/Markdown ledgers, audits, operator docs, golden paths, release-candidate packet artifacts, and a status report; Bats validates output truthfulness and boundary claims.

**Tech Stack:** Python standard library, Bash wrappers, Bats tests, Markdown/JSON artifacts.

## Global Constraints

- No new autonomy escalation.
- No auto-merge, self-approval, default-branch push, scheduler, daemon, background runner, hidden GitHub writes, or GitHub Actions.
- No production secret handling, package installs, external AI/API/model calls, auth/security/permission/migration/deployment/trading behavior changes, or broad rewrites.
- Markdown reports are primary human output.
- JSON must be deterministic and pretty-printed.
- Never overclaim readiness-only, mock-only, local-only, dry-run-only, scaffolded, partial, or human-gated behavior as executed remote capability.

---

### Task 1: V61 Failing Bats Coverage

**Files:**
- Create: `tests/autonomy-v61-mainline-freeze.bats`

**Interfaces:**
- Consumes: `scripts/autospec-v60-status.sh`, V61 script names from the user spec.
- Produces: test assertions for required artifacts and truth classifications.

- [ ] Write tests that run each V61 command against a temporary repo fixture.
- [ ] Assert acceptance ledger, capability truth audit, command catalog, golden path docs, RC packet, human approval audit, remote-write audit, post-merge validation, and V61 status exist.
- [ ] Assert remote-write canary/PR update/issue publishing/merge/auto-merge/self-approval are not overclaimed.
- [ ] Run the test and verify it fails because V61 scripts do not exist yet.

### Task 2: V61 Harness Commands

**Files:**
- Modify: `scripts/autospec-baseline-v25.py`
- Create wrappers: `scripts/autospec-v61-*.sh`

**Interfaces:**
- Consumes: status JSON for V25-V60.
- Produces: V61 commands exposed through shell wrappers.

- [ ] Add helper functions to inspect V26-V60 status outputs.
- [ ] Add capability classification and no-overclaim checks.
- [ ] Add artifact writers for all required V61 JSON/Markdown outputs.
- [ ] Add command dispatch cases for each V61 command.
- [ ] Add thin wrappers that call the Python harness with the matching command.

### Task 3: Operator Docs And RC Packet

**Files:**
- Create: `docs/operators/AUTOSPEC_COMMAND_CATALOG.md`
- Create: `docs/operators/GOLDEN_PATH_AUTOTRADE.md`
- Create: `docs/operators/GOLDEN_PATH_GENERIC_REPO.md`
- Create runtime artifacts under `.autospec/releases/v60-mainline-rc/`

**Interfaces:**
- Consumes: V61 artifact payloads.
- Produces: operator-readable docs and RC packet.

- [ ] Generate catalog entries with safety classification.
- [ ] Generate Autotrade and generic repo golden paths with dry-run/approval boundaries.
- [ ] Generate RC summary, validation checklist, known limitations, and boundary packet.

### Task 4: Validation And Commit

**Files:**
- All changed files.

**Interfaces:**
- Consumes: V61 implementation.
- Produces: local validation evidence and commit.

- [ ] Run `python3 -m py_compile scripts/autospec-runtime-v1-lib.py scripts/autospec-baseline-v25.py`.
- [ ] Run `bats tests/autonomy-v61-mainline-freeze.bats`.
- [ ] Run V60/V61 acceptance commands and platform gates from the user spec.
- [ ] Run broader Bats if practical.
- [ ] Commit locally with the Lore commit protocol.
