# autospec-explore — QA sandbox-promotion gate (`--qa-gate`)

## Summary

`/autospec-explore` ships discovered features autonomously onto an isolated
sandbox branch and then leaves promotion (`git merge <sandbox> → main`) as a
**manual operator action** — with **no proof the shipped features actually
run**. Explore's researchers and reviewers are static/code-level; nothing stands
the app up and exercises it. `autospec-qa` is exactly that missing proof: it
revalidates a running app against its spec and emits a hard-gated
`.autospec/qa-verdict.json` (PASS / PARTIAL / FAIL).

This spec adds an **opt-in** `--qa-gate` to `/autospec-explore` that, before
surfacing the sandbox as ready to promote, runs `autospec-qa --no-heal` against
the **sandbox HEAD** and **withholds the promotion instructions** unless the
verdict is PASS. It is the one ROI-justified explore×QA integration (per the
2026-06-16 investigation): explore *uses* QA, not the reverse. QA is reused
**unchanged** via its existing verdict contract; no second perpetual loop is
introduced; `autospec-run` is untouched (no circular dependency).

Default behavior is **byte-unchanged** (gate off; promotion stays the current
manual, ungated path). The gate is additive and opt-out.

## Problem statement

- Explore promotion is manual and ungated (`autospec-explore/SKILL.md:124-125`,
  the `git merge autospec/explore/<slug> → main` block); the automated
  `/autospec-explore-promote` is explicitly v2/out-of-scope (`:213`).
- Explore's quality signals are all static (researchers, the adversarial-verify
  stage, severity ranking) — none prove the running app works. A sandbox can
  accumulate N auto-merged PRs that each passed per-PR review yet collectively
  break the app; only a running-app QA pass catches that.
- `autospec-qa` already produces the needed verdict
  (`autospec-qa/SKILL.md:38` → `.autospec/qa-verdict.json`; hard PASS gate on
  `.autospec/proof-matrix.json` at `:277`) but nothing wires it to explore's
  promotion decision.

## Team personality

- **Selected team:** Reliability/backend — backend developer, SRE, QA engineer,
  technical writer.
- **Why this team fits:** the change is a release-gate that stands an app up
  from an autonomously-built branch and blocks promotion on evidence; the QA
  engineer owns the verdict semantics, SRE the sandbox-worktree app launch and
  graceful degradation, the writer the operator-facing promotion-readiness
  output.
- **Carry into child issues:** reuse `autospec-qa` unchanged; default off
  (byte-unchanged promotion); never block a repo that has no QA config; the
  gate runs at promotion-readiness, not inside the perpetual loop (bounded
  cost); no `autospec-run`/loop coupling.

## Review counter-team

- **Selected counter-team:** Product/maintainer + cost.
- **What they should challenge:** does the gate stand the app up correctly from
  a *sandbox worktree* (not the operator's checkout)? Does a repo with no QA
  config get penalized (it must NOT — skip, don't block)? Is `PARTIAL` treated
  as block or pass, and is that the operator's choice? Does running full QA at
  stop blow the cost/context budget? Can the gate's verdict go stale if the
  sandbox advances after the gate ran?

## Architecture

```
/autospec-explore "<prompt>" --qa-gate
        │
   (perpetual research → ship loop onto sandbox — UNCHANGED)
        │
        ▼  loop terminates (operator_stop / cap)
   ┌──────────────────────────────────────────────┐
   │  QA promotion gate (NEW, only when --qa-gate) │
   │  scripts/explore-qa-gate.sh:                   │
   │   1. read .autospec/explore-mode.json          │
   │      → {branch, head_sha}                       │
   │   2. checkout sandbox HEAD in a fresh worktree  │
   │      off the sandbox branch (worktree-guard)    │
   │   3. if no QA config (.autospec/test.yml /       │
   │      qa config) → SKIP: emit                     │
   │      code_health:explore_qa_gate_skipped_no_config│
   │      (NOT a block) and exit 0                    │
   │   4. else run `autospec-qa --no-heal` against    │
   │      the sandbox-HEAD app; capture               │
   │      .autospec/qa-verdict.json                   │
   │   5. write .autospec/explore-qa-gate.json        │
   │      {verdict, head_sha, blocking, ran_at}       │
   └───────────────┬──────────────────────────────────┘
                   ▼
   PASS  → print the normal "To merge sandbox into main: git merge …" block
   FAIL / PARTIAL(default) → WITHHOLD merge instructions; print the blocking
        findings + the .autospec/qa-verdict.json path + the discard path;
        emit code_health:explore_qa_gate_failed
```

The loop, sandbox contract, researchers, aggregator, and `/autospec-run` drain
are all unchanged. The gate is a terminal, opt-in promotion-readiness step.

## QA gate runner contract (`scripts/explore-qa-gate.sh`)

- Reads `${AUTOSPEC_STATE_DIR:-$HOME/.autospec}` / repo `.autospec/explore-mode.json`
  for `{branch, head_sha}`. If absent → exit non-zero with
  `code_health:explore_qa_gate_no_sandbox` (the gate was requested but there's
  no sandbox — operator error).
- Creates a fresh worktree on the sandbox branch at `head_sha` via
  `worktree-guard.sh create` (never the primary checkout); removes it on exit.
- **QA-config probe:** if the repo has no QA config the gate can run against
  (no `.autospec/test.yml` and no autospec-qa config), SKIP — emit
  `code_health:explore_qa_gate_skipped_no_config`, write an
  `explore-qa-gate.json` with `verdict:"skipped"`, exit 0 (NOT a block; a repo
  with no QA setup is not penalized).
- Else invoke `autospec-qa --no-heal` (heal-off: the gate proves, it does not
  mutate the sandbox) against the sandbox-HEAD app, read the resulting
  `.autospec/qa-verdict.json`, and persist `.autospec/explore-qa-gate.json`
  (schema `schemas/autospec-explore-qa-gate.schema.json`, new):
  `{verdict: PASS|PARTIAL|FAIL|skipped, sandbox_branch, sandbox_head_sha,
    qa_verdict_path, blocking_findings:[...], ran_at}`.
- **Staleness:** record `sandbox_head_sha`; the explore promotion-readiness
  output warns if the sandbox branch has advanced past `sandbox_head_sha` since
  the gate ran (re-run needed).
- Degrades: a missing `autospec-qa` skill, a QA crash, or a `jq` error logs to
  stderr and yields `verdict:"error"` — treated as a BLOCK (fail-closed for an
  *attempted* gate), distinct from the no-config SKIP (fail-open).
- `bash -n` clean, bash 3.2 safe, no RETURN traps, no `git add -A`.

## Explore loop integration

- New flag `--qa-gate` (default OFF). Also honor `--qa-gate-pass-on-partial`
  (treat `PARTIAL` as pass; default is PARTIAL→block, matching QA's own
  "PARTIAL is not PASS" discipline).
- The gate runs ONCE at loop termination (operator_stop / cap), before the
  final summary's promotion block — NOT per round (bounds cost).
- **Promotion-readiness gating of the existing summary:**
  - `verdict == PASS` (or `PARTIAL` with `--qa-gate-pass-on-partial`): print the
    existing `To merge sandbox into main: git merge …` instructions as today,
    annotated `sandbox QA: PASS`.
  - `verdict == skipped`: print the merge instructions annotated
    `sandbox QA: skipped (no QA config)` — operator promotes at their own risk.
  - `verdict ∈ {FAIL, PARTIAL(default), error}`: WITHHOLD the merge
    instructions; print `sandbox QA: <verdict>`, the blocking findings, the
    `.autospec/qa-verdict.json` path, and the discard instructions; emit
    `code_health:explore_qa_gate_failed`.
- Record the gate verdict row in `.autospec/explore-summary.md` and
  `.autospec/explore-loop.json`.

## Reuse, not fork (ROI)

`autospec-qa` is invoked unchanged — the gate consumes its existing
`--no-heal` mode and `.autospec/qa-verdict.json` contract. No QA-side code
changes. No new researcher, no aggregator change, no `autospec-run` change.
This is the only explore×QA integration with a named consumer that benefits
today (the operator about to promote a sandbox); the reverse couplings (QA
consuming explore) were rejected as redundant with QA's own heal loop and
verify-first filter.

## Testing (validation-via-shell)

- `tests/explore/test_explore_qa_gate_runner.bats` — with a stubbed
  `autospec-qa` returning a PASS / FAIL / PARTIAL `qa-verdict.json`, the runner
  writes the matching `explore-qa-gate.json`; no-QA-config repo → `skipped`
  exit 0; QA-crash → `error` (block); reads sandbox HEAD from
  `explore-mode.json`; uses a worktree (never the primary checkout).
- `tests/explore/test_explore_qa_gate_promotion.bats` — at loop-stop with
  `--qa-gate`: PASS prints the merge block; FAIL/PARTIAL withholds it + emits
  `code_health:explore_qa_gate_failed`; `--qa-gate-pass-on-partial` flips
  PARTIAL to pass; `skipped` prints merge block with the no-config annotation;
  gate OFF (no flag) → output byte-unchanged from today.

## Acceptance

- [ ] `scripts/explore-qa-gate.sh` ships: reads `.autospec/explore-mode.json`,
      runs `autospec-qa --no-heal` against the sandbox HEAD in a worktree,
      writes `.autospec/explore-qa-gate.json` per the schema.
- [ ] No-QA-config repo → `verdict:"skipped"`, exit 0,
      `code_health:explore_qa_gate_skipped_no_config` (does NOT block); a QA
      crash/error → `verdict:"error"` (blocks, fail-closed).
- [ ] `schemas/autospec-explore-qa-gate.schema.json` defines the gate artifact.
- [ ] `/autospec-explore --qa-gate` runs the gate once at loop termination and
      gates the promotion-readiness output: PASS prints merge instructions;
      FAIL/PARTIAL(default) withholds them + emits
      `code_health:explore_qa_gate_failed`; `--qa-gate-pass-on-partial` treats
      PARTIAL as pass.
- [ ] Gate OFF (default, no flag) reproduces the current promotion output
      byte-for-byte.
- [ ] The promotion output warns when the sandbox advanced past the gate's
      `sandbox_head_sha` (stale verdict).
- [ ] `autospec-qa` is invoked unchanged (no QA-side edits); `autospec-run` is
      untouched.
- [ ] Explore trio (SKILL.md + codex/prompt.md + opencode/agent.md) documents
      `--qa-gate` + the promotion-readiness contract, passes `check_lockstep`,
      and the 3 autospec-explore goldens are regenerated **in the same commit**.
- [ ] `scripts/validate.sh` gains `check_autospec_explore_qa_gate_contract()`;
      all new bats pass; `bash scripts/validate.sh` is green.

## Decomposition into child issues

Aiming for 3 children plus an umbrella.

1. **Issue A — QA gate runner**: `scripts/explore-qa-gate.sh` + schema
   `schemas/autospec-explore-qa-gate.schema.json` + `tests/explore/test_explore_qa_gate_runner.bats`.
   No loop wiring. Files: 3.
2. **Issue B — explore loop integration**: `--qa-gate` /
   `--qa-gate-pass-on-partial` flags in `scripts/autospec-explore.sh`; run the
   gate at loop-stop; gate the promotion-readiness output; summary row; the
   `check_autospec_explore_qa_gate_contract()` validate gate. + bats. Depends A.
   Files: 3.
3. **Issue C — explore trio docs + goldens (atomic)**: document `--qa-gate` +
   the promotion-readiness contract in the explore trio prose (byte-identical),
   regenerate the 3 autospec-explore goldens **in the same issue/commit** (never
   split prose from goldens — validate fails closed on a prose-only
   intermediate). Depends A+B. Files: ~6 (trio + 3 goldens).

Total: 3 children + 1 umbrella + the Phase 5.5 audit child.

## Out of scope (defer to v2)

- A fully automated `/autospec-explore-promote` that runs the gate and then
  performs the sandbox→main merge unattended (v1 still leaves the merge as the
  operator's explicit action; the gate only gates the *readiness signal*).
- Per-round QA gating (v1 gates once at termination to bound cost).
- Standing up apps with no QA config (the gate skips those — config authoring
  belongs to `autospec-qa`/`autospec-test`, not this gate).
- Auto-healing the sandbox on a FAIL verdict (the gate proves; `--no-heal` is
  deliberate — remediation flows through the normal explore loop or operator).
