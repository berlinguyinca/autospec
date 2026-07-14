# autospec-explore — perpetual autonomous research + ship loop on a sandbox branch

## Summary

A new top-level skill `/autospec-explore "<initial prompt>"` starts an
autonomous loop that refines the prompt, ships it, then continues
researching and proposing new features autonomously from 6 input sources
(specs vs code gaps, prior run reports, codebase signals, open GitHub
issues, repo source analysis, internet competitor research). All work
lands on a **sandbox branch** — never directly to `main`. The operator
inspects when ready and either merges the sandbox into `main` or discards.

The loop continues until the operator stops it, hits a hard cap, or
exhausts usage budget (the existing usage-limit supervisor pauses and
resumes when budget refreshes).

## Team personality

- **Selected team:** Core product engineering — product manager, architect,
  backend developer, test engineer, technical writer, security advisor.
- **Why this team fits:** the skill autonomously generates and ships
  features. Needs balanced product judgment (what's worth proposing),
  engineering rigor (sandbox isolation, loop bookkeeping), and writing
  craft (research findings ARE the artifact). Security advisor catches
  outbound web-research risks.
- **Risks this team will notice:** runaway feature creep, low-value
  proposals flooding the queue, competitor-research scraping outside fair
  use, sandbox branch divergence.
- **Carry into child issues:** every proposed feature must have a cited
  source, sandbox isolation is non-negotiable, internet research is rate-
  limited and domain-allowlisted.

## Review counter-team

- **Selected counter-team:** Security + maintainability + legal.
- **Why this counter-team:** the loop reads external content (competitor
  product pages, open issues), files autonomous code-change issues, ships
  code that lands on a branch the operator may merge unreviewed if they're
  not careful. Trust boundary is "anything autonomously discovered".
- **What this team should challenge:** can an attacker influence external
  research content to plant malicious proposals? Does the sandbox branch
  isolation actually hold under rebase / force-push / accidental
  operator commands? Are competitor features being recommended without
  license or patent review?

## Architecture

```
/autospec-explore "<prompt>"
        │
        ▼
   create sandbox branch
   autospec/explore/<date>-<slug> off origin/main
        │
        ▼
   ┌──────────────────────────────────────────┐
   │  perpetual loop (single iteration shown) │
   │                                          │
   │  1. research cycle:                      │
   │     - 6 researchers run in parallel      │
   │     - aggregate proposals, dedup, rank   │
   │  2. file 1-5 auto-implement issues       │
   │     (max per round, configurable)        │
   │  3. drain via /autospec-run              │
   │     - implementer PRs target SANDBOX,    │
   │       not main                           │
   │  4. update .autospec/explore-summary.md  │
   │  5. check termination:                   │
   │     - operator stop flag                 │
   │     - round cap / time cap / token cap   │
   │     - usage-limit supervisor arms        │
   │  6. loop                                  │
   └──────────────────────────────────────────┘
        │
        ▼ (operator decides)
   git merge autospec/explore/<date>-<slug> → main
        │
   OR discard: gh branch -D
```

Skill family layout (mirrors existing autospec-refine / autospec-continue):

- `skills/autospec-explore/SKILL.md` — Claude Code adapter (authoritative).
- `skills/autospec-explore/codex/prompt.md` — Codex CLI mirror (lockstep).
- `skills/autospec-explore/opencode/agent.md` — OpenCode mirror (lockstep).
- `skills/autospec-explore/install.sh`, `uninstall.sh`, `README.md`.
- `scripts/autospec-explore.sh` — orchestrator.
- `scripts/explore-sandbox.sh` — sandbox branch creation + base-branch
  context export.
- `scripts/explore-research-cycle.sh` — runs all researchers, aggregates.
- `scripts/explore-research/` (subdir) — one researcher per source:
  - `spec-vs-code.sh`
  - `prior-reports.sh`
  - `codebase-signals.sh`
  - `open-issues.sh`
  - `source-analysis.sh`
  - `internet.sh`

## Invocation

```
/autospec-explore "<initial prompt>" \
    [--max-iterations N] \
    [--max-issues-per-round N] \
    [--budget-tokens N] \
    [--budget-hours N] \
    [--sandbox-slug <slug>] \
    [--research-sources <comma-list>] \
    [--no-internet] \
    [--internet-allowlist <comma-list>]
```

- `--max-iterations N` — outer loop round cap. Default unlimited.
- `--max-issues-per-round N` — research output cap. Default 5.
- `--budget-tokens N` — token budget across all iterations. Default 10M.
- `--budget-hours N` — wall-time budget. Default 24h.
- `--sandbox-slug <slug>` — override sandbox branch slug.
- `--research-sources <list>` — limit to a comma-separated subset of the
  6 researcher names. Default: all 6.
- `--no-internet` — disable internet research (the most expensive +
  highest-risk source).
- `--internet-allowlist <list>` — comma-separated domains the internet
  researcher is permitted to fetch. Default: a curated list of
  competitor-research-appropriate domains (GitHub, official product
  docs, HackerNews, etc.). Forbidden by default: paywalled content,
  social media, pastebin-class sites.

## Sandbox branch contract

1. **Creation**: at run start, create `autospec/explore/<YYYY-MM-DD>-<slug>`
   off `origin/main` and push. The branch lives until the operator merges
   or deletes it.
2. **Implementer integration**: every child-issue implementer reads
   `.autospec/explore-mode.json` (written by the orchestrator) to learn the
   sandbox branch name. PRs target `--base <sandbox-branch>` instead of
   `main`. This is enforced by extending the Phase 4 implementer prompt
   template (`skills/autospec-run/prompts/phase4-implementer.md`).
3. **No accidental main merges**: orchestrator refuses to invoke
   `gh pr merge` against `main` while `.autospec/explore-mode.json` is
   present. The sandbox → main merge is a separate explicit operator
   action (`/autospec-explore-promote <sandbox-branch>` — out of scope
   for v1; documented as the manual path).
4. **Sandbox refresh policy**: the sandbox branch is NOT auto-rebased onto
   main. Operator does that explicitly. This is intentional — rebasing
   under autonomous shipping is unsafe.

## Research cycle contract

Each round runs the 6 researchers (or the operator-specified subset) in
parallel. Each researcher returns 0-N proposals as JSON:

```json
{
  "source": "spec-vs-code",
  "proposals": [
    {
      "title": "feat: implement <X> from spec docs/specs/<Y>.md",
      "evidence": "Acceptance criterion 3 in <Y>:42 has no implementation",
      "estimated_complexity": "small|medium|large",
      "confidence": 0.85
    }
  ]
}
```

Aggregation:

1. **Deduplication**: by normalized title (lowercased, action verb +
   subject), drop duplicates across researchers.
2. **Ranking**: weighted score = `confidence × source_weight ×
   1/estimated_complexity`. Default source weights:
   - `spec-vs-code` = 1.0 (highest — spec drift is concrete and grounded)
   - `prior-reports` = 0.9 (operator-derived priorities)
   - `codebase-signals` = 0.7
   - `open-issues` = 0.6
   - `source-analysis` = 0.5
   - `internet` = 0.4 (lowest — least grounded)
3. **Filtering**: drop proposals that match recently-filed issue titles
   (last 7 days) to prevent oscillation.
4. **Cap**: top `--max-issues-per-round` proposals become issues.

Each researcher is a separate script and can be enabled/disabled via
`--research-sources`.

### Per-researcher contracts

| Researcher | Reads | Cap |
|---|---|---|
| `spec-vs-code` | `docs/specs/**.md` + `git grep` for matching code | 100 specs scanned per round |
| `prior-reports` | `.autospec/explore-summary.md` + `.autospec/run-summary.md` (this and recent) | last 10 reports |
| `codebase-signals` | TODO/FIXME comments, `find-dead-code` output, low-coverage files | 200 signals per round |
| `open-issues` | `gh issue list --state open --not-label autospec:v2-flow` | 50 issues per round |
| `source-analysis` | repo README + AGENTS.md + `tree -L 3` + recent commits → LLM call | 1 LLM dispatch per round |
| `internet` | web search for competitors → fetch top results → LLM extracts features → propose | max 5 web fetches per round; allowlist enforced |

### Internet researcher safety

The internet researcher is the highest-risk source. Hardening:

1. **Domain allowlist**: `--internet-allowlist` enforced via path-allowlist
   pattern from PR #685. Hits to non-allowlisted domains exit 3 with
   `code_health:explore_forbidden_domain`.
2. **Content sanitization**: fetched content passes through the
   prompt-injection guard from PR #702 before feeding to the LLM
   summarizer. Hits exit 3 with `code_health:explore_injection_detected`.
3. **License/legal check**: proposals citing competitor product pages
   include the citation URL in the issue body. Operator review is the
   final gate before any code lands.
4. **Rate limit**: max 5 fetches per round, max 30 per session, configurable
   via env vars.

## Loop driver integration

The outer loop uses `scripts/lib/autospec-loop.sh` from PR #712 with
explore-specific callbacks:

- **per-iteration callback**: `scripts/explore-research-cycle.sh` (file
  issues for top N proposals).
- **drain callback**: invoke `/autospec-run` (which honors sandbox base
  branch).
- **termination conditions**: inherited from #712 + new
  `operator_stop` checks `~/.autospec/explore-stop.flag` AND
  `~/.autospec/stop.flag`. No convergence-stop (explore is meant to keep
  generating until operator says enough).

## Usage-limit recovery

Inherits `scripts/autospec-usage-limit.sh` (already wired for autospec-run
per existing skill prose). When the harness reports a deterministic
quota pause, the orchestrator arms the supervisor with the resume command
(the same `/autospec-explore` invocation + the sandbox branch context) and
exits. The supervisor relaunches after reset.

## Loop summary

`.autospec/explore-summary.md` (markdown, human-readable) +
`.autospec/explore-loop.json` (machine-readable per-iteration log).
Structurally identical to the loop summaries from `/autospec --loop`,
`/autospec-continue`, `/autospec-qa --heal` (all four share the shape from
PR #712).

Markdown shape:

```
## /autospec-explore — sandbox autospec/explore/<date>-<slug>

| Round | Researchers run | Proposals | Issues filed | PRs merged | Time | Status |
|---|---|---|---|---|---|---|
| 1 | 6/6 | 17 (deduped to 12) | 5 | 5 | 28m | round_complete |
| 2 | 6/6 | 14 (deduped to 9) | 4 | 4 | 22m | round_complete |
| 3 | 6/6 | 8 (deduped to 6) | 5 | 3 + 2 in flight | 31m | operator_stop |

Final status: operator_stop after 3 rounds, 14 PRs merged on sandbox.

To merge sandbox into main:
  git checkout main && git merge autospec/explore/2026-05-29-X

To discard:
  git branch -D autospec/explore/2026-05-29-X && \
    git push origin --delete autospec/explore/2026-05-29-X
```

## Error handling

- **Researcher fails** (e.g., gh API error, LLM timeout) → that researcher
  contributes 0 proposals, loop continues with the others. Logged.
- **All researchers fail** → round produces no proposals → loop emits
  `code_health:explore_all_researchers_failed` and pauses for operator.
- **Issue-creation fails** → retry once, then skip that proposal.
- **/autospec-run fails** → record failure, continue loop with reduced
  rate (next round delayed by 5 min) to avoid hammering on a broken
  state.
- **Sandbox branch deleted out from under the loop** → orchestrator
  detects via `git rev-parse --verify` before each iteration. Missing →
  exit with `code_health:explore_sandbox_missing` and operator-recovery
  instructions.

## Testing

- `tests/explore/test_explore_sandbox.bats` — sandbox creation/management,
  no accidental main writes.
- `tests/explore/test_explore_researchers.bats` — each of the 6
  researchers produces well-formed JSON proposals from fixture inputs.
- `tests/explore/test_explore_research_cycle.bats` — aggregation,
  dedup, ranking, capping.
- `tests/explore/test_explore_loop.bats` — outer loop integration with
  shared driver; termination conditions reachable.
- `tests/explore/test_explore_internet_safety.bats` — domain allowlist,
  prompt-injection guard, rate limit, content sanitization.

## Acceptance

- New skill family `autospec-explore` ships per the scaffold; passes
  `check_lockstep`.
- 6 researcher scripts ship under `scripts/explore-research/`.
- Sandbox branch created at run start; ALL PRs from the loop target the
  sandbox, never `main` (enforced).
- Loop driver from PR #712 reused; termination identical to the other
  three loop-enabled skills.
- Usage-limit supervisor arms correctly on quota pause.
- `autospec validate` gains `check_autospec_explore_contract()`
  enforcing trio lockstep + 6 researchers present + sandbox isolation
  documented + bats suite.
- All bats fixtures pass.

## Decomposition into child issues

Aiming for 5 children plus an umbrella.

1. **Issue A — skill scaffold + sandbox branch management**: trio +
   install/uninstall + README with all structural sections, plus
   `scripts/explore-sandbox.sh` and `.autospec/explore-mode.json` schema.
   Files: 7.
2. **Issue B — implementer PR-base integration**: extend
   `skills/autospec-run/prompts/phase4-implementer.md` to honor
   `.autospec/explore-mode.json` and target the sandbox branch as PR
   base. Lockstep update to `skills/autospec-run/` trio. Depends on A.
   Files: 4.
3. **Issue C — research cycle + 4 deterministic researchers**:
   `scripts/explore-research-cycle.sh` (aggregator) + 4 researcher
   scripts (spec-vs-code, prior-reports, codebase-signals, open-issues).
   Bats for each. Depends on A. Files: 3.
4. **Issue D — 2 LLM-heavy researchers + internet safety**: `source-
   analysis.sh` + `internet.sh` + domain allowlist + prompt-injection
   guard + rate limit. Depends on C. Files: 3.
5. **Issue E — orchestrator + loop integration + validate gate + e2e**:
   `scripts/autospec-explore.sh` ties everything to
   `scripts/lib/autospec-loop.sh` (PR #712). `check_autospec_explore_contract()`
   in `autospec validate`. End-to-end bats. Depends on A+B+C+D.
   Files: 3.

Total: 5 children + 1 umbrella.

## Out of scope (defer to v2)

- `/autospec-explore-promote <sandbox>` — automated sandbox-to-main merge
  with operator confirmation (manual git merge for v1).
- Auto-rebase sandbox onto main when main advances.
- Cross-repo exploration.
- Multi-arm bandit ranking of researcher weights based on which
  proposals get merged vs discarded.
- Researcher proposals with multi-step plans (only single-PR proposals
  for v1).
- Cost-aware proposal generation (LLM estimates token cost of each
  proposal before filing).
