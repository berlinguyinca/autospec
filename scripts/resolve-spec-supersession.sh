#!/usr/bin/env bash
# scripts/resolve-spec-supersession.sh — resolve which spec is authoritative for a
# given behavior key under implicit-by-recency supersession (issue #635).
#
# When two specs in docs/specs/ overlap on a behavior (matching heading or
# behavior-clause text), the spec whose last-modifying commit on the current
# branch is most recent wins. Operators do NOT write `Supersedes:` frontmatter
# — recency alone decides.
#
# Usage:
#   scripts/resolve-spec-supersession.sh <behavior-key>           # print winning spec path (exit 0) or "" (exit 1)
#   scripts/resolve-spec-supersession.sh --json <behavior-key>    # JSON: {"winner":"...","candidates":[...],"behavior":"..."}
#   scripts/resolve-spec-supersession.sh --list-overlapping <behavior-key>  # print all candidates oldest-first, one per line
#   scripts/resolve-spec-supersession.sh --specs-dir <dir> ...    # override docs/specs/ search root
#   scripts/resolve-spec-supersession.sh --help
#
# Behavior key matching:
#   A spec is a "candidate" for <behavior-key> when the key appears as a
#   case-insensitive substring of any line in the spec (heading or body).
#   Deleted specs (no longer present on disk) are excluded.
#
# Recency tie-break:
#   Most recent `git log -1 --format=%ct -- <spec>` wins. If git is unavailable
#   or two specs share a commit timestamp, falls back to mtime, then lexical
#   sort (later path wins) for determinism.
#
# Exit codes:
#   0  authoritative spec resolved (printed on stdout)
#   1  no spec covers the behavior key
#   2  invalid arguments

set -eu

HELP_TEXT="Usage:
  scripts/resolve-spec-supersession.sh <behavior-key>
  scripts/resolve-spec-supersession.sh --json <behavior-key>
  scripts/resolve-spec-supersession.sh --list-overlapping <behavior-key>
  scripts/resolve-spec-supersession.sh [--specs-dir <dir>] <behavior-key>
  scripts/resolve-spec-supersession.sh --help

Resolve the authoritative spec for a behavior key using implicit-by-recency
supersession (issue #635). When multiple specs overlap on a behavior, the spec
with the most recent last-modifying commit on the current branch wins.

Exit codes:
  0  authoritative spec resolved (printed on stdout)
  1  no spec covers the behavior key
  2  invalid arguments"

MODE="path"
SPECS_DIR=""
BEHAVIOR_KEY=""

while [ $# -gt 0 ]; do
    case "$1" in
        --help|-h)
            printf '%s\n' "$HELP_TEXT"
            exit 0
            ;;
        --json)
            MODE="json"
            shift
            ;;
        --list-overlapping)
            MODE="list"
            shift
            ;;
        --specs-dir)
            shift
            [ $# -gt 0 ] || { printf 'resolve-spec-supersession: --specs-dir requires a value\n' >&2; exit 2; }
            SPECS_DIR="$1"
            shift
            ;;
        --)
            shift
            break
            ;;
        -*)
            printf 'resolve-spec-supersession: unknown flag: %s\n' "$1" >&2
            exit 2
            ;;
        *)
            if [ -z "$BEHAVIOR_KEY" ]; then
                BEHAVIOR_KEY="$1"
                shift
            else
                printf 'resolve-spec-supersession: unexpected extra argument: %s\n' "$1" >&2
                exit 2
            fi
            ;;
    esac
done

[ -n "$BEHAVIOR_KEY" ] || { printf '%s\n' "$HELP_TEXT" >&2; exit 2; }

if [ -z "$SPECS_DIR" ]; then
    if [ -d docs/specs ]; then
        SPECS_DIR="docs/specs"
    else
        printf 'resolve-spec-supersession: no docs/specs/ directory; pass --specs-dir <dir>\n' >&2
        exit 1
    fi
fi

[ -d "$SPECS_DIR" ] || { printf 'resolve-spec-supersession: specs dir not found: %s\n' "$SPECS_DIR" >&2; exit 1; }

# Discover candidate spec files (existing only — deleted specs are excluded).
candidates=""
while IFS= read -r f; do
    [ -f "$f" ] || continue
    # Case-insensitive substring match anywhere in the file.
    if grep -i -F -q -- "$BEHAVIOR_KEY" "$f" 2>/dev/null; then
        candidates="${candidates}${f}
"
    fi
done <<EOF
$(find "$SPECS_DIR" -type f \( -name '*.md' -o -name '*.markdown' \) 2>/dev/null | sort)
EOF

# Strip trailing newlines.
candidates="$(printf '%s' "$candidates" | sed '/^$/d')"

if [ -z "$candidates" ]; then
    if [ "$MODE" = "json" ]; then
        printf '{"winner":null,"candidates":[],"behavior":%s}\n' "$(printf '%s' "$BEHAVIOR_KEY" | awk 'BEGIN{printf "\""} {gsub(/\\/,"\\\\"); gsub(/"/,"\\\""); printf "%s", $0} END{print "\""}')"
        exit 1
    fi
    exit 1
fi

# Recency ranking: produce "<sort-key>\t<path>" lines, sort descending.
RANK_FILE="$(mktemp -t resolve-spec-supersession.XXXXXX)"
trap 'rm -f "$RANK_FILE"' EXIT

while IFS= read -r f; do
    [ -n "$f" ] || continue
    ctime=""
    if command -v git >/dev/null 2>&1 && git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        ctime="$(git log -1 --format=%ct -- "$f" 2>/dev/null || true)"
    fi
    if [ -z "$ctime" ]; then
        # Not tracked or no commit history — use filesystem mtime so candidates
        # without git history still receive a deterministic recency rank.
        if [ -f "$f" ]; then
            if stat -f %m "$f" >/dev/null 2>&1; then
                ctime="$(stat -f %m "$f")"
            elif stat -c %Y "$f" >/dev/null 2>&1; then
                ctime="$(stat -c %Y "$f")"
            else
                ctime=0
            fi
        else
            ctime=0
        fi
    fi
    # Zero-pad ctime to 12 digits so lexical sort matches numeric sort, append
    # path as deterministic secondary key.
    printf '%012d\t%s\n' "$ctime" "$f" >> "$RANK_FILE"
done <<EOF
$candidates
EOF

sort -r "$RANK_FILE" -o "$RANK_FILE"

# Winner is the top line's path.
winner="$(head -n 1 "$RANK_FILE" | awk -F'\t' '{print $2}')"

case "$MODE" in
    path)
        printf '%s\n' "$winner"
        ;;
    list)
        # Print candidates oldest-first (reverse the ranking).
        sort "$RANK_FILE" | awk -F'\t' '{print $2}'
        ;;
    json)
        # Build JSON array of candidates ranked newest-first.
        json_candidates="$(awk -F'\t' 'BEGIN{first=1; printf "["} {if (!first) printf ","; first=0; p=$2; gsub(/\\/,"\\\\",p); gsub(/"/,"\\\"",p); printf "\"%s\"", p} END{print "]"}' "$RANK_FILE")"
        # Escape strings for JSON output.
        esc_winner="$(printf '%s' "$winner" | awk 'BEGIN{ORS=""} {gsub(/\\/,"\\\\"); gsub(/"/,"\\\""); print}')"
        esc_behavior="$(printf '%s' "$BEHAVIOR_KEY" | awk 'BEGIN{ORS=""} {gsub(/\\/,"\\\\"); gsub(/"/,"\\\""); print}')"
        printf '{"winner":"%s","candidates":%s,"behavior":"%s"}\n' "$esc_winner" "$json_candidates" "$esc_behavior"
        ;;
esac

exit 0
