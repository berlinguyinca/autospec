#!/usr/bin/env bats
# security-remediation-loop.bats

setup() {
    SCRIPT_DIR="$(cd "$(dirname "${BATS_TEST_FILENAME}")/../.." && pwd)"
    LOOP="${SCRIPT_DIR}/scripts/security-remediation-loop.sh"
    TMP="$(mktemp -d /tmp/autospec-secloop-XXXXXX)"
    export AUTOSPEC_STATE_DIR="$TMP/state"; mkdir -p "$AUTOSPEC_STATE_DIR"
}
teardown() { rm -rf "$TMP"; }

mk_scan_stub() {  # mk_scan_stub <file-with-findings-per-line>
    cat > "$TMP/scan.sh" <<EOF
#!/usr/bin/env bash
cat "$1"
exit 0
EOF
    chmod +x "$TMP/scan.sh"
    export AUTOSPEC_SECSCAN_BIN="$TMP/scan.sh"
}

@test "clean scan -> decision=pass exit 0" {
    : > "$TMP/empty.txt"; mk_scan_stub "$TMP/empty.txt"
    run bash "$LOOP" --decide
    [ "$status" -eq 0 ]
    printf '%s' "$output" | grep -q 'decision=pass'
}

@test "surviving must-fix -> decision=block exit 1" {
    printf '%s\n' '{"gap_id":"G1","dimension":"vuln","severity":"must-fix","file":"a.py","line":2,"title":"sqli","body":"x","dedupe_key":"k"}' > "$TMP/f.txt"
    mk_scan_stub "$TMP/f.txt"
    run bash "$LOOP" --decide
    [ "$status" -eq 1 ]
    printf '%s' "$output" | grep -q 'decision=block'
}

@test "must-fix secret emits a rotation annotation" {
    printf '%s\n' '{"gap_id":"G1","dimension":"secrets","severity":"must-fix","file":"c.py","line":1,"title":"AWS key","body":"x","dedupe_key":"k"}' > "$TMP/f.txt"
    mk_scan_stub "$TMP/f.txt"
    run bash "$LOOP" --decide
    printf '%s' "$output" | grep -qi 'ROTATE'
}

@test "advisory-only findings (nice-to-have) -> decision=pass" {
    printf '%s\n' '{"gap_id":"G1","dimension":"cve","severity":"nice-to-have","file":"p","line":0,"title":"cve","body":"x","dedupe_key":"k"}' > "$TMP/f.txt"
    mk_scan_stub "$TMP/f.txt"
    run bash "$LOOP" --decide
    [ "$status" -eq 0 ]
    printf '%s' "$output" | grep -q 'decision=pass'
}

@test "engine fail-closed (scan exit 2) -> decision=block exit 2" {
    cat > "$TMP/scan2.sh" <<'EOF'
#!/usr/bin/env bash
exit 2
EOF
    chmod +x "$TMP/scan2.sh"
    export AUTOSPEC_SECSCAN_BIN="$TMP/scan2.sh"
    run bash "$LOOP" --decide
    [ "$status" -eq 2 ]
    printf '%s' "$output" | grep -q 'engine-failed-closed'
}
