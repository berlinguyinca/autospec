#!/usr/bin/env bats
# Integration tests for snapshot/pg.sh
# Requires docker to spin up an ephemeral PostgreSQL container.
# Skip gracefully if docker is unavailable.

SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts/snapshot"
COMPOSE_FILE="$(dirname "$BATS_TEST_FILENAME")/fixtures/docker-compose-pg.yml"

PG_CONTAINER_NAME="autospec-test-pg-$$"
PG_PORT="15432"
PG_USER="testuser"
PG_PASSWORD="testpass"
PG_DB="testdb"
export PG_DSN="postgres://${PG_USER}:${PG_PASSWORD}@127.0.0.1:${PG_PORT}/${PG_DB}"

setup_file() {
    if ! command -v docker >/dev/null 2>&1; then
        return 0
    fi
    docker run -d \
        --name "$PG_CONTAINER_NAME" \
        -e POSTGRES_USER="$PG_USER" \
        -e POSTGRES_PASSWORD="$PG_PASSWORD" \
        -e POSTGRES_DB="$PG_DB" \
        -p "${PG_PORT}:5432" \
        postgres:15-alpine \
        >/dev/null 2>&1 || true

    # Wait for PG to be ready (up to 30s)
    local i=0
    while [ $i -lt 30 ]; do
        if docker exec "$PG_CONTAINER_NAME" pg_isready -U "$PG_USER" -d "$PG_DB" >/dev/null 2>&1; then
            break
        fi
        sleep 1
        i=$((i + 1))
    done

    # Seed test data
    docker exec "$PG_CONTAINER_NAME" psql -U "$PG_USER" -d "$PG_DB" -c "
        CREATE TABLE IF NOT EXISTS users (id SERIAL PRIMARY KEY, name TEXT, email TEXT);
        INSERT INTO users (name, email) VALUES ('Alice', 'alice@example.com'), ('Bob', 'bob@example.com');
        CREATE TABLE IF NOT EXISTS events (id SERIAL PRIMARY KEY, user_id INT, action TEXT);
        INSERT INTO events (user_id, action) VALUES (1, 'login'), (2, 'logout'), (1, 'purchase');
    " >/dev/null 2>&1 || true
}

teardown_file() {
    if command -v docker >/dev/null 2>&1; then
        docker rm -f "$PG_CONTAINER_NAME" >/dev/null 2>&1 || true
    fi
}

setup() {
    TMP_DIR="$(mktemp -d)"
}

teardown() {
    rm -rf "$TMP_DIR"
}

@test "pg.sh: schema.sql created for postgres source" {
    require_docker_and_pg
    OUT_DIR="$TMP_DIR/out"
    DSN="$PG_DSN" run bash "$SCRIPT_DIR/pg.sh" "users" "{}" "$OUT_DIR"
    [ "$status" -eq 0 ]
    [ -f "$OUT_DIR/schema.sql" ]
}

@test "pg.sh: full table dump creates <table>.sql" {
    require_docker_and_pg
    OUT_DIR="$TMP_DIR/out"
    DSN="$PG_DSN" run bash "$SCRIPT_DIR/pg.sh" "users" "{}" "$OUT_DIR"
    [ "$status" -eq 0 ]
    [ -f "$OUT_DIR/users.sql" ]
}

@test "pg.sh: multiple full tables dumped" {
    require_docker_and_pg
    OUT_DIR="$TMP_DIR/out"
    DSN="$PG_DSN" run bash "$SCRIPT_DIR/pg.sh" "users,events" "{}" "$OUT_DIR"
    [ "$status" -eq 0 ]
    [ -f "$OUT_DIR/users.sql" ]
    [ -f "$OUT_DIR/events.sql" ]
}

@test "pg.sh: fails with exit 2 if DSN not set" {
    OUT_DIR="$TMP_DIR/out"
    run env -u DSN bash "$SCRIPT_DIR/pg.sh" "users" "{}" "$OUT_DIR"
    [ "$status" -eq 2 ]
}

@test "pg.sh: fails with exit 1 if OUT_DIR not provided" {
    run bash "$SCRIPT_DIR/pg.sh"
    [ "$status" -eq 1 ]
}

@test "pg.sh: out_dir is created if it does not exist" {
    require_docker_and_pg
    OUT_DIR="$TMP_DIR/deep/nested/out"
    DSN="$PG_DSN" run bash "$SCRIPT_DIR/pg.sh" "" "{}" "$OUT_DIR"
    [ "$status" -eq 0 ]
    [ -d "$OUT_DIR" ]
}

@test "dispatch.sh: routes postgres source correctly" {
    require_docker_and_pg
    OUT_BASE="$TMP_DIR/clone-snapshots"
    export TEST_PG_DSN="$PG_DSN"
    CONTRACT_JSON=$(cat <<JSON
{
  "sources": [
    {
      "kind": "postgres",
      "dsn_env": "TEST_PG_DSN",
      "tables_full": ["users"]
    }
  ],
  "expose": { "kind": "docker_compose" }
}
JSON
)
    DISPATCH="$SCRIPT_DIR/dispatch.sh"
    run bash -c "printf '%s' \"\$CONTRACT_JSON\" | bash '$DISPATCH' '$OUT_BASE'"
    [ "$status" -eq 0 ]
    SCHEMA_COUNT=$(find "$OUT_BASE" -name "schema.sql" | wc -l | tr -d ' ')
    [ "$SCHEMA_COUNT" -ge 1 ]
}

# Helpers
require_docker_and_pg() {
    if ! command -v docker >/dev/null 2>&1; then
        skip "docker not available"
    fi
    if ! command -v pg_dump >/dev/null 2>&1; then
        skip "pg_dump not available"
    fi
    if ! docker ps --filter "name=$PG_CONTAINER_NAME" --format "{{.Names}}" | grep -q "$PG_CONTAINER_NAME"; then
        skip "postgres container not running"
    fi
}
