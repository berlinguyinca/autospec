# autospec-listen

A passive conversation listener that watches for issue-filing or
spec-writing intent mid-conversation and dispatches to the right
downstream flow without forcing the user to remember a slash command.

Works on **Claude Code**, **OpenCode**, and **Codex CLI**.

## Overview

Per spec §2.1, `autospec-listen` runs alongside the rest of an agent
session and only activates when the user says one of the canonical
trigger phrases (see [Trigger phrases](#trigger-phrases)). On a match it
either drafts a GitHub issue body for confirmation (`issue` trigger) or
hands off to `/autospec-define` (`spec` trigger). It never fires on bare
nouns like "issue", "spec", or "ticket".

## Trigger phrases

The full set of trigger phrases — and the rule that bare nouns are NOT
triggers — lives in
[`references/trigger-keywords.md`](./references/trigger-keywords.md).

## Install

```bash
./install.sh                    # interactive (prompts for harness)
./install.sh --harness all      # install for Claude Code + OpenCode + Codex CLI
./install.sh --harness claude   # one harness
./install.sh --harness opencode
./install.sh --harness codex
./install.sh --harness all --update   # idempotent re-install
```

## Uninstall

```bash
./uninstall.sh                  # interactive
./uninstall.sh --harness all    # remove from all harnesses
./uninstall.sh --harness claude
```

Both scripts are idempotent: re-running on an already-(un)installed
system exits 0.

## How it works

- **Issue trigger flow** (spec §3.1): on a match like `file an issue`,
  the listener reads the last ~10 conversation turns, drafts a
  `Goal / Context / Suggested AC` body, asks the user to confirm, and on
  approval runs `gh issue create --label needs-classify`.
- **Spec trigger flow** (spec §3.2): on a match like `write a spec`,
  the listener confirms intent and routes to `/autospec-define` for the
  full Phase 0–3 planning flow. The listener does not duplicate
  `/autospec-define` logic — it is purely a router.
- **Lifecycle:** issues filed by the listener carry the
  `needs-classify` label until `/autospec-classify` transitions them to
  `auto-implement` (per spec §5.3).

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec-define`](../autospec-define/README.md) | Planning half. Listener routes spec triggers here. |
| [`autospec-classify`](../autospec-classify/README.md) | Sweeps `needs-classify` issues into the implementation queue. |
| [`autospec-run`](../autospec-run/README.md) | Implementation half. Consumes the `auto-implement` queue. |
| [`autospec`](../autospec/README.md) | Umbrella skill that runs planning + implementation end-to-end. |
