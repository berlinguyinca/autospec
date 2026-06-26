# autospec-autonomous

Perpetual self-driving conductor for the **autospec** suite. It runs the autospec
machinery unattended for weeks, walking a priority waterfall (Tier 0 control channel +
Tier 1 backlog → main), obeying a GitHub control channel for live steering, and parking
before quota exhaustion.

**Phase 1 scope:** Tier 0 + Tier 1 only. Tiers 2–4 (explore-backed discovery, persona,
self-brainstorm) are Phase 2/3 roadmap entries.

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

## Phase-1 waterfall

Each cycle executes in order:

1. **Tier-0 poll** — read GitHub control channel (`autonomous-control-channel.sh`).
2. **Tier selection** — `autonomous-waterfall.sh` picks Tier 0 or Tier 1.
3. **Tier-1 drain** — pick the highest-priority `auto-implement` issue and invoke `/autospec-run`.
4. **Pre-merge gate** — `autonomous-premerge-gate.sh` + `autospec-autonomy-gate.sh` validate before merge.
5. **Spend ledger** — `autonomous-spend-ledger.sh` tallies tokens/issues; parks when `AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS` or the issue ceiling is reached.
6. **Resilience** — `autonomous-resilience.sh` handles failures; notifies operator on persistent errors.

**Phase 1 scope:** Tier 0 + Tier 1 only. Tiers 2–4 (explore-backed discovery, persona,
self-brainstorm) are Phase 2/3 roadmap entries and are **not yet enabled**.

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

## Design spec

`docs/specs/2026-06-25-autospec-autonomous-design.md`
