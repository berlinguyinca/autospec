#!/usr/bin/env bash
set -eu

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP_DIR="$(mktemp -d)"
BIN="$TMP_DIR/bin"
CORE="$TMP_DIR/core"
TEST_HOME="$TMP_DIR/home"
AUTOSPEC_HOME="$TEST_HOME/.autospec"
LOG="$TMP_DIR/invocations.log"
failures=0

cleanup() {
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

fail() {
    printf 'FAIL: %s\n' "$*" >&2
    failures=$((failures + 1))
}

mkdir -p "$BIN" "$CORE" "$AUTOSPEC_HOME/repo/.git"
for command_name in bash cat chmod env dirname mkdir; do
    command_path="$(command -v "$command_name")"
    ln -s "$command_path" "$CORE/$command_name"
done

cat > "$BIN/id" <<'SHIM'
#!/usr/bin/env bash
[ "${1:-}" = "-u" ] && printf '1000\n'
SHIM

cat > "$BIN/sudo" <<SHIM
#!/usr/bin/env bash
printf 'sudo %s\n' "\$*" >> "$LOG"
exec "\$@"
SHIM

cat > "$BIN/apt-get" <<SHIM
#!/usr/bin/env bash
printf 'apt-get %s\n' "\$*" >> "$LOG"
cat > "$BIN/git" <<'GIT_SHIM'
#!/usr/bin/env bash
printf 'git %s\n' "\$*" >> "$LOG"
exit 0
GIT_SHIM
chmod +x "$BIN/git"
exit 0
SHIM

cat > "$AUTOSPEC_HOME/repo/install.sh" <<SHIM
#!/usr/bin/env bash
printf 'install %s\n' "\$*" >> "$LOG"
exit 0
SHIM

chmod +x "$BIN/id" "$BIN/sudo" "$BIN/apt-get" "$AUTOSPEC_HOME/repo/install.sh"

set +e
output=$(HOME="$TEST_HOME" PATH="$BIN:$CORE" AUTOSPEC_HOME="$AUTOSPEC_HOME" \
    bash "$ROOT/bootstrap.sh" --update 2>&1)
status=$?
set -e

if [ "$status" -ne 0 ]; then
    safe_output=$(printf '%s\n' "$output" | sed 's/command not found/COMMAND_MISSING/g; s/not found/MISSING/g')
    fail "bootstrap did not recover from missing Git: $safe_output"
fi
if [ ! -f "$LOG" ]; then
    fail "bootstrap did not invoke the package manager"
else
    grep -q '^sudo apt-get install -y git$' "$LOG" \
        || fail "bootstrap did not install Git through sudo and APT"
    grep -q '^install --update$' "$LOG" \
        || fail "bootstrap did not forward installer arguments"
fi
case "$output" in
    *"git not found; attempting installation"*) ;;
    *) fail "bootstrap did not report the Git installation attempt" ;;
esac

rm -f "$BIN/git" "$LOG"
set +e
dry_output=$(HOME="$TEST_HOME" PATH="$BIN:$CORE" AUTOSPEC_HOME="$AUTOSPEC_HOME" \
    bash "$ROOT/bootstrap.sh" --dry-run 2>&1)
dry_status=$?
set -e
if [ "$dry_status" -ne 0 ]; then
    fail "bootstrap dry-run failed without Git"
fi
case "$dry_output" in
    *"[dry-run] would install git via apt-get using sudo"*) ;;
    *) fail "bootstrap dry-run did not report the planned sudo installation" ;;
esac
if [ ! -f "$LOG" ]; then
    fail "bootstrap dry-run did not invoke the existing checkout installer"
else
    grep -q '^install --dry-run$' "$LOG" \
        || fail "bootstrap dry-run did not forward to the existing installer"
    if grep -Eq '^(sudo|apt-get|git) ' "$LOG"; then
        fail "bootstrap dry-run invoked a package manager, sudo, or Git mutation"
    fi
fi

EMPTY_HOME="$TEST_HOME/empty-autospec"
rm -f "$LOG"
set +e
empty_output=$(HOME="$TEST_HOME" PATH="$BIN:$CORE" AUTOSPEC_HOME="$EMPTY_HOME" \
    bash "$ROOT/bootstrap.sh" --dry-run 2>&1)
empty_status=$?
set -e
if [ "$empty_status" -ne 0 ]; then
    fail "bootstrap dry-run failed without an existing checkout"
fi
if [ -e "$EMPTY_HOME" ] || [ -e "$LOG" ]; then
    fail "bootstrap dry-run wrote state without an existing checkout"
fi
case "$empty_output" in
    *"[dry-run] would clone"*"[dry-run] would run"*) ;;
    *) fail "bootstrap dry-run did not report clone and install intent" ;;
esac

if [ "$failures" -ne 0 ]; then
    exit 1
fi

printf 'PASS\n'
