#!/usr/bin/env bash
# scripts/lint-cfg-test-statics.sh — reject new process-global `#[cfg(test)]`
# mutable statics in the executor_bridge tree (issue #3951).
#
# A `#[cfg(test)] static X: AtomicU8` (or `Mutex<T>`, `Cell<T>`, `RefCell<T>`,
# `UnsafeCell<T>`, `OnceCell<T>`, `RwLock<T>`) is a process-global, so every
# parallel `cargo test` thread in the same binary shares it. Consume-once
# failpoints of this shape made unrelated tests collide: one thread's
# `compare_exchange(0, 1)` consumed the value another thread had just armed
# (issue #3951; measured across 25 failpoints and 2 collector statics in
# crates/autospec-cli/src/commands/autonomous/executor_bridge.rs).
#
# The thread-scoped replacement is `thread_local! { static X: ... }`, whose
# per-thread state cannot collide. This lint therefore flags any `#[cfg(test)]`
# static whose type carries interior mutability, EXCEPT:
#
#   - statics declared inside a `thread_local! { ... }` block;
#   - `static X: Mutex<()>` — a unit-type lock is used purely as a barrier /
#     poison-free mutex; it holds no per-test data (e.g.
#     tests/test_coordination.rs TEST_FORK_LIFECYCLE).
#
# Constants (`static X: &str`, `static X: u64 = 1`, fn pointers, struct
# constants without interior mutability) are not findings.
#
# Waiver: the line immediately above the `#[cfg(test)]` attribute, the line
# immediately above the `static` declaration, or the declaration line itself
# may carry `// linter:allow-CFG_TEST_STATIC <reason>`. The reason is
# mandatory; a bare marker is rejected and the static stays flagged. Waived
# statics emit an advisory `INFO:CFG_TEST_STATIC:...` line for audit.
#
# Usage:
#   scripts/lint-cfg-test-statics.sh [PATH...]
#   scripts/lint-cfg-test-statics.sh --help
#
# With no arguments the scan covers the executor_bridge tree:
#   crates/autospec-cli/src/commands/autonomous/executor_bridge.rs and every
#   *.rs under crates/autospec-cli/src/commands/autonomous/executor_bridge/.
# PATH may be a .rs file or a directory (scanned recursively for *.rs).
#
# Output: one finding per line on stdout:
#   CFG_TEST_STATIC:<path>:<line>: <NAME>: <type>
#
# Exit code = number of findings (0 = pass), capped at 64.

set -eu

SCRIPT_PATH="$0"
SCRIPT_DIR=$(cd "$(dirname "$SCRIPT_PATH")" && pwd)
ROOT_DIR=$(cd "$SCRIPT_DIR/.." && pwd)

ALLOW_MARKER='linter:allow-CFG_TEST_STATIC'

AWK_PROG=$(cat <<'AWK'
function trim(s) { gsub(/^[ \t]+/, "", s); gsub(/[ \t]+$/, "", s); return s }
function nopen(s,  n) { n = 0; while ((m = index(s, "{")) > 0) { n++; s = substr(s, m + 1) } return n }
function nclose(s,  n) { n = 0; while ((m = index(s, "}")) > 0) { n++; s = substr(s, m + 1) } return n }
function waiver(line,  rest) {
    if (index(line, MARKER) == 0) return ""
    rest = line
    sub("^.*" MARKER "[ \t]*", "", rest)
    rest = trim(rest)
    sub(/^\/\/[ \t]*/, "", rest)
    return rest
}
{ lines[NR] = $0 }
END {
    in_tl = 0
    tl_depth = 0
    pending_cfg = 0
    for (i = 1; i <= NR; i++) {
        line = lines[i]
        t = trim(line)

        # thread_local! block tracking (declarations inside are exempt).
        if (in_tl == 0) {
            if (index(t, "thread_local!") > 0) {
                tl_depth = nopen(line) - nclose(line)
                if (tl_depth > 0) in_tl = 1
            }
        } else {
            tl_depth += nopen(line) - nclose(line)
            if (tl_depth <= 0) { in_tl = 0; tl_depth = 0 }
        }

        if (t ~ /^#\[[ \t]*cfg\([ \t]*test[ \t]*\)[ \t]*\][ \t]*$/ || t ~ /^#\[.*all\([ \t]*test([ \t]*,|$)/) {
            if (in_tl == 0) { pending_cfg = i; continue }
        }
        if (pending_cfg > 0) {
            if (t ~ /^#\[.*\]$/ || t ~ /^\/\/.*$/ || t == "") { continue }
            if (t ~ /^static[ \t]+[A-Z][A-Za-z0-9_]*[ \t]*:/) {
                name = t
                sub(/^static[ \t]+/, "", name)
                sub(/[ \t]*:.*/, "", name)
                typ = t
                sub(/^static[ \t]+[A-Z][A-Za-z0-9_]*[ \t]*:[ \t]*/, "", typ)
                sub(/[ \t]*=.*/, "", typ)
                typ = trim(typ)
                kind = ""
                if (typ ~ /(^|::)Atomic[A-Z]/) kind = "Atomic"
                else if (typ ~ /(^|::)Mutex<\(\)>$/) kind = "unit-mutex"
                else if (typ ~ /(^|::)Mutex</) kind = "Mutex"
                else if (typ ~ /(^|::)RefCell</) kind = "RefCell"
                else if (typ ~ /(^|::)Cell</) kind = "Cell"
                else if (typ ~ /(^|::)UnsafeCell</) kind = "UnsafeCell"
                else if (typ ~ /(^|::)OnceCell</) kind = "OnceCell"
                else if (typ ~ /(^|::)RwLock</) kind = "RwLock"
                w = waiver(lines[i])
                if (w == "" && i > 1) w = waiver(lines[i - 1])
                if (w == "" && pending_cfg > 1) w = waiver(lines[pending_cfg - 1])
                if (kind != "" && kind != "unit-mutex") {
                    if (w != "") {
                        printf "INFO:CFG_TEST_STATIC:%s:%d: waived (%s): %s: %s\n", FILE, i, kind, name, w
                    } else {
                        printf "CFG_TEST_STATIC:%d:%s:%s\n", i, name, typ
                    }
                } else if (kind == "unit-mutex") {
                    printf "INFO:CFG_TEST_STATIC:%s:%d: exempt (Mutex<()>): %s\n", FILE, i, name
                }
            }
            pending_cfg = 0
        }
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

BRIDGE_DIR="$ROOT_DIR/crates/autospec-cli/src/commands/autonomous/executor_bridge"
if [ ${#paths[@]} -eq 0 ]; then
    [ -f "$BRIDGE_DIR.rs" ] && paths+=("$BRIDGE_DIR.rs")
    [ -d "$BRIDGE_DIR" ] && paths+=("$BRIDGE_DIR")
fi
[ ${#paths[@]} -gt 0 ] || die 2 "no paths to scan (pass .rs file or directory paths)"

# ---- file collection --------------------------------------------------------

files=()
for p in "${paths[@]}"; do
    [ -e "$p" ] || die 2 "no such file or directory: $p"
    if [ -f "$p" ]; then
        case "$p" in
            *.rs) files+=("$p") ;;
            *) continue ;;
        esac
    elif [ -d "$p" ]; then
        while IFS= read -r f; do
            files+=("$f")
        done < <(find "$p" -type f -name '*.rs' | LC_ALL=C sort)
    else
        die 2 "not a file or directory: $p"
    fi
done

findings=0

if [ ${#files[@]} -eq 0 ]; then
    printf 'INFO: lint-cfg-test-statics: no .rs files to scan\n'
    exit 0
fi

for file in "${files[@]}"; do
    rel="$file"
    case "$rel" in
        "$ROOT_DIR"/*) rel="${rel#"$ROOT_DIR"/}" ;;
    esac

    out=$(LC_ALL=C awk -v FILE="$rel" -v MARKER="$ALLOW_MARKER" "$AWK_PROG" "$file") || die 3 "awk failed on $file"
    [ -n "$out" ] || continue

    while IFS= read -r cand; do
        case "$cand" in
            CFG_TEST_STATIC:*)
                rest="${cand#CFG_TEST_STATIC:}"
                ln="${rest%%:*}"
                rest="${rest#*:}"
                name="${rest%%:*}"
                typ="${rest#*:}"
                printf 'CFG_TEST_STATIC:%s:%s: %s: %s\n' "$rel" "$ln" "$name" "$typ"
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
