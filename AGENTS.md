# AGENTS.md

## Engineering standards

- **Conventional commits** (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`).
- **Branch-per-issue**: `feat/<slug>`. Never push to `main`.
- **Never bypass hooks** (`--no-verify`) or signing flags.
- **Never amend** committed PRs; create a new commit instead.
- **Lock-step rule** (per `CONTRIBUTING.md`): every multi-harness skill keeps `SKILL.md` / `opencode/agent.md` / `codex/prompt.md` bodies identical; only frontmatters differ.
- **Validation in lieu of code tests**: this repo has no language-level test runner. Validation is via shell scripts that check lock-step diffs, frontmatter parsing, `bash -n` on install scripts, and file presence. Each PR adds or extends a validation script that passes after the change.

## Subagent model selection (two-tier, cost-aware)

When the workflow dispatches a subagent, choose tier based on the **type of work**, not by phase number alone. Two tiers:

### Tier A — Specification work (top model + extended/maximum thinking)

Used by: research subagents (Phase 1), decomposition subagents (Phase 3 — turning a spec into linked GitHub issues), Phase 3.5 review-and-label subagents (turning issue bodies into model-fit metadata that drives all downstream filtering).

Reasoning: spec/issue quality is the bottleneck. A cheap model here costs you N cheap-implementer cycles correcting it downstream. The orchestrator/user is also typically running on a top model in Phase 2 (design + spec writing); subagents in spec-adjacent phases match that quality.

| Harness     | Preferred model | Thinking budget | Fallback (next-tier UP on unavailability) |
|-------------|-----------------|-----------------|--------------------------------------------|
| Claude Code | `opus` (current Claude Opus — e.g. `claude-opus-4-7`) | `ultrathink` (max thinking budget) | latest available top model |
| Codex CLI   | current top non-spark GPT (e.g. `gpt-5.1` or latest top-tier variant) | `reasoning_effort=high` | latest top variant |
| OpenCode    | top tier configured for `task` agents | provider-equivalent of "high" reasoning | next available |

### Tier B — Implementation work (cheaper model + medium thinking)

Used by: implementer subagents inside Phase 4's `process(ISSUE)` (the one writing code on `feat/*` branches), LGTM-review subagents (the inner-loop self-review of a PR).

Reasoning: implementation follows a well-specified contract from Tier A. The work is mechanical relative to the spec. We run this loop many times per spec, so cheaper-tier amortizes well.

| Harness     | Preferred model | Thinking budget | Fallback (UP on unavailability) |
|-------------|-----------------|-----------------|----------------------------------|
| Claude Code | `sonnet` (current Claude Sonnet — e.g. `claude-sonnet-4-6`) | medium thinking | `opus` → latest |
| Codex CLI   | `gpt-5.1-codex-spark` (or current spark/cost-optimized variant) | `reasoning_effort=medium` | next-larger Codex → latest |
| OpenCode    | smaller-tier task model | medium reasoning | next-larger configured tier |

### Harness detection protocol

Detect your harness by checking available tools before any phase, then resolve `TIER_A` and
`TIER_B` once for the entire skill run. Every "Tier A" and "Tier B" reference in skill bodies
maps to these harness-specific values.

1. **Claude Code** — the `Agent` tool with a `subagent_type` parameter is available.
   - `TIER_A` = `opus` + `ultrathink`  (see Tier A table row above)
   - `TIER_B` = `sonnet`               (see Tier B table row above)

2. **OpenCode** — a `task` tool with model/tier configuration is available (no `subagent_type`).
   - `TIER_A` = top-tier task model + high reasoning
   - `TIER_B` = smaller-tier task model + medium reasoning

3. **Codex CLI** — neither `Agent` nor a configurable `task` tool is available; `apply_patch`
   is the primary edit tool.
   - `TIER_A` = current top GPT model + `reasoning_effort=high`
   - `TIER_B` = `gpt-5.1-codex-spark` + `reasoning_effort=medium`

**Fallback rule:** If TIER_B is unavailable in your harness (model unknown, quota/capacity
failure, authorization failure, or tool call returns an error for that model), silently
retry the same subagent dispatch with TIER_A instead. Never collapse the delegated work
back into the main session solely because the Spark/cheap model failed.

**Context preservation rule:** Model fallback must preserve the parent context. For Codex
native subagents, fork/inherit the current conversation context when retrying on the
latest top GPT model so the upgraded subagent sees the same task state, repository
findings, and constraints as the failed Spark attempt.

### Flexibility rule (both tiers)

If the preferred model name is rejected (deprecated, quota/capacity, unauthorized), retry
the subagent with the next tier **UP** — never silently downgrade below the tier's intent
and never switch to main-lane execution just because the cheaper model failed. Never
hard-code exact version strings in dispatch code; resolve "current Opus / Sonnet / spark /
top GPT" at call time so the skill survives model-family churn.

### Tier assignment by phase (quick reference)

| Phase | Skill(s) | Tier |
|-------|----------|------|
| 1 — Investigate (research) | autospec, autospec-define | A |
| 2 — Brainstorm + design | autospec, autospec-define | (orchestrator only — no subagent dispatch; user invokes skill on top model) |
| 3 — Decompose into issues | autospec, autospec-define | A |
| 3.5 — Review and label | autospec, autospec-define | A |
| classify (per-issue review) | autospec-classify | A |
| 4 — Implementer (process(ISSUE) in worktree) | autospec, autospec-run | B |
| 4 — LGTM self-review | autospec, autospec-run | B |
| 4 — Implementation guardian | autospec, autospec-run | A |

## Auto-merge authority for auto-implement PRs

Admin-merge `auto-implement` PRs (`gh pr merge <#> --admin --squash --delete-branch`) when:
- All required CI checks pass (slow optional checks pending is acceptable).
- The self-review subagent returned `LGTM`.
- PR closes an `auto-implement` issue from a `feat/*` branch.

Spec PRs (head branch matches `feat/spec-*` OR body contains a `Source spec` line
referencing `docs/specs/`) carry the same admin-merge authority: orchestrators run
`gh pr merge <#> --admin --squash --delete-branch` once required CI checks pass and
the body matches one of those criteria.
Escape hatch: set `AUTOSPEC_NO_AUTOMERGE_SPEC=1` to short-circuit the auto-merge and
fall back to "open PR + ask user".

## Stop mode authority

Operators can halt a running autospec monitor in two ways, both leaving clean state:

- **Graceful** (`/autospec-stop --graceful`, default): the monitor finishes the current `process(ISSUE)` to its natural end (success → admin-merge, or 3-iter failure → label restore + comment). The outer loop exits BEFORE dispatching the next issue.
- **Immediate** (`/autospec-stop --immediate`): the current `process(ISSUE)` commits any uncommitted work (`chore: WIP — autospec stop`), pushes the branch, marks the issue `paused-by-user`, inserts a `## Resume context` block, and exits at the next major-step boundary.

**Sentinel file**: `~/.autospec/stop.flag`. Two-line format: `<mode>\n<ISO8601> <user>@<host>`. Atomic write via `temp+mv`. Stale flags (>24h) are ignored with a WARN to stderr.

**`paused-by-user` label**: color `#d4c5f9` (lavender), created idempotently by the abort path. Issues carrying this label are removed from the `auto-implement` queue until `/autospec-stop --resume` strips the label.

**Resume procedure**: `/autospec-stop --resume` strips `paused-by-user` from every paused issue, restores `auto-implement`, and deletes `~/.autospec/stop.flag`. The `## Resume context` block is kept as an audit trail. The next `/autospec-run` invocation picks up the restored issues normally.

**Inline sub-modes**: both `/autospec` and `/autospec-run` accept `stop [--flag]` as a feature-request argument (regex `^\s*stop(\s+--\w+)*\s*$`, case-insensitive), routing through the same `scripts/autospec-stop.sh`.

## Startup self-update

Every multi-harness skill runs a preflight at startup that updates the installed copy
from `main` at most once per 24 hours (fail-open: any network or install error logs a
`WARN:` line and continues). Set `AUTOSPEC_NO_SELF_UPDATE=1` to skip. The canonical
bash block lives in `skills/autospec/SKILL.md` (`## Startup self-update` section) and
is mirrored byte-identically (modulo `SKILL_NAME=`) across all multi-harness skill trios.
`scripts/validate.sh` (`check_startup_preflight`) enforces byte-identity.

## Small-LLM target

Generated child issues are sized for 32B-class local LLMs. Pre-staged context, sectional spec anchors, checkbox AC, one Primary smoke test per inner loop.

## Anti-loop guardrails

Per spec §5.1, both the Phase 1 research subagent and the Phase 4
implementer subagent run under hard, no-wall-clock-cap limits to keep a
runaway model from burning tokens or getting stuck rewriting the same
file forever:

- **Phase 1 research subagent.** Max **25 tool calls**. If 3 consecutive
  read/grep calls return nothing useful, stop and write a best-effort
  summary even if it is incomplete. Never retry the same query verbatim.
- **Phase 4 implementer subagent.** Max **40 tool calls** per issue. Max
  **3 self-review iterations**. If the implementer rewrites the same
  file twice with no test progress, abort: comment the blocker on the
  issue, release the `locked-by-autospec-processor` label, and exit.
- **No wall-clock cap.** Both limits are tool-call / iteration based,
  not time based, so stalled work is detected by behavior, not clock
  time.
- **Where they live.** These limits live inline in
  `skills/autospec/SKILL.md` (Phases 1 and 4) and
  `skills/autospec-run/SKILL.md` (Phase 4). The lock-step rule
  replicates the same body to `opencode/agent.md` and
  `codex/prompt.md`.

## Listener-filed issues lifecycle

Per spec §4.1 and §5.3, issues filed by `autospec-listen` follow a
distinct two-step lifecycle on the way to the `auto-implement` queue:

- **Step 1: listener creates with `needs-classify`.** When the listener
  fires on an issue trigger and the user confirms, the resulting
  `gh issue create` call carries `--label needs-classify` (color
  `#fbca04`, idempotently created via
  `gh label create needs-classify --color fbca04 --force`). The issue
  is NOT yet on the implementation queue — it is a draft awaiting
  classification.
- **Step 2: classifier transitions to `auto-implement`.**
  `/autospec-classify` walks BOTH `auto-implement` AND `needs-classify`
  issues. After applying `ctx:*` / `reasoning:*` labels and inserting
  the `## Model fit` block, on any issue carrying `needs-classify` it
  ALSO performs:
  `gh issue edit <N> --add-label auto-implement --remove-label needs-classify`.
  Issues that already carried `auto-implement` (and not
  `needs-classify`) are re-classified in place; no label transition.
- **No auto-promotion.** There is no TTL-based promotion. Stuck
  `needs-classify` issues are swept by re-running `/autospec-classify`
  manually or via the sample crontab in
  `docs/runbooks/needs-classify-sweep.md`.

## Issue-quality contract

Every GitHub issue created by autospec (Phase 3 decomposer, Phase 3.5 reviewer, or
`/autospec-classify`) must satisfy the three rules below before an implementation
agent picks it up. The enforcer is `scripts/lint-issue.sh` (exits 0 on pass, N on
fail where N = number of findings).

### Goal concreteness

The `## Goal` section must contain exactly one sentence (one terminal `.`, `?`, or
`!`). It must NOT use bare vague verbs (`improve`, `enhance`, `optimize`, `polish`,
`simplify`, `refactor`, `harden`) unless the same sentence also contains a concrete
object (a file path, a backtick-quoted command/identifier, a number, or an
`UPPER_SNAKE` label/env-var). It must NOT use hedging words (`should`, `might`,
`could try`, `try to`).

PASS: `Add \`scripts/lint-issue.sh\` that exits non-zero if the body fails the §3 quality contract.`

FAIL: `Improve the decomposer prompt for better issue quality.`

### AC machine-checkability

Every non-blank line in the `## Acceptance criteria` section must start with
`- [ ] ` followed by content. Each item must contain at least one of: a path-shaped
token, a backtick-quoted span, an integer, or a regex literal. Each item must NOT
use subjective adjectives (`looks`, `feels`, `seems`, `clean`, `elegant`). Each
item ≤120 characters. Section must contain ≥1 item.

### Primary smoke test shape

The first fenced code block under `### Primary smoke test (inner loop)` must contain
exactly one non-blank, non-comment line. It must NOT contain `...`, `<TODO>`, `TBD`,
or `XXX`.

## Subagent vs inline decision matrix

Every autospec skill author MUST consult this matrix before choosing between a
nested subagent dispatch and inline (main-session) execution. Skills that fan
out work inline burn orchestrator context tokens; skills that over-dispatch pay
subagent boilerplate cost on trivial work. The defaults below normalize that
tradeoff across the skill family.

| Work shape | Choose | Why |
|---|---|---|
| Read-only exploration across many files (grep, file enumeration, pattern survey) | **Subagent (Explore type)** | Bounded context, returns short summary, doesn't pollute orchestrator |
| 2+ independent tasks with disjoint scopes | **Parallel subagents (foreground, worktree-isolated per PR #691)** | True parallelism; isolated branches prevent collision |
| Single-purpose long-running work (Phase 4 implementer, peer-review) | **Subagent (general-purpose, single-agent absorbed discipline per PR #653)** | Quarantines context; orchestrator stays lean |
| Risky / quarantine-worthy work (codex peer-review, security scans) | **Subagent (foreground)** | Failure doesn't poison orchestrator |
| Orchestration / decision-making | **Inline (main session)** | State must persist across turns |
| One-shot tool calls (single grep, single gh) | **Inline (Bash tool)** | Subagent overhead exceeds work |
| Short edits (1-3 file modifications) | **Inline (Edit tool)** | Subagent boilerplate dominates cost |
| Routing / control flow (stop, listen, classify-trigger) | **Inline** | Decision is the work |
| Multi-area fan-out (specs / docs / tests / impl / QA) | **Parallel subagents, one per area** | Token cost per area is independent; main session aggregates findings |

Each skill's `## Required capabilities & harness adapter` table carries a
**Subagent dispatch policy** row pointing back to this matrix; the
`scripts/validate.sh::check_agents_md_subagent_matrix` gate enforces lockstep
across every adapter trio.

## Implementation-quality contract

Every PR produced by an `auto-implement` agent must satisfy the rules below before
the LGTM reviewer is dispatched. The enforcer is `scripts/lint-implementation.sh`
(exits 0 on pass, N on fail where N = number of blocking findings, capped at 200).

### RULE_ID table

| RULE_ID | Detector | Tier | Threshold / regex |
|---|---|---|---|
| `OUT_OF_SCOPE` | det | path-list compare | files touched ∉ issue body `## Implementation outline` paths |
| `MISSING_TEST` | det | path-prefix scan | required test type from issue body `## Tests required` not present in diff under `tests/{unit,integration,smoke,e2e}/` |
| `COMPLEXITY` | det | line/regex scan | function >50 LOC, file >500 LOC, nesting >4 |
| `SECURITY` | det | regex match | `eval\(`, `exec\(`, `--no-verify`, `git reset --hard`, `rm -rf /`, AWS-key shape `AKIA[0-9A-Z]{16}`, GitHub-token shape `gh[pousr]_[A-Za-z0-9]{36,}`, private-key markers `-----BEGIN [A-Z ]*PRIVATE KEY-----` |
| `TODO_LEFT` | det | regex on non-test diff | `\b(TODO\|XXX\|FIXME)\b` |
| `MOCK_DB` | det | regex on test diff | `\b(mock\|stub)\b` near DB-symbol heuristics (`db\.`, `database`, `DataSource`, `pg`, `mysql`, `sqlite`) |
| `HALLUCINATED_API` | LLM | semantic | symbol referenced in diff not defined in diff, not in pre-PR repo (verifiable via repo search), not in dependency manifests |
| `DUPLICATE_CODE` | LLM | semantic | new code mirrors an existing helper (must cite `<path>:<line>`) |
| `STRING_MATCH_DOMAIN_LOGIC` | LLM | semantic | code uses substring checks against free-form text to encode domain meaning, AND a proper-representation library is imported in the file. Recognized primitives — Python: `rdkit`/`ast`/`urllib.parse`/`datetime`/`ipaddress`/`lxml`/`jsonschema`; JS/TS: `URL`/`Date`/`@babel/parser`/`acorn`/`ts-morph`/`zod`/`ajv`/`joi`; Go: `net/url`/`time`/`go/ast`/`net.ParseIP`/`encoding/json` + struct tags; Java: `java.net.URI`/`java.time.*`/`JavaParser`/`com.github.javaparser`/`javax.validation`; Scala: `java.net.URI`/`java.time.*`/`scalameta`/refined types/circe schemas; Rust: `url::Url`/`chrono`/`time`/`syn`/`std::net::IpAddr`/`serde` with strong types |
| `REPEATED_STRUCTURE_AS_CODE` | LLM | semantic | ≥5 branches in the same function/method sharing identical structural shape (same return-tuple/case-class/struct-literal shape, same predicate signature, same side-effect line). Language-agnostic — Python if/elif, Java/Scala switch/match, Rust match arms, Go switch cases, JS if/else |
| `DOC_OUT_OF_SYNC` | hybrid | det+LLM | det: any change to public surface (CLI flag, env var, exported function, config key) WITHOUT a touched doc file (`README*`, `AGENTS.md`, `docs/**`, `SKILL.md`); LLM: judges semantic accuracy when a doc IS touched |
| `INVENTED_CONFIG` | LLM | semantic | flag/env-var/config-key introduced in diff not present in issue body or referenced spec |

### Corrective directive map

Each RULE_ID has a single-line corrective directive injected into the implementer's
retry prompt as cumulative context.

| RULE_ID | Directive |
|---|---|
| `OUT_OF_SCOPE` | "Restrict diff to files listed in issue's ## Implementation outline. Revert other changes or amend the issue body." |
| `MISSING_TEST` | "Add a test under tests/<TIER>/ for the listed required test type before re-pushing." |
| `COMPLEXITY` | "Split functions >50 LOC, files >500 LOC, nesting >4. No copy-paste branches." |
| `SECURITY` | "Remove the flagged pattern. NEVER hardcode secrets, NEVER use --no-verify or git reset --hard, validate input at boundaries." |
| `TODO_LEFT` | "Remove TODO/XXX/FIXME from non-test code. File a follow-up issue if the work is genuinely deferred." |
| `MOCK_DB` | "Remove DB mock/stub. Use the real DB per AGENTS.md ## Engineering standards." |
| `HALLUCINATED_API` | "The flagged symbol does not exist. Verify identifier names against the pre-PR repo and dependency manifests." |
| `DUPLICATE_CODE` | "Reuse the existing helper at <path>:<line> instead of re-implementing." |
| `STRING_MATCH_DOMAIN_LOGIC` | "Replace substring checks with the proper domain primitive (SMARTS/AST/parsed URL/IP/date/schema). Substring-on-name is brittle to synonyms, locants, salt forms, escaping, and case." |
| `REPEATED_STRUCTURE_AS_CODE` | "Extract the N branches into a table + single dispatcher loop. In Python use a list of tuples or dict; in Java/Scala use a `Map`/sealed-trait registry; in Rust use a `&[(predicate, value)]` slice; in Go use a `[]struct{...}` table. Each new entry should be one row, not a ~10-line block." |
| `DOC_OUT_OF_SYNC` | "Update the doc file(s) covering the changed public surface in this same PR." |
| `INVENTED_CONFIG` | "Remove the invented flag/env/key, or amend the issue body to introduce it as scope." |

### Per-issue opt-out grammar

The issue body MAY declare per-RULE_ID opt-outs with mandatory justification.
Parsed by both the deterministic script and the LLM guardian.

```
Guardian: skip-MISSING_TEST # docs-only refactor, no behavior change
Guardian: skip-OUT_OF_SCOPE, skip-COMPLEXITY # large rename touching many files
```

Grammar (regex):

```
^Guardian:\s+(skip-[A-Z_]+(,\s*skip-[A-Z_]+)*)\s+#\s+\S.+$
```

Rules:

- Justification (text after `#`) is **mandatory**. Bare `Guardian: skip-X` is
  rejected (treated as malformed and ignored — RULE remains active).
- Skipped RULE_IDs are still emitted by the linter as `INFO:RULE_ID...`
  (audit-trail visibility) but do NOT block the merge.
- Skips apply only to the specific PR derived from this issue; they do NOT cascade
  to other issues.

### Env-var contract

- `AUTOSPEC_NO_GUARDIAN=1` — short-circuit guardian, fall back to LGTM-only path.
  Mirrors `AUTOSPEC_NO_AUTOMERGE_SPEC=1` and `AUTOSPEC_NO_SELF_UPDATE=1`. Logged
  as `WARN: guardian disabled by AUTOSPEC_NO_GUARDIAN` on every Phase 4 dispatch.

## Memory management scripts

Scripts for managing project memory files under `AUTOSPEC_MEMORY_DIR`
(`~/.claude/projects/-Users-<user>-IdeaProjects-autospec/memory/`).

| Script | Purpose | Flags |
|---|---|---|
| `scripts/memory-tags.yml` | Manifest mapping each `feedback_*.md` to 4–5 tags | — (data file) |
| `scripts/apply-memory-tags.sh` | Idempotent tagger: prepends `tags:` YAML frontmatter to each `feedback_*.md` | `--dry-run`, `--memory-dir DIR`, `--manifest FILE` |
| `scripts/install-implementer-precommit.sh` | Installs blocking pre-commit lint hook into an implementer worktree | `<worktree-path>` positional arg |
| `skills/autospec-shared/scripts/mempalace-compress.sh` | AAAK compression GC: wraps `mempalace compress`; no-op below LOC threshold | `--dir DIR`, `--threshold LOC`, `--dry-run`, `--quiet` |
| `skills/autospec-shared/scripts/mine-pr-history.sh` | Extract lessons from merged PR descriptions into `docs/memory/lesson_*.md` | `--repo OWNER/REPO`, `--output-dir DIR`, `--quiet` |
| `skills/autospec-shared/scripts/inject-relevant-memory.sh` | Grep/search `docs/memory/*.md` for keyword matches; emit top-k context block for skill prompt injection | `--context KEYWORDS`, `--top-k N`, `--memory-dir DIR` |

`AUTOSPEC_MEMORY_DIR` — override for the memory directory path (default: auto-detected
from `$HOME/.claude/projects/`).

`AUTOSPEC_COMPRESS_THRESHOLD` — LOC threshold for `mempalace-compress.sh` (default: `5000`).
`AUTOSPEC_COMPRESS_EVERY` — invoke compress every N calls to `auto-init-memory.sh` (default: `10`).
`AUTOSPEC_MINE_PR_HISTORY` — set to `1` to enable PR history mining in `auto-init-memory.sh` (off by default; bandwidth-heavy).
`AUTOSPEC_MINE_MIN_BODY` — minimum PR body length for `mine-pr-history.sh` (default: `200`).
`AUTOSPEC_MINE_LIMIT` — max PRs to scan per `mine-pr-history.sh` run (default: `200`).

## Pre-commit lint hook

`scripts/install-implementer-precommit.sh <worktree>` writes `.git/hooks/pre-commit`
into the given worktree. The hook runs `lint-implementation.sh --pre-commit --staged`
on `git diff --cached` and blocks commits containing RULE_ID violations.

`lint-implementation.sh` extended flags:
- `--pre-commit` / `--staged` — read staged diff (`git diff --cached`) instead of a PR diff
- `--directives` — reformat each finding as `Fix RULE_ID: <imperative action>` for use in implementer retry prompts

## CI-wait sentinel

Replaces synchronous `gh pr checks --watch` with a fire-and-forget background poller.

| Script | Purpose | Flags |
|---|---|---|
| `scripts/ci-wait.sh` | Spawns background CI poller; returns immediately | `<PR>`, `--timeout SECONDS`, `--required-only` |
| `scripts/ci-wait-poll.sh` | Reads sentinel; returns state as exit code | `<PR>` |
| `scripts/ci-wait-cleanup.sh` | Kills poller; removes sentinel files | `<PR>` |

Signal file: `~/.autospec/ci-state/<PR>.signal` — JSON `{pr, state, checks, settled_at}`.
State values: `pending | pass | fail | stalled`.
Exit codes from `ci-wait-poll.sh`: 0=pass, 1=fail/stalled, 2=pending, 3=no sentinel.

## Batch size policy

Default `AUTOSPEC_BATCH_SIZE=3`; force batch=1 when the next ready issue is `reasoning:deep` (high blast-radius work runs one-at-a-time per monitor session).

## Memory inventory

Persistent cross-session memory lives at [`docs/memory/`](docs/memory/).
Index: [`docs/memory/MEMORY.md`](docs/memory/MEMORY.md).

Memory types (mempalace wings):
- **semantic** — codebase facts, architecture, conventions
- **episodic** — session diary (`docs/memory/diary/`), in-flight project status
- **procedural** — playbooks, runbooks, recipes (also see SKILL.md files)
- **synthesis** — lessons learned (feedback patterns, anti-patterns, gotchas)

Read memories relevant to your task at session start. Write new memories by adding/editing files in `docs/memory/` and updating the index. Mempalace MCP layer (`mempalace search`, `mempalace traverse`, `mempalace kg_query`) is available if your tool supports MCP.
