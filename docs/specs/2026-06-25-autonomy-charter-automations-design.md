# Autonomy Charter automations — design spec

**Date:** 2026-06-25
**Source charter:** `docs/AUTONOMY-CHARTER.md` §5 ("Still to build")
**Origin:** session-transcript mining (see `docs/memory/project_babysit_tax_autonomy_charter.md`).

## Goal

Eliminate the two operator-babysitting patterns that the Autonomy Charter named
but did not yet automate: (1) the queue-drain stall ("standing by") and (2)
async-wait silence ("how do they look?"). Both are `autospec-run` /
`autospec-shared` changes.

## Team personality

**Reliability/backend automation team** — orchestration engineer (monitor loop),
platform/shell engineer (sentinels, notifiers), test engineer (bats with mocked
external boundaries). Fits because both features are control-flow + side-effect
plumbing in the autonomous monitor, where the risks are runaway loops and
cross-platform notifier portability.

**Review counter-team** — operations + safety lens: challenge "does the
drain→explore chain ever loop unbounded?", "does a notifier failure ever block
the merge path?", "does this fire during CI/headless runs where notifications
are noise?".

---

## Feature 1 — Queue-drain → autospec-explore auto-chain

### Problem
When the Phase 4 monitor drains the queue it writes `status=ALL_DONE` to
`~/.autospec/batch-done.json` and the orchestrator exits with a "standing by"
report. The operator's reflexive next instruction is always "review the whole
project for gaps" — which is exactly what `/autospec-explore` does.

### Design
Hook the existing `ALL_DONE` branch in `skills/autospec-run/SKILL.md` (the
orchestrator relaunch loop, ~the `if [ "$status" = "ALL_DONE" ]` block). Add an
**opt-in** sentinel so default behavior is byte-unchanged:

- `~/.autospec/explore-on-drain.flag` **absent** → current behavior (write final
  report, exit). This is the default; no regression.
- **present** → instead of exiting, auto-chain into `/autospec-explore` on the
  sandbox branch (never `main`, per the existing explore sandbox contract).

Guardrails (all must hold to chain; otherwise fall back to normal exit + log):
- **Autonomy gate** — call `autospec-autonomy-gate.sh --check all` before
  chaining; exit 1 → do not chain, surface instead.
- **Max-cycle cap** — `AUTOSPEC_EXPLORE_ON_DRAIN_MAX_CYCLES` (default 3). A
  per-repo counter at `~/.autospec/explore-on-drain.cycles` increments each
  chain; at the cap, stop and log `explore-on-drain: max cycles reached`.
  Counter resets when the operator clears it or starts a fresh `/autospec-run`.
- **Idle/empty guard** — if `/autospec-explore` itself produces zero shippable
  PRs for a full cycle, stop chaining (no point looping on a dry well).

A small helper `scripts/explore-on-drain.sh` encapsulates: flag check →
gate call → cycle-cap check/increment → emit the decision (`chain` | `stop`)
as stdout for the orchestrator to act on. Pure decision logic, no side effects
beyond the counter file, so it is unit-testable.

### Files touched (1 logical unit + 1 trio unit)
- `scripts/explore-on-drain.sh` (new) + `tests/explore-on-drain.bats` (new)
- `skills/autospec-run/{SKILL.md,codex/prompt.md,opencode/agent.md}` + derived
  `tests/fixtures/skill-goldens/autospec-run.*.sha256` (one trio unit — edit
  SKILL.md, `derive-trio.sh --in-place`, `gen-skill-goldens.sh`)

### Tests required
- flag absent → decision `stop` (default unchanged).
- flag present, gate OK, under cap → decision `chain`, counter incremented.
- at `AUTOSPEC_EXPLORE_ON_DRAIN_MAX_CYCLES` → decision `stop`.
- gate exit 1 → decision `stop` even with flag present.

### Acceptance criteria
- [ ] `bash scripts/explore-on-drain.sh` with no flag prints `stop` and exits 0.
- [ ] With `~/.autospec/explore-on-drain.flag` present and cycles < cap, prints `chain`.
- [ ] At cap, prints `stop` and does not increment past cap.
- [ ] `tests/explore-on-drain.bats` passes.
- [ ] `autospec validate` passes (trio goldens regenerated).

---

## Feature 2 — Async-transition push notifications

### Problem
During CI/monitor waits the agent goes silent; the operator pings "how do they
look?". Transcript mining found that exact phrase repeated within a single run.

### Design
**Reuse the existing notifier**, do not build a new one. `packages/
autospec_context_monitor/autospec_context_monitor/adapters/claude_hook.py` and
`injector.py` already route to `osascript` (macOS) / `notify-send` (Linux) with
proper escaping. Extract that into a single shared shell entry point
`skills/autospec-shared/scripts/notify.sh "<title>" "<body>"` that:
- emits a desktop notification via `osascript`/`notify-send` when available;
- **degrades gracefully** to a stdout log line `notify: <title> — <body>` when
  no notifier exists (headless/CI);
- is a no-op when `AUTOSPEC_NOTIFY=0` (default on; set 0 to silence).

Wire `notify.sh` at the run-state transition points:
- **CI verdict** — in `scripts/ci-wait.sh`'s background poller, on the
  queued→running→settled transition (one notification per terminal verdict:
  pass/fail/timeout), not per poll.
- **Monitor lifecycle** — in the Phase 4 monitor, on `claimed → tests_passed →
  pr_created → merged` and on terminal `failed`. One line per transition,
  deduped so a transition fires at most once per issue.

### Files touched (1 logical unit + targeted edits)
- `skills/autospec-shared/scripts/notify.sh` (new) + `tests/notify.bats` (new)
- `scripts/ci-wait.sh` (transition hook)
- `skills/autospec-run/{SKILL.md,...}` trio (monitor transition hook + goldens)

> Split into two child issues so each stays ≤3 logical units: (2a) the shared
> `notify.sh` helper + tests + ci-wait hook; (2b) the monitor-transition wiring
> in the autospec-run trio.

### Tests required
Mock the external notifier subprocess — `osascript`/`notify-send` are EXTERNAL
boundaries, so subprocess mocks ARE allowed (per
`feedback_per_pr_lgtm_misses_integration`). No real notifications in tests.
- notifier present (mock) → mock invoked with title+body.
- notifier absent → stdout log fallback, exit 0.
- `AUTOSPEC_NOTIFY=0` → no-op, exit 0.
- transition dedup → same transition fires at most once.

### Acceptance criteria
- [ ] `AUTOSPEC_NOTIFY=0 bash skills/autospec-shared/scripts/notify.sh t b` is a silent no-op exit 0.
- [ ] With a mocked `osascript` on PATH, `notify.sh "t" "b"` invokes it once with the title+body.
- [ ] With no notifier and `AUTOSPEC_NOTIFY` unset, `notify.sh` prints `notify: t — b` and exits 0.
- [ ] `tests/notify.bats` passes.
- [ ] `autospec validate` passes.

---

## Decomposition preview (≈5 children + 1 epic + Phase 5.5 audit)

1. EPIC umbrella — Autonomy Charter automations.
2. `explore-on-drain.sh` decision helper + bats (Feature 1 logic).
3. autospec-run trio: ALL_DONE → explore-on-drain chain wiring + goldens (Feature 1).
4. shared `notify.sh` helper (reusing context-monitor pattern) + bats + ci-wait hook (Feature 2a).
5. autospec-run trio: monitor-transition notify wiring + goldens (Feature 2b).
6. Phase 5.5 audit + remediation.
Each ≤400 words, ≤3 logical units, `reasoning:medium` / `ctx:64k`.

## Self-review
- **Placeholders:** none.
- **Consistency:** both features opt-in / default-off by toggle; neither blocks
  the merge path (notifier failure only logs; drain-chain failure only exits).
- **Scope:** single multi-issue pipeline; two related epics under one umbrella.
- **Critical risk:** runaway drain→explore loop — mitigated by the max-cycle
  cap + autonomy gate + dry-well guard. Notifier portability — mitigated by
  reusing the already-shipped, already-tested context-monitor notify path.
- **On merge:** update `docs/AUTONOMY-CHARTER.md` §5, removing each item as it ships.
