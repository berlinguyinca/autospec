#!/usr/bin/env bash
# scripts/verify-finding.sh — require a reproduction for every review finding,
# and record "could not reproduce" as a terminal outcome rather than a
# reviewer failure (issue #3492).
#
# Why this exists: a cross-family reviewer rated a replay finding HIGH on an
# assumed economic double-pay. A probe showed the safety half was unreachable
# — an `already_claimed` backstop — while the denial half reproduced exactly.
# The finding was real and its severity was wrong in both directions at once.
# Without a reproduction, a severity rating is the reviewer's prior, and the
# pipeline cannot tell a verified defect from a plausible one. The fix is not
# to distrust the reviewer but to make the finding carry the probe that
# settles it: no `repro:` command, no report.
#
# Three rules, and the script exists to hold them:
#
#   1. A finding without a `repro:` line is not reportable (exit 4). It is not
#      "verified clean" and it is not a defect; it is an unreported finding.
#   2. "could not reproduce" is a recorded result, not an error (exit 0).
#      Treating it as a reviewer failure teaches the reviewer to assert
#      instead of check. The verdict and the repro exit status go into the
#      record so a later reader can see what was actually observed.
#   3. The repro command runs only inside the PR worktree — never in whatever
#      directory the caller happened to be standing in. If no worktree can be
#      established, the command is not run at all (exit 3, no verdict),
#      because a verdict produced anywhere else describes a different tree.
#
# The repro command signals the verdict with its own exit status: 0 means it
# demonstrated the finding (`reproduced`), any non-zero status means it did
# not (`could-not-reproduce`). A broken command (missing binary, exit 127)
# therefore lands on the conservative side: it is recorded as not reproduced
# with the status printed as evidence, never as a phantom reproduction.
#
# Usage:
#   scripts/verify-finding.sh --finding-file <path> [--worktree <dir>]
#   scripts/verify-finding.sh --help
#
# Arguments:
#   --finding-file <path>  A file holding ONE finding. The reproduction is the
#                          first line matching `repro: <command>` (leading
#                          `-`/`*` bullets and surrounding backticks allowed).
#   --worktree <dir>       The PR worktree to run the repro in. Must itself be
#                          a git worktree root. Defaults to the worktree root
#                          containing the finding file.
#
# Output (stdout, one key=value line each; parse `verdict:`):
#   finding: <path>
#   worktree: <path>              (omitted when no worktree could be resolved)
#   repro: <command>              (or `repro: (none)`)
#   verdict: reproduced | could-not-reproduce | no-repro-command
#   repro-exit=<int> | repro-exit=n/a
#   repro-output-1=<first non-empty output line>   (stdout+stderr, 300 chars)
#
# Exit codes:
#   0   a verdict was recorded (reproduced, or could-not-reproduce)
#   3   no PR worktree could be established; the repro command was NOT run
#   4   no-repro-command; the finding is not reportable
#   64  usage error

set -eu

SCRIPT_PATH="$0"

usage() {
    # Print this file's leading comment block (skip the shebang line).
    awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$SCRIPT_PATH"
}

die_usage() {
    printf 'ERROR: %s\n' "$1" >&2
    printf 'Usage: verify-finding.sh --finding-file <path> [--worktree <dir>]\n' >&2
    exit 64
}

die_worktree() {
    printf 'ERROR: no PR worktree: %s; the repro command was not run\n' "$1" >&2
    exit 3
}

finding_file=""
worktree_arg=""

while [ $# -gt 0 ]; do
    case "$1" in
        -h | --help)
            usage
            exit 0
            ;;
        --finding-file)
            [ $# -ge 2 ] || die_usage "--finding-file requires a path"
            finding_file="$2"
            shift 2
            ;;
        --finding-file=*)
            finding_file="${1#--finding-file=}"
            shift
            ;;
        --worktree)
            [ $# -ge 2 ] || die_usage "--worktree requires a directory"
            worktree_arg="$2"
            shift 2
            ;;
        --worktree=*)
            worktree_arg="${1#--worktree=}"
            shift
            ;;
        *)
            die_usage "unknown option: $1"
            ;;
    esac
done

[ -n "$finding_file" ] || die_usage "--finding-file <path> is required"
[ -f "$finding_file" ] && [ -r "$finding_file" ] || die_usage "finding file not found or unreadable: $finding_file"

# ---- resolve the PR worktree ------------------------------------------------
# --worktree wins; otherwise the worktree root containing the finding file.
# Never the caller's cwd, so the verdict always describes the PR tree.

resolve_toplevel() {
    local dir="$1" top
    top="$(git -C "$dir" rev-parse --show-toplevel 2>/dev/null || true)"
    [ -n "$top" ] || return 1
    # Canonicalise both sides: /tmp vs /private/tmp on macOS.
    printf '%s\n' "$(cd "$top" && pwd -P)"
}

worktree=""
if [ -n "$worktree_arg" ]; then
    [ -d "$worktree_arg" ] || die_worktree "--worktree is not a directory: $worktree_arg"
    worktree="$(resolve_toplevel "$worktree_arg")" || die_worktree "--worktree is not inside a git worktree: $worktree_arg"
    requested="$(cd "$worktree_arg" && pwd -P)"
    [ "$requested" = "$worktree" ] || die_worktree "--worktree must be the worktree root (got $requested, root is $worktree)"
else
    finding_dir="$(cd "$(dirname "$finding_file")" && pwd -P)"
    worktree="$(resolve_toplevel "$finding_dir")" || die_worktree "finding file is not inside a git worktree: $finding_file"
fi

# ---- extract the repro command ----------------------------------------------

repro="$(awk '
    {
        line = $0
        sub(/^[ \t]+/, "", line)
        sub(/^[-*][ \t]+/, "", line)
        if (line ~ /^repro[ \t]*:/) {
            sub(/^repro[ \t]*:[ \t]*/, "", line)
            # Unwrap a single pair of surrounding backticks.
            if (line ~ /^`.*`$/) { sub(/^`/, "", line); sub(/`$/, "", line) }
            sub(/[ \t]+$/, "", line)
            print line
            exit
        }
    }
' "$finding_file")"

printf 'finding: %s\n' "$finding_file"
printf 'worktree: %s\n' "$worktree"

if [ -z "$repro" ]; then
    printf 'repro: (none)\n'
    printf 'verdict: no-repro-command\n'
    printf 'repro-exit=n/a\n'
    printf 'note: a finding without a repro command is not reportable\n' >&2
    exit 4
fi

printf 'repro: %s\n' "$repro"

# ---- run the repro inside the worktree --------------------------------------
# No pipe: the status is taken from `sh -c` directly, output captured to a
# file, so an empty output is never mistaken for success.

out_file="$(mktemp "${TMPDIR:-/tmp}/verify-finding.XXXXXX")"
cleanup() { rm -f "$out_file"; }
trap cleanup EXIT

repro_status=0
(cd "$worktree" && sh -c "$repro" >"$out_file" 2>&1) || repro_status=$?

first_line="$(awk 'NF { line = $0; sub(/[ \t]+$/, "", line); print line; exit }' "$out_file")"
# Keep the evidence line bounded; the record is a log line, not a dump.
first_line="${first_line:0:300}"

if [ "$repro_status" -eq 0 ]; then
    printf 'verdict: reproduced\n'
else
    printf 'verdict: could-not-reproduce\n'
fi
printf 'repro-exit=%s\n' "$repro_status"
printf 'repro-output-1=%s\n' "$first_line"
exit 0
