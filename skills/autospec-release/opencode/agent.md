---
name: autospec-release
description: Use when a repo needs an end-to-end release readiness sweep across specs, docs, implementation, tests, QA proof artifacts, legacy cleanup, and merge readiness using existing autospec skills.
mode: primary
---

# autospec-release workflow

Run the repo-level release readiness loop. This is the "I want to ship the
current repo" wrapper over the existing autospec surfaces: sweep, review, run,
test, QA, proof validation, documentation drift checks, and legacy-code cleanup.

Use this when the operator wants one explicit release gate instead of invoking
each lower-level skill by hand.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-release -->

## Self-update mode

If the feature-request argument matches `update` after trimming and lowercasing,
re-install the full autospec suite from `main`, show the before/after diff if the
harness exposes it, then stop. Do not run the release gate.

## Required capabilities & harness adapter

| Capability | Claude Code | OpenCode | Codex CLI | Fallback if missing |
| --- | --- | --- | --- | --- |
| Subagent model tier | Tier A: `opus` + ultrathink | Tier A: top-tier `task` + max reasoning | Tier A: current top GPT + `reasoning_effort=high` | Run inline, but keep the same PASS/PARTIAL/FAIL contract |
<!-- autospec-block:harness-adapter-core -->
| Existing autospec skills | Slash skills | agent prompts | skills/prompts | Run the matching workflow instructions inline |
| GitHub operations | `gh` through shell | shell | shell | Mark merge/issue steps BLOCKED with exact command |
| Test and QA execution | Browser/E2E or shell | browser/task or shell | shell/browser when available | Mark app-only checks NOT TESTED with blocker |

**Model tier:** TIER_A for release synthesis and final readiness verdicts
because a false PASS can ship broken behavior.

## Harness detection

Detect the harness once at skill start:

1. Claude Code: `Agent` with `subagent_type` is available.
   - `TIER_A` = `opus` + ultrathink.
   - `TIER_B` = `sonnet`.
2. OpenCode: `task` tool is available.
   - `TIER_A` = top-tier task model + high reasoning.
   - `TIER_B` = smaller-tier task model + medium reasoning.
3. Codex CLI: `apply_patch` is the primary edit tool.
   - `TIER_A` = current top GPT + `reasoning_effort=high`.
   - `TIER_B` = current cost-optimized Codex model + `reasoning_effort=medium`.

Prefer a Tier A reviewer/verifier subagent for the final release verdict when
the harness supports it. If `TIER_A` is unavailable, silently fall back to the
next available top-tier model. If delegation is unavailable, run the verdict
inline.

## Area dispatch (parallel subagents)

`/autospec-release` parallelizes Stages 3-8 across 6 area subagents instead
of walking them sequentially in the orchestrator (issue #731). The
canonical orchestrator script is `release-area-dispatch.sh`, which:

1. Reads area definitions from `skills/autospec-release/areas/<name>.md`.
2. Dispatches all 6 area subagents in parallel via the harness-aware
   dispatcher (`lib/autospec-harness-detect.sh`, PR #725).
3. Aggregates per-area findings into `.autospec/release-verdict.json`
   honoring the schema consumed by `compute-release-verdict.sh`
   (PR #636).
4. Optionally re-probes stale findings via the qa-finding-filter
   (`qa-finding-filter.sh`, PR #650) when
   `AUTOSPEC_RELEASE_VERIFY_FIRST=1`.

The 6 areas:

| Area | Stage | Description |
| --- | --- | --- |
| `spec-completeness` | 3 | Specs match live code; no stale/superseded blockers. |
| `docs-freshness` | 3 | README/AGENTS/USER_MANUAL describe current behavior. |
| `implementation-completeness` | 5 | Every acceptance criterion has live code. |
| `test-coverage` | 6 | Release-gate suites green; mutation + density floors met. |
| `qa-artifact-integrity` | 7 | proof-matrix + control-ledger + mutation-proof + canary-results fresh + schema-valid. |
| `legacy-cleanup` | 8 | No dead routes/caches/configs masquerading as current. |

Each area subagent receives a tight prompt bounded to 50-120k context and
25 tool calls. Findings carry `verified_at: <head_sha>` since each
subagent is its own verifier; the orchestrator merges them and surfaces
`live_app_proof` from `qa-artifact-integrity` so
`compute-release-verdict.sh` continues to consume the merged verdict
unchanged.

**Model tier:** Tier A (spec work) for `spec-completeness`,
`implementation-completeness`, and `legacy-cleanup`; Tier B
(implementation work) for `docs-freshness`, `test-coverage`, and
`qa-artifact-integrity`.

Invoke the orchestrator directly:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/release-area-dispatch.sh"
```

Or, when iterating on a single area:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/release-area-dispatch.sh" --area spec-completeness
```

## When to use

- The repo has existing specs, issues, code, and docs, and the operator asks
  "can we release this?"
- A project needs QA, review, testing, documentation drift checks, and legacy
  cleanup in one coordinated pass.
- Lower-level autospec skills have been run piecemeal and the operator wants a
  single release-readiness answer.
- The team wants a repeatable gate before tagging, deployment, or handoff.

## When not to use

- Do not use this as the first step for a brand-new feature idea. Use
  `/autospec` or `/autospec-define`.
- Do not use it as a replacement for `/autospec-stop` while a monitor is active.
- Do not mark release PASS with mocked tests only when a no-mock smoke path is
  required by the repo contract.
- Do not paper over legacy code by leaving stale spec/docs references behind.

## Release pipeline

Run these stages in order. If a lower-level skill is installed and invocable,
call that skill. If not, follow its documented workflow inline and report that
the explicit skill invocation was unavailable.

1. Preflight
   - Record branch, dirty files, remotes, GitHub repo, open PRs, open
     `auto-implement` and `needs-classify` issues, and installed autospec skill
     versions if visible.
   - Do not revert user work. If dirty files exist, treat them as current repo
     reality and avoid overwriting them.

2. Sweep
   - Run `/autospec-sweep` or the equivalent sweep workflow.
   - Ensure `.autospec/autospec.yml` and the latest sweep report exist when the
     repo is configured for sweeps.
   - Turn recurring drift into concrete spec, issue, test, or docs work.

3. Spec, docs, and story sync
   - Compare `docs/specs/**`, README docs, user-facing docs, config docs, and
     implementation reality.
   - Remove stale claims for legacy code, legacy storage, deprecated APIs,
     removed flags, dead routes, and old workflows.
   - If behavior still exists only for compatibility, document that explicitly
     as compatibility, not as the preferred path.

4. Review
   - Run `/autospec-review`.
   - File or surface high-priority regression issues for spec-vs-code gaps.
   - Do not proceed to PASS while untriaged release-blocking review findings
     remain.

5. Implementation queue
   - Run `/autospec-classify` if `needs-classify` issues exist.
   - Run `/autospec-run` until there are no ready release-blocking
     `auto-implement` issues, or until a blocker is documented with the exact
     issue number and command to resume.

6. Test gate
   - Run `/autospec-test` where available.
   - Require lint, typecheck, unit, integration, smoke, E2E, and static checks
     that the target repo defines as release gates.
   - Failed tests are work, not report decoration: fix them or create a
     release-blocking issue with exact reproduction.

7. QA proof (MANDATORY — required for PASS)
   - Run `/autospec-qa` against a live booted app. Either:
     (a) boot the app locally (dev or prod build) and exercise it, or
     (b) point QA at an already-deployed environment whose commit SHA matches
         HEAD (or is explicitly recorded as the SHA under test).
   - Mocked unit/integration tests do NOT satisfy this gate. Stage 7 must
     produce browser- or app-level interaction artifacts (screenshots, DOM
     snapshots, network traces, or recorded sessions) covering the user-
     facing surfaces in the release range.
   - Require control-level proof for text boxes, selects, dropdowns, buttons,
     validation, API effects, failure banners, fallback behavior, accessibility,
     and no-mock smoke paths where the repo contract requires them.
   - Validate QA artifacts with
     `skills/autospec-shared/scripts/validate-qa-artifacts.sh` when available.
   - Do not skip silently. If the app cannot be booted or reached, record the
     reason, mark Stage 7 as `NOT EXECUTED`, and apply the Stage 9 verdict cap.
   - After `/autospec-qa` completes, confirm `.autospec/qa-verdict.json`
     exists with `head_sha` matching current HEAD and `live_app_proof: true`.
     If absent, stale, or `live_app_proof: false`, the QA gate has not been
     satisfied for this commit — treat as `NOT EXECUTED` for Stage 9.

8. Legacy cleanup gate
   - Search code, specs, docs, tests, fixtures, config, infra, and examples for
     removed or deprecated behavior.
   - Delete dead code when tests prove it is unreachable.
   - Remove stale spec/docs references in the same change that removes the code.
   - If a legacy path cannot be removed yet, create a deprecation issue that
     names the owner, replacement path, and removal trigger.

8a. Repo quality audit
   - Run the shared read-only audit before computing the release verdict:
     ```bash
     bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/repo-quality-audit.sh" \
       --repo . \
       --json ".autospec/repo-quality-audit.json" \
       --markdown ".autospec/repo-quality-audit.md" \
       ${AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES:+--file-issues}
     ```
   - Include audit status, filed issue links, suppressed findings, and unfiled
     residual risks in the release report.
   - Release PASS is blocked by unresolved `current-branch-regression` security
     or verification findings unless they are explicitly classified as inherited
     accepted debt in `.autospec/quality-audit-accepted.json`.

9. Verdict computation and release iteration loop

   The deterministic computation is implemented by
   `compute-release-verdict.sh` and is the canonical source of truth.
   Invoke it with the current verdict path; stdout is one of `PASS` /
   `PARTIAL` / `FAIL` and the exit code mirrors that (0/1/2; 3 for
   infrastructure errors). The prose rules below describe what that script
   does — if they diverge from the script's actual behavior, fix the script
   and the bats tests in `tests/compute-release-verdict.bats`, not the
   prose. Never re-derive the verdict by hand when the script is available.

   Compute the verdict deterministically from `.autospec/qa-verdict.json`:

   - If the file is missing → verdict is `FAIL` with a synthetic finding
     "qa-verdict.json not produced by Stage 7".
   - If `head_sha` does not match `git rev-parse HEAD` → treat as missing.
     Synthesize a FAIL finding "QA verdict stale relative to HEAD" and
     require Stage 7 to re-run in the same iteration before recomputing
     the verdict.
   - If `live_app_proof: false` → cap at `PARTIAL` regardless of findings.
   - For each finding with `release_blocking: true`:
     - `status: FAIL` → verdict is at least `FAIL` for that finding.
     - `status: PARTIAL` or `NOT_TESTED` → cap at `PARTIAL`.
   - Otherwise → verdict is `PASS`.

   Hard caps that override the file contents:
   - "No merged PR in this range requires it" is NOT a valid reason to skip
     live-app QA or any release-blocking finding. This gate evaluates the
     cumulative shippable state of the repo, not the diff of the most
     recent PRs.

   Iteration loop:
   - If verdict is `PASS`, emit the release report and stop.
   - Otherwise, do NOT emit a final report yet. Pick the smallest
     remediation that converts the highest-priority `release_blocking`
     finding into evidence:
     - `benchmark_overfit` / `mutation_proof_missing` /
       `automated_test_gap` → re-enter `/autospec-qa`'s regeneration loop
       to rewrite the offending tests.
     - `no_mock_smoke` / `live_backend_blocker` → boot the app or unblock
       the live backend, then re-run Stage 7.
     - `spec_contradiction` / `legacy_residue` → fix docs/specs/code in one
       commit, then re-run Stages 3-4.
     - `outsourced_implementation` → replace the third-party API/SDK call
       with an in-repo implementation of the spec's described algorithm;
       the external service MUST be either removed or gated behind an
       off-by-default flag scoped to evaluation-only use, with cached
       payloads carrying explicit redistribution permission. Then re-run
       Stages 4-7. If the same offending client/URL recurs in the next
       iteration's findings, escalate to human review rather than
       re-iterating on the same remediation.
     - Untriaged `auto-implement` issue or queued blocker → run
       `/autospec-run` on it, then re-run Stages 4-7.
     - Any other `release_blocking` finding (e.g. `accessibility`,
       `cross_browser`, `duplicate_code`, `new_code_intent`,
       `presentational_misclassified`, `other`) → file a tracked GitHub
       issue with reproduction steps, link it from the release report,
       then re-run Stage 7.
     After the remediation lands and is committed, re-enter at Stage 2
     (sweep) and recompute. Each iteration MUST re-run Stage 7 before
     recomputing the verdict — a verdict file whose `head_sha` does not
     match the iteration's current HEAD is treated as missing.

   Termination conditions (any one ends the loop):
   - Verdict reached `PASS`.
   - Iteration cap hit. Default: 5 iterations. Override:
     `AUTOSPEC_RELEASE_MAX_ITERATIONS=<n>`. On cap, emit `PARTIAL` with the
     unresolved findings and the exact next remediation command.
   - `~/.autospec/stop.flag` is present (operator halt). Emit `PARTIAL`
     with the current state.
   - No-progress detector: two consecutive iterations produced an identical
     set of `release_blocking` findings (by category + summary). Emit
     `FAIL` with the stuck findings and an escalation note.

   Verdict values:
   - `PASS`: all gates green; verdict file confirms live-app proof; no
     blocking findings remain.
   - `PARTIAL`: useful progress landed but the iteration cap or operator
     halt ended the loop with at least one non-FAIL blocking finding
     still open.
   - `FAIL`: at least one `release_blocking` finding has `status: FAIL`
     after the loop terminated, or the no-progress detector tripped, or
     the loop aborted on infrastructure failure.

## Spec supersession (recency)

When `docs/specs/` contains two specs that overlap on a behavior, the spec
whose last-modifying commit on the current branch is most recent wins
(issue #635: implicit-by-recency supersession; untracked specs fall back
to filesystem mtime). Operators do NOT write `Supersedes:` frontmatter —
recency alone decides.

Release verdict computation MUST consult the resolver before treating any
finding as `release_blocking`. A finding whose `source_spec` is older than
another spec covering the same behavior is downgraded from
`release_blocking` to `informational` — the newer spec has already replaced
the requirement.

Use the shared helper for every finding under review:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/resolve-spec-supersession.sh" "<behavior-key>"
```

If the resolver returns a spec path different from the finding's
`source_spec`, the finding is referencing a superseded behavior. Downgrade
the severity and annotate the finding with the winning spec path so the
operator can confirm the supersession intent. Do NOT block the release on a
superseded finding.

This rule applies to release report rows, QA proof-matrix carryover, and
legacy-cleanup blockers alike.

## Legacy cleanup prompt

Use this prompt whenever code, config, docs, tests, specs, infra, or examples
mention a deprecated path:

```text
Before accepting this release:

1. Is this path still reachable in the product, CLI, API, deployment, tests, or
   docs?
2. Is it the preferred path, a compatibility shim, or dead code?
3. Does the spec still describe it as current behavior?
4. Does any test fixture, README, runbook, env var, script, or infra module keep
   it alive accidentally?
5. What is the replacement path and where is that replacement proven?
6. Can I delete it now? If not, what exact issue tracks removal and what event
   will make removal safe?

Required outcome:
- Preferred path: keep it and prove it.
- Compatibility path: label and document it as compatibility only.
- Dead path: delete it and remove matching spec/docs/tests references.
- Unknown path: mark release PARTIAL and create the smallest investigation or
  cleanup issue needed to settle it.
```

## Worktree guard (commit gate)

<!-- worktree-assert:begin -->
Before committing any remediation fix (stage 9 iteration) or any other
change produced by this skill, run `worktree-guard.sh assert` and require
exit 0. A non-zero exit (in_primary_checkout / dirty / stale_base) is NEVER
worked around — stop the release flow and surface the emitted
`code_health:*` identifier to the operator.

```bash
# MANDATORY assert gate: MUST exit 0 before any commit in the release flow.
if ! bash "${AUTOSPEC_SCRIPTS_DIR:-scripts}/worktree-guard.sh" assert; then
  echo "worktree-guard assert failed (see code_health identifier above); aborting release commit" >&2
  exit 1
fi
```

Cleanup after a release-flow commit lands: `git worktree remove` the linked
worktree and `git worktree prune`. Never work in the primary checkout.
<!-- worktree-assert:end -->

## Release report contract

Return a concise report with:

- Verdict: `PASS`, `PARTIAL`, or `FAIL`.
- Branch and commit range reviewed.
- Skills/stages run, with command evidence.
- Changed files and simplifications made, if edits were performed.
- Test, QA, docs, review, and legacy-cleanup evidence.
- Remaining blockers with exact issue numbers or next commands.
- Known `NOT TESTED` areas, separated from tested evidence.

Never say the repo is release-ready unless the evidence above is complete.
