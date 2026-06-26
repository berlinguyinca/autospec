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

## Design spec

`docs/specs/2026-06-25-autospec-autonomous-design.md`
