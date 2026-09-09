#!/usr/bin/env bash
# scripts/lint-process-matchers.sh — reject command-line process matchers.
#
# pgrep/pkill with the -f flag matches the full command line, which
# includes the waiter's own argv. A wrapper whose argv contains the
# pattern matches itself and waits (or kills) forever — the self-matching
# deadlock from issue #3938. Process control in this repo is pid-based:
#   wait: scripts/lib/autospec-process-wait.sh (autospec_wait_pid /
#         autospec_wait_sentinel, both deadline-bounded)
#   kill: scripts/lib/autospec-process-tree.sh (autospec_kill_tree)
#
# Usage: scripts/lint-process-matchers.sh [--help] [path ...]
#   No args: scans scripts/ and .github/workflows/ (same scope as
#   lint-factual-claims.sh).
#   Args: explicit files or directories to scan instead.
#
# Exemptions:
#   - Reviewed exceptions: tests/fixtures/lint-process-matchers/allowlist.txt
#     (one repo-relative path per line; # comments and blank lines).
#   - Inline waiver: `# process-matcher:allow <reason>` on the offending
#     line or the line immediately above it. The reason is mandatory; a
#     bare marker is rejected and the line stays a finding.
#
# Findings print as: PROCESS_MATCHER:<path>:<line>: <snippet>
# Waivers print as:  INFO:PROCESS_MATCHER:<path>:<line>: waived: <reason>
# Exit code = finding count, capped at 64 (0 = pass).

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ALLOWLIST="$ROOT_DIR/tests/fixtures/lint-process-matchers/allowlist.txt"

usage() {
    cat <<'EOF'
Usage: scripts/lint-process-matchers.sh [--help] [path ...]

Rejects pgrep/pkill with the -f flag (full-command-line process matching).

A -f pattern matches the waiter's own argv when the pattern appears in it:
a wrapper whose argv contains the pattern waits for itself to exit and
hangs forever (issue #3938). Use pid-based waits instead:
  source scripts/lib/autospec-process-wait.sh
  autospec_wait_pid <pid> <deadline_secs> <label>

With no arguments, scans scripts/ and .github/workflows/. Explicit path
arguments (files or directories) override the default scope.

Exemptions:
  - tests/fixtures/lint-process-matchers/allowlist.txt (repo-relative
    paths, one per line) for reviewed exceptions.
  - `# process-matcher:allow <reason>` on the offending line or the line
    immediately above it (reason mandatory).

Exit code = finding count, capped at 64.
EOF
}

for arg in "$@"; do
    case "$arg" in
        -h|--help)
            usage
            exit 0
            ;;
    esac
done

# The tool names are assembled from fragments so this script's own text
# never contains a literal "pgrep -f" / "pkill -f" adjacency — the lint
# must not flag its own definition.
_PW_TOOL='pg''rep|pk''ill'
# A tool name (word-bounded), then any run of flag tokens (no bare
# arguments may separate them), then a flag token containing f (-f, -af,
# -fa, ...) or the --full long form.
RE_MATCHER="(^|[^[:alnum:]_-])(${_PW_TOOL})([[:space:]]+[-][A-Za-z0-9]+)*[[:space:]]+([-][A-Za-z0-9]*f[A-Za-z0-9]*|--full)([[:space:]]|$)"
RE_WAIVE='^[[:space:]]*#[[:space:]]*process-matcher:allow[[:space:]]+[^[:space:]]'
RE_WAIVE_INLINE='#[[:space:]]*process-matcher:allow[[:space:]]+[^[:space:]]'

# ── Allowlist ────────────────────────────────────────────────────────────────

_in_allowlist() {
    local rel="$1" entry
    [ -f "$ALLOWLIST" ] || return 1
    while IFS= read -r entry; do
        entry="${entry#"${entry%%[![:space:]]*}"}"
        entry="${entry%"${entry##*[![:space:]]}"}"
        [ -n "$entry" ] || continue
        case "$entry" in
            \#*) continue ;;
        esac
        if [ "$entry" = "$rel" ]; then
            return 0
        fi
    done < "$ALLOWLIST"
    return 1
}

# ── File collection ──────────────────────────────────────────────────────────

FILES=()
if [ "$#" -gt 0 ]; then
    for p in "$@"; do
        if [ -d "$p" ]; then
            while IFS= read -r -d '' f; do
                FILES+=("$f")
            done < <(find "$p" -type f \( -name '*.sh' -o -name '*.yml' -o -name '*.yaml' \) -print0 | sort -z)
        elif [ -f "$p" ]; then
            FILES+=("$p")
        else
            printf 'lint-process-matchers: no such file or directory: %s\n' "$p" >&2
            exit 2
        fi
    done
else
    for d in "$ROOT_DIR/scripts" "$ROOT_DIR/.github/workflows"; do
        [ -d "$d" ] || continue
        while IFS= read -r -d '' f; do
            FILES+=("$f")
        done < <(find "$d" -type f \( -name '*.sh' -o -name '*.yml' -o -name '*.yaml' \) -print0 | sort -z)
    done
fi

# ── Scan ─────────────────────────────────────────────────────────────────────

findings=0
for f in ${FILES[@]+"${FILES[@]}"}; do
    rel="${f#"$ROOT_DIR"/}"
    if _in_allowlist "$rel"; then
        continue
    fi
    # Candidate line numbers in one pass; waivers are checked per hit so
    # the previous line stays available for the same-line-or-above rule.
    while IFS= read -r hit; do
        [ -n "$hit" ] || continue
        lineno="$hit"
        line="$(sed -n "${lineno}p" "$f")"
        prevline=""
        if [ "$lineno" -gt 1 ]; then
            prevline="$(sed -n "$((lineno - 1))p" "$f")"
        fi
        if printf '%s\n' "$line" | grep -Eq "$RE_WAIVE_INLINE"; then
            reason="$(printf '%s\n' "$line" | sed -E 's/.*#[[:space:]]*process-matcher:allow[[:space:]]+//')"
            printf 'INFO:PROCESS_MATCHER:%s:%d: waived: %s\n' "$rel" "$lineno" "$reason"
        elif [ -n "$prevline" ] && printf '%s\n' "$prevline" | grep -Eq "$RE_WAIVE"; then
            reason="$(printf '%s\n' "$prevline" | sed -E 's/^[[:space:]]*#[[:space:]]*process-matcher:allow[[:space:]]+//')"
            printf 'INFO:PROCESS_MATCHER:%s:%d: waived: %s\n' "$rel" "$lineno" "$reason"
        else
            snippet="$(printf '%s' "$line" | sed -E 's/^[[:space:]]+//')"
            printf 'PROCESS_MATCHER:%s:%d: %s\n' "$rel" "$lineno" "$snippet"
            findings=$((findings + 1))
        fi
    done < <(grep -nE "$RE_MATCHER" "$f" 2>/dev/null | cut -d: -f1 || true)
done

if [ "$findings" -gt 0 ]; then
    echo "lint-process-matchers: $findings finding(s) — use pid-based waits (scripts/lib/autospec-process-wait.sh), not command-line matchers" >&2
fi
if [ "$findings" -ge 64 ]; then
    exit 64
fi
exit "$findings"
