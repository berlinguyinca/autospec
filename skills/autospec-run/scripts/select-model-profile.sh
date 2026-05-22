#!/usr/bin/env bash
# select-model-profile.sh — Select the TIER_B implementer profile for a given issue.
#
# Usage:
#   select-model-profile.sh --labels <comma-separated-labels> [--profiles-file <path>]
#
# Prints the profile name to stdout (e.g. "claude-haiku-cloud" or "claude-sonnet-cloud").
#
# Routing rules (Phase 2 Haiku trial):
#   - reasoning:shallow  → claude-haiku-cloud  (if available in profiles file)
#   - reasoning:medium   → claude-haiku-cloud  (if available in profiles file)
#   - reasoning:deep     → claude-sonnet-cloud (always)
#   - default (no reasoning label) → AUTOSPEC_TIER_B_PROFILE or claude-sonnet-cloud
#
# Environment:
#   AUTOSPEC_TIER_B_PROFILE   override default TIER_B profile name
#   AUTOSPEC_MODEL_PROFILES   path to model-profiles.yml (default: ~/.autospec/model-profiles.yml)

set -eu

LABELS=""
PROFILES_FILE="${AUTOSPEC_MODEL_PROFILES:-$HOME/.autospec/model-profiles.yml}"

while [ $# -gt 0 ]; do
    case "$1" in
        --labels)
            LABELS="${2:-}"
            shift 2
            ;;
        --profiles-file)
            PROFILES_FILE="${2:-}"
            shift 2
            ;;
        --help|-h)
            printf 'Usage: select-model-profile.sh --labels <labels> [--profiles-file <path>]\n'
            exit 0
            ;;
        *)
            printf 'select-model-profile.sh: unknown option: %s\n' "$1" >&2
            exit 1
            ;;
    esac
done

# ── Default TIER_B profile ─────────────────────────────────────────────────────
DEFAULT_PROFILE="${AUTOSPEC_TIER_B_PROFILE:-claude-sonnet-cloud}"
HAIKU_PROFILE="claude-haiku-cloud"

# ── Check if Haiku profile exists in profiles file ────────────────────────────
_haiku_available() {
    if [ ! -f "$PROFILES_FILE" ]; then
        return 1
    fi
    grep -q "^${HAIKU_PROFILE}:" "$PROFILES_FILE" 2>/dev/null || \
    grep -q "^  ${HAIKU_PROFILE}:" "$PROFILES_FILE" 2>/dev/null || \
    grep -q "^    ${HAIKU_PROFILE}:" "$PROFILES_FILE" 2>/dev/null
}

# ── Routing decision ──────────────────────────────────────────────────────────
# Parse reasoning label from comma-separated labels
reasoning_label=""
IFS=',' read -ra label_arr <<< "$LABELS"
for lbl in "${label_arr[@]}"; do
    lbl="$(printf '%s' "$lbl" | tr -d ' ')"
    case "$lbl" in
        reasoning:shallow|reasoning:medium|reasoning:deep)
            reasoning_label="$lbl"
            break
            ;;
    esac
done

case "$reasoning_label" in
    reasoning:shallow|reasoning:medium)
        if _haiku_available; then
            printf '%s\n' "$HAIKU_PROFILE"
        else
            printf '%s\n' "$DEFAULT_PROFILE"
        fi
        ;;
    reasoning:deep)
        printf '%s\n' "$DEFAULT_PROFILE"
        ;;
    *)
        printf '%s\n' "$DEFAULT_PROFILE"
        ;;
esac
