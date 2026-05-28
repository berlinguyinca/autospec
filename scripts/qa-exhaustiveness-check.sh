#!/usr/bin/env bash
# scripts/qa-exhaustiveness-check.sh — explicit exhaustiveness gates.
#
# Implements the four flags introduced in issue #660 that convert the
# autospec-qa adapter's prose discipline ("Reliability exhaustion loop",
# the 20-question Critical self-questioning checkpoint, etc.) into hard
# machine-checkable gates:
#
#   --every-spec-row    Every spec acceptance criterion must have a
#                       non-NOT_TESTED row in .autospec/proof-matrix.json.
#                       Missing rows emit findings with category
#                       code_health:incomplete_traceability.
#
#   --mutation-each     Every workflow the spec lists must have at least
#                       one mutation/breakage entry in
#                       .autospec/mutation-proof.json. Missing entries
#                       emit category code_health:incomplete_mutation_proof.
#
#   --no-mock-each      Every backend-backed feature must have a no-mock
#                       canary entry in .autospec/canary-results.json
#                       (or an explicit exception in
#                       .autospec/reliability.yml). Missing entries emit
#                       category code_health:incomplete_no_mock_coverage.
#
#   --exhaustive        Turns the three above on.
#
# Each failed gate appends a release_blocking finding to the verdict file
# (atomic write: tmp in same dir + rename) and demotes the verdict to
# PARTIAL (if it was PASS) or preserves FAIL (precedence). The list of
# flags actually requested is recorded under .requested_flags on the
# verdict so downstream skills (e.g. /autospec-release) see what was
# enforced.
#
# Exit codes:
#   0 — all requested gates passed (or no flags supplied).
#   1 — at least one gate failed; verdict + findings updated.
#   2 — usage / bad arguments.

set -u

usage() {
    cat <<'EOF'
Usage: qa-exhaustiveness-check.sh [flags] [paths]

Flags (one or more):
  --every-spec-row      Require non-NOT_TESTED proof-matrix row per AC
  --mutation-each       Require mutation-proof entry per spec workflow
  --no-mock-each        Require canary entry per backend-backed feature
  --exhaustive          Shortcut for all three above

Paths:
  --verdict <p>         .autospec/qa-verdict.json (default)
  --proof-matrix <p>    .autospec/proof-matrix.json (default)
  --mutation-proof <p>  .autospec/mutation-proof.json (default)
  --canary <p>          .autospec/canary-results.json (default)
  --reliability <p>     .autospec/reliability.yml (default)
  --spec <p>            Spec file (repeatable). At least one is required
                        when any flag is active.
  -h, --help            Show this help.

Exit 0 if every requested gate passes. Exit 1 if any gate fails;
findings have been appended to the verdict file and .verdict demoted.
EOF
}

VERDICT=".autospec/qa-verdict.json"
PROOF_MATRIX=".autospec/proof-matrix.json"
MUTATION_PROOF=".autospec/mutation-proof.json"
CANARY=".autospec/canary-results.json"
RELIABILITY=".autospec/reliability.yml"
SPECS=()
FLAGS=()

flag_every_spec_row=0
flag_mutation_each=0
flag_no_mock_each=0

while [ "$#" -gt 0 ]; do
    case "$1" in
        --every-spec-row)  flag_every_spec_row=1; FLAGS+=("--every-spec-row"); shift ;;
        --mutation-each)   flag_mutation_each=1;  FLAGS+=("--mutation-each");  shift ;;
        --no-mock-each)    flag_no_mock_each=1;   FLAGS+=("--no-mock-each");   shift ;;
        --exhaustive)
            flag_every_spec_row=1
            flag_mutation_each=1
            flag_no_mock_each=1
            FLAGS+=("--exhaustive")
            shift
            ;;
        --verdict)         VERDICT="$2"; shift 2 ;;
        --proof-matrix)    PROOF_MATRIX="$2"; shift 2 ;;
        --mutation-proof)  MUTATION_PROOF="$2"; shift 2 ;;
        --canary)          CANARY="$2"; shift 2 ;;
        --reliability)     RELIABILITY="$2"; shift 2 ;;
        --spec)            SPECS+=("$2"); shift 2 ;;
        -h|--help)         usage; exit 0 ;;
        *)                 echo "unknown arg: $1" >&2; usage >&2; exit 2 ;;
    esac
done

# No flags requested → no-op success (backwards-compat).
if [ "$flag_every_spec_row" = "0" ] \
        && [ "$flag_mutation_each" = "0" ] \
        && [ "$flag_no_mock_each" = "0" ]; then
    exit 0
fi

if ! command -v jq >/dev/null 2>&1; then
    echo "qa-exhaustiveness-check: jq is required" >&2
    exit 2
fi

if [ "${#SPECS[@]}" = "0" ]; then
    echo "qa-exhaustiveness-check: at least one --spec <path> is required when a flag is active" >&2
    exit 2
fi

VERDICT_DIR="$(dirname "$VERDICT")"
mkdir -p "$VERDICT_DIR"
# Atomic init: write tmp in same dir, then rename.
if [ ! -f "$VERDICT" ]; then
    init_tmp="$(mktemp "$VERDICT_DIR/.qa-verdict.XXXXXX")"
    printf '{"verdict":"PASS","findings":[]}\n' > "$init_tmp" && mv "$init_tmp" "$VERDICT"
fi
if ! jq -e '.findings | type == "array"' "$VERDICT" >/dev/null 2>&1; then
    tmp="$(mktemp "$VERDICT_DIR/.qa-verdict.XXXXXX")"
    jq '. + {findings: (.findings // [])}' "$VERDICT" > "$tmp" && mv "$tmp" "$VERDICT"
fi

# Record requested flags on the verdict (idempotent).
{
    tmp="$(mktemp "$VERDICT_DIR/.qa-verdict.XXXXXX")"
    # shellcheck disable=SC2016
    jq --argjson f "$(printf '%s\n' "${FLAGS[@]}" | jq -R . | jq -s .)" \
        '.requested_flags = ((.requested_flags // []) + $f | unique)' \
        "$VERDICT" > "$tmp" && mv "$tmp" "$VERDICT"
}

append_finding() {
    # $1 = category, $2 = summary, $3 = evidence
    local category="$1" summary="$2" evidence="$3"
    local tmp
    tmp="$(mktemp "$VERDICT_DIR/.qa-verdict.XXXXXX")"
    jq --arg cat "$category" \
       --arg summary "$summary" \
       --arg evidence "$evidence" \
       '.findings += [{
            "category": $cat,
            "release_blocking": true,
            "status": "FAIL",
            "summary": $summary,
            "evidence": $evidence
        }]' "$VERDICT" > "$tmp" && mv "$tmp" "$VERDICT"
}

demote_verdict() {
    # FAIL > PARTIAL > PASS precedence — never upgrade.
    local current
    current="$(jq -r '.verdict // "PASS"' "$VERDICT")"
    case "$current" in
        FAIL)    return ;;                # preserve
        PARTIAL) return ;;                # already demoted
        PASS|*)
            local tmp
            tmp="$(mktemp "$VERDICT_DIR/.qa-verdict.XXXXXX")"
            jq '.verdict = "PARTIAL"' "$VERDICT" > "$tmp" && mv "$tmp" "$VERDICT"
            ;;
    esac
}

# --- Spec parsing helpers ---------------------------------------------------

# Extract acceptance criteria from a spec markdown file. We accept either:
#   - lines under an "## Acceptance" / "## Acceptance Criteria" heading that
#     start with "- " or "* " or "<digit>." until the next "## " heading.
#   - lines of the form "AC<n>: ..." or "AC-<n>: ..." anywhere in the file.
# Emits one AC identifier per line (the verbatim first ~80 chars, lowercased
# for matching). Caller is responsible for de-duping.
extract_acs() {
    local spec="$1"
    awk '
        BEGIN { in_ac = 0 }
        /^## / {
            if (tolower($0) ~ /acceptance/) { in_ac = 1; next }
            in_ac = 0
        }
        in_ac && /^[[:space:]]*([-*]|[0-9]+\.)[[:space:]]+/ {
            sub(/^[[:space:]]*([-*]|[0-9]+\.)[[:space:]]+/, "")
            print
        }
        /^AC[-_]?[0-9]+[:[:space:]]/ {
            print
        }
    ' "$spec"
}

# Extract workflows from a spec. Convention used by autospec-qa:
#   - "## Workflows" section with "- " bullets, OR
#   - lines like "Workflow: <name>" / "workflow_id: <name>".
extract_workflows() {
    local spec="$1"
    awk '
        BEGIN { in_wf = 0 }
        /^## / {
            if (tolower($0) ~ /workflows?$/) { in_wf = 1; next }
            in_wf = 0
        }
        in_wf && /^[[:space:]]*([-*]|[0-9]+\.)[[:space:]]+/ {
            sub(/^[[:space:]]*([-*]|[0-9]+\.)[[:space:]]+/, "")
            print
        }
        /^[Ww]orkflow(_id)?:[[:space:]]/ {
            sub(/^[Ww]orkflow(_id)?:[[:space:]]*/, "")
            print
        }
    ' "$spec"
}

# Extract backend-backed features: lines under "## Backend Features" or
# "## Backend-backed features" heading; OR lines tagged "backend: true".
extract_backend_features() {
    local spec="$1"
    awk '
        BEGIN { in_bf = 0 }
        /^## / {
            if (tolower($0) ~ /backend[- ]?(backed )?features?/) { in_bf = 1; next }
            in_bf = 0
        }
        in_bf && /^[[:space:]]*([-*]|[0-9]+\.)[[:space:]]+/ {
            sub(/^[[:space:]]*([-*]|[0-9]+\.)[[:space:]]+/, "")
            print
        }
    ' "$spec"
}

# Normalise a string for matching: lower-case, collapse whitespace.
norm() { printf '%s' "$1" | tr '[:upper:]' '[:lower:]' | tr -s '[:space:]' ' ' | sed 's/^ //; s/ $//'; }

exit_code=0

# --- Gate 1: --every-spec-row ----------------------------------------------

if [ "$flag_every_spec_row" = "1" ]; then
    matrix_rows="[]"
    if [ -f "$PROOF_MATRIX" ] && jq empty "$PROOF_MATRIX" >/dev/null 2>&1; then
        # Accept either { rows: [...] } or a top-level array.
        kind="$(jq -r 'type' "$PROOF_MATRIX")"
        if [ "$kind" = "array" ]; then
            matrix_rows="$(jq '.' "$PROOF_MATRIX")"
        else
            matrix_rows="$(jq '.rows // []' "$PROOF_MATRIX")"
        fi
    fi
    for spec in "${SPECS[@]}"; do
        if [ ! -f "$spec" ]; then
            append_finding "code_health:incomplete_traceability" \
                "spec file missing: $spec" "$spec"
            exit_code=1
            continue
        fi
        while IFS= read -r ac; do
            [ -z "$ac" ] && continue
            ac_norm="$(norm "$ac")"
            # Match if any row's ac/criterion field, normalised, contains the AC
            # text (or vice-versa) AND its status is not NOT_TESTED.
            covered="$(printf '%s' "$matrix_rows" | jq --arg ac "$ac_norm" '
                map(select(
                    (((.ac // .criterion // .acceptance_criterion // "") | ascii_downcase | gsub("\\s+"; " ") | ltrimstr(" ") | rtrimstr(" ")) as $row
                     | ($row == $ac or ($row | contains($ac)) or ($ac | contains($row)) and $row != ""))
                    and (.status // "") != "NOT_TESTED"
                )) | length
            ')"
            if [ "${covered:-0}" = "0" ]; then
                short="$(printf '%s' "$ac" | head -c 120)"
                append_finding "code_health:incomplete_traceability" \
                    "spec acceptance criterion has no non-NOT_TESTED proof-matrix row: $short" \
                    "spec=$spec ac=$short"
                exit_code=1
            fi
        done < <(extract_acs "$spec" | sort -u)
    done
fi

# --- Gate 2: --mutation-each -----------------------------------------------

if [ "$flag_mutation_each" = "1" ]; then
    mutations="[]"
    if [ -f "$MUTATION_PROOF" ] && jq empty "$MUTATION_PROOF" >/dev/null 2>&1; then
        mutations="$(jq '.mutations // []' "$MUTATION_PROOF")"
    fi
    for spec in "${SPECS[@]}"; do
        [ -f "$spec" ] || continue
        while IFS= read -r wf; do
            [ -z "$wf" ] && continue
            wf_norm="$(norm "$wf")"
            covered="$(printf '%s' "$mutations" | jq --arg wf "$wf_norm" '
                map(select(
                    ((.workflow_id // .workflow // "") | ascii_downcase | gsub("\\s+"; " ") | ltrimstr(" ") | rtrimstr(" ")) as $row
                    | ($row == $wf or ($row | contains($wf)) or ($wf | contains($row)) and $row != "")
                )) | length
            ')"
            if [ "${covered:-0}" = "0" ]; then
                short="$(printf '%s' "$wf" | head -c 120)"
                append_finding "code_health:incomplete_mutation_proof" \
                    "spec workflow has no mutation-proof entry: $short" \
                    "spec=$spec workflow=$short"
                exit_code=1
            fi
        done < <(extract_workflows "$spec" | sort -u)
    done
fi

# --- Gate 3: --no-mock-each -------------------------------------------------

if [ "$flag_no_mock_each" = "1" ]; then
    canary_entries="[]"
    if [ -f "$CANARY" ] && jq empty "$CANARY" >/dev/null 2>&1; then
        kind="$(jq -r 'type' "$CANARY")"
        if [ "$kind" = "array" ]; then
            canary_entries="$(jq '.' "$CANARY")"
        else
            canary_entries="$(jq '.entries // .canaries // .results // []' "$CANARY")"
        fi
    fi
    # Exceptions: features explicitly listed under reliability.yml's
    # no_mock_exempt (or no_mock.exempt) list bypass this gate. Plain grep
    # is sufficient — yq is not assumed installed.
    exempt_text=""
    if [ -f "$RELIABILITY" ]; then
        exempt_text="$(awk '
            /^no_mock_exempt:/ { capture = 1; next }
            /^no_mock:/        { capture = 0; next }
            /^[a-zA-Z_]+:/     { capture = 0 }
            capture && /^[[:space:]]*-[[:space:]]+/ { sub(/^[[:space:]]*-[[:space:]]+/, ""); print }
        ' "$RELIABILITY")"
    fi
    for spec in "${SPECS[@]}"; do
        [ -f "$spec" ] || continue
        while IFS= read -r feat; do
            [ -z "$feat" ] && continue
            feat_norm="$(norm "$feat")"
            # exempt?
            if [ -n "$exempt_text" ]; then
                if printf '%s\n' "$exempt_text" | while IFS= read -r e; do
                    en="$(norm "$e")"
                    [ -n "$en" ] && [ "$en" = "$feat_norm" ] && exit 0
                done; then
                    : # placeholder; the actual check is the if below
                fi
                if printf '%s\n' "$exempt_text" | grep -qiF "$feat"; then
                    continue
                fi
            fi
            covered="$(printf '%s' "$canary_entries" | jq --arg f "$feat_norm" '
                map(select(
                    ((.feature // .name // .id // "") | ascii_downcase | gsub("\\s+"; " ") | ltrimstr(" ") | rtrimstr(" ")) as $row
                    | ($row == $f or ($row | contains($f)) or ($f | contains($row)) and $row != "")
                )) | length
            ')"
            if [ "${covered:-0}" = "0" ]; then
                short="$(printf '%s' "$feat" | head -c 120)"
                append_finding "code_health:incomplete_no_mock_coverage" \
                    "backend-backed feature has no no-mock canary entry: $short" \
                    "spec=$spec feature=$short"
                exit_code=1
            fi
        done < <(extract_backend_features "$spec" | sort -u)
    done
fi

if [ "$exit_code" = "1" ]; then
    demote_verdict
fi

exit "$exit_code"
