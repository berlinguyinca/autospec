# Phase 4 reuse / build-vs-buy interrogation lens — design

## Problem

In full autonomous mode the Phase 4 implementer makes many micro-decisions with
no human review. The operator asked for a per-step self-interrogation loop (why /
how does it help / simpler-or-existing alternative / how to test / overcomplicating
/ right architecture) to suppress AI slop. A naive 12-question litany at every step
is itself slop: an LLM self-reviewing in the same context rubber-stamps, costs
tokens, and changes no behavior ("reflection theater").

Applying that interrogation to the feature itself: Phase 4 **already** covers most
of the asked-for axes —

- `scripts/lint-implementation.sh` (deterministic, run at `SKILL.md:954` via
  `--pre-commit --staged --directives`) already flags `COMPLEXITY`,
  `OUT_OF_SCOPE`, `MISSING_TEST` and more.
- The **fused guardian + LGTM** subagent (`SKILL.md:998-1071`) adversarially
  reviews correctness/scope in a separate context, with a 3-iteration retry loop.
- Codex peer-review adds a second independent context.

The **one axis no existing gate covers** is **build-vs-buy / reuse**: "you
reinvented a util that already exists in `scripts/lib/` or a well-known library."
This pairs with the known "feature wired but never invoked" defect class. So this
spec EXTENDS existing machinery with a reuse lens — it does NOT fork a new gate
(`feedback_roi_check_new_components`: default to invoking upstream over forking).

## Team personality

**Quality-infrastructure engineering.**
- Roles: tooling engineer (deterministic detectors), prompt/LLM engineer (reviewer
  lens), test engineer (fixtures + proof harness), framework maintainer (trio +
  validate wiring), metrics engineer (ledger + precision).
- Why it fits: the deliverable is a gate inside an autonomous pipeline — correctness
  and false-positive control matter more than feature breadth.
- Risks it should notice: false-positive halts that stall merges; the gate becoming
  permanent ceremony; integration-dead code (wired but never invoked).

**Review counter-team — false-positive & integration skeptics.**
- Roles: false-positive skeptic, integration auditor, anti-over-engineering maintainer.
- Challenges: Does a BLOCK ever hallucinate "a library exists"? Is the lens actually
  invoked by the conductor, or just defined? Does the ledger earn its keep, or is it
  a heavy new dependency for a marginal signal? Reviews stay inside each issue's
  scope.

## Architecture

Four extension points, all on existing files (current `origin/main` anchors):

1. **Deterministic triage — `scripts/lint-implementation.sh`** (1263 lines;
   deterministic detectors listed at `:39`, emit via `emit_finding RULE_ID PATH
   LINE DESC` at `:161`; staged-diff source at `:259`). Add three flag-only
   RULE_IDs (no LLM), reading `git diff --cached`:
   - `REINVENT_REPO_UTIL` — a net-new function whose name/shape duplicates an
     existing helper found by `rg` across `scripts/`, `scripts/lib/`, and repo
     source. Heuristic: new `name()` definition where `rg -w name` already matches
     a definition elsewhere.
   - `NEW_DEP_UNJUSTIFIED` — a dependency added to a manifest (`package.json`,
     `requirements.txt`, `go.mod`, `Cargo.toml`, `pyproject.toml`, `Gemfile`) with
     no `# why:` / `why:`-style justification marker in the same hunk.
   - `NEW_ABSTRACTION_SINGLE_CALLER` — a net-new file matching
     `*manager*|*factory*|*adapter*|*wrapper*|*base*|*abstract*` (or a new exported
     class) with exactly one call site in the diff + tree.
   Trivial / no-match diffs emit nothing (risk-proportional). These are INFO-or-
   finding lines following the existing grammar; opt-out via the existing
   `# linter:allow-RULE_ID <reason>` mechanism (`:225`).

2. **Reuse reviewer lens — `scripts/gen-reviewer-prompt.sh`** (221 lines; wraps
   `bundle-static-context.sh --role reviewer` cached prefix + dynamic suffix from
   the PR diff). When triage flagged a reuse RULE_ID, append a "build-vs-buy +
   reuse" block to the **dynamic suffix** (never the cached prefix — preserves
   prompt-cache stability). The block instructs the existing fused guardian+LGTM
   subagent (`SKILL.md:998-1071`) to run a **real search** (`rg` the repo first,
   then optional package-registry / web) and emit, per flagged item, a verdict:
   `BLOCK` (reuse existing X / adopt library Y) / `ADVISE` / `PASS`. Evidence-bound:
   the verdict must name the matched util or library, never assert from belief.

3. **Consequence + refute pass — `skills/autospec-run/SKILL.md`** adaptive commit
   loop (`:938-965`) and fused-review verdict (`:1054`). A `BLOCK` must survive a
   **cheap refute pass** (a second short voter tries to kill the BLOCK; majority
   needed) before it halts the commit — per `feedback_llm_validator_adaptive_retry`,
   so a hallucinated "library exists" cannot stall a merge. The
   simplicity/"how-can-I-improve" axis is **ADVISE-only**: it may never BLOCK toward
   *more* code, only toward *less*, and only tied to a named AC (anti-gold-plating).
   This is a **trio change**: edit `SKILL.md`, then re-derive `codex/prompt.md` +
   `opencode/agent.md` with `derive-trio.sh --in-place` and regenerate goldens with
   `gen-skill-goldens.sh` — all in the same commit (`feedback_skill_golden_
   derivation_workflow`, `feedback_decompose_trio_prose_goldens_atomic`).

4. **Decision ledger + precision proof — new `scripts/`** + flag. Append each
   verdict `{ts, issue, trigger_type, verdict, upheld}` to
   `.autospec/interrogation-ledger.jsonl` (lightweight JSONL, reuse run-summary
   plumbing — do NOT build a heavy ledger). A precision report (modeled on
   `quality-differential.sh`) computes per-trigger precision = upheld BLOCKs ÷ total
   BLOCKs; a synthetic always-wrong trigger must auto-demote to ADVISE below a floor
   after N runs. Whole feature gated by `AUTOSPEC_REUSE_LENS` (default OFF) until
   precision is proven.

## Interaction / API shape

- `lint-implementation.sh --pre-commit --staged` gains 3 RULE_IDs in its existing
  output grammar; no new flags.
- `gen-reviewer-prompt.sh` gains an internal `--reuse-flags <file>` input (the
  triage output) that toggles the appended block; absent → byte-identical to today.
- New `scripts/interrogation-ledger.sh {record|report}` and
  `scripts/reuse-lens-precision.sh`.
- Env: `AUTOSPEC_REUSE_LENS=1` to arm; `AUTOSPEC_REUSE_PRECISION_FLOOR` (default
  0.6); `AUTOSPEC_REUSE_DEMOTE_AFTER` (default 10).

## Data model

`.autospec/interrogation-ledger.jsonl`, one JSON object per line:
`{"ts":<unix>,"issue":"<N>","pr":"<N>","trigger":"REINVENT_REPO_UTIL","verdict":"BLOCK","upheld":true}`.
`upheld` is back-filled when the operator/peer-review agrees (or null until known).

## Error handling

- Triage `rg` failure → emit nothing, exit 0 (never block on tooling error;
  fail-open, like `usage-governor` JSON handling).
- Reviewer search unavailable (no network) → reuse lens degrades to `rg`-only;
  records `verdict:PASS, note:search-unavailable`. Never a false BLOCK.
- Ledger write failure → warn, continue (best-effort, never blocks the PR).
- Flag OFF → all four extensions are inert; lint/reviewer/commit-loop byte-identical
  to today (assert this in tests).

## Testing (TDD, real services, no mocks per AGENTS.md)

- `tests/lint/` — fixtures: a diff that reinvents an existing `scripts/lib` helper
  (→ REINVENT_REPO_UTIL), an unjustified dep add (→ NEW_DEP_UNJUSTIFIED), a
  single-caller `*-manager` file (→ NEW_ABSTRACTION_SINGLE_CALLER), and a clean
  reuse-correct diff (→ silent). Negative-path pair each.
- `tests/<reuse-lens area>/` — gen-reviewer-prompt emits the block ONLY with
  `--reuse-flags`; refute pass suppresses a BLOCK when the second voter disagrees;
  simplicity axis can never BLOCK; flag-OFF inertness.
- Ledger: record→report round-trip; synthetic always-wrong trigger auto-demotes.
- **`validate.sh` must enumerate any new `tests/<area>/` in the same issue that
  creates it** (`feedback_validate_must_gate_every_test_dir`).
- **Wire-in proof**: each script child includes a test that greps the conductor /
  reviewer path to prove the new code is actually invoked, not just defined
  (`feedback_feature_wired_to_script_but_never_invoked`).

## Acceptance criteria

- [ ] `lint-implementation.sh --pre-commit --staged` emits the 3 new RULE_IDs on the
      3 positive fixtures and nothing on the clean fixture.
- [ ] `gen-reviewer-prompt.sh` output is byte-identical to baseline when
      `--reuse-flags` is absent.
- [ ] With `--reuse-flags`, the appended block names a real `rg`/registry match in
      its verdict.
- [ ] A BLOCK is suppressed when the refute pass disagrees (no false-positive halt).
- [ ] The simplicity axis never produces a BLOCK (only ADVISE).
- [ ] Verdicts append to `.autospec/interrogation-ledger.jsonl`; precision report
      runs; a synthetic always-wrong trigger auto-demotes after the floor.
- [ ] `AUTOSPEC_REUSE_LENS` unset → lint, reviewer, and commit-loop behavior
      byte-identical to today (proved by test).
- [ ] `validate.sh` gates every new `tests/<area>/`; conductor-grep wiring test green.
- [ ] Trio (`SKILL.md` + `codex/prompt.md` + `opencode/agent.md`) + goldens are
      consistent (`validate.sh` check_derive_trio_consistency passes).

## Decomposition guidance (parent + 4 children, ≤3 files / ≤400 words each)

1. Triage RULE_IDs in `lint-implementation.sh` + `tests/lint/` fixtures.
2. Reuse reviewer lens in `gen-reviewer-prompt.sh` + its test (block only on flags).
3. Consequence + refute + anti-gold-plating wiring in `SKILL.md` **trio + goldens**
   (one unit) — re-derive mirrors and regen goldens in the same commit.
4. Ledger + precision proof + `AUTOSPEC_REUSE_LENS` flag + `validate.sh` gate for
   the new test dir.

Plus the standard Phase 5.5 audit tracker issue.
