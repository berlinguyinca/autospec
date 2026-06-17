#!/usr/bin/env bash
# derive-trio.sh — single-source derivation of a multi-harness skill trio.
#
# A multi-harness skill is three files kept byte-identical in the BODY by hand:
#   SKILL.md          authoritative source: YAML frontmatter + body
#   codex/prompt.md   the SKILL.md body with frontmatter stripped (NO frontmatter)
#   opencode/agent.md its OWN frontmatter + the SKILL.md body
# Both mirrors are pure mechanical transforms of SKILL.md's body. This script
# regenerates them deterministically so the trio collapses to one source of
# truth (SKILL.md) plus a derive step.
#
# CRITICAL transform contract (must byte-match validate.sh's check_lockstep):
#   * codex/prompt.md  == strip_body(SKILL.md)            — no frontmatter at all
#   * opencode/agent.md == <opencode's existing frontmatter> + strip_body(SKILL.md)
# where strip_body drops the YAML frontmatter (and any later `---` HR line) per
# the repo convention: awk '/^---$/{c++; next} c>=2'. Only the BODY of each
# mirror is regenerated; each mirror's existing frontmatter is preserved exactly
# (codex has none; opencode keeps its own mode:/name:/description:).
#
# Usage:
#   derive-trio.sh <skill-dir> --check          # diff derived vs committed; exit 0 if in sync, non-zero + diff on drift
#   derive-trio.sh <skill-dir> --in-place       # write both mirrors (idempotent)
#   derive-trio.sh <skill-dir> --stdout codex   # print derived codex/prompt.md to stdout
#   derive-trio.sh <skill-dir> --stdout opencode # print derived opencode/agent.md to stdout
#   derive-trio.sh -h | --help
#
# Exit codes:
#   0  success / --check in sync
#   1  --check drift (one or more mirrors diverge from the derived bytes)
#   2  usage error (bad args, missing/unreadable trio member)
#
# Conventions: set -u (no -e — we branch on diff exits explicitly); if/then/fi
# for one-sided conditionals; no RETURN traps (repo bash 3.2 gotchas).
set -u

PROG="$(basename "$0")"

usage() {
    cat <<EOF
$PROG — derive codex/prompt.md and opencode/agent.md from SKILL.md.

Usage:
  $PROG <skill-dir> --check            Diff derived mirrors vs committed; exit 0
                                       if identical, 1 + unified diff on drift.
  $PROG <skill-dir> --in-place         Rewrite both mirrors from SKILL.md (idempotent).
  $PROG <skill-dir> --stdout <member>  Print a derived member (codex|opencode) to stdout.
  $PROG -h | --help                    Show this help.

Transform:
  codex/prompt.md   = SKILL.md body with frontmatter stripped (no frontmatter).
  opencode/agent.md = opencode's existing frontmatter + SKILL.md body.
EOF
}

die() {  # die <code> <message>
    code="$1"; shift
    printf '%s: %s\n' "$PROG" "$*" >&2
    exit "$code"
}

# Strip YAML frontmatter (and any later `---` HR separator) per repo convention,
# matching validate.sh's strip_body exactly. The body is everything after the
# second `---` line; every `---` line is itself dropped.
strip_body() {  # strip_body <file>
    awk '/^---$/{c++; next} c>=2' "$1"
}

# Emit the leading frontmatter block of a file: every line up to and INCLUDING
# the second `---` line. For opencode/agent.md this is its own preserved
# frontmatter. If the file has fewer than two `---` lines, emits nothing.
front_matter() {  # front_matter <file>
    awk '{print} /^---$/{c++; if(c==2) exit}' "$1"
}

# Build the derived bytes for a given member into stdout.
#   derive_member codex    <skill-dir>  -> strip_body(SKILL.md)
#   derive_member opencode <skill-dir>  -> opencode frontmatter + strip_body(SKILL.md)
derive_member() {  # derive_member <member> <skill-dir>
    member="$1"; dir="$2"
    skill="$dir/SKILL.md"
    [ -f "$skill" ] || die 2 "missing SKILL.md in $dir"
    [ -r "$skill" ] || die 2 "unreadable SKILL.md in $dir"
    case "$member" in
        codex)
            strip_body "$skill"
            ;;
        opencode)
            oc="$dir/opencode/agent.md"
            [ -f "$oc" ] || die 2 "missing opencode/agent.md in $dir"
            [ -r "$oc" ] || die 2 "unreadable opencode/agent.md in $dir"
            front_matter "$oc"
            strip_body "$skill"
            ;;
        *)
            die 2 "unknown member '$member' (expected: codex | opencode)"
            ;;
    esac
}

# Diff a derived member against its committed file. Echoes the relative path of a
# drifting member to stdout (and a unified diff to stderr) on mismatch; nothing
# on a match. Never exits — the caller accumulates drift.
check_member() {  # check_member <member> <skill-dir> <committed-file>
    member="$1"; dir="$2"; committed="$3"
    [ -f "$committed" ] || die 2 "missing $committed"
    tmp="$(mktemp "${TMPDIR:-/tmp}/derive-trio.XXXXXX")" || die 2 "cannot create temp file"
    derive_member "$member" "$dir" > "$tmp"
    if ! diff -u "$committed" "$tmp" >/dev/null 2>&1; then
        diff -u "$committed" "$tmp" >&2
        printf '%s\n' "$committed"
    fi
    rm -f "$tmp"
}

# Atomically write derived bytes to a target via temp+mv.
write_member() {  # write_member <member> <skill-dir> <target-file>
    member="$1"; dir="$2"; target="$3"
    target_dir="$(dirname "$target")"
    [ -d "$target_dir" ] || die 2 "missing directory $target_dir for $target"
    tmp="$(mktemp "${TMPDIR:-/tmp}/derive-trio.XXXXXX")" || die 2 "cannot create temp file"
    if derive_member "$member" "$dir" > "$tmp"; then
        mv "$tmp" "$target"
    else
        rc=$?
        rm -f "$tmp"
        exit "$rc"
    fi
}

main() {
    skill_dir=""
    mode=""
    member=""
    while [ $# -gt 0 ]; do
        case "$1" in
            -h|--help) usage; exit 0 ;;
            --check)    mode="check"; shift ;;
            --in-place) mode="in-place"; shift ;;
            --stdout)
                mode="stdout"
                shift
                [ $# -gt 0 ] || die 2 "--stdout requires a member argument (codex|opencode)"
                member="$1"; shift ;;
            --) shift; break ;;
            -*) die 2 "unknown option: $1" ;;
            *)
                if [ -n "$skill_dir" ]; then die 2 "unexpected extra argument: $1"; fi
                skill_dir="$1"; shift ;;
        esac
    done
    if [ $# -gt 0 ]; then
        if [ -n "$skill_dir" ]; then die 2 "unexpected extra argument: $1"; fi
        skill_dir="$1"; shift
        [ $# -eq 0 ] || die 2 "unexpected extra argument: $1"
    fi

    [ -n "$skill_dir" ] || { usage >&2; die 2 "missing <skill-dir> argument"; }
    [ -n "$mode" ] || { usage >&2; die 2 "missing mode (--check | --in-place | --stdout <member>)"; }
    skill_dir="${skill_dir%/}"
    [ -d "$skill_dir" ] || die 2 "not a directory: $skill_dir"
    [ -f "$skill_dir/SKILL.md" ] || die 2 "not a skill directory (no SKILL.md): $skill_dir"

    case "$mode" in
        stdout)
            derive_member "$member" "$skill_dir"
            ;;
        check)
            drift=""
            d="$(check_member codex "$skill_dir" "$skill_dir/codex/prompt.md")"
            drift="${drift}${d:+$d
}"
            if [ -f "$skill_dir/opencode/agent.md" ]; then
                d="$(check_member opencode "$skill_dir" "$skill_dir/opencode/agent.md")"
                drift="${drift}${d:+$d
}"
            fi
            if [ -n "$drift" ]; then
                printf '%s: drift — run `%s %s --in-place`:\n%s' \
                    "$PROG" "$PROG" "$skill_dir" "$drift" >&2
                exit 1
            fi
            ;;
        in-place)
            write_member codex "$skill_dir" "$skill_dir/codex/prompt.md"
            if [ -f "$skill_dir/opencode/agent.md" ]; then
                write_member opencode "$skill_dir" "$skill_dir/opencode/agent.md"
            fi
            ;;
    esac
}

main "$@"
