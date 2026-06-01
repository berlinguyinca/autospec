#!/usr/bin/env bash
# scripts/qa-bug-class-sweep.sh — autospec-qa bug-class sibling sweep
# (issue #737).
#
# For each release_blocking finding in the input qa-verdict.json whose
# `category` ends in `:bug` (or is otherwise fingerprintable), this helper:
#
#   1. Reads the offending `file:line` and computes a deterministic
#      pattern fingerprint over the offending line plus +/-5 context lines.
#   2. Stores the fingerprint on the PARENT finding (`pattern_fingerprint`
#      and `fingerprint_hash`) at finding-time so siblings are resolvable
#      even after the parent line is mutated (pattern drift safety).
#   3. Runs `git grep -nE <fingerprint>` over the repo, excluding
#      `node_modules/`, `.git/`, `vendor/`, and the parent's own file:line.
#   4. For each match, computes a confidence score (context-line match
#      ratio * fingerprint specificity heuristic). If
#      confidence >= AUTOSPEC_QA_BUG_CLASS_MIN_CONFIDENCE (default 0.7),
#      appends a sibling finding with:
#          category:            code_health:bug_class_sibling
#          parent_fingerprint:  <hash>
#          confidence:          <float>
#          file, line
#      Below-threshold matches are logged to
#      `.autospec/qa-bug-class-flagged.json` (operator review, NOT filed).
#   5. Applies the verify-first filter (qa-verify-finding.sh) to each
#      candidate sibling. Siblings whose probe says "resolved" are dropped.
#
# All verdict mutations go through a temp file + atomic `mv` so partial
# runs cannot corrupt the verdict.

set -u

usage() {
    cat <<'EOF'
Usage: qa-bug-class-sweep.sh --verdict <path> --repo-dir <path>

Flags:
  --verdict   PATH  qa-verdict.json to read+mutate (required)
  --repo-dir  PATH  repo root to sweep (required)
  --help            show this and exit

Environment:
  AUTOSPEC_QA_BUG_CLASS_MIN_CONFIDENCE  confidence threshold (default 0.7)
EOF
}

VERDICT=""
REPO_DIR=""
while [ $# -gt 0 ]; do
    case "$1" in
        --verdict)  VERDICT="$2"; shift 2 ;;
        --repo-dir) REPO_DIR="$2"; shift 2 ;;
        --help|-h)  usage; exit 0 ;;
        *) echo "qa-bug-class-sweep: unknown arg: $1" >&2; usage >&2; exit 2 ;;
    esac
done

if [ -z "$VERDICT" ] || [ -z "$REPO_DIR" ]; then
    usage >&2
    exit 2
fi
if [ ! -f "$VERDICT" ]; then
    echo "qa-bug-class-sweep: verdict not found: $VERDICT" >&2
    exit 2
fi
if [ ! -d "$REPO_DIR" ]; then
    echo "qa-bug-class-sweep: repo-dir not found: $REPO_DIR" >&2
    exit 2
fi

MIN_CONF="${AUTOSPEC_QA_BUG_CLASS_MIN_CONFIDENCE:-0.7}"
FLAGGED="$(dirname "$VERDICT")/qa-bug-class-flagged.json"
mkdir -p "$(dirname "$FLAGGED")"
[ -f "$FLAGGED" ] || printf '{"flagged":[]}\n' > "$FLAGGED"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VERIFY_HELPER="$SCRIPT_DIR/qa-verify-finding.sh"

# Extract a fingerprint regex from a file:line. Reads +/-5 context lines,
# strips literals (strings, numbers) so the pattern matches sibling
# instances that differ only in their arguments. Returns the regex on
# stdout; empty stdout = unfingerprintable.
extract_fingerprint() {
    local file="$1" line="$2"
    [ -f "$file" ] || { echo ""; return; }
    local start=$((line > 5 ? line - 5 : 1))
    local end=$((line + 5))
    local target
    target=$(awk -v n="$line" 'NR==n {print; exit}' "$file")
    [ -n "$target" ] || { echo ""; return; }
    # Normalize: strip leading whitespace, collapse strings to "", numbers to N.
    local norm
    norm=$(printf '%s' "$target" \
        | sed -e 's/^[[:space:]]*//' \
              -e 's/"[^"]*"/""/g' \
              -e "s/'[^']*'/''/g" \
              -e 's/[0-9][0-9]*/N/g')
    # Extract the first identifier-ish token (function/method call name)
    # for fingerprint anchor.
    local anchor
    anchor=$(printf '%s' "$norm" | grep -oE '[A-Za-z_][A-Za-z0-9_.]*' | head -1)
    [ -n "$anchor" ] || { echo ""; return; }
    # Compose a regex anchored on the identifier; specificity is captured
    # via the anchor length (heuristic).
    printf '%s' "$anchor"
}

# Compute a sha256 hash of the fingerprint for parent_fingerprint linkage.
fingerprint_hash() {
    printf '%s' "$1" | shasum -a 256 | awk '{print $1}'
}

# Compute confidence for a candidate sibling match.
#   ratio = matches in +/-5 context window / 11
#   specificity = min(1.0, len(fingerprint)/8)
#   confidence = ratio * specificity (clamped 0..1)
compute_confidence() {
    local file="$1" line="$2" fingerprint="$3"
    [ -f "$file" ] || { printf '0.0\n'; return; }
    local start=$((line > 5 ? line - 5 : 1))
    local end=$((line + 5))
    local hits
    hits=$(awk -v s="$start" -v e="$end" -v p="$fingerprint" '
        NR < s { next } NR > e { exit }
        index($0, p) > 0 { c++ }
        END { print c+0 }
    ' "$file")
    local len=${#fingerprint}
    awk -v h="$hits" -v l="$len" 'BEGIN {
        # Presence: 1.0 if anchor appears at least once in the +/-5 window,
        # else 0. Density bonus for multi-hit windows (up to +0.2).
        if (h <= 0) { printf "0.000\n"; exit }
        presence = 1.0
        density  = (h - 1) * 0.1
        if (density > 0.2) density = 0.2
        spec = (l >= 8 ? 1.0 : l/8.0)
        c = (presence + density) * spec
        if (c > 1.0) c = 1.0
        if (c < 0.0) c = 0.0
        printf "%.3f\n", c
    }'
}

# Run verify-first probe for a sibling. Treat siblings as missing_function
# probes anchored on the fingerprint anchor: if the anchor is absent from
# the candidate file at HEAD, the bug has been resolved → DROP.
verify_sibling() {
    local file="$1" anchor="$2"
    [ -x "$VERIFY_HELPER" ] || return 0   # no verifier → keep all
    bash "$VERIFY_HELPER" --category missing_function \
        --symbol "$anchor" --file "$file" >/dev/null 2>&1
    # exit 0 = symbol absent (resolved upstream) → DROP this sibling
    # exit 1 = symbol present → KEEP
    case "$?" in
        0) return 1 ;;
        *) return 0 ;;
    esac
}

# Phase 1: compute and STORE fingerprints on parent findings at
# finding-time (pattern drift safety — issue #737 acceptance criterion).
PARENTS_TMP="$(mktemp)"
trap 'rm -f "$PARENTS_TMP"' EXIT

jq -c '
    (.findings // [])
    | to_entries[]
    | select(.value.release_blocking == true)
    | select((.value.category // "") | test(":bug$|:bug_|^bug:|_bug$"))
    | select(.value.file and .value.line)
    | {idx: .key, file: .value.file, line: .value.line, category: .value.category}
' "$VERDICT" > "$PARENTS_TMP" 2>/dev/null || true

# Build a jq script that patches in fingerprint + sibling findings atomically.
SIBLINGS_TMP="$(mktemp)"
FLAGGED_TMP="$(mktemp)"
PATCH_TMP="$(mktemp)"
trap 'rm -f "$PARENTS_TMP" "$SIBLINGS_TMP" "$FLAGGED_TMP" "$PATCH_TMP"' EXIT

printf '[]' > "$SIBLINGS_TMP"
printf '[]' > "$FLAGGED_TMP"
# parent_idx → fingerprint/hash patches
printf '[]' > "$PATCH_TMP"

while IFS= read -r parent; do
    [ -n "$parent" ] || continue
    idx=$(printf '%s' "$parent" | jq -r '.idx')
    pfile=$(printf '%s' "$parent" | jq -r '.file')
    pline=$(printf '%s' "$parent" | jq -r '.line')

    abs_pfile="$pfile"
    [ -f "$abs_pfile" ] || abs_pfile="$REPO_DIR/$pfile"
    [ -f "$abs_pfile" ] || continue

    fingerprint=$(extract_fingerprint "$abs_pfile" "$pline")
    [ -n "$fingerprint" ] || continue
    fhash=$(fingerprint_hash "$fingerprint")

    # Record patch for parent.
    jq --argjson idx "$idx" --arg fp "$fingerprint" --arg fh "$fhash" \
        '. + [{idx: $idx, fingerprint: $fp, hash: $fh}]' \
        "$PATCH_TMP" > "${PATCH_TMP}.new" && mv "${PATCH_TMP}.new" "$PATCH_TMP"

    # Sweep the repo for sibling matches (exclude parent file:line).
    while IFS=: read -r mfile mline mtext; do
        [ -n "$mfile" ] || continue
        [ -n "$mline" ] || continue
        # Skip parent's own file:line.
        case "$mfile" in
            "$pfile"|"./$pfile")
                [ "$mline" = "$pline" ] && continue
                ;;
        esac
        abs_mfile="$mfile"
        [ -f "$abs_mfile" ] || abs_mfile="$REPO_DIR/$mfile"
        [ -f "$abs_mfile" ] || continue

        conf=$(compute_confidence "$abs_mfile" "$mline" "$fingerprint")
        # Numeric compare against threshold.
        above=$(awk -v c="$conf" -v t="$MIN_CONF" 'BEGIN {print (c+0 >= t+0) ? 1 : 0}')

        sib=$(jq -n \
            --arg cat "code_health:bug_class_sibling" \
            --arg pf "$fhash" \
            --argjson c "$conf" \
            --arg f "$mfile" \
            --argjson l "$mline" \
            '{category:$cat, parent_fingerprint:$pf, confidence:$c, file:$f, line:$l, release_blocking:true}')

        if [ "$above" = "1" ]; then
            # Apply verify-first filter.
            if verify_sibling "$abs_mfile" "$fingerprint"; then
                jq --argjson s "$sib" '. + [$s]' "$SIBLINGS_TMP" > "${SIBLINGS_TMP}.new" \
                    && mv "${SIBLINGS_TMP}.new" "$SIBLINGS_TMP"
            fi
        else
            jq --argjson s "$sib" '. + [$s]' "$FLAGGED_TMP" > "${FLAGGED_TMP}.new" \
                && mv "${FLAGGED_TMP}.new" "$FLAGGED_TMP"
        fi
    done < <(
        cd "$REPO_DIR" && git grep -nE "$fingerprint" -- \
            ':(exclude)node_modules' ':(exclude).git' ':(exclude)vendor' \
            2>/dev/null || true
    )
done < "$PARENTS_TMP"

# Atomic verdict mutation: apply parent fingerprint patches + append siblings.
VERDICT_TMP="${VERDICT}.tmp.$$"
jq --slurpfile patches "$PATCH_TMP" --slurpfile sibs "$SIBLINGS_TMP" '
    . as $root
    | (.findings // []) as $f
    | ($patches[0] // []) as $ps
    | reduce $ps[] as $p ($f;
        if .[$p.idx] then
            .[$p.idx] = (.[$p.idx]
                + {pattern_fingerprint: $p.fingerprint,
                   fingerprint_hash:    $p.hash})
        else . end)
    | . + ($sibs[0] // [])
    | $root + {findings: .}
' "$VERDICT" > "$VERDICT_TMP" && mv "$VERDICT_TMP" "$VERDICT"

# Atomic flagged log mutation.
FLAGGED_OUT_TMP="${FLAGGED}.tmp.$$"
jq --slurpfile sibs "$FLAGGED_TMP" '
    .flagged = ((.flagged // []) + ($sibs[0] // []))
' "$FLAGGED" > "$FLAGGED_OUT_TMP" && mv "$FLAGGED_OUT_TMP" "$FLAGGED"

exit 0
