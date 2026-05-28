#!/usr/bin/env bash
# scripts/qa-incident-check.sh — production-incident regression registry gate.
#
# For each `active` incident in .autospec/production-incidents.json (or the
# path passed via --registry), verify that the four release-blocking checks
# pass:
#
#   1. regression_test path exists on disk.
#   2. The test actually runs (we treat presence + non-empty body as the
#      minimum runnable signal; CI is responsible for executing the suite —
#      this gate only refuses to ship if the test is missing/empty/stub).
#   3. The test is non-trivially assertive. Reject bodies that are empty
#      or contain only `expect(true).toBe(true)` / `assert True` /
#      `assert(true)` / `pass` stubs without any other assertion.
#   4. The recorded mutation has been proven (a) by a matching
#      `.autospec/mutation-proof.json` entry whose `test.path` matches the
#      regression_test AND `observed_result` is `failed_as_expected`, OR
#      (b) by an inline `mutation_proof` block on the incident itself with
#      the same shape. Absence of either is a finding.
#
# `superseded` incidents are skipped (logged). `wont_test` incidents emit a
# warning finding (release_blocking: false) so they remain visible.
#
# Findings are written into the verdict file under `.findings[]` with
# category `code_health:production_incident_regression_missing`. The verdict
# is updated atomically (tmp + mv).
#
# Exit codes:
#   0 — every active incident passes the four checks (or registry empty).
#   1 — at least one active incident failed; release_blocking finding written.
#   2 — usage / bad arguments / malformed registry.

set -u

usage() {
    cat <<'EOF'
Usage: qa-incident-check.sh --registry <path> --verdict <path>

Options:
  --registry <path>   Path to .autospec/production-incidents.json
                      (default: .autospec/production-incidents.json)
  --verdict  <path>   Path to .autospec/qa-verdict.json to append findings to
                      (default: .autospec/qa-verdict.json). The verdict file
                      will be created if missing.
  --mutation-proof <path>
                      Path to .autospec/mutation-proof.json (default:
                      .autospec/mutation-proof.json). Used to satisfy check #4.
  --repo-root <path>  Repository root used to resolve regression_test paths
                      (default: PWD).
  -h, --help          Show this help.

Exit 0 if every `active` incident is covered. Exit 1 if any active incident
is missing coverage; findings have been appended to the verdict file.
EOF
}

REGISTRY=".autospec/production-incidents.json"
VERDICT=".autospec/qa-verdict.json"
MUTATION_PROOF=".autospec/mutation-proof.json"
REPO_ROOT="${PWD}"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --registry)        REGISTRY="$2"; shift 2 ;;
        --verdict)         VERDICT="$2"; shift 2 ;;
        --mutation-proof)  MUTATION_PROOF="$2"; shift 2 ;;
        --repo-root)       REPO_ROOT="$2"; shift 2 ;;
        -h|--help)         usage; exit 0 ;;
        *)                 echo "unknown arg: $1" >&2; usage >&2; exit 2 ;;
    esac
done

if ! command -v jq >/dev/null 2>&1; then
    echo "qa-incident-check: jq is required" >&2
    exit 2
fi

# Empty/missing registry is a no-op success.
if [ ! -f "$REGISTRY" ]; then
    exit 0
fi

if ! jq empty "$REGISTRY" >/dev/null 2>&1; then
    echo "qa-incident-check: $REGISTRY is not valid JSON" >&2
    exit 2
fi

registry_kind="$(jq -r 'type' "$REGISTRY")"
if [ "$registry_kind" != "array" ]; then
    echo "qa-incident-check: $REGISTRY must be a JSON array" >&2
    exit 2
fi

count="$(jq 'length' "$REGISTRY")"
if [ "$count" = "0" ]; then
    exit 0
fi

mkdir -p "$(dirname "$VERDICT")"
if [ ! -f "$VERDICT" ]; then
    printf '{"findings": []}\n' > "$VERDICT"
fi

# Ensure findings array exists.
if ! jq -e '.findings | type == "array"' "$VERDICT" >/dev/null 2>&1; then
    tmp="$(mktemp)"
    jq '. + {findings: (.findings // [])}' "$VERDICT" > "$tmp" && mv "$tmp" "$VERDICT"
fi

append_finding() {
    # $1 = release_blocking (true/false), $2 = summary, $3 = evidence, $4 = id
    local blocking="$1" summary="$2" evidence="$3" inc_id="$4"
    local tmp
    tmp="$(mktemp)"
    jq --argjson blocking "$blocking" \
       --arg summary "$summary" \
       --arg evidence "$evidence" \
       --arg incident_id "$inc_id" \
       '.findings += [{
            "category": "code_health:production_incident_regression_missing",
            "release_blocking": $blocking,
            "status": "FAIL",
            "summary": $summary,
            "evidence": $evidence,
            "incident_id": $incident_id
        }]' "$VERDICT" > "$tmp" && mv "$tmp" "$VERDICT"
}

is_trivial_test_body() {
    # Reads stdin (test file body). Exit 0 if trivial/tautological.
    # Trivial when the body has NO substantive assertion line — only one of:
    #   expect(true).toBe(true)
    #   assert(true) / assert true / assert True
    #   pass        (Python no-op)
    #   empty body
    # Detect by counting "assertion-like" lines that are NOT trivial patterns.
    local body
    body="$(cat)"
    # Strip comments + blank lines for scanning.
    local stripped
    stripped="$(printf '%s\n' "$body" \
        | sed -e 's|//.*$||' -e 's|#.*$||' \
        | grep -v '^[[:space:]]*$' || true)"
    if [ -z "$stripped" ]; then
        return 0
    fi
    # Tautological patterns. ERE: ( ) are groups, so we escape literal parens.
    local taut='expect\(true\)\.toBe\(true\)|expect\(1\)\.toBe\(1\)|assert[[:space:]]+True|assert[[:space:]]*\([[:space:]]*true|assert[[:space:]]*\([[:space:]]*1|^[[:space:]]*pass[[:space:]]*$'
    # Real assertion patterns we treat as non-trivial.
    local real='expect\(|assert[[:space:]]|assert\(|assertEqual|assertTrue|assertFalse|assertRaises|assertThat|should\.|to\.equal|toEqual|toBe\(|toMatch|toThrow|toHaveBeen'
    local real_count taut_count
    real_count="$(printf '%s\n' "$stripped" | grep -E "$real" | grep -v -E "$taut" | wc -l | tr -d ' ')"
    taut_count="$(printf '%s\n' "$stripped" | grep -E "$taut" | wc -l | tr -d ' ')"
    if [ "$real_count" -ge 1 ]; then
        return 1  # non-trivial
    fi
    if [ "$taut_count" -ge 1 ]; then
        return 0  # trivial
    fi
    # No assertions at all → trivial/empty for our purposes.
    return 0
}

mutation_proven() {
    # $1 = regression_test relative path
    local rt="$1"
    if [ ! -f "$MUTATION_PROOF" ]; then
        return 1
    fi
    if ! jq empty "$MUTATION_PROOF" >/dev/null 2>&1; then
        return 1
    fi
    local hit
    hit="$(jq --arg rt "$rt" -r '
        (.mutations // [])
        | map(select(
            (.test.path // "") == $rt
            and (.observed_result // "") == "failed_as_expected"
          ))
        | length
    ' "$MUTATION_PROOF" 2>/dev/null || echo 0)"
    [ "${hit:-0}" -ge 1 ]
}

inline_mutation_proven() {
    # $1 = registry incident index
    local idx="$1"
    local result
    result="$(jq -r --argjson i "$idx" '.[$i].mutation_proof.observed_result // ""' "$REGISTRY" 2>/dev/null)"
    [ "$result" = "failed_as_expected" ]
}

exit_code=0
i=0
while [ "$i" -lt "$count" ]; do
    inc_id="$(jq -r --argjson i "$i" '.[$i].id // ""' "$REGISTRY")"
    status="$(jq -r --argjson i "$i" '.[$i].status // ""' "$REGISTRY")"
    rt="$(jq -r --argjson i "$i" '.[$i].regression_test // ""' "$REGISTRY")"

    case "$status" in
        superseded)
            i=$((i + 1)); continue
            ;;
        wont_test)
            append_finding "false" \
                "wont_test incident $inc_id retained for audit" \
                "registry:$REGISTRY status=wont_test" \
                "$inc_id"
            i=$((i + 1)); continue
            ;;
        active) : ;;
        *)
            append_finding "true" \
                "incident $inc_id has unknown status '$status'" \
                "registry:$REGISTRY" \
                "$inc_id"
            exit_code=1
            i=$((i + 1)); continue
            ;;
    esac

    rt_abs="$REPO_ROOT/$rt"

    # Check 1 + 2: file present and non-empty (runnable).
    if [ -z "$rt" ] || [ ! -f "$rt_abs" ]; then
        append_finding "true" \
            "incident $inc_id: regression_test missing ($rt)" \
            "expected file: $rt" \
            "$inc_id"
        exit_code=1
        i=$((i + 1)); continue
    fi
    if [ ! -s "$rt_abs" ]; then
        append_finding "true" \
            "incident $inc_id: regression_test is empty ($rt)" \
            "$rt" \
            "$inc_id"
        exit_code=1
        i=$((i + 1)); continue
    fi

    # Check 3: non-trivially assertive.
    if is_trivial_test_body < "$rt_abs"; then
        append_finding "true" \
            "incident $inc_id: regression_test is not non-trivially assertive ($rt)" \
            "$rt" \
            "$inc_id"
        exit_code=1
        i=$((i + 1)); continue
    fi

    # Check 4: mutation proof present.
    if ! mutation_proven "$rt" && ! inline_mutation_proven "$i"; then
        append_finding "true" \
            "incident $inc_id: mutation proof missing for $rt" \
            "expected mutation entry with observed_result=failed_as_expected" \
            "$inc_id"
        exit_code=1
        i=$((i + 1)); continue
    fi

    i=$((i + 1))
done

exit "$exit_code"
