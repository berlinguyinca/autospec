# AutoSpec Growth — /autospec-grow-run (Plan 4 of 5)

**Status:** Design
**Date:** 2026-07-09
**Author:** berlinguyinca
**Depends on:** Plan 1 foundation, Plan 2 candidate pipeline, Plan 3 grow-define
**Feeds:** `/autospec-grow` perpetual conductor (Plan 5)

## Summary

`/autospec-grow-run` drains the growth backlog that `/autospec-grow-define`
files. It is the "produce → gate → publish" half of the growth loop, plus the
measurement backbone that closes it. It does three things:

1. **Artifact drain** — for each open `growth:artifact` issue, invoke
   `/autospec-run` (not a fork) to implement it, with an added **content-quality
   gate** running alongside the standard reviewer before the PR auto-merges.
2. **Outbound pipeline** — for each `growth:outbound` / `growth/needs-draft`
   issue, draft the content, pass it through the blocking **`growth-ethics`
   gate** plus deterministic FTC / cadence / relevance validators, then file a
   `growth/needs-approval` issue in the control repo and fire a
   **PushNotification**. Publishing is **package-only, never automated**: on
   `growth/approved` the agent hands back a one-click-publish package and the
   human posts; the agent records `growth/published` + the live URL.
3. **Measure & attribute** — run four live measurement adapters (GSC,
   analytics, GitHub, rank/backlink), normalize via Plan 1
   `growth-measure.sh --normalize`, correlate metric deltas against shipped
   ledger items, and recompute per-lens source weights that feed the next
   `/autospec-grow-define` cycle (the recursive self-improvement loop, G5).

The deterministic spine (validators, cadence counter, adapters, attribution) is
new bash/jq under `skills/autospec-shared/scripts/`; the orchestration is a
lock-step trio skill.

### Non-goals

- **No auto-publishing to third-party platforms.** Outbound is package-only;
  the human is always the publisher. No browser-assisted posting in this plan.
- **No new researcher lenses or candidate generation** — that is Plan 3.
- **No perpetual scheduling / conductor loop** — that is Plan 5. This skill runs
  one drain+measure pass per invocation.
- **No new ethics policy.** The hard-coded blocklist and pre-checks shipped in
  Plan 1 (`growth-ethics-blocklist.sh`, `growth-ethics-precheck.sh`); this plan
  consumes them.

## Design decisions (locked)

- **Adapters:** all four providers make real `curl` calls, each behind a
  `GROWTH_FETCH_CMD` seam so tests inject fixtures and CI needs no live
  credentials. Missing credentials → the adapter fails closed (emits nothing,
  non-zero) and the pass degrades to `{}` for that provider, never a crash.
- **Outbound publishing:** package-only, never auto-post. The agent posts to a
  third-party platform **never**; it produces a ready-to-paste package and the
  human publishes.

## Skill shape

A trio skill under `skills/autospec-grow-run/`: `SKILL.md` + `codex/prompt.md`
+ `opencode/agent.md`, bodies byte-identical (lock-step), derived via
`derive-trio.sh --in-place` and fingerprinted with `gen-skill-goldens.sh`.
`codex/prompt.md` carries the leading blank line the lock-step check requires.

**Required structural sections** (per repo conventions / prior new-skill
gotchas):

- **Self-update mode** — pure prose, no shelled-out user text.
- **Required capabilities & harness adapter** — the adapter row.
- **Harness detection + Model-tier** — resolve `TIER_A` (draft, gate-review,
  attribute) and `TIER_B` (cheap mechanical) once at skill start, emitted as the
  literal `**Model tier:**` directive the shared gate requires.
- **Relevant memory injection** — run-start memory query.

Opt-in: reads `.autospec/growth.yml` (Plan 1 `validate-growth-config.sh`).
Without a valid config the skill is inert (one-line pointer + exit).

## Flow

### R0 — Detect (once at start)

Harness detection → resolve `TIER_A`/`TIER_B`; load + validate `growth.yml`;
inject relevant memory; resolve the growth-shared script dir once.

### R1 — Artifact drain

Enumerate open `growth:artifact` issues (via `gh`, label-filtered). For each,
invoke `/autospec-run` scoped to that issue — the standard implement → review →
PR → auto-merge loop — with one addition: a **content-quality gate** runs
alongside the standard reviewer. The gate is a `TIER_A` reviewer subagent backed
by `growth-content-quality-precheck.sh` (deterministic pre-checks) and wrapped
in the standard **5-attempt adaptive-retry loop** that feeds findings back as
directives. Gate criteria: E-E-A-T signals present; no keyword stuffing (density
under a configurable ceiling); every factual/benchmark claim grounded or cited;
brand voice via `autospec-persona`; landing/comparison pages meet
`frontend-design` quality. The standard `growth-ethics` gate + `autospec-secaudit`
gate also run before merge. On success the PR auto-merges and the ledger line
for that issue moves to `merged_clean`.

### R2 — Outbound: draft → gate → queue

Enumerate open `growth:outbound` / `growth/needs-draft` issues. For each:

1. **Draft** (`TIER_A`) — produce the post/email body, the target venue, the
   specific self-promo rule the draft satisfies, and any required disclosure.
   The draft conforms to the **outbound-draft schema** and is validated by
   `validate-outbound-draft.sh` (fail-closed: a malformed draft is skipped and
   logged, never queued).
2. **Ethics gate** — the blocking `growth-ethics` gate: deterministic
   pre-checks (`growth-ethics-precheck.sh` + `growth-ethics-blocklist.sh`, Plan
   1) then a `TIER_A` reviewer, 5-attempt adaptive retry. A blocked draft
   records a `rejected` ledger line with the machine-readable reason and is not
   queued.
3. **Policy validators** (deterministic, all fail-closed):
   - **FTC disclosure** — `growth-content-quality-precheck.sh` disclosure check
     (token-boundary, reused) when the draft is sponsored/affiliate/incentivized.
   - **Cadence cap** — `growth-cadence-check.sh <ledger> <platform> <cap>`
     counts prior `published` lines for that platform in the window and refuses
     when at/over cap.
   - **Relevance** — the draft must carry the target venue's self-promo rule
     from config; a draft with no matching rule is refused.
4. **Queue** — a surviving draft becomes a `growth/needs-approval` issue in
   `approval.control_repo` (config), body built by `growth-outbound-queue.sh`
   (ready-to-paste content + target link + rationale + the rule satisfied). A
   `pending` ledger line records the queued draft. A **PushNotification** fires
   summarizing new drafts ready for approval.

### R3 — Handle approval outcomes

For each `growth/needs-approval` issue in the control repo, read its current
label state (`growth-outbound-queue.sh --read-state` parses labels; string
equality, never regex — no metacharacter injection):

- **`growth/approved`** → the agent produces a **one-click-publish package**
  (the exact body + target URL + the rule satisfied) as an issue comment for the
  human to post. It does **not** post. When the human confirms publication
  (replies with the live URL / applies `growth/published`), the agent records a
  `published` ledger line with the live URL.
- **body edited** → re-gate (R2 steps 2–3) and re-queue; the ledger records the
  re-gate outcome.
- **`growth/rejected`** → a `rejected` ledger line (full seen-set dedup keeps it
  from being re-proposed next cycle).

This control channel is the same live-steering surface the Plan 5 conductor
uses (mirrors `autospec-autonomous`).

### R4 — Measure & attribute (G5)

1. **Fetch** — run the four adapters for each provider present in config; each
   adapter fetches via the `GROWTH_FETCH_CMD` seam and pipes raw output to
   `growth-measure.sh --normalize <provider>`. Missing creds / fetch error →
   that provider degrades to `{}` (logged), never aborts the pass.
2. **Attribute** — `growth-attribute.sh <metrics-before> <metrics-after> <ledger>`
   correlates metric deltas over the configured lag window with shipped ledger
   items, emitting a per-lens contribution table.
3. **Re-weight** — feed the contribution table into `growth-source-weights.sh`
   (Plan 1) to recompute per-lens weights and write a learnings memo. The next
   `/autospec-grow-define` G1 fan-out reads these weights.

### Stop

Print a summary (artifacts drained/merged; drafts queued/blocked/rejected;
approvals handled; providers measured; weight deltas) and hand off to the Plan 5
conductor (or exit). One pass per invocation.

## Components (new, `skills/autospec-shared/scripts/`)

All `set -euo pipefail`, bash 3.2 compatible, with bats suites under
`tests/unit/`. Each installs via `SHARED_LIB_SCRIPT_FILES` in the skill's
`install.sh` (resolve_shared_lib_scripts_dir → `skills/autospec-shared/scripts/`;
NOT the repo-root `SHARED_SCRIPT_FILES` group — the Plan 3 install bug).

1. **`schemas/growth-outbound-draft.schema.json` + `validate-outbound-draft.sh`**
   — draft record `{issue, platform, target_url, body, self_promo_rule,
   disclosure?, evidence[]}`. Required-field + type validation, fail-closed.
2. **`growth-cadence-check.sh <ledger> <platform> <cap>`** — deterministic
   per-platform cadence counter over `published` ledger lines in the window;
   exit 0 = under cap, non-zero = at/over. Platform matched by string equality
   (capture()+== per the jq-injection rule), not regex.
3. **`growth-content-quality-precheck.sh <content-file>`** — deterministic
   pre-checks: keyword-density ceiling, FTC-disclosure token-boundary presence,
   citation/link presence. Emits a machine-readable findings list; non-zero when
   a hard pre-check fails. Feeds both the content-quality gate (R1) and the FTC
   check (R2).
4. **`growth-outbound-queue.sh`** — `--build-body` composes the
   `growth/needs-approval` issue body from a validated draft; `--read-state
   <labels>` maps the control issue's labels to `approved|edited|rejected|pending`
   (string equality). No `gh` calls inside (the skill shells `gh`); this is the
   deterministic body/label logic so it is unit-testable.
5. **`growth-attribute.sh <before.json> <after.json> <ledger.jsonl>`** —
   per-lens contribution table from metric deltas over the lag window; feeds
   `growth-source-weights.sh`.
6. **Four adapters** — `growth-adapter-gsc.sh`, `growth-adapter-analytics.sh`
   (Plausible + GA4 by config), `growth-adapter-github.sh`,
   `growth-adapter-rank.sh`. Each: read creds from the config-named env var →
   fetch via `GROWTH_FETCH_CMD` (default `curl`, overridable for tests) → emit
   raw JSON → callable through `growth-measure.sh --normalize`. Missing creds →
   non-zero + empty, fail-closed.

### `validate.sh` wiring

- `check_grow_run_pipeline_contract` — `bash -n` on the new scripts + runs their
  bats suites; enumerated in `main`'s run list (net-new suites, gate-atomicity
  rule). Registered next to `check_grow_define_contract`.
- `check_grow_run_contract` — structural + lock-step for the trio skill
  (Self-update mode, adapter row, `**Model tier:**` directive, the R0–R4 phase
  headings).

## Reuse map

**Reused (no fork):**

- `autospec-run` — drains `growth:artifact` issues (invoked, filtered by label).
- `autospec-review` / `autospec-secaudit` — standard review + security gate.
- `autospec-persona` — brand voice for drafts and artifact content.
- `growth-ethics-blocklist.sh` / `growth-ethics-precheck.sh` (Plan 1) — the
  ethics gate's deterministic backing.
- `growth-measure.sh` / `growth-source-weights.sh` / `growth-ledger.sh` (Plan 1)
  — normalize, re-weight, ledger append.
- `PushNotification` — outbound-ready alert.
- `autospec-autonomous` — control-channel / live-steering pattern.

**New (each with a named consumer):**

- Outbound-draft schema + validator — consumed by R2.
- `growth-cadence-check.sh` — consumed by R2 policy validators.
- `growth-content-quality-precheck.sh` — consumed by R1 gate + R2 FTC check.
- `growth-outbound-queue.sh` — consumed by R2 queue + R3 state read.
- `growth-attribute.sh` — consumed by R4, feeding the next define cycle.
- 4 measurement adapters — consumed by R4 (and Plan 5's G0 baseline).

## Error handling (fail-closed throughout)

- No/invalid `growth.yml` → inert exit with pointer.
- A single artifact issue whose `/autospec-run` fails → logged, ledger
  `failed`, the drain continues (no one issue aborts the pass).
- Malformed draft, blocked ethics gate, over-cadence, or missing self-promo rule
  → the draft is not queued; a machine-readable reason is recorded.
- Missing adapter credentials or a fetch error → that provider degrades to `{}`;
  the measure pass continues.
- The agent never publishes to a third-party platform; a `published` ledger line
  is written only after the human confirms the live URL.
- Every gate/queue/attribution decision is logged to the ledger with a reason.

## Testing (validation shell scripts + bats, per repo convention)

- **Outbound-draft validator:** valid draft passes; each missing required field
  / bad type / empty body → rejected; malformed JSON → rejected.
- **Cadence check:** under cap → exit 0; at/over cap → non-zero; platform match
  is exact (a draft for `reddit` is not counted against `reddit-r-foo`); window
  boundary honored. Ledger built via real `growth-ledger.sh --append` (never a
  self-consistent fixture — the Plan 1 cadence bug was exactly that).
- **Content-quality pre-check:** keyword-stuffed content fails density; missing
  disclosure on sponsored content fails; uncited benchmark claim fails; clean
  content passes; `#adventure` does not satisfy `#ad` (token boundary).
- **Outbound-queue:** `--build-body` renders the ready-to-paste package from a
  validated draft; `--read-state` maps each label set to the right state and
  is injection-safe (a crafted label string cannot flip the state).
- **Attribution:** a lens whose shipped items precede a positive metric delta
  gets positive contribution; deterministic ordering; empty ledger → empty
  table.
- **Adapters:** with `GROWTH_FETCH_CMD` pointed at a fixture, each adapter emits
  raw JSON that `growth-measure.sh --normalize` accepts; missing cred env var →
  non-zero + empty (fail-closed), never a crash.
- **Skill:** lock-step + goldens; frontmatter parse ×3; structural-section
  check; **smoke test (bats)** with subprocess mocks for `/autospec-run`, `gh`,
  subagent dispatch, and `PushNotification` — assert: an artifact issue routes
  through the drain; a malformed draft is dropped; a blocked draft records
  `rejected` and is not queued; an over-cadence draft is refused; a clean draft
  is queued with a `pending` ledger line + notification; `growth/approved`
  produces a package but never posts; measure degrades to `{}` on missing creds.
- **Install/uninstall** `bash -n` clean; skill registered in the installer with
  the growth scripts in `SHARED_LIB_SCRIPT_FILES`; root `install.sh --hook-mode`
  stays green.

## Open questions

- Attribution lag-window default — the master design leaves this configurable;
  start with a config value (e.g. 14 days) and tune once real cycles land.
- Whether GA4 (OAuth service-account) and GSC (OAuth) share one token-exchange
  helper or each adapter handles its own — default: each adapter self-contained
  behind the fetch seam; extract a shared helper only if duplication proves real
  (ROI-check).
- Keyword-density ceiling default for the content-quality pre-check — start
  conservative (config-overridable), revisit from gate block-rate.
