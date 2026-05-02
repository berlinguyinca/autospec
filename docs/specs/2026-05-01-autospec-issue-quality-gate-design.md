# Autospec Issue-Quality Gate — Design Spec

**Date**: 2026-05-01
**Repo**: github.com/berlinguyinca/autospec
**Status**: Approved (Phase 2 brainstorm complete; ready to decompose into issues)

## 1. Goals

Every GitHub issue created by autospec (`/autospec` Phase 3, `/autospec-define`
Phase 3, and the standalone `/autospec-classify`) must satisfy a
**clearly-defined / reachable / verifiable** quality contract before the
implementer monitor picks it up. Today the Phase 3 decomposer prompt
*describes* the contract (`skills/autospec/SKILL.md:183` "Each item
machine-checkable"; line 184 "exactly one fast command") but nothing
*enforces* it; failures land silently and consume implementer cycles.

The contract narrows to three properties (per Phase 2 Q2):

1. **Goal concreteness** — 1-sentence outcome with a concrete object
   (file path, command, label, number). No bare vague verbs
   (`improve|enhance|optimize|polish|simplify|refactor|harden`); no
   hedging (`should|might|could try|try to`).
2. **AC machine-checkability** — every checkbox starts `- [ ]`, contains
   a path/backtick/number/regex shape, ≤120 chars, no subjective
   words (`looks|feels|seems|nice|clean|elegant|appropriate`).
3. **Primary smoke test shape** — exactly one fenced bash block with
   exactly one command line (`&&` chains allowed), no `...` /
   `<TODO>` / `TBD` / `XXX` placeholders.

Non-goals (per Phase 2 Q2, only Core bundle approved): structural
section presence beyond today's `needs-autospec-template`, reachability
cross-checks between Implementation outline and Files-to-read-first,
sizing-cap re-enforcement (those live in the Phase-3 prompt text).

## 2. Architecture

A single shared script `scripts/lint-issue.sh` is the source of truth
for the contract. Two enforcement points invoke it:

- **Phase 3 (decomposer subagent)** — pre-filing self-check on each
  candidate body before `gh issue create`. Retry once with tightened
  prompt on fail; surface inline to operator if still failing.
- **Phase 3.5 (review subagent)** — post-filing audit on every
  just-created child issue. Apply `needs-quality-bar` label, insert a
  `## Quality lint` block, post a comment with findings on fail. Do
  NOT remove `auto-implement` — operator decides whether to proceed.

Both `skills/autospec` and `skills/autospec-define` Phase 3 prompts get
the lock-step trio update; both `skills/autospec` and
`skills/autospec-define` Phase 3.5 prompts plus the standalone
`skills/autospec-classify` SKILL.md get the audit-step update.

```
scripts/
  lint-issue.sh         NEW — see §6 contract
  validate.sh           extended with check_lint_issue_helpers

skills/autospec/SKILL.md
  Phase 3 (lines 166-210)        — adds: "lint each candidate body via
                                   scripts/lint-issue.sh before gh issue
                                   create; retry once on fail; surface
                                   inline if still failing"
  Phase 3.5 (lines 212-337)      — adds: "lint each just-created child
                                   issue; apply needs-quality-bar label,
                                   insert ## Quality lint block, comment
                                   findings if fail"

skills/autospec-define/SKILL.md  ← lock-step copy of both updates
skills/autospec-classify/SKILL.md ← retro-audit step mirroring Phase 3.5

tests/
  fixtures/issue-quality/{good,bad-goal,bad-ac,bad-smoke}.md
  unit/test_lint_issue.bats
  unit/test_phase3_lint_integration.bats
```

## 3. Quality contract — concrete rules

### 3.1 Goal concreteness

**Goal section**: lines between `## Goal` and the next `## ` header.

PASS (concrete + bounded):
```
## Goal

Add `scripts/lint-issue.sh` that exits non-zero if the body fails the §3 quality contract.
```

FAIL — bare vague verb:
```
## Goal

Improve the decomposer prompt for better issue quality.
```

FAIL — hedging:
```
## Goal

Should probably refactor the AC checkbox handling.
```

Rules:
- Goal section content (after the heading, trimmed) must be exactly one
  sentence (one terminal `.`, `?`, or `!`; intermediate punctuation
  allowed).
- Must NOT match the regex `\b(improve|enhance|optimize|polish|simplify|refactor|harden)\b`
  *unless* the same sentence also contains a concrete object: a path
  (`/`-prefixed or matching `*.{md,sh,bash,yml,yaml,json,bats,py,js,ts,go,toml}`),
  a backtick-quoted command/identifier, a number, or a label/env-var
  in `UPPER_SNAKE` form.
- Must NOT match the regex `\b(should|might|could try|try to)\b`
  case-insensitively.

### 3.2 AC machine-checkability

**AC section**: lines between `## Acceptance criteria` and the next `## ` header.

PASS:
```
## Acceptance criteria

- [ ] `bash scripts/lint-issue.sh tests/fixtures/issue-quality/good.md` exits 0.
- [ ] `grep -c '^- \[ \]' tests/fixtures/issue-quality/bad-ac.md` returns >= 3.
- [ ] `scripts/validate.sh` exits 0.
```

FAIL — subjective:
```
- [ ] AC formatting looks clean and appropriate.
```

FAIL — prose, not checkbox:
```
- [ ] We add a check that runs after each issue is created.
```

Rules:
- Every non-blank line in the AC section must match the regex
  `^\s*-\s*\[\s\]\s+\S` (`- [ ]` followed by content).
- Each item must contain at least one of: a path-shaped token
  (`/` or `*.<ext>`), a backtick-quoted span, an integer, or a regex
  literal pattern (`\` characters indicating regex use).
- Each item must NOT match `\b(looks|feels|seems|nice|clean|elegant|appropriate)\b`.
- Each item ≤120 characters (excluding the leading `- [ ] `).
- Section must contain ≥1 item (empty AC is a fail).

### 3.3 Primary smoke test shape

**Smoke block**: the first fenced code block (any language tag) under
the `### Primary smoke test (inner loop)` subsection (or directly under
`## Verification` when the subsection heading is omitted).

PASS:
````
### Primary smoke test (inner loop)

```bash
bash scripts/lint-issue.sh tests/fixtures/issue-quality/good.md && echo OK
```
````

FAIL — multi-line:
````
```bash
cd /tmp
bash test.sh
```
````

FAIL — placeholder:
````
```bash
bash scripts/<TODO>.sh
```
````

Rules:
- Exactly one fenced code block under the Primary smoke heading.
- Block content (between fences, excluding language tag) must contain
  exactly one non-blank, non-comment line.
- Block content must NOT match `\.\.\.` or `<TODO>` or `\bTBD\b` or `\bXXX\b`.

## 4. Data model

| Path | Format | Producer | Consumer |
|---|---|---|---|
| `scripts/lint-issue.sh` | Bash, `set -eu`, exits 0 on pass / non-zero on fail | shipped via PR | Phase 3 + Phase 3.5 + operator |
| `tests/fixtures/issue-quality/{good,bad-goal,bad-ac,bad-smoke}.md` | Markdown body fixtures | shipped via PR | bats unit tests |
| `needs-quality-bar` GitHub label | `#fbca04` (yellow / warning) | idempotent at Phase 3.5 run start | `gh issue list --label needs-quality-bar` operator sweep |
| `## Quality lint` block (per-issue body insertion) | Markdown with `<!-- autospec-quality:begin --> ... <!-- autospec-quality:end -->` markers | Phase 3.5 audit | Operator review |

The body block convention mirrors the existing `## Model fit` block in
Phase 3.5 (`autospec/SKILL.md:271-289`): markered, idempotent (replace
in place between markers, never stack), inserted before the first
`## Dependencies` line or at end-of-body if absent.

## 5. Failure handling — adaptive retry with cumulative prompt enrichment

Per user directive 2026-05-01: *"these are all generated by LLM's so when you
lint/check for them, ensure you optimize the llm prompts until it gets it
right"*. The Phase-3 pre-filing path is therefore an **adaptive retry loop**,
not a one-shot retry. Each iteration accumulates the prior findings into the
prompt as actionable directives, so the LLM's next draft has the full failure
history to react to.

```
MAX_LINT_RETRIES = 5
attempt = 1
accumulated_directives = []
while attempt <= MAX_LINT_RETRIES:
    draft = generate_issue_body(base_prompt + accumulated_directives)
    findings = scripts/lint-issue.sh draft  # exit code = number of findings
    if findings is empty:
        gh issue create   # PASS — file the draft
        break
    accumulated_directives += findings_to_directives(findings)
    attempt += 1
else:
    # exhausted — surface to operator
    print all 5 drafts + their findings
    SKIP this child (do not file)
```

`findings_to_directives()` converts each `RULE_ID: <description>` into an
LLM-actionable guidance line, e.g.:

| Finding | Directive appended to next prompt |
|---|---|
| `GOAL_VAGUE: "improve" used without concrete object` | `AVOID: bare verb \`improve\` without naming a file path, command, label, or number in the same sentence.` |
| `GOAL_HEDGE: "should probably"` | `AVOID: hedging words \`should/might/could try/try to\`. State the outcome flatly.` |
| `AC_SUBJECTIVE: "looks clean"` | `AVOID: subjective adjectives \`looks/feels/seems/clean/elegant\` in AC items. Use a `grep`/`test`/`diff`/`bats` command instead.` |
| `AC_TOO_LONG: 145 chars` | `SHORTEN: AC item exceeds 120 chars; split into two AC items or compress to one assertion.` |
| `SMOKE_MULTI_LINE: 3 lines` | `COLLAPSE: Primary smoke test must be exactly one command line. Use `&&` to chain or extract setup into Operator/full verification.` |
| `SMOKE_PLACEHOLDER: contains "<TODO>"` | `RESOLVE: Replace placeholders \`<TODO>/TBD/XXX/...\` with the actual command before filing.` |

| Stage | Outcome | Action |
|---|---|---|
| Phase 3 attempt 1..N (N≤5) | lint fails | Append directives derived from findings; loop back, regenerate draft with cumulative context. |
| Phase 3 attempt K (K≤5) | lint passes | `gh issue create`; record `attempts=<K>` in the body's `## Quality lint` comment for later analysis. |
| Phase 3 attempt 5 | lint still fails | Print all 5 drafts + accumulated findings inline; SKIP that child (do not file); continue to next child. Operator can hand-fix or shrink the issue. |
| Phase 3.5 audit | lint passes | No-op. |
| Phase 3.5 audit | lint fails | Idempotent: `gh label create needs-quality-bar --color fbca04 --force` once at run start; `gh issue edit <N> --add-label needs-quality-bar`; insert `## Quality lint` block; `gh issue comment <N> --body "<findings>"`. Do NOT remove `auto-implement`. |

Phase 3.5 does NOT loop — it audits once. The retry loop is exclusively a
pre-filing optimization.

**Telemetry hook (lightweight)**: each successfully-filed child records
`attempts: <K>` inside the `<!-- autospec-quality:begin --> ... <!-- autospec-quality:end -->`
markers in its body. An operator running
`gh issue list --label auto-implement --state all --json body | jq` can later
spot rules that consistently take K≥3 attempts — those are signals that the
**base** Phase-3 prompt should permanently absorb the directive (a follow-up
out of scope for this spec).

The `## Quality lint` block format:

```markdown
## Quality lint

- **GOAL** — <1-line finding>.
- **AC#<n>** — <1-line finding>.
- **SMOKE** — <1-line finding>.

<!-- autospec-quality:begin -->
*Auto-linted by Phase 3.5 on YYYY-MM-DD.*
<!-- autospec-quality:end -->
```

## 6. lint-issue.sh contract

Authoritative interface for both enforcement points:

```bash
#!/usr/bin/env bash
# Usage:
#   scripts/lint-issue.sh <body-file>          # exit 0=pass, N=number of findings
#   scripts/lint-issue.sh --json <body-file>   # findings as JSON array
#   scripts/lint-issue.sh --help               # print rules summary
#
# Output (default, on fail): one finding per stderr line, format:
#   <RULE_ID>: <1-line description>
# where RULE_ID is GOAL_VAGUE | GOAL_HEDGE | GOAL_NOT_ONE_SENTENCE
#                | AC_PROSE | AC_SUBJECTIVE | AC_TOO_LONG | AC_EMPTY
#                | SMOKE_MULTI_LINE | SMOKE_PLACEHOLDER | SMOKE_NOT_FENCED
#
# Exit code = number of distinct findings (capped at 64); 0 means pass.
set -eu
```

The script is pure-bash with `grep -E`, `awk`, and `sed` only. No `jq`,
no `python` runtime. Self-contained so the Phase 3 subagent can
`bash scripts/lint-issue.sh /tmp/draft.md` without staging extra deps.

## 7. Testing

Per AGENTS.md (validation in lieu of code tests; real services; no mocks).

### 7.1 Bats unit (`tests/unit/test_lint_issue.bats`)

8–12 cases driven by golden fixtures under
`tests/fixtures/issue-quality/`:

| Fixture | Expect |
|---|---|
| `good.md` | exit 0, no findings |
| `bad-goal-vague.md` | exit ≥1, finding includes `GOAL_VAGUE` |
| `bad-goal-hedge.md` | exit ≥1, finding includes `GOAL_HEDGE` |
| `bad-goal-multi-sentence.md` | exit ≥1, `GOAL_NOT_ONE_SENTENCE` |
| `bad-ac-prose.md` | exit ≥1, `AC_PROSE` |
| `bad-ac-subjective.md` | exit ≥1, `AC_SUBJECTIVE` |
| `bad-ac-too-long.md` | exit ≥1, `AC_TOO_LONG` |
| `bad-ac-empty.md` | exit ≥1, `AC_EMPTY` |
| `bad-smoke-multi-line.md` | exit ≥1, `SMOKE_MULTI_LINE` |
| `bad-smoke-placeholder.md` | exit ≥1, `SMOKE_PLACEHOLDER` |
| `bad-smoke-no-fence.md` | exit ≥1, `SMOKE_NOT_FENCED` |
| `bad-multiple.md` | exit ≥3, all three rule families flagged |

### 7.2 Phase-3 trio integration (`tests/unit/test_phase3_lint_integration.bats`)

5–6 grep assertions:
- `skills/autospec/SKILL.md` Phase-3 section mentions `scripts/lint-issue.sh`.
- `skills/autospec/SKILL.md` Phase-3.5 section mentions `needs-quality-bar` and `## Quality lint`.
- `skills/autospec-define/SKILL.md` lock-step (same two greps).
- `skills/autospec-classify/SKILL.md` audit step mentions both.

### 7.3 Validator extension (`scripts/validate.sh`)

Add `check_lint_issue_helpers` (mirroring `check_self_update`):
- `bash -n scripts/lint-issue.sh` passes.
- `bash scripts/lint-issue.sh --help` exits 0 and prints "Usage:".
- `tests/fixtures/issue-quality/` directory exists and contains
  `good.md` plus at least 4 `bad-*.md` files.

No e2e against a live gh repo — the lint is body-text-on-text and adds
no system-integration risk worth the CI minutes.

## 8. Documentation updates

- `README.md` — add a one-paragraph "Quality gate" section near the
  feature list referencing `needs-quality-bar` and the lint script.
- `AGENTS.md` — add a `## Issue-quality contract` heading that
  inlines the three §3 rules so any human contributor knows the bar
  the autospec workflow enforces.
- `docs/runbook.md` (if it exists; skip otherwise) — document the
  operator sweep `gh issue list --label needs-quality-bar`.

## 9. Decomposition outline

EPIC umbrella: **Add issue-quality gate to autospec workflow**.

| # | Title | Files | Deps |
|---|---|---|---|
| 1 | Add `scripts/lint-issue.sh` (rule engine) + `--help` + `--json` | 1 | — |
| 2 | Golden fixtures: `tests/fixtures/issue-quality/{good,bad-goal,bad-ac}.md` (3 files) | 3 | 1 |
| 3 | Golden fixtures: `bad-smoke.md` + `bad-multiple.md` + `bad-ac-too-long.md` (3 files) | 3 | 1 |
| 4 | Bats unit tests `tests/unit/test_lint_issue.bats` | 1 | 2, 3 |
| 5 | Extend `scripts/validate.sh` with `check_lint_issue_helpers` | 1 | 1 |
| 6 | Wire **adaptive** pre-filing lint loop (MAX_LINT_RETRIES=5 + directive mapping table) into `autospec` Phase 3 trio | 3 (trio) | 1 |
| 7 | Wire adaptive pre-filing lint loop into `autospec-define` Phase 3 trio (lock-step copy of #6) | 3 (trio) | 6 |
| 8 | Wire post-filing audit into `autospec` Phase 3.5 trio | 3 (trio) | 1 |
| 9 | Wire post-filing audit into `autospec-define` Phase 3.5 trio | 3 (trio) | 8 |
| 10 | Wire audit step into `autospec-classify` trio | 3 (trio) | 8 |
| 11 | Bats integration: trio greps for `lint-issue.sh` mentions | 1 | 6, 7, 8, 9, 10 |
| 12 | Doc updates: README + AGENTS.md `## Issue-quality contract` | 2 | 8 |

Total: 12 child issues + 1 umbrella.

## 10. Out of scope

- Reachability cross-checks (Implementation outline ↔ Files-to-read-first).
- All-11-section presence check (today's `needs-autospec-template` two-section check stays).
- Sizing-cap re-enforcement (stays in Phase-3 prompt text).
- Rule customization via repo-local config files.
- Auto-fix of failing issues (the bar is detection + label only).
- Live-gh e2e tests (the lint is text-on-text).

## 11. Open questions

None at spec time. All five Phase-2 questions resolved.
