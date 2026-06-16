#!/usr/bin/env bash
# scripts/gen-explore-round-spec.sh — deterministic round-spec renderer.
#
# Turns the explore aggregator's ranked-proposals JSON into a round design spec
# markdown.  One ## <title> section per proposal, carrying evidence,
# estimated_complexity, source, severity, named_consumer, and an Acceptance
# block of - [ ] checkboxes so spec-vs-code can later parse it.
#
# Usage:
#   gen-explore-round-spec.sh <proposals-json> [--round N] [--branch <slug>]
#                             [--out <path>]
#
# Output:
#   docs/specs/<date>-explore-<slug>-round-<N>-design.md style markdown,
#   written to --out <path> if given, else stdout.
#
# Exit codes:
#   0 — success
#   1 — missing input file or argument error

set -u

# ── argument parsing ──────────────────────────────────────────────────────────

PROPOSALS_JSON=""
ROUND="1"
BRANCH="main"
OUT_PATH=""

while [ $# -gt 0 ]; do
    case "$1" in
        --round)
            if [ -z "${2:-}" ]; then
                printf 'gen-explore-round-spec.sh: --round requires a value\n' >&2
                exit 1
            fi
            ROUND="$2"
            shift 2
            ;;
        --branch)
            if [ -z "${2:-}" ]; then
                printf 'gen-explore-round-spec.sh: --branch requires a value\n' >&2
                exit 1
            fi
            BRANCH="$2"
            shift 2
            ;;
        --out)
            if [ -z "${2:-}" ]; then
                printf 'gen-explore-round-spec.sh: --out requires a path argument\n' >&2
                exit 1
            fi
            OUT_PATH="$2"
            shift 2
            ;;
        --help|-h)
            cat <<'EOF'
gen-explore-round-spec.sh — deterministic round-spec renderer

Usage:
  scripts/gen-explore-round-spec.sh <proposals-json> [--round N]
      [--branch <slug>] [--out <path>]

Arguments:
  <proposals-json>   Path to the ranked-proposals JSON from explore-research-cycle.sh.
  --round N          Round number (default: 1).
  --branch <slug>    Sandbox branch name (default: main).
  --out <path>       Write output to this path instead of stdout.
EOF
            exit 0
            ;;
        -*)
            printf 'gen-explore-round-spec.sh: unknown option: %s\n' "$1" >&2
            exit 1
            ;;
        *)
            if [ -z "$PROPOSALS_JSON" ]; then
                PROPOSALS_JSON="$1"
            else
                printf 'gen-explore-round-spec.sh: unexpected argument: %s\n' "$1" >&2
                exit 1
            fi
            shift
            ;;
    esac
done

if [ -z "$PROPOSALS_JSON" ]; then
    printf 'gen-explore-round-spec.sh: <proposals-json> argument required\n' >&2
    exit 1
fi

if [ ! -f "$PROPOSALS_JSON" ]; then
    printf 'gen-explore-round-spec.sh: file not found: %s\n' "$PROPOSALS_JSON" >&2
    exit 1
fi

# ── validate jq availability ──────────────────────────────────────────────────

if ! command -v jq >/dev/null 2>&1; then
    printf 'gen-explore-round-spec.sh: jq is required but not found\n' >&2
    exit 1
fi

# ── render ─────────────────────────────────────────────────────────────────────

TODAY="$(date +%Y-%m-%d)"

render_spec() {
    # Header
    printf '# %s round %s\n\n' "$BRANCH" "$ROUND"
    printf '**Date:** %s  \n' "$TODAY"
    printf '**Branch:** `%s`  \n' "$BRANCH"
    printf '**Round:** %s  \n\n' "$ROUND"

    printf '**Summary:** Explore round %s for sandbox branch `%s`. ' "$ROUND" "$BRANCH"
    printf 'Each section below corresponds to one ranked proposal from the research cycle.\n\n'

    # Count proposals
    local count
    count="$(jq '.proposals | length' "$PROPOSALS_JSON")"

    if [ "$count" -eq 0 ]; then
        printf '_No proposals for this round._\n'
        return 0
    fi

    # One section per proposal
    local i=0
    while [ "$i" -lt "$count" ]; do
        local title evidence complexity source severity named_consumer

        title="$(jq -r ".proposals[$i].title // \"(untitled)\"" "$PROPOSALS_JSON")"
        evidence="$(jq -r ".proposals[$i].evidence // \"\"" "$PROPOSALS_JSON")"
        complexity="$(jq -r ".proposals[$i].estimated_complexity // \"unknown\"" "$PROPOSALS_JSON")"
        source="$(jq -r ".proposals[$i].source // \"unknown\"" "$PROPOSALS_JSON")"
        severity="$(jq -r ".proposals[$i].severity // \"feature\"" "$PROPOSALS_JSON")"
        named_consumer="$(jq -r ".proposals[$i].named_consumer // \"\"" "$PROPOSALS_JSON")"

        printf '## %s\n\n' "$title"
        printf '| Field | Value |\n'
        printf '|---|---|\n'
        printf '| **Evidence** | `%s` |\n' "$evidence"
        printf '| **Complexity** | %s |\n' "$complexity"
        printf '| **Source** | %s |\n' "$source"
        printf '| **Severity** | %s |\n' "$severity"
        printf '| **Named consumer** | %s |\n' "$named_consumer"
        printf '\n'

        printf '### Acceptance\n\n'
        printf -- '- [ ] Implement: %s\n' "$title"
        printf -- '- [ ] Tests pass for the changed files.\n'
        printf -- '- [ ] `bash scripts/validate.sh` exits 0.\n'
        printf '\n'

        i=$((i + 1))
    done
}

# ── emit output ───────────────────────────────────────────────────────────────

if [ -n "$OUT_PATH" ]; then
    render_spec > "$OUT_PATH"
else
    render_spec
fi
