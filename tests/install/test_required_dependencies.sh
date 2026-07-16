#!/usr/bin/env bash
set -eu

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP_DIR="$(mktemp -d)"
ISOLATED_BIN="$TMP_DIR/bin"
FAKE_HOME="$TMP_DIR/home"
ORIGINAL_HOME="$HOME"
ORIGINAL_PATH="$PATH"
failures=0

cleanup() {
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

fail() {
    printf 'FAIL: %s\n' "$*" >&2
    failures=$((failures + 1))
}

mkdir -p "$ISOLATED_BIN" "$FAKE_HOME"

# Preserve normal shell utilities while excluding the dependencies under test.
old_ifs="$IFS"
IFS=:
for command_dir in $ORIGINAL_PATH; do
    [ -d "$command_dir" ] || continue
    for command_path in "$command_dir"/*; do
        [ -e "$command_path" ] || continue
        command_name="$(basename "$command_path")"
        case "$command_name" in
            git|curl|cargo|python3|gh|jq|codex|claude|opencode) continue ;;
        esac
        [ -e "$ISOLATED_BIN/$command_name" ] && continue
        ln -s "$command_path" "$ISOLATED_BIN/$command_name" 2>/dev/null || true
    done
done
IFS="$old_ifs"

dry_output=$(AUTOSPEC_SKIP_AGENT_ENV_ALIASES=1 \
    bash "$ROOT/install.sh" --dry-run --skill autospec --harness codex 2>&1 || true)
required_line=$(printf '%s\n' "$dry_output" | grep -n 'ensure_required_system_tools' | head -1 | cut -d: -f1 || true)
build_line=$(printf '%s\n' "$dry_output" | grep -n 'install_autospec_runtime_binary: cargo build' | head -1 | cut -d: -f1 || true)
if [ -z "$required_line" ] || [ -z "$build_line" ] || [ "$required_line" -ge "$build_line" ]; then
    fail "required tools were not ensured before cargo build"
fi

set +e
missing_output=$(HOME="$FAKE_HOME" PATH="$ISOLATED_BIN" \
    AUTOSPEC_SKIP_SYSTEM_TOOLS=1 \
    AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1 \
    AUTOSPEC_SKIP_AGENT_ENV_ALIASES=1 \
    AUTOSPEC_NO_DB_PROMPT=1 \
    AUTOSPEC_NO_STAR_PROMPT=1 \
    bash "$ROOT/install.sh" --skill autospec --harness codex 2>&1)
missing_status=$?
set -e

if [ "$missing_status" -eq 0 ]; then
    fail "missing required tools did not fail installation"
fi
case "$missing_output" in
    *"required missing:"*) ;;
    *) fail "missing dependency report has no required-missing category" ;;
esac
for tool in git curl cargo python3 gh jq; do
    case "$missing_output" in
        *"$tool"*) ;;
        *) fail "missing dependency report omitted $tool" ;;
    esac
done
case "$missing_output" in
    *"install_autospec_runtime_binary: building"*) fail "cargo build ran after required verification failed" ;;
esac

# Make every core command available while keeping all harnesses absent. Real
# Cargo exercises the actual build boundary; all network-facing commands are
# inert local shims.
for tool in git curl python3 gh jq; do
    printf '#!/usr/bin/env bash\nexit 0\n' > "$ISOLATED_BIN/$tool"
    chmod +x "$ISOLATED_BIN/$tool"
done
ln -s "$(command -v cargo)" "$ISOLATED_BIN/cargo"

set +e
harness_output=$(HOME="$FAKE_HOME" PATH="$ISOLATED_BIN" \
    CARGO_HOME="${CARGO_HOME:-$ORIGINAL_HOME/.cargo}" \
    RUSTUP_HOME="${RUSTUP_HOME:-$ORIGINAL_HOME/.rustup}" \
    AUTOSPEC_SKIP_SYSTEM_TOOLS=1 \
    AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1 \
    AUTOSPEC_SKIP_AGENT_ENV_ALIASES=1 \
    AUTOSPEC_NO_DB_PROMPT=1 \
    AUTOSPEC_NO_STAR_PROMPT=1 \
    bash "$ROOT/install.sh" --skill autospec --harness codex 2>&1)
harness_status=$?
set -e

if [ "$harness_status" -eq 0 ]; then
    fail "installation succeeded without codex, claude, or opencode"
fi
case "$harness_output" in
    *"required harness missing: codex claude opencode"*) ;;
    *) fail "harness failure did not list the required harness alternatives" ;;
esac

if [ "$failures" -ne 0 ]; then
    exit 1
fi

printf 'PASS\n'
