#!/usr/bin/env bash
# select-model-profile.sh — Select the TIER_B implementer profile for a given issue.
#
# Usage:
#   select-model-profile.sh --labels <comma-separated-labels> [--profiles-file <path>]
#                           [--print-model]
#
# Prints the profile name to stdout (e.g. "claude-haiku-cloud" or "claude-sonnet-cloud").
#
# With --print-model, prints the resolved profile's `model:` id instead (e.g.
# "claude-haiku-4-5") — this is what a dispatch site needs to override TIER_B.
# The lookup is fail-closed: if no id can be resolved (profiles file missing, the
# resolved profile has no `model:` key, or the key is commented out) it prints
# NOTHING and exits 3, so the caller keeps its harness-detected TIER_B rather
# than guessing a model. autospec-run's auto-init writes ctx/reasoning ceilings
# only, so exit 3 is the expected outcome on an auto-initialised profiles file.
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
#
# Exit codes:
#   0  profile name (or model id) printed
#   1  usage error
#   3  --print-model only: no model id resolvable (caller must keep TIER_B)

set -eu

LABELS=""
PRINT_MODEL=0
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
        --print-model)
            PRINT_MODEL=1
            shift
            ;;
        --help|-h)
            printf 'Usage: select-model-profile.sh --labels <labels> [--profiles-file <path>] [--print-model]\n'
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

# ── Resolve a profile's `model:` id ───────────────────────────────────────────
# Handles both shipped layouts: top-level `<profile>:` blocks (examples/
# model-profiles.yml) and profiles nested under a `profiles:` key (auto-init
# output). Scoped to the matched profile's own block so a `model:` belonging to
# an adjacent profile is never harvested. Prints nothing when unresolvable and
# always exits 0 — the caller decides what an empty result means.
_profile_model() {
    awk -v want="$2" '
        function indent_of(s) { match(s, /^ */); return RLENGTH }
        {
            line = $0
            sub(/[[:space:]]*#.*$/, "", line)          # drop comments
            if (line ~ /^[[:space:]]*$/) { next }      # skip blank/comment-only
            ind = indent_of(line)
            key = line
            sub(/^[[:space:]]+/, "", key)

            if (in_block && ind <= block_ind) { in_block = 0 }

            if (key ~ /^[^:]+:[[:space:]]*$/) {        # a mapping-opener line
                name = key
                sub(/:[[:space:]]*$/, "", name)
                if (name == want) { in_block = 1; block_ind = ind }
                next
            }

            if (in_block && key ~ /^model:[[:space:]]*[^[:space:]]/) {
                val = key
                sub(/^model:[[:space:]]*/, "", val)
                sub(/[[:space:]]+$/, "", val)
                gsub(/[\"\047]/, "", val)
                if (val != "") { print val; exit }
            }
        }
    ' "$1"
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

RESOLVED_PROFILE=""
case "$reasoning_label" in
    reasoning:shallow|reasoning:medium)
        if _haiku_available; then
            RESOLVED_PROFILE="$HAIKU_PROFILE"
        else
            RESOLVED_PROFILE="$DEFAULT_PROFILE"
        fi
        ;;
    reasoning:deep)
        RESOLVED_PROFILE="$DEFAULT_PROFILE"
        ;;
    *)
        RESOLVED_PROFILE="$DEFAULT_PROFILE"
        ;;
esac

# ── Emit ──────────────────────────────────────────────────────────────────────
# Default: the profile name. With --print-model: the profile's `model:` id, or
# exit 3 so the caller keeps its harness-detected TIER_B (fail closed — never
# guess a model id).
if [ "$PRINT_MODEL" -eq 0 ]; then
    printf '%s\n' "$RESOLVED_PROFILE"
    exit 0
fi

resolved_model=""
if [ -f "$PROFILES_FILE" ]; then
    resolved_model="$(_profile_model "$PROFILES_FILE" "$RESOLVED_PROFILE")"
fi

if [ -z "$resolved_model" ]; then
    exit 3
fi

printf '%s\n' "$resolved_model"
