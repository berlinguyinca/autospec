# autospec-run

Implementation half of the **autospec** suite. Picks up where `/autospec-define`
stops: takes the populated `auto-implement` queue on the current GitHub repo and
runs Phases 4–6 of autospec — autonomous monitor + admin-squash-merge of each PR
+ periodic status updates + final report.

Works on **Claude Code**, **OpenCode**, and **Codex CLI**.

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-run/install.sh | sh -s -- --harness all
```

Per-harness:

```bash
# Claude Code only
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-run/install.sh | sh -s -- --harness claude
# OpenCode only
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-run/install.sh | sh -s -- --harness opencode
# Codex CLI only
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-run/install.sh | sh -s -- --harness codex
```

From a clone:

```bash
cd skills/autospec-run
./install.sh --harness all
```

## Invocation

```
/autospec-run [--profile <name>] [--worker-id <id>] [--coordination-status] [--max-parallel-safe]
```

- `--profile <name>` — filter the candidate queue against
  `~/.autospec/model-profiles.yml` so only issues whose `ctx:*` and `reasoning:*`
  labels fit the named profile are picked up. Issues that exceed the profile on
  either axis are appended to a `Deferred` list and reported at run-end.
- (no flag) — run with the file's `default:` profile if present; otherwise prints
  `Profile filter inactive — running all auto-implement issues.` and continues
  without filtering.
- `--worker-id <id>` — override the distributed worker identity written to the
  GitHub run-state comment for a claimed issue.
- `--coordination-status` — print the distributed queue state, including ready,
  blocked, claimed, conflicting, and next-batch issues, then exit without
  claiming work.
- `--max-parallel-safe` — print only the next conflict-free batch from the
  distributed queue planner, then exit without claiming work.
- `--claim <issue>` — attempt to claim one issue and exit with status `0` when
  claimed or `2` when the issue is already claimed or not ready.
- `--release <issue>` — release a claim during failure, stop, or manual recovery.

The full filter logic, deferred-summary accumulation, and `model-profiles.yml`
auto-init arrive in PR B2 (issue #15). This skill currently plumbs the flag with
no filter behaviour yet — runs all auto-implement issues regardless of profile.

## Distributed coordination

Multiple workstations can run `autospec-run` against the same GitHub repository.
The shared queue is GitHub Issues:

- `auto-implement` is the ready queue.
- `in-progress-by-bot` is the active claim label.
- A single marked `autospec-run-state` issue comment records worker identity,
  step, branch, PR, owned paths, and TTL.

Before selecting work, the monitor uses `list-ready-issues.sh` to exclude issues
with open dependencies or path conflicts. It then uses `claim-issue.sh` to swap
labels, write run-state, and verify that the same worker still owns the state
comment. Lost claims are treated as normal races, not failures. The watchdog
reclaims stale GitHub run-state in addition to same-host heartbeat files.

## What it does

- **Phase 4** — Background autonomous monitor: outer loop picks the next ready
  `auto-implement` issue (deps closed), labels it `in-progress-by-bot`, dispatches
  a foreground subagent to implement it (worktree → TDD → push → PR → self-review
  → admin-squash-merge), then loops without sleeping so a freshly-merged PR can
  unblock the next issue immediately.
- **Phase 5** — Periodic status updates posted to the user every ~25 min (slows
  to ~50 min when quiet).
- **Phase 6** — Final report on monitor termination: every issue processed, every
  PR merged, total elapsed wall time, any failures needing human attention.

## Profile filter (forward reference)

When PR B2 (issue #15) lands, the candidate queue is filtered by profile:

```yaml
# ~/.autospec/model-profiles.yml
default: claude-sonnet-cloud
profiles:
  qwen3-32b-laptop:
    ctx: 64k       # 32k < 64k < 120k
    reasoning: medium  # shallow < medium < deep
  claude-sonnet-cloud:
    ctx: 120k
    reasoning: deep
```

The filter excludes issues whose `ctx:*` / `reasoning:*` labels exceed the
profile ceilings; excluded issues are appended to a `Deferred` list and reported
at run-end.

## Subagent model selection (Tier B only)

This skill is the implementation half — every subagent it dispatches (Phase 4 `process(ISSUE)` implementer + inner-loop LGTM self-review) runs on **Tier B** per `AGENTS.md`: cheaper model with medium thinking. Implementation follows a well-specified contract from the planning half; the loop runs many times per spec, so cheaper-tier amortizes well.

The Tier A (top + extended thinking) tier is **not used** by this skill; it lives in `autospec-define` / `autospec-classify` where spec quality is being authored.

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec`](../autospec/README.md) | The full pipeline (Phases 0–6) when you want one skill that plans **and** ships. |
| [`autospec-define`](../autospec-define/README.md) | Planning half (Phases 0–3). Produces the `auto-implement` queue this skill consumes. |
| [`autospec-classify`](../autospec-classify/README.md) | Standalone retro-labeler that backfills `ctx:*` / `reasoning:*` labels onto already-existing `auto-implement` issues so the `--profile` filter has data to work with. |

## Self-update

Once installed, run the skill with the literal argument `update` to refresh in
place. The skill detects its harness, re-runs the canonical install one-liner
with `--update`, shows the diff, and stops without entering Phase 4:

```
/autospec-run update
```

Or re-run the per-skill installer with `--update`:

```bash
./install.sh --harness all --update
```

## License

MIT — see the repository [LICENSE](../../LICENSE) file.
