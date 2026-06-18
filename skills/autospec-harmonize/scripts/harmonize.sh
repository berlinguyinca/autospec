#!/usr/bin/env bash
# skills/autospec-harmonize/scripts/harmonize.sh
#
# Orchestrator for the 6-stage autospec-harmonize pipeline:
#   1. design-discover.sh  → discovered-tokens.json + inventory.md
#   2. design-generalize.mjs --tokens <discovered>  → baseline.json
#   3. design-variants.mjs --baseline <baseline> --axes <axes>  → variants.json
#   4. design-preview.mjs --variants variants.json --out <out>  → preview/index.html
#   5. (pick gate lives in SKILL.md; non-interactive default used here)
#   6. gen-migration-spec.mjs ...  → docs/specs/<date>-harmonize-<slug>-design.md
#
# CLI:
#   harmonize.sh --root <dir> [--url <u>] [--source-only] [--pages <list>]
#                [--variants <axes>] [--num-variants <n>] [--no-live-preview]
#                --out <dir>
#
# Exit codes:
#   0  success
#   1  discovery yielded empty token profile — abort
#   2  usage error
#   3  generalize or variants stage failed
#
# Conventions: set -u (no -e); if/then/fi for one-sided conditionals; no RETURN
# traps (repo bash 3.2 / set -e gotchas); functions under 50 LOC.

set -u

PROG="$(basename "${BASH_SOURCE[0]}")"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ---------------------------------------------------------------------------
# Stage-script resolver — prefer AUTOSPEC_SCRIPTS_DIR, fall back to sibling dir
# ---------------------------------------------------------------------------
resolve_stage_dir() {
    if [ -n "${AUTOSPEC_SCRIPTS_DIR:-}" ] && [ -d "${AUTOSPEC_SCRIPTS_DIR}" ]; then
        printf '%s' "$AUTOSPEC_SCRIPTS_DIR"
    else
        printf '%s' "$SCRIPT_DIR"
    fi
}

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
err()  { printf '%s: error: %s\n' "$PROG" "$*" >&2; }
info() { printf '%s\n' "$*"; }

code_health() {  # code_health <signal> <message>
    signal="$1"; shift
    printf 'code_health:%s — %s\n' "$signal" "$*" >&2
}

usage() {
    cat <<EOF
Usage: $PROG --root <dir> [--url <u>] [--source-only] [--pages <list>]
             [--variants <axes>] [--num-variants <n>] [--no-live-preview]
             --out <dir>

Flags:
  --root <dir>         Codebase root to analyze (required)
  --url <u>            Extract tokens from live URL (falls back to source on failure)
  --source-only        Skip runtime extraction; always use source CSS/SCSS
  --pages <list>       Comma-separated URL paths for runtime extraction
  --variants <axes>    Comma-separated variant axes beyond baseline (default: minimal,high-contrast)
  --num-variants <n>   Total variants to generate including baseline (default: 3)
  --no-live-preview    Suppress live-preview offer in pick gate
  --out <dir>          Output directory for all artifacts (required)
EOF
}

# ---------------------------------------------------------------------------
# Arg parsing
# ---------------------------------------------------------------------------
ROOT=""
URL=""
SOURCE_ONLY=0
PAGES=""
VARIANT_AXES="minimal,high-contrast"
NUM_VARIANTS=3
NO_LIVE_PREVIEW=0
OUT=""

while [ $# -gt 0 ]; do
    case "$1" in
        --root)           ROOT="$2";           shift 2 ;;
        --url)            URL="$2";            shift 2 ;;
        --source-only)    SOURCE_ONLY=1;       shift   ;;
        --pages)          PAGES="$2";          shift 2 ;;
        --variants)       VARIANT_AXES="$2";   shift 2 ;;
        --num-variants)   NUM_VARIANTS="$2";   shift 2 ;;
        --no-live-preview) NO_LIVE_PREVIEW=1;  shift   ;;
        --out)            OUT="$2";            shift 2 ;;
        -h|--help)        usage; exit 0 ;;
        *) err "unknown option: $1"; usage >&2; exit 2 ;;
    esac
done

if [ -z "$ROOT" ]; then
    err "--root <dir> is required"
    usage >&2
    exit 2
fi

if [ -z "$OUT" ]; then
    err "--out <dir> is required"
    usage >&2
    exit 2
fi

STAGE_DIR="$(resolve_stage_dir)"

# ---------------------------------------------------------------------------
# Stage 1 — Discover
# ---------------------------------------------------------------------------
run_discover() {
    local discover_script="$STAGE_DIR/design-discover.sh"
    if [ ! -f "$discover_script" ]; then
        err "design-discover.sh not found in $STAGE_DIR"
        exit 2
    fi

    local discover_args
    discover_args="--root $ROOT --out $OUT"
    if [ -n "$URL" ] && [ "$SOURCE_ONLY" -eq 0 ]; then
        discover_args="$discover_args --url $URL"
    fi
    if [ "$SOURCE_ONLY" -eq 1 ]; then
        discover_args="$discover_args --source-only"
    fi
    if [ -n "$PAGES" ]; then
        discover_args="$discover_args --pages $PAGES"
    fi

    # shellcheck disable=SC2086
    bash "$discover_script" $discover_args
}

# ---------------------------------------------------------------------------
# Stage 2 — Generalize
# ---------------------------------------------------------------------------
run_generalize() {
    local generalize_script="$STAGE_DIR/design-generalize.mjs"
    if [ ! -f "$generalize_script" ]; then
        err "design-generalize.mjs not found in $STAGE_DIR"
        exit 2
    fi
    node "$generalize_script" --tokens "$OUT/discovered-tokens.json" > "$OUT/baseline.json"
}

# ---------------------------------------------------------------------------
# Stage 3 — Variants
# ---------------------------------------------------------------------------
run_variants() {
    local variants_script="$STAGE_DIR/design-variants.mjs"
    if [ ! -f "$variants_script" ]; then
        err "design-variants.mjs not found in $STAGE_DIR"
        exit 2
    fi
    node "$variants_script" \
        --baseline "$OUT/baseline.json" \
        --axes "$VARIANT_AXES" \
        > "$OUT/variants.json"
}

# ---------------------------------------------------------------------------
# Stage 4 — Preview
# ---------------------------------------------------------------------------
run_preview() {
    local preview_script="$STAGE_DIR/design-preview.mjs"
    if [ ! -f "$preview_script" ]; then
        code_health "harmonize_preview_failed" "design-preview.mjs not found in $STAGE_DIR"
        return 0
    fi
    if ! node "$preview_script" --variants "$OUT/variants.json" --out "$OUT"; then
        code_health "harmonize_preview_failed" "preview stage exited non-zero"
    fi
}

# ---------------------------------------------------------------------------
# Pick a default variant (non-interactive; pick gate lives in SKILL.md)
# ---------------------------------------------------------------------------
pick_default_variant() {
    # Use jq to pick first non-baseline variant; fall back to baseline
    if command -v jq >/dev/null 2>&1; then
        local picked
        picked="$(jq -r '[.[] | select(.id != "baseline")] | first | .id // "baseline"' "$OUT/variants.json" 2>/dev/null || true)"
        if [ -z "$picked" ] || [ "$picked" = "null" ]; then
            picked="baseline"
        fi
        printf '%s' "$picked"
    else
        printf 'baseline'
    fi
}

# ---------------------------------------------------------------------------
# Stage 6 — Gen migration spec
# ---------------------------------------------------------------------------
run_gen_spec() {
    local spec_script="$STAGE_DIR/gen-migration-spec.mjs"
    if [ ! -f "$spec_script" ]; then
        code_health "harmonize_spec_failed" "gen-migration-spec.mjs not found in $STAGE_DIR"
        return 0
    fi

    local picked_id
    picked_id="$(pick_default_variant)"

    local today
    today="$(date +%Y-%m-%d 2>/dev/null || printf '2026-01-01')"

    # Derive slug from root dir basename, sanitized
    local slug
    slug="$(basename "$ROOT" | tr '[:upper:]' '[:lower:]' | tr -cs 'a-z0-9' '-' | sed 's/^-//;s/-$//')"
    if [ -z "$slug" ]; then
        slug="app"
    fi

    if ! node "$spec_script" \
        --variant "$picked_id" \
        --tokens "$OUT/variants.json" \
        --inventory "$OUT/inventory.md" \
        --date "$today" \
        --slug "$slug" \
        --out "$OUT"; then
        code_health "harmonize_spec_failed" "gen-migration-spec stage exited non-zero"
    fi
}

# ---------------------------------------------------------------------------
# Empty-profile guard
# ---------------------------------------------------------------------------
check_nonempty_profile() {
    if [ ! -f "$OUT/discovered-tokens.json" ]; then
        code_health "harmonize_discover_empty" "no design tokens found in $ROOT"
        exit 1
    fi
    if command -v jq >/dev/null 2>&1; then
        local palette_len type_scale_len spacing_len
        palette_len="$(jq '.palette | length' "$OUT/discovered-tokens.json" 2>/dev/null || printf '0')"
        type_scale_len="$(jq '.type_scale | length' "$OUT/discovered-tokens.json" 2>/dev/null || printf '0')"
        spacing_len="$(jq '.spacing | length' "$OUT/discovered-tokens.json" 2>/dev/null || printf '0')"
        if [ "$palette_len" -eq 0 ] && [ "$type_scale_len" -eq 0 ] && [ "$spacing_len" -eq 0 ]; then
            code_health "harmonize_discover_empty" "no design tokens found in $ROOT"
            exit 1
        fi
    fi
}

# ---------------------------------------------------------------------------
# Main pipeline
# ---------------------------------------------------------------------------
mkdir -p "$OUT"

info "harmonize: stage 1 — discover"
if ! run_discover; then
    code_health "harmonize_discover_failed" "discover stage exited non-zero"
    exit 1
fi
check_nonempty_profile

info "harmonize: stage 2 — generalize"
if ! run_generalize; then
    code_health "harmonize_generalize_failed" "generalize stage exited non-zero"
    exit 3
fi

info "harmonize: stage 3 — variants"
if ! run_variants; then
    code_health "harmonize_variants_failed" "variants stage exited non-zero"
    exit 3
fi

info "harmonize: stage 4 — preview"
run_preview

info "harmonize: stage 6 — gen-migration-spec (default pick)"
run_gen_spec

info "harmonize: done — artifacts in $OUT"
exit 0
