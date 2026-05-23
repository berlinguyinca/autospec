#!/usr/bin/env bats
# Integration tests for snapshot/sqlite.sh
# These tests use real sqlite3 — no docker required.

SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts/snapshot"

setup() {
    TMP_DIR="$(mktemp -d)"
    # Create a fixture SQLite database
    FIXTURE_DB="$TMP_DIR/source.db"
    if command -v sqlite3 >/dev/null 2>&1; then
        sqlite3 "$FIXTURE_DB" <<'SQL'
CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, email TEXT);
INSERT INTO users VALUES (1, 'Alice', 'alice@example.com');
INSERT INTO users VALUES (2, 'Bob', 'bob@example.com');
CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT, price REAL);
INSERT INTO products VALUES (1, 'Widget', 9.99);
SQL
    else
        # Create a minimal binary-looking file so we can still test copy behavior
        printf 'SQLite format 3\x00' > "$FIXTURE_DB"
    fi
}

teardown() {
    rm -rf "$TMP_DIR"
}

@test "sqlite.sh: snapshot.db is created in out_dir" {
    OUT_DIR="$TMP_DIR/out"
    run bash "$SCRIPT_DIR/sqlite.sh" "$FIXTURE_DB" "$OUT_DIR"
    [ "$status" -eq 0 ]
    [ -f "$OUT_DIR/snapshot.db" ]
}

@test "sqlite.sh: snapshot.db contains the same tables as the source" {
    skip_if_no_sqlite3
    OUT_DIR="$TMP_DIR/out"
    run bash "$SCRIPT_DIR/sqlite.sh" "$FIXTURE_DB" "$OUT_DIR"
    [ "$status" -eq 0 ]
    # Verify same tables exist (VACUUM may change bytes but not logical content)
    src_tables=$(sqlite3 "$FIXTURE_DB" ".tables" | tr ' ' '\n' | sort | tr '\n' ',')
    snap_tables=$(sqlite3 "$OUT_DIR/snapshot.db" ".tables" | tr ' ' '\n' | sort | tr '\n' ',')
    [ "$src_tables" = "$snap_tables" ]
}

@test "sqlite.sh: snapshot.db is readable by sqlite3" {
    skip_if_no_sqlite3
    OUT_DIR="$TMP_DIR/out"
    bash "$SCRIPT_DIR/sqlite.sh" "$FIXTURE_DB" "$OUT_DIR"
    result=$(sqlite3 "$OUT_DIR/snapshot.db" "SELECT COUNT(*) FROM users;")
    [ "$result" -eq 2 ]
}

@test "sqlite.sh: out_dir is created if it does not exist" {
    OUT_DIR="$TMP_DIR/deep/nested/out"
    run bash "$SCRIPT_DIR/sqlite.sh" "$FIXTURE_DB" "$OUT_DIR"
    [ "$status" -eq 0 ]
    [ -d "$OUT_DIR" ]
}

@test "sqlite.sh: fails with exit 2 if source db does not exist" {
    run bash "$SCRIPT_DIR/sqlite.sh" "/nonexistent/path/db.sqlite" "$TMP_DIR/out"
    [ "$status" -eq 2 ]
}

@test "sqlite.sh: fails with exit 1 if no arguments given" {
    run bash "$SCRIPT_DIR/sqlite.sh"
    [ "$status" -eq 1 ]
}

@test "sqlite.sh: fails with exit 1 if only src_path given" {
    run bash "$SCRIPT_DIR/sqlite.sh" "$FIXTURE_DB"
    [ "$status" -eq 1 ]
}

@test "dispatch.sh: routes sqlite source correctly" {
    skip_if_no_python3
    OUT_BASE="$TMP_DIR/clone-snapshots"
    export SQLITE_DB_PATH="$FIXTURE_DB"
    CONTRACT_JSON=$(cat <<JSON
{
  "sources": [
    {
      "kind": "sqlite",
      "dsn_env": "SQLITE_DB_PATH"
    }
  ],
  "expose": { "kind": "docker_compose" }
}
JSON
)
    DISPATCH="$SCRIPT_DIR/dispatch.sh"
    run bash -c "printf '%s' '$CONTRACT_JSON' | bash '$DISPATCH' '$OUT_BASE'"
    [ "$status" -eq 0 ]
    # Snapshot dir created
    SNAP_COUNT=$(find "$OUT_BASE" -name "snapshot.db" | wc -l | tr -d ' ')
    [ "$SNAP_COUNT" -ge 1 ]
}

# Helper to skip if sqlite3 not available
skip_if_no_sqlite3() {
    if ! command -v sqlite3 >/dev/null 2>&1; then
        skip "sqlite3 not installed"
    fi
}

skip_if_no_python3() {
    if ! command -v python3 >/dev/null 2>&1; then
        skip "python3 not installed"
    fi
}
