# /autospec-autonomous — perpetual self-driving conductor (design spec)

**Date:** 2026-06-25
**Branch:** `feat/autonomous-perpetual-loop`
**Origin:** operator request — "a 100% autonomous autospec process that keeps
running for weeks." Builds on `docs/AUTONOMY-CHARTER.md`,
`docs/memory/project_babysit_tax_autonomy_charter.md`, and the existing
`autospec-explore` / `autospec-run` engines.

## Phase-4 supersession note (2026-07-06)

`docs/specs/2026-07-06-autospec-autonomous-platform-design.md` is the current source of record for never-idle/never-ask semantics. It supersedes this Phase-1 document wherever Phase 1 describes dry-cycle park/notify, a blocking notify-operator halt, or a startup `AskUserQuestion` gate. The reconciled behavior is: dry backlog descends to value-ranked discovery and quality/surface/RAG floors; below `AUTOSPEC_VALUE_FLOOR` the conductor enters idle-rescan rather than convergence-stop; fenced or failed work is quarantined asynchronously and the loop continues; resource/spend/operator-control park remains authoritative and distinct from convergence-stop.

## Goal

Ship a top-level skill `/autospec-autonomous` (plus its calibration companion
`/autospec-persona`) that runs the autospec machinery **unattended for weeks**. Each cycle it walks a fixed **priority
waterfall**, picks the highest-priority available work, ships it, writes a daily
digest, obeys a GitHub control channel for live steering, routes work to the
cheapest capable model tier, and **parks itself before exhausting usage quota**,
resuming automatically when quota resets.

This skill is a **conductor**, not a new engine. It reuses, without
reimplementing, components that **exist in repo source today**: the `autospec-run`
autonomous merge pipeline, the Autonomy Charter gate (`autospec-autonomy-gate.sh`),
hard-quota recovery (`autospec-usage-limit.sh`), worktree isolation
(`worktree-guard.sh`), the loop driver (`scripts/lib/autospec-loop.sh`), and
`/autospec-resume`. Components it depends on that are **not yet built**
(`notify.sh`, `autospec-secaudit`, `autospec-explore-ledger`) and the
`autospec-explore` single-cycle interface are sequenced as prerequisites in the
"Phasing & post-review corrections" section — they are NOT assumed to exist.

## Non-goals

- Not rebuilding researchers, the merge pipeline, or sandbox management.
- Not a new planning UI — planning stays with `autospec-define`/`-refine`.
- Not promoting the sandbox to `main` — that stays an explicit operator action.
- No convergence-stop: the loop is meant to keep finding work indefinitely.

## Design decisions (locked with operator 2026-06-25)

1. **Packaging** — new top-level skill `/autospec-autonomous` (per the
   skill-per-capability rule). It calls `/autospec-run` and `/autospec-explore`
   as sub-engines; it does not fork their internals.
2. **Merge target by provenance** — issues that exist in the backlog (operator-
   or human-authored, OR previously promoted) merge to **main** via the normal
   `autospec-run` admin auto-merge. Work the loop **generates itself** (tiers 2–4
   below) lands on the **explore sandbox branch** for batched operator review.
3. **Usage governor** — investigate what each harness actually exposes for live
   usage; use it when present, else fall back to a self-accounted token-budget
   ceiling; always keep `autospec-usage-limit.sh` hard-quota recovery as a
   backstop. (Investigation spike is the first child issue — see Decomposition.)
4. **Control channel** — reserved GitHub labels, read at each **cycle boundary**
   (never mid-issue): `autospec:priority`, `autospec:steer`, `autospec:pause`,
   `autospec:stop`.

## Phasing & post-review corrections (AUTHORITATIVE — added 2026-06-25 after peer review)

Two independent reviews (Opus critic + Codex peer) converged on the same
blockers. **This section governs; where the descriptive body below disagrees, this
section wins.** The full vision (Tiers 2–4, persona, self-brainstorm, live-usage
governor) is **retained but re-phased** — nothing is deleted, only sequenced.

### Phase boundaries

- **Phase 1 (the only thing this spec's plan builds now): boring-safe core.**
  Tier 0 control channel + Tier 1 (backlog → `main`) + resilience + an
  `autospec-qa`-only pre-merge gate + a **cumulative cost kill-switch**. Goal:
  prove the loop ships the real backlog to `main` safely, forever, unattended.
- **Phase 2: explore-backed discovery.** Tiers 2–3 (local discovery + competitor
  RE) + the secaudit half of the gate + the live-usage governor. **Blocked on**
  an `autospec-explore` single-cycle interface (below) and on `autospec-secaudit`
  being built.
- **Phase 3: operator intelligence.** Tier 4 polish lenses, the operator persona
  model, the self-brainstorm panel, and the `/autospec-persona` interview skill.

### Dependency reality (verified against repo source, not assumed)

These are **design-only / unbuilt** in repo source and MUST NOT be cited as
"reused" until built; depending on them as-is makes the gate fail closed or
no-op on clean installs (`feedback_installer_excludes_runtime_libs`):

- `autospec-secaudit` — spec only → **Phase-2 prerequisite**. Phase-1 gate is
  `autospec-qa` only. Any gate that calls a scan skill MUST check presence and
  **halt with a `code_health` identifier if missing — never silently skip**
  (current `autospec-run` logs-and-continues; the conductor must fail closed).
- `notify.sh` — spec only → **Phase-1 prerequisite** (it is the operator's only
  window during unattended runs). Build it first (the Autonomy-Charter
  automations spec already designs it), or wire the `osascript`/`notify-send`
  path the explore/run skills already use; do not reference a nonexistent script.
- `autospec-explore-ledger` — no repo footprint → **Phase-2** (only consumed once
  discovery tiers exist).
- Path fixes: `scripts/lib/autospec-loop.sh` (not `lib/...`); Tier-2/3 invoke
  `/autospec-explore --research-sources internet` (there is no `--internet` flag;
  internet is on by default and gated by `--no-internet` / `--internet-allowlist`).

### Phase-1 contract corrections

- **Cumulative cost kill-switch (NEW, Phase 1, hard requirement).** The autonomy
  gate only checks per-invocation estimates — no running total. Add a persistent
  spend ledger (`~/.autospec/autonomous-spend.json`, path-scoped) that tallies
  tokens/issues across the whole run; at `AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS`
  (or issue count) the loop **parks and notifies** (not just per-cycle caps).
  This replaces the earlier "no cumulative cost cap" decision, which the reviews
  flagged as unsafe for a forever loop. The live-usage % governor remains Phase 2.
- **Main-health primitive.** Use `gh api repos/{owner}/{repo}/commits/main/status`
  (or `/check-runs` for the post-merge workflow), NOT `ci-wait.sh` (which polls a
  single PR). Decision thresholds: green → continue; pending → wait one poll;
  red → halt Tier-1 merges, file `autospec:needs-human`, notify.
- **Single-instance lock — reconcile with resume, don't contradict it.**
  `autospec-resume` deliberately adds **no second lock** (GitHub CAS comments +
  `updated_at` age thresholds 300s/10800s). The conductor lock MUST reuse those
  same age thresholds and define explicit handoff: on resume, the fresh process
  inherits/clears the conductor lock using resume's staleness logic, so there is
  never double-run or a stale lock that blocks recovery. Drop "mirrors resume."

### Phase-2 contract corrections (specify before building Phase 2)

- **`autospec-explore` single-cycle contract.** Explore is itself a perpetual
  loop; the conductor must NOT nest two forever-loops. Add an explore `--once`
  mode (or call `explore-research-cycle.sh` directly) that runs exactly one
  research pass and returns `{tier, proposals_seen, new_candidates, filed,
  dry, reason}` so the conductor can measure dryness at its own cycle boundary.
  The "dry tier" + `AUTOSPEC_AUTO_DRY_CYCLES` escalation depends entirely on this.
- **Priority reweight formula.** Make the multiplier explicit, e.g. a
  priority-matched proposal's score = `base × AUTOSPEC_PRIORITY_BOOST`
  (default 1.5), bounded so it reorders without swamping `confidence ×
  source_weight × 1/complexity`.

### Phase-3 contract corrections (specify before building Phase 3)

- **Panel → filed issues** must be a defined step: who converts synthesis output
  to `gh` issues, with what title/body/labels, deduped against open issues, on
  which base. Currently unspecified.
- **Persona merge is NOT a pure shell transform.** Split it: a deterministic
  shell helper for source-gathering + precedence ordering (unit-testable), and an
  explicit Tier-A LLM synthesis step (evaluated, not unit-asserted) — do not
  claim determinism for an LLM judgment (`feedback_self_consistent_test_fixtures_mask_bugs`).
- **ROI gate on the persona layer.** Before building Phase 3, justify it against
  `--priorities` + `autospec:steer` (which already deliver steering today). If the
  delta is only "rank slightly differently," cut it (`feedback_roi_check_new_components`).

## Value-gated prioritization model

The conductor's cross-workstream queue uses a SAFe WSJF-inspired score, with Cost of Delay represented by severity, value, and confidence signals and divided by implementation size/risk: `Priority = (Severity × Value × Confidence × Reversibility) / (Effort × BlastRadius)`. This keeps the waterfall aligned with SAFe WSJF / Cost of Delay guidance while adding autospec-specific safety divisors: high blast-radius or fenced changes route to a human gate, and candidates below the value floor idle for a re-scan instead of manufacturing low-value work.

Recently touched files receive deterministic score decay so the conductor does not ping-pong A→B→A across adjacent cycles. The daily digest includes the ranked queue and considered-and-skipped reasons when a value queue has been produced.

## The priority waterfall

One **cycle** = evaluate tiers top-down, execute the highest-value available work,
then loop. A tier is "dry" when it yields zero *new, shippable* items (after the
existing explore dedup + dry-well guard). Phase 4 keeps dry counts as
observability, but a dry count no longer parks the conductor: it descends through
discovery and the quality/surface/RAG floors, then idles on a re-scan heartbeat
when the best candidate is below `AUTOSPEC_VALUE_FLOOR`.

| Tier | Source | Engine | PR base |
|---|---|---|---|
| **0 — Control** | `autospec:priority` / `:steer` / `:pause` / `:stop` labels | conductor | — |
| **1 — Backlog** | open `auto-implement` GitHub issues (real queue) | `/autospec-run` | `main` |
| **2 — Local discovery** | spec-vs-code, codebase-signals, source-analysis, dependency-health, prior-reports researchers | `/autospec-explore` | sandbox |
| **3 — Competitor RE** | internet researcher (reverse-engineer prominent competitor features) | `/autospec-explore --internet` | sandbox |
| **4 — Polish** | charts/plots/statistics, UI polish, tutorials, documentation | conductor files issues → `/autospec-run` | sandbox |

Tier 0 is evaluated every cycle and **preempts** the rest. After Tier 0, the
loop drops to the highest non-dry tier. When a higher tier refills (e.g. the
operator files a new backlog issue while the loop is doing polish work), the next
cycle returns to it — the waterfall is re-evaluated top-down every cycle, so the
loop naturally floats back up.

> AUTONOMOUS ASSUMPTION: Tier 4 "polish" proposals are generated by a small set
> of polish-specific lenses (a chart/stats lens, a UI-polish lens, a docs/tutorial
> lens) added to the explore researcher set, gated behind reaching Tier 4 so they
> don't dilute earlier tiers. Flag if you'd rather Tier 4 draw from a hand-curated
> backlog template instead.

## Tier 0 — control channel semantics

At each cycle boundary the conductor runs one `gh issue list` query for the
reserved labels and acts:

- **`autospec:priority`** — the issue jumps to the front of Tier 1 for the next
  cycle (sorted by label-applied time). It still flows through the normal
  `autospec-run` gates; "priority" only reorders, it never bypasses safety.
- **`autospec:steer`** — the issue **body** is read as a free-text directive and
  injected into the next research cycle's aggregator prompt as a high-weight
  steering hint (e.g. "focus discovery on the export pipeline"). The conductor
  comments an acknowledgement and removes the label so it fires once.
- **`autospec:pause`** — write `~/.autospec/autonomous-pause.flag`; the loop
  parks (heartbeats continue) until the label/flag is cleared.
- **`autospec:stop`** — graceful stop: finish the in-flight issue, write the
  final digest, remove the loop, mirror `autospec-stop.sh --graceful`.

All four are **reversible, in-scope, local-or-already-authorized** actions, so
per the Autonomy Charter they execute without an extra confirmation turn. Label
parsing uses `capture()`/`==`, never interpolated `test()` regex
(`feedback_jq_test_regex_metachar_injection`).

## Operator-priority intake & self-brainstorm (operator simulation)

The conductor must decide *what to build next* the way the operator would. Two
mechanisms: a startup priority intake, and an internal multi-agent brainstorm
grounded in a model of the operator's past behavior.

### Startup priority intake

At skill start the conductor accepts free-text priorities:
`/autospec-autonomous --priorities "refine UX; add multi-user accounts; polish dashboards"`
(or, first run with none supplied, it infers priorities from `autonomous-priorities.md`, the operator persona, and control labels without `AskUserQuestion`). Priorities persist to
`~/.autospec/autonomous-priorities.md` (operator-editable mid-run; also
appendable via an `autospec:steer` issue). They are **high-weight directives**
that bias discovery and ranking across every tier — a discovery proposal aligned
with a stated priority gets a ranking multiplier; the polish tier draws its
lenses from them (e.g. "refine UX" → UX-polish lens fires earlier). Priorities
never bypass the safety gate or the waterfall ordering; they reweight, not
override.

### Operator persona model

A derived profile of the operator's judgment — *how they decide, prioritize, and
reject* — built by a Tier-A agent from, in precedence order:

0. **Explicit calibration interview** (`/autospec-persona`, below) — direct
   operator answers to ≤50 repo-grounded questions. This is the *supervised*
   signal and outranks everything inferred; a direct answer beats a mined
   pattern.
1. `docs/memory/` (esp. `feedback_*` and `project_*`) and `AGENTS.md` — the
   operator's explicit, durable preferences (correctness ≫ speed, small-LLM
   target, conservative guardrails, ROI-check new components, skill-per-capability,
   lock-step sacred, etc.).
2. `docs/AUTONOMY-CHARTER.md` — the recommendation=action boundary.
3. **Mined past sessions** — the same transcript corpus that produced the charter
   (`project_babysit_tax_autonomy_charter`). When transcripts are present, mine
   them for recurring decision patterns ("review the whole project for gaps",
   what the operator greenlights vs. rejects, where they push back). When absent
   (clean/headless/CI environment), fall back to sources 1–2 only — never block.

Cached to `~/.autospec/operator-persona.md` and refreshed on a cadence
(`AUTOSPEC_PERSONA_REFRESH_DAYS`, default 7). This is "simulate what I would do"
made concrete and inspectable — the operator can read and hand-edit the persona.

### Self-brainstorm panel

At each **planning boundary** (entering a discovery tier, priorities/steer
changed, or a fresh sandbox cycle — NOT every drain, to bound cost) the conductor
runs an internal brainstorm that simulates the operator deliberating, in place of
the human ratification the Autonomy Charter collapses:

1. **Candidates** — researchers + active priorities produce candidate directions.
2. **Operator-proxy panel** — N Tier-A agents (default 3, scales with budget),
   each adopting `operator-persona.md` plus a distinct lens that mirrors the
   operator's own review counter-teams (UX, architecture/ROI, risk/safety). Each
   argues, as the operator would, which directions to pursue, defer, or reject —
   with rationale citing the persona/memory.
3. **Synthesis** — a Tier-A synthesizer reconciles the panel into a ranked way
   forward and a one-paragraph `> AUTONOMOUS RATIONALE:` recorded with the filed
   issues and surfaced in the daily digest, so the operator can audit "why did it
   choose this?".

This reuses the `superpowers:brainstorming` + judge-panel patterns, run
autonomously with the operator *simulated* as the human in the loop. Panel
disagreement that produces a genuine no-clear-winner fork escalates to the
operator via the control channel (Charter boundary), instead of guessing.

> AUTONOMOUS ASSUMPTION: panel runs at planning boundaries, not every cycle, and
> default size 3. Flag if you want it every cycle (higher fidelity, higher cost)
> or a different lens set.

### `/autospec-persona` — calibration interview (separate companion skill)

A standalone skill that **trains the persona on the operator directly** by asking
repo-grounded questions, so the digital twin reflects how *this* operator decides
— not a generic persona. Run once to bootstrap, re-run to recalibrate.

- **Bounded:** asks **≤50 questions total** (hard cap), in themed batches via
  `AskUserQuestion` (multiple-choice preferred, free-text where needed), so it is
  finishable in one sitting and resumable if interrupted (progress persisted).
- **Repo-grounded, not generic:** a Tier-A agent first reads the repo — `docs/specs/`,
  `docs/memory/`, `AGENTS.md`, the charter, recent commits, open issues, code
  areas — and *generates* the questions from real tensions it finds, e.g.:
  *"Issue #X traded test-coverage for speed — would you have done that?"*,
  *"Two specs disagree on Y; which wins?"*, *"Rank these five candidate features
  for this repo."*, *"When is a new skill worth forking vs. invoking upstream?"*.
  Questions span: priority ordering, quality/risk tolerance, UX-vs-backend
  weighting, when to defer/reject, model-cost tradeoffs, review rigor, and
  destructive-action comfort.
- **Calibration, not just collection:** interleaves a few questions whose
  "answer" is already inferable from memory/charter, to *measure agreement* and
  flag where the inferred persona was wrong — correcting the model, not just
  appending to it.
- **Output:** writes/updates `~/.autospec/operator-persona.md` (the source-0
  input above) with answers, derived decision rules, and a confidence note per
  dimension. Human-readable and hand-editable.
- **Lifecycle:** `/autospec-autonomous` checks for the persona on startup; if
  absent, it offers to run `/autospec-persona` first (the operator may skip — the
  conductor then falls back to inferred sources 1–3). It is its own top-level
  skill (`feedback_autospec_skill_per_capability`), trio-derived like the others.

## Model-tier routing

Reuse the AGENTS.md two-tier policy; extend with an explicit **Tier C** for
trivial/deterministic work (label edits, digest rendering, dedup):

| Tier | Claude Code | Codex CLI | Used for |
|---|---|---|---|
| **A (planning)** | `opus` + ultrathink | top GPT + `reasoning_effort=high` | aggregation, proposal ranking, spec/plan authoring |
| **B (implementation)** | `sonnet` | `gpt-5.1-codex-spark` + medium | per-issue implementers, individual researchers |
| **C (mechanical)** | `haiku` | `gpt-5.1-codex-spark` + low | digest render, label triage, dedup, dry-well check |

"All planning with the top models" → Tier A is mandatory for any step that
decides *what* to build (research aggregation, ranking, issue authoring).
Codex-with-spark and Claude-with-haiku/sonnet are honored automatically via the
existing harness-detection block; Tier C is the new addition. Unavailable tier →
silently retry one tier up (existing fallback rule), never ask.

## Daily digest

Once per UTC day (first cycle whose date differs from the last digest's date),
the conductor renders a digest with a Tier-C model and:

1. Commits `docs/autonomous/digests/<YYYY-MM-DD>.md` to the **sandbox** branch
   (keeps `main` clean of operational noise).
2. Updates a single pinned GitHub issue titled `Autospec daily digest` (creates
   it once, edits its body each day with a rolling N-day table + link to the
   committed page).

Digest contents: cycles run, tier breakdown, issues filed, PRs merged to main vs
sandbox, competitor features mined, usage spent / governor parks, any
`autospec:steer` directives honored, **sandbox→main drift** (commits/files behind
+ conflict-risk estimate), quarantined `autospec:needs-human` items, and any
main-health halts. The date is derived from the sandbox script
convention, not local `date` ad-hoc (`feedback_explore_sandbox_utc_date_branch`).

## Usage governor (90% pre-emptive park)

Default-off soft governor layered on the existing hard-quota recovery.

- **Investigation spike (child #1):** determine, per harness, whether a live
  usage fraction is observable (Claude Code session/usage signals; Codex quota
  headers). Document findings in the skill.
- **If observable:** park when usage ≥ `AUTOSPEC_USAGE_SOFT_PCT` (default 90).
- **If not:** self-account output tokens spent in the current rolling window
  against `AUTOSPEC_USAGE_CEILING`; park at 90% of it. Harness-neutral, no hidden
  API dependency.
- **Park behavior:** write the resume command + sandbox context (mirroring
  `autospec-usage-limit.sh`), emit a `notify.sh` transition, arm a
  `ScheduleWakeup`/cron for the reset window, and exit cleanly. On wake, resume
  the same `/autospec-autonomous` invocation.
- **Backstop:** `autospec-usage-limit.sh` still catches a hard wall if the soft
  governor mis-estimates.

A pure decision helper `scripts/autonomous-usage-governor.sh` returns
`continue` | `park <resume-epoch>` from (observed % or token tally) — no side
effects beyond its tally file, so it is unit-testable.

## Pre-merge quality & security gate (mandatory, every tier)

**No PR merges — to `main` OR to the sandbox — until it passes both
`/autospec-qa` and `/autospec-secaudit`.** This gate is non-optional and applies
uniformly across all four tiers; it cannot be skipped by any flag, autonomy
setting, or governor state. It runs *after* the existing per-PR LGTM + Phase 4
secaudit gate, as a second, blocking conductor-level barrier before merge.

Per merge candidate:

1. **`/autospec-qa`** — revalidate the running app against its spec, regenerate
   missing/weak tests, and audit UI controls/forms/validation/API behavior/
   accessibility, to prove the change works and **does not regress** existing
   behavior.
2. **`/autospec-secaudit`** — scan the changed code for vulnerabilities, secret/
   credential leaks, injection (SQL/prompt), PII/data leaks, IP/license
   violations, and backdoors.

**Finding disposition (severity-gated):**

- **high / severe / critical / medium** findings from either skill are
  **blockers**. The conductor dispatches a Tier-B implementer to **fix them in
  place on the same PR branch**, then re-runs *both* skills on the updated branch.
  Loop until the branch is clean of high/severe/medium findings (bounded by an
  LLM-validator adaptive-retry loop, max 5 attempts —
  `feedback_llm_validator_adaptive_retry`).
- **low / info** findings do not block; they are recorded in the daily digest
  and, if actionable, filed as fresh Tier-4 issues for a later cycle.
- **Retries exhausted still dirty** → do **not** merge. Label the PR
  `autospec:needs-human`, leave it open, emit a `notify.sh` transition, record
  it in the digest, and move to the next work item. A genuinely unfixable
  security finding is exactly the Charter's "surface to operator" case — never
  merged around.

Because both skills already auto-fire post-batch (`autospec-review` /
`autospec-secaudit` per existing prose), the conductor's contribution is making
them a **blocking pre-merge barrier with a fix-and-recheck loop**, not just a
post-hoc report. The fix-then-recheck (not fix-then-trust) discipline mirrors
`feedback_self_consistent_test_fixtures_mask_bugs`: re-run the validators against
the *patched* branch, never assume the fix is clean.

## Resilience & long-run operation

The conductor is expected to run for **weeks**, so it is designed to survive
crashes, never double-run, never spin on broken state, and report its own drift.

**Crash recovery & resume.** The conductor writes durable run-state
(`~/.autospec/autonomous-state.json`: active tier, cycle count, sandbox branch,
last digest date, in-flight issue) plus a periodic heartbeat. A fresh process
detects an interrupted run and resumes at the last waterfall position by reusing
`/autospec-resume` (host/session/terminal-crash recovery, capped retries) rather
than starting over. Run-state lives under the path-scoped slug subdir, not a flat
shared dir (`feedback_heartbeat_cross_repo_collision`).

**Single-instance lock.** One conductor per repo. A path-scoped lock
(harness-neutral per-session id chain, `reference_harness_session_id_envs`)
prevents a second accidental `/autospec-autonomous` from stomping the first; a
second launch attaches as a status viewer or exits, never co-drives. Lock
ownership is reclaimable only when the holder's heartbeat is stale (so a genuine
live worker on another host is never stolen — mirrors `autospec-resume`).

**Stuck-work quarantine.** A per-issue failure cap
(`AUTOSPEC_ISSUE_FAILURE_CAP`, default 3): if `/autospec-run` fails the same
issue that many times, label it `autospec:needs-human`, notify, and move on —
the loop never spins forever on an unbuildable issue
(`feedback_monitor_silent_exit`, `feedback_implementer_subagent_ends_mid_ceremony`).
Implementer "ended mid-ceremony but work is correct" is verified via gh/git
before counting a failure.

**Outcome-ledger integration.** The conductor consults and feeds
`autospec-explore-ledger` — the recursive-self-improvement memory that records
which research sources actually ship clean PRs and derives dynamic source
weights. This stops the loop re-proposing already-rejected features every week
and biases discovery toward sources that historically merge clean.

**Main-health gate.** Because Tier-1 work auto-merges to `main` unattended, a bad
PR slipping the gates would poison every later cycle. After each main merge the
conductor confirms `main` CI is green (via the existing `ci-wait.sh`); if `main`
goes red, it **halts further Tier-1 main merges**, files an `autospec:needs-human`
fix issue, notifies asynchronously, and continues only safe sandbox/non-main tiers
until `main` is green again.

**Lifetime.** Phase 4 adopts the authoritative cumulative spend kill-switch and
usage governor: never-idle forbids convergence-stop for lack of work, but resource
park remains valid at usage/spend/operator-control boundaries. The loop resumes on
reset and ends only on `autospec:stop` or an unrecoverable crash. The per-cycle
autonomy-gate cost caps still apply per cycle.

**Sandbox drift reporting.** A single long-lived sandbox (operator decision). The
daily digest reports merge-base distance to `main` (commits/files behind) and a
conflict-risk estimate, so the operator knows when to review/promote. No
auto-rebase (unsafe under autonomous shipping — explore sandbox contract).

**Clean-room competitor RE (Tier 3).** The internet researcher reverse-engineers
competitors at the **behavior level only — describe what a feature does, never
copy competitor source**. `/autospec-secaudit`'s IP/license/copyright check is
the backstop in the pre-merge gate. Competitor targets come from
`.autospec/competitors.yml` (or are derived from the README/spec "purpose"
statement when absent), constrained by the existing internet allowlist.

## Safety & autonomy-gate integration

Every would-have-asked decision is evaluated by deterministic gates and out-of-band
control labels; the conductor weakens nothing and does not prompt. Hard-boundary
violations (irreversible destructive remote ops, force-push to a protected branch,
out-of-scope file changes, cost over caps, or no-clear-winner forks) are quarantined
to `autospec:needs-human` and the loop continues with the next safe item, unless a
resource/control park condition is explicitly tripped. Self-generated work is
sandboxed, so the highest-risk path (unreviewed feature code on `main`) is
structurally avoided.

Worktree isolation: the conductor asserts `worktree-guard.sh assert` before any
commit, per the explore sandbox contract and
`feedback_per_session_worktree_isolation`. The primary checkout stays read-only.

## Skill family layout (trio + helpers)

- `skills/autospec-autonomous/SKILL.md` — Claude Code adapter (authoritative).
- `skills/autospec-autonomous/codex/prompt.md` — Codex mirror (lockstep).
- `skills/autospec-autonomous/opencode/agent.md` — OpenCode mirror (lockstep).
- `skills/autospec-autonomous/{install.sh,uninstall.sh,README.md}`.
- `skills/autospec-persona/{SKILL.md,codex/prompt.md,opencode/agent.md,install.sh,uninstall.sh,README.md}`
  — the calibration-interview companion skill (its own trio).
- `scripts/autonomous-persona.sh` — persona build/merge from interview + inferred
  sources; pure transform over input files, unit-testable.
- `tests/fixtures/skill-goldens/autospec-autonomous.*.sha256` +
  `autospec-persona.*.sha256` (derived).
- `scripts/autonomous-waterfall.sh` — pure tier-selection decision logic.
- `scripts/autonomous-usage-governor.sh` — pure usage decision logic.
- `scripts/autonomous-control-channel.sh` — label query → command decision.
- Self-update + Stop + Self-paced-wakeup blocks mirror the autospec-explore
  structure (`feedback_autospec_decomposer_gotchas`: first new-skill issue MUST
  include Self-update + Model-tier + harness-adapter sections).

Trio edits use `derive-trio.sh --in-place` + `gen-skill-goldens.sh`; never
hand-maintain the codex/opencode mirrors or goldens
(`reference_trio_derivation_tooling`, `feedback_skill_golden_derivation_workflow`).

## Testing

External boundaries (gh, osascript/notify-send, the harness usage API) are
mocked subprocesses — allowed per `feedback_per_pr_lgtm_misses_integration`.
Fixtures materialized at runtime or `git add -f`'d, never left gitignored
(`feedback_gitignored_fixtures_pass_in_authoring_worktree`). bats `[ -f ]`
checks write a real temp file first (`feedback_bash32_process_sub_test_file`).

- `tests/autonomous/test_waterfall.bats` — tier selection: backlog present →
  Tier 1; backlog empty records dry observability then descends to Tier 1.5/Tier 2;
  below `AUTOSPEC_VALUE_FLOOR` → idle-rescan; Tier 0 always preempts.
- `tests/autonomous/test_control_channel.bats` — each reserved label maps to its
  command; `:steer` body becomes a directive + label removed; metachar-safe
  label parsing.
- `tests/autonomous/test_usage_governor.bats` — observed-% path parks at 90%;
  token-tally fallback parks at 90% of ceiling; under threshold → continue;
  resume epoch computed.
- `tests/autonomous/test_digest.bats` — one digest per UTC day; idempotent
  within a day; pinned issue created once then edited.
- `tests/autonomous/test_premerge_gate.bats` — clean qa+secaudit → merge allowed;
  high/severe/medium finding → blocked + fix dispatched + both skills re-run on the
  patched branch; recheck-clean → merge; retries exhausted → `autospec:needs-human`,
  no merge; low/info → non-blocking, recorded. qa/secaudit invoked as mocked
  subprocesses (external boundary).
- `tests/autonomous/test_persona.bats` — interview caps at 50 questions;
  progress persists/resumes; agreement-calibration flags an inferred-vs-stated
  mismatch; `operator-persona.md` written/merged with interview as source-0.
- `tests/autonomous/test_self_brainstorm.bats` — priorities reweight ranking;
  panel runs at planning boundaries only (not every drain); no-clear-winner
  panel split escalates to control channel; `AUTONOMOUS RATIONALE` captured.
- `tests/autonomous/test_resilience.bats` — single-instance lock blocks a second
  conductor; stale-heartbeat lock is reclaimable but a live one is not; resume
  restores waterfall position; per-issue failure cap → `autospec:needs-human`;
  main red → Tier-1 merges halt, sandbox tiers continue; ledger consulted/fed.
- `scripts/validate.sh` — trio goldens regenerated;
  `check_autospec_autonomous_contract` gate added.

## Decomposition — phase-structured (post-review)

Only **Phase 1** is decomposed and planned now. Phases 2–3 are roadmap entries to
revisit once Phase 1 is proven; they carry the contract corrections above and are
re-decomposed when reached.

### Phase 1 — boring-safe core (planned now: ≈8 children + 1 epic + Phase 5.5 audit)

1. **EPIC** — /autospec-autonomous perpetual conductor (Phase 1).
2. **`notify.sh` shared notifier** (prerequisite) — `osascript`/`notify-send`
   with graceful stdout fallback + bats. (Or land the Autonomy-Charter automations
   notifier first and depend on it.)
3. **SKILL.md scaffold trio** (Self-update + Stop + Model-tier + harness-adapter +
   Phase-1 waterfall contract) + goldens + `check_autospec_autonomous_contract`.
4. **`autonomous-control-channel.sh`** — reserved-label query → command decision
   (`priority`/`steer`/`pause`/`stop`), metachar-safe parsing + bats.
5. **`autonomous-waterfall.sh`** — Phase-1 tier selection (Tier 0 preempt → Tier 1
   backlog; tiers 2–4 stubbed as "not-yet-enabled") + bats.
6. **`autonomous-spend-ledger.sh`** — cumulative token/issue tally + hard
   kill-switch (park + notify at lifetime cap) + bats. **(NEW per review.)**
7. **Pre-merge gate (`autospec-qa` only in Phase 1)** —
   `scripts/autonomous-premerge-gate.sh`: blocking qa barrier with severity-gated
   fix-and-recheck (high/severe/medium → fix-in-place + re-run, max 5); **presence
   check that halts if a configured scan skill is missing** (no silent skip) + bats.
8. **Resilience unit** — durable run-state + heartbeat; single-instance lock
   reconciled with `autospec-resume`'s 300s/10800s age model + explicit handoff;
   stuck-work quarantine (per-issue failure cap → `autospec:needs-human`);
   **main-health gate via `gh api .../commits/main/status`** (not ci-wait.sh) +
   bats.
9. **Conductor orchestrator** — wire control + waterfall + spend-ledger + gate +
   resilience + daily digest into `scripts/lib/autospec-loop.sh`;
   ScheduleWakeup/cron resume; install scripts.
10. **Phase 5.5 broad integration audit + remediation**
    (`feedback_per_pr_lgtm_misses_integration` — never skip 5.5).

### Phase 2 — explore-backed discovery (roadmap)

Prereqs: `autospec-secaudit` built; `autospec-explore` `--once`/yield contract.
Then: Tiers 2–3 (+ `--research-sources internet`), secaudit added to the gate,
`autospec-explore-ledger` wiring, the live-usage % governor + its investigation
spike, sandbox drift reporting, clean-room `.autospec/competitors.yml`.

### Phase 3 — operator intelligence (roadmap)

Tier-4 polish lenses; `--priorities` intake + persona model
(`autonomous-persona.sh` split into shell gather + Tier-A synthesis); self-
brainstorm panel + `AUTONOMOUS RATIONALE`; `/autospec-persona` interview skill —
**only after the Phase-3 ROI gate** justifies it over `--priorities` alone.

Each child ≤400 words, ≤3 logical units; trio-prose + goldens kept atomic in one
issue (`feedback_decompose_trio_prose_goldens_atomic`).

## Self-review

- **Placeholders:** none.
- **Consistency:** backlog→main / self-generated→sandbox is uniform; control
  labels are reversible + gate-respecting. The pre-merge gate is the one
  mandatory, non-skippable barrier (Phase 1: `autospec-qa`; secaudit added in
  Phase 2 when built) and fails **closed** if a configured scan skill is absent.
- **Scope:** phased (see "Phasing & post-review corrections"). Phase 1 = boring-
  safe backlog→main loop with a cumulative cost kill-switch; discovery tiers and
  the persona/intelligence layer are deferred to Phases 2–3 with their contract
  corrections recorded. Only Phase 1 is planned now.
- **Peer-review resolution (2026-06-25):** two independent reviews converged on —
  unbuilt deps (secaudit/notify/ledger), no cumulative cost cap, nested
  perpetual loops, lock-vs-resume contradiction, wrong main-health primitive, and
  persona over-build. All are resolved or sequenced in the Phasing section above.
- **Critical risks & mitigations:** (a) runaway tier escalation → dry-cycle
  threshold + autonomy gate; (b) unreviewed code on main → structurally avoided
  by sandboxing self-generated work; (c) governor mis-estimate → hard-quota
  backstop; (d) trio/goldens drift → derivation tooling + validate gate;
  (e) regression / security defect reaching any branch → mandatory blocking
  `/autospec-qa` + `/autospec-secaudit` pre-merge gate with severity-gated
  fix-and-recheck; (f) crash over weeks → durable run-state + `/autospec-resume`;
  (g) double-run → single-instance lock; (h) spinning on broken work → per-issue
  failure cap → quarantine; (i) bad PR poisoning `main` → main-health gate quarantines
  Tier-1 merges on red; (j) re-proposing rejected work → outcome-ledger; (k) IP
  risk in competitor RE → clean-room behavior-only + secaudit IP check.
- **On merge:** add `/autospec-autonomous` and `/autospec-persona` to the skill
  catalog, llms.txt, the per-skill token-cost table, and cross-link
  `docs/AUTONOMY-CHARTER.md` §5.
- **Operator simulation:** the self-brainstorm is the autonomous replacement for
  the brainstorming-ratification gate the Charter collapses — persona-grounded
  (interview source-0 + inferred sources 1–3), auditable via `AUTONOMOUS
  RATIONALE`, and escalates genuine forks rather than guessing.
