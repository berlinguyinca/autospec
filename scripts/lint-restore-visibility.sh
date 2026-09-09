#!/usr/bin/env bash
# scripts/lint-restore-visibility.sh — ratchet for the mtime-preserving restore hazard.
#
# Restoring a file is not the same as making the restoration visible to a
# mtime-based build system. `mv` (and `cp -p|-a|--preserve`, `install -p`,
# `tar -x`, `git stash pop`) put a file back with the backup's timestamp. If
# the backup was taken before the last build of that input, the restored file
# looks OLDER than the artefact built from the corrupted one, the rebuild is a
# silent no-op, and the stale artefact is wrong rather than merely old — with
# the failure surfacing far from the build (issue #3878; measured on a
# manifest whose pinned digest reached the binary through an include-style
# embedding).
#
# A harness that backs up and restores a file must therefore make the
# restoration observable: `touch` the restored path — or rewrite its content,
# which updates the mtime as a side effect — after every restore, in the main
# path AND in any trap/cleanup body, because a trap may fire before later
# main-path lines ever run.
#
# This script scans shell harnesses (tests/, scripts/, skills/; *.sh and
# *.bats) for mtime-preserving restore sites without a later
# touch/rewrite of the restored path in the same scope.
#
# A restore site is a line whose command restores from a backup:
#   mv <backup-named> <dst>              (mv always preserves mtime)
#   cp -p|-a|--preserve <backup> <dst>
#   install -p <backup> <dst>
#   tar -x <archive> ...                 (restores stored mtimes; path unnamed)
#   git stash pop                        (restores tracked files; path unnamed)
# "backup-named" means the source operand ends in .bak/.backup/.orig/.saved.
#
# Visibility rules:
#   - Named destination: a later `touch <dst>` or content rewrite
#     (`> <dst>` / `>> <dst>`) in the same scope satisfies the site.
#   - Unnamed restore (tar, stash): a later `touch <anything>` in the same
#     scope satisfies the site.
#   - Scope is the whole file for top-level and ordinary-function sites, but a
#     trap-referenced function (and an inline `trap '...'` body) can only
#     satisfy itself: the trap may run before later main-path lines.
#   - Heredoc payloads are data, not commands.
#   - Escape hatch: `# restore-visibility:allow <reason>` on the site line or
#     the immediately preceding non-blank line suppresses that site.
#
# Usage:
#   scripts/lint-restore-visibility.sh [--root <dir>] [--list] [--help]
#
# Findings (stdout, one per line): STALE_RESTORE:<path>:<line>: <description>
# Exit 0 when clean; exit 1 on any finding. --list prints every site, exit 0.

set -eu

script_dir="$(cd "${0%/*}" 2>/dev/null && pwd -P || pwd -P)"
root="$(cd "${script_dir}/.." && pwd -P)"
mode="check"

usage() {
    awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --root) root="$(cd "$2" && pwd -P)"; shift 2 ;;
        --list) mode="list"; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "lint-restore-visibility: unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

files=""
for d in tests scripts skills; do
    [ -d "${root}/${d}" ] || continue
    found="$(find "${root}/${d}" \
        -type d \( -name node_modules -o -name .git \) -prune -o \
        -type f \( -name '*.sh' -o -name '*.bats' \) -print 2>/dev/null)"
    files="${files}${found:+$found
}"
done
files="$(printf '%s\n' "$files" | awk 'NF' | LC_ALL=C sort)"

if [ -z "$files" ]; then
    echo "lint-restore-visibility: OK (no shell harnesses found under ${root})"
    exit 0
fi

awk_prog="$(mktemp)"
trap 'rm -f "$awk_prog"' EXIT

cat > "$awk_prog" <<'AWK'
# One finding per mtime-preserving restore site without later visibility.
function hd_delim(line,   i, s, c) {
    i = index(line, "<<")
    if (i == 0) return ""
    if (substr(line, i + 2, 1) == "<") return ""          # here-string, not heredoc
    s = substr(line, i + 2)
    sub(/^-/, "", s)
    sub(/^[ \t]+/, "", s)
    c = substr(s, 1, 1)
    if (c == SQ || c == DQ) {
        s = substr(s, 2)
        i = index(s, c)
        if (i > 0) s = substr(s, 1, i - 1)
    } else if (match(s, /[^A-Za-z0-9_]/)) {
        s = substr(s, 1, RSTART - 1)
    }
    return s
}
function isbak(t,   u) { u = t; gsub(/["']/, "", u); return (u ~ /\.(bak|backup|orig|saved)$/) }
function isarchive(t,   u) { u = t; gsub(/["']/, "", u); return (u ~ /\.(tar(\.[a-z0-9]+)?|tgz|zip)$/) }
# mtime-preserving copy flag: -p / -a / combined short flags / --preserve
function flagval(F,   kk) {
    for (kk in F) if (F[kk] ~ /^-[a-z]*(p|a)[a-z]*$/ || F[kk] ~ /^--preserve([= ]|$)/) return 1
    return 0
}
# tar extract flag: -x / combined short flags / --extract
function flagextract(F,   kk) {
    for (kk in F) if (F[kk] ~ /^-[a-z]*x[a-z]*$/ || F[kk] == "--extract") return 1
    return 0
}
# touch command as the first (or ;/&&/||/| chained) command of the line
function istouch(v) { return (v ~ /(^|[;&|])[ \t]*touch[ \t]/) }
function visible_in(scope, core) {
    if (core != "") {
        if (istouch(scope) && index(scope, core) > 0) return 1
        if (index(scope, "> " core) > 0) return 1
        if (index(scope, ">> " core) > 0 || index(scope, ">>" core) > 0) return 1
        return 0
    }
    return istouch(scope)
}
# Classify one candidate line; print a finding for each unobserved restore.
# scope_end bounds the visibility search (index into L[]); lno is the report
# line; prevline is the preceding non-blank line for the escape hatch.
function classify(line, lno, prevline, scope_end,   m, W, start, cmd, k, no, OPS, nf, FLAGS, site, dst, core, allow, j, vj, msg) {
    tline = line
    sub(/^[ \t]+/, "", tline)
    m = split(tline, W, /[ \t]+/)
    start = 1
    if (m >= 2 && W[1] == "run") start = 2                # bats: `run cmd ...`
    if (start > m) return
    cmd = W[start]
    if (cmd == "" || cmd == "#") return
    if (cmd != "mv" && cmd != "cp" && cmd != "install" && cmd != "tar" && cmd != "git") return
    no = 0; nf = 0
    split("", OPS); split("", FLAGS)
    for (k = start + 1; k <= m; k++) {
        if (W[k] ~ /^-/) FLAGS[++nf] = W[k]
        else OPS[++no] = W[k]
    }
    site = 0; dst = ""
    if (cmd == "mv" && no == 2 && isbak(OPS[1])) { site = 1; dst = OPS[2] }
    else if ((cmd == "cp" || cmd == "install") && no >= 2 && isbak(OPS[1]) && flagval(FLAGS)) {
        site = 1; dst = OPS[no]
    } else if (cmd == "tar" && flagextract(FLAGS)) {
        for (k = 1; k <= no && !site; k++) if (isarchive(OPS[k])) site = 1
    } else if (cmd == "git" && m >= start + 2 && W[start + 1] == "stash" && W[start + 2] == "pop") {
        site = 1
    }
    if (!site) return
    if (dst ~ /\/["']?$/) dst = ""                        # moved/copied into a directory
    core = ""
    if (dst != "") { core = dst; gsub(/["']/, "", core) }
    allow = (line ~ /restore-visibility:allow[ \t]+[^ \t]/)
    if (!allow && prevline ~ /restore-visibility:allow[ \t]+[^ \t]/) allow = 1
    if (allow) return
    for (j = i_next; j <= scope_end && !visible; j++) {
        if (trap_scoped && LFu[j] != LFu[i_next]) break   # left the trap function
        vj = L[j]
        gsub(/["']/, "", vj)
        visible = visible_in(vj, core)
    }
    if (!visible) {
        if (core != "")
            msg = sprintf("restore of \"%s\" via %s keeps the backup mtime; touch (or rewrite) it after restoring so mtime-based rebuilds see the change", core, cmd)
        else
            msg = sprintf("%s restores files with their stored mtimes; touch the restored paths afterwards in this scope so mtime-based rebuilds see the change", cmd)
        printf "STALE_RESTORE:%s:%d: %s\n", FNAME, lno, msg
        fails++
    }
}
# Inline trap bodies: the trap may fire before later main-path lines, so the
# body must contain its own touch after the restore.
function classify_inline_trap(line, lno,   rl, rest, q, c, body, m, SEG, i, seg, m2, W2, k, no2, OPS2, FLAGS2, site, dst, core, allow, pos, after, av) {
    if (match(line, /^[ \t]*trap[ \t]+/) == 0) return
    rest = substr(line, RSTART + RLENGTH)
    q = substr(rest, 1, 1)
    if (q != SQ && q != DQ) return
    c = index(substr(rest, 2), q)
    if (c == 0) return
    body = substr(rest, 2, c - 1)
    if (body ~ /restore-visibility:allow[ \t]+[^ \t]/) return
    m = split(body, SEG, /;/)
    for (i = 1; i <= m; i++) {
        seg = SEG[i]
        if (seg ~ /^[ \t]*#/) continue
        m2 = split(seg, W2, /[ \t]+/)
        no2 = 0
        split("", OPS2); split("", FLAGS2)
        for (k = 2; k <= m2; k++) {
            if (W2[k] == "") continue
            if (W2[k] ~ /^-/) FLAGS2[W2[k]] = 1
            else OPS2[++no2] = W2[k]
        }
        site = 0; dst = ""
        if (m2 >= 2 && W2[1] == "mv" && no2 == 2 && isbak(OPS2[1])) { site = 1; dst = OPS2[2] }
        else if (m2 >= 2 && (W2[1] == "cp" || W2[1] == "install") && no2 >= 2 && isbak(OPS2[1]) && flagval(FLAGS2)) {
            site = 1; dst = OPS2[no2]
        } else if (m2 >= 2 && W2[1] == "tar" && flagextract(FLAGS2)) {
            for (k = 1; k <= no2 && !site; k++) if (isarchive(OPS2[k])) site = 1
        } else if (seg ~ /(^|[;&|])[ \t]*git[ \t]+stash[ \t]+pop/) {
            site = 1
        }
        if (!site) continue
        if (dst ~ /\/["']?$/) dst = ""
        core = ""
        if (dst != "") { core = dst; gsub(/["']/, "", core) }
        pos = index(body, seg)
        after = (pos > 0) ? substr(body, pos + length(seg)) : ""
        av = after
        gsub(/["']/, "", av)
        if (!visible_in(av, core)) {
            if (core != "")
                msg = sprintf("inline trap restore of \"%s\" via %s keeps the backup mtime; touch it after restoring inside the trap body", core, W2[1])
            else
                msg = sprintf("inline trap %s restores stored mtimes; touch the restored paths inside the trap body", W2[1])
            printf "STALE_RESTORE:%s:%d: %s\n", FNAME, lno, msg
            fails++
        }
    }
}
function flushfile(   i, j, p, line, norm, prev, scend) {
    # previous non-blank line index, and the trap-function scope decision
    for (i = 1; i <= n; i++) {
        line = L[i]
        norm = line
        sub(/^[ \t]+/, "", norm)
        if (norm == "" || norm ~ /^#/) { prev = 0; continue }
        if (norm ~ /^trap[ \t]+["']/) { classify_inline_trap(line, LF[i]); continue }
        p = i - 1
        while (p >= 1 && L[p] ~ /^[ \t]*$/) p--
        prev = (p >= 1) ? L[p] : ""
        trap_scoped = (LFu[i] != "" && (LFu[i] in TRAPFUNC))
        if (trap_scoped) {
            scend = i
            for (j = i + 1; j <= n && LFu[j] == LFu[i]; j++) scend = j
        } else scend = n
        i_next = i
        visible = 0
        classify(line, LF[i], prev, scend)
        prev = 0
    }
    n = 0; curfunc = ""
    for (k in TRAPFUNC) delete TRAPFUNC[k]
}
BEGIN {
    SQ = sprintf("%c", 39); DQ = sprintf("%c", 34)
    n = 0; curfunc = ""; hd = ""; fails = 0; FNAME = ""
}
FNR == 1 { flushfile(); FNAME = FILENAME; hd = ""; n = 0 }
hd != "" {
    if ($0 ~ "^[ \t]*" hd "[ \t]*$") hd = ""
    next
}
/^[A-Za-z_][A-Za-z0-9_]*[ \t]*\([ \t]*\)[ \t]*\{[ \t]*$/ {
    curfunc = $1; sub(/[ \t]*\(.*/, "", curfunc)
}
/^function[ \t]+[A-Za-z_][A-Za-z0-9_]*[ \t]*\{[ \t]*$/ { curfunc = $2 }
/^\}[ \t]*$/ { curfunc = "" }
{
    line = $0
    sub(/^[ \t]+/, "", line)
    if (line ~ /^trap[ \t]+[A-Za-z_][A-Za-z0-9_]*[ \t]/) {
        split(line, TW, /[ \t]+/)
        TRAPFUNC[TW[2]] = 1
    }
    n++
    L[n] = $0
    LF[n] = FNR
    LFu[n] = curfunc
    hd = hd_delim($0)
}
END { flushfile(); exit (fails > 0 ? 1 : 0) }
AWK

# fail closed: a broken awk program must never read as a clean scan
if ! awk -f "$awk_prog" /dev/null > /dev/null 2>&1; then
    echo "lint-restore-visibility: internal awk program failed to execute" >&2
    exit 2
fi

# awk exits 1 when it reports a site; that is a finding, not an error.
findings="$(printf '%s\n' "$files" | xargs awk -f "$awk_prog" 2>/dev/null || true)"
if [ -n "$findings" ]; then
    # rewrite the absolute path prefix to be repo-relative
    findings="$(printf '%s\n' "$findings" | sed "s#^STALE_RESTORE:${root}/#STALE_RESTORE:#")"
fi

if [ "$mode" = "list" ] && [ -n "$findings" ]; then
    printf '%s\n' "$findings"
fi
if [ "$mode" = "list" ]; then
    exit 0
fi

if [ -n "$findings" ]; then
    count="$(printf '%s\n' "$findings" | grep -c .)"
    printf '%s\n' "$findings"
    echo "lint-restore-visibility: ${count} unobserved mtime-preserving restore site(s)." >&2
    echo "A restored file is invisible to mtime-based rebuilds until it is touched or rewritten." >&2
    exit 1
fi

echo "lint-restore-visibility: OK (no unobserved mtime-preserving restore sites)"
exit 0
