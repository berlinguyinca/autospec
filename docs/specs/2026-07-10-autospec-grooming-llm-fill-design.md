# Autospec grooming — LLM template-fill + canary→auto self-governance

**Status:** design approved 2026-07-10 · extends the merged v1 backlog-grooming
pipeline (PR #1736, squash `485d7d31`).

## Problem

v1 backlog-grooming auto-promotes **eligible** (legacy-drainable-as-is) issues
into the `auto-implement` queue, but the largest stuck bucket — issues at
`needs-template` (raw/discovery-filed, structurally insufficient) — still
dead-ends. v1 routes them one of two ways, neither of which makes them usable:

- `template-promote` gate **inactive** (the seed/default state) → `hold:needs-human`.
- `template-promote` gate **active** → applies a `route:groom` label and an audit
  comment… but **nothing consumes `route:groom`**. The label is a no-op.

So the `needs-template` bucket never drains autonomously. Closing that is this
enhancement: an LLM fills a valid autospec template, gated by a **canary→auto**
ratchet so autonomy is *earned* from real merge outcomes, not assumed.

## Non-goals

- Not touching the v1 eligible-promote path — it is live and correct.
- Not grooming `epic` (still routed to `/autospec-split`) or `hold` (ambiguous /
  unresolvable-dep / thin → `hold:needs-human`) decisions.
- Not adding a new govern gate. The enhancement reuses the existing two-rung
  ratchet (`eligible-promote` seed → `template-promote`); the seed state simply
  gains *canary* behavior instead of holding.

## The chicken-and-egg, and how the canary breaks it

The ratchet holds `template-promote` OFF until it observes a good clean-merge
rate for template-groomed issues — but that rate cannot exist until
`template-promote` is ON and grooming has run. The **canary window** breaks it:

- In the **seed** state, grooming still runs, but the filled template is
  **proposed for human approval**, not auto-promoted. Each human-approved groom
  that later merges cleanly is a real telemetry sample.
- Once the ratchet sees `canary_floor` template-groom clean samples **at or above
  the ambient baseline** (and `baseline_samples ≥ min-samples`), it graduates
  `template-promote` to **auto** (no human gate).
- Retract-on-regression **never drops below the seed** — so the failure mode is
  "fall back to the human gate (canary)," never "grooming off."

**Selection-bias risk (documented, mitigated):** canary samples are
*human-approved* grooms — humans filter out the bad ones — so post-graduation
*auto* grooms may be worse than the canary sample suggested. Mitigation: a
conservative `canary_floor` (default 5), and retraction sensitive enough that a
single revert/escalate cluster drops `template-promote` back to canary quickly.

## Architecture

Three new deterministic scripts around one LLM shell-out, all inside the existing
`autonomous-promote-open-issues.sh` promoter at its `needs-template` branch.

### 1. `scripts/groom-fill.sh` — the LLM template-fill

`groom-fill.sh --issue <n> --repo <owner/name> [--attempts N]`

- Fetches the raw issue (title + body) via `gh`.
- Shells out to **`codex exec`** (overridable via `AUTOSPEC_GROOM_FILL_BIN`, the
  Phase-4 peer-review precedent) with the raw issue + the autospec-template
  contract, asking for a filled template body.
- Validates the returned body with the existing **`groom-validate.sh`** (v1) —
  which wraps `lint-issue.sh`. On `{"ok":false,...}`, retries up to `--attempts`
  (default from `grooming-config.sh budget.groom_attempts_per_issue`), feeding the
  findings back as directives (the LLM-validator adaptive-retry pattern).
- **Output:** `{"ok":true,"body":"<filled template>"}` on success;
  `{"ok":false,"reason":"..."}` on codex-absent, codex-error, or
  attempts-exhausted. Fail-closed: any non-ok result → the caller **holds** the
  issue for a human (never promotes an unvalidated body).

### 2. `scripts/groom-reconcile.sh` — stamp real outcomes into the log

`groom-reconcile.sh --telemetry <jsonl> --repo <owner/name>`

- Reads the promote-time telemetry log. For each record with `outcome: null`
  whose issue is now **closed**, queries `gh` for the closing PR and stamps:
  - `closing_pr` — the PR number that closed the issue (null if closed without a PR).
  - `outcome ∈ clean | reverted | reopened | escalate | rejected` — `clean` =
    merged, not reverted, not reopened, no `escalate:human` label/verdict.
    `rejected` = closed carrying `groom:rejected`, or closed with no merged PR
    after a canary proposal (a human declined the groom). Every non-`clean`
    outcome is a non-clean sample that penalizes the groom rate — so a rejected
    canary groom correctly signals low grooming quality.
- Rewrites the log atomically (temp + `mv`). Bounded to **unresolved** records, so
  cost scales with in-flight grooms, not history.
- **Fail-closed:** a `gh` failure leaves the record `outcome: null` (unresolved →
  never counted as clean). Runs each Tier-1.5 cycle, **before** the govern tick.

### 3. `grooming-observe.sh` rework — measure *template-groom* clean rate

v1's observe lumps every `source:grooming` record (including eligible-promotes)
into `samples`. For the graduation decision we need the **template-groom** clean
rate specifically. Rework the `is_groomed` definition:

- **`samples`** now counts records with `template_groomed == true` (the canary /
  auto template grooms) whose `outcome` is resolved (non-null).
- **`baseline_samples`** counts resolved **non-template-groom** records
  (eligible-promotes + ambient issues) — the ambient population the groom rate
  must match.
- `is_clean` unchanged, but now reads the **reconciled** `outcome` field
  (`outcome == "clean"`) rather than the stubbed `reverted`/`reopened`/`verdict`
  fields v1 never populated. (Back-compat: fall back to the old field logic when
  `outcome` is absent, so pre-enhancement records don't crash the aggregate.)

`grooming-govern.sh` is **unchanged** — its widen-guard (`baseline_samples ≥
min-samples`) and retract-never-below-seed already implement the graduation and
the safe failure mode. `canary_floor` is applied by the **caller** as the
`--min-samples` value for the graduation tick (see wiring).

### 4. Promoter `needs-template` branch — canary vs auto

Replace the current no-op `route:groom` logic:

```
needs-template:
  # gate: grooming policy active AND not already proposed/rejected/attempts-spent
  fill = groom-fill.sh --issue N --repo R
  if fill.ok == false:
      audit "grooming: codex fill unavailable/failed (<reason>) — held for human"
      hold:needs-human;  record_held N "needs-template:fill-<reason>"
  else:
      apply fill.body to issue N            # body passed groom-validate → a real template
      remove needs-autospec-template        # BOTH paths: the template is now filled+valid
      if template-promote ∈ govern active set:      # GRADUATED → auto
          add-label auto-implement
          audit "grooming: auto-template-groomed (template-promote active)"
          record_routed N "groom-auto" (template_groomed:true, canary:false)
      else:                                          # SEED → canary
          add-label groom:proposed
          audit "grooming: template drafted for human approval (canary) — <diff/summary>"
          record_routed N "groom-canary" (template_groomed:true, canary:true)
```

- **Human approval signal (canary):** a human approving = adding `auto-implement`
  (and removing `groom:proposed`); rejecting = adding `groom:rejected` (→ the
  candidate is excluded from future grooming and, once the issue resolves,
  reconcile stamps `outcome: rejected` — a non-clean sample). These are plain
  labels — no new tooling.
- **Idempotency / budget:** `list-groomable.sh` already excludes `no-auto`; extend
  the promoter to skip issues already carrying `groom:proposed` or
  `groom:rejected`, and respect `budget.max_issues_per_cycle` /
  `budget.groom_attempts_per_issue`.

## Telemetry record shape (promote-time)

Emitted by the conductor when a groom is applied (extends v1's emit block):

```json
{ "issue": 1234, "source": "grooming", "template_groomed": true,
  "canary": true, "promoted_at": "2026-07-10T14:00:00Z",
  "closing_pr": null, "outcome": null }
```

Eligible-promotes keep emitting v1's shape but with `template_groomed:false` (so
they fall into `baseline_samples`, not the graduation sample).

## Config (extends v1's `grooming:` block)

```yaml
grooming:
  policy: auto              # auto | on | off   (v1, unchanged)
  budget:
    max_issues_per_cycle: 5     # v1
    groom_attempts_per_issue: 2 # v1 (reused by groom-fill.sh)
    canary_floor: 5             # NEW: approved-clean template-grooms before graduation
```

`grooming-config.sh` gains a `budget.canary_floor` key (default 5, same
env>yaml>default + fail-closed parsing as v1's other keys).

## Wiring (`scripts/lib/autospec-loop.sh`, Tier-1.5)

Per cycle, in order:
1. `groom-reconcile.sh` — stamp outcomes into the telemetry log.
2. `autonomous-promote-open-issues.sh --apply` — select → safety → classify →
   eligibility → (eligible auto-promote | **canary/auto groom** | epic-split | hold).
3. When `policy == auto`: `grooming-observe.sh` → `grooming-govern.sh tick
   --observed <json> --min-samples <canary_floor>` — graduate/retract
   `template-promote`.

## Error handling (all fail-closed)

| Failure | Behavior |
|---|---|
| `codex` CLI absent | `groom-fill` → `{ok:false,reason:codex-absent}` → hold for human |
| `codex` errors / times out | hold for human; counts an attempt |
| Filled body fails `groom-validate` after N attempts | hold for human |
| `gh` failure in reconcile | record stays `outcome:null` (never counted clean) |
| Corrupt/unparseable telemetry line | skipped (v1 observe already does this) |
| Corrupt `autospec.yml` | `grooming-config` → `policy:off` (v1 kill-switch) |

## Testing (TDD; all suites registered in `check_grooming_contract`)

`validate.sh` runs bats suites by explicit per-suite name inside `check_*`
functions — there is no blanket `find` over `tests/`. Every new `.bats` suite
must be registered in `check_grooming_contract` (the v1 gate) or it never runs in
validate, passing locally while silently ungated (the repo already ate a
regression from exactly this, #1185/#1211).

- `groom-fill.bats` — codex stubbed via `AUTOSPEC_GROOM_FILL_BIN`: ok body,
  codex-absent, codex-error, validate-fail-then-retry-succeed,
  attempts-exhausted-holds.
- `groom-reconcile.bats` — gh stubbed: null→clean, null→reverted, null→reopened,
  null→escalate, gh-failure-leaves-null, already-resolved-untouched, atomic-rewrite.
- `grooming-observe.bats` (extend v1) — template_groomed vs baseline partition;
  reconciled `outcome` drives is_clean; back-compat with pre-enhancement records.
- `promote-open-issues.bats` (extend v1) — needs-template seed→canary
  (`groom:proposed`, no auto-implement); needs-template graduated→auto
  (`auto-implement`, no `groom:proposed`); already-proposed skipped; fill-fail→hold.
- `grooming-config.bats` (extend v1) — `budget.canary_floor` resolution + default.

No DB/external mocks — codex and gh stubbed via subprocess/env-override only.

## Trio / derivation

Only scripts + config + `autospec.yml` change here; no SKILL.md trio prose edits
are required, so no `derive-trio.sh` / `gen-skill-goldens.sh` regeneration. If a
task does touch SKILL.md prose, it must re-derive the codex/opencode mirrors
(`derive-trio.sh <path> --in-place`) and regenerate skill-goldens
(`gen-skill-goldens.sh <bare-skill-name>`) in the **same commit**, or
`validate.sh`'s derive-consistency / block-expansion checks fail closed.
