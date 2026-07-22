#!/usr/bin/env bats

setup() {
    SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
    FIXTURE="$SCRIPT_DIR/tests/fixtures/secaudit/pii-ui.js"
}

@test "PII UI fixture contains no debug logging" {
    run grep -nE '(^|[^[:alnum:]_])console[[:space:]]*\.[[:space:]]*(log|debug)[[:space:]]*\(' "$FIXTURE"
    [ "$status" -ne 0 ]
}

@test "PII UI fixture remains valid JavaScript" {
    run node -e 'const fs = require("node:fs"); const vm = require("node:vm"); new vm.Script(fs.readFileSync(process.argv[1], "utf8"));' "$FIXTURE"
    [ "$status" -eq 0 ]
}
