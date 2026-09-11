#!/usr/bin/env bash
# select-model-profile.sh — Select the TIER_B implementer profile for a given issue.
#
# Usage:
#   select-model-profile.sh --labels <comma-separated-labels> [--profiles-file <path>]
#                           [--print-model] [--print-effort] [--kind <dispatch_kind>]
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
# Roles (Guardrail R2):
#   A profile may declare which dispatch kinds it may serve via a `roles:` list.
#   `--kind <dispatch_kind>` is checked against the resolved profile: if that
#   profile declares `roles:` and the kind is absent, the script prints nothing
#   and exits 3 so the caller keeps its cloud tier. A profile with NO `roles:`
#   key is unconstrained and routes exactly as it does today (absent means
#   unconstrained, so existing catalogs do not change routing).
#
# Exit codes:
#   0  profile name (or model id) printed
#   1  usage error
#   3  nothing resolvable in the catalog (--print-model / --print-effort: caller
#      must keep its harness-detected TIER_B / its default effort), OR a role
#      mismatch: --kind is absent from the resolved profile's declared `roles:`
#      (caller must keep its cloud tier)

set -eu

LABELS=""
PRINT_MODEL=0
PRINT_EFFORT=0
KIND=""
PROFILES_FILE="${AUTOSPEC_MODEL_PROFILES:-$HOME/.autospec/model-profiles.yml}"

while [ $# -gt 0 ]; do
    case "$1" in
        --labels)
            LABELS="${2:-}"
            shift 2
            ;;
        --kind)
            KIND="${2:-}"
            shift 2
            ;;
        --profiles-file)
            PROFILES_FILE="${2:-}"
            shift 2
            ;;
        --print-effort)
            PRINT_EFFORT=1
            shift
            ;;
        --print-model)
            PRINT_MODEL=1
            shift
            ;;
        --help|-h)
            printf 'Usage: select-model-profile.sh --labels <labels> [--profiles-file <path>] [--print-model] [--print-effort] [--kind <dispatch_kind>]\n'
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
# Generalised over the key ($3) rather than duplicated per key: two awk programs
# in one file that both define indent_of() trip the duplicate-function gate, and
# the block-scoping logic is the part worth having exactly once.
_profile_field() {
    awk -v want="$2" -v field="$3" '
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

            if (in_block && key ~ "^" field ":[[:space:]]*[^[:space:]]") {
                val = key
                sub("^" field ":[[:space:]]*", "", val)
                sub(/[[:space:]]+$/, "", val)
                gsub(/["\047]/, "", val)
                if (val != "") { print val; exit }
            }
        }
    ' "$1"
}

# ── Resolve a profile's declared `roles:` list ─────────────────────────────────
# Guardrail R2: which dispatch kinds a profile may serve. Scoped to the matched
# profile's own block (same block-scoping as _profile_field) so an adjacent
# profile's roles are never harvested. Handles both flow (`roles: [a, b]`) and
# block (`roles:` + `- item`) YAML list forms. Prints one role per line; prints
# NOTHING when the profile declares no `roles:` key (absent = unconstrained), so
# the caller treats an empty result as "no restriction".
_profile_roles() {
    awk -v want="$2" '
        function indent_of(s) { match(s, /^ */); return RLENGTH }
        {
            line = $0
            sub(/[[:space:]]*#.*$/, "", line)          # drop comments
            if (line ~ /^[[:space:]]*$/) { next }      # skip blank/comment-only
            ind = indent_of(line)
            key = line
            sub(/^[[:space:]]+/, "", key)

            if (in_roles) {
                if (key ~ /^-[[:space:]]*/) {
                    item = key
                    sub(/^-[[:space:]]*/, "", item)
                    sub(/[[:space:]]+$/, "", item)
                    gsub(/["\047]/, "", item)
                    if (item != "") print item
                    next
                }
                in_roles = 0                             # a non-list line ends it
            }

            if (in_block && ind <= block_ind) { in_block = 0 }

            # `roles:` looks like a mapping-opener line, so it must be tested
            # before the generic opener below or it is swallowed as a new block.
            if (in_block && key ~ /^roles:/) {
                val = key
                sub(/^roles:[[:space:]]*/, "", val)
                sub(/[[:space:]]+$/, "", val)
                if (val == "") { in_roles = 1; next }   # block-list opener
                sub(/^\[/, "", val)                     # strip a flow-list wrapper
                sub(/]$/, "", val)
                n = split(val, parts, ",")
                for (i = 1; i <= n; i++) {
                    p = parts[i]
                    sub(/^[[:space:]]+/, "", p)
                    sub(/[[:space:]]+$/, "", p)
                    gsub(/["\047]/, "", p)
                    if (p != "") print p
                }
                next
            }

            if (key ~ /^[^:]+:[[:space:]]*$/) {          # a mapping-opener line
                name = key
                sub(/:[[:space:]]*$/, "", name)
                if (name == want) { in_block = 1; block_ind = ind }
                next
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

# ── Role guard (Guardrail R2) ─────────────────────────────────────────────────
# If --kind is given and the resolved profile declares `roles:` that omits the
# kind, fail closed (exit 3, empty stdout) so the caller keeps its cloud tier.
# A profile with no `roles:` key is unconstrained and routing is unchanged.
if [ -n "$KIND" ]; then
    profile_roles=""
    if [ -f "$PROFILES_FILE" ]; then
        profile_roles="$(_profile_roles "$PROFILES_FILE" "$RESOLVED_PROFILE")"
    fi
    if [ -n "$profile_roles" ]; then
        _role_ok=0
        _old_ifs="$IFS"; IFS=$'\n'
        for _r in $profile_roles; do
            if [ "$_r" = "$KIND" ]; then _role_ok=1; break; fi
        done
        IFS="$_old_ifs"
        if [ "$_role_ok" -eq 0 ]; then
            exit 3
        fi
    fi
fi

# ── Emit ──────────────────────────────────────────────────────────────────────
# Default: the profile name. With --print-model: the profile's `model:` id, or
# exit 3 so the caller keeps its harness-detected TIER_B (fail closed — never
# guess a model id).
# --print-effort resolves the profile's `effort:` tier. Effort is a routable
# dimension in its own right, and often a BETTER dial than swapping models:
# switching model invalidates the entire prompt cache across all three tiers,
# while raising effort on the same model keeps the cached prefix intact. It is
# reported, never modelled as a cost multiplier — two profiles that differ only in
# effort are separate catalog rows, so the ledger MEASURES the difference instead
# of the scorer guessing a factor.
if [ "$PRINT_EFFORT" -eq 1 ]; then
    resolved_effort=""
    if [ -f "$PROFILES_FILE" ]; then
        resolved_effort="$(_profile_field "$PROFILES_FILE" "$RESOLVED_PROFILE" effort)"
    fi
    if [ -z "$resolved_effort" ]; then
        exit 3
    fi
    printf '%s\n' "$resolved_effort"
    exit 0
fi

if [ "$PRINT_MODEL" -eq 0 ]; then
    printf '%s\n' "$RESOLVED_PROFILE"
    exit 0
fi

resolved_model=""
if [ -f "$PROFILES_FILE" ]; then
    resolved_model="$(_profile_field "$PROFILES_FILE" "$RESOLVED_PROFILE" model)"
fi

if [ -z "$resolved_model" ]; then
    exit 3
fi

printf '%s\n' "$resolved_model"
