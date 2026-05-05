---
description: Use when the user wants to halt a running autospec monitor gracefully or immediately — writes the ~/.autospec/stop.flag sentinel so the monitor exits after the current issue (--graceful) or aborts at the next step boundary (--immediate). Also supports --status, --resume, and inline stop sub-mode matching `^\s*stop(\s+--\w+)*\s*$`.
mode: primary
---

# autospec-stop (harness-neutral)

Halt a running autospec monitor gracefully or immediately. All paths route
through `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" "$@"`.

Manage your own context — never exceed 60%. Delegate to subagents whenever your harness supports it.

## Startup self-update

```bash
#!/usr/bin/env bash
# autospec-startup-self-update — see docs/specs/2026-05-01-autospec-startup-self-update-design.md
set +e
SKILL_NAME=autospec-stop   # per-skill: autospec-define / autospec-run / autospec-listen / autospec-classify / autospec-stop
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
bash <(curl -fsSL --max-time 30 \
    "https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/$SKILL_NAME/install.sh") \
    --harness all --update >/dev/null 2>&1
RC=$?
if [ "$RC" -ne 0 ]; then
    echo "WARN: self-update skipped (install rc=$RC); continuing on installed version" >&2; exit 0
fi
printf '%s\n' "$REMOTE" > "$INSTALLED.tmp" && mv "$INSTALLED.tmp" "$INSTALLED"
echo "[autospec] updated ${LOCAL:-fresh} → $REMOTE"
```

## Self-update mode

If the feature-request argument matches the regex `^\s*update\s*$` (case-insensitive, whitespace-padded), this skill enters self-update mode and does not run the normal pipeline:

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-stop/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-stop.md`
   - Codex CLI:   `~/.codex/prompts/autospec-stop.md`
2. **Re-install from `main`** by piping the canonical installer:
   ```bash
   bash <(curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-stop/install.sh) --harness <detected> --update
   ```
   If multiple harness paths exist, run the one-liner once per detected harness.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy.
4. **Stop.** Do not enter any pipeline phase. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-stop found; run install.sh first.` and exit.

## Stop mode

If the feature-request argument matches the regex `^\s*stop(\s+--\w+)*\s*$`
(case-insensitive), this skill enters stop mode and does not run the normal
pipeline:

1. Dispatch to `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" <args>`.
2. Print the helper's stdout to the user.
3. Stop. Do not enter Phase 0 or any pipeline phase.

## Invocation

```
/autospec-stop [--graceful|--immediate|--status|--resume|--help]
```

Dispatches directly to `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" "$@"`. All logic lives
in the helper script.

| Flag | Behaviour |
|---|---|
| `--graceful` (default) | Write sentinel mode=graceful; monitor exits after current issue finishes. |
| `--immediate` | Write sentinel mode=immediate; process(ISSUE) commits+pushes+marks paused at next step boundary. |
| `--status` | Report current sentinel state, paused-by-user issue count, last monitor progress line. |
| `--resume` | Strip paused-by-user labels from all paused issues, restore auto-implement, delete stop.flag. |
| `--help` | Print usage and exit. |

## Required capabilities & harness adapter

This workflow assumes a small set of capabilities. Map each one to your harness's actual tool. If a capability is missing, use the listed fallback.

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Run shell command            | `Bash`                               | `bash` tool                              | `shell` / `apply_patch`                  | Ask user to run manually                           |
| Ask the user a question      | `AskUserQuestion`                    | inline prompt                            | inline prompt                            | Ask in the response and wait for the next turn     |
| Subagent model tier          | Tier B: `sonnet` + medium thinking   | Tier B: smaller-tier `task` + medium reasoning | Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Fall back UP on unavailability |

**Model tier:** Tier B (implementation work) — this skill is a thin dispatch wrapper.

## Procedure

1. Run startup self-update block above.
2. Parse the user's invocation arguments (default to `--graceful` if no flag given).
3. Execute: `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" "$@"`
4. Print stdout to user and exit.

If `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh` is not found, print:
```
autospec-stop: helper not found at ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh.
Reinstall autospec-stop from the autospec repo, then retry.
```
and exit non-zero.

## Hard rules

- Never call `autospec-stop.sh --abort-current-issue` from user invocation. That flag is internal.
- Never modify issue bodies, labels, or PRs directly — the helper script owns all GitHub mutations.
- This skill is a thin wrapper. All behaviour is in the helper script.
