#!/usr/bin/env bash
# skills/autospec-shared/scripts/notify.sh — shared session event notifier
#
# Usage: notify.sh "<title>" "<body>"
#
# Persists an event for the active session. Desktop notifications are disabled
# by default; set AUTOSPEC_NOTIFY_DESKTOP=1 only for an explicit opt-in.
#
# Reuses the osascript/notify-send routing + escaping pattern from
# packages/autospec_context_monitor/autospec_context_monitor/adapters/claude_hook.py
# (ClaudeHookAdapter.notify_clear_needed), which already ships and is tested.
# This script is a thin shell entry point over the same pattern.
#
# Exit code is always 0 — notifier failures must never block the merge path.

set -u

# ── Guard: silent no-op when notifications are disabled ──────────────────────
if [ "${AUTOSPEC_NOTIFY:-1}" = "0" ]; then
    exit 0
fi

TITLE="${1:-}"
BODY="${2:-}"

EVENT_LOG="${AUTOSPEC_SESSION_EVENTS:-${HOME}/.autospec/session-events.jsonl}"
mkdir -p "$(dirname "$EVENT_LOG")" 2>/dev/null || true
python3 - "$EVENT_LOG" "$TITLE" "$BODY" <<'PY' 2>/dev/null || true
import json, sys, time
path, title, body = sys.argv[1:]
with open(path, "a", encoding="utf-8") as f:
    f.write(json.dumps({"ts": time.time(), "title": title, "body": body}) + "\n")
PY

if [ "${AUTOSPEC_NOTIFY_DESKTOP:-0}" != "1" ]; then
    exit 0
fi

# ── macOS: osascript ──────────────────────────────────────────────────────────
if command -v osascript > /dev/null 2>&1; then
    # Mirror claude_hook.py: escape backslashes then double-quotes so the
    # strings are safe inside an AppleScript double-quoted literal.
    escaped_title="${TITLE//\\/\\\\}"
    escaped_title="${escaped_title//\"/\\\"}"
    escaped_body="${BODY//\\/\\\\}"
    escaped_body="${escaped_body//\"/\\\"}"
    osascript -e "display notification \"${escaped_body}\" with title \"${escaped_title}\"" \
        > /dev/null 2>&1 || true
    exit 0
fi

# ── Linux: notify-send ───────────────────────────────────────────────────────
if command -v notify-send > /dev/null 2>&1; then
    # Args are separate argv entries — no AppleScript escaping needed.
    notify-send "$TITLE" "$BODY" > /dev/null 2>&1 || true
    exit 0
fi

# ── Fallback: stdout log line (headless / CI) ─────────────────────────────────
printf 'notify: %s \xe2\x80\x94 %s\n' "$TITLE" "$BODY"
