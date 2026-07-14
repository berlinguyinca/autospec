# Prompt-cache reclaim (reviewer slim + v2 cache + redundant-pass trims)

## Summary

The 2026-06-03 cost-efficiency spec (D3) slimmed and cache-stabilized the
**implementer** prompt prefix, but the same disease was left untreated on every
other LLM dispatch path. Concretely:

- The **reviewer** prefix still injects the entire ~14 k-token `autospec-run/SKILL.md`
  (almost all monitor-loop bash a reviewer never runs) + full ~6 k-token AGENTS.md
  + a duplicate RULE_ID re-extraction, all inside ONE non-byte-stable
  `<!-- CACHE BOUNDARY -->` → it misses the prompt cache every inner-loop iteration
  (≤3/issue). ~30–45 k wasted tokens per review-heavy issue.
- The **v2-flow implementer** (`phase4-implementer.md`, the documented default
  path) does NOT route through the cache prefix at all — it re-reads issue body +
  AGENTS.md/RULE_IDs in-context, so the shipped D3 benefit is unrealized on the
  path that actually runs.
- **Phase 3.75** shared-contracts is a full Tier-A pass whose steps 1–2 are
  mechanical signature/path/name scans.
- **Phase 5.5** round ≥2 re-audits the entire batch window, not just the new gap PRs.
- Duplicate RULE_ID re-extraction also rides the decomposer/classifier paths;
  tag-filtered memory injection has no per-dispatch cap.

This spec reclaims those tokens — extending the existing cost specs to the roles
and code paths they did not reach.

## Concurrency phasing (REQUIRED)

- **Phase 1 (ship now — non-contended shared scripts only):** the reviewer
  prefix slim + cache stabilization (`bundle-static-context.sh`,
  `gen-reviewer-prompt.sh`, a new `reviewer-contract.md` mirror of
  `implementer-contract.md`), the duplicate-RULE_ID trim, and the memory-injection
  cap. None of these touch the `autospec-run`/`autospec-define` trios.
- **Phase 2 (DEFERRED until the queue is clear of concurrent run/define-trio
  edits):** the v2-flow cache wiring (edits `phase4-implementer.md` +
  `autospec-run` trio), Phase 3.75 deterministic-extract (edits `autospec-define`),
  Phase 5.5 round-≥2 scoping (edits `autospec-run` trio). Filed
  `deferred:concurrency` (NOT `auto-implement`).

## Team personality
- **Selected team:** Reliability/backend — backend dev, perf engineer, test engineer.
- **Carry into child issues:** prove savings with a token before/after, not just
  prose; the cache prefix must be BYTE-STABLE across dispatches (per-issue dynamic
  content goes BELOW the `<!-- CACHE BOUNDARY -->`); no behavior change to review
  verdicts — only prompt-assembly cost.

## Review counter-team
- **Selected counter-team:** Correctness + review-quality.
- **Challenge:** does slimming the reviewer prefix drop context the reviewer
  actually needs (a RULE_ID, an AGENTS.md rule it must enforce)? Does the
  memory cap drop a feedback file that would have caught a bug? Is the cache
  boundary truly stable, or does a stray per-issue token still bust it?

## Architecture (Phase 1)

```
skills/autospec-shared/scripts/bundle-static-context.sh
  reviewer role  ──► emit a SLIM, BYTE-STABLE cached prefix:
     reviewer-contract.md (guardian rubric + RULE_ID table, static)
   + the small subset of AGENTS.md a reviewer enforces
   — NOT the full autospec-run/SKILL.md, NOT a duplicate RULE_ID block
   per-issue memory + the diff move BELOW the <!-- CACHE BOUNDARY -->
  decomposer/classifier roles ──► drop the duplicate RULE_ID re-extraction
  all roles ──► cap injected memory to the top-K most-specific tag matches
                (AUTOSPEC_MAX_MEMORY_FILES, default e.g. 6)

skills/autospec-run/prompts/reviewer-contract.md   (NEW — mirrors implementer-contract.md)
```

## Phase 2 (deferred) design — captured, not shipped yet

- **v2 cache wiring:** prepend the `gen-implementer-prompt.sh` cached prefix to
  the `phase4-implementer.md` body (or have the orchestrator assemble it), so the
  default v2 path realizes the D3 prefix-slim + cache.
- **Phase 3.75 deterministic-extract:** a `extract-shared-contracts.sh` scanner
  greps signatures/paths/names across issue bodies; a narrowed Tier-B call only
  reconciles conflicts — or fold the synthesis into the decomposer's existing
  context (it already holds all bodies), eliminating the standalone Tier-A pass.
- **Phase 5.5 round-≥2 scoping:** scope the broad-review `--since` window on
  round ≥2 to only the newly-filed gap PRs (round 1 stays full — preserves the
  per-PR-LGTM-misses-integration value).

## Testing (Phase 1)
- `tests/bundle-static-context-reviewer.bats` (extend) — the reviewer prefix no
  longer contains the monitor-loop bash / full SKILL.md; contains the
  reviewer-contract rubric + RULE_ID table exactly once; per-issue memory + diff
  are BELOW the cache boundary; the prefix is byte-identical across two dispatches
  with different issue bodies (cache-stable).
- A token-count assertion: reviewer prefix tokens drop from ~20 k toward ~5 k on
  the fixture.
- Memory-cap test: with many tag matches, ≤ `AUTOSPEC_MAX_MEMORY_FILES` files injected.

## Acceptance (Phase 1 only)
- [ ] `skills/autospec-run/prompts/reviewer-contract.md` ships (static guardian
      rubric + RULE_ID table), mirroring `implementer-contract.md`.
- [ ] `bundle-static-context.sh` reviewer role emits the slim, byte-stable cached
      prefix (no full `autospec-run/SKILL.md`, no duplicate RULE_ID block);
      per-issue memory + diff move below the cache boundary.
- [ ] Duplicate RULE_ID re-extraction dropped on reviewer/decomposer/classifier
      roles; memory injection capped at `AUTOSPEC_MAX_MEMORY_FILES`.
- [ ] Reviewer prefix token count drops ≥50% on the fixture; prefix byte-stable
      across two differing-issue dispatches.
- [ ] No `autospec-run`/`autospec-define` trio file edited in Phase 1.
- [ ] `autospec validate` green; reviewer/bundle bats pass.

## Decomposition into child issues

**Phase 1 (file as `auto-implement`, drain now):** 2 children + umbrella.
1. **A — reviewer prefix slim + cache stabilization**: `reviewer-contract.md` +
   `bundle-static-context.sh` reviewer role + `gen-reviewer-prompt.sh` + the
   reviewer bats. Files: 3.
2. **B — duplicate-RULE_ID trim + memory-injection cap** in
   `bundle-static-context.sh` (decomposer/classifier/reviewer) + a cap test.
   Depends A (same file). Files: 2.

**Phase 2 (file as `deferred:concurrency`, do NOT drain):**
3. C — v2-flow cache wiring (`phase4-implementer.md` + `autospec-run` trio).
4. D — Phase 3.75 deterministic-extract (`autospec-define`).
5. E — Phase 5.5 round-≥2 scoping (`autospec-run` trio).

Plus a Phase 5.5 audit child for Phase 1.

## Out of scope
- Changing review verdict logic or the fused guardian+LGTM structure (D4 keeps it).
- Re-deriving the implementer prefix (D3 already shipped it).
