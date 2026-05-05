# autospec-stop

Halt a running **autospec** monitor gracefully or immediately. Writes the
`~/.autospec/stop.flag` sentinel so the monitor exits cleanly after the current
issue (`--graceful`) or aborts at the next step boundary with a WIP-commit +
paused label (`--immediate`).

Works on **Claude Code**, **OpenCode**, and **Codex CLI**.

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-stop/install.sh | sh -s -- --harness all
```

Per-harness:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-stop/install.sh | sh -s -- --harness claude
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-stop/install.sh | sh -s -- --harness opencode
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-stop/install.sh | sh -s -- --harness codex
```

From a clone:

```bash
cd skills/autospec-stop
./install.sh --harness all
```

## Flags

| Flag | Behaviour |
|---|---|
| `--graceful` (default) | Write sentinel mode=graceful; monitor exits after current issue finishes. |
| `--immediate` | Write sentinel mode=immediate; process(ISSUE) commits+pushes+marks paused at next step boundary. |
| `--status` | Report current sentinel state, paused-by-user issue count. |
| `--resume` | Strip paused-by-user labels, restore auto-implement, delete stop.flag. |

## Invocation examples

```bash
/autospec-stop --immediate          # canonical (new skill)
/autospec stop --immediate          # inline sub-mode in existing /autospec skill
/autospec-run stop --graceful       # inline sub-mode in monitor skill
```

All three paths route through the installed `autospec-stop.sh` helper in `~/.autospec/scripts` (or `AUTOSPEC_SCRIPTS_DIR`).

## Spec

`docs/specs/2026-05-01-autospec-stop-mechanism-design.md`
