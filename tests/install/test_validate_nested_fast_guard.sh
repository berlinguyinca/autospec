#!/usr/bin/env bash
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
VALIDATE_SH="$SCRIPT_DIR/scripts/validate.sh"
TEST_TMP="$(mktemp -d)"
trap 'rm -rf "$TEST_TMP"' EXIT
mkdir -p "$TEST_TMP/install"
cat > "$TEST_TMP/install/probe.sh" <<'SH'
#!/usr/bin/env bash
set -eu
printf 'ran\n' >> "${PROBE_OUT:?}"
SH
chmod +x "$TEST_TMP/install/probe.sh"

run_install_only() {
    PROBE_OUT="$TEST_TMP/probe.out" \
    AUTOSPEC_FORCE_LEGACY_SHELL=1 \
    AUTOSPEC_VALIDATE_INSTALL_TESTS_ONLY=1 \
    AUTOSPEC_VALIDATE_INSTALL_TEST_GLOB="$TEST_TMP/install/*.sh" \
    "$@"
}

: > "$TEST_TMP/probe.out"
run_install_only env -u AUTOSPEC_VALIDATE_LEGACY_ACTIVE bash "$VALIDATE_SH" --fast >"$TEST_TMP/validate.out" 2>&1
if ! grep -qx 'ran' "$TEST_TMP/probe.out"; then
    cat "$TEST_TMP/validate.out" >&2
    echo "FAIL: direct --fast validation must still run install tests"
    exit 1
fi

: > "$TEST_TMP/probe.out"
run_install_only env AUTOSPEC_VALIDATE_LEGACY_ACTIVE=1 bash "$VALIDATE_SH" --fast >"$TEST_TMP/validate.out" 2>&1
if grep -q 'ran' "$TEST_TMP/probe.out"; then
    cat "$TEST_TMP/validate.out" >&2
    echo "FAIL: nested --fast validation should skip install tests"
    exit 1
fi
if ! grep -q 'skipping install tests during nested --fast validation' "$TEST_TMP/validate.out"; then
    cat "$TEST_TMP/validate.out" >&2
    echo "FAIL: nested --fast skip did not explain the bounded install-test skip"
    exit 1
fi

: > "$TEST_TMP/probe.out"
run_install_only env AUTOSPEC_VALIDATE_LEGACY_ACTIVE=1 bash "$VALIDATE_SH" >"$TEST_TMP/validate.out" 2>&1
if ! grep -qx 'ran' "$TEST_TMP/probe.out"; then
    cat "$TEST_TMP/validate.out" >&2
    echo "FAIL: nested full validation must still run install tests"
    exit 1
fi

echo "PASS"
