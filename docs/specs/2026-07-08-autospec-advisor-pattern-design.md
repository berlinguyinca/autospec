# Autospec advisor pattern — design

- **Date:** 2026-07-08
- **Status:** Design (approved for planning)
- **Author:** berlinguyinca (with Claude)
- **Topic:** On-demand strong-model advisor escalation for cheap-tier executors

## Summary

Adopt Anthropic's [advisor strategy](https://claude.com/blog/the-advisor-strategy)
inside autospec's Phase 4 loop: a **cheap executor** (Haiku/Sonnet) runs an issue
end-to-end and, at bounded decision points, escalates a single hard question to a
**strong advisor** (harness-native TIER_A — Opus on Claude Code, top GPT on Codex,
top task model on OpenCode). The advisor is **advice-only** — it returns a short
plan / correction / stop-signal (≤700 tokens) and never calls tools or emits
user-facing output. The executor consumes the guidance and resumes.

The published economics are the motivation: Sonnet + Opus-advisor beats Sonnet-solo
on both quality (**+2.7pp** SWE-bench Multilingual) **and** cost (**−11.9%**), and
Haiku + Opus-advisor makes Haiku viable at **85% less cost** than Sonnet-solo. That
maps directly onto autospec's current dilemma: the `claude-haiku-cloud` trial is
gated on LGTM-first-pass rate precisely because Haiku occasionally botches a hard
sub-decision. The advisor pattern is the mechanism that closes that gap while
keeping the cost win.

## Motivation & current state

Autospec's model routing today is **coarse and up-front**. `classify-model-fit.sh`
labels each issue `ctx:*` / `reasoning:*`; `select-model-profile.sh` then commits
the *entire* dispatch to one fixed tier (`examples/model-profiles.yml`):

- **TIER_A** = Opus + ultrathink → spec work (define/decompose/review). Low volume.
- **TIER_B** = Sonnet → Phase 4 implementer + LGTM reviewer. High volume.
- **Haiku trial** (`claude-haiku-cloud`) for `reasoning:shallow|medium` implementers,
  gated by LGTM-first-pass rate with an env rollback.

The only escalation primitives that exist are (a) *fall up the whole tier* on
quota/unavailability, and (b) *adaptive-retry*, which re-runs the entire implement
step at the **same** tier with accumulated directives. There is **no in-loop,
per-decision escalation**. Autospec classifies-then-commits; the advisor strategy
starts-cheap-and-escalates-surgically.

Autospec already thinks in second-opinion terms — `codex exec` peer-review, the
LGTM reviewer, the reuse-BLOCK cheap-refute voter, OMC `architect`/`consult-oracle`
— but every one of those is a **fixed-checkpoint, whole-phase** pass, never
"pull in the strong model at the exact hard moment for a 700-token correction."

## Goals

1. Add a single, deterministic, tested primitive for advisor escalation shared
   across all wiring points.
2. Wire four escalation gates (the "full escalation layer"), each independently
   flag-gated, with the Haiku-trial gate enabled first and alone until its
   economics clear.
3. Keep the emulated advisor **net-positive**: bounded call count, deterministic
   trigger gates, cost/quality telemetry, one-env-flip rollback.
4. Stay harness-neutral (Claude Code / Codex / OpenCode) and forward-compatible
   with a future native `advisor_20260301` tool.

## Non-goals

- No advisor on the low-volume define/decompose/spec phases — the blog's own
  framing is that the advisor helps the high-volume *executor* loop, not planning,
  and reasoning quality there already justifies TIER_A wholesale.
- No new terminal state — an advisor `stop` reuses autospec's existing soft-fail
  (return-to-queue) path.
- Not blocked on the native advisor API tool; the design is forward-compatible
  with it but ships on emulation.

## Architecture

Two responsibilities, deliberately split because **a bash script cannot spawn a
subagent or switch models** — those are executor-LLM capabilities, not shell
capabilities:

### 1. `advisor-escalate.sh` — deterministic bookkeeping (never calls a model)

Location: `skills/autospec-shared/scripts/advisor-escalate.sh`, installed to
`~/.autospec/scripts/` and invoked as
`${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/advisor-escalate.sh`
(matching `worktree-guard.sh` / `claim-guard.sh`).

Two phases:

```
advisor-escalate.sh --phase precheck --issue <N> --repo <owner/repo> --gate <gate-id> \
                    --question-file <f> --context-file <f> [--json]
advisor-escalate.sh --phase record   --issue <N> --repo <owner/repo> --gate <gate-id> \
                    --response-file <f> [--json]
```

- **`precheck`** — reads/increments the per-issue counter at
  `~/.autospec/advisor-state/<repo-slug>/<issue>.json` (path-scoped so it never
  collides across repos). Emits `GO` + the curated context payload, or
  `CAP-REACHED`. Exit `0` = GO, `7` = cap reached.
- **`record`** — validates the advisor's returned JSON against the response
  contract, enforces the ≤700-token budget, strips any tool-call / user-facing
  output, appends telemetry. Exit `0` on valid guidance, `2` on
  unparseable/backend-failure (fail-open — caller proceeds as if no advisor).

`<repo-slug>` = `<owner/repo>` with `/` → `_`. The script is **fail-open**
throughout: a `2` or `7` never blocks the issue; the gate degrades to today's
behavior. This mirrors `claim-guard.sh`'s degrade-to-no-op philosophy.

### 2. Advisor-invocation contract — capability-ordered dispatch (prose in trios)

The model call is described as prose the executor follows, with a per-context
capability ladder (subagent-first, CLI last):

1. **Native advisor tool** if the harness exposes it (`advisor_20260301`) — future.
2. **Harness-native subagent at TIER_A**: Claude Code `Agent(model: opus,
   read-only)`; OpenCode `task` top-tier model. Preferred path.
3. **CLI shell-out** — `codex exec` (native on Codex, which has no subagent
   primitive); `claude -p` / `opencode run` as legacy fallback — **only** where
   the executor's context lacks a subagent tool.

Rung 3 exists because of a hard constraint: per `autospec-run/SKILL.md`, a
**background-dispatched subagent does not inherit the `Agent` tool**. An in-loop
Phase 4 implementer running in the background therefore *cannot* spawn a nested
advisor subagent and must use the CLI rung. A top-level/foreground executor and
the orchestrator-level `reviewer` gate take rung 2. The ladder degrades
per-context automatically; the executor picks the highest rung its context
supports.

`claude -p` is treated as legacy (deprecation-prone); the durable Claude path is
rung 2 (subagent with `model: opus`), and the eventual durable path is rung 1.
Only the dispatch rung differs across harnesses — cap, curation, validation, and
telemetry are identical because they live in the one script.

### Flow

```
executor hits a gate precondition
  └─ advisor-escalate.sh --phase precheck  → GO + payload  (or CAP-REACHED → no-op, continue)
       └─ executor dispatches advisor via capability ladder (payload in, JSON out)
            └─ advisor-escalate.sh --phase record  → validated {verdict, guidance}
                 └─ verdict=plan|correction → apply + continue
                    verdict=stop            → soft-fail (return-to-queue + comment)
```

## The four gates

Each gate = a **deterministic precondition** (WHERE escalation is allowed) +
**executor self-judgment** (WHETHER to call) + the shared cap. All four call the
same script; only the `--gate` id and trigger prose differ.

| Gate id | Location | Deterministic precondition (WHERE) | On advisor response |
|---|---|---|---|
| `impl-haiku` | Phase 4 implement step, **`claude-haiku-cloud` profile only** | About to take the "ambiguous contract → comment & exit to queue" branch in `phase4-implementer.md` Expand step | `plan`/`correction` → continue implementing; `stop` → existing soft-fail |
| `impl-decision` | Phase 4 implement step, **any profile** | Executor judges a design/architecture sub-decision it "can't reasonably solve" | `plan`/`correction` → apply and continue; `stop` → soft-fail |
| `retry` | Adaptive-retry loop (`implementer-contract.md` §Retry) | **2nd+** blocking-finding retry on the *same* RULE_ID | Guidance injected as an extra corrective directive instead of a blind same-tier re-run |
| `reviewer` | LGTM reviewer verdict | Reviewer lands a **borderline** verdict (not clean LGTM, not hard BLOCK) | `plan` → uphold/refine verdict; complements the existing reuse-BLOCK refute pass |

**Trigger model (hybrid):** the deterministic precondition bounds *where* the
executor may escalate; within that, the executor exercises self-judgment on
*whether* the specific decision warrants the strong model. Both are subordinate to
the hard cap.

## Response contract & cap discipline

The advisor returns exactly one JSON object, no tools, no user-facing prose:

```json
{ "verdict": "plan" | "correction" | "stop",
  "guidance": "<= 700 tokens of actionable direction",
  "confidence": "high" | "medium" | "low" }
```

- **`plan`** — a short path forward for a decision the executor was stuck on.
- **`correction`** — a redirect when the executor is heading wrong (injected as an
  extra corrective directive on the `retry` gate).
- **`stop`** — "cannot be solved cheaply / out of scope" → maps to autospec's
  existing soft-fail (return-to-queue + comment). No new terminal state.

`--phase record` enforcement:

- Response >700 tokens → truncate + flag `over_budget:true`.
- Anything resembling a tool call or user-facing output → stripped.
- Unparseable/empty response → default to `verdict=stop` **fail-safe** (unparseable
  advice must not silently continue a shaky implementation).

The prose contract instructs the executor to prompt the advisor with: *"Return
advice only. You have no tools and produce no user-facing output."*

**Cap:** `budget.max_calls_per_issue` (default `3`) per issue, counted in the
per-issue state file, **shared across all four gates** so a single issue cannot
run away. Cap-reached → the gate no-ops to today's behavior.

## Configuration & self-governance

Configuration is a single declarative block in `.autospec/autospec.yml` — **not**
env-var levers (env vars survive only as CI/test overrides):

```yaml
advisor:
  policy: auto          # auto | on | off  (default: auto)
  budget:
    max_calls_per_issue: 3
    guidance_char_cap: 2800
```

The operator sets **intent (`policy`) + bounds (`budget`)**, mirroring the
existing `improvement_budget` / `sweep` pattern. `advisor-config.sh` resolves
these (precedence: env override > yaml > default); `advisor-escalate.sh` consumes
the resolved values.

**`policy: auto` — autospec self-governs the active gate set.** Rather than the
operator enumerating gates, autospec decides which are active from its own
telemetry, like an enterprise architect adjusting a standing order from results.
`advisor-govern.sh` maintains an active set (state file `active-gates.json`),
seeded at the low-risk `impl-haiku` gate, and ticked once per end-of-run sweep:

- **Promote** the next gate in the fixed safety order
  (`impl-haiku → retry → reviewer → impl-decision`) — one per tick — only when,
  over a **minimum-sample floor**, quality ≥ baseline AND cost ≤ baseline.
- **Retract** the last-added gate on regression (never below the `impl-haiku`
  seed).
- **Hold** below the sample floor.

`policy: on` activates every gate within budget (no self-tuning); `policy: off`
is inert. **Rollback** is one line: `policy: off`.

### Measurement pipeline (what feeds the tick)

The sweep calls one orchestrator, `advisor-sweep-tick.sh`, which:

1. **Observes** — `advisor-observe.sh` derives the batch's LGTM-first-pass rate
   from legacy telemetry or one effective, supersession-aware outcome per reviewed PR
   and mean cost/issue from autospec's main telemetry JSONL, using the *same*
   formulas as `gen-telemetry-dashboard.sh` (LGTM-first-pass = reviewer issues
   whose first dispatch had `cache_read > 0`; cost/issue = mean input+output
   tokens per issue).
2. **Snapshots the baseline on first activation** — the first sweep where the
   advisor is active freezes the current (pre-advisor) metrics into
   `.autospec/advisor-baseline.json` and stops. This is the **auto-snapshot**
   baseline: no operator input, captured before the advisor has influenced
   anything. (Cost is the executor-side cost/issue; the advisor's own call cost
   is bounded by the cap and tracked separately — the primary signal is whether
   escalation reduced retries/first-pass failures.)
3. **Ticks** — thereafter it feeds baseline + observed to `advisor-govern.sh`.
   Fail-safe: not-`auto`, no telemetry, or no reviewer signal → a logged no-op.

## Telemetry

Each `record` appends one line to `.autospec/telemetry/advisor-escalate.jsonl`:

```json
{"ts","issue","repo","gate","verdict","tokens_in","tokens_out","use_count","over_budget"}
```

`advisor-report.sh` summarizes this JSONL (per-gate calls/verdicts/tokens) and
computes the `promote` predicate for inspection. The governance decision itself
is driven by `advisor-sweep-tick.sh` against the auto-snapshot baseline (above).

## Files touched

New:

- `skills/autospec-shared/scripts/advisor-escalate.sh` (+ `.bats`)
- `skills/autospec-shared/scripts/advisor-report.sh` (+ `.bats`)
- `skills/autospec-shared/scripts/advisor-config.sh` (+ `.bats`) — YAML config resolver
- `skills/autospec-shared/scripts/advisor-govern.sh` (+ `.bats`) — self-governance ratchet
- `skills/autospec-shared/scripts/advisor-observe.sh` (+ `.bats`) — observed metrics from main telemetry
- `skills/autospec-shared/scripts/advisor-sweep-tick.sh` (+ `.bats`) — sweep orchestrator (baseline + tick)

Modified:

- `.autospec/autospec.yml` — the declarative `advisor:` block (`policy` + `budget`).

- `skills/autospec-run/SKILL.md` — advisor-invocation contract + `reviewer`/gate
  prose (**trio**: regenerate `codex/prompt.md` + `opencode/agent.md` via
  `derive-trio.sh --in-place`, then `gen-skill-goldens.sh`; update `validate.sh`
  named-content checks for any new section heading — all in the same change).
- `skills/autospec-run/prompts/phase4-implementer.md` — `impl-haiku` /
  `impl-decision` gate prose (standalone file, edit directly).
- `skills/autospec-run/prompts/implementer-contract.md` — `retry` gate prose
  (standalone file, edit directly).
- `install.sh` — ship `advisor-escalate.sh` + `advisor-report.sh` explicitly
  (the installer has a known gap dropping runtime scripts that ship-completeness
  does not catch).
- `examples/model-profiles.yml` — document the advisor env vars near the Haiku
  trial block.

## Testing

Tests are **fully deterministic — the model call is mocked** (the subprocess-mock
pattern used for tmux/osascript). `advisor-escalate.bats` covers:

- cap increment + rollover at `MAX_USES` (shared across gates);
- path-scoped state: two repos, same issue number → no collision;
- fail-open exit codes (`2`/`7` never block);
- response validation: >700-token truncation → `over_budget`; unparseable →
  fail-safe `stop`; tool-call / user-output stripping;
- backend-ladder resolution per `AUTOSPEC_HARNESS`;
- telemetry append shape.

`advisor-report.bats` covers the JSONL → decision-table summarization and the
promotion-gate arithmetic (quality ≥ baseline AND cost ≤ baseline).

**Bash constraints to honor up front** (all previously bit this repo): `set -e`
short-circuit → use `if/then/fi` for one-sided conditionals; no `RETURN` traps
(they leak under `set -u`); bash-3.2 needs a real temp file before any
`[ -f <(...) ]`; zsh exposes lowercase `pipestatus`; a background pipeline's
`echo`/`tail` can mask a non-zero gate exit — parse the gate's own final status
line.

**Acceptance checks:**

- Fresh-install smoke: after `install.sh`, `advisor-escalate.sh` and
  `advisor-report.sh` resolve on `~/.autospec/scripts/` (ship-completeness).
- `validate.sh` passes (trio lock-step + goldens consistent after derivation).
- With `advisor.policy: off`, every gate is a no-op and Phase 4 behavior is
  byte-identical to today.

## Decomposition guidance (for planning)

- Ship the primitive (`advisor-escalate.sh` + bats) and `advisor-report.sh` first;
  they have no consumer risk (fail-open, off by default).
- Keep **trio-prose edit + goldens regen as ONE issue** — a prose-only
  intermediate fails `validate.sh` closed.
- Wire `impl-haiku` before the other three gates; land the telemetry/report
  tooling alongside it so the promotion gate can be evaluated.

## Forward compatibility

Rung 1 of the dispatch ladder is the native `advisor_20260301` tool. When a
harness exposes it to subagent dispatch, only the dispatch contract's top rung
changes — `advisor-escalate.sh` (cap/curation/validation/telemetry) and all four
gate preconditions are untouched.

### Amendment 2026-08-05 — the native tool now exists, but not where autospec is

`advisor_20260301` has shipped as a beta server-side tool. This section records its
verified contract so nobody re-derives it, and states precisely why rung 1 is still
not reachable.

Contract, as of 2026-08-05:

- Tool definition `{"type": "advisor_20260301", "name": "advisor", "model": "<advisor>"}`.
  The executor is the request's top-level `model`; the advisor is the model inside
  the tool definition.
- Beta header / flag: `advisor-tool-2026-03-01`.
- **The advisor must be at least as capable as the executor**, or the request is
  rejected with `400 invalid_request_error`. The pairings this design cares about
  are valid: executor Haiku 4.5 or Sonnet 5 → advisor Opus 5 or Fable 5.
- Multi-turn: append the full `response.content`, *including* `advisor_tool_result`
  blocks, back into `messages`. Removing the advisor tool from `tools` on a later
  turn while the history still contains those blocks is a 400.

One gotcha will silently produce empty advice if missed. The response block is
always `advisor_tool_result`, but its `content` is a **discriminated union** that
varies by advisor model: `advisor_result` carries `text`, while
`advisor_redacted_result` carries `encrypted_content` — and Opus 5 and Fable 5, the
two advisors this design would actually use, both return the *encrypted* form.
Code must switch on `advisor_tool_result.content` type, never read `.text`
unconditionally. Encrypted content cannot be inspected, only replayed on the next
turn, which also means the validation step in `advisor-escalate.sh` cannot inspect
an Opus 5 advisor's answer directly — a constraint the emulated rung does not have.

**Blocking dependency: this is a Messages-API feature, and autospec does not call
the Messages API.** Every autospec dispatch goes through a harness — Claude Code's
`Agent` tool, `codex exec`, or OpenCode's `task` — and none of them currently
exposes an executor/advisor pairing to a subagent dispatch. Rung 1 therefore stays
future work; the trigger to revisit is a harness surfacing the pairing, not a
further API change. Rungs 2 and 3 (harness-native TIER_A subagent, CLI shell-out)
remain the implementable path, exactly as this design assumed.

Consequence for the cost model: the "emulated cost inversion" risk below is not
mitigated by the tool shipping. Until a harness exposes it, an advisor call still
re-loads context, so the deterministic gates and the hard cap remain load-bearing.

## Risks & open questions

- **Emulated cost inversion** — an emulated advisor call re-loads context and is
  far heavier than a native in-loop call; over-triggering can erase the −11.9%
  win. Mitigated by the deterministic gates, the hard cap, and the promotion gate
  that refuses to expand unless net cost actually held. This is the single risk
  the rollout is designed around.
- **Self-judgment reliability** — LLM difficulty self-assessment is noisy; the
  deterministic preconditions exist to keep the WHETHER decision inside a bounded
  WHERE.
- **Context curation quality** — the value of the advisor's answer depends on the
  payload the executor sends; the contract must specify a tight, decision-scoped
  payload, not a context dump (which would also inflate `tokens_in`).
