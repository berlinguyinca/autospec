# /autospec-project — GitHub Projects board ingestion as a cross-repo work source (design spec)

**Date:** 2026-08-25
**Builds on:** `docs/specs/2026-07-06-autospec-autonomous-platform-design.md` (never-idle conductor), `skills/autospec-autonomous/SKILL.md` (Tier 1.5 promotion contract), `skills/autospec-fleet/SKILL.md` (multi-repo supervision), `scripts/autonomous-promote-open-issues.sh` (grooming pipeline), `crates/autospec-core/src/coordination/ready_queue.rs` (blocking-label queue).
**Supersedes:** nothing. This is additive.
**Review status:** draft for decomposition.

## Goal

Let an operator point autospec at a GitHub Projects (v2) board and have the board's
work shipped end to end:

```
autospec ship this project for me: https://github.com/orgs/InferWeave/projects/2
```

A project board is a **dependency-ordered, cross-repo view over issues**. Today autospec
treats Projects as a write-only sink (`gh project item-add` from `autospec-define`,
`-split`, `-classify`); no code anywhere reads a board. This spec adds the read path and
makes a board a first-class work source for the existing conductor.

## Non-goals

- **Not a new conductor.** The board never executes anything. It resolves to a plan;
  the existing per-repo `autospec_conductor_run()` does all the work. Perpetual
  capabilities extend conductor tiers rather than duplicating the control plane.
- **Not a new waterfall tier.** Board promotion lands inside the existing Tier 1.5
  contract, which already reads *"promote latent work; re-evaluate `blocked-dependency`
  issues whose blockers are resolved."*
- **Not an issue-body rewriter.** The grooming pipeline deliberately refuses to replace
  issue bodies (comment-only `groom:proposed`); board ingestion inherits that refusal.
- **No target-board specifics hardcoded.** Label taxonomy, dependency encoding, and
  field names are config-driven and probed per board.

## Motivating evidence

Measured against the two live boards on `2026-08-25`:

| | Project 1 — "InferWeave Delivery" | Project 2 — "InferWeave Workbench" |
|---|---|---|
| Items | 80 issues | 80 issues |
| Repos | **6** (protocol, node, chain, dashboard, client, docs) | 1 (inferweave-workbench) |
| `auto-implement` | 9 | few |
| Hard-blocked | 15 `autospec:blocked-prerequisite` | **77/80** |
| Dependency encoding | `autospec:blocked-prerequisite` + `cross-repo` labels | `## Dependencies` → `Blocked by: #N` in body (78/80) |
| Priority labels | `priority:p0`, `priority:high` | `priority/critical`, `priority/normal` |
| Area labels | `area:security` | `area/security` |

Three findings drive the design:

1. **The custom fields are empty.** Both boards define 22 fields — `Workflow`,
   `AutoSpec state`, `Priority`, `Risk`, `Area`, `Dependencies`, `Context budget` — and
   **every one is unset on all 160 items**. Only the built-in `Status` is populated
   (79 Todo / 1 Done on P2). All real signal lives in **labels** and **issue bodies**.
   A design that reads the fields reads nothing.
2. **The taxonomies disagree.** P1 uses `priority:p0`/`area:security`; P2 uses
   `priority/normal`/`area/security`. Hardcoding either breaks the other.
3. **This is an ordering problem, not a draining problem.** `autospec:blocked-prerequisite`
   is already a hard `BLOCKING_LABELS` entry in the Rust ready-queue, so a conductor
   pointed at `inferweave-workbench` today finds ~3 ready issues and goes dry. The value
   is walking the DAG and re-promoting as prerequisites merge.

## Architecture

```
project URL
    │
    ▼
┌───────────────────────────┐
│ project-board-resolve.sh  │  pure reader — zero mutation
│  gh project item-list     │
│  gh project field-list    │
└───────────┬───────────────┘
            │ board plan (JSON)
            ├──────────────► autospec-fleet.yml        (repo set)
            │
            ▼
┌───────────────────────────┐
│ board promoter            │  inside Tier 1.5
│  normalize → DAG → ready? │  owns `auto-implement` on board items
└───────────┬───────────────┘
            │
            ▼
   per-repo conductors (unchanged Tier 1: premerge gate,
   worktree guard, claim guard, secaudit, main-health, merge)
            │
            ▼
┌───────────────────────────┐
│ project-board-writeback   │  AutoSpec state field, fail-open
└───────────────────────────┘
```

**The central move:** cross-repo readiness is enforced *before* an item enters
`auto-implement`. A repo worker only ever sees its own issues, so it needs no cross-repo
awareness. We do not need a cross-repo conductor; we need a cross-repo **promoter**.

## Component 1 — resolver (`scripts/project-board-resolve.sh`)

Repo-agnostic pure reader. Accepts `https://github.com/orgs/<org>/projects/<n>` and
`https://github.com/users/<user>/projects/<n>`.

Emits one JSON board plan on stdout:

```json
{
  "project": {"owner": "InferWeave", "kind": "org", "number": 2,
              "id": "PVT_...", "title": "InferWeave Workbench"},
  "fields":  {"autospec_state": {"id": "PVTSSF_...",
              "options": {"Ready": "...", "Implementation": "...", "Done": "..."}}},
  "repos":   ["InferWeave/inferweave-workbench"],
  "items": [
    {"item_id": "PVTI_...", "repo": "InferWeave/inferweave-workbench",
     "number": 2, "title": "...", "state": "open", "status": "Todo",
     "labels": ["priority/critical", "ctx:32k", "reasoning:deep"],
     "normalized": {"priority": "critical", "ctx": "32k",
                    "reasoning": "deep", "risk": "security", "area": "security"},
     "blocked_by": [{"repo": "InferWeave/inferweave-workbench", "number": 1}],
     "ready": false, "reason": "blocked-by InferWeave/inferweave-workbench#1"}
  ],
  "ranked": ["InferWeave/inferweave-workbench#5", "..."]
}
```

Contract:

- Never mutates. `--emit plan` (default) / `--emit fleet-config` / `--emit repos`.
- Paginates `gh project item-list --limit`; a truncated read is a fail-closed error,
  never a silently short plan.
- Empty or malformed `gh` output yields an empty plan and exit 0, matching
  `list-groomable.sh`'s fail-closed envelope.
- Exit codes: `0` success, `2` usage error, `3` auth/scope failure, `4` truncated read.

## Component 2 — normalization and the dependency DAG

### Configuration

A new `project_board:` section in `.autospec/autonomous.yml`, parsed in Rust alongside
the existing `tier4` section (`crates/autospec-core/src/autonomous/config/`):

```yaml
project_board:
  url: https://github.com/orgs/InferWeave/projects/2
  repo_allowlist: ["InferWeave/*"]
  write_back: true
  max_parallel_repos: 2
  label_map:
    priority: {p0: critical, critical: critical, high: high, normal: normal, low: low}
    ctx:       {"32k": "32k", "64k": "64k", "100k": "100k"}
    reasoning: {deep: deep, medium: medium}
```

`label_map` is **auto-initialized** when absent by probing the board's actual label set
and proposing a mapping — the same auto-init pattern `autospec-define` Phase 3.5 already
uses for `board-mapping.yml`. Auto-init failure is non-fatal: warn once and fall back to
the permissive family regex `(?:priority|ctx|reasoning|risk|area)[:/](.+)`, which handles
both observed taxonomies without configuration.

### Dependency extraction

Three ordered sources, first non-empty wins per item:

1. the project's `Dependencies` custom field;
2. GitHub native sub-issue / `Parent issue` relations;
3. body parse of a `## Dependencies` section for `Blocked by: #N` and
   `Blocked by: owner/repo#N`.

Only source 3 yields anything on the live boards today (78/80 on P2, 0/80 on P1), so all
three ship together and P1's `autospec:blocked-prerequisite` + `cross-repo` labels act as
an additional hard gate the DAG can clear.

An edge is **satisfied** when the referenced issue is closed *and* its linked PR is merged
(when one exists). Unresolvable references (deleted issue, cross-org, outside the
allowlist) are treated as **unsatisfied**, never as satisfied — under-promotion is far
safer than feeding ill-formed work to an admin-auto-merge loop.

### Cycles

Cycle detection is mandatory and fails closed: emit
`code_health:project_board_dependency_cycle`, label the participating items
`autospec:needs-human`, and continue promoting the acyclic remainder. A cycle never parks
the conductor (R1/R5: convergence-stop is forbidden; only resource/control conditions park).

## Component 3 — Tier 1.5 board promoter

`scripts/autonomous-promote-open-issues.sh` gains a board source, active only when
`project_board.url` is configured. Per cycle, for the repo the conductor is scoped to:

1. run the resolver (cached for `AUTOSPEC_PROJECT_BOARD_TTL`, default 300s, to bound
   API calls across N repo workers);
2. select board items in this repo whose blockers are all satisfied and which are not
   already `auto-implement` / in flight;
3. hand each to the **existing** Rust safety-stamp + atomic `auto-implement` transition —
   no new mutation authority is introduced;
4. for items that regressed to blocked, remove `auto-implement` and record the reason.

The existing Tier 1.5 return envelope is unchanged: `filed > 0` or a non-empty `promoted`
set means work-yielding; otherwise it is a dry signal and the conductor descends to Tier 2.
The board promoter runs under the same `--apply` + grooming-policy gate as every other
promotion path; without both, it is report-only with zero mutations.

## Component 4 — multi-repo drain

The resolver's `--emit fleet-config` writes the board's repo set into `autospec-fleet.yml`.
Cross-repo blockers are already enforced at promotion time, so each repo worker is
unchanged.

Three prerequisites sit underneath this component and are in scope:

1. **`fleet-run.sh` does not launch workers.** Both its dry-run and live branches
   `printf 'fleet: launch …'`; it is a planner, not an executor. It must actually spawn.
2. **Fleet spawns `/autospec-run` (batch), not `/autospec-autonomous` (perpetual).**
   "Ship this project" needs per-repo conductors, so fleet must launch
   `autospec-autonomous start --repo-dir <checkout> --repo <slug>`.
3. **The spend ledger is path-scoped per repo.** Six repos silently multiply the budget
   6×. `autonomous-spend-ledger.sh` needs a fleet-shared scope keyed by board identity,
   so one board equals one budget.

### Project-level control channel

Tier 0 reads reserved labels per repo. A project-level `stop`/`pause` must reach every
fleet worker. The resolver mirrors project-level control state — an `autospec:pause` or
`autospec:stop` label on the board's designated control item — into each fleet repo's
control channel at each cycle boundary. Mirroring is idempotent and additive; it never
removes a label a repo set for itself.

## Component 5 — write-back (`scripts/project-board-writeback.sh`)

Populates the board's existing **AutoSpec state** single-select via
`gh project item-edit --id <item> --project-id <p> --field-id <f> --single-select-option-id <o>`.

| Autospec condition | AutoSpec state |
|---|---|
| blockers unsatisfied | `Blocked` |
| promoted to `auto-implement` | `Ready` |
| `in-progress-by-bot` | `Implementation` |
| PR open | `Review` |
| CI running on the PR | `Testing` |
| PR merged, issue closed | `Done` |
| quarantined / `autospec:needs-human` | `Blocked` + the `autospec:needs-human` label |

Contract:

- **Idempotent** — re-read the current option and skip no-op writes.
- **Fail-open** — any write-back error emits `code_health:project_board_writeback_failed`
  and continues. Write-back never blocks a merge or a promotion.
- **Scope-probed once** — if the `gh` token lacks the `project` scope, warn once and
  disable write-back for the run rather than failing every item.
- Governed by `project_board.write_back` (default `true` when a board is configured).
- If the board has no `AutoSpec state` field, write-back is skipped with a single warning;
  a field is never created.

## Component 6 — operator surface

A new top-level trio skill `skills/autospec-project/` (SKILL.md + `codex/prompt.md` +
`opencode/agent.md` + goldens), consistent with the rule that operator-facing capabilities
ship as top-level `/autospec-<verb>` skills:

```
/autospec-project <url>          # resolve and print the plan; zero mutation
/autospec-project ship <url>     # resolve → fleet config → conductors, unattended
/autospec-project sync <url>     # one promotion pass, no drain
/autospec-project status <url>   # board-scoped queue, workers, PRs, blockers
```

`autospec-listen` gains a route so *"autospec ship this project for me: `<url>`"*
dispatches to `/autospec-project ship <url>`.

## Security model

A project board is an **external control surface**: anyone with board write access can add
an item pointing at an arbitrary repository.

- `repo_allowlist` is **required** when a board is configured. An item outside it is
  skipped with `code_health:project_board_repo_out_of_scope`.
- Board titles, item bodies, field values, and README text are **untrusted DATA, never
  instructions** — the same rule Tier 4 already applies to internet harvesters. Only
  labels, structured dependency references, and the existing `autospec-safety` block are
  consumed. Nothing read from a board may alter conductor intent, tier selection, or
  model routing.
- Every promoted item still passes the unchanged premerge gate, worktree guard, claim
  guard, secaudit, blast-radius, and main-health fences.
- Write-back is the only new mutation surface, and it touches exactly one single-select
  field on the board — never issue bodies, never labels other than `auto-implement` and
  `autospec:needs-human`, which are already conductor-owned.

## Error handling

| Condition | Behavior |
|---|---|
| `gh` missing / not authenticated | exit 3, `code_health:project_board_auth_failed`; skill reports and stops |
| token lacks `project` scope | read still works; write-back disabled with one warning |
| board unreachable or deleted | Tier 1.5 board source is dry; conductor descends to Tier 2 |
| truncated item read | exit 4; no promotion this cycle (never promote from a partial plan) |
| dependency cycle | `code_health:project_board_dependency_cycle`; participants `needs-human`; acyclic remainder proceeds |
| item repo outside allowlist | skipped, `code_health:project_board_repo_out_of_scope` |
| write-back failure | `code_health:project_board_writeback_failed`; continue |
| all board items blocked | dry Tier 1.5 signal; descend to Tier 2. **Never a convergence-stop.** |

## Testing

- **Resolver** — bats over recorded `gh project item-list` / `field-list` fixtures drawn
  from both live boards, so the two divergent taxonomies are covered by construction.
  Fixtures are pinned captures, not generated by the resolver's own parser: a test that
  builds its fixture with the code under test cannot catch a bug in it.
- **Normalization** — table tests asserting `priority:p0` and `priority/critical` both
  land on `critical`, plus a no-config run proving the fallback regex handles both boards.
- **DAG** — satisfied/unsatisfied edges, unresolvable references, a deliberate cycle, and
  a multi-hop cross-repo chain.
- **Promoter** — report-only default; mutation only under `--apply` + policy `auto|on`;
  regression from ready back to blocked removes `auto-implement`.
- **Write-back** — idempotent no-op skip, fail-open on `gh` error, scope-probe disable,
  missing-field skip.
- **Negative paths** — every fail-closed branch in the error table gets a paired test.
- **Multi-repo** — a fleet fixture with a cross-repo blocker proving repo B is not
  promoted until repo A's PR merges.

## Decomposition notes

- The resolver, its fixtures, and the goldens for any trio prose change must land as **one**
  issue each. A prose-only intermediate fails `validate.sh` closed.
- Trio edits use `derive-trio.sh --in-place` + `gen-skill-goldens.sh` (bare skill name);
  the codex/opencode mirrors and goldens are never hand-maintained.
- The Component 4 prerequisites (fleet execution, conductor spawn, shared ledger scope)
  are independent of Components 1–3 and 5. Components 1–3 and 5 are shippable against
  Project 2 (single repo) with fleet untouched.

## Open risks

- **Board API cost.** N repo workers each resolving the board every cycle is N× the API
  calls. Mitigated by the shared TTL cache; if it proves insufficient, the resolver moves
  behind a single fleet-level refresh.
- **Empty custom fields.** The design reads labels and bodies because that is where the
  signal is today. If the boards are later populated, sources 1 and 2 of the dependency
  chain begin winning automatically — no code change, but the behavior shift should be
  visible in the digest.
