# Implementer contract (autospec:v2-flow)

This is the **implementer-relevant extract** of the autospec project rules. It is
the curated subset an implementer ACTS on — project rules, the RULE_ID table,
lock-step discipline, heartbeat schema, worktree/branch rules, the retry/review
loop contract, and the merge-gate summary. The full 70KB `autospec-run/SKILL.md`
monitor-loop machinery is intentionally NOT here: as a single-agent
absorbed-discipline implementer you own one issue end-to-end and do not run the
orchestrator's monitor phases, profile machinery, or invocation flags.

Treat this contract as authoritative for HOW to take one issue from "open" to
"PR merged". When a rule here references a deeper procedure (e.g. the full
`phase4-implementer.md` step body), follow that procedure; this file is the
acting digest, not a replacement for the per-step prompt.

## Engineering standards

- **No mock databases.** Tests run against a real database (or the repo's
  established test-DB harness). Mocks/stubs of DB symbols are a blocking finding.
- **TDD is non-negotiable** for any non-docs change: write the failing test
  first, implement, make it pass. Pure prose/docs changes skip TDD.
- **Conventional commits** (`feat:` / `fix:` / `test:` / `docs:` / `refactor:`).
  Small commits as you go; the PR squashes.
- **NEVER** push to `main`, force-push, bypass hooks (`--no-verify`), or `amend`
  an already-pushed commit.
- **Small-LLM target.** Keep changes self-contained and within the context /
  reasoning budget implied by the issue's `ctx:*` / `reasoning:*` labels. If you
  hit budget pressure, stop and comment on the issue rather than rushing.
- **Reuse before re-implementing.** Run the mandatory pattern survey, then state
  either `Reusing <X> because <Y>` or `No reuse — <reason>` in the PR body.

## Implementation-quality contract

Every PR produced by an `auto-implement` agent must satisfy the rules below
before the LGTM reviewer is dispatched. The enforcer is
`lint-implementation.sh` (exits 0 on pass, N on fail where N = number of
blocking findings, capped at 200).

### RULE_ID table

| RULE_ID | Detector | Tier | Threshold / regex |
|---|---|---|---|
| `OUT_OF_SCOPE` | det | exact/prefix path compare | files touched ∉ exact files or trailing-slash directories declared in `## Implementation outline` ∪ `## Files touched` |
| `MISSING_TEST` | det | path-prefix scan | required test type from issue body `## Tests required` not present in diff under `tests/{unit,integration,smoke,e2e}/` |
| `COMPLEXITY` | det | line/regex scan | function >50 LOC, file >500 LOC, nesting >4 |
| `SECURITY` | det | regex match | `eval\(`, `exec\(`, `--no-verify`, `git reset --hard`, `rm -rf /`, AWS-key shape `AKIA[0-9A-Z]{16}`, GitHub-token shape `gh[pousr]_[A-Za-z0-9]{36,}`, private-key markers `-----BEGIN [A-Z ]*PRIVATE KEY-----` |
| `TODO_LEFT` | det | regex on non-test diff | `\b(TODO\|XXX\|FIXME)\b` |
| `MOCK_DB` | det | regex on test diff | `\b(mock\|stub)\b` near DB-symbol heuristics (`db\.`, `database`, `DataSource`, `pg`, `mysql`, `sqlite`) |
| `HALLUCINATED_API` | LLM | semantic | symbol referenced in diff not defined in diff, not in pre-PR repo (verifiable via repo search), not in dependency manifests |
| `DUPLICATE_CODE` | LLM | semantic | new code mirrors an existing helper (must cite `<path>:<line>`) |
| `STRING_MATCH_DOMAIN_LOGIC` | LLM | semantic | code uses substring checks against free-form text to encode domain meaning, AND a proper-representation library is imported in the file |
| `REPEATED_STRUCTURE_AS_CODE` | LLM | semantic | ≥5 branches in the same function/method sharing identical structural shape (same return shape, predicate signature, side-effect line) |
| `DOC_OUT_OF_SYNC` | hybrid | det+LLM | det: any change to public surface (CLI flag, env var, exported function, config key) WITHOUT a touched doc file (`README*`, `AGENTS.md`, `docs/**`, `SKILL.md`); LLM: judges semantic accuracy when a doc IS touched |
| `INVENTED_CONFIG` | LLM | semantic | flag/env-var/config-key introduced in diff not present in issue body or referenced spec |
| `PR_SIZE` | det | git diff/numstat | hard above 400 additions+deletions, 8 raw files, or 3 normalized logical units; binary rows are always hard |

### Corrective directive map

Each RULE_ID maps to a single-line corrective directive that is injected into the
implementer's retry prompt as cumulative context:

| RULE_ID | Directive |
|---|---|
| `OUT_OF_SCOPE` | "Restrict the diff to exact files or descendants of trailing-slash directories declared in `## Implementation outline` or `## Files touched`. Revert undeclared files; incomplete scope must be corrected by the issue author." |
| `MISSING_TEST` | "Add a test under tests/<TIER>/ for the listed required test type before re-pushing." |
| `COMPLEXITY` | "Split functions >50 LOC, files >500 LOC, nesting >4. No copy-paste branches." |
| `SECURITY` | "Remove the flagged pattern. NEVER hardcode secrets (remove AND rotate — a committed secret is compromised), NEVER use --no-verify or git reset --hard, validate input at boundaries, parameterize SQL, never eval/exec untrusted input, never let untrusted input reach an LLM/prompt sink. The Phase 4 security gate (security-remediation-loop.sh) must report decision=pass before merge." |
| `TODO_LEFT` | "Remove TODO/XXX/FIXME from non-test code. File a follow-up issue if the work is genuinely deferred." |
| `MOCK_DB` | "Remove DB mock/stub. Use the real DB per AGENTS.md ## Engineering standards." |
| `HALLUCINATED_API` | "The flagged symbol does not exist. Verify identifier names against the pre-PR repo and dependency manifests." |
| `DUPLICATE_CODE` | "Reuse the existing helper at <path>:<line> instead of re-implementing." |
| `STRING_MATCH_DOMAIN_LOGIC` | "Replace substring checks with the proper domain primitive (AST/parsed URL/IP/date/schema)." |
| `REPEATED_STRUCTURE_AS_CODE` | "Extract the N branches into a table + single dispatcher loop." |
| `DOC_OUT_OF_SYNC` | "Update the doc file(s) covering the changed public surface in this same PR." |
| `INVENTED_CONFIG` | "Remove the invented flag/env/key, or amend the issue body to introduce it as scope." |
| `PR_SIZE` | "Freeze the completed capped slice and move unmet acceptance criteria to ordered continuation issues; never push or merge this oversized diff." |
| `DESIGN_DRIFT` | "If the repo has a DESIGN.md, use its tokens (color/spacing/typography/component) instead of hardcoding values; match the adopted design language for any user-facing UI. Run `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lint-ui.sh` on changed UI files — it deterministically flags raw hex, off-grid spacing, ad-hoc z-index, and banned fonts. It also flags motion and input defects: motion with no `prefers-reduced-motion` fallback, infinite animation with no pause control (WCAG 2.2.2), a viewport blocking zoom (WCAG 1.4.4), and `:hover` styled without a `:focus` equivalent. Ship a reduced-motion fallback with every animation." |
| `ANNOUNCE_STATE_CHANGE` | "When a UI change updates content without moving focus — a list reloading, a save succeeding, a validation failing, a filter narrowing results — announce it. Put an **empty** live region in the markup (`role=\"status\"` for routine results, `role=\"alert\"` for errors) and fill it when the state changes. A region created together with its text is not announced, because there is no change for a screen reader to observe. Clear `aria-busy` when the work finishes. Network-driven states are checked automatically by `ui-liveregion-evidence.mjs`, which fails the request to see whether the app says anything; **states no request produces — form validation, optimistic updates, client-side route changes, empty states — are only reachable if you declare them.** When you build one, add it to `.autospec/ui-test-hooks.json` and expose the hook, so the state stays checked rather than assumed. Keep the hook to dispatching states the app already has; it is a test seam, not a second code path." |

### Enforcement and opt-out

- `TDD non-negotiable`, `No DB mocks/stubs`, and `No hardcoded secrets / unsafe
  git ops` are enforced deterministically by `lint-implementation.sh`.
- **Inline escape hatch:** `# linter:allow-RULE_ID <reason>` on the same line as,
  or the line immediately before, the offending pattern. A bare
  `# linter:allow-X` with no reason is rejected and the rule stays active.
- **Per-issue opt-out** (issue body): `Guardian: skip-RULE_ID # <mandatory
  justification>`. Bare `Guardian: skip-X` is rejected. Skips are emitted as
  `INFO:RULE_ID...` (audit trail) but do NOT block the merge, and apply only to
  the PR derived from that issue.
- `AUTOSPEC_NO_GUARDIAN=1` short-circuits the guardian to the LGTM-only path
  (logged as `WARN: guardian disabled by AUTOSPEC_NO_GUARDIAN`).

## Lock-step discipline

Many autospec skills ship as a multi-harness trio: `SKILL.md`,
`codex/prompt.md`, and `opencode/agent.md`. The trio bodies MUST stay
byte-identical (after frontmatter is stripped per the repo convention
`awk '/^---$/{c++; next} c>=2'`).

- **Author `SKILL.md` only**, then regenerate `codex/prompt.md` and
  `opencode/agent.md` so the bodies are byte-identical. Never hand-edit the
  three copies independently.
- A SKILL.md + codex/prompt.md **duo** (no opencode/agent.md) must also stay
  byte-identical.
- Renaming any prose section heading in a trio SKILL.md requires updating
  `validate.sh` named-content checks in the same PR.
- `validate.sh` is the authoritative lock-step gate. Run it before every
  push; a non-zero exit means the trio (or a named-content check) diverged.

## Heartbeat schema

Liveness is tracked per-issue, per-repo via
`skills/autospec-run/scripts/heartbeat-write.sh`:

```
heartbeat-write.sh --issue <N> --step <step> [--branch <b>] [--pr <p>] --repo <owner/repo> \
  --worker-id <worker> --claim-id <claim> --session-id <actual-wait-session>
```

- Writes `~/.autospec/process-heartbeats/<repo-key>/<issue>.json`. Rust claim
  acquisition uses the collision-safe `o<len>_<owner>_r<len>_<repo>` key; the
  shell writer uses the canonical `owner__repo` key, and the reader recognizes
  both plus the documented legacy forms.
- Update the heartbeat at each major step:
  `claimed → worktree_ready → tests_started → tests_passed → pr_created →
  reviewed → merged`.
- Delete the heartbeat on any terminal outcome (merged, failed, or returned to
  queue).
- Recovery-bound writes also update
  `<repo-key>/sessions/<hex-actual-wait-session>.json`. Wait recovery
  reads that sidecar by the failed target's actual session ID; it never derives
  a claim generation from the currently active issue claim. Legacy heartbeats
  remain liveness-only and are ineligible for recovery.
- The exact-session sidecar is create-once. A refresh with the same
  session/issue/worker/branch/claim identity is idempotent and updates only the
  per-issue liveness file; a conflicting identity fails closed without replacing
  either record.
- `--issue` accepts only canonical positive integers before any heartbeat path
  is constructed.

## Worktree / branch rules

- Branch name: `feat/<slug>` derived from the issue.
- Create an isolated worktree off freshly-fetched `origin/main`:

  ```bash
  git fetch origin
  git worktree add -b feat/<slug> /tmp/wt-feat-<slug> origin/main
  cd /tmp/wt-feat-<slug>
  ```

- Do all work in the worktree. Clean the worktree up on every terminal outcome.
- A post-merge branch-switch error in the original checkout is harmless.

## Retry / review loop contract

1. **Pattern survey** (mandatory, before any code): search for analogous
   utilities; document the reuse decision in the PR body.
2. **Expand:** read the issue body in full; verify every referenced file path
   exists. If a path is wrong or the contract is ambiguous in a way that changes
   implementation, comment on the issue and exit — do NOT guess.
3. **Implement + TDD:** failing test first (for functional changes), then code,
   then green.
4. **Finalize:** run the project's full test/lint command; if the diff touches a
   migration path, run the migration-replay hook before opening the PR; run the
   docs-drift gate.
5. **Peer-review (Codex):** if `codex` is on PATH, get a second opinion on the
   diff; apply must-fix findings as additional commits and re-run tests; append
   nice-to-have findings verbatim to the PR body under
   `## Peer-review notes (not addressed)`. If `codex` is absent, log
   `Peer-review: codex not on PATH, skipping` and proceed.
6. **Adaptive retry:** on a blocking finding, re-run the implement step with the
   corrective directive(s) accumulated as cumulative context, up to
   `MAX_IMPL_RETRIES`. On the **second or later** retry of the *same* RULE_ID,
   before re-running, run the `## Advisor escalation` protocol (from
   `autospec-run/SKILL.md`) with `--gate retry` — send the stuck RULE_ID plus the
   failing finding as the question. Inject a returned `plan`/`correction` as an
   additional corrective directive for this retry instead of blindly re-running the
   same tier; on `stop` (or cap-reached / disabled), fall back to the normal
   retry/exhaustion behavior. On exhaustion, comment
   `Implementer hit max retries; manual intervention needed`, release the
   lock-label, and stop.

## Merge-gate summary

Immediately before `gh pr create`, then before `gh pr merge`:

1. **Lock-step deps:** re-check every `Depends on issue #N` line — each must
   return `CLOSED` via `gh issue view <N> --json state --jq .state`. If any dep
   is not merged, do NOT open the PR; comment which dep blocks and exit.
2. **Full test suite gate:** the repo's full validation/test suite (default
   `autospec validate`, override `AUTOSPEC_FULL_TEST_COMMAND`) MUST pass
   before LGTM review and again immediately before admin-merge. A failing suite
   blocks both review and merge — fix, recommit, rerun, repeat. Record the exact
   command and passing summary as merge evidence.
3. **Smoke gate:** run the issue body's **Primary smoke test** command. If it is
   not an executable single-command fence, comment and exit (missing AC).
4. **Rebase-and-retest:** if `mergeStateStatus` is `BEHIND`, `gh pr update-branch`
   and wait for CI green (cap `AUTOSPEC_REBASE_MAX_ATTEMPTS`, default 3). `DIRTY`
   (merge conflict) → comment and exit for human resolution.
5. **Sandbox guard:** if `.autospec/explore-mode.json` exists, target its
   `branch` as PR base and REFUSE any merge to `main`
   (`code_health:explore_main_merge_refused`).
6. **Merge:** when the merge-state is `CLEAN`/`HAS_HOOKS`/`UNSTABLE` and all
   gates pass: `gh pr merge <PR> --admin --squash --delete-branch`.
7. **Closeout evidence:** record the Closeout report (below) as merge evidence
   alongside the passing full-suite summary. Do NOT merge on a closeout you did
   not emit. Re-read the cited artifacts — treat the closeout as a claim, not
   proof — and downgrade any `[verified]` runtime claim whose proof type is
   `static`/build-only to `[assumed]`.

## Closeout report

End every issue by appending a **Closeout report** to the PR body and printing it
to the monitor log. Terse and result-first. Canonical contract: AGENTS.md
`## Closeout report contract`. Required fields:

- **Result** — one line, outcome first (open with the result, not "I'll"/"Let me").
- **Claims** — each load-bearing claim labeled `[verified]` (you checked it),
  `[assumed]` (inferred / another agent's word), `[couldnt-verify]`, or
  `[likely-wrong]`.
- **Proof type** — per `[verified]` claim, `runtime` or `static`. Runtime claims
  need runtime proof, not a build/read.
- **Before/after** — the measurable delta, or `n/a — <reason>` (mandatory field).
- **Artifacts** — exact paths + a re-runnable command.
- **Scoped git status** — the files this issue touched.
- **One likely hidden failure** — the most probable thing still wrong ("none" is a claim).

## Exit conditions

- **Success** — PR opened, CI green, admin auto-merge complete; heartbeat removed.
- **Soft fail (return to queue)** — clarification needed, lock-step blocked, or
  budget exhausted. Comment on the issue; do NOT open a PR; restore the
  `auto-implement` label; remove the heartbeat.
- **Hard fail (escalate)** — broken test infra, inconsistent repo state, or
  conflicting changes. Comment on the issue and add the `escalate:human` label.
