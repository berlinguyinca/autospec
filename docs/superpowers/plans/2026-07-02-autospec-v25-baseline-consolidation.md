# AutoSpec V25 Baseline Consolidation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create a deterministic V25 baseline validation and release foundation that generates repository audit, spec inventory, dependency validation, documentation/test matrices, performance/quality baselines, baseline snapshot, release notes, and readiness status.

**Architecture:** Add one focused Python generator at `scripts/autospec-baseline-v25.py` and thin shell wrappers for the public commands. The generator performs local-only scans, writes pretty/sorted JSON and Markdown artifacts, and exposes subcommands for spec coverage, release validation, baseline validation, and v25 status.

**Tech Stack:** Python standard library, Bash wrappers, Bats regression tests.

## Global Constraints

- No network calls or GitHub writes.
- No package installs or dependency upgrades.
- No default branch pushes, merges, approvals, self-approval, scheduler, daemon, background runner, or hidden automation.
- Markdown reports are primary human output.
- JSON is deterministic and pretty-printed with sorted keys.
- Do not overclaim scaffolded/mock/local-only behavior as complete.

---

### Task 1: Add V25 Regression Tests

**Files:**
- Create: `tests/autonomy-v25-l3-pr-postcreation-governance.bats`

**Interfaces:**
- Consumes: public scripts `scripts/autospec-spec-coverage.sh`, `scripts/autospec-release-validation.sh`, `scripts/autospec-baseline-validation.sh`, `scripts/autospec-v25-status.sh`.
- Produces: failing tests that define V25 artifact and safety expectations.

- [ ] Write Bats tests that create a copied fixture repo, run the three requested validation scripts, verify reports/baselines/releases exist, assert `V25_BASELINE_READY=true`, and prove no network/GitHub write flags are set.
- [ ] Run the Bats file before implementation and confirm it fails because scripts are missing.

### Task 2: Implement Baseline Generator and Wrappers

**Files:**
- Create: `scripts/autospec-baseline-v25.py`
- Create: `scripts/autospec-spec-coverage.sh`
- Create: `scripts/autospec-release-validation.sh`
- Create: `scripts/autospec-baseline-validation.sh`
- Create: `scripts/autospec-v25-status.sh`

**Interfaces:**
- `autospec-baseline-v25.py --command spec-coverage|release-validation|baseline-validation|v25-status --repo-root DIR`
- Wrappers forward to the Python generator.

- [ ] Implement deterministic local scans for specs, scripts, docs, tests, examples, runbooks, release artifacts, and metrics.
- [ ] Generate all requested artifacts under `.autospec/`.
- [ ] Emit `V25_BASELINE_READY=true` only when all V25 sections pass.

### Task 3: Integrate Documentation and Handoff

**Files:**
- Modify: `docs/KNOWN_LIMITATIONS.md` if present, or include limitations in generated release notes.
- Generated: `.autospec/releases/v25.md`, `.autospec/baselines/v25-baseline.json`, `.autospec/reports/*`.

- [ ] Ensure release notes include summary, metrics, limitations, and V26 compatibility.
- [ ] Ensure v25 status JSON exists for future v26 gates.
- [ ] Run requested validation commands and the V25 Bats test.
