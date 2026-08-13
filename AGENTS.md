# AGENTS.md

## Engineering standards

- **Conventional commits** (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`).
- **Branch-per-issue**: `feat/<slug>`. Never push to `main`.
- **Never bypass hooks** (`--no-verify`) or signing flags.
- **Never amend** committed PRs; create a new commit instead.
- **Lock-step rule** (per `CONTRIBUTING.md`): every multi-harness skill keeps `SKILL.md` / `opencode/agent.md` / `codex/prompt.md` bodies identical; only frontmatters differ.
- **Validation and tests**: run the Rust test suite with `cargo test --workspace`. Also run the shell validation scripts that check lock-step diffs, frontmatter parsing, `bash -n` on install scripts, and file presence. Each PR adds or extends a validation script that passes after the change.

## Runtime resource isolation

- Use `autospec runtime env up|status|down|exec|session|gc|normalize-compose` for every
  manifest-v2 stack; use `down --purge-maven` only for the guarded Maven 4 prefix.
- `AUTOSPEC_MAVEN_ISOLATION=off`, `AUTOSPEC_COMPOSE_ISOLATION=off`, and
  `AUTOSPEC_ENV_DISABLE=1` export `AUTOSPEC_ISOLATION_BYPASSED=1`; never call the resulting
  isolation evidence verified.
- Runtime state is private on Unix (`0700` directories and `0600` files). Treat
  `RUNTIME_STATE_SYMLINK_REJECTED` and ownership ambiguity as fail-closed recovery signals;
  never delete an environment or session root manually.

## Subagent model selection (two-tier, cost-aware)

When the workflow dispatches a subagent, choose tier based on the **type of work**, not by phase number alone. Two tiers:

### Tier A — Specification work (top model + extended/maximum thinking)

Used by: research subagents (Phase 1), decomposition subagents (Phase 3 — turning a spec into linked GitHub issues).

Reasoning: spec/issue quality is the bottleneck. A cheap model here costs you N cheap-implementer cycles correcting it downstream. The orchestrator/user is also typically running on a top model in Phase 2 (design + spec writing); subagents in spec-adjacent phases match that quality.

**Tier right-sizing for classification (Phase 3.5 review-and-label + `/autospec-classify`):** classification is now **deterministic-first**. The deterministic rubric (file counts under `## Files to read first`, verb keywords — see `scripts/classify-model-fit.sh`, tracker #421) runs FIRST and gates any LLM call. Only issues the rubric scores as ambiguous (confidence below `LLM_ESCALATION_THRESHOLD`) get an LLM call, and that call runs at **Tier B**, not Tier A. Sibling normalization stays deterministic. This keeps the common case zero-LLM-cost while preserving a cheap LLM tie-breaker for genuinely ambiguous issues.

| Harness     | Preferred model | Thinking budget | Fallback (next-tier UP on unavailability) |
|-------------|-----------------|-----------------|--------------------------------------------|
| Claude Code | `opus` — the alias, which resolves to the current Claude Opus generation (`claude-opus-5` as of 2026-08) | `ultrathink` (max thinking budget) | latest available top model |
| Codex CLI   | the configured top non-spark model from `~/.codex/config.toml` (`gpt-5.6-sol` as of 2026-08) | `reasoning_effort=high` | latest top variant |
| OpenCode    | top tier configured for `task` agents | provider-equivalent of "high" reasoning | next available |

### Tier B — Implementation work (cheaper model + medium thinking)

Used by: implementer subagents inside Phase 4's `process(ISSUE)` (the one writing code on `feat/*` branches); the fused guardian + LGTM reviewer subagent (the inner-loop self-review of a PR — Tier B for ALL issues including `regression`/`priority:high`, see the env hatch below); and the Tier-B LLM tie-breaker for ambiguous classification (above).

Reasoning: implementation follows a well-specified contract from Tier A. The work is mechanical relative to the spec. We run this loop many times per spec, so cheaper-tier amortizes well.

| Harness     | Preferred model | Thinking budget | Fallback (UP on unavailability) |
|-------------|-----------------|-----------------|----------------------------------|
| Claude Code | `sonnet` — the alias, which resolves to the current Claude Sonnet generation (`claude-sonnet-5` as of 2026-08) | medium thinking | `opus` → latest |
| Codex CLI   | the spark / cost-optimized variant of the configured model when one exists, else the configured model | `reasoning_effort=medium` | next-larger Codex → latest |
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
   - `TIER_A` = configured top model (see Tier A table row above) + `reasoning_effort=high`
   - `TIER_B` = spark variant if present, else configured model + `reasoning_effort=medium`

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
| 3.5 — Review and label | autospec, autospec-define | deterministic-first; **B** on ambiguity |
| classify (per-issue review) | autospec-classify | deterministic-first; **B** on ambiguity |
| 4 — Implementer (process(ISSUE) in worktree) | autospec, autospec-run | B |
| 4 — Fused guardian + LGTM reviewer | autospec, autospec-run | **B** for ALL issues (incl. `regression`/`priority:high`); `AUTOSPEC_REVIEWER_TIER=opus` → A |
| 4 — Implementation guardian (Tier-A escape hatch) | autospec, autospec-run | **A** when `AUTOSPEC_REVIEWER_TIER=opus` (otherwise folded into the Tier-B fused reviewer row above) |

**`AUTOSPEC_REVIEWER_TIER` (reviewer escape hatch):** the fused guardian + LGTM reviewer runs at **Tier B for every issue**, including `regression` and `priority:high`. The former second Tier-A regression meta-review pass is folded into the single reviewer brief (the reviewer self-asks "would the reviewer have caught the original gap?" and writes any missing checks to `reports/autospec-review/reviewer-lessons.md`). To restore Tier A for the reviewer, set `AUTOSPEC_REVIEWER_TIER=opus`; unset (or any other value) keeps Tier B (sonnet). This is the one-variable revert if a high-stakes run shows the cheaper reviewer missing real bugs.

## Auto-merge authority for auto-implement PRs

Admin-merge `auto-implement` PRs (`gh pr merge <#> --admin --squash --delete-branch`) when:
- The full target-repo validation/test suite has passed locally after the branch is current with `main`.
- All **non-advisory** required CI checks pass — checks matching `AUTOSPEC_PR_ADVISORY_CHECKS` (default `AUTOSPEC_MAIN_HEALTH_IGNORE_CHECKS`; e.g. self-hosted TeamCity) are advisory and may be pending **or failing** once the full local suite is green.
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

## Autonomy charter (default-on)

The operator's standing preference is **autonomous by default**: when
`~/.autospec/autonomous.flag` is present, `/autospec-define` and `/autospec-run`
skip design-ratification, spec→plan→run handoff gates, and per-issue
confirmations. The governing rule is **recommendation = action** — if the agent
is confident enough to recommend a next step, it takes it and reports rather than
asking permission. Safety is unchanged: `scripts/autospec-autonomy-gate.sh
--check all` still surfaces a confirmation for destructive remote actions,
force-push to a protected branch, out-of-scope files, cost over the aggressive
token cap (`AUTOSPEC_AUTONOMOUS_TOKEN_CAP`), and
genuine no-clear-winner forks. Full policy and rationale (mined from session
transcripts): [`docs/AUTONOMY-CHARTER.md`](docs/AUTONOMY-CHARTER.md).

## Startup self-update

Every multi-harness skill runs a preflight at startup that updates the installed copy
from `main` at most once per 24 hours (fail-open: any network or install error logs a
`WARN:` line and continues). Set `AUTOSPEC_NO_SELF_UPDATE=1` to skip. The canonical
bash block lives in `skills/autospec/SKILL.md` (`## Startup self-update` section) and
is mirrored byte-identically (modulo `SKILL_NAME=`) across all multi-harness skill trios.
`autospec validate` (`check_startup_preflight`) enforces byte-identity.

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
`autospec validate::check_agents_md_subagent_matrix` gate enforces lockstep
across every adapter trio.

## Implementation-quality contract

Every PR produced by an `auto-implement` agent must satisfy the rules below before
the LGTM reviewer is dispatched. The enforcer is `scripts/lint-implementation.sh`
(exits 0 on pass, N on fail where N = number of blocking findings, capped at 200).

### RULE_ID table

| RULE_ID | Detector | Tier | Threshold / regex |
|---|---|---|---|
| `PR_SIZE` | det | git diff/numstat | hard above 400 additions+deletions, 8 raw files, or 3 normalized logical units; binary rows are always hard |
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
| `PR_SIZE` | "Freeze the completed capped slice and move unmet acceptance criteria to ordered continuation issues; never push or merge this oversized diff." |
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

### Enforcement

The following rules are enforced deterministically by `scripts/lint-implementation.sh`.
Each rule has an inline escape hatch for genuine exceptions.

| Rule | Enforcing check | `linter:allow-` escape hatch |
|---|---|---|
| TDD non-negotiable (test must accompany every non-docs change) | `MISSING_TEST` detector | `# linter:allow-MISSING_TEST <reason>` on the issue body or in-file |
| No DB mocks/stubs in tests | `MOCK_DB` detector | `# linter:allow-MOCK_DB <reason>` — allowed only for unit tests with no accessible DB |
| No hardcoded secrets or unsafe git operations | `SECURITY` detector | `# linter:allow-SECURITY <reason>` — allowed only for test fixtures with non-secret values |

**Inline escape hatch syntax** (in source code, not issue body):

```
# linter:allow-MOCK_DB integration test requires mock — no test DB available in CI
# linter:allow-MISSING_TEST docs-only change, no behavior to test
# linter:allow-SECURITY fixture value is not a real secret
```

The `linter:allow-` comment must appear on the same line as or the line immediately before
the offending pattern. A bare `# linter:allow-X` without a reason is rejected and the
rule remains active. Allowed escape hatches are emitted as `INFO:RULE_ID:...` (audit trail)
but do NOT block the merge.

The existing `Guardian: skip-RULE_ID` opt-out grammar in the issue body remains valid for
per-PR-level skips. Inline `# linter:allow-*` is for line-level exceptions inside the code.

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
- `PR_SIZE` accepts only `generated migration: <generator>`,
  `dependency-solver lockfile: <solver>`, or
  `mandatory lock-step artifacts: <identity>`. The measured diff must prove the
  stated category; binary, forged, mixed manual code, and nested test paths stay blocking.
- Skipped RULE_IDs are still emitted by the linter as `INFO:RULE_ID...`
  (audit-trail visibility) but do NOT block the merge.
- Skips apply only to the specific PR derived from this issue; they do NOT cascade
  to other issues.

### Env-var contract

- `AUTOSPEC_NO_GUARDIAN=1` — short-circuit guardian, fall back to LGTM-only path.
  Mirrors `AUTOSPEC_NO_AUTOMERGE_SPEC=1` and `AUTOSPEC_NO_SELF_UPDATE=1`. Logged
  as `WARN: guardian disabled by AUTOSPEC_NO_GUARDIAN` on every Phase 4 dispatch.

## Closeout report contract

Every `auto-implement` agent ends an issue by emitting a **Closeout report** —
appended to the PR body and printed to the monitor log. It is the structured,
result-first summary the merge-gate and the done-challenge consume as *evidence*.
Keep it terse: a tight body, long only where a claim genuinely needs it.

Required fields (exact field names are gated by `autospec validate`):

- **Result** — one line, outcome first (what shipped), not a narration of the
  agent's own process. Open with the result, not "I'll" / "Let me".
- **Claims** — each load-bearing claim carries one label: `[verified]` (the agent
  checked it itself), `[assumed]` (inferred or taken from another agent's report),
  `[couldnt-verify]`, or `[likely-wrong]`. Unlabeled load-bearing claims are a
  defect.
- **Proof type** — for each `[verified]` claim, `runtime` or `static`. **Runtime
  claims need runtime proof, not just a build/read.** A `[verified]` runtime claim
  backed only by static/build evidence is downgraded to `[assumed]` by the
  consumer (see below).
- **Before/after** — the measurable delta this change produced (test count, perf
  number, error rate, …) or an explicit `n/a — <reason>`. A before/after is the
  marker of real work; the field is mandatory (the reason may be `n/a`).
- **Artifacts** — exact file paths and a re-runnable command a reviewer can
  execute to reproduce the proof.
- **Scoped git status** — the files this issue touched (scoped, not a raw global
  status dump).
- **One likely hidden failure** — the single most probable thing still wrong. Not
  optional; "none" is itself a claim to be challenged.

### Consumer contract (critic / merge-gate)

The merge-gate and the autospec-run done-challenge treat the Closeout report as a
**claim, not proof**:

- Record the Closeout report as merge evidence (alongside the full-suite passing
  summary) — never merge on a closeout the agent did not actually emit.
- Re-read the cited artifacts; do not accept a closeout's word for them.
- **Reject (or downgrade to `[assumed]`) any `[verified]` runtime claim whose
  proof type is `static`/build-only.** This is the one machine-checkable critic
  predicate.

The judgment-bound discipline that cannot be gated, applied throughout an issue:
state the blast radius before any global/destructive action; stay in the issue's
scope and park unrelated findings as follow-up issues rather than expanding the
diff.

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

## Auto context rollover skills

### autospec-session launcher (`scripts/autospec-session`)
Starts the `autospec_context_monitor` daemon in the background for a given tmux session.
The daemon monitors context percentage and fires compact/handoff/clear/resume actions automatically.
Stop it with `kill $(cat ~/.autospec/context-monitor.pid)` or by ending the tmux session.

- `/autospec-rollover-status` — reports current context % and last rollover event for the active session (see [`docs/specs/2026-05-31-auto-context-rollover-design.md`](docs/specs/2026-05-31-auto-context-rollover-design.md)).

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

Default `AUTOSPEC_BATCH_SIZE=1`; force batch=1 when the next ready issue is `reasoning:deep` (high blast-radius work runs one-at-a-time per monitor session). This only ends the current monitor batch: `autospec-run` must automatically relaunch fresh monitor batches until the queue is `ALL_DONE`.

## Memory inventory

Persistent cross-session memory lives at [`docs/memory/`](docs/memory/).
Index: [`docs/memory/MEMORY.md`](docs/memory/MEMORY.md).

Memory types (mempalace wings):
- **semantic** — codebase facts, architecture, conventions
- **episodic** — session diary (`docs/memory/diary/`), in-flight project status
- **procedural** — playbooks, runbooks, recipes (also see SKILL.md files)
- **synthesis** — lessons learned (feedback patterns, anti-patterns, gotchas)

Read memories relevant to your task at session start. Write new memories by adding/editing files in `docs/memory/` and updating the index. Mempalace MCP layer (`mempalace search`, `mempalace traverse`, `mempalace kg_query`) is available if your tool supports MCP.

## Git hygiene (agents)

These rules apply to every autospec skill that mutates the repository (run
implementers, define spec-PRs, doc regenerate commits, explore sandbox,
release). The enforcement tool is `scripts/worktree-guard.sh`.

### Primary checkout is read-only for agents

Agents MUST NOT `cd`, `git checkout`, or `git commit` in the primary checkout.
Operator dirt in it is never touched and never matters. Every git-mutating step
happens in a linked worktree, never in the primary checkout directory.

### Fetch-before-branch

Always run `git fetch origin` (with one automatic retry) before creating or
adopting a branch. `worktree-guard.sh create` handles this automatically;
callers that bypass `create` must fetch explicitly.

### Fresh-or-verified-clean worktrees only

Use `worktree-guard.sh create` to obtain a worktree. Before any edit or commit,
call `worktree-guard.sh assert`; it MUST exit 0. A non-zero exit means the
worktree is dirty, primary, or stale — stop work, comment on the issue, and
restore `auto-implement`. Never force-reuse a dirty worktree.

### PR-aware ladder (standard branch-exists behavior)

Before creating a new worktree, call `worktree-guard.sh resolve-branch`:

- `open-pr` — a PR already exists for this branch: validate + merge the
  existing PR; skip re-implementation (#886 recovery).
- `branch-only` — branch exists on origin but no open PR: adopt it in a fresh
  worktree and continue (#917 recovery).
- `fresh` — nothing exists: `worktree-guard.sh create` off `origin/main`.

### Cleanup after merge + prune

After a PR is confirmed merged, remove the worktree and prune the git metadata:

```bash
git worktree remove /tmp/wt-<branch> 2>/dev/null || true
git worktree prune
```

Never leave stale worktrees; the watchdog GC (`scripts/autospec-watchdog.sh`)
sweeps orphans as a safety net, but proactive cleanup is required.

The watchdog cross-checks the GitHub `autospec-run-state` comment before releasing
any `claimed` heartbeat — a live sibling's claim is never reclaimed on local age
alone. The default `claimed` threshold is **1800s**; override with
`AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS`. See SKILL.md §"Running concurrent workers"
for the full concurrency model and tuning table.

### Pointer to enforcement tool

`scripts/worktree-guard.sh` (installed to `~/.autospec/scripts/worktree-guard.sh`)
implements `assert`, `resolve-branch`, and `create`. See
`docs/specs/2026-06-03-worktree-guard-design.md` §D1 for the full contract and
pinned exit codes.
