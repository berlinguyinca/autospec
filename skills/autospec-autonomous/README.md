# autospec-autonomous

Perpetual self-driving conductor for the **autospec** suite. It runs the autospec
machinery unattended for weeks, walking a never-idle priority waterfall (Tier 0 control channel + Tier 1 backlog →
main + Tier 1.5 open-issue promotion + Tier 2 local discovery + Tier 3
architecture/coverage improvement + Tier 4 internet/operator discovery), obeying a
GitHub control channel for live steering, and parking before quota exhaustion.

The conductor parks only when every tier is dry, a stop/pause control signal is set,
or the usage/spend governor trips.

Works on **Claude Code**, **OpenCode**, and **Codex CLI**.

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-autonomous/install.sh | sh -s -- --harness all
```

Per-harness:

```bash
# Claude Code only
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-autonomous/install.sh | sh -s -- --harness claude
# OpenCode only
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-autonomous/install.sh | sh -s -- --harness opencode
# Codex CLI only
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-autonomous/install.sh | sh -s -- --harness codex
```

From a clone:

```bash
cd skills/autospec-autonomous
./install.sh --harness all
```

## Usage

```
/autospec-autonomous [--max-cycles N] [--budget-tokens N] [--budget-hours N] \
    [--budget-issues N] [--dry-run] [--no-digest] [--poll-interval-sec N]
```

Self-update: `/autospec-autonomous update`

Stop: `/autospec-autonomous stop [--graceful|--immediate]`

## Uninstall

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-autonomous/uninstall.sh | sh -s -- --harness all
```

Or from a clone:

```bash
cd skills/autospec-autonomous
./uninstall.sh --harness all
```

## Control labels (GitHub)

Apply these labels to any open issue to send a Tier-0 control-channel signal:

| Label                  | Effect                                                                     |
|------------------------|----------------------------------------------------------------------------|
| `autospec:stop`        | Write `~/.autospec/stop.flag`; finish the current issue; exit cleanly.    |
| `autospec:pause`       | Park the loop; notify operator; wait for resume or `autospec:stop`.       |
| `autospec:priority`    | Re-sort the Tier-1 backlog by the label body before the next drain.       |
| `autospec:steer`       | Parse the label body as a directive; update the active waterfall intent; remove the label. |

Tier 0 always preempts Tier 1. A `stop` or `pause` signal is honored at the next
cycle boundary — never mid-issue.

## Never-idle waterfall

Each cycle executes in order:

1. **Tier-0 poll** — read GitHub control channel (`autonomous-control-channel.sh`).
2. **Tier selection** — `autonomous-waterfall.sh` picks the highest-priority tier with work.
3. **Tier-1 drain** — pick the highest-priority `auto-implement` issue and invoke `/autospec-run`.
4. **Tier-1.5 promotion** — when Tier 1 is dry but other issues are open, promote/decompose/classify safe work into `auto-implement`.
5. **Tier-2 local discovery** — run `/autospec-explore --once` over local repo signals and file verified work.
6. **Tier-3 architecture/coverage improvement** — generate high-ROI architecture, debt, and test-coverage work.
7. **Tier-4 internet/operator discovery** — run internet/operator-polish discovery and file verified work.
8. **Pre-merge gate** — Tier-1 drains still require `autonomous-premerge-gate.sh` + `autospec-autonomy-gate.sh` before merge.
9. **Spend ledger/governor** — park when usage or lifetime token/issue ceilings are reached.
10. **Resilience** — `autonomous-resilience.sh` handles failures; notifies operator on persistent errors.

Set `AUTOSPEC_DISABLE_DISCOVERY_TIERS=1` only as an emergency fail-closed override; otherwise Tier 1.5–4 are active by default.

### Troubleshooting: main-health check-runs

When legacy commit statuses are absent, main-health reads GitHub check-runs on
`main`. Release-publish failures for `Publish @autospec/cli to npm` and
`Open PR on homebrew-autospec tap` are ignored by default, so they do not block
main-health or Tier-1 merges. Override the ignored check-run name regex with
`AUTOSPEC_MAIN_HEALTH_IGNORE_CHECKS`.

## Usage observability (F6a spike finding)

The Phase-2 usage governor parks the loop before quota exhaustion. The F6a spike
probed each harness for a **live usage fraction** (percent of quota consumed this
session). `scripts/usage-observe.sh <harness>` encodes the finding, emitting
`{harness, observable, percent, source}`.

**Finding: no supported harness exposes a deterministic live usage fraction today.**
All three report `observable:false`, so the governor falls back to the spend-ledger
token tally and parks at 90% of `AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS`.

| Harness     | Live % observable? | Why                                                                                       |
|-------------|--------------------|-------------------------------------------------------------------------------------------|
| Claude Code | No                 | No env/session signal carries a quota %; transcript token counts are a cumulative tally.   |
| Codex CLI   | No                 | No session-level quota %; rate-limit headers are per-request/reset-based.                  |
| OpenCode    | No                 | Provider-dependent; no unified session usage signal.                                       |

If a harness later ships a live fraction, set `AUTOSPEC_USAGE_PROBE_CLAUDE` /
`_CODEX` / `_OPENCODE` to an executable that prints a number `0-100`; the probe
then reports `observable:true` with that percent.

## Environment variables

| Variable                              | Default               | Description                                                |
|---------------------------------------|-----------------------|------------------------------------------------------------|
| `AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS` | `50000000`            | Cumulative token ceiling; triggers park-and-notify.        |
| `AUTOSPEC_SCRIPTS_DIR`                | `~/.autospec/scripts` | Runtime scripts directory.                                 |
| `CLAUDE_CONFIG_DIR`                   | `~/.claude`           | Claude Code config directory.                              |
| `OPENCODE_CONFIG_DIR`                 | `~/.config/opencode`  | OpenCode config directory.                                 |
| `CODEX_HOME`                          | `~/.codex`            | Codex CLI home directory.                                  |
| `AUTOSPEC_AUTONOMOUS_REF`             | `main`                | Git ref to fetch from when piped via curl.                 |
| `AUTOSPEC_AUTONOMOUS_RAW_BASE`        | —                     | Override the raw GitHub URL base entirely.                 |
| `AUTOSPEC_DISABLE_DISCOVERY_TIERS`     | `0`                   | Emergency fail-closed park at Tier-1 dry threshold.        |
| `AUTOSPEC_PROMOTE_OPEN_ISSUES_CMD`     | auto-detect           | Override Tier-1.5 promotion/decomposition/classification.  |
| `AUTOSPEC_ARCHITECTURE_IMPROVEMENT_CMD` | auto-detect          | Override Tier-3 architecture/coverage work generation.     |
| `AUTOSPEC_AUTONOMOUS_DRAIN_STALL_SECS` | `1800`                | No-output stall budget for one `$autospec-run` drain; `0` disables. |
| `AUTOSPEC_AUTONOMOUS_DRAIN_POLL_SECS` | `15`                  | Poll interval for Tier-1 drain output progress.            |

## Design spec

`docs/specs/2026-06-25-autospec-autonomous-design.md`
