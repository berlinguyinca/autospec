# Autospec Implementation Guardian — Design Spec

**Date**: 2026-05-02
**Repo**: github.com/berlinguyinca/autospec
**Status**: Approved (Phase 2 brainstorm complete; ready to decompose into issues)

## 1. Goals

Phase 4's `process(ISSUE)` inner loop today dispatches a single Tier-B
"LGTM reviewer" (`skills/autospec/SKILL.md:487`) that judges code
correctness on a finished PR. That reviewer catches diff-level bugs,
but it does NOT actively constrain the implementer against the broader
classes of failure that produce *AI slop*:

- **Hallucinated symbols** (functions, fields, files, env vars that
  don't exist).
- **Scope creep** (touching files outside exact files or trailing-slash
  directories declared by the issue's `## Implementation outline` and
  `## Files touched` union). The outline may embed safe paths in prose when
  individually backtick-wrapped; every non-blank `Files touched` line remains
  one strict standalone path declaration.
- **Code duplication** (re-implementing helpers that already exist).
- **Documentation drift** (changing public surface without touching
  README / SKILL.md / AGENTS.md).
- **Missing test types** (unit-only when the issue's `## Tests required`
  asks for unit + integration + smoke).
- **Complexity inflation** (long functions, deep nesting, copy-paste
  branches).
- **Security regressions** (`eval(`, hardcoded secrets, hook-bypass
  flags, `--no-verify`, `git reset --hard`).

The **Implementation Guardian** is a new Tier-A subagent that runs
inside Phase 4's `process(ISSUE)` inner loop, **upstream of the existing
LGTM reviewer**, and produces structured findings (`RULE_ID:<path>:<line>:
<description>`) that feed back into the implementer as cumulative
directives. The implementer must clear all guardian findings before the
LGTM reviewer is dispatched and the PR can be admin-merged.

The contract mirrors the proven Phase 3.5 audit pattern
(`scripts/lint-issue.sh` + Tier-A judge + 5-attempt cumulative-context
retry) — but applied to PR diffs instead of issue bodies.

**Non-goals** (Phase 2 explicitly):

- Replacing the existing LGTM reviewer (kept; guardian runs upstream).
- New operator-facing skill (`/autospec-guardian`). The guardian is
  internal Phase 4 mechanism — see `feedback_autospec_skill_per_capability`.
- Wall-clock timeouts (existing tool-call cap is sufficient).
- Cross-PR or repo-wide trend analysis (per-PR scope only).

## 2. Architecture

```
process(ISSUE) inner loop  (existing, max 3 iter)
  ┌──────────────────────────────────────────────────────────────┐
  │ implementer commits + pushes                                  │
  │ Primary smoke test                                            │
  │ ── NEW ────────────────────────────────────────────────────── │
  │   bash scripts/lint-implementation.sh <PR> --issue <N>        │
  │     → /tmp/guardian-<PR>.md   (deterministic findings)        │
  │   guardian-subagent (Tier A)                                  │
  │     → /tmp/guardian-<PR>.md   (LLM findings appended)         │
  │   if GUARDIAN_PASS && det_exit == 0:                          │
  │     proceed to LGTM                                           │
  │   else:                                                       │
  │     directives → implementer next iter (cumulative)           │
  │ ── END NEW ──────────────────────────────────────────────────  │
  │ LGTM review (existing, Tier B)                                │
  │ on LGTM: operator/full verification + admin-merge             │
  └──────────────────────────────────────────────────────────────┘
```

```
scripts/
  lint-implementation.sh   NEW — deterministic RULE_ID detector (mirrors lint-issue.sh)
  validate.sh              extended with check_lint_implementation_helpers

skills/autospec/SKILL.md
  Phase 4 process(ISSUE)   — inserts guardian dispatch block between
                             smoke-test (step 7 inner) and LGTM review
skills/autospec/codex/prompt.md      ← lock-step
skills/autospec/opencode/agent.md    ← lock-step

skills/autospec-run/SKILL.md         ← same Phase 4 wiring
skills/autospec-run/codex/prompt.md  ← lock-step
skills/autospec-run/opencode/agent.md ← lock-step

AGENTS.md
  ## Implementation-quality contract  — RULE_IDs, directive map, opt-out grammar

tests/
  fixtures/implementation-quality/
    good.diff
    bad-out-of-scope.diff
    bad-todo-left.diff
    bad-mock-db.diff
    bad-secret.diff
    bad-complexity.diff
    skip-respected.diff       — has Guardian: skip-X line
  unit/test_lint_implementation.bats
  unit/test_phase4_guardian_trio.bats     — byte-identity across 6 trio files
  unit/test_agents_md_guardian_contract.bats
  integration/test_guardian_flow.sh        — end-to-end (gated on $GH_TOKEN)
```

## 3. Implementation-quality contract — concrete rules

### 3.1 RULE_ID table (locked)

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
| `DOC_OUT_OF_SYNC` | hybrid | det+LLM | det: any change to public surface (CLI flag, env var, exported function, config key) WITHOUT a touched doc file (`README*`, `AGENTS.md`, `docs/**`, `SKILL.md`); LLM: judges semantic accuracy when a doc IS touched |
| `INVENTED_CONFIG` | LLM | semantic | flag/env-var/config-key introduced in diff not present in issue body or referenced spec |

### 3.2 Corrective directive map

Each RULE_ID has a single-line corrective directive injected into the
implementer's retry prompt as cumulative context. Map lives in
AGENTS.md (so all harnesses share one table).

| RULE_ID | Directive |
|---|---|
| `OUT_OF_SCOPE` | "Restrict the diff to exact files or descendants of trailing-slash directories declared in `## Implementation outline` or `## Files touched`. Revert undeclared files; incomplete scope must be corrected by the issue author." |
| `MISSING_TEST` | "Add a test under tests/<TIER>/ for the listed required test type before re-pushing." |
| `COMPLEXITY` | "Split functions >50 LOC, files >500 LOC, nesting >4. No copy-paste branches." |
| `SECURITY` | "Remove the flagged pattern. NEVER hardcode secrets, NEVER use --no-verify or git reset --hard, validate input at boundaries." |
| `TODO_LEFT` | "Remove TODO/XXX/FIXME from non-test code. File a follow-up issue if the work is genuinely deferred." |
| `MOCK_DB` | "Remove DB mock/stub. Use the real DB per AGENTS.md ## Engineering standards." |
| `HALLUCINATED_API` | "The flagged symbol does not exist. Verify identifier names against the pre-PR repo and dependency manifests." |
| `DUPLICATE_CODE` | "Reuse the existing helper at <path>:<line> instead of re-implementing." |
| `DOC_OUT_OF_SYNC` | "Update the doc file(s) covering the changed public surface in this same PR." |
| `INVENTED_CONFIG` | "Remove the invented flag/env/key, or amend the issue body to introduce it as scope." |

### 3.3 Per-issue opt-out grammar

The issue body MAY declare per-RULE_ID opt-outs with mandatory
justification. Parsed by both the deterministic script and the LLM
guardian.

```
Guardian: skip-MISSING_TEST # docs-only refactor, no behavior change
Guardian: skip-OUT_OF_SCOPE, skip-COMPLEXITY # large rename touching many files
```

Grammar (regex):

```
^Guardian:\s+(skip-[A-Z_]+(,\s*skip-[A-Z_]+)*)\s+#\s+\S.+$
```

Rules:

- Justification (text after `#`) is **mandatory**. Bare `Guardian:
  skip-X` is rejected (treated as malformed and ignored — RULE remains
  active).
- Skipped RULE_IDs are still emitted by the linter as `INFO:RULE_ID...`
  (audit-trail visibility) but do NOT block the merge.
- Skips apply only to the specific PR derived from this issue; they do
  NOT cascade to other issues.

### 3.4 Sizing / threshold caps

- Findings buffer (`/tmp/guardian-<PR>.md`) ≤ 200 lines hard cap. If
  exceeded, the deterministic linter exits 200 and the LLM guardian
  short-circuits with `RULE_ID:OUT_OF_SCOPE: too many findings — likely
  scope explosion`.
- Guardian subagent: max **20 tool calls** per dispatch (vs 40 for the
  implementer). Mirrors the Phase 3.5 audit budget.
- Per-RULE_ID emit cap: each RULE_ID may emit ≤ 10 lines per pass; an
  11th occurrence is collapsed to `RULE_ID:<...>: + N more (truncated)`.

## 4. Phase 4 inner loop — wiring

### 4.1 Call site (verbatim insertion)

The new block is inserted in `process(ISSUE)` step 7 of every Phase 4
trio, replacing the current line `- Run the **Primary smoke test** ...`
through the LGTM dispatch with:

```text
- Run the **Primary smoke test** from the issue body. If it fails, fix and recommit before guardian.
- **Guardian gate** (NEW):
    rm -f /tmp/guardian-<PR>.md
    bash scripts/lint-implementation.sh <PR> --issue <ISSUE> >> /tmp/guardian-<PR>.md
    det_exit=$?
    Dispatch a **foreground subagent** with brief: <see §4.2>
    If guardian returns GUARDIAN_PASS && det_exit == 0:
      Replace `<!-- guardian-block -->` PR comment with "Guardian: clean."
      proceed to LGTM dispatch.
    Else:
      Replace `<!-- guardian-block -->` PR comment with the findings (see §4.4).
      Append findings to implementer's retry context as cumulative `## Guardian directives — clear before re-push` block.
      Continue inner loop (count toward 3-iter cap).
- Dispatch a **foreground subagent** (LGTM, unchanged) ...
```

### 4.2 Subagent brief (Tier A, verbatim)

> **Model tier:** Tier A (audit work). Claude Code: `opus` + `ultrathink`; Codex: current top GPT + `reasoning_effort=high`; OpenCode: top task tier. Fall back UP on unavailability.
>
> You are the implementation guardian for PR #<PR> on {repo}, derived from issue #<ISSUE>.
>
> 1. Read AGENTS.md `## Implementation-quality contract` for the RULE_ID table and directive map.
> 2. Read issue #<ISSUE> body — note `## Implementation scope`, `## Implementation outline`, `## Files touched`, `## Tests required`, and any `Guardian: skip-*` lines.
> 3. Read deterministic findings already in /tmp/guardian-<PR>.md.
> 4. Run `gh pr diff <PR>` and `gh pr view <PR> --json files,title,body`.
> 5. Apply the LLM-tier RULE_IDs (HALLUCINATED_API, DUPLICATE_CODE, DOC_OUT_OF_SYNC semantic pass, INVENTED_CONFIG). Append findings to /tmp/guardian-<PR>.md as `RULE_ID:<path>:<line>: <one-line description>`. Honor `Guardian: skip-*` opt-outs by emitting `INFO:` instead of blocking.
> 6. Hard limits: max **20 tool calls**. If you cannot reach a verdict in 20 calls, append `RULE_ID:OUT_OF_SCOPE: guardian budget exhausted; PR needs human review` and exit.
> 7. If you appended ZERO blocking findings (only INFO lines OK), return ONLY the token: `GUARDIAN_PASS`. Otherwise return ONLY: `GUARDIAN_FAIL`.

### 4.3 Findings buffer schema

Path: `/tmp/guardian-<PR>.md`. Append-only across iterations. Header
written at start of each iteration:

```
# Guardian findings for PR #<PR> (issue #<N>)
# Iter <K> of 3 · YYYY-MM-DDTHH:MM:SSZ
```

Body lines:

```
RULE_ID:<path>:<line>: <one-line description>
INFO:RULE_ID:<path>:<line>: <opt-out: justification>
```

Cleaned up via `rm -f /tmp/guardian-<PR>.md` on `process(ISSUE)`
exit (success or fail). Re-deriveable from the PR comment body if
needed post-mortem.

### 4.4 PR comment + issue-body audit blocks

**PR comment (every iteration, edit-last):**

```markdown
<!-- guardian-block:begin -->
## Guardian findings (iter <K>/3)
- `RULE_ID` at `<path>:<line>` — <one-line>
- ...
*Re-evaluated on every push. Last update: YYYY-MM-DDTHH:MM:SSZ.*
<!-- guardian-block:end -->
```

Idempotent via `gh pr comment <PR> --edit-last` (or marker-based
recreate if no prior comment exists). On `GUARDIAN_PASS`, the comment
is replaced with `<!-- guardian-block --> Guardian: clean. <!-- /-->`.

**Issue-body block (terminal fail only, 3-iter cap exhausted):**

```markdown
<!-- guardian-fail:begin -->
## Guardian audit (failed)
PR #<PR> closed after 3 iterations with unresolved findings:
- ...
*Reopen by re-queuing the issue and pushing a clean diff.*
<!-- guardian-fail:end -->
```

Idempotent via markers. Mirrors the `## Resume context` pattern from
the stop-mechanism spec.

### 4.5 Failure path

When the inner loop exhausts 3 iterations with `GUARDIAN_FAIL` still
returning:

1. Apply label `guardian-blocked` (idempotent
   `gh label create guardian-blocked --color e11d21 --force`).
2. Append `## Guardian audit (failed)` block to issue body (see §4.4).
3. Run the existing failure path: comment failure on issue, swap label
   `in-progress-by-bot` → `auto-implement`, `gh pr close <PR>
   --delete-branch`.
4. Cleanup: `rm -f /tmp/guardian-<PR>.md`,
   `git worktree remove --force`.
5. Issue is re-queued — next monitor cycle picks it up. The
   `guardian-blocked` label remains until a clean PR lands.

## 5. Files & lock-step

### 5.1 New files

- `scripts/lint-implementation.sh` — deterministic RULE_ID detector.
  Exit code = number of blocking findings (0 = pass). Stdout =
  appended findings. Stderr = WARN/ERROR.
- `tests/fixtures/implementation-quality/{good,bad-*,skip-respected}.diff`
- `tests/unit/test_lint_implementation.bats`
- `tests/unit/test_phase4_guardian_trio.bats`
- `tests/unit/test_agents_md_guardian_contract.bats`
- `tests/integration/test_guardian_flow.sh`

### 5.2 Edited files (lock-step trio × 2 skills = 6 files)

- `skills/autospec/SKILL.md` Phase 4 — insert guardian block (§4.1).
- `skills/autospec/codex/prompt.md` — byte-identical lock-step.
- `skills/autospec/opencode/agent.md` — byte-identical lock-step.
- `skills/autospec-run/SKILL.md` Phase 4 — insert guardian block.
- `skills/autospec-run/codex/prompt.md` — lock-step.
- `skills/autospec-run/opencode/agent.md` — lock-step.

`autospec validate` extended with `check_lint_implementation_helpers`
and `check_phase4_guardian_block_lockstep` to enforce byte-identity.

### 5.3 AGENTS.md additions

New top-level section `## Implementation-quality contract` (sibling to
`## Issue-quality contract`), containing:

- RULE_ID table (§3.1) verbatim.
- Corrective directive map (§3.2) verbatim.
- Per-issue opt-out grammar (§3.3) verbatim.
- Env-var contract: `AUTOSPEC_NO_GUARDIAN=1` short-circuits (fall back
  to LGTM-only).
- Tier assignment update: append guardian to Tier A in the
  phase-mapping table.

## 6. Operator surfaces

### 6.1 Env vars

- `AUTOSPEC_NO_GUARDIAN=1` — short-circuit guardian, fall back to
  LGTM-only path. Mirrors `AUTOSPEC_NO_AUTOMERGE_SPEC=1` and
  `AUTOSPEC_NO_SELF_UPDATE=1`. Logged as `WARN: guardian disabled by
  AUTOSPEC_NO_GUARDIAN` on every Phase 4 dispatch.

(No `AUTOSPEC_GUARDIAN_TIER` env. Tier locked at A. Operators wanting
cheaper guardian disable it entirely.)

### 6.2 Standalone CLI usage

```bash
bash scripts/lint-implementation.sh <PR> --issue <N>     # deterministic only
bash scripts/lint-implementation.sh --diff-file <path>   # offline / pre-push
```

`--diff-file` mode is for ad-hoc operator runs against an unsubmitted
diff; `--worktree` is intentionally NOT supported (worktree state is
opaque to the linter; require an explicit diff input).

### 6.3 Labels (idempotent)

| Label | Color | Lifecycle |
|---|---|---|
| `guardian-blocked` | `#e11d21` | applied on terminal 3-iter fail; cleared on next clean PR |

`gh label create guardian-blocked --color e11d21 --force --repo {repo}`
runs once at the top of `process(ISSUE)` (idempotent).

## 7. Error handling

**Fail-open by default** (mirrors AGENTS.md ## Startup self-update
posture):

- Guardian subagent timeout / capacity / model-unavailable: log `WARN:
  guardian dispatch failed: <reason>; falling back to LGTM-only`.
  Continue to LGTM. Do NOT block the merge.
- `lint-implementation.sh` exits non-zero with stderr matching
  `^ERROR:` (script bug, not findings): same WARN, fall back to
  LGTM-only.
- `lint-implementation.sh` exits with N findings (non-zero): treated as
  blocking; this is the normal failure path, not a script error.
- Guardian disagrees with itself across iterations: cumulative buffer
  is the source of truth; latest iter's `GUARDIAN_PASS` clears all
  prior findings (the diff has changed).
- Guardian + LGTM disagree: not possible — guardian runs upstream;
  LGTM only sees post-guardian state.
- Stop-flag hit during guardian dispatch: respect existing
  stop-mechanism boundaries (§4.5 of stop-mechanism spec). On
  `--immediate`, abort guardian, commit WIP, mark `paused-by-user`.

**Hard fail (non-fail-open):**

- AGENTS.md is missing the `## Implementation-quality contract`
  section: `process(ISSUE)` exits with `ERROR: AGENTS.md missing
  ## Implementation-quality contract — re-run installer to repair`.
  This is a configuration error, not a runtime fault.

## 8. Testing strategy

Per AGENTS.md `## Engineering standards` ("validation in lieu of code
tests" — repo has no language test runner; bats + shell scripts).

**Unit tests** (`tests/unit/`):

- `test_lint_implementation.bats` — for each fixture diff
  (`tests/fixtures/implementation-quality/*.diff`), assert the linter
  emits the expected RULE_IDs and exits with the expected count.
  Coverage: every deterministic RULE_ID has a positive fixture (one
  bad-*.diff that triggers it) and a negative fixture (good.diff
  where it must NOT fire).
- `test_phase4_guardian_trio.bats` — extract the guardian dispatch
  block from each of 6 trio files and assert byte-identity (modulo
  frontmatter).
- `test_agents_md_guardian_contract.bats` — assert AGENTS.md contains
  the `## Implementation-quality contract` section, all 10 RULE_IDs,
  the directive map, and the opt-out grammar regex.

**Integration test** (`tests/integration/`):

- `test_guardian_flow.sh` — end-to-end: create a throwaway PR on a
  fixture branch in a sandbox repo, invoke `lint-implementation.sh`
  against the live PR, assert findings are emitted to
  `/tmp/guardian-<PR>.md` in the documented schema. Skipped in CI when
  `$GH_TOKEN` is unset (matches existing integration-test pattern).

**No e2e test** for the LLM guardian itself — Tier-A subagent dispatch
isn't testable without a live model. The deterministic-side tests
exercise everything regex-able; the LLM-side is exercised in real
Phase 4 runs.

## 9. Decomposition preview (for Phase 3 reference)

The decomposer subagent should generate roughly **9 children + 1
epic** along this dependency graph:

| # | Title (working) | ctx | reasoning | Depends on |
|---|---|---|---|---|
| 0 | EPIC: Implementation Guardian (umbrella) | — | — | — |
| 1 | Add `## Implementation-quality contract` to AGENTS.md | 32k | medium | — |
| 2 | Create `scripts/lint-implementation.sh` skeleton + arg parsing + RULE_ID emission framework | 64k | medium | 1 |
| 3 | Implement deterministic detectors in `lint-implementation.sh` (OUT_OF_SCOPE, MISSING_TEST, COMPLEXITY, SECURITY, TODO_LEFT, MOCK_DB, DOC_OUT_OF_SYNC det-half) | 64k | deep | 2 |
| 4 | Add `tests/fixtures/implementation-quality/*.diff` fixtures | 32k | shallow | 1 |
| 5 | Add `tests/unit/test_lint_implementation.bats` | 32k | medium | 3, 4 |
| 6 | Wire guardian dispatch block into `skills/autospec/SKILL.md` Phase 4 (lock-step trio) | 64k | medium | 1, 3 |
| 7 | Wire guardian dispatch block into `skills/autospec-run/SKILL.md` Phase 4 (lock-step trio) | 64k | medium | 1, 3 |
| 8 | Add `tests/unit/test_phase4_guardian_trio.bats` (byte-identity across 6 trio files) | 32k | shallow | 6, 7 |
| 9 | Add `tests/unit/test_agents_md_guardian_contract.bats` | 32k | shallow | 1 |
| 10 | Update `autospec validate` with `check_lint_implementation_helpers` + `check_phase4_guardian_block_lockstep` | 32k | medium | 3, 6, 7 |

(`tests/integration/test_guardian_flow.sh` folds into #5 to stay under
the 3-files-touched cap.)

## 10. Out of scope

- New top-level `/autospec-guardian` skill. Per
  `feedback_autospec_skill_per_capability`, internal Phase 4 mechanism
  → no operator-facing skill.
- Cross-PR trend analysis or guardian dashboards.
- Wall-clock timeouts (tool-call cap is sufficient).
- Auto-fix mode (guardian emits findings; implementer fixes them on
  retry). Auto-patching by the guardian itself is rejected — too easy
  to introduce its own slop.
- Replacing the existing LGTM reviewer.
- Per-language hallucination detection beyond grep/regex (e.g. AST
  walks). Tier-A LLM does the semantic pass; specialized linters can
  bolt on later as additional deterministic detectors.
