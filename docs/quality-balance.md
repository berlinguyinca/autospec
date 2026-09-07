# Quality-balance idle audit-remediation loop

The autonomous run loop can reach an idle state: no executable issues remain in
the queue, yet the repository still carries quality debt. Instead of proceeding
straight to feature discovery, the **quality-balance** loop runs a repeatable
repository audit, reconciles findings into a durable ledger, plans remediation,
and re-audits until the quality balance is met, the loop budget is exhausted,
or feature work resumes.

Core implementation: `crates/autospec-core/src/autonomous/quality_balance/`
(`QualityLedger`, `QualityBalancePolicy`, JSON codec).

## Audit dimensions (AC1)

An `AuditPass` is one deterministic, repeatable audit of the repository at one
revision. It records a sealed pass `digest` (SHA-256 over pass identity,
revision, and every finding) and covers ten fixed dimensions:

| Dimension | Scope |
|---|---|
| `Documentation` | docs, runbooks, config guides |
| `Tests` | coverage, flaky/skipped tests, smoke shape |
| `Security` | secrets, injection, unsafe patterns |
| `Dependencies` | manifest pins, known-vulnerable or unmaintained deps |
| `Performance` | regressions, unbounded growth, hot paths |
| `Architecture` | layering, duplication, cross-crate boundaries |
| `Compatibility` | API/CLI/env/manifest contract drift |
| `DataIntegrity` | persistence, migration, tamper evidence |
| `DeploymentSafety` | isolation, fail-closed behavior, operator recovery |
| `Usability` | ergonomics of scripts, runbooks, error messages |

Each `AuditFinding` carries `severity` (critical/high/medium/low), `confidence`
(low/medium/high — the observed-evidence strength), `evidence` (deterministic
description text), `affected_paths`, `regression_test_required`, and two
triage flags: `credential_gated` (the audit cannot verify it without
credentials) and `safe_to_autofix` (false means the remediation is unsafe to
apply automatically). Findings deduplicate by SHA-256 fingerprint over
dimension + evidence + paths; repeat audits of an unchanged revision produce
byte-identical fingerprints and a byte-identical pass digest.

## Ledger and reconciliation (AC2)

`QualityLedger` is the durable, append-only-by-construction state:

- `record_audit` reconciles a pass against previous entries: new findings
  open, previously open findings re-observed are re-confirmed (counted as
  regressions), findings no longer observed are marked remediated,
  false-positive dispositions are retained, and remediation-round counters
  advance.
- `remediation_plan` emits `RemediationTarget`s for open findings that are
  still remediable under the policy budget. **Credential-gated findings and
  findings with `safe_to_autofix = false` are never skipped: they stay
  `Tracked` (surfaced in `QualityGateResult.tracked`) until the operator
  closes them or re-audit shows them fixed.**
- `mark_false_positive` requires a reason, never applies to blocking
  findings, and survives repeat audits.
- `gate` enforces the quality gate: it fails while any open finding is
  blocking (severity in `blocking_severities` AND confidence at or above
  `blocking_min_confidence`).

## Policy (AC3)

Configured by the `quality_balance:` block in `.autospec/autonomous.yml`
(parsed strictly; unknown fields and duplicates are rejected):

| Key | Range | Default |
|---|---|---|
| `min_quality_share_bps` | 1..=10000 | `2500` |
| `max_reaudits` | 1..=10 | `3` |
| `max_remediation_rounds` | 1..=10 | `5` |
| `max_attempts_per_finding` | 1..=10 | `3` |
| `blocking_min_confidence` | `low`/`medium`/`high` | `high` |
| `blocking_severities` | non-empty list | `[critical]` |

`min_quality_share_bps` is the quality-balance floor: the share of recorded
work (basis points) that must be quality remediation. Below the floor the
ledger is **quality-starved** and the idle loop must remediate before
discovery; recording a verified remediation or a batch of feature work moves
the share back up.

## Idle-cycle decision (AC2/AC3)

`QualityLedger::decision` collapses state into one of four terminal,
budget-bounded outcomes:

- `ProceedToDiscovery { remaining }` — gate passes, balance met, nothing to
  plan.
- `Remediate { plan, ... }` — an executable plan exists and the loop budget
  has room.
- `Starved { plan }` — the quality share is below the floor; remediation is
  mandatory before discovery (empty queue with critical findings lands here
  when the floor is set low enough).
- `BudgetExhausted { open }` — `max_reaudits`, `max_remediation_rounds`, or
  `max_attempts_per_finding` was hit; the loop terminates deterministically
  with the still-open findings surfaced rather than looping forever.

`end_cycle` resets the per-cycle reaudit and round counters (attempts per
finding are lifetime-scoped).

## Durable persistence (AC3)

`QualityLedger::to_json` / `parse_json` round-trip the ledger with schema
version `1`. The codec:

- rejects unknown keys at every level and enforces field presence,
- recomputes each entry's finding fingerprint and the sealed pass digest on
  parse; any tampered byte (including a truncated digest or an injected key)
  is rejected,
- re-runs the ledger invariants (first/last-seen pass ordering, status
  counters) after hydration.

## Tests

`cargo test -p autospec-core quality_balance` covers: repeat-audit
idempotence (digest + reconciliation), empty-queue proceed, empty-queue
critical blocking, false-positive retention, credential-gated / unsafe
tracing, remediation failure with attempt budgets, repeat audits beyond
budget terminating with `BudgetExhausted` plus cycle-reset retry, the balance
floor starving then recovering the decision, full JSON round-trip, and
tamper rejection (injected key, wrong digest, unsupported schema).
