#!/usr/bin/env bats
# tests/issue-snapshot.bats — tests for scripts/issue-snapshot.sh
#
# Covers: single-fetch semantics (exactly one `gh issue view` call per get,
#         requesting body,title,url,labels in one payload), cache-first reuse,
#         --refresh re-fetch, atomic failure (failed refresh preserves the
#         prior snapshot), and usage errors.
#
# gh is stubbed via a PATH shim that logs every invocation to $GH_STUB_LOG
# and emits the JSON from $GH_STUB_JSON.

bats_require_minimum_version 1.5.0

SNAPSHOT_SCRIPT="${BATS_TEST_DIRNAME}/../scripts/issue-snapshot.sh"

setup() {
    TMP="$(mktemp -d)"
    STUB_BIN="$TMP/bin"
    SNAP_DIR="$TMP/snaps"
    mkdir -p "$STUB_BIN" "$SNAP_DIR"
    GH_STUB_LOG="$TMP/gh-calls.log"
    GH_STUB_JSON="$TMP/gh-payload.json"
    : > "$GH_STUB_LOG"
    cat > "$GH_STUB_JSON" <<'EOF'
{"body":"## Goal\nDo the thing.\n","title":"Snapshot test issue","url":"https://github.com/berlinguyinca/autospec/issues/123","labels":[{"id":"L1","name":"auto-implement"},{"id":"L2","name":"ctx:shallow"}]}
EOF
    cat > "$STUB_BIN/gh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "${GH_STUB_LOG:?}"
if [ -n "${GH_STUB_FAIL:-}" ]; then
    printf 'gh: stubbed failure\n' >&2
    exit 1
fi
cat "${GH_STUB_JSON:?}"
EOF
    chmod +x "$STUB_BIN/gh"
    export PATH="$STUB_BIN:$PATH"
    export GH_STUB_LOG GH_STUB_JSON
}

teardown() {
    rm -rf "$TMP" 2>/dev/null || true
}

gh_call_count() {
    wc -l < "$GH_STUB_LOG" | tr -d ' '
}

# ── syntax + usage ────────────────────────────────────────────────────────────

@test "bash -n exits 0" {
    run bash -n "$SNAPSHOT_SCRIPT"
    [ "$status" -eq 0 ]
}

@test "--help exits 0 and prints usage" {
    run bash "$SNAPSHOT_SCRIPT" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"issue-snapshot.sh get"* ]]
}

@test "no command exits 2" {
    run bash "$SNAPSHOT_SCRIPT"
    [ "$status" -eq 2 ]
}

@test "unknown command exits 2" {
    run bash "$SNAPSHOT_SCRIPT" bogus 123
    [ "$status" -eq 2 ]
}

@test "malformed issue number exits 2 (no path traversal)" {
    run bash "$SNAPSHOT_SCRIPT" get '../etc' --dir "$SNAP_DIR"
    [ "$status" -eq 2 ]
    run bash "$SNAPSHOT_SCRIPT" get '' --dir "$SNAP_DIR"
    [ "$status" -eq 2 ]
}

# ── get: single-fetch semantics ───────────────────────────────────────────────

@test "get: exactly one gh call requesting body,title,url,labels" {
    run bash "$SNAPSHOT_SCRIPT" get 123 --dir "$SNAP_DIR"
    [ "$status" -eq 0 ]
    [ "$(gh_call_count)" = "1" ]
    grep -q 'issue view 123 --json body,title,url,labels' "$GH_STUB_LOG"
    [ "$output" = "$SNAP_DIR/autospec-issue-123.json" ]
    [ -s "$SNAP_DIR/autospec-issue-123.json" ]
}

@test "get: snapshot file holds all four fields" {
    run bash "$SNAPSHOT_SCRIPT" get 123 --dir "$SNAP_DIR"
    [ "$status" -eq 0 ]
    run jq -r '.title' "$SNAP_DIR/autospec-issue-123.json"
    [ "$status" -eq 0 ]
    [ "$output" = "Snapshot test issue" ]
    run jq -r '[.labels[]?.name] | join(", ")' "$SNAP_DIR/autospec-issue-123.json"
    [ "$status" -eq 0 ]
    [ "$output" = "auto-implement, ctx:shallow" ]
}

@test "get: cache-first — second get makes no network call" {
    run bash "$SNAPSHOT_SCRIPT" get 123 --dir "$SNAP_DIR"
    [ "$status" -eq 0 ]
    run bash "$SNAPSHOT_SCRIPT" get 123 --dir "$SNAP_DIR"
    [ "$status" -eq 0 ]
    [ "$(gh_call_count)" = "1" ]
    [ "$output" = "$SNAP_DIR/autospec-issue-123.json" ]
}

@test "get --refresh: forces a fresh fetch" {
    run bash "$SNAPSHOT_SCRIPT" get 123 --dir "$SNAP_DIR"
    [ "$status" -eq 0 ]
    run bash "$SNAPSHOT_SCRIPT" get 123 --refresh --dir "$SNAP_DIR"
    [ "$status" -eq 0 ]
    [ "$(gh_call_count)" = "2" ]
}

@test "get: gh failure exits 1 and writes no snapshot" {
    export GH_STUB_FAIL=1
    run bash "$SNAPSHOT_SCRIPT" get 123 --dir "$SNAP_DIR"
    [ "$status" -eq 1 ]
    [ ! -e "$SNAP_DIR/autospec-issue-123.json" ]
}

@test "get --refresh: failed refresh preserves the prior snapshot (atomic)" {
    run bash "$SNAPSHOT_SCRIPT" get 123 --dir "$SNAP_DIR"
    [ "$status" -eq 0 ]
    export GH_STUB_FAIL=1
    run bash "$SNAPSHOT_SCRIPT" get 123 --refresh --dir "$SNAP_DIR"
    [ "$status" -eq 1 ]
    run jq -r '.title' "$SNAP_DIR/autospec-issue-123.json"
    [ "$status" -eq 0 ]
    [ "$output" = "Snapshot test issue" ]
}

# ── dir resolution ────────────────────────────────────────────────────────────

@test "AUTOSPEC_SNAPSHOT_DIR is used when --dir is absent" {
    export AUTOSPEC_SNAPSHOT_DIR="$SNAP_DIR"
    run bash "$SNAPSHOT_SCRIPT" get 123
    [ "$status" -eq 0 ]
    [ "$output" = "$SNAP_DIR/autospec-issue-123.json" ]
}

@test "--dir wins over AUTOSPEC_SNAPSHOT_DIR" {
    export AUTOSPEC_SNAPSHOT_DIR="$SNAP_DIR"
    override="$TMP/override"
    mkdir -p "$override"
    run bash "$SNAPSHOT_SCRIPT" get 123 --dir "$override"
    [ "$status" -eq 0 ]
    [ "$output" = "$override/autospec-issue-123.json" ]
}
