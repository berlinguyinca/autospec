# Autospec Supervise Design

## Purpose

`autospec-supervise` is a low-token deterministic supervisor for
`autospec-autonomous`. It watches an existing autonomous conductor, detects when
the conductor is stuck, doing no useful work, leaving the repository's product
scope, or repeating a failed self-repair loop, and feeds those failures back into
autospec as ordinary implementation issues.

The supervisor is not a second conductor and does not implement fixes inline. It
observes, files repair issues, quarantines unsafe work, updates autospec after a
fix lands, waits for a conductor cycle boundary, restarts the monitored
`autospec-autonomous` process, and continues monitoring.

## User-Facing Skills

### `/autospec-supervise`

Thin skill wrapper around the operator command:

```bash
autospec-autonomous supervise --repo OWNER/REPO --repo-dir DIR
```

It starts or runs the deterministic supervisor. Normal operation uses shell,
`jq`, `gh`, log files, heartbeat files, and existing autospec status helpers.
There are no LLM calls in the polling path.

### `/autospec-scope-review`

Companion skill for reviewing scope quarantines:

```bash
/autospec-scope-review --repo OWNER/REPO
/autospec-scope-review --issue N --desired
/autospec-scope-review --issue N --reject
```

It lists quarantined issues, lets the operator mark them desired or rejected,
restores desired issues to `auto-implement`, and updates the repository scope
config from repo evidence plus the operator decision.

## Command Shape

```bash
autospec-autonomous supervise \
  [--repo OWNER/REPO] \
  [--repo-dir DIR] \
  [--interval-sec N] \
  [--once] \
  [--dry-run]
```

Defaults:

- `--interval-sec`: `300`
- max supervisor-filed issues per day: `3`
- ambiguous scope action: soft quarantine
- restart policy: wait for cycle boundary

## Deterministic Inputs

The supervisor reads only compact machine surfaces during normal polling:

- `autospec-autonomous status --json`
- `autospec-autonomous list --json`
- `autospec-autonomous timeline --lines N`
- `bash ~/.autospec/scripts/autospec-run-status.sh --repo OWNER/REPO --json`
- `~/.autospec/autonomous-operator/<repo-scope>/state.json`
- `~/.autospec/autonomous-operator/<repo-scope>/launch.json`
- `~/.autospec/logs/<repo-scope>/*.log`
- `~/.autospec/process-heartbeats/<repo-slug>/*.json`
- `gh issue list` for dedupe and quarantine review
- recent merged PRs and closed issues only when checking whether a
  supervisor-filed fix landed or when learning accepted scope

Normal output is one line:

```text
autospec-supervise: ok repo=OWNER/REPO conductor=running queue=ready:0 claimed:0 blocked:2 action=none
```

Action output is also one line:

```text
autospec-supervise: filed issue #123 class=stuck signature=<sha>
autospec-supervise: fix landed issue=#123 pr=#456 updated=true restarted=true
```

## Failure Classes

### Stuck Conductor

File or update a repair issue when all are true:

- conductor PID exists or recently existed;
- no completed issue, merged PR, closed issue, new claim, or useful heartbeat for
  `AUTOSPEC_SUPERVISE_STUCK_SECS`;
- current state is not `resource-park`, `operator-stop`, `pause`, or
  `idle-rescan`;
- log tail repeats the same phase or signature at least the configured repeat
  threshold.

Labels:

- `auto-implement`
- `safety:reviewed`
- `supervisor:filed`
- `autonomy:stuck`

### No Useful Work

File or update a repair issue when:

- the conductor has run for `AUTOSPEC_SUPERVISE_NO_WORK_SECS`;
- no PR merged and no implementation issue closed;
- cycles repeatedly through dry promotion or discovery without filing valid work;
- queue state is not intentionally empty due to operator stop, pause, or
  resource park.

Label: `autonomy:no-work`.

### Runaway Self-Repair

File or update a repair issue when any hard cap trips:

- too many supervisor issues filed in a day;
- repeated duplicate issues by normalized title/body signature;
- repeated restarts without progress;
- same tier fires repeatedly while lower tiers starve;
- spend ledger crosses the supervisor soft budget.

Label: `autonomy:runaway`.

### Guideline Violation

File or update a repair issue when the autonomous daemon violates repository
operating rules, even if it is otherwise making progress. The first required
check is branch/worktree hygiene:

- implementation work must happen on a dedicated issue branch such as
  `feat/<slug>`;
- implementation work must happen in a linked worktree or other isolated issue
  workspace, not in the primary checkout;
- the daemon must not commit directly on `main`;
- the daemon must not push directly to `main`;
- merging a completed PR into `main` is allowed only after the normal
  `autospec-run` validation and merge gate.

This check exists because a conductor can appear productive while violating the
repo's most important safety guideline. If autonomous mode works directly on
`main` instead of creating dedicated branches per issue, the supervisor must
treat that as a repairable daemon bug.

Deterministic evidence sources:

- `git -C <repo-dir> branch --show-current`
- `git -C <repo-dir> status --short --branch`
- `git -C <repo-dir> worktree list --porcelain`
- recent commits on `main` compared with PR merge metadata;
- `autospec-run-status --json` branch fields;
- conductor logs that name checkout, branch, worktree, or merge commands.

Actions:

- file or update a repair issue with label `autonomy:guideline-violation`;
- add `autospec:needs-human` to any affected in-progress issue if the violation
  risks direct-main mutation;
- request a graceful stop/restart only at a cycle boundary unless direct-main
  mutation is still active;
- include evidence showing expected branch/worktree behavior and observed
  behavior.

This class is separate from scope drift. Scope drift asks whether the work
belongs to the product. Guideline violation asks whether the daemon is following
the repository's operating rules while doing the work.

### Scope Drift

Scope drift is product-scope drift. If a repository is a snake game, autonomous
work may improve snake gameplay, controls, scoring, levels, accessibility,
performance, packaging, tests, and docs. It must not invent Super Mario, a CRM,
an ecommerce platform, a social network, or unrelated infrastructure.

Clear drift is auto-quarantined:

- add `scope:quarantined`;
- add `autospec:needs-human`;
- add `autonomy:scope-drift`;
- remove `auto-implement`;
- comment with scope evidence, mismatch reason, and review command.

Ambiguous drift is soft-quarantined:

- add `scope:needs-review`;
- add `autospec:needs-human`;
- remove `auto-implement`;
- leave the issue open;
- do not file a repair issue unless the same ambiguity repeats.

Desired quarantined work can only be restored by explicit operator action via
`/autospec-scope-review --issue N --desired`.

### Regression After Repair

File or update a repair issue when:

- the supervisor previously filed a fix issue;
- the fix landed and autospec was updated;
- the conductor restarted at a cycle boundary;
- the same failure signature reappears within
  `AUTOSPEC_SUPERVISE_REGRESSION_WINDOW`.

Label: `autonomy:regression`.

## Scope Model

The shared scope config lives at:

```text
.autospec/autonomous-scope.yml
```

Initial shape:

```yaml
schema: autospec.autonomous_scope.v1
product_intent:
  summary: "A browser snake game"
  evidence:
    - README.md
    - docs/specs/2026-07-01-snake-game.md
allowed_domains:
  - snake gameplay
  - grid movement
  - food spawning
  - scoring
  - collision detection
  - keyboard controls
  - touch controls
  - accessibility
  - performance
  - packaging
  - tests
  - docs
learned_domains: []
forbidden_domains:
  - unrelated platformer games
  - ecommerce
  - crm
  - social network
  - unrelated SaaS
quarantine:
  ambiguous: soft
  clear_drift: quarantine
```

If the file is absent, `scope-classify-issue.sh` builds a compact fingerprint
from deterministic repo evidence:

- README title, headings, and first product paragraph;
- `docs/specs/**` titles and headings;
- package or project metadata;
- recent merged PR titles;
- recent closed issue titles;
- existing issue labels.

No LLM is used in the normal classifier path.

## Scope Learning

When the operator marks a quarantined issue as desired,
`autospec-scope-review` automatically updates `.autospec/autonomous-scope.yml`.

The update is narrow:

- add a `learned_domains` row for the accepted concept;
- cite the operator decision and supporting repo evidence;
- do not rewrite `product_intent`;
- do not remove forbidden domains automatically.

Example:

```yaml
learned_domains:
  - name: "daily challenge mode"
    source: "operator-desired issue #123"
    evidence:
      - "README.md mentions challenge modes"
      - "merged PR #98 added level selection"
    added_at: "2026-07-11"
```

If there is no supporting repo evidence beyond the operator's explicit desired
decision, the concept may still be learned, but it must be marked:

```yaml
evidence_strength: operator-only
```

## Repair Issue Contract

Every supervisor-filed issue uses a stable dedupe signature:

```text
<repo>|<failure_class>|<normalized_phase>|<normalized_error>|<top_log_anchor>
```

The SHA-256 digest appears in the issue body:

```text
Supervisor signature: <sha256>
```

Before filing, the supervisor searches open and recently closed issues for the
same signature. If found, it comments with fresh evidence instead of filing a
duplicate.

Issue body template:

```markdown
## Goal

Fix `<specific autonomous behavior>` so `autospec-autonomous` makes forward progress without supervisor intervention.

## Evidence

- Repo: `<OWNER/REPO>`
- Conductor session: `<session id or pid>`
- Failure class: `<stuck|no-work|runaway|guideline-violation|scope-drift|regression>`
- Guideline violated: `<branch-hygiene|primary-checkout|direct-main-push|other>`
- Supervisor signature: `<sha256>`
- First observed: `<timestamp>`
- Last observed: `<timestamp>`
- Status JSON excerpt: `<path or compact json>`
- Log anchors:
  - `<timestamp> <phase> <line>`

## Acceptance criteria

- [ ] `tests/autonomous/<specific-test>.bats` reproduces this failure signature.
- [ ] `scripts/<affected-helper>.sh` or `skills/autospec-autonomous/**` handles the failure deterministically.
- [ ] `bash scripts/validate.sh --changed` passes.
- [ ] Running `autospec-autonomous status --json` after restart shows progress or intentional park.
- [ ] The supervisor signature `<sha256>` does not reappear after one monitor interval.

### Primary smoke test (inner loop)

```bash
bats tests/autonomous/<specific-test>.bats
```

## Safety review

<!-- autospec-safety:begin -->
- **decision:** `SAFETY_PASS`
<!-- autospec-safety:end -->

## Implementation outline

- Add or update deterministic detection/recovery in the affected helper.
- Add a regression test using fixture logs/state.
- Avoid new LLM calls in polling paths.
- Avoid creating a second conductor.
- Preserve branch/worktree hygiene: per-issue work on a dedicated branch and
  isolated worktree; no direct `main` commits or pushes.
```

## Update And Restart Policy

When a supervisor-filed fix lands:

1. Confirm the linked PR merged or the issue closed with a merged commit
   reference.
2. Confirm `main` contains the expected changed files.
3. Run autospec update:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
4. Wait for a cycle boundary:
   - `autospec-run-status --json` reports `claimed=0`;
   - no current implementation issue heartbeat is active;
   - conductor is not mid-drain.
5. Restart:
   ```bash
   autospec-autonomous restart --repo OWNER/REPO --repo-dir DIR
   ```
6. Append evidence to:
   ```text
   ~/.autospec/supervisor/<repo-scope>/events.jsonl
   ```
7. Continue monitoring.

The supervisor must not interrupt an active implementation issue.

## State

Supervisor state is repo-scoped:

```text
~/.autospec/supervisor/<repo-scope>/
  state.json
  events.jsonl
  signatures.json
  scope-cache.json
```

`state.json` stores:

- active repo and repo-dir;
- monitored conductor scope;
- last useful progress timestamp;
- last observed queue summary;
- recently filed signatures;
- pending fix issue/PR references;
- last update/restart result.

## Implementation Components

### Skill wrappers

- `skills/autospec-supervise/SKILL.md`
- `skills/autospec-supervise/codex/prompt.md`
- `skills/autospec-supervise/opencode/agent.md`
- `skills/autospec-scope-review/SKILL.md`
- `skills/autospec-scope-review/codex/prompt.md`
- `skills/autospec-scope-review/opencode/agent.md`

The wrappers stay thin and dispatch to scripts.

### Scripts

- `scripts/autospec-autonomous.sh`
  - add `supervise` subcommand
- `skills/autospec-shared/scripts/autonomous-supervise.sh`
  - deterministic monitor loop
- `skills/autospec-shared/scripts/scope-classify-issue.sh`
  - deterministic classifier
- `skills/autospec-shared/scripts/scope-review.sh`
  - companion review/restore/reject workflow
- `skills/autospec-shared/scripts/autonomous-guideline-check.sh`
  - deterministic branch/worktree/main-safety checks

### Tests

- `tests/autonomous/test_supervise_stuck.bats`
- `tests/autonomous/test_supervise_no_work.bats`
- `tests/autonomous/test_supervise_scope_drift.bats`
- `tests/autonomous/test_supervise_guideline_violation.bats`
- `tests/autonomous/test_supervise_restart_boundary.bats`
- `tests/autonomous/test_scope_review.bats`

Validation must be wired into `scripts/validate.sh --changed`.

## Acceptance Criteria

- `autospec-autonomous supervise --once --dry-run` prints a valid one-line
  status and writes no GitHub state.
- A fixture with repeated no-progress logs files exactly one repair issue with a
  stable supervisor signature.
- A repeated signature comments on the existing issue instead of filing a
  duplicate.
- Clear out-of-scope autonomous issues are quarantined and removed from
  `auto-implement`.
- Ambiguous scope issues are soft-quarantined as `scope:needs-review`.
- Direct implementation work on `main` files exactly one
  `autonomy:guideline-violation` repair issue with branch/worktree evidence.
- Normal PR merge-to-main after validation is not flagged as a guideline
  violation.
- `/autospec-scope-review --issue N --desired` restores the issue to
  `auto-implement` and updates `.autospec/autonomous-scope.yml`.
- A landed supervisor-filed fix triggers autospec update and waits for
  `claimed=0` before restart.
- Normal monitor polling uses no LLM calls.
- `bash scripts/validate.sh --changed` passes.

## Approved Prompt

```text
Create autospec-supervise as a low-token deterministic supervisor for autospec-autonomous.

It must observe the existing autonomous conductor through machine-readable status, logs, heartbeats, GitHub issue/PR state, and deterministic branch/worktree checks. It must not become a second conductor and must not implement fixes inline. On proven stuck/no-work/runaway/guideline-violation/scope-drift behavior, it files lint-clean autospec repair issues with stable dedupe signatures. For scope drift it auto-quarantines clear drift and soft-quarantines ambiguous proposals by removing auto-implement and adding review labels. For guideline violations it verifies that implementation work happens on dedicated issue branches and isolated worktrees, never by direct commits or pushes to `main`; normal validated PR merges into `main` are allowed. A companion autospec-scope-review command lets the operator mark quarantined work desired or rejected; desired work is restored and the repo scope config is updated from repo documentation, commit/PR/issue history, and the operator decision.

Normal monitoring must use shell/JQ/gh only, with no LLM calls. After a supervisor-filed fix lands, update installed autospec, wait for the conductor cycle boundary, restart autospec-autonomous, and continue monitoring.
```
