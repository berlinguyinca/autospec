#!/usr/bin/env bash
# scripts/lint-factual-claims.sh — flag comments that assert an externally
# checkable fact the code cannot prove for itself (issue #3870).
#
# A comment that states a fact about the world — registry contents and their
# publicity, package visibility, a host's or worker's capability, network
# reachability of a registry or host — decays silently: it was true when
# written and nothing re-checks it. A comment cannot fail, so a fact the code
# depends on must be one of:
#
#   1. executed — the same file contains a runtime check for that fact
#      category (approximated per file; see below); or
#   2. dated and sourced — the comment carries a YYYY-MM-DD date plus the
#      observation that established it (verified, observed, measured,
#      confirmed, checked), e.g. a comment saying the production packages are
#      public, dated 2026-09-06, established by an anonymous manifest GET
#      returning 200; or
#   3. waived — the same line or the line immediately above carries
#      `# linter:allow-FACTUAL_CLAIM <reason>`. The reason is mandatory; a
#      bare marker is rejected and the claim stays flagged.
#
# A bare assertion satisfying none of the three is a finding. Intent comments
# ("publish must not depend on an unproven property") contain no checkable
# fact and are not findings.
#
# Runtime-check approximation (same file, .sh files only):
#   REGISTRY_VISIBILITY    HTTP status handling: 401 / 403 / 404, http_code,
#                          http_status
#   HOST_CAPABILITY        tool probing:          command -v, which <tool>
#   NETWORK_REACHABILITY   reachability probing:  curl, wget, nc, ping
# Workflow YAML files carry no runtime check of their own; there only forms 2
# and 3 pass.
#
# Usage:
#   scripts/lint-factual-claims.sh [PATH...]
#   scripts/lint-factual-claims.sh --help
#
# With no arguments the scan covers scripts/ and .github/workflows/ at the
# repository root. PATH may be a file or a directory (scanned recursively for
# *.sh, *.yml, *.yaml).
#
# Output: one finding per line on stdout:
#   FACTUAL_CLAIM:<path>:<line>: <CATEGORY>: <comment text>
# Waived and dated claims emit nothing (waived lines emit an advisory
# INFO:FACTUAL_CLAIM:... line for audit).
#
# Exit code = number of blocking findings (0 = pass), capped at 64.

set -eu

SCRIPT_PATH="$0"
SCRIPT_DIR=$(cd "$(dirname "$SCRIPT_PATH")" && pwd)
ROOT_DIR=$(cd "$SCRIPT_DIR/.." && pwd)

RE_REGISTRY='(packages?|images?|registries?|repositories?|artifacts?) (is|are) (public|private|publicly available)|(public|private|publicly available) (packages?|images?|registries?|artifacts?)([^a-z]|$)|anonymous (pull|access|fetch|clone|checkout)'
RE_HOST='(host|machine|worker|runner)( |,)(has|lacks|with|without|does not have|does not support) (a |an |the |no |any )?(container runtime|containerd|docker daemon|docker|podman|buildx|kubernetes|kubectl)([^a-z]|$)|(no|without) (a |an )?(container runtime|docker daemon|podman|buildx)([^a-z]|$)'
RE_NET='(registry|host|server|endpoint) (is|was) (always )?reachable|(always|can always) (reach|reaches) the (registry|host|server|network)|((network|internet) access|(network|internet) connectivity) (is|was) (always )?(available|open|present)|(no|without) (any )?(network|internet) (access|connectivity)'
RE_DATED='[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'
RE_VERB='verified|observed|measured|confirmed|checked'
ALLOW_MARKER='linter:allow-FACTUAL_CLAIM'

AWK_PROG=$(cat <<'AWK'
function trim(s) { sub(/^[ \t]+/, "", s); return s }
{ lines[NR] = $0 }
END {
    prev_allow = ""
    for (i = 1; i <= NR; i++) {
        t = trim(lines[i])
        allow = ""
        if (index(t, MARKER) > 0) {
            rest = t
            sub(".*" MARKER "[ \t]*", "", rest)
            if (rest != "") allow = rest
        }
        if (t ~ /^#/) {
            l = tolower(t)
            cat = ""
            if (l ~ RE_REG) cat = "REGISTRY_VISIBILITY"
            else if (l ~ RE_HOST) cat = "HOST_CAPABILITY"
            else if (l ~ RE_NET) cat = "NETWORK_REACHABILITY"
            if (cat != "") {
                dated = (t ~ DATED)
                if (dated && l ~ VERB) { prev_allow = allow; continue }
                if (allow != "") {
                    printf "INFO:FACTUAL_CLAIM:%s:%d: opt-out (same line): %s\n", FILE, i, allow
                    prev_allow = ""
                    continue
                }
                if (prev_allow != "") {
                    printf "INFO:FACTUAL_CLAIM:%s:%d: opt-out: %s\n", FILE, i, prev_allow
                    prev_allow = ""
                    continue
                }
                printf "CLAIM:%d:%s:%s\n", i, cat, t
            }
        }
        prev_allow = allow
    }
}
AWK
)

usage() {
    # Print this file's leading comment block (skip the shebang line).
    awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$SCRIPT_PATH"
}

die() {
    printf 'ERROR: %s\n' "$2" >&2
    exit "$1"
}

# ---- argument parsing -------------------------------------------------------

paths=()
for arg in "$@"; do
    case "$arg" in
        -h|--help)
            usage
            exit 0
            ;;
        -*)
            die 2 "unknown option: $arg"
            ;;
        *)
            paths+=("$arg")
            ;;
    esac
done

if [ ${#paths[@]} -eq 0 ]; then
    [ -d "$ROOT_DIR/scripts" ] && paths+=("$ROOT_DIR/scripts")
    [ -d "$ROOT_DIR/.github/workflows" ] && paths+=("$ROOT_DIR/.github/workflows")
fi
[ ${#paths[@]} -gt 0 ] || die 2 "no paths to scan (pass file or directory paths)"


# Runtime-check approximation: does the file itself check the given fact
# category at runtime? (See header: shell files only; per-file, not per-line.)
runtime_check_present() {
    case "$2" in
        REGISTRY_VISIBILITY)
            LC_ALL=C grep -qiE '401|403|404|http_code|http_status' "$1" ;;
        HOST_CAPABILITY)
            LC_ALL=C grep -qiE 'command -v|which ' "$1" ;;
        NETWORK_REACHABILITY)
            LC_ALL=C grep -qiE 'curl |wget |nc -|ping ' "$1" ;;
        *)
            return 1
            ;;
    esac
}

# ---- file collection --------------------------------------------------------

files=()
for p in "${paths[@]}"; do
    [ -e "$p" ] || die 2 "no such file or directory: $p"
    if [ -f "$p" ]; then
        case "$p" in
            *.sh|*.yml|*.yaml) files+=("$p") ;;
            *) continue ;;
        esac
    elif [ -d "$p" ]; then
        while IFS= read -r f; do
            files+=("$f")
        done < <(find "$p" -type f \( -name '*.sh' -o -name '*.yml' -o -name '*.yaml' \) | LC_ALL=C sort)
    else
        die 2 "not a file or directory: $p"
    fi
done

findings=0

if [ ${#files[@]} -eq 0 ]; then
    printf 'INFO: lint-factual-claims: no .sh/.yml/.yaml files to scan\n'
    exit 0
fi

for file in "${files[@]}"; do
    rel="$file"
    case "$rel" in
        "$ROOT_DIR"/*) rel="${rel#"$ROOT_DIR"/}" ;;
    esac
    case "$file" in
        *.yml|*.yaml) is_shell=0 ;;
        *) is_shell=1 ;;
    esac

    out=$(LC_ALL=C awk \
        -v FILE="$rel" \
        -v MARKER="$ALLOW_MARKER" \
        -v RE_REG="$RE_REGISTRY" \
        -v RE_HOST="$RE_HOST" \
        -v RE_NET="$RE_NET" \
        -v DATED="$RE_DATED" \
        -v VERB="$RE_VERB" \
        "$AWK_PROG" "$file") || die 3 "awk failed on $file"
    [ -n "$out" ] || continue

    while IFS= read -r cand; do
        case "$cand" in
            CLAIM:*)
                rest="${cand#CLAIM:}"
                ln="${rest%%:*}"
                rest="${rest#*:}"
                cat="${rest%%:*}"
                text="${rest#*:}"
                if [ "$is_shell" -eq 1 ] && runtime_check_present "$file" "$cat"; then
                    continue
                fi
                printf 'FACTUAL_CLAIM:%s:%s: %s: %s\n' "$rel" "$ln" "$cat" "$text"
                findings=$((findings + 1))
                ;;
            *)
                printf '%s\n' "$cand"
                ;;
        esac
    done <<< "$out"
done

[ "$findings" -gt 64 ] && findings=64
exit "$findings"
