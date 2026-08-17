# Self-discovering, cost-aware model routing — design

- **Date:** 2026-08-05
- **Status:** Superseded on 2026-08-16 by
  [`2026-08-16-multi-model-engineering-team-design.md`](2026-08-16-multi-model-engineering-team-design.md),
  which absorbs all three layers below (supply discovery, execution path, learned
  routing) and extends them. This document remains the reference for the ledger
  format, the `advisor:`-style single policy knob, and the verified current-state
  audit; it is no longer the source of decomposition.
- **Author:** berlinguyinca (with Claude)
- **Topic:** Let autospec discover its own hardware/provider supply, measure what each
  model is actually good at, and route every dispatch — implementer *and* explorer —
  to the cheapest option that still clears the quality bar, including local GPU models
  over paid APIs.

## Summary

Autospec should treat "which model runs this dispatch" as a **measured, self-updating
decision** rather than a hand-edited config file. Three layers, in dependency order:

1. **Supply discovery** — a deterministic probe that finds what this host can actually
   run (GPU + VRAM, local runtimes, installed models with *real* context lengths,
   cloud keys/subscriptions) and caches it keyed by a hardware fingerprint.
2. **Execution path** — a wired route from a profile name to an actual local-model
   dispatch. This does not exist today, and nothing else in this document matters
   without it.
3. **Learned routing** — an append-only outcome ledger that records tokens, cache
   hits, wall-clock, retries, and escalations per `(dispatch_kind, ctx, reasoning)`
   cell, and derives an **effective cost** used to pick the cheapest profile clearing
   a first-pass floor.

The design deliberately mirrors `explore-ledger.sh` (append-only JSONL, outcome enum,
`--stats`, derived Bayesian weights, `--rebuild`) rather than inventing a new telemetry
format, and hangs its policy knob off the existing `advisor:` block idiom (one
`policy: auto|on|off`, self-governed from telemetry, no per-gate levers).

---

## Current state — verified, not assumed

Everything in this section was checked against the repo and against this host on
2026-08-05. It matters because the gap is larger than the code suggests.

### The routing decision is dead code

`skills/autospec-run/scripts/select-model-profile.sh` takes `--labels` and prints a
profile name. It has **zero callers**. Grep across the repo returns only the script
itself, its bats test, one CHANGELOG line, and one prose reference in the advisor
spec. It is also **not installed** — `~/.autospec/scripts/` contains
`classify-model-fit.sh` but no `select-model-profile.sh`.

The advisor design (`docs/specs/2026-07-08-autospec-advisor-pattern-design.md:29`)
states that `select-model-profile.sh` "commits the *entire* dispatch to one fixed
tier." That describes the intent; the wiring was never completed.

### Admission and executor selection are two different levers

This distinction is blurred throughout the current docs and is worth stating flatly:

- **Admission** — `--profile <name>` / the `default:` key filters *which issues are
  eligible to be picked up*, by comparing each issue's `ctx:*` / `reasoning:*` labels
  against the profile's ceilings (`skills/autospec-run/SKILL.md:75-76`, ordinal fit
  rule at `:142`).
- **Executor** — the model that actually runs the dispatch comes from the
  **harness-detection block** (`SKILL.md:162-177`): in Claude Code, `TIER_A` = `opus`
  + ultrathink, `TIER_B` = `sonnet`. The Phase 4 implementer brief (`SKILL.md:816`)
  and the fused LGTM reviewer (`SKILL.md:1150`) both name `TIER_B`.

So a profile name never reaches a dispatch today. Setting `default:` to an expensive
profile does not make dispatches expensive, and setting it to a local profile does not
make them local.

### No local model can execute anything

There is no local-provider dispatch path anywhere: no OpenAI-compatible base URL, no
`localhost:11434` client, no `--model` passthrough to a local runtime. The
`qwen3-32b-laptop` entry in `examples/model-profiles.yml` has been **purely
declarative** since it was written.

This is a hard harness constraint, not an oversight to patch casually. In Claude Code,
`Agent(model:)` accepts Claude tiers only — you cannot point a subagent at Ollama. A
local executor therefore has to be one of:

- **Codex CLI** or **OpenCode** configured against a local OpenAI-compatible endpoint
  (both already appear in the harness-adapter table at `SKILL.md:150`),
- or a dedicated script that drives `:11434/v1` directly and returns a patch.

### The auto-init probe fabricates models

`skills/autospec-run/SKILL.md:93-120` specifies the Ollama probe as **prose for the
orchestrating LLM** to follow. This host's resulting `~/.autospec/model-profiles.yml`
shows what that produces:

| Profile in file | Real? |
|---|---|
| `qwen3-32b-laptop` | yes (`qwen3:32b`) |
| `qwen3-5-latest-laptop` | yes (`qwen3.5:latest`) |
| `qwen3-6-35b-a3b-laptop` | yes (`qwen3.6:35b-a3b`) |
| `gemma4-latest-laptop` | yes (`gemma4:latest`) |
| `qwen3-6-latest-laptop` | **not installed** |
| `qwen3-coder-480b-laptop` | **not installed** (and unrunnable on a laptop) |
| `gemma4-26b-laptop` | **not installed** |
| `gemma4-31b-laptop` | **not installed** |
| `glm-4-7-flash-latest-laptop` | **not installed** |

`ollama list` returns exactly five models on this host. **Five of nine local profiles
are fabricated** — a 56% hallucination rate on a step whose entire job is to report
ground truth. This is the same failure mode already recorded in
`feedback_explore_once_unverified_near_zero_precision`: unverified LLM enumeration
produces confident non-facts. A probe is exactly the kind of step that must be a
script, and this belongs under tracker #421 (LLM steps → deterministic tools).

### The context ceiling is a flat guess, and it is wrong in both directions

Auto-init assigns every local model `ctx: 64k, reasoning: medium`. Measured values on
this host:

| Model | `ollama show` context length | Rated |
|---|---|---|
| `qwen3:32b` | 131072 | 64k |
| `qwen3.5:latest` | 262144 | 64k |
| `qwen3.6:35b-a3b` | 262144 | 64k |

The guess understates real capability by 2–4×, so issues that local models could
handle get deferred to paid tiers. A name-based lookup table would not fix this:
`qwen3:32b` reports `parameters 8.0B`, and `qwen3:32b` and `gemma4:latest` share
blob ID `c6eb396dbd59` — the *same weights tagged twice*, which naive parsing turns
into two independent profiles. **Measure; never parse the tag.**

### GPU detection must fail closed

On this host `nvidia-smi` fails: `Failed to initialize NVML: Driver/library version
mismatch`. But `/dev/nvidia0` exists, and `command -v nvidia-smi` succeeds. A probe
that checks for the binary or the device node concludes "GPU available" while the GPU
is unusable until the driver is reloaded — inference silently falls back to CPU at
perhaps 10–30× the latency. Presence of a device is not evidence of a working
accelerator. Only a successful capability query counts.

(This host also has 503 GB system RAM, so CPU inference is *possible* — which is
precisely the trap: it will work, slowly, and look like a win on the cost axis while
destroying throughput.)

### Routing today covers only issue-shaped work

`classify-model-fit.sh` produces `ctx:*` / `reasoning:*` labels **for GitHub issues**.
Every routing mechanism in the codebase keys off those labels. That leaves the
highest-fan-out dispatch surfaces routed by nothing but a hardcoded tier:

| Surface | `TIER_A` refs | `TIER_B` refs | Has ctx/reasoning labels? |
|---|---|---|---|
| `autospec-explore` (7 universal + 4 discovery + N specialist researchers, verify voters) | 8 | 6 | no |
| `autospec-refine` (N rounds × M lenses) | 6 | 5 | no |
| `autospec-secaudit` | 7 | 5 | no |
| `autospec-qa` | 5 | 3 | no |

These are high-volume, mostly-shallow, mostly-generative dispatches — exactly where a
local 32B is most plausible and where per-dispatch savings multiply hardest. They are
also where a large share of `TIER_A` (Opus) volume lives. See R5.

### What exists and should be reused, not rebuilt

- `explore-ledger.sh` — append-only JSONL outcome ledger with an outcome enum,
  `--stats`, `--show`, `--validate`, and `--rebuild` from GitHub history, plus
  `explore-source-weights.sh --json --explain` deriving **dynamic Bayesian per-source
  weights**. This is the exact shape the routing ledger needs.
- `advisor:` block in `.autospec/autospec.yml` — one `policy: auto|on|off` knob that
  self-governs from telemetry. Routing policy should be a **sibling** of this block,
  not a competing config surface.
- `classify-model-fit.sh` — already emits `(ctx, reasoning)` per issue with a
  confidence score and telemetry. No new classifier needed for issue-shaped work.
- `schemas/autospec-fleet-node.schema.json` already has "Model profiles this node is
  allowed to run" — the hook for per-host capability declarations.

### What does *not* exist and must be added

- **Cost telemetry.** `autospec.events.v1`
  (`docs/specs/2026-07-10-autospec-db-telemetry-design.md:126`) carries `tier`,
  `outcome`, `step` — but **no token counts, no cache-hit counts, and no wall-clock**.
  Effective-cost routing is unimplementable until these fields are added
  (additive-only within v1, which the contract explicitly permits).
- **Any tier logic in the usage governor.** `scripts/autonomous-usage-governor.sh`
  only soft-parks at a token threshold; it never downgrades a tier. Good news: a
  learned router will not fight an existing mechanism. It should, however, *inform*
  one — see R10.

---

## Recommendations

Ordered by dependency. R1–R5 are the foundation; R6–R8 are the learning loop; R9–R12
are operational. R12 is the cheapest immediate win and is independent of everything
else.

### R1. Wire a local executor path (blocking prerequisite for the local half)

Add a **Local model dispatch** row to the harness-adapter table
(`skills/autospec-run/SKILL.md:150-157`) naming the concrete executor per harness:

| Harness | Local dispatch |
|---|---|
| Claude Code | not natively available — shell out to Codex CLI / OpenCode configured against a local base URL, or to a dedicated local-implementer script |
| OpenCode | provider configured against `:11434/v1` (or `:8000` for vLLM) |
| Codex CLI | `--model` + local base URL |
| Fallback | cloud tier (fail closed) |

Until this row is real, `--profile qwen3-32b-laptop` cannot execute an issue.
**Recommendation: treat R1 as its own decision** (see Open Question 1) and note that
Waves 0–3 below deliver most of the cost reduction without it.

### R2. Replace the prose probe with `discover-model-supply.sh`

A deterministic script, run at bootstrap and on fingerprint change, emitting
`~/.autospec/model-capability.json`. Probe steps, each fail-closed:

**Accelerator**
- `nvidia-smi --query-gpu=name,memory.total,memory.free --format=csv` — **require
  exit 0 and parseable output.** Non-zero (as on this host) → treat as *no GPU*, do
  not fall back to device-node sniffing.
- Apple Silicon: `sysctl hw.memsize` + `system_profiler SPDisplaysDataType` for
  unified memory.
- ROCm: `rocm-smi --showmeminfo vram`.
- Record `usable: true|false` **with the reason string** when false — an operator
  seeing `"nvml_driver_mismatch"` can fix it in a minute; a silent CPU fallback hides
  a fixable problem for weeks.

**Local runtimes** — probe endpoints, don't assume:
- `curl -sf --max-time 2 localhost:11434/api/tags` (Ollama)
- `curl -sf --max-time 2 localhost:1234/v1/models` (LM Studio)
- `curl -sf --max-time 2 localhost:8000/v1/models` (vLLM)
- `curl -sf --max-time 2 localhost:8080/v1/models` (llama.cpp server)

**Per-model facts** — for each *actually listed* model, `ollama show <model>` (or
`/v1/models`) to capture real `context length`, `parameters`, `quantization`,
`capabilities`, and on-disk size. **Exclude non-completion models**: filter on
`capabilities` containing `completion`, which drops `nomic-embed-text` and friends
without a name blocklist.

**Cloud supply** — `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, subscription/OAuth state,
and the harness's own available tiers.

**Caching** — key the file on a fingerprint of `(GPU model + VRAM total + accelerator
usable flag + sha256 of the runtime model list)`. TTL (say 24h) plus fingerprint
mismatch triggers re-probe. This fixes the current behaviour of probing **once, only
when the file is missing, and never again** — which is why this host's profiles file
still lists five models that are not installed.

**Reconciliation** — on re-probe, entries with no corresponding real model are
**removed**, not merged. The current file's five ghosts are a merge-forever bug.

### R3. Derive ceilings from measurement

Replace the flat `ctx: 64k / reasoning: medium` default:

- **ctx** — from the measured context length, discounted for what actually fits in
  memory. Usable context is bounded by KV-cache size, not the model's advertised
  maximum: a 262144-token rating on a model whose weights already fill most of VRAM
  is not a 262144-token *budget*. Compute `min(advertised, fits_in(free_vram −
  weights))` and snap down to the existing `32k | 64k | 120k` ordinals.
- **reasoning** — do **not** guess from parameter count. Leave a new local model at
  its lowest tier and let R8 promote it on evidence. `qwen3:32b` self-reporting
  `8.0B` parameters is sufficient warning against arithmetic on metadata.

### R4. Add token, cache, and wall-clock fields to `autospec.events.v1`

Additive-only, per the contract's own rules: `input_tokens`, `output_tokens`,
`cached_tokens`, `wall_clock_ms`, `profile`, `dispatch_kind`, `escalated` (bool),
`retry_index`. The collector already "accepts unknown fields silently," so agents can
emit these before the hub understands them. Without this, R6 has no inputs.

### R5. Generalize the routing key to `(dispatch_kind, ctx, reasoning)`

Today's key is issue labels, which only exist for Phase 4 work. Add a
`dispatch_kind` dimension so every subagent dispatch is routable:

`implementer` | `lgtm-reviewer` | `explore-researcher` | `verify-voter` |
`refine-lens` | `qa-sweep` | `secaudit-pass` | `growth-lens` | `spec-decompose`

For label-less surfaces, the `(ctx, reasoning)` coordinates come from the dispatch
brief's own shape (bounded handoff size, whether the brief asks for generation vs.
judgment) rather than from `classify-model-fit.sh`. A coarse default per kind is
enough to start; the ledger refines it.

Two payoffs:

- **Coverage.** The four surfaces in the current-state table above account for far
  more dispatches per run than Phase 4 does, and are the strongest local-model
  candidates. Routing them is where "token effective" actually pays.
- **A new capability from two existing ledgers.** Crossing `dispatch_kind` +
  `profile` with `explore-source-weights.sh`'s per-*source* weights yields a
  `(source × profile)` table: not just *which research sources ship clean PRs*, but
  **which sources are cheap-model-safe**. A source whose value comes from breadth may
  be perfectly served by a local 32B; one whose value comes from subtle cross-file
  reasoning is not. Neither ledger can answer that alone.

Constraint carried into invariants: a `verify-voter` must not run on the same profile
as the proposer it is checking — shared-profile voting collapses the independence the
adversarial-verify pattern depends on.

### R6. Route on **effective cost**, not sticker price

This is the central concept, and it is counterintuitive enough to state plainly:

> **A local model that fails 60% of the time and escalates to Opus costs more than
> dispatching Sonnet once.** Free-per-token is not free-per-merged-PR.

Score each candidate profile for a cell:

```
effective_cost(profile, cell)
  = unit_cost(profile) × (1 + E[retries | profile, cell]) × cache_penalty(profile)
  + P(escalate | profile, cell) × unit_cost(advisor_tier)
  + P(fail | profile, cell)     × unit_cost(fallback_tier)
```

- `unit_cost` for a local profile is **not zero** but a wall-clock-priced figure —
  GPU-minutes have an opportunity cost (see R9).
- `cache_penalty` makes prompt-cache behaviour first-class, which is the other half
  of "token effective." Autospec pre-stages large context into every dispatch (spec
  anchors, file pointers, memory injection). On a cloud tier that prefix is cached
  across dispatches; **most local runtimes have no cross-request prompt cache at
  all**, so the same prefix is re-processed every single time. A profile that breaks
  caching costs materially more than its per-token price implies, and this term is
  what stops the router from routing a long-context, many-retry job to a model whose
  nominal price looked best. Derived from `cached_tokens / input_tokens` in the ledger.
- All probabilities come from the ledger's per-cell history.

**Pick the cheapest profile whose measured first-pass rate clears a floor** (proposal:
0.6 for implementers), not the cheapest profile outright.

### R7. `routing-ledger.sh`, cloned from `explore-ledger.sh`

Same contract, same subcommands (`--append`, `--update-outcome`, `--stats`, `--show`,
`--validate`, `--rebuild`), same append-only discipline where readers take the latest
line per key. Record schema per dispatch:

```
{profile, model, harness, dispatch_kind, issue, cell_ctx, cell_reasoning,
 input_tokens, output_tokens, cached_tokens, wall_clock_ms, retries,
 escalated, outcome, reason, ts}
```

`outcome` enum reuses the explore vocabulary where it fits: `merged_clean` |
`lgtm_first_pass` | `retried_ok` | `escalated` | `qa_failed` | `reverted` |
`abandoned`. `--stats` derives per-cell first-pass rate, mean retries, escalation
rate, cache-hit ratio, and effective cost — the exact inputs R6 needs. `--rebuild`
reconstructs from GitHub PR history so the ledger survives a machine wipe. Reuse
`explore-source-weights.sh`'s Bayesian smoothing so small-N cells are not overconfident.

### R8. Cold start: bounded exploration, never starvation

A newly discovered local model has zero ledger rows. A pure argmin on effective cost
scores it at *unknown* and it never runs, so it never earns rows — starved forever.

Two mechanisms, either acceptable:

- **Bounded epsilon-greedy** — route a small fraction (5–10%) of *low-stakes cells
  only* (`ctx:small` + `reasoning:shallow`, and never a safety gate) to unproven
  profiles until N≈10 samples exist.
- **Shadow mode** — run the unproven profile in parallel with the cloud tier, discard
  its output, and score it against the accepted result. Costs GPU time, risks nothing.
  Preferred for anything above the lowest cell.

Cap total exploration spend per week and log it, so cost attribution stays honest.

### R9. Wall-clock ceiling per profile

For a weeks-long `/autospec-autonomous` run, **40 GPU-minutes on an issue Haiku
finishes in 90 seconds is a throughput regression, not a saving.** Give every profile
a `max_wall_clock_ms` per cell; exceeding it is an `abandoned` outcome that
down-weights the profile and re-dispatches to cloud. This is the mechanism that keeps
"local is free" from silently halving the merge rate — and it is the reason a local
profile's `unit_cost` in R6 must be wall-clock-priced rather than zero.

### R10. Coordinate with, don't duplicate, the usage governor

`autonomous-usage-governor.sh` soft-parks at 90% of the token cap. A learned router
gives it a better option than parking: **as remaining budget shrinks, raise the
effective-cost weight on paid tiers**, shifting eligible work to local profiles and
extending runway instead of stopping. The governor keeps final authority on the park
decision; the router just makes park arrive later. Explicitly *not* proposed: letting
the router override a park.

### R11. Optional calibration harness (do this before trusting local for real work)

Replay K previously-merged small issues (proposal: K=5) against a candidate profile in
a throwaway worktree and score with the repo's own gates (`validate`, tests, secaudit).
Run once per hardware fingerprint, cache the verdict.

**"This local model qualified for zero tiers" is a legitimate, expected outcome** and
must be reported as a clean result, not retried into submission. On a machine where
the GPU is unusable and inference runs on CPU, that is the *correct* answer.

### R12. Wire the selector that already exists (cheapest immediate win)

`select-model-profile.sh` is written, tested (`tests/select-model-profile.bats`), and
already routes `reasoning:shallow|medium` → `claude-haiku-cloud` with a
`reasoning:deep` → Sonnet fallback and an `AUTOSPEC_TIER_B_PROFILE` rollback. It is
never called and never installed, so **the Haiku trial has never actually fired** —
every Phase 4 implementer and LGTM reviewer runs `TIER_B` = Sonnet regardless of how
shallow the issue is.

Three small, independent changes:

1. Add `select-model-profile.sh` to `install.sh`'s script set (cf.
   `feedback_installer_excludes_runtime_libs` — this is the same class of miss).
2. Call it from the Phase 4 implementer dispatch and have the resolved profile's
   `model:` override `TIER_B` for that dispatch, keeping the silent fall-back-up rule.
3. Keep the reviewer on the unmodified tier — see invariant 1.

Note this is a change to the **executor** lever, not the admission lever; the
`default:` profile keeps doing queue filtering and needs no edit. Independent of all
local-model work, and it is the one recommendation worth landing before the rest is
designed.

---

## Invariants a cost optimizer will otherwise violate

These are constraints, not preferences. Each one is a thing that *reduces measured
cost* while degrading something the ledger cannot see.

1. **Reviewer tier ≥ implementer tier, always.** A 32B model LGTM'ing 32B-written
   code degrades quality invisibly — the ledger records `lgtm_first_pass` and the
   router *rewards* the pairing. Hard-code the ordering.
2. **Verify-voters must differ in profile from the proposer.** Same reasoning, applied
   to explore's adversarial-verify stage: a refuter running the same weights as the
   proposer is not an independent check.
3. **Safety gates never route local.** `autospec-secaudit`, the premerge gate, and
   spec/decompose (TIER_A) stay on the strongest available tier regardless of cost.
   Correctness >> speed is repo doctrine.
4. **Local GPU is capacity-1.** Two workers dispatching to one Ollama instance thrash
   into swap and both blow their wall-clock ceiling. Local profiles need a host-scoped
   lock; `autospec-fleet-node.schema.json`'s per-node allowed-profiles field is the
   place to declare it.
5. **Fail closed on ambiguity.** Missing capability file, stale fingerprint, unknown
   model, unparseable probe output → cloud tier, behaving exactly as today. Never
   local-by-default on uncertainty.
6. **Never pick a profile whose measured ctx is below the issue's `ctx:*` label.** The
   existing ordinal fit rule (`SKILL.md:142`) stays authoritative; effective cost only
   orders the profiles that already fit.
7. **Log every bounded cap.** If exploration is capped, or top-N profiles considered,
   log what was dropped. Silent truncation reads as "evaluated everything."

## Privacy note

Local execution is a genuine capability win beyond cost: repos under contracts that
forbid shipping source to a third-party vendor can run autospec at all only with a
local tier. Worth one config flag (`routing.local_only: true`) that hard-excludes
cloud profiles and fails loudly rather than falling back. Not worth building further
infrastructure around at this stage.

## Suggested sequencing

| Wave | Contents | Needs local execution? |
|---|---|---|
| 0 | R12 (install + wire the existing selector) | no — ship first |
| 1 | R2 (probe script) + R3 (measured ceilings) | no |
| 2 | R4 (token/cache/wall-clock telemetry) + R5 (dispatch_kind key) + R7 (routing ledger) | no |
| 3 | R6 (effective-cost routing) + R8 (cold start) + R10 (governor) | no |
| 4 | R1 (local executor) + R9 (wall-clock ceiling) + R11 (calibration) | yes |

Waves 0–3 deliver most of the cost reduction on cloud tiers alone — R12 alone
activates a Haiku trial that has never run, and R5 extends routing to the
highest-volume dispatch surfaces — and are worth landing whether or not local
execution ever ships. Wave 4 is where "prefer local qwen over paid" becomes real, and
it depends on a harness decision that should be made explicitly rather than inferred.

## Open questions

1. **Which executor for local dispatch?** Codex CLI against a local base URL,
   OpenCode provider config, or a purpose-built local-implementer script? This is the
   one decision that gates Wave 4 and it is a genuine trade-off (Codex reuses existing
   peer-review plumbing; a dedicated script is more controllable but is new surface —
   `feedback_roi_check_new_components` applies).
2. **Does the routing policy live in the `advisor:` block or a sibling `routing:`
   block?** Sibling is cleaner conceptually; the same block is fewer surfaces. Either
   way it is **one** `policy: auto|on|off` knob, self-governed from telemetry — not a
   per-gate lever farm.
3. **Is the first-pass floor (0.6) right?** It should probably be derived from the
   cloud tier's own measured rate rather than fixed, once the ledger has data.
4. **How are `(ctx, reasoning)` coordinates assigned to label-less dispatches (R5)?**
   A static default per `dispatch_kind` is the cheap start; measuring brief size at
   dispatch time is more accurate but touches every dispatch site.
