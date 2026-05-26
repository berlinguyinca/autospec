#!/usr/bin/env bash
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"

# Build an isolated bin dir containing symlinks to every tool except codex.
# Stripping /usr/bin from PATH alone isn't enough on usrmerge systems where
# /bin is a symlink to /usr/bin and codex may live there too.
ISOLATED_BIN="$(mktemp -d)"
trap 'rm -rf "$ISOLATED_BIN"' EXIT INT TERM

for d in /usr/local/bin /usr/bin /bin; do
    [ -d "$d" ] || continue
    for f in "$d"/*; do
        bn="$(basename "$f")"
        [ "$bn" = codex ] && continue
        [ -e "$ISOLATED_BIN/$bn" ] && continue
        ln -s "$f" "$ISOLATED_BIN/$bn" 2>/dev/null || true
    done
done

# Sanity check: codex must be unreachable under the isolated PATH.
if PATH="$ISOLATED_BIN" command -v codex >/dev/null 2>&1; then
    echo "FAIL: could not isolate PATH from codex (still found at $(PATH="$ISOLATED_BIN" command -v codex))"
    exit 1
fi

output=$(PATH="$ISOLATED_BIN" bash "$SCRIPT_DIR/install.sh" --dry-run --skill autospec --harness claude 2>&1 || true)

case "$output" in
    *check_codex*) ;;
    *) echo "FAIL: check_codex step not reported"; echo "----"; echo "$output"; exit 1 ;;
esac

case "$output" in
    *"codex CLI NOT found"*) ;;
    *) echo "FAIL: codex-absence message missing"; exit 1 ;;
esac

case "$output" in
    *"peer-review will skip gracefully"*) ;;
    *) echo "FAIL: graceful-skip note missing"; exit 1 ;;
esac

echo "PASS"
