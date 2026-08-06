#!/usr/bin/env bash
# scripts/dogfood-detectors.sh — issue #641
#
# Runs every heuristic detector autospec ships against the autospec repo
# itself, compares each detector's findings to a committed allowlist of
# intentional exceptions, and fails on regression.
#
# Detectors discovered:
#   1. All scripts/qa-*-sweep.sh (auto-discovered by filename pattern).
#   2. Any extra detector path registered in .autospec/dogfood.yml under
#      the top-level `detectors:` list (one path per line).
#
# Each detector is invoked with:
#   REPO_DIR=$(pwd)           — repo to scan
#   VERDICT_FILE=<tempfile>   — JSON-Lines findings sink
#   PATH=<stub>:$PATH         — `gh` stubbed to a no-op so detectors cannot
#                               file GitHub issues during the dogfood run.
#
# Each finding line in VERDICT_FILE is expected to be a single JSON object
# carrying at minimum `file`, `function`, `rule_id`. The allowlist file at
# tests/dogfood/allowlist/<detector-basename>.json is an array of
# {file, function, rule_id, justification} rows. Allowlist matching uses
# the (file, function, rule_id) tuple — justification is informational
# only.
#
# Exits 0 if every detector's finding set equals its allowlist; non-zero
# with a per-detector report otherwise.

set -eu

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

ALLOWLIST_DIR="tests/dogfood/allowlist"
CONFIG_FILE=".autospec/dogfood.yml"

# Build the detector list: glob + optional extras from config.
#
# Three sources contribute:
#   1. scripts/qa-*-sweep.sh — native VERDICT_FILE contract, auto-discovered.
#   2. .autospec/dogfood.yml `detectors:` — extra native-contract scripts.
#   3. .autospec/dogfood.yml `adapters:` — wrapper scripts that normalize
#      non-native detectors (per-PR args, non-JSON output) into the
#      VERDICT_FILE contract (issue #646). The `adapter:` path is what
#      the driver invokes; the `detector:` field is informational only.
discover_detectors() {
    # Sweep scripts always participate.
    for d in scripts/qa-*-sweep.sh; do
        [ -f "$d" ] || continue
        printf '%s\n' "$d"
    done
    # Extras from .autospec/dogfood.yml (best-effort YAML parse).
    if [ -f "$CONFIG_FILE" ]; then
        if command -v yq >/dev/null 2>&1; then
            yq -r '.detectors[]?' "$CONFIG_FILE" 2>/dev/null || true
            yq -r '.adapters[]?.adapter' "$CONFIG_FILE" 2>/dev/null || true
        else
            # Minimal awk fallback: collect lines under `detectors:` that
            # start with `- ` and strip the bullet; for `adapters:` collect
            # the `adapter: <path>` value from each list entry.
            awk '
                /^detectors:/ { mode="det"; next }
                /^adapters:/  { mode="adp"; next }
                mode=="det" && /^[^[:space:]-]/ { mode="" }
                mode=="adp" && /^[^[:space:]-]/ { mode="" }
                mode=="det" && /^[[:space:]]*-[[:space:]]+/ {
                    s=$0
                    sub(/^[[:space:]]*-[[:space:]]+/, "", s)
                    sub(/[[:space:]]*#.*$/, "", s)
                    print s
                    next
                }
                mode=="adp" && /^[[:space:]]*adapter:[[:space:]]+/ {
                    s=$0
                    sub(/^[[:space:]]*adapter:[[:space:]]+/, "", s)
                    sub(/[[:space:]]*#.*$/, "", s)
                    gsub(/[[:space:]]+$/, "", s)
                    print s
                }
            ' "$CONFIG_FILE"
        fi
    fi
}

# Create a temp PATH directory with a no-op `gh` so detectors cannot file
# real GitHub issues during dogfood.
make_gh_stub() {
    local stub_dir="$1"
    cat > "$stub_dir/gh" <<'STUB'
#!/usr/bin/env bash
# dogfood gh stub — no-op so detectors cannot file issues during self-scan.
exit 0
STUB
    chmod +x "$stub_dir/gh"
}

# Read a JSON-Lines verdict file and emit (file, function, rule_id) tuples
# one per line, tab-separated, sorted. Empty input → empty output.
verdict_tuples() {
    local verdict_file="$1"
    [ -s "$verdict_file" ] || return 0
    if command -v jq >/dev/null 2>&1; then
        jq -r 'select(.rule_id != null)
            | select((.file // "") | startswith(".claude/worktrees/") | not)
            | [.file, .function, .rule_id] | @tsv' \
            "$verdict_file" 2>/dev/null | LC_ALL=C sort -u
    else
        # Fallback: cheap regex extraction. Works because emit_finding writes
        # one flat JSON object per line with these exact keys.
        awk '
            {
                file=""; func=""; rid=""
                if (match($0, /"file":"[^"]*"/)) {
                    s = substr($0, RSTART+8, RLENGTH-9); file=s
                }
                if (match($0, /"function":"[^"]*"/)) {
                    s = substr($0, RSTART+12, RLENGTH-13); func=s
                }
                if (match($0, /"rule_id":"[^"]*"/)) {
                    s = substr($0, RSTART+11, RLENGTH-12); rid=s
                }
                if (rid != "" && file !~ /^\.claude\/worktrees\//) printf "%s\t%s\t%s\n", file, func, rid
            }
        ' "$verdict_file" | LC_ALL=C sort -u
    fi
}

# Read an allowlist JSON array and emit the same (file, function, rule_id)
# tuples. Missing or empty file → empty output.
allowlist_tuples() {
    local allowlist_file="$1"
    [ -f "$allowlist_file" ] || return 0
    if command -v jq >/dev/null 2>&1; then
        jq -r '.[] | [.file, .function, .rule_id] | @tsv' \
            "$allowlist_file" 2>/dev/null | LC_ALL=C sort -u
    else
        awk '
            {
                gsub(/\r/, "")
                if (match($0, /"file":[[:space:]]*"[^"]*"/)) {
                    s=substr($0, RSTART, RLENGTH); sub(/.*"file":[[:space:]]*"/, "", s); sub(/".*/, "", s); file=s
                }
                if (match($0, /"function":[[:space:]]*"[^"]*"/)) {
                    s=substr($0, RSTART, RLENGTH); sub(/.*"function":[[:space:]]*"/, "", s); sub(/".*/, "", s); func=s
                }
                if (match($0, /"rule_id":[[:space:]]*"[^"]*"/)) {
                    s=substr($0, RSTART, RLENGTH); sub(/.*"rule_id":[[:space:]]*"/, "", s); sub(/".*/, "", s); rid=s
                    printf "%s\t%s\t%s\n", file, func, rid
                    file=""; func=""; rid=""
                }
            }
        ' "$allowlist_file" | LC_ALL=C sort -u
    fi
}

run_one_detector() {
    local detector="$1"
    local detector_base
    detector_base="$(basename "$detector")"
    local detector_stem="${detector_base%.sh}"
    local allowlist_file="$ALLOWLIST_DIR/${detector_stem}.json"

    if [ ! -x "$detector" ] && [ ! -r "$detector" ]; then
        printf 'dogfood: %s findings=0 expected=0 status=FAIL (detector missing or not executable)\n' "$detector"
        return 1
    fi

    local workdir
    workdir="$(mktemp -d -t dogfood.XXXXXX)"
    local stub_dir="$workdir/bin"
    local verdict_file="$workdir/verdict.jsonl"
    mkdir -p "$stub_dir"
    : > "$verdict_file"
    make_gh_stub "$stub_dir"

    # Run the detector with gh stubbed. Capture stdout/stderr but don't
    # propagate detector exit code — by contract detectors emit findings
    # rather than failing on offenders.
    (
        PATH="$stub_dir:$PATH" \
        REPO_DIR="$REPO_ROOT" \
        VERDICT_FILE="$verdict_file" \
        bash "$detector" \
            > "$workdir/stdout.log" 2> "$workdir/stderr.log"
    ) || true

    local verdict_tuples_file="$workdir/verdict.tuples"
    local allow_tuples_file="$workdir/allow.tuples"
    # Sorted, because both consumers below require it and neither said so:
    #   * `diff -q` compares SEQUENCES, so an unchanged allowlist whose scan simply
    #     emitted the same tuples in a different order reported FAIL. An allowlist
    #     comparison is a SET comparison; detector output order is not a contract.
    #   * `comm` requires sorted input, in the SAME collation. Sorting with
    #     LC_ALL=C while comm compared in the ambient locale still tripped
    #     "not in sorted order", so both sides are pinned to C below.
    #   * `comm` requires sorted input and silently emits nonsense otherwise — it
    #     was printing "comm: file 1 is not in sorted order" onto the triage output,
    #     so the unexpected/missing lists a maintainer reads to fix a failure were
    #     themselves unreliable.
    verdict_tuples "$verdict_file"   | LC_ALL=C sort > "$verdict_tuples_file"
    allowlist_tuples "$allowlist_file" | LC_ALL=C sort > "$allow_tuples_file"

    local found expected
    found="$(wc -l < "$verdict_tuples_file" | tr -d ' ')"
    expected="$(wc -l < "$allow_tuples_file" | tr -d ' ')"

    if diff -q "$verdict_tuples_file" "$allow_tuples_file" >/dev/null 2>&1; then
        printf 'dogfood: %s findings=%s expected=%s status=PASS\n' "$detector_base" "$found" "$expected"
        rm -rf "$workdir"
        return 0
    fi

    printf 'dogfood: %s findings=%s expected=%s status=FAIL\n' "$detector_base" "$found" "$expected"
    printf '  allowlist: %s\n' "$allowlist_file"
    printf '  unexpected findings (in scan, not in allowlist):\n'
    LC_ALL=C comm -23 "$verdict_tuples_file" "$allow_tuples_file" | sed 's/^/    /'
    printf '  missing allowlisted entries (in allowlist, not in scan):\n'
    LC_ALL=C comm -13 "$verdict_tuples_file" "$allow_tuples_file" | sed 's/^/    /'
    # Surface stderr for triage.
    if [ -s "$workdir/stderr.log" ]; then
        printf '  detector stderr (head):\n'
        head -20 "$workdir/stderr.log" | sed 's/^/    /'
    fi
    rm -rf "$workdir"
    return 1
}

main() {
    local fail=0
    local ran=0
    local detectors
    detectors="$(discover_detectors)"
    if [ -z "$detectors" ]; then
        printf 'dogfood: no detectors discovered (scripts/qa-*-sweep.sh)\n' >&2
        return 1
    fi
    # shellcheck disable=SC2034
    while IFS= read -r d; do
        [ -n "$d" ] || continue
        ran=$((ran + 1))
        if ! run_one_detector "$d"; then
            fail=$((fail + 1))
        fi
    done <<EOF
$detectors
EOF
    if [ "$fail" -gt 0 ]; then
        printf 'dogfood: %s detector(s) failed out of %s\n' "$fail" "$ran" >&2
        return 1
    fi
    printf 'dogfood: all %s detector(s) passed\n' "$ran"
    return 0
}

main "$@"
