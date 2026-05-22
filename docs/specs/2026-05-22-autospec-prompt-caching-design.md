# Autospec Prompt Caching — Anthropic Ephemeral Cache for Implementer + Reviewer Dispatches

**Status:** Draft design (2026-05-22)
**Author:** berlinguyinca + diagnostic
**Scope:** Amends `autospec-run/SKILL.md` implementer + reviewer prompt templates. Adds shared `bundle-static-context.sh`. No new skill.

## 1. Goal & non-goals

### Goal
Eliminate the recurring monitor-wrapper crashes by leveraging Anthropic's prompt-caching API (`cache_control: { type: "ephemeral" }`, 5-min TTL). Every monitor dispatch currently re-loads ~6–10k tokens of static context (SKILL.md + AGENTS.md + RULE_IDs + lockstep rules + saved-memory rules) from scratch — no cache reuse across the N implementer + reviewer dispatches a single monitor batch performs. Restructuring those prompts so the static prefix is contiguous + cacheable gives 50–90% token-cost reduction on every dispatch after the first within a 5-min window.

This directly addresses the failure mode observed across the v1+v2+hardening+docs-amendment runs: monitor wrappers die at 30–180 tool calls because each implementer/reviewer dispatch they spawn rebuilds context, burning the wrapper's own working set.

### Non-goals
- Cross-session caching (Anthropic ephemeral is 5-min TTL only — within-session benefit only; not a persistence layer)
- Caching `gh issue view` / git diff outputs (those change per-PR; orthogonal concern)
- Caching across distinct repos (each repo has different SKILL.md / AGENTS.md)
- Replacing the LLM dispatches with deterministic alternatives (separate scope — tooling-optimization)

## 2. Architecture

Every Phase 4 LLM dispatch (implementer subagent, fused guardian+LGTM reviewer, Phase 3 decomposer, Phase 3.5 classifier) has the same structural shape today:

```
[static prefix: SKILL.md + AGENTS.md + RULE_IDs + saved-memory + lockstep rules]    ← ~6-10k tokens
[per-call dynamic body: issue body + PR diff + last-iteration findings]              ← ~1-3k tokens
```

The static prefix is nearly identical across dispatches in the same monitor session. Anthropic's prompt cache makes it free to repeat — IF you mark the prefix with `cache_control` AND keep it at the same position.

**Restructure all dispatch prompts as:**

```
[CACHED PREFIX with cache_control: ephemeral marker]
  ├─ SKILL.md verbatim
  ├─ AGENTS.md verbatim
  ├─ RULE_ID table verbatim
  ├─ Lockstep rules
  ├─ Saved-memory rules (tag-filtered subset stable across monitor session)
  └─ Generic implementer/reviewer scaffolding (worktree commands, etc.)

[UNCACHED SUFFIX]
  ├─ Issue body for THIS dispatch
  ├─ Per-iteration findings (review feedback)
  ├─ Worktree path / branch name
  └─ Final instruction ("begin coding now")
```

Cache hit on every dispatch after the first within the same 5-min window. Cache TTL refreshes on each hit, so an active monitor session sees continuous reuse.

**Mechanism:** `$AUTOSPEC_SCRIPTS_DIR/bundle-static-context.sh` produces the cached prefix as a single blob. `autospec-run/SKILL.md` (and its lockstep mirrors) reference the bundler in the dispatch prompt template; the orchestrator constructs the API call with a `cache_control` array element pointing at the prefix.

## 3. Implementation surface

### 3a. `bundle-static-context.sh` (new)

Lives at `skills/autospec-shared/scripts/bundle-static-context.sh`.

```
Usage: bundle-static-context.sh --role implementer|reviewer|decomposer|classifier
                                 [--issue-labels "label1,label2,..."]
Output (stdout): blob of static prefix content with `<!-- CACHE BOUNDARY -->` marker at start + end.
```

Composes:
- SKILL.md (full file for the role)
- AGENTS.md verbatim
- RULE_ID table extracted from AGENTS.md
- Saved-memory: intersect `--issue-labels` with `scripts/memory-tags.yml`; concatenate matching feedback files verbatim
- Lockstep rules (one-paragraph reference to the validate.sh checks)
- Role-specific scaffolding (worktree commands for implementer; review structure for reviewer; etc.)

Idempotent + deterministic. Same inputs → byte-identical output → consistent cache key.

### 3b. Implementer prompt template restructure

Current Phase 4 implementer prompt (in `skills/autospec-run/SKILL.md` + lockstep mirrors) is built ad-hoc. Restructure so:

1. Orchestrator calls `bundle-static-context.sh --role implementer --issue-labels "<labels>"` → captures static blob.
2. Orchestrator passes blob as the FIRST content block of the implementer subagent's prompt, with `cache_control: { type: "ephemeral" }` annotation.
3. Orchestrator appends the dynamic suffix: issue body, branch name, "begin coding now".

The SKILL.md amendment is structural — it specifies the dispatch contract and the cache-control marker placement. The actual `cache_control` annotation goes in the Anthropic API call constructed by whatever harness is making the dispatch (Claude Code's Agent tool already supports this internally; for Codex CLI, the equivalent is provider-specific).

### 3c. Reviewer prompt template restructure

Same pattern for the fused guardian+LGTM reviewer. Static prefix:
- AGENTS.md `## Implementation-quality contract` section
- RULE_ID table
- Lint-implementation.sh expected output schema
- Reviewer verdict format

Dynamic suffix:
- PR diff
- Issue body
- Previous-iteration findings (if iter>1)

### 3d. Telemetry capture

Every dispatch writes to `~/.autospec/telemetry.jsonl`:

```json
{"ts":"2026-05-22T...","role":"implementer","issue":391,"input_tokens":12450,"cache_creation_input_tokens":8200,"cache_read_input_tokens":0,"output_tokens":3100,"dispatch_id":"a1b2..."}
```

`cache_read_input_tokens` is the metric to watch — it should be > 0 for every dispatch after the first in a monitor session. A telemetry summary helper at `$AUTOSPEC_SCRIPTS_DIR/telemetry-summary.sh` rolls up cache-hit rate, total tokens, and per-role breakdowns.

## 4. Phased rollout (3 issues)

### Issue 1 — `bundle-static-context.sh` + implementer prompt template (the big rock)
- Create `bundle-static-context.sh`
- Amend `autospec-run/SKILL.md` Phase 4 implementer-dispatch prompt to call the bundler + structure prompt with cache-control marker
- Lockstep mirror to `codex/prompt.md` + `opencode/agent.md`
- Bats tests for the bundler (role variants, idempotency)

### Issue 2 — Reviewer prompt template + bundle role
- Add `--role reviewer` mode to `bundle-static-context.sh`
- Amend the fused guardian+LGTM reviewer prompt in `autospec-run/SKILL.md` (+ lockstep)
- Bats tests

### Issue 3 — Telemetry capture + summary helper
- Write `record-telemetry.sh` that appends a JSONL line per dispatch
- Hook into Phase 4 implementer + reviewer prompt templates (`record-telemetry.sh --dispatch-id <id> --tokens-json <file>`)
- Write `telemetry-summary.sh` that reads jsonl + emits cache-hit-rate / token-cost breakdown
- Bats tests for the summary computation

All 3 carry `priority:high` so they ship before docs-amendment leftovers resume.

## 5. Testing

### 5a. Unit tests
- `bundle-static-context.sh`: per-role output validates against a structural schema; idempotency (same inputs → byte-equal output)
- `record-telemetry.sh`: appends well-formed JSONL; handles missing `cache_read_input_tokens` field gracefully
- `telemetry-summary.sh`: fixture jsonl → expected summary (golden)

### 5b. Integration test
Spin up a monitor dispatch chain in dry-run mode (stubbed LLM responses with known token counts); assert:
- Dispatch 1: `cache_read_input_tokens=0`, `cache_creation_input_tokens > 5000`
- Dispatch 2 within 5 min: `cache_read_input_tokens > 5000`
- Dispatch 3 within 5 min: `cache_read_input_tokens > 5000`

### 5c. Production smoke
After issues 1–3 merge, the next monitor batch processing #369–#374 should show > 60% cache hit rate per `telemetry-summary.sh`. Manual verification post-merge.

## 6. Dependencies & scope boundaries

| Dependency | Status | Failure mode |
|---|---|---|
| `skills/autospec-shared/scripts/` infrastructure | live (established #361) | required |
| `memory-tags.yml` (from #387) | live | required for memory subset selection |
| `lint-implementation.sh` (#388) | live | reviewer prompt references it |
| Anthropic API ephemeral cache support | live (in all current Claude models) | Codex CLI fallback: use as much of the prompt as possible; no caching benefit, no breakage |

### Out of scope
- Cross-session caching (5-min TTL is a hard ceiling)
- Caching per-issue dynamic content (it changes; not cacheable)
- Provider-specific cache APIs other than Anthropic (Codex/OpenAI = no benefit, graceful degradation only)

## 7. Decision log

| Q | Decision | Rationale |
|---|---|---|
| Where does the static-context bundler live? | `$AUTOSPEC_SCRIPTS_DIR` (centralized, not vendored) | Matches existing convention |
| Per-dispatch or per-monitor-session bundle? | Per-dispatch (constructed fresh per call) | Issue labels change per dispatch → bundle differs → cache key differs. The static *content* is what gets cached, not the bundle script's invocation. |
| Telemetry storage? | `~/.autospec/telemetry.jsonl` append-only | Simple, queryable, no schema migration concerns |
| Cache-hit threshold to declare success? | ≥60% across a 3+ dispatch monitor batch | Realistic target; first dispatch is always a miss |
| Codex CLI fallback? | No caching applied, no error | Graceful degradation — single-provider benefit is acceptable |

## 8. Open follow-ups

1. **Persistent cache layer** (cross-session) — would require external storage (Redis, file-backed) + content-hash routing. Future work; not justified by current cost.
2. **Cache-key fingerprinting** — surface the cache key in telemetry so we can debug cache misses ("why did dispatch 2 miss?").
3. **Tier-A dispatches (decomposer, classifier)** — Phase 3 + 3.5 currently use one-shot prompts; caching helps less since they don't repeat within a session. Lower priority.
