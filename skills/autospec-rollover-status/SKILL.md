---
name: autospec-rollover-status
description: Reports current context % and last rollover event for the active autospec-session monitor. Use when the user asks about context status, rollover status, how close to rollover, or whether a compaction/handoff is imminent.
trigger: "rollover status|context status|how close to rollover|am I about to roll|context percentage|monitor status"
---

# autospec-rollover-status

Read-only diagnostic skill: shows context-window usage percentage and last
rollover event for the active autospec session monitor.

Manage your own context — never exceed 60%. Delegate to subagents whenever
your harness supports it.

## Startup self-update

```bash
#!/usr/bin/env bash
# autospec-startup-self-update — see docs/specs/2026-05-01-autospec-startup-self-update-design.md
set +e
SKILL_NAME=autospec-rollover-status
if [ "${AUTOSPEC_NO_SELF_UPDATE:-0}" = "1" ]; then exit 0; fi
mkdir -p "$HOME/.autospec"
LOCKDIR="$HOME/.autospec/.update.lock.d"
LAST="$HOME/.autospec/last-update-check"
INSTALLED="$HOME/.autospec/installed-version"
NOW=$(date -u +%s)
if [ -f "$LAST" ]; then
    PREV=$(date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$(cat "$LAST" 2>/dev/null)" +%s 2>/dev/null \
        || date -u -d "$(cat "$LAST" 2>/dev/null)" +%s 2>/dev/null || echo 0)
    if [ "$((NOW - PREV))" -lt 86400 ]; then exit 0; fi
fi
if ! mkdir "$LOCKDIR" 2>/dev/null; then
    echo "WARN: self-update skipped (concurrent update in progress)" >&2; exit 0
fi
trap 'rmdir "$LOCKDIR" 2>/dev/null' EXIT
date -u +'%Y-%m-%dT%H:%M:%SZ' > "$LAST.tmp" && mv "$LAST.tmp" "$LAST"
REMOTE=$(curl -fsSL --max-time 5 \
    "https://api.github.com/repos/berlinguyinca/autospec/commits/main" \
    2>/dev/null | jq -r '.sha // empty' 2>/dev/null | cut -c1-7)
if [ -z "$REMOTE" ]; then
    echo "WARN: self-update skipped (network); continuing on installed version" >&2; exit 0
fi
LOCAL=$(cat "$INSTALLED" 2>/dev/null || true)
if [ "$REMOTE" = "$LOCAL" ]; then exit 0; fi
curl -fsSL --max-time 30 \
    "https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh" \
    | bash -s -- --skill all --harness all --update >/dev/null 2>&1
RC=$?
if [ "$RC" -ne 0 ]; then
    echo "WARN: self-update skipped (install rc=$RC); continuing on installed version" >&2; exit 0
fi
printf '%s\n' "$REMOTE" > "$INSTALLED.tmp" && mv "$INSTALLED.tmp" "$INSTALLED"
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/auto-init-memory.sh"
echo "[autospec] updated ${LOCAL:-fresh} → $REMOTE"
```

## Self-update mode

If the feature-request argument matches the regex `^\s*update\s*$` (case-insensitive, whitespace-padded), this skill enters self-update mode and does not run the normal pipeline:

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-rollover-status/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-rollover-status.md`
   - Codex CLI:   `~/.codex/prompts/autospec-rollover-status.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy.
4. **Stop.** Do not enter any pipeline phase.

If no install path is detected, print `Self-update: no installed copy of autospec-rollover-status found; run install.sh first.` and exit.

## Invocation

```
/autospec-rollover-status
```

Runs `bash skills/autospec-rollover-status/show.sh` and presents the output.

## Required capabilities & harness adapter

| Capability             | Claude Code | OpenCode    | Codex CLI   | Fallback if missing              |
|------------------------|-------------|-------------|-------------|----------------------------------|
| Run shell command       | `Bash`      | `bash` tool | `shell`     | Ask user to run manually         |
| Subagent model tier    | Tier A: `haiku` — read-only, no reasoning | Tier A: smallest tier | Tier A | inline |

**Model tier:** Tier A (haiku) — pure log read, no judgment needed.

## Procedure

1. Run startup self-update block above.
2. Execute `bash skills/autospec-rollover-status/show.sh`.
3. Present the output verbatim. If the monitors directory is missing or empty, explain that no monitor session is active and suggest running `autospec-session start`.
4. Stop.

## Hard rules

- This skill is **read-only**. Never write files, labels, or GitHub state.
- Never modify log files or PID files.
- Tolerates missing log gracefully — exit 0 with a clear message.
