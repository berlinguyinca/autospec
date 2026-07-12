#!/usr/bin/env bash
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
VALIDATE_SH="$SCRIPT_DIR/scripts/validate.sh"

if ! grep -q 'AUTOSPEC_VALIDATE_LEGACY_ACTIVE_AT_ENTRY=' "$VALIDATE_SH"; then
    echo "FAIL: scripts/validate.sh does not snapshot validate reentrancy at entry"
    exit 1
fi

if ! awk '/check_install_tests\(\)/,/^}/' "$VALIDATE_SH" | grep -q 'VALIDATE_NESTED_FAST_AT_ENTRY'; then
    echo "FAIL: check_install_tests has no nested --fast recursion guard"
    exit 1
fi

if ! awk '/check_install_tests\(\)/,/^}/' "$VALIDATE_SH" | grep -q 'skipping install tests during nested --fast validation'; then
    echo "FAIL: nested --fast guard does not explain the bounded install-test skip"
    exit 1
fi

echo "PASS"
