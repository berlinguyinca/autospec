#!/usr/bin/env bash
# scripts/lint-stdin-worklist-loops.sh — report a shell loop that reads its
# worklist from stdin (FD 0) while running gh/cargo/git/ssh in the body
# (issue #3742).
#
# Why this exists (#3742). A `while ...; do ...; done < worklist` loop gives
# the entire body stdin = worklist. Any command in the body that reads stdin
# (gh, ssh, cargo run, git apply/commit, ...) therefore eats the worklist
# instead of the loop's own `read`, so the loop processes only the first
# candidate and exits 0 silently. The patch-to-PR conversion pass lost 72 of 73
# candidates this way. The fix is to read the worklist on a dedicated
# descriptor (`while ... <&3; do ...; done 3< worklist`) so a body command
# that reads FD 0 sees an empty stdin, not the worklist. This gate is the
# codebase-wide form of that rule: it flags the pattern so it cannot come back.
#
# Known limitation: a multi-line process substitution on a `done < <(...)`
# line is scanned as if its body were the enclosing loop's; a stdin-consuming
# command written there would be (rarely) attributed to the enclosing loop.
# The stdin-consuming `git` subcommand set and the `gh`/`ssh`/`cargo` rules
# make this a non-issue for the commands actually seen in the tree.
#
# This is a line-token scanner, not a shell parser. It catches the canonical
# command-position form — literal `gh`/`ssh`/`cargo`/`git`, or a `GH_*`/
# `GIT_*`/`CARGO_*`/`SSH_*` variable alias at command position — plus a
# stdin-consuming command buried in a `$( ... )` command substitution (which
# inherits the loop's stdin and eats the worklist the same way a bare command
# does), via a targeted regex over the line. Commands the scanner cannot see —
# `$(git -C x commit ...)`, `$( "$GIT_*" ...)`, `$(env VAR=x gh ...)`, or a
# command invoked only through a variable whose name does not start with
# GH/GIT/CARGO/SSH — are not a finding. The gate is a ratchet, not a proof of
# absence.
#
# A *stdin-fed worklist loop* is a `while`/`for`/`until`/`select` loop whose
# `done` line redirects the loop's stdin from FD 0 — `done < file`,
# `done <<< str`, or `done <<EOF`. A `done` line that redirects a *dedicated*
# descriptor (`done 3< file`, `done 3<<< str`, `done 3<<EOF`) is NOT
# stdin-fed and is never a finding. A `done` line with no redirect (a
# counter loop, or a loop whose stdin is a pipe inherited from the enclosing
# scope) does not re-bind FD 0 here and is not flagged by this gate.
#
# A finding is a stdin-fed worklist loop whose *direct* body runs a
# stdin-consuming command at command position: `gh`, `ssh`, `cargo`, or a
# stdin-consuming `git` subcommand (apply, commit, fast-import, hash-object,
# rebase, am). Plain `git` subcommands that never read stdin (log, ls-files,
# ls-remote, rev-parse, grep, diff, show, config, status, ...) are NOT a
# finding: they read the object database or index, not the worklist. *Direct*
# body means the lines between the loop's `do` and its matching `done`,
# EXCLUDING the bodies of any nested loops — a nested loop is checked on its
# own (its commands read the innermost loop's stdin, not the outer loop's), so
# they are not attributed to the outer loop. A command inside a process
# substitution (`<(...)`) runs in a subshell as an input source and does not
# consume the enclosing loop's stdin, so it is not a finding.
#
# *Command position* means the first token of a command: start of line, or
# right after a command separator (`;` `&` `&&` `||` `|` `(` `)`), or
# immediately after a launcher (`bash` `sh` `zsh` `env` `sudo` `nohup`
# `xargs` `time` `nice` `stdbuf` `command` `exec` `source`). A token that is
# a path (`.git`, `/usr/bin/git`), a value (`git@`, `cargo:`), a word
# (`.github`, `sshpass`), or inside a quoted string is not a command.
#
# Output (blocking mode): one finding per stdin-fed loop that runs a
# stdin-consuming command in its direct body, citing the `done` line where
# the redirect lives:
#   STDIN_WORKLIST_LOOP:<path>:<line>: <kind> loop (started line <N>) runs
#   '<cmd>' at line <M> while reading its worklist from stdin; read the
#   worklist on a dedicated FD (done 3< worklist, read <&3)
#
# Usage:
#   scripts/lint-stdin-worklist-loops.sh [PATH...]
#   scripts/lint-stdin-worklist-loops.sh --root DIR [PATH...]
#   scripts/lint-stdin-worklist-loops.sh --list
#
# With no PATH arguments the scan covers *.sh and *.bats under
# <ROOT>/{scripts,skills} — the harness shell this rule applies to. --root
# overrides ROOT (default: the repository root, one level above this
# script). PATH may be a file or a directory (scanned recursively).
#
# --list emits an audit line for every stdin-fed worklist loop found, with
# or without a stdin-consuming command in the body (always exits 0):
#   FD0_LOOP:<path>:<line>: <kind> loop (started line <N>) reads its
#   worklist from stdin [runs '<cmd>' at line <M>]
#
# Exit code (blocking mode) = number of findings (0 = pass), capped at 64.
# --list always exits 0.

set -eu

SCRIPT_PATH="$0"
SCRIPT_DIR=$(cd "$(dirname "$SCRIPT_PATH")" && pwd)
ROOT_DIR=$(cd "$SCRIPT_DIR/.." && pwd)

usage() {
    # Print this file's leading comment block (skip the shebang line).
    awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$SCRIPT_PATH"
}

die() {
    printf 'ERROR: %s\n' "$2" >&2
    exit "$1"
}

# ---- argument parsing -------------------------------------------------------

LIST_MODE=0
paths=()
while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help)
            usage
            exit 0
            ;;
        --list)
            LIST_MODE=1
            shift
            ;;
        --root)
            [ $# -ge 2 ] || die 2 "--root requires a directory argument"
            ROOT_DIR=$(cd "$2" && pwd)
            shift 2
            ;;
        --root=*)
            ROOT_DIR=$(cd "${1#--root=}" && pwd)
            shift
            ;;
        -*)
            die 2 "unknown option: $1"
            ;;
        *)
            paths+=("$1")
            shift
            ;;
    esac
done

if [ ${#paths[@]} -eq 0 ]; then
    for d in scripts skills; do
        [ -d "$ROOT_DIR/$d" ] && paths+=("$ROOT_DIR/$d")
    done
fi
[ ${#paths[@]} -gt 0 ] || die 2 "no paths to scan (pass file or directory paths)"

# ---- file collection --------------------------------------------------------

files=()
for p in "${paths[@]}"; do
    [ -e "$p" ] || die 2 "no such file or directory: $p"
    if [ -f "$p" ]; then
        case "$p" in
            *.sh|*.bats) files+=("$p") ;;
            *) continue ;;
        esac
    elif [ -d "$p" ]; then
        while IFS= read -r f; do
            files+=("$f")
        done < <(find "$p" -type f \( -name '*.sh' -o -name '*.bats' \) | LC_ALL=C sort)
    else
        die 2 "not a file or directory: $p"
    fi
done

if [ ${#files[@]} -eq 0 ]; then
    printf 'INFO: lint-stdin-worklist-loops: no .sh/.bats files to scan\n'
    exit 0
fi

# ---- stdin-fed-loop + command scan ------------------------------------------
# Single pass over each file with a loop stack. On a `do` line we push the
# loop; on a `done` line we pop and, if the done line re-binds stdin from FD 0
# and the loop's direct body ran a stdin-consuming command, we emit a finding.
# Command hits are attributed to the innermost (top-of-stack) loop only, which
# is exactly the "direct body" rule: a command inside a nested loop is
# attributed to the nested loop, not the outer one.
#
# Emits (blocking): STDIN_WORKLIST_LOOP:<file>:<line>: <msg>
# Emits (--list):   FD0_LOOP:<file>:<line>: <msg>

AWK_PROG=$(cat <<'AWK'
function stripq(s) {
    if (s ~ /^".*"$/ || s ~ /^'.*'$/) s = substr(s, 2, length(s) - 2)
    return s
}
function is_launcher(s) {
    return (s == "bash" || s == "sh" || s == "zsh" || s == "dash" || s == "ksh" \
        || s == "ash" || s == "python" || s == "python2" || s == "python3" \
        || s == "perl" || s == "ruby" || s == "node" || s == "nodejs" \
        || s == "source" || s == "." || s == "exec" || s == "env" \
        || s == "sudo" || s == "nohup" || s == "xargs" || s == "time" \
        || s == "nice" || s == "stdbuf" || s == "command")
}
# Is `rest` (the text after the leading "done") re-binding the loop stdin from
# FD 0? A `<` at the start of a redirect that is not preceded by a digit (a
# dedicated FD such as `3<`). `<<<` and `<<` also start with `<`, so a single
# `<` test covers all three; exclude a `<` preceded by a digit or another `<`.
function fd0_redirect(rest) {
    return (rest ~ /(^|[^0-9<])</)
}
# Does this `git` invocation read stdin? Only a small set of subcommands do
# (they take a patch or drive an editor). We find the subcommand by skipping
# leading option tokens; value-taking options (-C, -c, --git-dir, ...) also
# skip their following argument so `git -C path commit` still resolves to
# `commit`. A conservative miss (options we don't model) is safe for a ratchet.
function git_reads_stdin(toks, n, i,    j, tk, subcmd) {
    j = i + 1
    subcmd = ""
    while (j <= n) {
        tk = stripq(toks[j])
        if (tk ~ /^-/) {
            if (tk == "-C" || tk == "-c" \
                || tk ~ /^--(git-dir|work-tree|config-env|namespace|upload-pack)/) {
                j += 2
            } else {
                j += 1
            }
            continue
        }
        subcmd = tk
        break
    }
    return (subcmd == "apply" || subcmd == "commit" \
        || subcmd == "fast-import" || subcmd == "hash-object" \
        || subcmd == "rebase" || subcmd == "am")
}
# Resolve a command-position token to the base command it names. A literal
# command (gh/git/cargo/ssh) maps to itself. A command invoked through a
# variable alias whose name starts with the command -- the repo convention
# `GH_BIN="${AUTOSPEC_GH_BIN:-gh}"` -- maps to that command. Any other token
# returns "". The `$` requirement keeps the bare-name assignment form (e.g.
# `GH_OPEN=1`) out: that is an assignment, not a command.
function cmd_of(s,    s2, bare) {
    s2 = stripq(s)
    if (s2 == "gh" || s2 == "git" || s2 == "cargo" || s2 == "ssh") return s2
    if (s2 !~ /\$/) return ""
    bare = s2
    sub(/^\$/, "", bare)
    if (bare ~ /^\{.*\}$/) bare = substr(bare, 2, length(bare) - 2)
    if (bare ~ /^(GH|GIT|CARGO|SSH)[A-Z0-9_]*$/) {
        if (bare ~ /^GH/) return "gh"
        if (bare ~ /^GIT/) return "git"
        if (bare ~ /^CARGO/) return "cargo"
        if (bare ~ /^SSH/) return "ssh"
    }
    return ""
}
# Does this line contain a `$( ... )` command substitution whose command is a
# stdin-consumer? `$( ... )` inherits the enclosing shell's stdin, so
# `x="$(gh ...)"` inside `while read; do ...; done < wl` eats the worklist
# exactly like a bare `gh` would (verified empirically: only the first line of
# the worklist is read). The token scanner cannot look inside `$( ... )`, so
# this is a targeted regex over the line. It mirrors the command-position
# rule: gh/ssh/cargo (and a GH_/SSH_/CARGO_ variable alias) are treated as
# stdin-consumers; git is flagged only for the stdin-consuming subcommands.
# Returns the base command name or "". Documented misses: `$(git -C x commit
# ...)` (an option between git and subcommand), `$( "$GIT_*" ...)` (a git
# variable alias whose subcommand is not checked), and `$(env VAR=x gh ...)`.
function cmdsub_stdin_consumer(line) {
    if (line ~ /\$\([ \t]*"?gh"?[ \t]/ || \
        line ~ /\$\([ \t]*(sudo|time|nice|nohup)[ \t]+"?gh"?[ \t]/ || \
        line ~ /\$\([ \t]*"?\$\{?GH[A-Z0-9_]*/) return "gh"
    if (line ~ /\$\([ \t]*"?ssh"?[ \t]/ || \
        line ~ /\$\([ \t]*(sudo|time|nice|nohup)[ \t]+"?ssh"?[ \t]/ || \
        line ~ /\$\([ \t]*"?\$\{?SSH[A-Z0-9_]*/) return "ssh"
    if (line ~ /\$\([ \t]*"?cargo"?[ \t]/ || \
        line ~ /\$\([ \t]*(sudo|time|nice|nohup)[ \t]+"?cargo"?[ \t]/ || \
        line ~ /\$\([ \t]*"?\$\{?CARGO[A-Z0-9_]*/) return "cargo"
    if (line ~ /\$\([ \t]*"?git"?[ \t]+(commit|apply|fast-import|hash-object|rebase|am)[ \t]/) return "git"
    return ""
}
{
    raw = $0
    t = raw; sub(/^[ \t]+/, "", t)
    if (t == "" || t ~ /^#/) next
    # trailing-comment-stripped copy, used for the do/done keyword tests and
    # the command tokenization.
    body = t; sub(/[ \t]#.*$/, "", body)

    # ---- loop terminator: done -------------------------------------------
    if (body ~ /^done([ \t]|$)/) {
        drest = body; sub(/^done[ \t]*/, "", drest)
        if (depth >= 1) {
            is_fd0 = fd0_redirect(drest)
            if (LIST && is_fd0) {
                suffix = (found[depth]) ? sprintf(" runs \x27%s\x27 at line %d", fcmd[depth], fline[depth]) : ""
                printf "FD0_LOOP:%s:%d: %s loop (started line %d) reads its worklist from stdin%s\n", \
                    FILE, NR, do_kind[depth], do_line[depth], suffix
            }
            if (is_fd0 && found[depth]) {
                printf "STDIN_WORKLIST_LOOP:%s:%d: %s loop (started line %d) runs \x27%s\x27 at line %d while reading its worklist from stdin; read the worklist on a dedicated FD (done 3< worklist, read <&3)\n", \
                    FILE, NR, do_kind[depth], do_line[depth], fcmd[depth], fline[depth]
            }
            depth--
        }
        next
    }

    # ---- loop start: a line whose last token is the `do` keyword ---------
    if (body ~ /(^|[ \t;|&])do[ \t]*$/) {
        kind = "loop"
        # Preceding char is any non-identifier char (so `$(while`, `)while`,
        # `;while` all count); trailing char must be non-identifier or EOL so
        # a longer word (e.g. `x-whilefoo`) is not misread as `while`.
        if (body ~ /(^|[^A-Za-z0-9_])while([^A-Za-z0-9_]|$)/) kind = "while"
        else if (body ~ /(^|[^A-Za-z0-9_])for([^A-Za-z0-9_]|$)/) kind = "for"
        else if (body ~ /(^|[^A-Za-z0-9_])until([^A-Za-z0-9_]|$)/) kind = "until"
        else if (body ~ /(^|[^A-Za-z0-9_])select([^A-Za-z0-9_]|$)/) kind = "select"
        depth++
        do_line[depth] = NR
        do_kind[depth] = kind
        found[depth] = 0
    }

    # ---- command-position scan for stdin-consuming commands ---------------
    n = split(body, toks, /[ \t]+/)
    at_start = 1
    prev = ""
    for (i = 1; i <= n; i++) {
        tok = toks[i]
        if (tok == "") continue
        if (tok == ";" || tok == "&" || tok == "&&" || tok == "||" \
            || tok == "|" || tok == "(" || tok == ")") {
            prev = ""; at_start = 1; continue
        }
        st = stripq(tok)
        in_cmd_pos = (at_start || is_launcher(prev))
        if (!in_cmd_pos) { prev = st; at_start = 0; continue }
        cmd = cmd_of(tok)
        if (cmd == "gh" || cmd == "ssh" || cmd == "cargo" \
            || (cmd == "git" && git_reads_stdin(toks, n, i))) {
            if (depth >= 1 && !found[depth]) {
                found[depth] = 1
                fline[depth] = NR
                fcmd[depth] = cmd
            }
        }
        prev = st
        at_start = 0
    }

    # Command substitution: `$( ... )` inherits the enclosing shell's stdin,
    # so a stdin-consuming command buried in it is a finding even though the
    # token scanner cannot see inside. Attributed to the innermost loop, like
    # the command-position scan above.
    if (depth >= 1 && !found[depth]) {
        c = cmdsub_stdin_consumer(body)
        if (c != "") {
            found[depth] = 1
            fline[depth] = NR
            fcmd[depth] = "$(" c ")"
        }
    }
}
AWK
)

# ---- scan -------------------------------------------------------------------

emit=""
for file in "${files[@]}"; do
    rel="$file"
    case "$rel" in
        "$ROOT_DIR"/*) rel="${rel#"$ROOT_DIR"/}" ;;
    esac
    if [ "$LIST_MODE" -eq 1 ]; then
        out=$(LC_ALL=C awk -v FILE="$rel" -v LIST=1 "$AWK_PROG" "$file") || die 3 "awk failed on $file"
    else
        out=$(LC_ALL=C awk -v FILE="$rel" "$AWK_PROG" "$file") || die 3 "awk failed on $file"
    fi
    [ -n "$out" ] || continue
    emit="${emit}${out}
"
done

if [ -z "$emit" ]; then
    exit 0
fi

if [ "$LIST_MODE" -eq 1 ]; then
    printf '%s\n' "$emit" | LC_ALL=C sort
    exit 0
fi

printf '%s\n' "$emit" | LC_ALL=C sort
count=$(printf '%s\n' "$emit" | grep -c '^STDIN_WORKLIST_LOOP:')
[ "$count" -gt 64 ] && count=64
[ "$count" -gt 0 ] || exit 0
exit "$count"
