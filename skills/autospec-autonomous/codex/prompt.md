
# autospec-autonomous workflow (harness-neutral)

Run the autospec machinery **unattended for weeks**. `/autospec-autonomous` starts a
perpetual conductor loop that walks a fixed priority waterfall, picks the highest-priority
available work, ships it to `main` via the existing `autospec-run` merge pipeline, writes a
daily digest, obeys a GitHub control channel for live steering, routes work to the cheapest
capable model tier, and **parks itself before exhausting usage quota**, resuming automatically
when quota resets.

This skill is a **conductor**, not a new engine. It reuses without reimplementing:
`autospec-run` (the merge pipeline), `autospec-autonomy-gate.sh`, `autospec-usage-limit.sh`,
`worktree-guard.sh`, `autospec-loop.sh` (the shared loop driver), and `/autospec-resume`.

**Never-idle scope:** Tier 0 (control channel), Tier 1 (backlog → `main`), Tier 1.5
(open-issue promotion), Tier 2 (local discovery), Tier 3 (architecture/test-coverage
improvement), and Tier 4 (internet/operator-polish discovery) are active by default.
The conductor parks only when every tier is dry, a stop/pause control signal is set,
or the usage/spend governor trips.

Manage your own context — never exceed 60%. Delegate to subagents whenever your
harness supports it; do not run the waterfall or issue drain directly in the main
conversation when a subagent can do it.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-autonomous -->

## Self-update mode

If the feature-request argument matches the regex `^\s*update\s*$` (case-insensitive, whitespace-padded), this skill enters self-update mode and does not run the normal pipeline:

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-autonomous/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-autonomous.md`
   - Codex CLI:   `~/.codex/prompts/autospec-autonomous.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy.
4. **Stop.** Do not enter the autonomous pipeline. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-autonomous found; run install.sh first.` and exit.

## Stop mode

If the feature-request argument matches the regex `^\s*stop(\s+--\w+)*\s*$` (case-insensitive), this skill enters stop mode and does not run the normal pipeline:

1. Delegate to the shared stop helper:
   ```bash
   autospec-autonomous stop "$@"
   ```
2. Honor `--graceful` (write `~/.autospec/stop.flag` and `~/.autospec/autonomous-stop.flag`; running iteration finishes) and `--immediate` (also write `~/.autospec/refine-loop-stop.flag`; abort at next iteration boundary).
3. Print the stop summary and exit. Do not enter the autonomous pipeline.

## Operator commands and monitoring

The installer creates command wrappers in `~/.autospec/bin` and configures shells
to source `~/.autospec/env`, so these commands should resolve after a fresh shell
or after `. "$HOME/.autospec/env"`:

| Command | Purpose |
|---------|---------|
| `autospec-autonomous [start]` | Start the detached conductor. |
| `autospec-autonomous-status` | Print PID, log path, conductor state, spend ledger, and recent log tail. |
| `autospec-autonomous-timeline` | Print a chronological plain-English activity report plus queue forecast from the conductor log. |
| `autospec-autonomous-logs` | Print the current conductor log tail. |
| `autospec-autonomous-watch` | Follow the current conductor log. |
| `autospec-autonomous-stop --graceful` | Request stop after the current issue/cycle boundary. |
| `autospec-autonomous-stop --immediate` | Request immediate stop at the next major boundary. |
| `autospec-autonomous-restart` | Stop through the sentinel path, then launch a new conductor. |

All wrappers delegate to `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-autonomous.sh`.
The launcher records PID and log metadata under
`~/.autospec/autonomous-operator/`, keeps logs under `~/.autospec/logs/`, and
uses `autospec-autonomous-run-drain.sh` as the safe Tier-1 drain command. The
drain command invokes:

```bash
omx exec --cd "$AUTOSPEC_REPO_DIR" --dangerously-bypass-approvals-and-sandbox '$autospec-run'
```

Use `autospec-autonomous status --json` for monitoring integrations. Use
`autospec-autonomous timeline --lines N` when an operator needs a human-readable
sequence such as `4:30 am - implemented feature X` and `4:45 am - started
research on X`, followed by estimated remaining work, rough ETA, and planned next
steps when coordinator state is present in the log. Use `--repo-dir DIR` when launching from outside the target
checkout, and use `--repo OWNER/REPO` when the GitHub slug cannot be detected.

## Required capabilities & harness adapter

This workflow assumes six capabilities. Map each one to your harness's actual tool. If a capability is missing, use the listed fallback.

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Read-only codebase research | `Agent` (subagent_type=Explore)      | `task` agent in read-only mode           | `apply_patch` read-only / shell `grep`   | Do the search in-thread with `rg`/`grep`           |
| Foreground delegation       | `Agent` (subagent_type=general-purpose) | nested `task` agent, await output     | spawn nested CLI session                 | Do the work in-thread (more context cost)          |
| Background delegation       | `Agent` with `run_in_background: true` | detached `task` agent                  | nohup'd CLI session writing to a logfile | Run the monitor in a separate terminal/tmux pane   |
| Ask the user a question     | `AskUserQuestion`                    | inline prompt                            | inline prompt                            | Ask in the response and wait for the next turn     |
| Self-paced future wakeup    | `ScheduleWakeup` inside a `/loop`    | a recurring `task` or local `cron`       | local `cron`/`launchd` calling the CLI   | The user runs a status-update prompt manually      |
| Subagent model tier         | Tier A: `opus` + `ultrathink`; Tier B: `sonnet` + medium thinking | Tier A: top `task` model + high reasoning; Tier B: smaller-tier `task` + medium reasoning | Tier A: top GPT + `reasoning_effort=high`; Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Honor the per-phase tier mapping in AGENTS.md; retry the same subagent UP on unavailability |
<!-- autospec-block:harness-adapter-core -->

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in the repo root — recognized by Claude Code, OpenCode, and Codex. Per AGENTS.md, subagent dispatches use the two-tier policy: Tier A for waterfall-level decisions and ranking; Tier B for individual issue drain and deterministic steps (inherited from `/autospec-run`).

## Harness detection (run once at skill start, before any waterfall cycle)

Detect your harness by checking available tools before any pipeline step runs:

1. **Claude Code** — the `Agent` tool with a `subagent_type` parameter is available.
   - `TIER_A` = `opus` + `ultrathink`  (model ID: claude-opus-4-7)
   - `TIER_B` = `sonnet`               (model ID: claude-sonnet-4-6)

2. **OpenCode** — a `task` tool with model/tier configuration is available (no `subagent_type`).
   - `TIER_A` = top-tier task model + high reasoning
   - `TIER_B` = smaller-tier task model + medium reasoning

3. **Codex CLI** — neither `Agent` nor a configurable `task` tool is available; `apply_patch` is the primary edit tool.
   - `TIER_A` = current top GPT model + `reasoning_effort=high`
   - `TIER_B` = `gpt-5.1-codex-spark` + `reasoning_effort=medium`

**Fallback rule:** If `TIER_B` is not available in your harness, silently retry the same subagent dispatch with `TIER_A`. Preserve parent context on retry. Never ask the user.

Hold `TIER_A` and `TIER_B` for the entire skill run. Every "Tier A" and "Tier B" reference below resolves to these harness-specific values.

## Phase-1 waterfall contract

**Implementation:** the never-idle loop body is `autospec_conductor_run()` defined in
the shared loop driver (`${AUTOSPEC_SCRIPTS_DIR}/lib/autospec-loop.sh`, issue #1378).
Each cycle the function calls `autonomous-control-channel.sh` (Tier-0 preempt),
then `autonomous-waterfall.sh` (tier selection), then Tier-specific work. Tier-1
drains run `autonomous-premerge-gate.sh` (must emit `merge-ok` before any drain)
before invoking `autospec-run`; promotion/discovery tiers file or classify work back
into `auto-implement` so the next cycle returns to Tier 1. The loop then calls
`autonomous-spend-ledger.sh` (park on cap), `autonomous-resilience.sh`
(state/heartbeat/lock/main-health), and finally the once-per-UTC-day digest stub.
On park, `_conductor_arm_resume()` writes resume context and arms a
ScheduleWakeup/cron wake via `autospec-usage-limit.sh`.

The conductor walks tiers in priority order each cycle:

### Tier 0 — control channel (always preempts)

At every **cycle boundary** (never mid-issue), read reserved GitHub labels via
`autonomous-control-channel.sh` (`${AUTOSPEC_SCRIPTS_DIR}/autonomous-control-channel.sh`):

| Label                  | Command                                                              |
|------------------------|----------------------------------------------------------------------|
| `autospec:stop`        | Write `~/.autospec/stop.flag`; finish the current issue; exit cleanly. |
| `autospec:pause`       | Park the loop; notify operator; wait for resume or `autospec:stop`. |
| `autospec:priority`    | Re-sort the Tier-1 backlog by the label body before the next drain. |
| `autospec:steer`       | Parse the label body as a directive; update the active waterfall intent; remove the label. |

Tier 0 always preempts every lower tier. A `stop` or `pause` signal received mid-drain is honored at the NEXT cycle boundary, not mid-issue (never kills an in-flight implementer).

### Tier 1 — backlog → `main`

Drain `auto-implement` issues from the repository backlog to `main` via `/autospec-run`.
This is the primary merge loop body.

**Single cycle:**
1. **Worktree assert** — `bash ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/worktree-guard.sh assert` MUST exit 0. Non-zero → emit `code_health` identifier, park, notify.
2. **Tier-0 poll** — read control channel before any issue is picked.
3. **Pick next issue** — select the highest-priority `auto-implement` issue not already in flight (respecting `autospec:priority` directives). If none → `dry_cycle++`; once the dry threshold is reached, cascade to Tier 1.5 instead of parking.
4. **Pre-merge gate** — run `autospec-autonomy-gate.sh` (Phase-1: `autospec-qa` only; `autospec-secaudit` is Phase-2 prerequisite and MUST NOT be referenced until built). If missing → emit `code_health:autonomous_gate_missing`; halt merges; notify.
5. **Drain** — invoke `/autospec-run` for the selected issue. Inherits all existing `autospec-run` guards: worktree isolation, claim-guard, admin-merge authority, lock-step validation.
6. **Main-health check** — after each merge, poll `gh api repos/{owner}/{repo}/commits/main/status`. Green → continue. Pending → wait one poll interval. Red → halt Tier-1 merges; file `autospec:needs-human`; notify.
7. **Spend ledger** — tally tokens/issues in `~/.autospec/autonomous-spend.json` (path-scoped). At `AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS` (or issue count) → **park and notify** (cumulative cost kill-switch).
8. **Usage-limit recovery** — inherits `autospec-usage-limit.sh`. When the harness reports a deterministic quota pause, arm the supervisor with the resume command and exit. Supervisor relaunches after reset.
9. **Daily digest** — once per UTC day, write `.autospec/autonomous-digest.md` and open/update a pinned issue with the summary.
10. **Loop** — return to step 2.

### Tier 1.5 — promote existing open issues

When Tier 1 is dry but other open issues exist, the conductor promotes latent work
into the pipeline before attempting discovery:

- decompose open epics or tracked specs into linked `auto-implement` children via
  the configured promotion command (`AUTOSPEC_PROMOTE_OPEN_ISSUES_CMD` seam);
- re-evaluate `blocked-dependency` issues and promote only those whose blockers
  are resolved;
- classify `needs-classify` / unlabeled ready issues so they can enter the
  `auto-implement` queue.

Promotion-sourced work still flows through Tier 1 and inherits autonomy, premerge,
worktree, claim, validation, and merge gates. If promotion is dry for the configured
dry threshold, the conductor continues to Tier 2.

### Tier 2 — local codebase discovery

Run `/autospec-explore --once` over local sources (spec-vs-code, prior reports,
codebase signals, open issues, source analysis, dependency health). Verified, ranked
findings are filed as `auto-implement` issues and drained by Tier 1. Discovery uses
the explore sandbox safety model; it never merges unverified work directly to `main`.

### Tier 3 — architecture/test-coverage improvement

When local discovery is dry, generate higher-order improvement work: architecture rot,
complexity hotspots, duplication, dead code, and measurable test-coverage gaps. The
`AUTOSPEC_ARCHITECTURE_IMPROVEMENT_CMD` seam may provide a specialized generator;
otherwise the conductor uses the available explore single-cycle interface with
architecture/test-coverage/technical-debt sources. Filed work returns to Tier 1.

### Tier 4 — internet/operator-polish discovery

When Tier 3 is dry, run `/autospec-explore --once --research-sources internet` plus
operator-polish/persona lenses as available. Filed work returns to Tier 1.

### Park conditions

The conductor parks only when one of these named conditions fires:

- `control:pause` / `control:graceful-stop` from Tier 0;
- `usage-governor:park` or `spend-ledger:park`;
- `waterfall:park:all tiers dry` after Tier 1 backlog, Tier 1.5 promotion, Tier 2
  local discovery, Tier 3 architecture/coverage work, and Tier 4 discovery are all dry;
- fail-closed health conditions such as missing premerge gate or red `main`.

Set `AUTOSPEC_DISABLE_DISCOVERY_TIERS=1` only as an emergency fail-closed override;
with that override, the conductor parks at the Tier-1 dry threshold and names the
disable condition in the park reason.

## Usage observability (F6a spike finding)

The Phase-2 usage governor (F6b) parks the loop before quota exhaustion. It needs
to know whether a **live usage fraction** (percent of quota consumed this session)
is observable per harness. The F6a spike probed all three harnesses and the
finding is encoded in `usage-observe.sh <harness>` (`${AUTOSPEC_SCRIPTS_DIR}/usage-observe.sh`), which emits
`{harness, observable, percent, source}` (`percent` is `null` and
`observable` is `false` when no live fraction exists; an unknown harness exits
non-zero).

**Finding: no supported harness exposes a deterministic live usage fraction today.**
Every harness reports `observable:false`, so the governor MUST fall back to the
existing spend-ledger token tally (`autonomous-spend-ledger.sh`) and park at 90%
of `AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS`.

| Harness     | Live % observable? | Why / fallback                                                                                                          |
|-------------|--------------------|-----------------------------------------------------------------------------------------------------------------------|
| Claude Code | No                 | No env var or session signal carries a quota %. Per-message token counts in `~/.claude/projects/.../*.jsonl` are a cumulative tally (the spend-ledger fallback), not a live fraction. |
| Codex CLI   | No                 | No session-level quota fraction. Rate-limit headers are per-request/reset-based, not a cumulative session %.            |
| OpenCode    | No                 | Provider-dependent; no unified usage signal. Whatever the provider returns is per-request, not a normalized session %.  |

**Forward-compatible probe seam.** If a harness later ships a live fraction, wire
it without editing the script by setting the per-harness env var
`AUTOSPEC_USAGE_PROBE_CLAUDE` / `_CODEX` / `_OPENCODE` to an executable that prints
a single number `0-100` and exits `0`. When set and valid, `usage-observe.sh`
reports `observable:true` with that percent; otherwise it reports the honest
`observable:false` default above. This seam is what F6b consults and what the
bats suite mocks as a subprocess.

## Invocation

> **Model tier:** `TIER_A` for waterfall-level decisions and ranking; `TIER_B` for
> individual issue drain and deterministic steps (inherited from `/autospec-run`).

```
/autospec-autonomous [--max-cycles N] \
    [--budget-tokens N] \
    [--budget-hours N] \
    [--budget-issues N] \
    [--dry-run] \
    [--no-digest] \
    [--poll-interval-sec N]
```

Equivalent installed shell command:

```bash
autospec-autonomous start [--max-cycles N] [--dry-run] [--no-digest] [--poll-interval-sec N]
```

- `--max-cycles N` — outer loop cycle cap. Default unlimited.
- `--budget-tokens N` — lifetime token ceiling (sets `AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS`). Default 50M.
- `--budget-hours N` — wall-time budget. Default unlimited.
- `--budget-issues N` — lifetime issue ceiling. Default unlimited.
- `--dry-run` — go through the waterfall steps but do not invoke `/autospec-run` or merge; log what would happen.
- `--no-digest` — skip daily digest writes.
- `--poll-interval-sec N` — cycle polling interval in seconds. Default 60.

Tier-1 drain watchdog controls:

- `AUTOSPEC_AUTONOMOUS_DRAIN_STALL_SECS` — no-output stall budget for one `$autospec-run` drain. Default 1800; set `0` to disable.
- `AUTOSPEC_AUTONOMOUS_DRAIN_POLL_SECS` — poll interval for drain output progress. Default 15.

## Skill family layout

- `skills/autospec-autonomous/SKILL.md` — Claude Code adapter (authoritative).
- `skills/autospec-autonomous/codex/prompt.md` — Codex CLI mirror (lockstep).
- `skills/autospec-autonomous/opencode/agent.md` — OpenCode mirror (lockstep).
- `autospec-autonomous.sh` — installed operator lifecycle command (`start`, `status`, `timeline`, `logs`, `watch`, `stop`, `restart`).
- `autospec-autonomous-run-drain.sh` — installed Tier-1 drain wrapper that runs `$autospec-run` through `omx exec`.
- `autospec-loop.sh` (shared loop driver, `${AUTOSPEC_SCRIPTS_DIR}/lib/`) — extended with `autospec_conductor_run()`, the never-idle conductor entry point wiring control-channel → waterfall → Tier 1.5 promotion / Tier 2–4 discovery → premerge-gate → drain → spend-ledger → resilience → digest (issue #1378).
- `autonomous-control-channel.sh` — label query → command decision (Phase 1, Issue #1373).
- `autonomous-waterfall.sh` — Tier 0/1/1.5/2/3/4 selection logic (Issue #1374, activated by issue #1529).
- `autonomous-spend-ledger.sh` — cumulative token/issue tally + kill-switch (Phase 1, Issue #1375).
- `autonomous-premerge-gate.sh` — blocking autospec-qa pre-merge barrier (Phase 1, Issue #1376).
- `autonomous-resilience.sh` — run-state, lock, quarantine, main-health (Phase 1, Issue #1377).
- `usage-observe.sh` — per-harness live-usage observability probe; emits `{harness,observable,percent,source}` to gate the F6b governor's mechanism (F6a spike).
- `notify.sh` (autospec-shared) — shared desktop notifier (operator window during unattended runs).
- `tests/autospec/test_conductor_wiring.bats` — bats coverage for conductor wiring (issue #1378).
- `tests/fixtures/skill-goldens/autospec-autonomous.*.sha256` — derived goldens.

Trio edits use `derive-trio.sh --in-place` + `gen-skill-goldens.sh`; never hand-maintain the codex/opencode mirrors or goldens.

## Error handling

- **Worktree assert fails** → emit `code_health` identifier; park; notify operator.
- **Gate script missing** → `code_health:autonomous_gate_missing`; halt merges; notify.
- **`/autospec-run` fails** → per-issue failure cap; after cap → `autospec:needs-human`; no merge.
- **Main CI red** → halt Tier-1 merges; file `autospec:needs-human`; notify.
- **Spend ceiling reached** → park; write `~/.autospec/autonomous-stop.flag`; notify.
- **All waterfall tiers dry** → park with an `all tiers dry` reason naming exhausted tiers; notify.
- **Control-channel read error** → log `code_health:autonomous_control_channel_error`; treat as no-op for that cycle; continue.
- **Single-instance lock conflict** — reuses `autospec-resume` staleness thresholds (300s/10800s). A live lock blocks a second conductor. A stale lock (heartbeat older than threshold) is reclaimable by the fresh process.

## Primary smoke test

```bash
bash scripts/validate.sh
```
